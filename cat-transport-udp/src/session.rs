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

//! [`UdpCatSession`]: a [`CatSession`] implementation over
//! `monoio::net::udp::UdpSocket`, using an envelope format plus a
//! deduplication cache.
//!
//! # Why this is a different design from `cat-transport-tcp`, not a copy
//!
//! TCP is a connection-oriented, ordered, reliable byte *stream* with no
//! message boundaries of its own -- `cat-transport-tcp::TcpCatSession` has
//! to invent one (a length prefix). UDP is the opposite on every axis: it is
//! connectionless, gives no ordering or delivery guarantee, but *does*
//! preserve datagram (message) boundaries for free. This session type is
//! designed against those actual properties instead of forcing UDP to
//! pretend it has a TCP-shaped connection:
//!
//! - No length field in the envelope -- a UDP datagram already arrives (or
//!   doesn't) as one complete unit; whatever bytes remain after the fixed
//!   header *are* the payload.
//! - No assumption that a peer never repeats itself or always answers in
//!   order -- an explicit `session_id`/`request_id` pair plus a
//!   deduplication cache handle duplicate and stale/reordered deliveries.
//! - An explicit, bounded response timeout owned by this session type
//!   itself -- unlike `TcpCatSession` (which deliberately has none; see its
//!   module docs), plain UDP gives no OS-level signal at all when a peer
//!   goes silent (no FIN/EOF exists for a connectionless socket), so without
//!   a timeout here `execute()` could hang forever with no other layer able
//!   to detect it.
//!
//! # Wire format: the envelope
//!
//! Every datagram (request *or* response) sent over this transport is one
//! **envelope**:
//!
//! ```text
//! +-------------------+-------------------+----------------------------+
//! | session_id        | request_id        | payload (remaining bytes)  |
//! | 8 bytes, u64 BE    | 8 bytes, u64 BE    | raw request/response bytes |
//! +-------------------+-------------------+----------------------------+
//! ```
//!
//! - **Header width**: 16 bytes total (`session_id` 8 bytes + `request_id` 8
//!   bytes), both unsigned 64-bit integers in **big-endian** (network) byte
//!   order -- same endianness convention as `cat-transport-tcp`'s length
//!   prefix, for terminology/style parity across the two crates.
//! - **`session_id`** identifies one logical `UdpCatSession` instance. It is
//!   randomized once per session (not sequential, not derived from the
//!   socket address) and stays constant for the session's lifetime. Its
//!   purpose is to let a receiver (this session, or a future `cat-server`
//!   UDP listener serving many clients on one socket) distinguish traffic
//!   belonging to *this* logical session from anything else that might
//!   arrive on the same local port -- a stale packet from a previous
//!   session that reused the port, unrelated traffic, or (server-side)
//!   packets belonging to a different client session entirely.
//! - **`request_id`** identifies one request *within* a session. It is a
//!   per-session counter starting at 1 and incrementing by exactly 1 for
//!   every `execute()`/`send()` call (never reset, never reused). Its
//!   purpose is (a) correlating a response datagram with the specific
//!   request it answers, and (b) the deduplication cache's key (see below).
//! - **No length field.** Unlike `cat-transport-tcp`'s length-prefixed
//!   frame, this envelope does not encode the payload's length anywhere.
//!   This is deliberate, not an oversight: a UDP datagram is a single
//!   message with its own boundary already established by `recv_from` --
//!   the payload is simply "whatever bytes came back after the 16-byte
//!   header in *this* datagram." There is no reassembly across multiple
//!   reads (there is nothing to reassemble; a datagram either arrives whole
//!   or not at all).
//! - **Payload contents**: identical convention to `cat-transport-tcp` --
//!   the raw CAT command/response bytes exactly as they would appear on the
//!   wire for serial (e.g. `b"FA;"` or `b"FA00014250000;"`), with no
//!   additional wrapping or terminator added by this layer.
//! - **Zero-length payload is valid and meaningful**: a datagram containing
//!   exactly the 16-byte header and nothing else represents
//!   [`ResponseDisposition::NoResponse`] -- same convention as
//!   `cat-transport-tcp` and `cat-transport-core::ScriptedCatSession`.
//! - **[`MAX_PAYLOAD_SIZE`]** (1024 bytes) bounds the payload on the send
//!   side ([`UdpSessionError::PayloadTooLarge`] is returned before anything
//!   is sent if exceeded). This is deliberately much smaller than TCP's 64
//!   KiB cap: a UDP datagram larger than the path MTU (typically ~1472
//!   payload bytes after Ethernet/IP/UDP headers) risks IP fragmentation,
//!   and a fragmented datagram is dropped in full if even one fragment is
//!   lost -- there is no partial-retransmission the way TCP can recover a
//!   dropped segment. 1024 bytes (1040 with the header) stays comfortably
//!   under common MTU limits while leaving an order of magnitude of
//!   headroom over known CAT frame sizes (tens of bytes).
//! - **Receive-side sizing, and its limitation**: this session allocates a
//!   fixed `ENVELOPE_HEADER_LEN + MAX_PAYLOAD_SIZE`-byte buffer for each
//!   `recv_from`. A well-behaved peer (bounded by the same
//!   `MAX_PAYLOAD_SIZE` on its own send side) never exceeds this. Unlike
//!   TCP's length prefix, which lets a reader reject an oversized frame
//!   *before* reading its payload, UDP gives no such preview: a peer that
//!   sends an oversized datagram anyway will have it silently truncated by
//!   the kernel to the receive buffer's size. In practice a truncated
//!   datagram will very likely fail envelope decoding or `session_id`/
//!   `request_id` matching and simply be discarded as noise (see below) --
//!   but this is a known, documented limitation of message-oriented I/O,
//!   not a hard guarantee equivalent to TCP's `FrameTooLarge` rejection.
//!
//! # Request/response pairing: not connection-ordered, explicitly correlated
//!
//! `execute()` sends exactly one request envelope (with a freshly
//! incremented `request_id`) to a single, fixed `peer_addr` configured when
//! the session was constructed, then waits for a response envelope. Because
//! UDP does not guarantee the response (if any) arrives, arrives once, or
//! arrives before some *other* stray datagram, every datagram received while
//! waiting is filtered before being accepted as *the* answer:
//!
//! 1. **Source address** must equal `peer_addr` (an application-level check
//!    performed on every received datagram -- this session never calls
//!    `UdpSocket::connect`, so the kernel does not filter by peer for us;
//!    see "Why not `UdpSocket::connect`?" below).
//! 2. **`session_id`** in the envelope must equal this session's own
//!    `session_id`.
//! 3. **`request_id`** in the envelope must equal the `request_id` of the
//!    request just sent.
//!
//! A datagram failing any check is discarded and the wait continues (bounded
//! by the overall timeout described next) -- it is never surfaced as an
//! error and never mistaken for the real answer.
//!
//! ## Why not `UdpSocket::connect`?
//!
//! `monoio::net::udp::UdpSocket` (like most UDP APIs) supports an optional
//! `connect()` that filters incoming datagrams by source address at the
//! kernel level and allows `send`/`recv` instead of `send_to`/`recv_from`.
//! This session deliberately does **not** use it. `connect()` on a UDP
//! socket is purely a local addressing convenience -- it adds no ordering,
//! delivery, or acknowledgement semantics -- but per this crate's charter
//! ("do not force connection-oriented semantics onto UDP"), the address
//! filter is kept at the application layer (the explicit `from != peer_addr`
//! check above) alongside the `session_id`/`request_id` checks that do the
//! real work, rather than mixing an OS-level filter with an application-level
//! one for the same concern.
//!
//! # Deduplication cache
//!
//! Every `UdpCatSession` keeps a bounded FIFO cache
//! ([`DEDUP_CACHE_CAPACITY`] = 32 entries) of `request_id`s it has already
//! completed (i.e. successfully matched a response for). When a received
//! envelope's `session_id` matches but its `request_id` does *not* match the
//! request currently being waited on, this cache is consulted to classify
//! *why*:
//!
//! - If the `request_id` is in the cache, it is an explicit, recognized
//!   **duplicate** -- a response this session already accepted for an
//!   earlier request, redelivered (network-level duplication, or a peer
//!   retransmitting because it didn't think its first reply arrived).
//! - If it is not in the cache, it is an **unrecognized/stale** id -- older
//!   than the cache's retention window, or foreign traffic that happens to
//!   share this session's `session_id` space (astronomically unlikely, but
//!   not assumed impossible).
//!
//! Both cases are discarded identically (the wait loop continues) --
//! classifying them is about making the discard *explicit and intentional*,
//! not incidental. **Honesty about what this buys, given `request_id` is
//! strictly monotonic and never reused within a session's lifetime: the
//! plain "does not match the request I'm currently waiting for" check above
//! is, by itself, already sufficient for correctness.** The cache does not
//! change what gets accepted or rejected today. It is kept because (a) the
//! project charter (`.claude/agents/cat_transport.md`) names a deduplication
//! cache as a required design element of this transport, (b) it gives future
//! maintainers/tests a named, directly-inspectable mechanism instead of an
//! implicit side effect of integer comparison, and (c) it is the natural
//! place to bound memory growth over a long-lived session's lifetime rather
//! than growing an unbounded history.
//!
//! **Key**: `request_id: u64` alone (not `(session_id, request_id)`) --
//! scoping by `session_id` is unnecessary here because a single
//! `UdpCatSession` instance only ever has one, constant `session_id` for its
//! entire lifetime; the cache is inherently already scoped to "this
//! session's own completed requests."
//!
//! **Eviction policy**: FIFO, bounded at [`DEDUP_CACHE_CAPACITY`] (32)
//! entries -- oldest completed `request_id` evicted first once the cache is
//! full. 32 is a judgment call: since `request_id`s are monotonically
//! increasing and never repeat, a duplicate/stale datagram arriving more
//! than 32 requests after the one it duplicates is not a realistic scenario
//! for any plausible network delay or duplication behavior; this bounds the
//! cache's memory to a small constant regardless of how long the session
//! lives or how many requests it issues over its lifetime.
//!
//! # Response timeout
//!
//! `response_timeout: Duration`, supplied at construction, bounds each
//! `execute()` call as a **single fixed deadline**
//! (`monoio::time::Instant::now() + response_timeout`, computed once at the
//! start of `execute()`), not a duration re-applied on every loop iteration.
//! This distinction matters: if the deadline were recomputed (or the
//! duration re-applied) every time an irrelevant datagram was discarded, a
//! peer or network condition that kept delivering noise could extend the
//! wait indefinitely, defeating the bound entirely. A single fixed deadline
//! guarantees `execute()` returns (successfully or with
//! [`UdpSessionError::Timeout`]) within `response_timeout` of when the
//! request was sent, regardless of how much irrelevant traffic arrives in
//! the meantime.
//!
//! **This requires the consuming application's monoio runtime to be built
//! with its timer driver enabled** (`RuntimeBuilder::enable_timer()`, or
//! `#[monoio::main(timer_enabled = true)]` for a binary's entry point) --
//! `monoio::time::timeout_at` (used internally) does nothing useful without
//! it. This is a real, load-bearing requirement on any binary that
//! constructs a `UdpCatSession`, not an optional nicety.
//!
//! There is no hard requirement analogous to `cat-transport-tcp`'s "every
//! request gets exactly one response, even empty" placed on a peer here --
//! a well-behaved peer over UDP still SHOULD answer every request with
//! exactly one response envelope (empty payload for `NoResponse`, mirroring
//! TCP's convention), but because UDP cannot guarantee delivery in either
//! direction, this session cannot *require* it the way `TcpCatSession` can
//! lean on TCP's reliability -- hence the timeout above as the actual
//! backstop instead of an assumed guarantee.
//!
//! # Reusable codec primitives
//!
//! [`encode_envelope`] and [`decode_envelope`] are `pub` so a server-side
//! peer implementing this same envelope format (e.g. `cat-server`'s UDP
//! listener, which both decodes incoming request envelopes and encodes
//! outgoing response envelopes) can import and call them directly instead
//! of hand-rolling a second copy that could drift out of sync. Their
//! signatures are unchanged from this crate's internal use -- only
//! visibility moved.

use std::net::SocketAddr;
use std::time::Duration;

use async_trait::async_trait;
use monoio::net::udp::UdpSocket;

use cat_transport_core::{CatSession, ResponseDisposition};

use crate::codec::{
    self, decode_envelope, encode_envelope, RequestIdCache, UdpSessionError,
    DEFAULT_RESPONSE_TIMEOUT, MAX_DATAGRAM_SIZE,
};

#[cfg(test)]
use crate::codec::{DEDUP_CACHE_CAPACITY, ENVELOPE_HEADER_LEN, MAX_PAYLOAD_SIZE};

/// [`CatSession`] backed by a `monoio::net::udp::UdpSocket`, using an
/// envelope-framed, deduplicated request/response protocol.
///
/// See the module-level docs for the exact wire format, the deduplication
/// cache's key/eviction policy, and the response-timeout requirement. Like
/// `cat-transport-tcp::TcpCatSession`, this type wraps its socket type
/// directly rather than being generic over `cat_transport_core::Transport`
/// -- envelope I/O needs whole-datagram `send_to`/`recv_from` semantics, not
/// `Transport`'s borrowed-slice `read`/`write` shape.
pub struct UdpCatSession {
    socket: UdpSocket,
    peer_addr: SocketAddr,
    session_id: u64,
    next_request_id: u64,
    dedup_cache: RequestIdCache,
    response_timeout: Duration,
}

impl UdpCatSession {
    /// Wrap an already-bound `UdpSocket` in a session that always talks to
    /// `peer_addr`, using `response_timeout` as the bound on every
    /// `execute()` call. The session's `session_id` is randomized
    /// internally.
    pub fn new(socket: UdpSocket, peer_addr: SocketAddr, response_timeout: Duration) -> Self {
        Self {
            socket,
            peer_addr,
            session_id: codec::random_session_id(),
            next_request_id: 0,
            dedup_cache: RequestIdCache::new(),
            response_timeout,
        }
    }

    /// Bind a fresh ephemeral local socket and wrap it in a session talking
    /// to `peer_addr`, using [`DEFAULT_RESPONSE_TIMEOUT`].
    ///
    /// Named `bind_to` rather than `connect` (unlike
    /// `TcpCatSession::connect`) deliberately: no kernel-level UDP
    /// `connect()` happens here (see the module docs' "Why not
    /// `UdpSocket::connect`?" section) -- this only binds a local socket and
    /// remembers `peer_addr` as the fixed application-level destination.
    /// Calling it `connect` would misleadingly imply a handshake or
    /// kernel-level peer filter that does not exist for this transport.
    pub fn bind_to(peer_addr: SocketAddr) -> Result<Self, UdpSessionError> {
        Self::bind_to_with_timeout(peer_addr, DEFAULT_RESPONSE_TIMEOUT)
    }

    /// Like [`bind_to`](Self::bind_to), with an explicit `response_timeout`
    /// instead of [`DEFAULT_RESPONSE_TIMEOUT`].
    pub fn bind_to_with_timeout(
        peer_addr: SocketAddr,
        response_timeout: Duration,
    ) -> Result<Self, UdpSessionError> {
        let bind_addr: SocketAddr = if peer_addr.is_ipv6() {
            "[::]:0".parse().expect("valid IPv6 wildcard address")
        } else {
            "0.0.0.0:0".parse().expect("valid IPv4 wildcard address")
        };
        let socket = UdpSocket::bind(bind_addr)?;
        Ok(Self::new(socket, peer_addr, response_timeout))
    }

    /// This session's randomized session id.
    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    /// The local address this session's socket is bound to.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Record `request_id` as completed in the dedup cache, evicting the
    /// oldest entry first if already at [`DEDUP_CACHE_CAPACITY`].
    fn remember_completed(&mut self, request_id: u64) {
        self.dedup_cache.remember_completed(request_id);
    }

    /// `true` if `request_id` is one this session has already completed --
    /// used only to classify a mismatched datagram as an explicit,
    /// recognized duplicate for documentation/testability; either way a
    /// mismatch is discarded (see module docs).
    fn is_known_duplicate(&self, request_id: u64) -> bool {
        self.dedup_cache.is_known_duplicate(request_id)
    }
}

#[async_trait(?Send)]
impl CatSession for UdpCatSession {
    type Error = UdpSessionError;

    async fn execute(
        &mut self,
        request: &[u8],
        response: &mut Vec<u8>,
    ) -> Result<ResponseDisposition, UdpSessionError> {
        self.next_request_id = self.next_request_id.wrapping_add(1);
        let request_id = self.next_request_id;

        let datagram = encode_envelope(self.session_id, request_id, request)?;
        let (result, _buf) = self.socket.send_to(datagram, self.peer_addr).await;
        result?;

        // A single fixed deadline for the whole wait -- NOT re-applied per
        // loop iteration. See the module docs' "Response timeout" section
        // for why this distinction is load-bearing.
        let deadline = monoio::time::Instant::now() + self.response_timeout;

        loop {
            let buf = vec![0u8; MAX_DATAGRAM_SIZE];
            let outcome = monoio::time::timeout_at(deadline, self.socket.recv_from(buf)).await;

            let (n, from, buf) = match outcome {
                Ok((Ok((n, from)), buf)) => (n, from, buf),
                Ok((Err(io_err), _buf)) => return Err(UdpSessionError::Io(io_err)),
                Err(_elapsed) => {
                    return Err(UdpSessionError::Timeout {
                        peer: self.peer_addr,
                        timeout: self.response_timeout,
                    });
                }
            };

            if from != self.peer_addr {
                // Datagram from an address other than our configured peer
                // -- not connection-filtered at the kernel level (see
                // module docs), so this application-level check does that
                // job instead. Keep waiting.
                continue;
            }

            let Some((incoming_session_id, incoming_request_id, payload)) =
                decode_envelope(&buf[..n])
            else {
                // Too short to be a valid envelope -- noise, not an error.
                continue;
            };

            if incoming_session_id != self.session_id {
                // Belongs to a different logical session sharing this
                // source address (or this port, in a server-side reuse
                // scenario) -- ignore.
                continue;
            }

            if incoming_request_id != request_id {
                // Mismatched request id: classify (see module docs'
                // "Deduplication cache" section) but discard identically
                // either way.
                let _is_duplicate = self.is_known_duplicate(incoming_request_id);
                continue;
            }

            self.remember_completed(request_id);

            let disposition = if payload.is_empty() {
                ResponseDisposition::NoResponse
            } else {
                ResponseDisposition::ResponseWritten
            };
            response.extend_from_slice(payload);
            return Ok(disposition);
        }
    }

    // `send` is NOT overridden -- the default (forward to `execute`, discard
    // the response) is used, same as `TcpCatSession`. Unlike
    // `SerialCatSession` (which overrides `send` because the real radio
    // never answers set commands over serial at all), a well-behaved UDP
    // peer is still expected to answer every request with an envelope
    // (empty payload for `NoResponse`) -- the `response_timeout` bound
    // above is what protects a fire-and-forget-shaped call from hanging if
    // that expectation doesn't hold, not an override here.
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_transport_core::conformance;
    use std::time::Instant as StdInstant;

    const TEST_TIMEOUT: Duration = Duration::from_millis(200);

    /// Bind an ephemeral loopback `UdpSocket` for a test peer.
    fn bind_loopback_socket() -> (UdpSocket, SocketAddr) {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("UdpSocket::bind failed");
        let addr = socket.local_addr().expect("local_addr failed");
        (socket, addr)
    }

    /// Encode one raw envelope directly, independent of [`encode_envelope`]
    /// -- deliberately hand-rolled so the test suite cross-checks the
    /// *documented* wire format against the production implementation,
    /// mirroring `cat-transport-tcp`'s test-module convention.
    fn raw_envelope(session_id: u64, request_id: u64, payload: &[u8]) -> Vec<u8> {
        let mut datagram = Vec::with_capacity(16 + payload.len());
        datagram.extend_from_slice(&session_id.to_be_bytes());
        datagram.extend_from_slice(&request_id.to_be_bytes());
        datagram.extend_from_slice(payload);
        datagram
    }

    /// Decode one raw envelope directly, independent of [`decode_envelope`].
    fn parse_raw_envelope(datagram: &[u8]) -> (u64, u64, Vec<u8>) {
        assert!(
            datagram.len() >= 16,
            "datagram shorter than envelope header"
        );
        let session_id = u64::from_be_bytes(datagram[0..8].try_into().unwrap());
        let request_id = u64::from_be_bytes(datagram[8..16].try_into().unwrap());
        (session_id, request_id, datagram[16..].to_vec())
    }

    async fn peer_recv_request(peer: &UdpSocket) -> (u64, u64, Vec<u8>, SocketAddr) {
        let buf = vec![0u8; MAX_DATAGRAM_SIZE];
        let (result, buf) = peer.recv_from(buf).await;
        let (n, from) = result.expect("peer: recv_from failed");
        let (session_id, request_id, payload) = parse_raw_envelope(&buf[..n]);
        (session_id, request_id, payload, from)
    }

    async fn peer_send_response(
        peer: &UdpSocket,
        to: SocketAddr,
        session_id: u64,
        request_id: u64,
        payload: &[u8],
    ) {
        let datagram = raw_envelope(session_id, request_id, payload);
        let (result, _buf) = peer.send_to(datagram, to).await;
        result.expect("peer: send_to failed");
    }

    // -----------------------------------------------------------------------
    // conformance harness -- reused unchanged from cat-transport-core,
    // exercised here against a real loopback UdpCatSession.
    // -----------------------------------------------------------------------

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn conformance_query_round_trip() {
        let (peer, peer_addr) = bind_loopback_socket();
        let mut session =
            UdpCatSession::bind_to_with_timeout(peer_addr, TEST_TIMEOUT).expect("bind_to failed");

        monoio::spawn(async move {
            let (session_id, request_id, payload, from) = peer_recv_request(&peer).await;
            assert_eq!(payload, b"ID;");
            peer_send_response(&peer, from, session_id, request_id, b"ID017;").await;
        });

        conformance::query_round_trip(&mut session, b"ID;", b"ID017;").await;
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn conformance_set_is_fire_and_forget() {
        let (peer, peer_addr) = bind_loopback_socket();
        let mut session =
            UdpCatSession::bind_to_with_timeout(peer_addr, TEST_TIMEOUT).expect("bind_to failed");

        monoio::spawn(async move {
            let (session_id, request_id, payload, from) = peer_recv_request(&peer).await;
            assert_eq!(payload, b"TX;");
            // Fire-and-forget still gets an explicit empty response
            // envelope -- required by this protocol's convention (see
            // module docs), mirroring TCP's equivalent test.
            peer_send_response(&peer, from, session_id, request_id, b"").await;
        });

        conformance::set_is_fire_and_forget(&mut session, b"TX;").await;
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn conformance_surfaces_transport_error() {
        let (peer, peer_addr) = bind_loopback_socket();
        let mut session =
            UdpCatSession::bind_to_with_timeout(peer_addr, TEST_TIMEOUT).expect("bind_to failed");

        // The peer receives the request but never responds -- UDP's
        // equivalent of TCP's "peer disconnects" surfaced-error scenario,
        // since UDP has no connection to drop; the session's own bounded
        // wait is what turns silence into an error.
        monoio::spawn(async move {
            let _ = peer_recv_request(&peer).await;
        });

        conformance::surfaces_transport_error(&mut session, b"FA;").await;
    }

    // -----------------------------------------------------------------------
    // UDP-specific tests: duplicate delivery, out-of-order/stale delivery,
    // never-answered bounded wait, malformed/foreign datagrams ignored.
    // -----------------------------------------------------------------------

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn duplicate_response_delivery_does_not_corrupt_next_request() {
        let (peer, peer_addr) = bind_loopback_socket();
        let mut session =
            UdpCatSession::bind_to_with_timeout(peer_addr, TEST_TIMEOUT).expect("bind_to failed");

        monoio::spawn(async move {
            // Request 1: respond correctly, TWICE in a row (network-level
            // duplication of the same response).
            let (session_id, request_id, payload, from) = peer_recv_request(&peer).await;
            assert_eq!(payload, b"FA;");
            peer_send_response(&peer, from, session_id, request_id, b"FA00014250000;").await;
            peer_send_response(&peer, from, session_id, request_id, b"FA00014250000;").await;

            // Request 2: respond once, correctly. The leftover duplicate
            // from request 1 (still unread in the session's socket buffer)
            // must not be mistaken for this answer.
            let (session_id, request_id, payload, from) = peer_recv_request(&peer).await;
            assert_eq!(payload, b"FB;");
            peer_send_response(&peer, from, session_id, request_id, b"FB00014250000;").await;
        });

        let mut response = Vec::new();
        let disposition = session
            .execute(b"FA;", &mut response)
            .await
            .expect("execute(1) should succeed");
        assert_eq!(disposition, ResponseDisposition::ResponseWritten);
        assert_eq!(response, b"FA00014250000;");

        response.clear();
        let disposition = session
            .execute(b"FB;", &mut response)
            .await
            .expect("execute(2) should succeed despite a leftover duplicate");
        assert_eq!(disposition, ResponseDisposition::ResponseWritten);
        assert_eq!(
            response, b"FB00014250000;",
            "must not double-respond with the stale duplicate from request 1"
        );
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn stale_response_for_older_request_is_not_misattributed_to_newer_request() {
        let (peer, peer_addr) = bind_loopback_socket();
        let mut session =
            UdpCatSession::bind_to_with_timeout(peer_addr, TEST_TIMEOUT).expect("bind_to failed");

        monoio::spawn(async move {
            // Request 1: answer normally so execute(1) returns.
            let (session_id, request_id_1, payload, from) = peer_recv_request(&peer).await;
            assert_eq!(payload, b"FA;");
            peer_send_response(&peer, from, session_id, request_id_1, b"FA00014250000;").await;

            // Request 2 arrives (session has since moved on). Before
            // answering it, re-send a STALE/reordered copy of request 1's
            // response -- simulating a late, reordered delivery arriving
            // only after a newer request is already outstanding.
            let (session_id, request_id_2, payload, from) = peer_recv_request(&peer).await;
            assert_eq!(payload, b"FB;");
            peer_send_response(&peer, from, session_id, request_id_1, b"FA00014250000;").await;
            peer_send_response(&peer, from, session_id, request_id_2, b"FB00014250000;").await;
        });

        let mut response = Vec::new();
        session
            .execute(b"FA;", &mut response)
            .await
            .expect("execute(1) should succeed");
        assert_eq!(response, b"FA00014250000;");

        response.clear();
        let disposition = session
            .execute(b"FB;", &mut response)
            .await
            .expect("execute(2) should succeed despite the reordered stale reply");
        assert_eq!(disposition, ResponseDisposition::ResponseWritten);
        assert_eq!(
            response, b"FB00014250000;",
            "the stale response for the OLDER request id must not be misattributed \
             to the newer request"
        );
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn never_answered_request_times_out_instead_of_hanging() {
        let (peer, peer_addr) = bind_loopback_socket();
        let mut session =
            UdpCatSession::bind_to_with_timeout(peer_addr, TEST_TIMEOUT).expect("bind_to failed");

        monoio::spawn(async move {
            // Receive the request, then do absolutely nothing -- no
            // response, ever.
            let _ = peer_recv_request(&peer).await;
        });

        let mut response = Vec::new();
        let started = StdInstant::now();
        let result = session.execute(b"FA;", &mut response).await;
        let elapsed = started.elapsed();

        match result {
            Err(UdpSessionError::Timeout { peer, timeout }) => {
                assert_eq!(peer, peer_addr);
                assert_eq!(timeout, TEST_TIMEOUT);
            }
            other => panic!("expected Timeout, got {:?}", other),
        }
        assert!(response.is_empty());

        // Bounded wait, not a hang: elapsed time is at least the configured
        // timeout, and well under a generous multiple of it -- proving
        // execute() returned because the deadline fired, not because of
        // some unrelated delay, and that it did not wait indefinitely.
        assert!(
            elapsed >= TEST_TIMEOUT,
            "returned before the configured timeout elapsed: {:?}",
            elapsed
        );
        assert!(
            elapsed < TEST_TIMEOUT * 10,
            "took far longer than the configured timeout, looks like a near-hang: {:?}",
            elapsed
        );
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn ignores_malformed_short_datagram_and_still_receives_real_response() {
        let (peer, peer_addr) = bind_loopback_socket();
        let mut session =
            UdpCatSession::bind_to_with_timeout(peer_addr, TEST_TIMEOUT).expect("bind_to failed");

        monoio::spawn(async move {
            let (session_id, request_id, payload, from) = peer_recv_request(&peer).await;
            assert_eq!(payload, b"FA;");

            // Garbage datagram, too short to contain a full 16-byte header.
            let (result, _buf) = peer.send_to(vec![0xAAu8, 0xBB, 0xCC], from).await;
            result.expect("peer: garbage send_to failed");

            peer_send_response(&peer, from, session_id, request_id, b"FA00014250000;").await;
        });

        let mut response = Vec::new();
        let disposition = session
            .execute(b"FA;", &mut response)
            .await
            .expect("execute should succeed despite leading garbage datagram");
        assert_eq!(disposition, ResponseDisposition::ResponseWritten);
        assert_eq!(response, b"FA00014250000;");
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn ignores_datagram_with_foreign_session_id() {
        let (peer, peer_addr) = bind_loopback_socket();
        let mut session =
            UdpCatSession::bind_to_with_timeout(peer_addr, TEST_TIMEOUT).expect("bind_to failed");

        monoio::spawn(async move {
            let (session_id, request_id, payload, from) = peer_recv_request(&peer).await;
            assert_eq!(payload, b"FA;");

            // A well-formed envelope with the correct request_id, but a
            // WRONG session_id -- simulating crosstalk from an unrelated
            // session -- must be ignored, not accepted as the answer.
            let foreign_session_id = session_id.wrapping_add(1).wrapping_add(0xDEAD_BEEF);
            peer_send_response(
                &peer,
                from,
                foreign_session_id,
                request_id,
                b"WRONG_SESSION;",
            )
            .await;

            peer_send_response(&peer, from, session_id, request_id, b"FA00014250000;").await;
        });

        let mut response = Vec::new();
        let disposition = session
            .execute(b"FA;", &mut response)
            .await
            .expect("execute should succeed, ignoring the foreign-session-id datagram");
        assert_eq!(disposition, ResponseDisposition::ResponseWritten);
        assert_eq!(response, b"FA00014250000;");
    }

    // -----------------------------------------------------------------------
    // Pure-logic unit tests: dedup cache and encode/decode. `encode_envelope`
    // / `decode_envelope` need no runtime at all (plain `#[test]`). The
    // dedup-cache and session-id tests construct a real `UdpCatSession`,
    // which needs a real bound `UdpSocket` -- `UdpSocket::bind` touches
    // monoio's internal driver thread-local even though it does no actual
    // I/O, so those run under `#[monoio::test]` even though no datagram is
    // ever sent or received in them.
    // -----------------------------------------------------------------------

    #[test]
    fn encode_decode_round_trip() {
        let datagram = encode_envelope(0x0102_0304_0506_0708, 42, b"FA;").unwrap();
        assert_eq!(datagram.len(), ENVELOPE_HEADER_LEN + 3);

        let (session_id, request_id, payload) = decode_envelope(&datagram).unwrap();
        assert_eq!(session_id, 0x0102_0304_0506_0708);
        assert_eq!(request_id, 42);
        assert_eq!(payload, b"FA;");
    }

    #[test]
    fn encode_rejects_oversized_payload() {
        let oversized = vec![0u8; MAX_PAYLOAD_SIZE + 1];
        let result = encode_envelope(1, 1, &oversized);
        match result {
            Err(UdpSessionError::PayloadTooLarge { len, max }) => {
                assert_eq!(len, MAX_PAYLOAD_SIZE + 1);
                assert_eq!(max, MAX_PAYLOAD_SIZE);
            }
            other => panic!("expected PayloadTooLarge, got {:?}", other),
        }
    }

    #[test]
    fn decode_rejects_datagram_shorter_than_header() {
        assert!(decode_envelope(&[0u8; ENVELOPE_HEADER_LEN - 1]).is_none());
        assert!(decode_envelope(&[]).is_none());
    }

    #[test]
    fn decode_accepts_header_only_datagram_as_empty_payload() {
        let datagram = encode_envelope(7, 8, b"").unwrap();
        let (session_id, request_id, payload) = decode_envelope(&datagram).unwrap();
        assert_eq!(session_id, 7);
        assert_eq!(request_id, 8);
        assert!(payload.is_empty());
    }

    #[monoio::test(driver = "legacy")]
    async fn dedup_cache_recognizes_a_completed_request_id() {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("bind failed");
        let peer_addr: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let mut session = UdpCatSession::new(socket, peer_addr, DEFAULT_RESPONSE_TIMEOUT);

        assert!(!session.is_known_duplicate(5));
        session.remember_completed(5);
        assert!(session.is_known_duplicate(5));
        assert!(!session.is_known_duplicate(6));
    }

    #[monoio::test(driver = "legacy")]
    async fn dedup_cache_evicts_oldest_beyond_capacity() {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("bind failed");
        let peer_addr: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let mut session = UdpCatSession::new(socket, peer_addr, DEFAULT_RESPONSE_TIMEOUT);

        for id in 1..=DEDUP_CACHE_CAPACITY as u64 {
            session.remember_completed(id);
        }
        assert!(
            session.is_known_duplicate(1),
            "oldest entry not yet evicted"
        );

        // One more insertion should evict request id 1 (the oldest).
        session.remember_completed(DEDUP_CACHE_CAPACITY as u64 + 1);
        assert!(
            !session.is_known_duplicate(1),
            "oldest entry should have been evicted once capacity was exceeded"
        );
        assert!(
            session.is_known_duplicate(2),
            "second-oldest should survive"
        );
        assert!(session.is_known_duplicate(DEDUP_CACHE_CAPACITY as u64 + 1));
    }

    #[monoio::test(driver = "legacy")]
    async fn session_id_is_randomized_across_instances() {
        let socket_a = UdpSocket::bind("127.0.0.1:0").expect("bind failed");
        let socket_b = UdpSocket::bind("127.0.0.1:0").expect("bind failed");
        let peer_addr: SocketAddr = "127.0.0.1:1".parse().unwrap();

        let session_a = UdpCatSession::new(socket_a, peer_addr, DEFAULT_RESPONSE_TIMEOUT);
        let session_b = UdpCatSession::new(socket_b, peer_addr, DEFAULT_RESPONSE_TIMEOUT);

        assert_ne!(session_a.session_id(), session_b.session_id());
    }
}
