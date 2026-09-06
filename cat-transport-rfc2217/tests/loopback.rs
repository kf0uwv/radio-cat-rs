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

//! The client and the device server, over a real socket.
//!
//! The unit tests either side of this prove each half against byte arrays.
//! What they cannot prove is that the two halves agree, that a line driven
//! from a synchronous method reaches a peer that is busy, and that framing
//! sits correctly on top — which is the whole proposition of the crate.
//!
//! So the client here is the real [`Rfc2217Port`] under the real
//! `SerialCatSession`, and the server is [`Rfc2217Peer`] behind an ordinary
//! `TcpListener`, in about the shape a device server actually has.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cat_transport_core::{CatSession, ModemControlLines, Transport};
use cat_transport_rfc2217::codec::modem;
use cat_transport_rfc2217::{PeerEvent, Rfc2217Config, Rfc2217Peer, Rfc2217Port};
use cat_transport_serial::SerialCatSession;

/// What the fake radio behind the device server did, so a test can assert
/// on it from the outside.
#[derive(Default)]
struct Observed {
    dtr: Vec<bool>,
    rts: Vec<bool>,
    commands: Vec<String>,
}

/// A device server with a very small radio behind it: it answers `FA;` and
/// records the lines it was driven with.
struct Fake {
    addr: String,
    observed: Arc<Mutex<Observed>>,
    /// Set by a test to make the server publish a modem state.
    cts: Arc<AtomicBool>,
}

fn spawn_server() -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let observed = Arc::new(Mutex::new(Observed::default()));
    let cts = Arc::new(AtomicBool::new(false));

    let server_observed = Arc::clone(&observed);
    let server_cts = Arc::clone(&cts);
    std::thread::spawn(move || {
        // A device server serves whoever connects, for as long as it runs.
        // Accepting exactly once looks equivalent until a test opens a
        // second port and waits forever for a server that already stopped
        // listening.
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let observed = Arc::clone(&server_observed);
            let cts = Arc::clone(&server_cts);
            std::thread::spawn(move || serve_one(stream, observed, cts));
        }
    });

    Fake {
        addr,
        observed,
        cts,
    }
}

fn serve_one(mut stream: TcpStream, observed: Arc<Mutex<Observed>>, cts: Arc<AtomicBool>) {
    stream.set_nodelay(true).ok();
    // Short reads so the loop can also publish modem state while the
    // client is quiet — exactly the situation a real device server is in.
    stream
        .set_read_timeout(Some(Duration::from_millis(10)))
        .ok();

    let mut peer = Rfc2217Peer::new();
    let mut buf = [0u8; 1024];
    let mut last_cts = None;

    loop {
        match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                let (events, reply) = peer.push(&buf[..n]);
                if !reply.is_empty() && stream.write_all(&reply).is_err() {
                    return;
                }
                for event in events {
                    match event {
                        PeerEvent::Dtr(on) => observed.lock().unwrap().dtr.push(on),
                        PeerEvent::Rts(on) => observed.lock().unwrap().rts.push(on),
                        PeerEvent::Data(data) => {
                            let text = String::from_utf8_lossy(&data).into_owned();
                            observed.lock().unwrap().commands.push(text.clone());
                            // The one command this radio knows.
                            if text.starts_with("FA") {
                                let out = peer.encode_data(b"FA00014250000;");
                                if stream.write_all(&out).is_err() {
                                    return;
                                }
                            }
                        }
                        PeerEvent::Break(_) => {}
                    }
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => return,
        }

        // Publish the line state whenever it moves.
        let now = cts.load(Ordering::SeqCst);
        if last_cts != Some(now) {
            last_cts = Some(now);
        }
        let state = if now { modem::CTS } else { 0 };
        if let Some(bytes) = peer.modem_state(state) {
            if stream.write_all(&bytes).is_err() {
                return;
            }
        }
    }
}

fn connect(fake: &Fake, config: Rfc2217Config) -> Rfc2217Port {
    Rfc2217Port::connect(fake.addr.as_str(), config).expect("connect to the device server")
}

/// Spin until `f` holds, or fail. The socket and the reader thread are
/// genuinely concurrent, so there is nothing to synchronise on.
fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn a_cat_command_round_trips_through_the_framing_layer() {
    // The proposition of the crate: an RFC 2217 endpoint is a serial port,
    // so the ordinary serial framing sits on it unchanged.
    let fake = spawn_server();
    let mut session = SerialCatSession::new(connect(&fake, Rfc2217Config::default()));

    let mut response = Vec::new();
    futures::executor::block_on(session.execute(b"FA;", &mut response)).expect("execute");

    assert_eq!(response, b"FA00014250000;");
    assert_eq!(
        fake.observed.lock().unwrap().commands,
        vec!["FA;".to_string()]
    );
}

#[test]
fn opening_leaves_dtr_low_and_rts_high() {
    // The TS-570D's two facts, per the ACC2-IF datasheet: RTS is
    // receive-enable and must be up, and DTR keys PTT so it must not be.
    let fake = spawn_server();
    let _port = connect(&fake, Rfc2217Config::default());

    eventually("the opening line states", || {
        let observed = fake.observed.lock().unwrap();
        observed.dtr == vec![false] && observed.rts == vec![true]
    });
}

#[test]
fn keying_reaches_the_peer_while_the_reader_is_parked() {
    // This is the scenario that decided the crate's shape. The fake radio
    // says nothing unless asked, so from the moment it connects the reader
    // thread is parked inside a blocking socket read that will not return
    // for the length of the test. A design that funnelled writes and reads
    // through one worker thread would have `set_dtr` queued behind that
    // read and the transmitter would never key.
    let fake = spawn_server();
    let port = connect(&fake, Rfc2217Config::default());

    port.set_dtr(true).expect("set_dtr");
    eventually("DTR to be asserted", || {
        fake.observed.lock().unwrap().dtr == vec![false, true]
    });

    port.set_dtr(false).expect("set_dtr");
    eventually("DTR to be released", || {
        fake.observed.lock().unwrap().dtr == vec![false, true, false]
    });
}

#[test]
fn cts_becomes_readable_after_the_peer_notifies() {
    let fake = spawn_server();
    let port = connect(&fake, Rfc2217Config::default());

    assert!(!port.read_cts().expect("read_cts"), "CTS starts low");

    fake.cts.store(true, Ordering::SeqCst);
    eventually("CTS to be reported", || port.read_cts().unwrap_or(false));

    fake.cts.store(false, Ordering::SeqCst);
    eventually("CTS to drop", || !port.read_cts().unwrap_or(true));
}

#[test]
fn the_lines_are_reachable_through_the_framing_layer_too() {
    // `SerialCatSession`'s blanket forwarding impl is what lets an
    // application hold one value and both talk CAT and key PTT with it.
    let fake = spawn_server();
    let session = SerialCatSession::new(connect(&fake, Rfc2217Config::default()));

    session.set_dtr(true).expect("set_dtr through the session");
    eventually("DTR driven through the session", || {
        fake.observed.lock().unwrap().dtr.contains(&true)
    });
}

#[test]
fn a_255_byte_in_a_cat_response_is_not_mistaken_for_telnet() {
    // Not a real Kenwood response, but the transparency rule has to hold
    // for any byte or the framing layer above will see a truncated one.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();

    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut peer = Rfc2217Peer::new();
        let mut buf = [0u8; 1024];
        loop {
            let Ok(n) = stream.read(&mut buf) else { return };
            if n == 0 {
                return;
            }
            let (events, reply) = peer.push(&buf[..n]);
            if !reply.is_empty() && stream.write_all(&reply).is_err() {
                return;
            }
            for event in events {
                if let PeerEvent::Data(_) = event {
                    let out = peer.encode_data(&[b'F', 255, b'A', b';']);
                    if stream.write_all(&out).is_err() {
                        return;
                    }
                }
            }
        }
    });

    let mut port = Rfc2217Port::connect(addr.as_str(), Rfc2217Config::default()).expect("connect");
    futures::executor::block_on(port.write(b"FA;")).expect("write");

    let mut got = Vec::new();
    let mut buf = [0u8; 16];
    while !got.ends_with(b";") {
        let n = futures::executor::block_on(port.read(&mut buf)).expect("read");
        got.extend_from_slice(&buf[..n]);
    }
    assert_eq!(got, vec![b'F', 255, b'A', b';']);
}

#[test]
fn dropping_the_port_disconnects_the_peer() {
    // Not tidiness. The reader thread holds its own reference to the
    // socket, so without an explicit shutdown the peer never learns the
    // client has gone -- and on a station whose DTR keys PTT, a peer that
    // still believes DTR is asserted is a transmitter left keyed by a
    // program that has already exited.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let disconnected = Arc::new(AtomicBool::new(false));

    let flag = Arc::clone(&disconnected);
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut buf = [0u8; 1024];
        loop {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => {
                    flag.store(true, Ordering::SeqCst);
                    return;
                }
                Ok(_) => {}
            }
        }
    });

    let port = Rfc2217Port::connect(addr.as_str(), Rfc2217Config::default()).expect("connect");
    assert!(!disconnected.load(Ordering::SeqCst));

    drop(port);
    eventually("the peer to see the disconnect", || {
        disconnected.load(Ordering::SeqCst)
    });
}

#[test]
fn the_lines_are_readable_the_instant_connect_returns() {
    // Regression. `read_cts()` used to report every line low straight after
    // connecting -- not because they were, but because no
    // NOTIFY-MODEMSTATE had arrived yet. The tool most likely to ask is a
    // bench utility that prints CTS/DSR/DCD as its first act, so "not yet"
    // reaching it as "deasserted" is a wrong answer to the question it
    // actually asked. Found by running `ts570d-line <addr> status` against
    // the emulator and being told CTS was down while the radio held it up.
    let fake = spawn_server();
    fake.cts.store(true, Ordering::SeqCst);

    let port = connect(&fake, Rfc2217Config::default());
    assert!(
        port.modem_state_reported(),
        "connect must not return before the peer has reported the lines"
    );
    assert!(
        port.read_cts().expect("read_cts"),
        "CTS was asserted before this client connected; it must read as asserted"
    );
}
