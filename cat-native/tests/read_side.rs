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

//! A client and a server driving each other, over a socket, with a radio
//! behind them.
//!
//! Until this landed the protocol was write-only: `ReadMeter` validated
//! that a meter existed and answered `Ack` without a reading, and nothing
//! reported the dial at all. A console on it could send and could not see.
//! These tests are the shape a dummy radio needs to be testable against.

use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cat_framework::capabilities::*;
use cat_native::{
    Command, Connection, ErrorCode, MeterSample, RadioHost, RadioState, ServerMessage,
};
use cat_signal::SpectrumFrame;

const MODES: &[ModeDescriptor] = &[
    ModeDescriptor {
        id: ModeId::Lsb,
        label: "LSB",
        kind: ModeKind::Ssb,
        sideband: Some(Sideband::Lower),
        default_bandwidth_hz: 2400,
    },
    ModeDescriptor {
        id: ModeId::Usb,
        label: "USB",
        kind: ModeKind::Ssb,
        sideband: Some(Sideband::Upper),
        default_bandwidth_hz: 2400,
    },
];

const METERS: &[MeterDescriptor] = &[
    MeterDescriptor {
        kind: MeterKind::S,
        raw_range: RawRange::new(0, 30),
        active_on_transmit: false,
        s_units: Some(SUnitScale::TS570D),
    },
    MeterDescriptor {
        kind: MeterKind::Swr,
        raw_range: RawRange::new(0, 30),
        active_on_transmit: true,
        s_units: None,
    },
];

const ENDPOINTS: &[EndpointDescriptor] = &[EndpointDescriptor {
    role: EndpointRole::Cat,
    required: true,
    shareable_with: &[],
}];

static RADIO: RadioCapabilities = RadioCapabilities {
    model: "Dummy Radio",
    endpoints: EndpointSet::new(ENDPOINTS),
    vfos: VfoCapability {
        count: 2,
        split: true,
        rit_hz: Some(9999),
        xit_hz: Some(9999),
    },
    modes: MODES,
    tuning_steps_hz: &[10, 100],
    rx_range: FrequencyRange::new(500_000, 60_000_000),
    filters: FilterCapability {
        if_shift_hz: Some(1_000),
        widths_hz: None,
        notch: false,
    },
    meters: MeterSet::new(METERS),
    memory: None,
    menu: None,
    signal: SignalSupport::IfTapPoint {
        if_center_hz: 73_050_000,
        inverted: true,
    },
};

/// A radio made of atomics, so `apply` can take `&self` the way the trait
/// asks and every connection sees the same dial.
struct Dummy {
    vfo_a_hz: AtomicU64,
    mode: AtomicU8,
    split: AtomicBool,
    /// `None` makes the S meter unreadable, for the inert-meter case.
    smeter: Mutex<Option<u16>>,
    spectrum: Mutex<Option<SpectrumFrame>>,
    /// Set when `apply` is told to refuse, so a test can exercise the
    /// path where the radio knows something capabilities do not.
    refuse: AtomicBool,
}

impl Dummy {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            vfo_a_hz: AtomicU64::new(14_074_000),
            mode: AtomicU8::new(ModeId::Usb as u8),
            split: AtomicBool::new(false),
            smeter: Mutex::new(Some(17)),
            spectrum: Mutex::new(None),
            refuse: AtomicBool::new(false),
        })
    }
}

impl RadioHost for Dummy {
    fn capabilities(&self) -> &'static RadioCapabilities {
        &RADIO
    }

    fn state(&self) -> RadioState {
        let mut meters = Vec::new();
        if let Some(raw) = *self.smeter.lock().unwrap() {
            meters.push(MeterSample {
                kind: MeterKind::S,
                raw,
            });
        }
        RadioState {
            vfo_a_hz: self.vfo_a_hz.load(Ordering::Relaxed),
            vfo_b_hz: 7_074_000,
            mode: if self.mode.load(Ordering::Relaxed) == ModeId::Lsb as u8 {
                ModeId::Lsb
            } else {
                ModeId::Usb
            },
            split: self.split.load(Ordering::Relaxed),
            transmitting: false,
            memory_channel: None,
            if_shift_hz: Some(0),
            filter_width_hz: None,
            meters,
        }
    }

    fn apply(&self, command: &Command) -> Result<(), String> {
        if self.refuse.load(Ordering::Relaxed) {
            return Err("the radio said no".to_string());
        }
        match command {
            Command::SetFrequency { hz, .. } | Command::Retune { hz } => {
                self.vfo_a_hz.store(*hz, Ordering::Relaxed)
            }
            Command::SetMode { mode } => self.mode.store(*mode as u8, Ordering::Relaxed),
            Command::SetSplit { enabled } => self.split.store(*enabled, Ordering::Relaxed),
            _ => {}
        }
        Ok(())
    }

    fn spectrum(&self) -> Option<SpectrumFrame> {
        self.spectrum.lock().unwrap().clone()
    }
}

fn serve(host: Arc<Dummy>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let _ = cat_native::serve(listener, host);
    });
    port
}

#[test]
fn a_console_can_finally_see_what_the_radio_is_doing() {
    let host = Dummy::new();
    let port = serve(Arc::clone(&host));
    let mut conn = Connection::connect(("127.0.0.1", port), false).expect("connect");

    let state = conn.read_state().expect("read state");
    assert_eq!(state.vfo_a_hz, 14_074_000);
    assert_eq!(state.mode, ModeId::Usb);
    assert!(!state.split);
    assert_eq!(state.meter(MeterKind::S), Some(17));
}

#[test]
fn a_command_changes_what_the_next_read_reports() {
    // The whole loop: send, the host applies, the next read sees it. This
    // is what makes a dummy radio testable rather than merely reachable.
    let host = Dummy::new();
    let port = serve(Arc::clone(&host));
    let mut conn = Connection::connect(("127.0.0.1", port), false).expect("connect");

    assert_eq!(
        conn.command(Command::Retune { hz: 7_030_000 }).unwrap(),
        ServerMessage::Ack
    );
    assert_eq!(conn.read_state().unwrap().vfo_a_hz, 7_030_000);

    conn.command(Command::SetMode { mode: ModeId::Lsb })
        .unwrap();
    assert_eq!(conn.read_state().unwrap().mode, ModeId::Lsb);
}

#[test]
fn reading_a_meter_returns_a_reading_and_not_an_ack() {
    // The bug this whole change exists to fix. `ReadMeter` used to
    // validate that the meter existed and answer `Ack` -- an answer that
    // confirms the question was well-formed and says nothing.
    let host = Dummy::new();
    let port = serve(Arc::clone(&host));
    let mut conn = Connection::connect(("127.0.0.1", port), false).expect("connect");

    match conn
        .command(Command::ReadMeter { kind: MeterKind::S })
        .unwrap()
    {
        ServerMessage::Meter(sample) => {
            assert_eq!(sample.kind, MeterKind::S);
            assert_eq!(sample.raw, 17);
        }
        other => panic!("expected a reading, got {other:?}"),
    }
}

#[test]
fn a_meter_the_radio_lacks_is_still_refused_as_unsupported() {
    // Capability validation has to keep working now that the same command
    // has a real answer.
    let host = Dummy::new();
    let port = serve(Arc::clone(&host));
    let mut conn = Connection::connect(("127.0.0.1", port), false).expect("connect");
    match conn
        .command(Command::ReadMeter {
            kind: MeterKind::Comp,
        })
        .unwrap()
    {
        ServerMessage::Error { code, .. } => assert_eq!(code, ErrorCode::Unsupported),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_meter_the_radio_has_but_is_not_reporting_is_not_unsupported() {
    // A TX meter during receive is the ordinary case. Reporting it as
    // "this radio has no SWR meter" would be false, and a console would
    // reasonably stop drawing the row.
    let host = Dummy::new();
    let port = serve(Arc::clone(&host));
    let mut conn = Connection::connect(("127.0.0.1", port), false).expect("connect");
    match conn
        .command(Command::ReadMeter {
            kind: MeterKind::Swr,
        })
        .unwrap()
    {
        ServerMessage::Error { code, .. } => assert_eq!(code, ErrorCode::NotReady),
        other => panic!("expected NotReady, got {other:?}"),
    }
}

#[test]
fn a_radio_that_refuses_a_command_is_reported_rather_than_silently_ignored() {
    // Capabilities cannot know everything -- an empty memory channel, a
    // mode the radio will not enter on this band. The host gets the last
    // word, and the client hears about it.
    let host = Dummy::new();
    let port = serve(Arc::clone(&host));
    let mut conn = Connection::connect(("127.0.0.1", port), false).expect("connect");
    host.refuse.store(true, Ordering::Relaxed);
    match conn.command(Command::Retune { hz: 7_030_000 }).unwrap() {
        ServerMessage::Error { message, .. } => assert!(message.contains("said no")),
        other => panic!("expected the radio's refusal, got {other:?}"),
    }
    // And the dial did not move.
    host.refuse.store(false, Ordering::Relaxed);
    assert_eq!(conn.read_state().unwrap().vfo_a_hz, 14_074_000);
}

#[test]
fn a_command_the_capability_set_refuses_never_reaches_the_radio() {
    // The point of validating first. A frequency outside coverage must not
    // be applied and then reported as an error.
    let host = Dummy::new();
    let port = serve(Arc::clone(&host));
    let mut conn = Connection::connect(("127.0.0.1", port), false).expect("connect");
    let reply = conn
        .command(Command::SetFrequency {
            vfo: 0,
            hz: 450_000_000,
        })
        .unwrap();
    assert!(matches!(reply, ServerMessage::Error { .. }));
    assert_eq!(host.vfo_a_hz.load(Ordering::Relaxed), 14_074_000);
}

#[test]
fn state_read_before_the_radio_has_said_anything_is_not_ready_rather_than_zero() {
    // A server that has just started has not heard from its radio either,
    // and answering with a zeroed state would be asserting something
    // false. This exercises `NativeSession` directly: over a socket the
    // listener always publishes first, so the window does not exist there.
    let mut session = cat_native::NativeSession::new(&RADIO);
    session.handle(cat_native::ClientMessage::Hello {
        version: cat_native::PROTOCOL_VERSION,
        spectrum: false,
    });
    match session.handle(cat_native::ClientMessage::Command(Command::ReadState)) {
        ServerMessage::Error { code, .. } => assert_eq!(code, ErrorCode::NotReady),
        other => panic!("expected NotReady, got {other:?}"),
    }
}

#[test]
fn spectrum_frames_flow_to_a_client_that_asked_for_them() {
    let host = Dummy::new();
    *host.spectrum.lock().unwrap() = Some(SpectrumFrame {
        center_hz: 14_074_000,
        span_hz: 48_000,
        ref_level_dbm: -100.0,
        sequence: 1,
        bins: vec![-90.0; 8],
    });
    let port = serve(Arc::clone(&host));
    let mut conn = Connection::connect(("127.0.0.1", port), true).expect("connect");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got = None;
    while Instant::now() < deadline && got.is_none() {
        got = conn.poll(Some(Duration::from_millis(100))).unwrap();
    }
    let frame = got.expect("a frame arrived");
    assert_eq!(frame.center_hz, 14_074_000);
    assert_eq!(frame.bins.len(), 8);
}

#[test]
fn the_same_frame_is_not_sent_twice() {
    // A source slower than the pump would otherwise have its newest frame
    // resent every 30 ms, which costs bandwidth and makes a stalled source
    // look live.
    let host = Dummy::new();
    *host.spectrum.lock().unwrap() = Some(SpectrumFrame {
        center_hz: 14_074_000,
        span_hz: 48_000,
        ref_level_dbm: -100.0,
        sequence: 42,
        bins: vec![-90.0; 8],
    });
    let port = serve(Arc::clone(&host));
    let mut conn = Connection::connect(("127.0.0.1", port), true).expect("connect");

    // Take the first one.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut first = None;
    while Instant::now() < deadline && first.is_none() {
        first = conn.poll(Some(Duration::from_millis(100))).unwrap();
    }
    assert_eq!(first.expect("first frame").sequence, 42);

    // The host's frame has not changed, so nothing more should arrive.
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        conn.poll(Some(Duration::from_millis(100)))
            .unwrap()
            .is_none(),
        "an unchanged frame was resent"
    );
}

#[test]
fn a_client_that_declined_spectrum_still_gets_nothing_from_the_listener() {
    let host = Dummy::new();
    *host.spectrum.lock().unwrap() = Some(SpectrumFrame {
        center_hz: 14_074_000,
        span_hz: 48_000,
        ref_level_dbm: -100.0,
        sequence: 1,
        bins: vec![-90.0; 8],
    });
    let port = serve(Arc::clone(&host));
    let mut conn = Connection::connect(("127.0.0.1", port), false).expect("connect");
    std::thread::sleep(Duration::from_millis(150));
    assert!(conn
        .poll(Some(Duration::from_millis(150)))
        .unwrap()
        .is_none());
    // ...and control still works alongside the silence.
    assert_eq!(conn.read_state().unwrap().vfo_a_hz, 14_074_000);
}

#[test]
fn two_clients_see_one_radio() {
    // One dial, two consoles. A change made by one is visible to the
    // other, which is the property that makes a shared server worth
    // having at all.
    let host = Dummy::new();
    let port = serve(Arc::clone(&host));
    let mut a = Connection::connect(("127.0.0.1", port), false).expect("connect a");
    let mut b = Connection::connect(("127.0.0.1", port), false).expect("connect b");

    a.command(Command::Retune { hz: 21_074_000 }).unwrap();
    assert_eq!(b.read_state().unwrap().vfo_a_hz, 21_074_000);
}

#[test]
fn the_threaded_client_gets_state_as_an_event() {
    // A frame loop asks and carries on drawing; the answer arrives later.
    let host = Dummy::new();
    let port = serve(Arc::clone(&host));
    let client = cat_native::Client::connect(("127.0.0.1", port), false).expect("connect");
    assert!(client.request_state());

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(cat_native::Event::Reply(ServerMessage::State(state))) = client.try_event() {
            assert_eq!(state.vfo_a_hz, 14_074_000);
            return;
        }
        assert!(Instant::now() < deadline, "no state reached the frame loop");
        std::thread::sleep(Duration::from_millis(10));
    }
}
