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

//! ADR 0023 §6 / T4: `serve` tells a host *which* console is speaking.
//!
//! The CW arm has one owner, and an owner that hangs up must be noticed
//! as that owner -- so a host needs an identity per connection, on every
//! path in and out, including the ones that end in an error.

use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cat_framework::capabilities::RadioCapabilities;
use cat_native::testing::STUB_RADIO;
use cat_native::{
    serve, Command, Connection, ConsoleId, ErrorCode, HostRefusal, ModeId, RadioHost, RadioState,
    ServerMessage, Streams,
};

fn state() -> RadioState {
    RadioState {
        vfo_a_hz: 14_074_000,
        vfo_b_hz: 7_074_000,
        mode: ModeId::Usb,
        split: false,
        transmitting: false,
        memory_channel: None,
        if_shift_hz: Some(0),
        filter_width_hz: None,
        meters: Vec::new(),
        levels: None,
        cw: None,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Seen {
    Attached(ConsoleId, SocketAddr),
    Applied(ConsoleId, Command),
    State(ConsoleId),
    Detached(ConsoleId),
}

#[derive(Default)]
struct Recording {
    seen: Mutex<Vec<Seen>>,
    refuse_with: Mutex<Option<HostRefusal>>,
}

impl Recording {
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
    fn push(&self, s: Seen) {
        self.seen.lock().unwrap().push(s);
    }
}

impl RadioHost for Recording {
    fn capabilities(&self) -> &'static RadioCapabilities {
        &STUB_RADIO
    }
    fn state(&self) -> RadioState {
        unreachable!("serve asks state_as, and this host answers it")
    }
    fn state_as(&self, id: ConsoleId) -> RadioState {
        self.push(Seen::State(id));
        state()
    }
    fn apply(&self, _: &Command) -> Result<(), String> {
        unreachable!("serve asks apply_as, and this host answers it")
    }
    fn apply_as(&self, id: ConsoleId, command: &Command) -> Result<(), HostRefusal> {
        self.push(Seen::Applied(id, command.clone()));
        match self.refuse_with.lock().unwrap().clone() {
            Some(refusal) => Err(refusal),
            None => Ok(()),
        }
    }
    fn console_attached_as(&self, id: ConsoleId, peer: SocketAddr) {
        self.push(Seen::Attached(id, peer));
    }
    fn console_detached_as(&self, id: ConsoleId) {
        self.push(Seen::Detached(id));
    }
}

fn listen<H: RadioHost>(host: Arc<H>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || serve(listener, host));
    port
}

fn connect(port: u16) -> Connection {
    Connection::connect(("127.0.0.1", port), Streams::none()).expect("connect")
}

/// Wait for something a server thread does after the client has gone.
fn eventually(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn id_of(seen: &[Seen], n: usize) -> ConsoleId {
    seen.iter()
        .filter_map(|s| match s {
            Seen::Attached(id, _) => Some(*id),
            _ => None,
        })
        .nth(n)
        .expect("attached")
}

#[test]
fn each_connection_has_its_own_id_on_every_call() {
    let host = Arc::new(Recording::default());
    let port = listen(Arc::clone(&host));

    let mut a = connect(port);
    let mut b = connect(port);
    a.command(Command::SetSplit { enabled: true }).unwrap();
    b.command(Command::SetSplit { enabled: false }).unwrap();
    a.read_state().unwrap();

    let seen = host.seen();
    let (ia, ib) = (id_of(&seen, 0), id_of(&seen, 1));
    assert_ne!(ia, ib);
    // Which attached first is a race between two server threads; what
    // matters is that each command carries its own sender's id.
    let applied: Vec<_> = seen
        .iter()
        .filter_map(|s| match s {
            Seen::Applied(id, Command::SetSplit { enabled }) => Some((*id, *enabled)),
            _ => None,
        })
        .collect();
    assert_eq!(applied.len(), 2);
    assert_ne!(applied[0].0, applied[1].0);
    assert!(applied.iter().all(|(id, _)| *id == ia || *id == ib));
    assert!(seen
        .iter()
        .any(|s| matches!(s, Seen::State(id) if *id == ia || *id == ib)));

    // The peer is the client's own address, as the server saw it.
    for s in &seen {
        if let Seen::Attached(_, peer) = s {
            assert!(peer.ip().is_loopback());
        }
    }

    drop(a);
    drop(b);
    eventually("both detaches", || {
        let seen = host.seen();
        [ia, ib]
            .iter()
            .all(|id| seen.contains(&Seen::Detached(*id)))
    });
}

#[test]
fn a_connection_that_ends_in_an_error_is_still_detached_once() {
    let host = Arc::new(Recording::default());
    let port = listen(Arc::clone(&host));

    // Not a client at all: a frame header promising more than the server
    // accepts, then gone.
    let mut raw = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let _ = raw.write_all(&[0xFF; 16]);
    drop(raw);

    eventually("the detach", || {
        host.seen().iter().any(|s| matches!(s, Seen::Detached(_)))
    });
    let seen = host.seen();
    let id = id_of(&seen, 0);
    let detaches = seen.iter().filter(|s| **s == Seen::Detached(id)).count();
    assert_eq!(detaches, 1);
}

#[test]
fn a_host_refusal_carries_its_own_code() {
    let host = Arc::new(Recording::default());
    *host.refuse_with.lock().unwrap() = Some(HostRefusal {
        code: ErrorCode::Forbidden,
        message: "armed by another console".to_string(),
    });
    let port = listen(Arc::clone(&host));
    let mut conn = connect(port);
    match conn.command(Command::SetSplit { enabled: true }).unwrap() {
        ServerMessage::Error { code, message } => {
            assert_eq!(code, ErrorCode::Forbidden);
            assert_eq!(message, "armed by another console");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_plain_string_refusal_is_out_of_range_as_it_always_was() {
    let refusal: HostRefusal = "channel 12 is empty".to_string().into();
    assert_eq!(refusal.code, ErrorCode::OutOfRange);
    assert_eq!(refusal.message, "channel 12 is empty");
}

/// A host written before T4: overrides none of the `_as` methods.
#[derive(Default)]
struct OldHost {
    attached: AtomicUsize,
    detached: AtomicUsize,
    applied: Mutex<Vec<Command>>,
    refuse: bool,
}

impl RadioHost for OldHost {
    fn capabilities(&self) -> &'static RadioCapabilities {
        &STUB_RADIO
    }
    fn state(&self) -> RadioState {
        state()
    }
    fn apply(&self, command: &Command) -> Result<(), String> {
        self.applied.lock().unwrap().push(command.clone());
        if self.refuse {
            Err("no".to_string())
        } else {
            Ok(())
        }
    }
    fn console_attached(&self) {
        self.attached.fetch_add(1, Ordering::SeqCst);
    }
    fn console_detached(&self) {
        self.detached.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn a_host_that_overrides_nothing_new_behaves_exactly_as_before() {
    let host = Arc::new(OldHost::default());
    let port = listen(Arc::clone(&host));
    let mut conn = connect(port);
    assert_eq!(
        conn.command(Command::SetSplit { enabled: true }).unwrap(),
        ServerMessage::Ack
    );
    assert_eq!(conn.read_state().unwrap(), state());
    assert_eq!(host.attached.load(Ordering::SeqCst), 1);
    drop(conn);
    eventually("the detach", || host.detached.load(Ordering::SeqCst) == 1);
    assert_eq!(
        *host.applied.lock().unwrap(),
        vec![Command::SetSplit { enabled: true }]
    );

    let refusing = Arc::new(OldHost {
        refuse: true,
        ..OldHost::default()
    });
    let port = listen(refusing);
    match connect(port)
        .command(Command::SetSplit { enabled: true })
        .unwrap()
    {
        ServerMessage::Error { code, message } => {
            assert_eq!(code, ErrorCode::OutOfRange);
            assert_eq!(message, "no");
        }
        other => panic!("{other:?}"),
    }
}
