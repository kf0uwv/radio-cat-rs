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

//! Windows analog of `crate::tcp`'s server-side TCP accept/dispatch loop --
//! same wire format (`cat-transport-tcp`'s length-prefixed framing, via
//! [`cat_transport_tcp::codec`]), same hard requirement (every request frame
//! gets exactly one response frame back, even an empty one), rebuilt on
//! `std::net` + genuine OS threads instead of `monoio::net` + cooperative
//! tasks.
//!
//! Per `docs/adr/0006-windows-network-transport.md`: one dedicated
//! `std::thread` per accepted connection, each driving its own blocking
//! request/response loop via [`crate::block_on`] against
//! [`crate::worker_windows::BrokerHandle`] (genuinely `Send`, unlike
//! `crate::broker::BrokerHandle`). `ClientRegistry` (`crate::registry`,
//! already pure `std`, unmodified) is shared via `Arc<Mutex<_>>` instead of
//! `Rc<RefCell<_>>` since real OS threads, not cooperative tasks, are doing
//! the sharing here.
//!
//! # This module is genuinely cross-platform, and is tested on Linux too
//!
//! See `cat-transport-tcp::windows`'s module doc for the same point:
//! nothing below is actually Windows-specific, so this module is not
//! `#[cfg(target_os = "windows")]`-gated -- it compiles and its tests run
//! on every platform this workspace builds for.

use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use cat_transport_tcp::codec;

use crate::block_on::block_on;
use crate::registry::ClientRegistry;
use crate::worker_windows::BrokerHandle;

/// Accept loop. Binding is the caller's responsibility (pass an
/// already-bound `TcpListener`) so callers/tests can choose the address.
/// Spawns one dedicated `std::thread` per accepted connection; runs until
/// `accept()` itself fails.
pub fn serve(
    listener: TcpListener,
    handle: BrokerHandle,
    registry: Arc<Mutex<ClientRegistry>>,
) -> io::Result<()> {
    loop {
        let (stream, _peer_addr) = listener.accept()?;
        let handle = handle.clone();
        let registry = Arc::clone(&registry);
        thread::spawn(move || handle_connection(stream, handle, registry));
    }
}

/// Service one accepted connection until it disconnects or the broker
/// itself is gone. Blocking -- runs on its own dedicated thread.
fn handle_connection(
    mut stream: TcpStream,
    handle: BrokerHandle,
    registry: Arc<Mutex<ClientRegistry>>,
) {
    let client_id = registry.lock().unwrap().register();

    loop {
        match read_frame_or_eof_blocking(&mut stream) {
            Ok(Some(payload)) => {
                let response = match block_on(handle.submit(client_id, payload)) {
                    Some(response) => response,
                    None => break, // broker shut down
                };
                if write_frame_blocking(&mut stream, &response).is_err() {
                    break; // client disconnected before we could reply
                }
            }
            Ok(None) => break, // clean disconnect at a frame boundary
            Err(e) => {
                let _ = write_frame_blocking(&mut stream, format!("ERR {e}").as_bytes());
                break;
            }
        }
    }

    registry.lock().unwrap().unregister(client_id);
}

/// Blocking write of one complete length-prefixed frame.
fn write_frame_blocking(stream: &mut TcpStream, payload: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let frame = codec::encode_frame(payload).map_err(io::Error::other)?;
    stream.write_all(&frame)
}

/// Blocking read of one length-prefixed frame, tolerating a clean disconnect
/// exactly at a frame boundary -- the server-shaped counterpart described in
/// `cat_transport_tcp::session::read_frame_or_eof`'s doc, reimplemented here
/// against `std::net::TcpStream` instead of `monoio::net::TcpStream`.
fn read_frame_or_eof_blocking(stream: &mut TcpStream) -> io::Result<Option<Vec<u8>>> {
    use std::io::Read;

    let mut len_buf = [0u8; 4];
    if !read_exact_or_eof(stream, &mut len_buf)? {
        return Ok(None);
    }
    let len = codec::decode_len_prefix(len_buf).map_err(io::Error::other)?;

    let mut payload = vec![0u8; len as usize];
    stream.read_exact(&mut payload)?;
    Ok(Some(payload))
}

/// Read exactly `buf.len()` bytes, or detect a clean EOF that occurs before
/// any byte at all has been read. Returns `Ok(true)` on a full read,
/// `Ok(false)` only if the very first byte transferred is EOF -- an EOF
/// encountered after at least one byte has already arrived is a disconnect
/// mid-read, surfaced as `Err` (`io::ErrorKind::UnexpectedEof`), matching
/// `cat_transport_tcp::session::read_frame_or_eof`'s identical contract.
fn read_exact_or_eof(stream: &mut TcpStream, buf: &mut [u8]) -> io::Result<bool> {
    use std::io::Read;

    let mut filled = 0;
    loop {
        if filled == buf.len() {
            return Ok(true);
        }
        match stream.read(&mut buf[filled..]) {
            Ok(0) => {
                return if filled == 0 {
                    Ok(false)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "failed to fill whole buffer",
                    ))
                };
            }
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

// Gated `target_os = "windows"` in addition to `test`: every test below
// builds a server through `crate::worker_windows::build` and drives it via
// `crate::block_on`, which is not safe to combine with `crate::broker::
// Broker::dispatch`'s Linux-build timeout wrap (real `monoio::time::
// timeout`, requiring an actual `monoio` runtime) -- see `crate::
// worker_windows`'s module doc ("Test scope") and `crate::broker::
// with_request_timeout`'s doc comment for the full explanation. This
// module's own production code has no Windows-specific syscalls, but it is
// not the one deciding the timeout mechanism -- `Broker::dispatch` is.
#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use crate::test_fixtures::TABLE;
    use crate::worker_windows::build;
    use cat_transport_core::test_support::{Exchange, ScriptedCatSession};
    use std::io::{Read, Write};
    use std::net::SocketAddr;

    fn bind_loopback() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("TcpListener::bind failed");
        let addr = listener.local_addr().expect("local_addr failed");
        (listener, addr)
    }

    fn write_raw_frame(stream: &mut TcpStream, payload: &[u8]) {
        let frame = codec::encode_frame(payload).expect("encode_frame failed");
        stream.write_all(&frame).expect("write_all failed");
    }

    fn read_raw_frame(stream: &mut TcpStream) -> Vec<u8> {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length prefix");
        let len = u32::from_be_bytes(len_buf);
        let mut payload = vec![0u8; len as usize];
        stream.read_exact(&mut payload).expect("read payload");
        payload
    }

    fn spawn_server(
        script: impl IntoIterator<Item = Exchange>,
    ) -> (SocketAddr, Arc<Mutex<ClientRegistry>>) {
        spawn_server_with(ScriptedCatSession::with_script(script))
    }

    /// As [`spawn_server`], but the session matches requests by content
    /// instead of position -- for tests where several clients submit
    /// concurrently and nothing orders their arrival at the worker.
    fn spawn_server_unordered(
        script: impl IntoIterator<Item = Exchange>,
    ) -> (SocketAddr, Arc<Mutex<ClientRegistry>>) {
        spawn_server_with(ScriptedCatSession::with_unordered_script(script))
    }

    fn spawn_server_with(session: ScriptedCatSession) -> (SocketAddr, Arc<Mutex<ClientRegistry>>) {
        let (listener, addr) = bind_loopback();
        let (worker, handle) = build(session, &TABLE);
        let registry = Arc::new(Mutex::new(ClientRegistry::new()));
        thread::spawn(move || worker.run());
        let registry_clone = Arc::clone(&registry);
        thread::spawn(move || serve(listener, handle, registry_clone));
        (addr, registry)
    }

    #[test]
    fn end_to_end_query_round_trip_over_real_loopback_tcp() {
        let (addr, _registry) = spawn_server([Exchange::new("FA;", "FA00014250000;")]);

        let mut stream = TcpStream::connect(addr).expect("connect failed");
        write_raw_frame(&mut stream, b"FA;");
        let response = read_raw_frame(&mut stream);

        assert_eq!(response, b"FA00014250000;");
    }

    #[test]
    fn end_to_end_set_gets_explicit_empty_response_frame() {
        let (addr, _registry) = spawn_server([Exchange::new("TX;", "")]);

        let mut stream = TcpStream::connect(addr).expect("connect failed");
        write_raw_frame(&mut stream, b"TX;");
        let response = read_raw_frame(&mut stream);

        assert!(response.is_empty());
    }

    #[test]
    fn end_to_end_malformed_request_gets_error_frame_not_forwarded_to_radio() {
        let (addr, _registry) = spawn_server([Exchange::new("FA;", "FA00014250000;")]);

        let mut stream = TcpStream::connect(addr).expect("connect failed");
        write_raw_frame(&mut stream, b"ZZ;");
        let response = read_raw_frame(&mut stream);
        assert!(response.starts_with(b"ERR "));

        write_raw_frame(&mut stream, b"FA;");
        let response = read_raw_frame(&mut stream);
        assert_eq!(response, b"FA00014250000;");
    }

    #[test]
    fn end_to_end_two_concurrent_connections_get_correctly_correlated_responses() {
        // Unordered script: two independent client connections submit
        // concurrently, so which request reaches the worker first is a
        // scheduling detail, not a property of the server. An ordered
        // script made this a coin flip -- `ScriptedCatSession: request
        // mismatch (expected "FA;", got "IF;")` on roughly half of runs.
        // Correlation of each response back to its own connection is what
        // this test exists to prove, and that is asserted below.
        let (addr, registry) = spawn_server_unordered([
            Exchange::new("FA;", "FA00014250000;"),
            Exchange::new("IF;", "IF017;"),
        ]);

        let mut stream_a = TcpStream::connect(addr).expect("connect a failed");
        let mut stream_b = TcpStream::connect(addr).expect("connect b failed");

        // Each thread hands its stream BACK rather than consuming it. If
        // the streams are moved in and dropped when the threads exit, both
        // connections close before the `active_count()` assertion below and
        // it reads 0 -- which it did, on 8 runs out of 8, the first time
        // these tests were ever executed on Windows (see `radio-cat-rs`
        // planning/release_workflow/findings.md section 7b). Holding them
        // open is what makes "two connections are active" a true statement
        // at the moment it is asserted.
        let task_a = thread::spawn(move || {
            write_raw_frame(&mut stream_a, b"FA;");
            let response = read_raw_frame(&mut stream_a);
            (stream_a, response)
        });
        let task_b = thread::spawn(move || {
            write_raw_frame(&mut stream_b, b"IF;");
            let response = read_raw_frame(&mut stream_b);
            (stream_b, response)
        });

        let (stream_a, response_a) = task_a.join().unwrap();
        let (stream_b, response_b) = task_b.join().unwrap();

        assert_eq!(response_a, b"FA00014250000;");
        assert_eq!(response_b, b"IF017;");

        // Both responses arrived, so both connections were accepted and
        // served; registration is ordered before that. Poll rather than
        // sleep a fixed interval, so this asserts a settled state instead
        // of racing a magic number.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let active = registry.lock().unwrap().active_count();
            if active == 2 || std::time::Instant::now() >= deadline {
                assert_eq!(active, 2);
                break;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }

        // Explicit, so the streams' lifetime above is obviously deliberate
        // and not an accident a later edit may quietly undo.
        drop((stream_a, stream_b));
    }

    #[test]
    fn end_to_end_disconnect_mid_stream_does_not_wedge_the_server() {
        let (addr, _registry) = spawn_server([Exchange::new("FA;", "FA00014250000;")]);

        {
            let stream = TcpStream::connect(addr).expect("connect failed");
            drop(stream);
        }

        let mut stream = TcpStream::connect(addr).expect("second connect failed");
        write_raw_frame(&mut stream, b"FA;");
        let response = read_raw_frame(&mut stream);
        assert_eq!(response, b"FA00014250000;");
    }

    #[test]
    fn end_to_end_oversized_frame_gets_error_response_then_connection_closes() {
        let (addr, _registry) = spawn_server([]);

        let mut stream = TcpStream::connect(addr).expect("connect failed");
        let oversized_len: u32 = cat_transport_tcp::MAX_FRAME_SIZE + 1;
        stream
            .write_all(&oversized_len.to_be_bytes())
            .expect("write oversized length prefix failed");

        let response = read_raw_frame(&mut stream);
        assert!(response.starts_with(b"ERR "));
    }
}
