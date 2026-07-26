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

//! Windows analog of `crate::udp`'s server-side UDP accept/dispatch loop --
//! same envelope wire format and server-side dedup cache
//! ([`crate::dedup::DedupCache`]) as the Linux backend, rebuilt on
//! `std::net::UdpSocket` + genuine OS threads instead of `monoio::net` +
//! cooperative tasks.
//!
//! Per `docs/adr/0006-windows-network-transport.md`: the main thread runs a
//! blocking `recv_from` loop; each received datagram is handed to a freshly
//! spawned `std::thread` (mirroring `crate::udp`'s "one task per datagram,
//! so a slow in-flight request never stalls reading the next datagram"
//! property) that decodes it, checks/updates the shared
//! `Arc<Mutex<DedupCache>>`, submits to
//! [`crate::worker_windows::BrokerHandle`] via [`crate::block_on`], and
//! sends the response back over a shared `Arc<UdpSocket>` (safe for
//! concurrent `send_to` from multiple threads).
//!
//! # This module is genuinely cross-platform, and is tested on Linux too
//!
//! See `cat-transport-tcp::windows`'s module doc for the same point:
//! nothing below is actually Windows-specific, so this module is not
//! `#[cfg(target_os = "windows")]`-gated.

use std::collections::HashMap;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread;

use cat_transport_udp::{decode_envelope, encode_envelope, ENVELOPE_HEADER_LEN, MAX_PAYLOAD_SIZE};

use crate::block_on::block_on;
use crate::dedup::DedupCache;
use crate::registry::{ClientId, ClientRegistry};
use crate::worker_windows::BrokerHandle;

const MAX_DATAGRAM_SIZE: usize = ENVELOPE_HEADER_LEN + MAX_PAYLOAD_SIZE;

/// Accept/dispatch loop over an already-bound `UdpSocket`. Spawns one
/// dedicated `std::thread` per received datagram; runs until `recv_from`
/// itself fails.
pub fn serve(
    socket: UdpSocket,
    handle: BrokerHandle,
    registry: Arc<Mutex<ClientRegistry>>,
) -> io::Result<()> {
    let socket = Arc::new(socket);
    let dedup = Arc::new(Mutex::new(DedupCache::new()));
    let peer_ids: Arc<Mutex<HashMap<SocketAddr, ClientId>>> = Arc::new(Mutex::new(HashMap::new()));

    loop {
        let mut buf = vec![0u8; MAX_DATAGRAM_SIZE];
        let (n, from) = socket.recv_from(&mut buf)?;

        let Some((session_id, request_id, payload)) = decode_envelope(&buf[..n]) else {
            continue;
        };
        let payload = payload.to_vec();

        let socket = Arc::clone(&socket);
        let handle = handle.clone();
        let dedup = Arc::clone(&dedup);
        let registry = Arc::clone(&registry);
        let peer_ids = Arc::clone(&peer_ids);

        thread::spawn(move || {
            handle_datagram(
                &socket, handle, &dedup, &registry, &peer_ids, from, session_id, request_id,
                payload,
            );
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_datagram(
    socket: &UdpSocket,
    handle: BrokerHandle,
    dedup: &Mutex<DedupCache>,
    registry: &Mutex<ClientRegistry>,
    peer_ids: &Mutex<HashMap<SocketAddr, ClientId>>,
    from: SocketAddr,
    session_id: u64,
    request_id: u64,
    payload: Vec<u8>,
) {
    let cached = dedup
        .lock()
        .unwrap()
        .get(from, session_id, request_id)
        .cloned();
    if let Some(cached) = cached {
        send_envelope(socket, from, session_id, request_id, &cached);
        return;
    }

    let client_id = *peer_ids
        .lock()
        .unwrap()
        .entry(from)
        .or_insert_with(|| registry.lock().unwrap().register());

    let response = match block_on(handle.submit(client_id, payload)) {
        Some(response) => response,
        None => return, // broker gone; nothing to send back
    };

    dedup
        .lock()
        .unwrap()
        .insert(from, session_id, request_id, response.clone());
    send_envelope(socket, from, session_id, request_id, &response);
}

/// Send one response envelope. Best-effort, mirroring `crate::udp`'s
/// identical fallback for an oversized dispatch-output payload.
fn send_envelope(
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
    let _ = socket.send_to(&datagram, to);
}

// Gated `target_os = "windows"` in addition to `test`: same reasoning as
// `crate::tcp_windows`'s identical gate -- every test below drives
// `crate::broker::Broker::dispatch` via `crate::block_on`, which is unsafe
// to combine with `dispatch`'s Linux-build `monoio::time::timeout` wrap.
#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use crate::test_fixtures::TABLE;
    use crate::worker_windows::build;
    use cat_transport_core::test_support::{Exchange, ScriptedCatSession};

    fn bind_loopback() -> (UdpSocket, SocketAddr) {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("UdpSocket::bind failed");
        let addr = socket.local_addr().expect("local_addr failed");
        (socket, addr)
    }

    fn spawn_server(script: impl IntoIterator<Item = Exchange>) -> SocketAddr {
        let (socket, addr) = bind_loopback();
        let (worker, handle) = build(ScriptedCatSession::with_script(script), &TABLE);
        let registry = Arc::new(Mutex::new(ClientRegistry::new()));
        thread::spawn(move || worker.run());
        thread::spawn(move || serve(socket, handle, registry));
        addr
    }

    fn send_request(
        client: &UdpSocket,
        server_addr: SocketAddr,
        session_id: u64,
        request_id: u64,
        payload: &[u8],
    ) {
        let datagram =
            encode_envelope(session_id, request_id, payload).expect("test payload too large");
        client
            .send_to(&datagram, server_addr)
            .expect("send_to failed");
    }

    fn recv_response(client: &UdpSocket) -> (u64, u64, Vec<u8>) {
        let mut buf = vec![0u8; MAX_DATAGRAM_SIZE];
        let (n, _from) = client.recv_from(&mut buf).expect("recv_from failed");
        let (session_id, request_id, payload) = decode_envelope(&buf[..n]).expect("short datagram");
        (session_id, request_id, payload.to_vec())
    }

    #[test]
    fn end_to_end_query_round_trip_over_real_loopback_udp() {
        let server_addr = spawn_server([Exchange::new("FA;", "FA00014250000;")]);
        let (client, _client_addr) = bind_loopback();

        send_request(&client, server_addr, 111, 1, b"FA;");
        let (session_id, request_id, payload) = recv_response(&client);

        assert_eq!(session_id, 111);
        assert_eq!(request_id, 1);
        assert_eq!(payload, b"FA00014250000;");
    }

    #[test]
    fn end_to_end_set_gets_explicit_empty_response_envelope() {
        let server_addr = spawn_server([Exchange::new("TX;", "")]);
        let (client, _client_addr) = bind_loopback();

        send_request(&client, server_addr, 222, 1, b"TX;");
        let (_session_id, _request_id, payload) = recv_response(&client);

        assert!(payload.is_empty());
    }

    #[test]
    fn end_to_end_malformed_request_gets_error_envelope_not_forwarded_to_radio() {
        let server_addr = spawn_server([Exchange::new("FA;", "FA00014250000;")]);
        let (client, _client_addr) = bind_loopback();

        send_request(&client, server_addr, 333, 1, b"ZZ;");
        let (_sid, _rid, payload) = recv_response(&client);
        assert!(payload.starts_with(b"ERR "));

        send_request(&client, server_addr, 333, 2, b"FA;");
        let (_sid, request_id, payload) = recv_response(&client);
        assert_eq!(request_id, 2);
        assert_eq!(payload, b"FA00014250000;");
    }

    #[test]
    fn duplicate_request_gets_cached_response_without_re_executing() {
        let server_addr = spawn_server([Exchange::new("FA;", "FA00014250000;")]);
        let (client, _client_addr) = bind_loopback();

        send_request(&client, server_addr, 444, 1, b"FA;");
        let (_sid, _rid, first) = recv_response(&client);

        send_request(&client, server_addr, 444, 1, b"FA;");
        let (_sid, _rid, second) = recv_response(&client);

        assert_eq!(first, b"FA00014250000;");
        assert_eq!(second, first, "duplicate must get the cached answer");
    }

    #[test]
    fn malformed_short_datagram_is_ignored_as_noise() {
        let server_addr = spawn_server([Exchange::new("FA;", "FA00014250000;")]);
        let (client, _client_addr) = bind_loopback();

        client
            .send_to(&[0xAAu8, 0xBB, 0xCC], server_addr)
            .expect("garbage send_to failed");

        send_request(&client, server_addr, 555, 1, b"FA;");
        let (_sid, _rid, payload) = recv_response(&client);
        assert_eq!(payload, b"FA00014250000;");
    }
}
