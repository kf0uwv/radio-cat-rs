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

//! Windows `UdpCatSession` backend.
//!
//! Per `docs/adr/0006-windows-network-transport.md`: UDP sockets are
//! natively cross-platform in `std` (`std::net::UdpSocket`), so this needs
//! no `windows-sys`/FFI. What's missing on Windows is `monoio` itself (no
//! io_uring, and no `monoio::time::timeout_at` to bound the response wait),
//! so this backend uses the same dedicated-background-thread +
//! [`cat_transport_core::completion`] shape as `cat-transport-tcp::windows`
//! and `cat-transport-serial`'s Windows backend (ADR 0004 §1).
//!
//! Unlike TCP (or serial), the "wait for a matching response, filtering out
//! foreign/duplicate/stale datagrams, bounded by an overall deadline"
//! session state (`session_id`, `next_request_id`, the dedup cache) is
//! genuinely part of *this session's* long-lived state, not a stateless
//! per-call codec -- so it lives inside the worker thread's closure, moved
//! there once at construction, exactly mirroring how the Linux backend
//! (`crate::session`) keeps that same state on `UdpCatSession` itself.
//! **The response-timeout wait itself is implemented with
//! `UdpSocket::set_read_timeout` + a per-iteration remaining-time
//! recomputation** -- a direct, simpler analog of `monoio::time::timeout_at`
//! that needs no async timer machinery at all, since the whole wait already
//! happens inside a blocking worker thread.
//!
//! # This module is genuinely cross-platform, and is tested on Linux too
//!
//! See `cat-transport-tcp::windows`'s module doc for the same point, which
//! applies identically here: nothing below is actually Windows-specific
//! (plain `std::net`/`std::thread`/`std::sync::mpsc` plus the portable
//! completion primitive), so this module is not `#[cfg(target_os =
//! "windows")]`-gated -- it compiles and its tests run on every platform.
//! Only `lib.rs`'s top-level `pub use` of `UdpCatSession` is `cfg`-gated.

use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cat_transport_core::completion;
use cat_transport_core::{CatSession, ResponseDisposition};

use crate::codec::{
    self, decode_envelope, encode_envelope, RequestIdCache, UdpSessionError,
    DEFAULT_RESPONSE_TIMEOUT, MAX_DATAGRAM_SIZE,
};

/// One request sent to [`UdpCatSession`]'s background worker thread: send
/// `payload` as one envelope to the configured peer, then wait (bounded by
/// this session's `response_timeout`) for a matching response envelope,
/// applying the exact same source-address/`session_id`/`request_id`
/// filtering and dedup-cache bookkeeping as the Linux backend.
struct WorkerRequest {
    payload: Vec<u8>,
    reply: completion::CompletionTx<Result<Vec<u8>, UdpSessionError>>,
}

/// Long-lived state the worker thread owns for the whole session lifetime --
/// the Windows-backend analog of the fields `crate::session::UdpCatSession`
/// keeps on itself directly.
struct WorkerState {
    socket: UdpSocket,
    peer_addr: SocketAddr,
    session_id: u64,
    next_request_id: u64,
    dedup_cache: RequestIdCache,
    response_timeout: Duration,
}

impl WorkerState {
    /// Perform one full send-then-wait exchange, identical filtering logic
    /// to `crate::session::UdpCatSession::execute` (see its module doc's
    /// "Request/response pairing" and "Deduplication cache" sections).
    fn exchange(&mut self, payload: &[u8]) -> Result<Vec<u8>, UdpSessionError> {
        self.next_request_id = self.next_request_id.wrapping_add(1);
        let request_id = self.next_request_id;

        let datagram = encode_envelope(self.session_id, request_id, payload)?;
        self.socket.send_to(&datagram, self.peer_addr)?;

        let deadline = Instant::now() + self.response_timeout;
        let mut buf = vec![0u8; MAX_DATAGRAM_SIZE];

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(UdpSessionError::Timeout {
                    peer: self.peer_addr,
                    timeout: self.response_timeout,
                });
            }
            // `set_read_timeout` bounds the NEXT single `recv_from` call --
            // recomputed every loop iteration (not set once) for the same
            // reason `crate::session`'s Linux backend uses a single fixed
            // deadline rather than re-applying the full duration per
            // iteration: a peer that keeps delivering irrelevant noise must
            // not be able to extend the overall wait past `response_timeout`.
            self.socket
                .set_read_timeout(Some(remaining))
                .map_err(UdpSessionError::Io)?;

            let (n, from) = match self.socket.recv_from(&mut buf) {
                Ok(result) => result,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    return Err(UdpSessionError::Timeout {
                        peer: self.peer_addr,
                        timeout: self.response_timeout,
                    });
                }
                Err(e) => return Err(UdpSessionError::Io(e)),
            };

            if from != self.peer_addr {
                continue;
            }

            let Some((incoming_session_id, incoming_request_id, response_payload)) =
                decode_envelope(&buf[..n])
            else {
                continue;
            };

            if incoming_session_id != self.session_id {
                continue;
            }

            if incoming_request_id != request_id {
                let _is_duplicate = self.dedup_cache.is_known_duplicate(incoming_request_id);
                continue;
            }

            self.dedup_cache.remember_completed(request_id);
            return Ok(response_payload.to_vec());
        }
    }
}

/// The worker thread body: owns `state` for its whole lifetime, performing
/// one blocking [`WorkerState::exchange`] per [`WorkerRequest`] received
/// over `rx`. Exits when `rx.recv()` returns `Err` -- i.e.
/// [`UdpCatSession`]'s `Drop` impl has dropped the request sender.
fn worker_loop(mut state: WorkerState, rx: mpsc::Receiver<WorkerRequest>) {
    while let Ok(request) = rx.recv() {
        let result = state.exchange(&request.payload);
        request.reply.send(result);
    }
}

/// Build the [`UdpSessionError`] used when the worker thread is
/// unexpectedly gone. Mirrors `cat-transport-tcp::windows::worker_gone_error`.
fn worker_gone_error() -> UdpSessionError {
    UdpSessionError::Io(std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        "cat-transport-udp: Windows worker thread is no longer running",
    ))
}

/// [`CatSession`] backed by a `std::net::UdpSocket`, using the same
/// envelope framing, dedup cache, and response-timeout policy as the Linux
/// (`monoio`) backend -- see `crate::session`'s module doc for the exact
/// wire format, which this backend reuses unchanged via [`crate::codec`].
///
/// Same public API as the Linux backend (`new`, `bind_to`,
/// `bind_to_with_timeout`, `session_id`, `local_addr`, `CatSession`) so
/// application code needs no platform branching to use it.
pub struct UdpCatSession {
    request_tx: Option<mpsc::Sender<WorkerRequest>>,
    worker: Option<thread::JoinHandle<()>>,
    session_id: u64,
    local_addr: SocketAddr,
}

impl UdpCatSession {
    /// Wrap an already-bound `UdpSocket` in a session that always talks to
    /// `peer_addr`, using `response_timeout` as the bound on every
    /// `execute()` call. The session's `session_id` is randomized
    /// internally.
    pub fn new(socket: UdpSocket, peer_addr: SocketAddr, response_timeout: Duration) -> Self {
        let session_id = codec::random_session_id();
        let local_addr = socket
            .local_addr()
            .expect("a bound UdpSocket always has a local address");

        let state = WorkerState {
            socket,
            peer_addr,
            session_id,
            next_request_id: 0,
            dedup_cache: RequestIdCache::new(),
            response_timeout,
        };
        let (request_tx, request_rx) = mpsc::channel();
        let worker = thread::spawn(move || worker_loop(state, request_rx));

        Self {
            request_tx: Some(request_tx),
            worker: Some(worker),
            session_id,
            local_addr,
        }
    }

    /// Bind a fresh ephemeral local socket and wrap it in a session talking
    /// to `peer_addr`, using [`DEFAULT_RESPONSE_TIMEOUT`].
    pub fn bind_to(peer_addr: SocketAddr) -> Result<Self, UdpSessionError> {
        Self::bind_to_with_timeout(peer_addr, DEFAULT_RESPONSE_TIMEOUT)
    }

    /// Like [`bind_to`](Self::bind_to), with an explicit `response_timeout`.
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
        Ok(self.local_addr)
    }

    fn request_tx(&self) -> &mpsc::Sender<WorkerRequest> {
        self.request_tx
            .as_ref()
            .expect("UdpCatSession::request_tx is only None after Drop::drop")
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

    // `send` is NOT overridden -- same reasoning as the Linux backend (see
    // `crate::session`'s module doc): a well-behaved UDP peer is still
    // expected to answer every request, and `response_timeout` is the
    // backstop if it doesn't.
}

impl Drop for UdpCatSession {
    /// Same shutdown sequencing as `cat-transport-tcp::windows::
    /// TcpCatSession::drop`: drop the sender first (unblocking the worker's
    /// `recv()`), then join the thread.
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
    use cat_transport_core::conformance;
    use std::time::Instant as StdInstant;

    const TEST_TIMEOUT: Duration = Duration::from_millis(200);

    fn bind_loopback_socket() -> (UdpSocket, SocketAddr) {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("UdpSocket::bind failed");
        let addr = socket.local_addr().expect("local_addr failed");
        (socket, addr)
    }

    fn raw_envelope(session_id: u64, request_id: u64, payload: &[u8]) -> Vec<u8> {
        let mut datagram = Vec::with_capacity(16 + payload.len());
        datagram.extend_from_slice(&session_id.to_be_bytes());
        datagram.extend_from_slice(&request_id.to_be_bytes());
        datagram.extend_from_slice(payload);
        datagram
    }

    fn parse_raw_envelope(datagram: &[u8]) -> (u64, u64, Vec<u8>) {
        assert!(
            datagram.len() >= 16,
            "datagram shorter than envelope header"
        );
        let session_id = u64::from_be_bytes(datagram[0..8].try_into().unwrap());
        let request_id = u64::from_be_bytes(datagram[8..16].try_into().unwrap());
        (session_id, request_id, datagram[16..].to_vec())
    }

    fn peer_recv_request(peer: &UdpSocket) -> (u64, u64, Vec<u8>, SocketAddr) {
        let mut buf = vec![0u8; MAX_DATAGRAM_SIZE];
        let (n, from) = peer.recv_from(&mut buf).expect("peer: recv_from failed");
        let (session_id, request_id, payload) = parse_raw_envelope(&buf[..n]);
        (session_id, request_id, payload, from)
    }

    fn peer_send_response(
        peer: &UdpSocket,
        to: SocketAddr,
        session_id: u64,
        request_id: u64,
        payload: &[u8],
    ) {
        let datagram = raw_envelope(session_id, request_id, payload);
        peer.send_to(&datagram, to).expect("peer: send_to failed");
    }

    /// Minimal, single-threaded block_on for this test module -- see
    /// `cat-transport-tcp::windows`'s test module for the identical helper
    /// and rationale.
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
        let (peer, peer_addr) = bind_loopback_socket();
        let mut session =
            UdpCatSession::bind_to_with_timeout(peer_addr, TEST_TIMEOUT).expect("bind_to failed");

        let peer_thread = thread::spawn(move || {
            let (session_id, request_id, payload, from) = peer_recv_request(&peer);
            assert_eq!(payload, b"ID;");
            peer_send_response(&peer, from, session_id, request_id, b"ID017;");
        });

        block_on(conformance::query_round_trip(
            &mut session,
            b"ID;",
            b"ID017;",
        ));
        peer_thread.join().unwrap();
    }

    #[test]
    fn conformance_set_is_fire_and_forget() {
        let (peer, peer_addr) = bind_loopback_socket();
        let mut session =
            UdpCatSession::bind_to_with_timeout(peer_addr, TEST_TIMEOUT).expect("bind_to failed");

        let peer_thread = thread::spawn(move || {
            let (session_id, request_id, payload, from) = peer_recv_request(&peer);
            assert_eq!(payload, b"TX;");
            peer_send_response(&peer, from, session_id, request_id, b"");
        });

        block_on(conformance::set_is_fire_and_forget(&mut session, b"TX;"));
        peer_thread.join().unwrap();
    }

    #[test]
    fn conformance_surfaces_transport_error() {
        let (peer, peer_addr) = bind_loopback_socket();
        let mut session =
            UdpCatSession::bind_to_with_timeout(peer_addr, TEST_TIMEOUT).expect("bind_to failed");

        let peer_thread = thread::spawn(move || {
            let _ = peer_recv_request(&peer);
        });

        block_on(conformance::surfaces_transport_error(&mut session, b"FA;"));
        peer_thread.join().unwrap();
    }

    #[test]
    fn duplicate_response_delivery_does_not_corrupt_next_request() {
        let (peer, peer_addr) = bind_loopback_socket();
        let mut session =
            UdpCatSession::bind_to_with_timeout(peer_addr, TEST_TIMEOUT).expect("bind_to failed");

        let peer_thread = thread::spawn(move || {
            let (session_id, request_id, payload, from) = peer_recv_request(&peer);
            assert_eq!(payload, b"FA;");
            peer_send_response(&peer, from, session_id, request_id, b"FA00014250000;");
            peer_send_response(&peer, from, session_id, request_id, b"FA00014250000;");

            let (session_id, request_id, payload, from) = peer_recv_request(&peer);
            assert_eq!(payload, b"FB;");
            peer_send_response(&peer, from, session_id, request_id, b"FB00014250000;");
        });

        let mut response = Vec::new();
        let disposition =
            block_on(session.execute(b"FA;", &mut response)).expect("execute(1) should succeed");
        assert_eq!(disposition, ResponseDisposition::ResponseWritten);
        assert_eq!(response, b"FA00014250000;");

        response.clear();
        let disposition = block_on(session.execute(b"FB;", &mut response))
            .expect("execute(2) should succeed despite a leftover duplicate");
        assert_eq!(disposition, ResponseDisposition::ResponseWritten);
        assert_eq!(
            response, b"FB00014250000;",
            "must not double-respond with the stale duplicate from request 1"
        );
        peer_thread.join().unwrap();
    }

    #[test]
    fn never_answered_request_times_out_instead_of_hanging() {
        let (peer, peer_addr) = bind_loopback_socket();
        let mut session =
            UdpCatSession::bind_to_with_timeout(peer_addr, TEST_TIMEOUT).expect("bind_to failed");

        let peer_thread = thread::spawn(move || {
            let _ = peer_recv_request(&peer);
        });

        let mut response = Vec::new();
        let started = StdInstant::now();
        let result = block_on(session.execute(b"FA;", &mut response));
        let elapsed = started.elapsed();

        match result {
            Err(UdpSessionError::Timeout { peer, timeout }) => {
                assert_eq!(peer, peer_addr);
                assert_eq!(timeout, TEST_TIMEOUT);
            }
            other => panic!("expected Timeout, got {:?}", other),
        }
        assert!(response.is_empty());
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
        peer_thread.join().unwrap();
    }

    #[test]
    fn ignores_malformed_short_datagram_and_still_receives_real_response() {
        let (peer, peer_addr) = bind_loopback_socket();
        let mut session =
            UdpCatSession::bind_to_with_timeout(peer_addr, TEST_TIMEOUT).expect("bind_to failed");

        let peer_thread = thread::spawn(move || {
            let (session_id, request_id, payload, from) = peer_recv_request(&peer);
            assert_eq!(payload, b"FA;");

            peer.send_to(&[0xAAu8, 0xBB, 0xCC], from)
                .expect("peer: garbage send_to failed");

            peer_send_response(&peer, from, session_id, request_id, b"FA00014250000;");
        });

        let mut response = Vec::new();
        let disposition = block_on(session.execute(b"FA;", &mut response))
            .expect("execute should succeed despite leading garbage datagram");
        assert_eq!(disposition, ResponseDisposition::ResponseWritten);
        assert_eq!(response, b"FA00014250000;");
        peer_thread.join().unwrap();
    }

    #[test]
    fn session_id_is_randomized_across_instances() {
        let (_peer_a, peer_addr) = bind_loopback_socket();
        let session_a = UdpCatSession::bind_to(peer_addr).expect("bind_to failed");
        let session_b = UdpCatSession::bind_to(peer_addr).expect("bind_to failed");

        assert_ne!(session_a.session_id(), session_b.session_id());
    }
}
