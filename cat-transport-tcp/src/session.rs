// Copyright 2026 Matt Franklin
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! [`TcpCatSession`]: a [`CatSession`] implementation over
//! `monoio::net::TcpStream`, using length-prefixed frames.
//!
//! # Wire format
//!
//! Every message (request *or* response) sent over the connection is one
//! **frame**:
//!
//! ```text
//! +----------------------------+---------------------------------+
//! | length prefix (4 bytes)    | payload (`length` bytes)         |
//! | u32, big-endian            | raw request/response bytes       |
//! +----------------------------+---------------------------------+
//! ```
//!
//! - The length prefix is a 4-byte unsigned integer in **big-endian**
//!   (network) byte order.
//! - The length prefix counts **only** the payload bytes that follow it --
//!   it does **not** include itself (the 4 prefix bytes), and it does
//!   **not** include any terminator.
//! - The payload is the raw CAT command/response bytes exactly as they
//!   would appear on the wire for serial (e.g. `b"FA;"` or
//!   `b"FA00014250000;"`), with no additional wrapping, escaping, or
//!   terminator added by this framing layer. The length prefix alone
//!   determines where the payload ends -- there is no `';'` scan.
//! - A payload length of zero is a valid, complete frame (length prefix
//!   `0x00000000`, zero payload bytes) and represents
//!   [`ResponseDisposition::NoResponse`].
//! - [`MAX_FRAME_SIZE`] (64 KiB) bounds the payload length both crates
//!   apply. A frame whose declared length exceeds this is rejected
//!   ([`TcpSessionError::FrameTooLarge`]) as soon as the length prefix is
//!   read, *before* attempting to read (or allocate a buffer for) the
//!   declared payload.
//!
//! # Request/response pairing
//!
//! `execute()` writes exactly **one** request frame, then reads exactly
//! **one** response frame in reply, in order, on the same connection.
//! There is no in-frame request/session ID here -- TCP already guarantees
//! ordered, reliable, connection-scoped delivery, so 1:1 framing is
//! sufficient; envelope IDs are `cat-transport-udp`'s concern, not this
//! crate's.
//!
//! **This places a hard requirement on any wire-compatible peer (e.g. a
//! future `cat-server` TCP listener): it MUST write back exactly one
//! response frame for every request frame it receives, even when the
//! underlying dispatch produces no response bytes -- in that case it must
//! send an *empty* (zero-length) frame, never silence.** `TcpCatSession`
//! does not override [`CatSession::send`]'s default (forward to `execute`
//! and discard the response) specifically because this guarantee holds; a
//! peer that goes silent on a request will hang the client's read forever
//! (see the module-level doc for why this differs from
//! `cat-transport-serial::SerialCatSession`, which overrides `send` for the
//! opposite reason -- the real radio never answers set commands at all).
//!
//! # Reusable codec primitives
//!
//! [`write_frame`], [`read_frame`], and [`read_frame_or_eof`] are `pub` so a
//! server-side accept loop implementing this same wire format (e.g.
//! `cat-server`'s TCP listener) can import and call them directly instead of
//! hand-rolling a second copy of this encode/decode logic that could drift
//! out of sync. [`read_frame`] is client-shaped (any I/O failure, including
//! a clean disconnect, is an error); [`read_frame_or_eof`] is the primitive
//! a server needs instead, since a clean hangup *between* requests on an
//! already-open connection is normal, not an error -- see
//! [`read_frame_or_eof`]'s own doc for the exact distinction it draws.

use async_trait::async_trait;
use monoio::io::{AsyncReadRent, AsyncReadRentExt, AsyncWriteRentExt};
use monoio::net::TcpStream;
use thiserror::Error;

use cat_transport_core::{CatSession, ResponseDisposition};

/// Maximum payload length, in bytes, that [`TcpCatSession`] will write or
/// accept in a single frame.
///
/// 64 KiB. Judgment call (not derived from any existing spec): known
/// TS-570D-class CAT command/response frames are on the order of tens of
/// bytes (the widest known response type is well under 100 bytes), so 64
/// KiB leaves roughly three orders of magnitude of headroom for future
/// radios or coarser batched frames, while still bounding the worst-case
/// single-frame buffer allocation to a small, fixed amount per connection
/// (important for a future `cat-server` handling many concurrent client
/// connections). A frame declaring a larger length is rejected outright,
/// without attempting to read or allocate for the declared payload.
pub const MAX_FRAME_SIZE: u32 = 64 * 1024;

/// Errors from [`TcpCatSession`]'s frame I/O.
#[derive(Debug, Error)]
pub enum TcpSessionError {
    /// The underlying TCP connection failed while reading or writing a
    /// frame -- includes a peer disconnecting mid-frame (surfaces as
    /// `io::ErrorKind::UnexpectedEof`, since `read_exact`-style framing
    /// cannot complete when the connection closes before the declared
    /// number of bytes arrive).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A frame's declared length prefix exceeded [`MAX_FRAME_SIZE`]. The
    /// connection's byte stream is left mid-frame (the payload was never
    /// read) -- callers should treat this session as unusable and drop the
    /// connection rather than attempting further requests on it.
    #[error("frame length {len} exceeds max frame size {max} bytes")]
    FrameTooLarge {
        /// The length the peer declared.
        len: u32,
        /// The configured maximum ([`MAX_FRAME_SIZE`]).
        max: u32,
    },
}

/// [`CatSession`] backed by a `monoio::net::TcpStream`, using
/// length-prefixed framing.
///
/// See the module-level docs for the exact wire format. Unlike
/// `cat-transport-serial::SerialCatSession<T: Transport>`, this type wraps
/// `monoio::net::TcpStream` directly rather than being generic over the
/// byte-level `Transport` trait -- TCP framing needs its own read/write
/// primitives (length-prefixed `read_exact`/`write_all` over owned buffers),
/// not `Transport`'s borrowed-slice `read`/`write` shape.
pub struct TcpCatSession {
    stream: TcpStream,
}

impl TcpCatSession {
    /// Wrap an already-connected `TcpStream` in a session that performs
    /// length-prefixed frame I/O.
    pub fn new(stream: TcpStream) -> Self {
        Self { stream }
    }

    /// Connect to `addr` and wrap the resulting stream in a new session.
    ///
    /// Convenience constructor for callers that don't need to configure the
    /// stream (e.g. `set_nodelay`) before wrapping it -- construct a
    /// `TcpStream` directly and use [`new`](Self::new) for that case.
    pub async fn connect<A: std::net::ToSocketAddrs>(addr: A) -> Result<Self, TcpSessionError> {
        let stream = TcpStream::connect(addr).await?;
        Ok(Self::new(stream))
    }
}

/// Write one length-prefixed frame: a 4-byte big-endian length prefix
/// (payload bytes only), followed by `payload` verbatim.
///
/// `pub` so a server-side accept loop (e.g. `cat-server`'s TCP listener) can
/// write response frames using exactly this crate's own encoder instead of
/// a hand-rolled duplicate that could drift out of sync with it. Writing a
/// response frame is symmetric with writing a request frame -- there is no
/// EOF-tolerance concern on the write side, so this needs no counterpart
/// analogous to [`read_frame_or_eof`].
pub async fn write_frame(stream: &mut TcpStream, payload: &[u8]) -> Result<(), TcpSessionError> {
    let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    if len > MAX_FRAME_SIZE {
        return Err(TcpSessionError::FrameTooLarge {
            len,
            max: MAX_FRAME_SIZE,
        });
    }

    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(payload);

    let (result, _buf) = stream.write_all(frame).await;
    result?;
    Ok(())
}

/// Read one length-prefixed frame: a 4-byte big-endian length prefix,
/// followed by exactly that many payload bytes.
///
/// Rejects a declared length greater than [`MAX_FRAME_SIZE`] immediately
/// after reading the prefix, without attempting to read (or allocate a
/// buffer for) the declared payload.
///
/// This is the **client-shaped** primitive: a frame is always assumed to be
/// coming, so *any* I/O failure -- including a clean disconnect before a
/// single byte of it arrives -- is reported as an error (a client waiting
/// on a response never expects the peer to simply vanish). A thin wrapper
/// over [`read_frame_or_eof`], which is the primitive a server-side accept
/// loop needs instead: it must distinguish a clean hangup *between*
/// requests (not an error) from a disconnect *mid*-frame (an error). Both
/// share the same length-prefix/payload-size logic in `read_frame_or_eof`
/// so neither a client-shaped caller nor a server-shaped one has to
/// duplicate it.
pub async fn read_frame(stream: &mut TcpStream) -> Result<Vec<u8>, TcpSessionError> {
    match read_frame_or_eof(stream).await? {
        Some(payload) => Ok(payload),
        None => Err(TcpSessionError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "connection closed before a frame was received",
        ))),
    }
}

/// Read one length-prefixed frame, tolerating a clean disconnect exactly at
/// a frame boundary (i.e. before any byte of a new frame has arrived).
///
/// Returns:
/// - `Ok(Some(payload))` -- a complete frame was read.
/// - `Ok(None)` -- the peer closed the connection cleanly with *zero* bytes
///   of a new frame read. Not an error: on a server-side accept loop
///   reading requests one after another on the same connection, this is
///   just a client that hung up between requests.
/// - `Err(_)` -- anything else: a disconnect *after* at least one byte of
///   the length prefix (or payload) had already arrived (mid-frame -- the
///   connection is now in an unrecoverable, partially-consumed state), an
///   oversized declared length (rejected immediately after the prefix is
///   read, before attempting to read or allocate a buffer for the declared
///   payload), or any other I/O failure.
///
/// [`read_frame`] is a thin wrapper over this function for callers (like
/// [`TcpCatSession::execute`]) that always expect a frame and have no use
/// for the EOF-tolerant `None` case.
pub async fn read_frame_or_eof(stream: &mut TcpStream) -> Result<Option<Vec<u8>>, TcpSessionError> {
    let Some(len_buf) = read_exact_or_eof(stream, 4).await? else {
        return Ok(None);
    };
    let len = u32::from_be_bytes(
        len_buf
            .try_into()
            .expect("read_exact_or_eof(.., 4) always yields a 4-byte Vec on Some"),
    );

    if len > MAX_FRAME_SIZE {
        return Err(TcpSessionError::FrameTooLarge {
            len,
            max: MAX_FRAME_SIZE,
        });
    }

    let (result, payload) = stream.read_exact(vec![0u8; len as usize]).await;
    result?;
    Ok(Some(payload))
}

/// Read exactly `len` bytes, or detect a clean EOF that occurs before any
/// byte at all has been read.
///
/// Returns `Ok(None)` only when the very first bytes transferred is zero --
/// i.e. the peer closed the connection at a clean boundary, before sending
/// anything. An EOF encountered after at least one byte has already
/// arrived is a disconnect *mid*-read, which is just as much an error here
/// as it is for `AsyncReadRentExt::read_exact`'s own behavior (both
/// surface as `io::ErrorKind::UnexpectedEof`).
///
/// Not `pub`: this is purely an internal helper for [`read_frame_or_eof`]'s
/// header read. A future caller needing an EOF-tolerant exact-length read
/// for something other than this crate's 4-byte length prefix does not
/// exist today -- keep this crate-private until one does.
async fn read_exact_or_eof(
    stream: &mut TcpStream,
    len: usize,
) -> Result<Option<Vec<u8>>, TcpSessionError> {
    let mut out = Vec::with_capacity(len);
    loop {
        let remaining = len - out.len();
        if remaining == 0 {
            return Ok(Some(out));
        }

        let chunk = vec![0u8; remaining];
        let (result, chunk) = stream.read(chunk).await;
        match result {
            Ok(0) => {
                return if out.is_empty() {
                    Ok(None)
                } else {
                    Err(TcpSessionError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "failed to fill whole buffer",
                    )))
                };
            }
            Ok(n) => out.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(TcpSessionError::Io(e)),
        }
    }
}

#[async_trait(?Send)]
impl CatSession for TcpCatSession {
    type Error = TcpSessionError;

    async fn execute(
        &mut self,
        request: &[u8],
        response: &mut Vec<u8>,
    ) -> Result<ResponseDisposition, TcpSessionError> {
        write_frame(&mut self.stream, request).await?;
        let payload = read_frame(&mut self.stream).await?;

        let disposition = if payload.is_empty() {
            ResponseDisposition::NoResponse
        } else {
            ResponseDisposition::ResponseWritten
        };

        response.extend_from_slice(&payload);
        Ok(disposition)
    }

    // `send` is deliberately NOT overridden -- see the module-level doc for
    // why the default (forward to `execute`, discard the response) is
    // correct here: this wire protocol requires a peer to answer every
    // request frame with exactly one response frame, so there is never a
    // "the radio will never answer this" case to guard against the way
    // `SerialCatSession::send` does.
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_transport_core::conformance;
    use std::net::SocketAddr;

    use monoio::net::TcpListener;

    /// Write one raw length-prefixed frame directly to `stream`.
    ///
    /// Deliberately does **not** call [`write_frame`] -- this is an
    /// independent, hand-rolled encoder so the test suite cross-checks the
    /// *documented* wire format (see this module's doc comment) against the
    /// production implementation, rather than only checking `write_frame`
    /// against `read_frame`.
    async fn write_raw_frame(stream: &mut TcpStream, payload: &[u8]) {
        let len = payload.len() as u32;
        let mut frame = Vec::with_capacity(4 + payload.len());
        frame.extend_from_slice(&len.to_be_bytes());
        frame.extend_from_slice(payload);
        let (result, _buf) = stream.write_all(frame).await;
        result.expect("write_raw_frame: write_all failed");
    }

    /// Read one raw length-prefixed frame directly from `stream`.
    ///
    /// Independent, hand-rolled decoder -- see [`write_raw_frame`].
    async fn read_raw_frame(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
        let (result, len_buf) = stream.read_exact(vec![0u8; 4]).await;
        result?;
        let len = u32::from_be_bytes(len_buf.try_into().unwrap());
        let (result, payload) = stream.read_exact(vec![0u8; len as usize]).await;
        result?;
        Ok(payload)
    }

    /// Bind an ephemeral loopback TCP listener for a test.
    fn bind_loopback() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("TcpListener::bind failed");
        let addr = listener.local_addr().expect("local_addr failed");
        (listener, addr)
    }

    /// Connect a `TcpCatSession` to `addr`.
    async fn connect_session(addr: SocketAddr) -> TcpCatSession {
        let stream = TcpStream::connect(addr)
            .await
            .expect("TcpStream::connect failed");
        TcpCatSession::new(stream)
    }

    // -----------------------------------------------------------------------
    // conformance harness -- reused unchanged from cat-transport-core,
    // exercised here against a real loopback TcpCatSession instead of
    // ScriptedCatSession.
    // -----------------------------------------------------------------------

    #[monoio::test(driver = "legacy")]
    async fn conformance_query_round_trip() {
        let (listener, addr) = bind_loopback();
        monoio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept failed");
            let req = read_raw_frame(&mut stream)
                .await
                .expect("peer: read request frame failed");
            assert_eq!(req, b"ID;");
            write_raw_frame(&mut stream, b"ID017;").await;
        });

        let mut session = connect_session(addr).await;
        conformance::query_round_trip(&mut session, b"ID;", b"ID017;").await;
    }

    #[monoio::test(driver = "legacy")]
    async fn conformance_set_is_fire_and_forget() {
        let (listener, addr) = bind_loopback();
        monoio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept failed");
            let req = read_raw_frame(&mut stream)
                .await
                .expect("peer: read request frame failed");
            assert_eq!(req, b"TX;");
            // Fire-and-forget still gets an explicit empty response frame --
            // required by this wire protocol's 1:1 framing (see module doc).
            write_raw_frame(&mut stream, b"").await;
        });

        let mut session = connect_session(addr).await;
        conformance::set_is_fire_and_forget(&mut session, b"TX;").await;
    }

    #[monoio::test(driver = "legacy")]
    async fn conformance_surfaces_transport_error() {
        let (listener, addr) = bind_loopback();
        monoio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept failed");
            // Simulate a disconnect: close immediately without reading or
            // responding.
            drop(stream);
        });

        let mut session = connect_session(addr).await;
        conformance::surfaces_transport_error(&mut session, b"FA;").await;
    }

    // -----------------------------------------------------------------------
    // TCP-specific framing tests: partial reads, oversized frames,
    // disconnect mid-frame.
    // -----------------------------------------------------------------------

    #[monoio::test(driver = "legacy")]
    async fn handles_response_length_prefix_and_payload_arriving_in_separate_reads() {
        let (listener, addr) = bind_loopback();
        let expected_response = b"FA00014250000;".to_vec();
        let response_for_peer = expected_response.clone();

        monoio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept failed");
            let req = read_raw_frame(&mut stream)
                .await
                .expect("peer: read request frame failed");
            assert_eq!(req, b"FA;");

            // Write the length prefix on its own, then the payload split
            // across two separate writes, with a real io_uring write
            // completion boundary between each -- exercising the client's
            // read_exact-based reassembly across multiple `read()` calls
            // rather than a single one.
            let len = response_for_peer.len() as u32;
            let (result, _) = stream.write_all(len.to_be_bytes().to_vec()).await;
            result.expect("peer: write length prefix failed");

            let (first_half, second_half) = response_for_peer.split_at(response_for_peer.len() / 2);
            let (result, _) = stream.write_all(first_half.to_vec()).await;
            result.expect("peer: write first half failed");

            let (result, _) = stream.write_all(second_half.to_vec()).await;
            result.expect("peer: write second half failed");
        });

        let mut session = connect_session(addr).await;
        let mut response = Vec::new();
        let disposition = session
            .execute(b"FA;", &mut response)
            .await
            .expect("execute should succeed across partial reads");

        assert_eq!(disposition, ResponseDisposition::ResponseWritten);
        assert_eq!(response, expected_response);
    }

    #[monoio::test(driver = "legacy")]
    async fn rejects_oversized_frame_without_reading_payload() {
        let (listener, addr) = bind_loopback();

        monoio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept failed");
            let _req = read_raw_frame(&mut stream)
                .await
                .expect("peer: read request frame failed");

            // Declare a frame far larger than MAX_FRAME_SIZE, but never
            // actually send that much payload -- if the client tried to
            // read the declared length before rejecting it, this test
            // would hang forever instead of failing fast.
            let oversized_len: u32 = MAX_FRAME_SIZE + 1;
            let (result, _) = stream.write_all(oversized_len.to_be_bytes().to_vec()).await;
            result.expect("peer: write oversized length prefix failed");
            // Deliberately send no payload bytes at all, then let `stream`
            // drop (closing the connection) at the end of this task. This
            // is safe to race against the client's read: TCP preserves byte
            // order across a close, so the 4 already-written prefix bytes
            // are guaranteed to arrive before EOF regardless of exactly
            // when the client gets around to reading them.
        });

        let mut session = connect_session(addr).await;
        let mut response = Vec::new();
        let result = session.execute(b"FA;", &mut response).await;

        match result {
            Err(TcpSessionError::FrameTooLarge { len, max }) => {
                assert_eq!(len, MAX_FRAME_SIZE + 1);
                assert_eq!(max, MAX_FRAME_SIZE);
            }
            other => panic!("expected FrameTooLarge, got {:?}", other),
        }
        assert!(response.is_empty());
    }

    #[monoio::test(driver = "legacy")]
    async fn disconnect_mid_length_prefix_returns_error_not_hang() {
        let (listener, addr) = bind_loopback();

        monoio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept failed");
            let _req = read_raw_frame(&mut stream)
                .await
                .expect("peer: read request frame failed");

            // Send only 2 of the 4 length-prefix bytes, then close.
            let (result, _) = stream.write_all(vec![0u8, 0u8]).await;
            result.expect("peer: partial prefix write failed");
            drop(stream);
        });

        let mut session = connect_session(addr).await;
        let mut response = Vec::new();
        let result = session.execute(b"FA;", &mut response).await;

        match result {
            Err(TcpSessionError::Io(e)) => {
                assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
            }
            other => panic!("expected Io(UnexpectedEof), got {:?}", other),
        }
    }

    #[monoio::test(driver = "legacy")]
    async fn disconnect_mid_payload_returns_error_not_hang() {
        let (listener, addr) = bind_loopback();

        monoio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept failed");
            let _req = read_raw_frame(&mut stream)
                .await
                .expect("peer: read request frame failed");

            // Declare a 14-byte payload, but only send 5 bytes of it, then
            // close.
            let declared_len: u32 = 14;
            let (result, _) = stream.write_all(declared_len.to_be_bytes().to_vec()).await;
            result.expect("peer: write length prefix failed");
            let (result, _) = stream.write_all(b"FA000".to_vec()).await;
            result.expect("peer: partial payload write failed");
            drop(stream);
        });

        let mut session = connect_session(addr).await;
        let mut response = Vec::new();
        let result = session.execute(b"FA;", &mut response).await;

        match result {
            Err(TcpSessionError::Io(e)) => {
                assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
            }
            other => panic!("expected Io(UnexpectedEof), got {:?}", other),
        }
    }
}
