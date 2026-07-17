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

//! Server-side TCP accept/dispatch loop, wire-compatible with
//! `cat-transport-tcp::TcpCatSession`'s length-prefixed framing
//! (`planning/cat_transport/progress.md`'s Task 4a writeup, cross-checked
//! against `cat-transport-tcp/src/session.rs`).
//!
//! This calls `cat-transport-tcp`'s own codec functions
//! ([`cat_transport_tcp::read_frame_or_eof`], [`cat_transport_tcp::write_frame`])
//! directly rather than hand-rolling a second copy of the same encode/decode
//! logic (Task 6) — those were made `pub` specifically so this listener
//! could stop maintaining a duplicate that could drift out of sync. This is
//! not the same thing Task 5 originally ruled out (depending on
//! `TcpCatSession` itself, which is requester-shaped and would need an
//! unnatural role inversion to reuse on the accept/answer side, per
//! `planning/cat_server/task_plan.md`'s "Server-side framing" section) —
//! `read_frame_or_eof`/`write_frame` are plain free functions with no
//! requester/answerer role baked in, so no inversion issue applies to
//! importing just these. The frame shape is a 4-byte big-endian `u32`
//! length prefix (payload bytes only, no terminator), followed by exactly
//! that many raw payload bytes; a length of zero is a valid, complete frame.
//! [`MAX_FRAME_SIZE`] (64 KiB, re-exported from `cat_transport_tcp`) bounds
//! it — both sides must agree on the same cap.
//!
//! **Hard requirement this honors**: every request frame gets exactly one
//! response frame back, even an empty one, and even when the broker itself
//! rejects the request — the client-side `TcpCatSession` blocks reading a
//! response with no timeout of its own, so silence here would hang it
//! forever.

use std::cell::RefCell;
use std::io;
use std::rc::Rc;

use cat_transport_tcp::{read_frame_or_eof, write_frame};
use monoio::net::{TcpListener, TcpStream};

use crate::broker::BrokerHandle;
use crate::registry::ClientRegistry;

/// Accept loop. Binding is the caller's responsibility (pass an
/// already-bound `TcpListener`) so callers/tests can choose the address
/// (e.g. an ephemeral loopback port). Spawns one task per accepted
/// connection; runs until `accept()` itself fails.
pub async fn serve(
    listener: TcpListener,
    handle: BrokerHandle,
    registry: Rc<RefCell<ClientRegistry>>,
) -> io::Result<()> {
    loop {
        let (stream, _peer_addr) = listener.accept().await?;
        let handle = handle.clone();
        let registry = Rc::clone(&registry);
        monoio::spawn(handle_connection(stream, handle, registry));
    }
}

/// Service one accepted connection until it disconnects or the broker
/// itself is gone.
async fn handle_connection(
    mut stream: TcpStream,
    handle: BrokerHandle,
    registry: Rc<RefCell<ClientRegistry>>,
) {
    let client_id = registry.borrow_mut().register();

    loop {
        match read_frame_or_eof(&mut stream).await {
            Ok(Some(payload)) => {
                let response = match handle.submit(client_id, payload).await {
                    Some(response) => response,
                    None => break, // broker shut down
                };
                if write_frame(&mut stream, &response).await.is_err() {
                    break; // client disconnected before we could reply
                }
            }
            Ok(None) => break, // clean disconnect at a frame boundary
            Err(e) => {
                // Disconnect mid-frame, or an oversized declared length
                // (`TcpSessionError::FrameTooLarge`/`Io`). Best-effort tell
                // the client why before closing — if the connection is
                // already dead this write simply fails too, which is fine.
                let _ = write_frame(&mut stream, format!("ERR {e}").as_bytes()).await;
                break;
            }
        }
    }

    registry.borrow_mut().unregister(client_id);
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use cat_transport_core::test_support::{Exchange, ScriptedCatSession};
    use cat_transport_tcp::MAX_FRAME_SIZE;
    use monoio::io::{AsyncReadRentExt, AsyncWriteRentExt};

    use super::*;
    use crate::broker::build;
    use crate::test_fixtures::TABLE;

    fn bind_loopback() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("TcpListener::bind failed");
        let addr = listener.local_addr().expect("local_addr failed");
        (listener, addr)
    }

    /// Independent, hand-rolled raw frame writer/reader — deliberately does
    /// NOT call `cat_transport_tcp::write_frame`/`read_frame_or_eof` (the
    /// functions `handle_connection` itself uses), so these tests
    /// cross-check the *documented* wire format against the production
    /// implementation, mirroring `cat-transport-tcp`'s own test-module
    /// convention.
    async fn write_raw_frame(stream: &mut TcpStream, payload: &[u8]) {
        let len = payload.len() as u32;
        let mut frame = Vec::with_capacity(4 + payload.len());
        frame.extend_from_slice(&len.to_be_bytes());
        frame.extend_from_slice(payload);
        let (result, _buf) = stream.write_all(frame).await;
        result.expect("write_raw_frame failed");
    }

    async fn read_raw_frame(stream: &mut TcpStream) -> Vec<u8> {
        let (result, len_buf) = stream.read_exact(vec![0u8; 4]).await;
        result.expect("read_raw_frame: length prefix failed");
        let len = u32::from_be_bytes(len_buf.try_into().unwrap());
        let (result, payload) = stream.read_exact(vec![0u8; len as usize]).await;
        result.expect("read_raw_frame: payload failed");
        payload
    }

    fn spawn_server(
        script: impl IntoIterator<Item = Exchange>,
    ) -> (SocketAddr, Rc<RefCell<ClientRegistry>>) {
        let (listener, addr) = bind_loopback();
        let (worker, handle) = build(ScriptedCatSession::with_script(script), &TABLE);
        let registry = Rc::new(RefCell::new(ClientRegistry::new()));
        monoio::spawn(worker.run());
        monoio::spawn(serve(listener, handle, Rc::clone(&registry)));
        (addr, registry)
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn end_to_end_query_round_trip_over_real_loopback_tcp() {
        let (addr, _registry) = spawn_server([Exchange::new("FA;", "FA00014250000;")]);

        let mut stream = TcpStream::connect(addr).await.expect("connect failed");
        write_raw_frame(&mut stream, b"FA;").await;
        let response = read_raw_frame(&mut stream).await;

        assert_eq!(response, b"FA00014250000;");
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn end_to_end_set_gets_explicit_empty_response_frame() {
        let (addr, _registry) = spawn_server([Exchange::new("TX;", "")]);

        let mut stream = TcpStream::connect(addr).await.expect("connect failed");
        write_raw_frame(&mut stream, b"TX;").await;
        let response = read_raw_frame(&mut stream).await;

        assert!(response.is_empty());
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn end_to_end_malformed_request_gets_error_frame_not_forwarded_to_radio() {
        let (addr, _registry) = spawn_server([Exchange::new("FA;", "FA00014250000;")]);

        let mut stream = TcpStream::connect(addr).await.expect("connect failed");
        write_raw_frame(&mut stream, b"ZZ;").await;
        let response = read_raw_frame(&mut stream).await;
        assert!(response.starts_with(b"ERR "));

        // The scripted exchange for FA is still there afterward -- proof
        // the malformed request never touched the physical session.
        write_raw_frame(&mut stream, b"FA;").await;
        let response = read_raw_frame(&mut stream).await;
        assert_eq!(response, b"FA00014250000;");
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn end_to_end_two_concurrent_connections_get_correctly_correlated_responses() {
        let (addr, registry) = spawn_server([
            Exchange::new("FA;", "FA00014250000;"),
            Exchange::new("IF;", "IF017;"),
        ]);

        let mut stream_a = TcpStream::connect(addr).await.expect("connect a failed");
        let mut stream_b = TcpStream::connect(addr).await.expect("connect b failed");

        let task_a = monoio::spawn(async move {
            write_raw_frame(&mut stream_a, b"FA;").await;
            read_raw_frame(&mut stream_a).await
        });
        let task_b = monoio::spawn(async move {
            write_raw_frame(&mut stream_b, b"IF;").await;
            read_raw_frame(&mut stream_b).await
        });

        let response_a = task_a.await;
        let response_b = task_b.await;

        assert_eq!(response_a, b"FA00014250000;");
        assert_eq!(response_b, b"IF017;");
        assert_eq!(registry.borrow().active_count(), 2);
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn end_to_end_disconnect_mid_stream_does_not_wedge_the_server() {
        let (addr, registry) = spawn_server([Exchange::new("FA;", "FA00014250000;")]);

        {
            let stream = TcpStream::connect(addr).await.expect("connect failed");
            // Drop immediately without sending anything -- a disconnect
            // before any request frame arrives.
            drop(stream);
        }

        // The server must still be alive and correctly serve a fresh
        // connection afterward.
        let mut stream = TcpStream::connect(addr)
            .await
            .expect("second connect failed");
        write_raw_frame(&mut stream, b"FA;").await;
        let response = read_raw_frame(&mut stream).await;
        assert_eq!(response, b"FA00014250000;");
        let _ = registry;
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn end_to_end_oversized_frame_gets_error_response_then_connection_closes() {
        let (addr, _registry) = spawn_server([]);

        let mut stream = TcpStream::connect(addr).await.expect("connect failed");
        let oversized_len: u32 = MAX_FRAME_SIZE + 1;
        let (result, _) = stream.write_all(oversized_len.to_be_bytes().to_vec()).await;
        result.expect("write oversized length prefix failed");
        // Deliberately send no payload bytes -- the listener must reject
        // based on the length prefix alone, without waiting to read a
        // payload that will never come.

        let response = read_raw_frame(&mut stream).await;
        assert!(response.starts_with(b"ERR "));
    }
}
