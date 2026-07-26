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

//! Windows `TcpCatSession` backend.
//!
//! Per `docs/adr/0006-windows-network-transport.md`: TCP sockets are
//! natively cross-platform in `std` (`std::net::TcpStream`), so this needs
//! no `windows-sys`/FFI at all -- unlike `cat-transport-serial`'s Windows
//! backend. What Windows genuinely lacks is `monoio` itself (no io_uring),
//! so `TcpCatSession::execute`'s `#[async_trait(?Send)]` method still needs
//! something to satisfy the `async fn` signature without a `monoio`
//! executor. This mirrors ADR 0004 §1's serial design exactly: a dedicated
//! background `std::thread` performs blocking I/O, and the `async fn` body
//! sends a request over it and `.await`s a
//! [`cat_transport_core::completion`] completion for the result --
//! genuinely suspending the calling task rather than blocking its thread.
//!
//! Unlike serial (which drives a generic `Transport` with separate
//! `read`/`write` primitives), `TcpCatSession` already owns its own framing
//! and does a whole request/response exchange as one unit inside
//! `execute()` -- so this backend's worker thread does the same: one
//! [`WorkerRequest`] per `execute()` call, covering the write and the read
//! together, rather than separate read/write round-trips through the
//! channel.
//!
//! # This module is genuinely cross-platform, and is tested on Linux too
//!
//! Unlike `cat-transport-serial`'s Windows backend (real Win32 FFI, which
//! cannot run outside a Windows target at all), everything in this file is
//! plain `std::net`/`std::thread`/`std::sync::mpsc` plus the portable
//! [`cat_transport_core::completion`] primitive -- nothing here is actually
//! Windows-specific. So this module is **not** `#[cfg(target_os =
//! "windows")]`-gated at all: it compiles, and its tests run, on every
//! platform this workspace builds for, giving this backend real executable
//! test coverage on ordinary Linux CI rather than relying solely on
//! `cargo check --target x86_64-pc-windows-gnu`. Only `lib.rs`'s top-level
//! `pub use` of `TcpCatSession` is `cfg`-gated, to select this module's type
//! as the crate's canonical one on Windows (where [`crate::session`]'s
//! `monoio`-based type does not exist) while [`crate::session`]'s type
//! remains canonical on Linux.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::thread;

use async_trait::async_trait;
use cat_transport_core::completion;
use cat_transport_core::{CatSession, ResponseDisposition};

use crate::codec::{self, TcpSessionError};

/// One request sent to [`TcpCatSession`]'s background worker thread: write
/// `payload` as one frame, then read exactly one response frame back,
/// replying with the decoded payload bytes (or the first I/O/framing
/// error encountered).
struct WorkerRequest {
    payload: Vec<u8>,
    reply: completion::CompletionTx<Result<Vec<u8>, TcpSessionError>>,
}

/// The worker thread body: owns `stream` for its whole lifetime, performing
/// one blocking write-then-read exchange per [`WorkerRequest`] received over
/// `rx`. Exits (ending the thread) when `rx.recv()` returns `Err` -- i.e.
/// [`TcpCatSession`]'s `Drop` impl has dropped the request sender.
fn worker_loop(mut stream: TcpStream, rx: mpsc::Receiver<WorkerRequest>) {
    while let Ok(request) = rx.recv() {
        let result = exchange(&mut stream, &request.payload);
        request.reply.send(result);
    }
}

/// Perform one blocking write-frame-then-read-frame exchange over `stream`.
fn exchange(stream: &mut TcpStream, payload: &[u8]) -> Result<Vec<u8>, TcpSessionError> {
    write_frame_blocking(stream, payload)?;
    read_frame_blocking(stream)
}

/// Write one complete length-prefixed frame, blocking until the whole frame
/// (or an error) has been written.
fn write_frame_blocking(stream: &mut TcpStream, payload: &[u8]) -> Result<(), TcpSessionError> {
    let frame = codec::encode_frame(payload)?;
    stream.write_all(&frame)?;
    Ok(())
}

/// Read one complete length-prefixed frame, blocking until the whole frame
/// (or an error/EOF) has arrived. A clean disconnect before any byte of the
/// length prefix arrives is reported as `Io(UnexpectedEof)` -- unlike
/// `crate::session::read_frame_or_eof`'s EOF-tolerant server-shaped
/// counterpart, `TcpCatSession` (client-shaped, see `crate::session`'s
/// module doc) always expects a frame in reply, mirroring `read_frame`'s
/// contract exactly.
fn read_frame_blocking(stream: &mut TcpStream) -> Result<Vec<u8>, TcpSessionError> {
    let mut len_buf = [0u8; 4];
    read_exact_blocking(stream, &mut len_buf)?;
    let len = codec::decode_len_prefix(len_buf)?;

    let mut payload = vec![0u8; len as usize];
    read_exact_blocking(stream, &mut payload)?;
    Ok(payload)
}

/// `Read::read_exact`, mapped onto [`TcpSessionError`]. A clean EOF
/// (`read` returning `Ok(0)`) at any point -- including before the first
/// byte -- surfaces as `io::ErrorKind::UnexpectedEof`, matching
/// `std::io::Read::read_exact`'s own documented behavior.
fn read_exact_blocking(stream: &mut TcpStream, buf: &mut [u8]) -> Result<(), TcpSessionError> {
    stream.read_exact(buf).map_err(TcpSessionError::from)
}

/// Build the [`TcpSessionError`] used when the worker thread is unexpectedly
/// gone -- either sending a [`WorkerRequest`] failed (the worker's
/// `mpsc::Receiver` was dropped, i.e. its thread already exited, most
/// plausibly via a panic, since [`TcpCatSession`]'s ordinary shutdown path
/// (`Drop`) always joins the thread before this could otherwise be
/// observed) or the paired `CompletionRx` resolved to
/// `Err(`[`completion::Canceled`]`)` (same underlying cause). Mirrors
/// `cat-transport-serial::windows::worker_gone_error` exactly.
fn worker_gone_error() -> TcpSessionError {
    TcpSessionError::Io(std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        "cat-transport-tcp: Windows worker thread is no longer running",
    ))
}

/// [`CatSession`] backed by a `std::net::TcpStream`, using the same
/// length-prefixed framing as the Linux (`monoio`) backend -- see
/// [`crate::session`]'s module doc for the exact wire format, which this
/// backend reuses unchanged via [`crate::codec`].
///
/// Same public API as the Linux backend (`new`, `connect`, `CatSession`)
/// so application code needs no platform branching to use it.
pub struct TcpCatSession {
    request_tx: Option<mpsc::Sender<WorkerRequest>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl TcpCatSession {
    /// Wrap an already-connected `TcpStream` in a session that performs
    /// length-prefixed frame I/O via a dedicated background worker thread.
    pub fn new(stream: TcpStream) -> Self {
        let (request_tx, request_rx) = mpsc::channel();
        let worker = thread::spawn(move || worker_loop(stream, request_rx));
        Self {
            request_tx: Some(request_tx),
            worker: Some(worker),
        }
    }

    /// Connect to `addr` and wrap the resulting stream in a new session.
    ///
    /// `async fn` for signature parity with the Linux backend (application
    /// code calls `.await` unconditionally on either platform); the body
    /// itself blocks briefly and synchronously on `std::net::TcpStream::
    /// connect` -- a deliberate, narrow exception mirroring
    /// `docs/adr/0004-windows-serial-backend.md` §3's identical
    /// `FlushFileBuffers`/`tcdrain` precedent ("brief blocking is acceptable
    /// in an async context where callers invoke this deliberately, once, at
    /// startup").
    pub async fn connect<A: ToSocketAddrs>(addr: A) -> Result<Self, TcpSessionError> {
        let stream = TcpStream::connect(addr)?;
        Ok(Self::new(stream))
    }

    fn request_tx(&self) -> &mpsc::Sender<WorkerRequest> {
        self.request_tx
            .as_ref()
            .expect("TcpCatSession::request_tx is only None after Drop::drop")
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
        let (tx, rx) = completion::channel();
        self.request_tx()
            .send(WorkerRequest {
                payload: request.to_vec(),
                reply: tx,
            })
            .map_err(|_| worker_gone_error())?;
        let payload = rx.await.map_err(|_| worker_gone_error())??;

        let disposition = if payload.is_empty() {
            ResponseDisposition::NoResponse
        } else {
            ResponseDisposition::ResponseWritten
        };
        response.extend_from_slice(&payload);
        Ok(disposition)
    }

    // `send` is deliberately NOT overridden -- see `crate::session`'s
    // module doc for why the default (forward to `execute`, discard the
    // response) is correct for this wire protocol on both platforms.
}

impl Drop for TcpCatSession {
    /// Drop the request sender (the worker's `recv()` then returns `Err`,
    /// ending its loop), then join the worker thread -- same shutdown
    /// sequencing as `cat-transport-serial::windows::SerialPort::drop`
    /// (dropping the sender before joining is load-bearing: joining first
    /// would hang forever with nothing left to wake `recv()`). The
    /// `TcpStream` itself is dropped (closing the socket) when the worker
    /// thread's stack unwinds after `worker_loop` returns.
    fn drop(&mut self) {
        self.request_tx.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::MAX_FRAME_SIZE;
    use cat_transport_core::conformance;
    use std::net::{SocketAddr, TcpListener};

    fn bind_loopback() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("TcpListener::bind failed");
        let addr = listener.local_addr().expect("local_addr failed");
        (listener, addr)
    }

    fn write_raw_frame(stream: &mut TcpStream, payload: &[u8]) {
        let frame = codec::encode_frame(payload).expect("encode_frame failed");
        stream.write_all(&frame).expect("write_all failed");
    }

    fn read_raw_frame(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let len = u32::from_be_bytes(len_buf);
        let mut payload = vec![0u8; len as usize];
        stream.read_exact(&mut payload)?;
        Ok(payload)
    }

    async fn connect_session(addr: SocketAddr) -> TcpCatSession {
        let stream = TcpStream::connect(addr).expect("TcpStream::connect failed");
        TcpCatSession::new(stream)
    }

    /// Minimal, single-threaded block_on for this test module only -- these
    /// tests don't need monoio (unavailable to a Windows target build
    /// anyway); a trivial thread-parking driver is sufficient to run one
    /// future at a time. Mirrors `cat-transport-serial::completion`'s own
    /// test-only `block_on_with_timeout` shape.
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake, Waker};

        struct ThreadWaker(std::thread::Thread);
        impl Wake for ThreadWaker {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
            fn wake_by_ref(self: &Arc<Self>) {
                self.0.unpark();
            }
        }

        let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut fut = Box::pin(fut);
        loop {
            match fut.as_mut().poll(&mut cx) {
                Poll::Ready(out) => return out,
                Poll::Pending => std::thread::park(),
            }
        }
    }

    #[test]
    fn conformance_query_round_trip() {
        let (listener, addr) = bind_loopback();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept failed");
            let req = read_raw_frame(&mut stream).expect("peer: read request frame failed");
            assert_eq!(req, b"ID;");
            write_raw_frame(&mut stream, b"ID017;");
        });

        block_on(async {
            let mut session = connect_session(addr).await;
            conformance::query_round_trip(&mut session, b"ID;", b"ID017;").await;
        });
        peer.join().unwrap();
    }

    #[test]
    fn conformance_set_is_fire_and_forget() {
        let (listener, addr) = bind_loopback();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept failed");
            let req = read_raw_frame(&mut stream).expect("peer: read request frame failed");
            assert_eq!(req, b"TX;");
            write_raw_frame(&mut stream, b"");
        });

        block_on(async {
            let mut session = connect_session(addr).await;
            conformance::set_is_fire_and_forget(&mut session, b"TX;").await;
        });
        peer.join().unwrap();
    }

    #[test]
    fn conformance_surfaces_transport_error() {
        let (listener, addr) = bind_loopback();
        let peer = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept failed");
            drop(stream);
        });

        block_on(async {
            let mut session = connect_session(addr).await;
            conformance::surfaces_transport_error(&mut session, b"FA;").await;
        });
        peer.join().unwrap();
    }

    #[test]
    fn rejects_oversized_frame_without_reading_payload() {
        let (listener, addr) = bind_loopback();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept failed");
            let _req = read_raw_frame(&mut stream).expect("peer: read request frame failed");

            let oversized_len: u32 = MAX_FRAME_SIZE + 1;
            stream
                .write_all(&oversized_len.to_be_bytes())
                .expect("peer: write oversized length prefix failed");
            // No payload bytes sent; `stream` drops at the end of this
            // closure, closing the connection -- safe to race against the
            // client's read since TCP preserves byte order across a close.
        });

        block_on(async {
            let mut session = connect_session(addr).await;
            let mut response = Vec::new();
            let result = session.execute(b"FA;", &mut response).await;
            match result {
                Err(TcpSessionError::FrameTooLarge { len, max }) => {
                    assert_eq!(len, MAX_FRAME_SIZE + 1);
                    assert_eq!(max, MAX_FRAME_SIZE);
                }
                other => panic!("expected FrameTooLarge, got {other:?}"),
            }
            assert!(response.is_empty());
        });
        peer.join().unwrap();
    }

    #[test]
    fn disconnect_mid_payload_returns_error_not_hang() {
        let (listener, addr) = bind_loopback();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept failed");
            let _req = read_raw_frame(&mut stream).expect("peer: read request frame failed");

            let declared_len: u32 = 14;
            stream
                .write_all(&declared_len.to_be_bytes())
                .expect("peer: write length prefix failed");
            stream
                .write_all(b"FA000")
                .expect("peer: partial payload write failed");
            drop(stream);
        });

        block_on(async {
            let mut session = connect_session(addr).await;
            let mut response = Vec::new();
            let result = session.execute(b"FA;", &mut response).await;
            match result {
                Err(TcpSessionError::Io(e)) => {
                    assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
                }
                other => panic!("expected Io(UnexpectedEof), got {other:?}"),
            }
        });
        peer.join().unwrap();
    }

    #[test]
    fn drop_joins_worker_thread_cleanly() {
        let (listener, addr) = bind_loopback();
        let peer = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept failed");
            // Keep the connection open until the client drops it.
            thread::sleep(std::time::Duration::from_millis(50));
            drop(stream);
        });

        let stream = TcpStream::connect(addr).expect("connect failed");
        let session = TcpCatSession::new(stream);
        drop(session); // must not hang
        peer.join().unwrap();
    }
}
