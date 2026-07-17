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

//! Server-side UDP accept/dispatch loop, wire-compatible with
//! `cat-transport-udp::UdpCatSession`'s envelope framing
//! (`planning/cat_transport/progress.md`'s Task 4b writeup, cross-checked
//! against `cat-transport-udp/src/session.rs`).
//!
//! Calls `cat-transport-udp`'s own [`cat_transport_udp::encode_envelope`]/
//! [`cat_transport_udp::decode_envelope`] directly rather than hand-rolling
//! a second copy (Task 6) — those were made `pub` specifically so this
//! listener (which both decodes incoming request envelopes and encodes
//! outgoing response envelopes) could stop maintaining a duplicate that
//! could drift out of sync. This is not the same thing Task 5 originally
//! ruled out (depending on `UdpCatSession` itself, which is
//! requester-shaped and would need an unnatural role inversion to reuse on
//! the accept/answer side, per `planning/cat_server/task_plan.md`'s
//! "Server-side framing" section) — `encode_envelope`/`decode_envelope` are
//! plain free functions with no requester/answerer role baked in. The
//! envelope shape is `session_id` (8 bytes, `u64` BE) + `request_id` (8
//! bytes, `u64` BE) + raw payload (remaining bytes, no length field — the
//! datagram's own boundary is the payload's boundary). A well-formed
//! envelope is always answered with exactly one response envelope carrying
//! the same `session_id`/`request_id` (empty payload for a `NoResponse`
//! outcome) — a datagram too short to contain the 16-byte header is
//! silently discarded as noise, matching `UdpCatSession`'s own convention,
//! never surfaced as an error.
//!
//! # Server-side deduplication cache — a different, load-bearing cache
//!
//! This is explicitly **not** the same mechanism as `UdpCatSession`'s own
//! client-side cache (which only recognizes "have I already seen the
//! answer to this," never replays anything, and is keyed by `request_id`
//! alone because one client session has one constant `session_id`). This
//! server serves *many* client sessions on one bound socket, so a duplicate
//! incoming request must be answered from a cache **without re-executing it
//! against the physical radio** — the genuinely load-bearing use case
//! `planning/cat_transport/progress.md` calls out by name.
//!
//! Keyed by `(peer_addr, session_id, request_id)` — a judgment call beyond
//! what that writeup requires (it explicitly allows `(session_id,
//! request_id)` or `(peer_addr, request_id)` alone): `peer_addr` alone can
//! collide across a client restart (a fresh `UdpCatSession` resets its
//! `request_id` counter to 1 but keeps the same source port), and
//! `session_id` alone relies purely on randomization; combining both with
//! `request_id` costs one extra tuple field and removes both edge cases at
//! once.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::SocketAddr;
use std::rc::Rc;

use cat_transport_udp::{decode_envelope, encode_envelope, ENVELOPE_HEADER_LEN, MAX_PAYLOAD_SIZE};
use monoio::net::udp::UdpSocket;

use crate::broker::BrokerHandle;
use crate::registry::{ClientId, ClientRegistry};

/// Total datagram size (header + max payload) allocated for each `recv_from`.
/// `ENVELOPE_HEADER_LEN`/`MAX_PAYLOAD_SIZE` are `cat_transport_udp`'s own
/// (re-exported here), matching `UdpCatSession`'s receive-side sizing
/// exactly — both sides of the wire must agree on the same cap.
const MAX_DATAGRAM_SIZE: usize = ENVELOPE_HEADER_LEN + MAX_PAYLOAD_SIZE;
/// Bounded FIFO capacity of the server-side dedup cache. See the module
/// docs for the key and why it differs from `UdpCatSession`'s own.
pub const DEDUP_CACHE_CAPACITY: usize = 256;

/// Server-side deduplication cache: caches the **actual response bytes**
/// for a `(peer_addr, session_id, request_id)` key, bounded FIFO. See the
/// module docs for why this differs from `UdpCatSession`'s own
/// membership-only cache.
#[derive(Default)]
pub struct DedupCache {
    order: VecDeque<(SocketAddr, u64, u64)>,
    responses: HashMap<(SocketAddr, u64, u64), Vec<u8>>,
}

impl DedupCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The cached response for this key, if this exact request has already
    /// been answered.
    pub fn get(&self, peer: SocketAddr, session_id: u64, request_id: u64) -> Option<&Vec<u8>> {
        self.responses.get(&(peer, session_id, request_id))
    }

    /// Record the response for this key, evicting the oldest entry first
    /// if already at [`DEDUP_CACHE_CAPACITY`].
    pub fn insert(
        &mut self,
        peer: SocketAddr,
        session_id: u64,
        request_id: u64,
        response: Vec<u8>,
    ) {
        let key = (peer, session_id, request_id);
        if !self.responses.contains_key(&key) {
            if self.order.len() >= DEDUP_CACHE_CAPACITY {
                if let Some(oldest) = self.order.pop_front() {
                    self.responses.remove(&oldest);
                }
            }
            self.order.push_back(key);
        }
        self.responses.insert(key, response);
    }
}

/// Accept/dispatch loop over an already-bound `UdpSocket`. Spawns one task
/// per received datagram (so a slow in-flight request never stalls reading
/// the next datagram from a different peer) sharing the socket, the dedup
/// cache, and the client registry via `Rc`. Runs until `recv_from` itself
/// fails.
pub async fn serve(
    socket: UdpSocket,
    handle: BrokerHandle,
    registry: Rc<RefCell<ClientRegistry>>,
) -> io::Result<()> {
    let socket = Rc::new(socket);
    let dedup = Rc::new(RefCell::new(DedupCache::new()));
    let peer_ids: Rc<RefCell<HashMap<SocketAddr, ClientId>>> =
        Rc::new(RefCell::new(HashMap::new()));

    loop {
        let buf = vec![0u8; MAX_DATAGRAM_SIZE];
        let (result, buf) = socket.recv_from(buf).await;
        let (n, from) = result?;

        let Some((session_id, request_id, payload)) = decode_envelope(&buf[..n]) else {
            // Too short to be a valid envelope -- noise, matches
            // UdpCatSession's own convention of silently discarding it.
            continue;
        };
        let payload = payload.to_vec();

        let socket = Rc::clone(&socket);
        let handle = handle.clone();
        let dedup = Rc::clone(&dedup);
        let registry = Rc::clone(&registry);
        let peer_ids = Rc::clone(&peer_ids);

        monoio::spawn(async move {
            handle_datagram(
                socket, handle, dedup, registry, peer_ids, from, session_id, request_id, payload,
            )
            .await;
        });
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_datagram(
    socket: Rc<UdpSocket>,
    handle: BrokerHandle,
    dedup: Rc<RefCell<DedupCache>>,
    registry: Rc<RefCell<ClientRegistry>>,
    peer_ids: Rc<RefCell<HashMap<SocketAddr, ClientId>>>,
    from: SocketAddr,
    session_id: u64,
    request_id: u64,
    payload: Vec<u8>,
) {
    // Recognized duplicate: resend the cached answer without re-executing
    // against the physical radio. This is the load-bearing use of this
    // cache (see module docs). The `RefCell` borrow is dropped before the
    // `await` below (bound to a local first, not held as the `if let`
    // scrutinee) -- holding it across an await point would be unsound if
    // another task tried to borrow the same cache concurrently.
    let cached = dedup.borrow().get(from, session_id, request_id).cloned();
    if let Some(cached) = cached {
        send_envelope(&socket, from, session_id, request_id, &cached).await;
        return;
    }

    let client_id = *peer_ids
        .borrow_mut()
        .entry(from)
        .or_insert_with(|| registry.borrow_mut().register());

    let response = match handle.submit(client_id, payload).await {
        Some(response) => response,
        None => return, // broker gone; nothing to send back
    };

    dedup
        .borrow_mut()
        .insert(from, session_id, request_id, response.clone());
    send_envelope(&socket, from, session_id, request_id, &response).await;
}

/// Send one response envelope. Best-effort — UDP delivery is never
/// guaranteed, and a client-side timeout is the documented backstop for a
/// response that never arrives.
///
/// `cat_transport_udp::encode_envelope` returns `Result` (it rejects a
/// payload wider than [`MAX_PAYLOAD_SIZE`] before sending anything). This
/// listener's own dispatch output (a real CAT response, or this crate's own
/// short `b"ERR <message>"` convention) is never expected to approach 1024
/// bytes, but "expected" is not "guaranteed" — a pathological error message
/// (e.g. one that echoes back an oversized malformed request) could
/// conceivably exceed it. Neither truncating the original payload (a
/// truncated CAT response could still parse as a different, wrong answer)
/// nor unwrapping (panicking a production listener over a
/// client-triggerable input) is acceptable, so a too-large payload is
/// replaced with a short, fixed, guaranteed-under-the-cap error envelope
/// instead — `encode_envelope` cannot fail for a payload this small, so the
/// fallback's own `expect` is not a live panic risk.
async fn send_envelope(
    socket: &UdpSocket,
    to: SocketAddr,
    session_id: u64,
    request_id: u64,
    payload: &[u8],
) {
    let datagram = encode_envelope(session_id, request_id, payload).unwrap_or_else(|_| {
        encode_envelope(session_id, request_id, b"ERR response too large to send")
            .expect("fallback error payload is well under MAX_PAYLOAD_SIZE")
    });
    let (_result, _buf) = socket.send_to(datagram, to).await;
}

#[cfg(test)]
mod tests {
    use cat_transport_core::test_support::{Exchange, ScriptedCatSession};

    use super::*;
    use crate::broker::build;
    use crate::test_fixtures::TABLE;

    fn bind_loopback() -> (UdpSocket, SocketAddr) {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("UdpSocket::bind failed");
        let addr = socket.local_addr().expect("local_addr failed");
        (socket, addr)
    }

    fn spawn_server(script: impl IntoIterator<Item = Exchange>) -> SocketAddr {
        let (socket, addr) = bind_loopback();
        let (worker, handle) = build(ScriptedCatSession::with_script(script), &TABLE);
        let registry = Rc::new(RefCell::new(ClientRegistry::new()));
        monoio::spawn(worker.run());
        monoio::spawn(serve(socket, handle, registry));
        addr
    }

    async fn send_request(
        client: &UdpSocket,
        server_addr: SocketAddr,
        session_id: u64,
        request_id: u64,
        payload: &[u8],
    ) {
        let datagram =
            encode_envelope(session_id, request_id, payload).expect("test payload too large");
        let (result, _buf) = client.send_to(datagram, server_addr).await;
        result.expect("send_to failed");
    }

    async fn recv_response(client: &UdpSocket) -> (u64, u64, Vec<u8>) {
        let buf = vec![0u8; MAX_DATAGRAM_SIZE];
        let (result, buf) = client.recv_from(buf).await;
        let (n, _from) = result.expect("recv_from failed");
        let (session_id, request_id, payload) = decode_envelope(&buf[..n]).expect("short datagram");
        (session_id, request_id, payload.to_vec())
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn end_to_end_query_round_trip_over_real_loopback_udp() {
        let server_addr = spawn_server([Exchange::new("FA;", "FA00014250000;")]);
        let (client, _client_addr) = bind_loopback();

        send_request(&client, server_addr, 111, 1, b"FA;").await;
        let (session_id, request_id, payload) = recv_response(&client).await;

        assert_eq!(session_id, 111);
        assert_eq!(request_id, 1);
        assert_eq!(payload, b"FA00014250000;");
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn end_to_end_set_gets_explicit_empty_response_envelope() {
        let server_addr = spawn_server([Exchange::new("TX;", "")]);
        let (client, _client_addr) = bind_loopback();

        send_request(&client, server_addr, 222, 1, b"TX;").await;
        let (_session_id, _request_id, payload) = recv_response(&client).await;

        assert!(payload.is_empty());
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn end_to_end_malformed_request_gets_error_envelope_not_forwarded_to_radio() {
        let server_addr = spawn_server([Exchange::new("FA;", "FA00014250000;")]);
        let (client, _client_addr) = bind_loopback();

        send_request(&client, server_addr, 333, 1, b"ZZ;").await;
        let (_sid, _rid, payload) = recv_response(&client).await;
        assert!(payload.starts_with(b"ERR "));

        // The scripted exchange for FA is still available afterward --
        // proof the malformed request never touched the physical session.
        send_request(&client, server_addr, 333, 2, b"FA;").await;
        let (_sid, request_id, payload) = recv_response(&client).await;
        assert_eq!(request_id, 2);
        assert_eq!(payload, b"FA00014250000;");
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn duplicate_request_gets_cached_response_without_re_executing() {
        // Only ONE scripted exchange -- if the duplicate were re-executed
        // against the physical session, the second attempt would panic
        // (ScriptedCatSession panics on an exhausted script).
        let server_addr = spawn_server([Exchange::new("FA;", "FA00014250000;")]);
        let (client, _client_addr) = bind_loopback();

        send_request(&client, server_addr, 444, 1, b"FA;").await;
        let (_sid, _rid, first) = recv_response(&client).await;

        // Re-send the exact same request envelope (same session_id AND
        // request_id) -- a duplicate delivery.
        send_request(&client, server_addr, 444, 1, b"FA;").await;
        let (_sid, _rid, second) = recv_response(&client).await;

        assert_eq!(first, b"FA00014250000;");
        assert_eq!(second, first, "duplicate must get the cached answer");
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn malformed_short_datagram_is_ignored_as_noise() {
        let server_addr = spawn_server([Exchange::new("FA;", "FA00014250000;")]);
        let (client, _client_addr) = bind_loopback();

        // Garbage datagram, too short to contain a full 16-byte header.
        let (result, _buf) = client.send_to(vec![0xAAu8, 0xBB, 0xCC], server_addr).await;
        result.expect("garbage send_to failed");

        // A real, well-formed request afterward still gets served
        // normally -- the garbage did not wedge or crash the listener.
        send_request(&client, server_addr, 555, 1, b"FA;").await;
        let (_sid, _rid, payload) = recv_response(&client).await;
        assert_eq!(payload, b"FA00014250000;");
    }

    // -------------------------------------------------------------------
    // Dedup cache: pure-logic unit tests
    // -------------------------------------------------------------------

    #[test]
    fn dedup_cache_recognizes_a_cached_response() {
        let peer: SocketAddr = "127.0.0.1:9999".parse().unwrap();
        let mut cache = DedupCache::new();
        assert!(cache.get(peer, 1, 1).is_none());

        cache.insert(peer, 1, 1, b"FA00014250000;".to_vec());
        assert_eq!(cache.get(peer, 1, 1), Some(&b"FA00014250000;".to_vec()));
    }

    #[test]
    fn dedup_cache_distinguishes_same_peer_addr_different_session_id() {
        // The exact edge case this key shape (peer_addr + session_id +
        // request_id) is meant to close: a client restarting from the same
        // source port/request_id but a new (randomized) session_id must
        // not be served a stale cached response from a previous session.
        let peer: SocketAddr = "127.0.0.1:9999".parse().unwrap();
        let mut cache = DedupCache::new();
        cache.insert(peer, /* session_id */ 1, 1, b"OLD".to_vec());

        assert!(cache.get(peer, /* session_id */ 2, 1).is_none());
    }

    #[test]
    fn dedup_cache_evicts_oldest_beyond_capacity() {
        let peer: SocketAddr = "127.0.0.1:9999".parse().unwrap();
        let mut cache = DedupCache::new();
        for id in 1..=DEDUP_CACHE_CAPACITY as u64 {
            cache.insert(peer, 1, id, vec![id as u8]);
        }
        assert!(cache.get(peer, 1, 1).is_some(), "oldest not yet evicted");

        cache.insert(peer, 1, DEDUP_CACHE_CAPACITY as u64 + 1, vec![0xFF]);
        assert!(
            cache.get(peer, 1, 1).is_none(),
            "oldest entry should have been evicted once capacity was exceeded"
        );
        assert!(
            cache.get(peer, 1, 2).is_some(),
            "second-oldest should survive"
        );
    }

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
    fn decode_rejects_datagram_shorter_than_header() {
        assert!(decode_envelope(&[0u8; ENVELOPE_HEADER_LEN - 1]).is_none());
        assert!(decode_envelope(&[]).is_none());
    }
}
