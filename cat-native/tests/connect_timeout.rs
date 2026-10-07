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

//! `connect_timeout`: one bound on connect *and* handshake, for a client
//! (the CW trainer) that must not hang on a server that accepts and then
//! says nothing.

use std::io::Write;
use std::net::TcpListener;
use std::time::{Duration, Instant};

use cat_native::testing::{serve_stub, StubHost};
use cat_native::{encode_frame, Client, ClientError, Connection, FrameKind, Streams};

fn timed_out(r: Result<Connection, ClientError>) -> bool {
    matches!(r, Err(ClientError::Io(e)) if e.kind() == std::io::ErrorKind::TimedOut)
}

/// A server that accepts and then never answers `Hello`.
fn silent() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming().flatten() {
            held.push(stream); // keep it open, say nothing
        }
    });
    port
}

/// A server that starts a well-formed control frame promising a long
/// payload, then delivers it one byte at a time -- too slowly to ever
/// finish, and often enough that a per-read timeout alone never fires.
fn dripping() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let header = &encode_frame(FrameKind::Control, &[b' '; 4_096])[..5];
                if stream.write_all(header).is_err() {
                    return;
                }
                // Capped, so a regression fails this test rather than
                // hanging the suite.
                for _ in 0..100 {
                    if stream.write_all(b" ").is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            });
        }
    });
    port
}

#[test]
fn a_server_that_never_answers_hello_times_out_within_the_bound() {
    let port = silent();
    let started = Instant::now();
    let r = Connection::connect_timeout(
        ("127.0.0.1", port),
        Streams::none(),
        Duration::from_millis(300),
    );
    let took = started.elapsed();
    assert!(timed_out(r), "a timeout, not a hang or another error");
    assert!(took >= Duration::from_millis(300), "{took:?}");
    assert!(took < Duration::from_secs(2), "{took:?}");
}

#[test]
fn a_dripping_server_cannot_stretch_the_bound() {
    let port = dripping();
    let started = Instant::now();
    let r = Connection::connect_timeout(
        ("127.0.0.1", port),
        Streams::none(),
        Duration::from_millis(400),
    );
    let took = started.elapsed();
    assert!(timed_out(r), "the bound, not a decode error");
    assert!(took >= Duration::from_millis(400), "{took:?}");
    assert!(took < Duration::from_millis(1_500), "{took:?}");
}

#[test]
fn a_real_server_connects_and_then_behaves_like_any_connection() {
    let addr = serve_stub(StubHost::new());
    let mut conn =
        Connection::connect_timeout(addr.as_str(), Streams::none(), Duration::from_millis(500))
            .expect("connects");
    assert_eq!(conn.capabilities().model, "Stub Radio");
    // Well past the bound: nothing of it is left on the connection.
    std::thread::sleep(Duration::from_millis(700));
    conn.ping()
        .expect("still usable after the bound has passed");
    conn.read_state().expect("and answers reads");
}

#[test]
fn nobody_listening_fails_promptly_rather_than_waiting_out_the_bound() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let started = Instant::now();
    let r =
        Connection::connect_timeout(("127.0.0.1", port), Streams::none(), Duration::from_secs(5));
    assert!(r.is_err());
    // Under the 5 s bound. Not tighter: Windows retries a refused SYN to
    // localhost for about two seconds before giving up.
    assert!(started.elapsed() < Duration::from_secs(4));
}

#[test]
fn the_threaded_client_has_the_same_bound() {
    let port = silent();
    let started = Instant::now();
    let r = Client::connect_timeout(
        ("127.0.0.1", port),
        Streams::none(),
        Duration::from_millis(300),
    );
    assert!(matches!(r, Err(ClientError::Io(ref e)) if e.kind() == std::io::ErrorKind::TimedOut));
    assert!(started.elapsed() < Duration::from_secs(2));

    let addr = serve_stub(StubHost::new());
    let client = Client::connect_timeout(addr.as_str(), Streams::none(), Duration::from_secs(2))
        .expect("connects");
    assert_eq!(client.capabilities().model, "Stub Radio");
}
