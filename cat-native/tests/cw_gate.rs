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

//! ADR 0023's gate, end to end through `serve`: a host that has not said
//! it serves CW is never handed a CW command, whatever the radio can do.

use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use cat_framework::capabilities::*;
use cat_native::{
    serve, Command, Connection, ErrorCode, ModeId, RadioHost, RadioState, ServerMessage, Streams,
};

const MODES: &[ModeDescriptor] = &[ModeDescriptor {
    id: ModeId::CwUpper,
    label: "CW",
    kind: ModeKind::Cw,
    sideband: Some(Sideband::Upper),
    default_bandwidth_hz: 500,
}];
const ENDPOINTS: &[EndpointDescriptor] = &[EndpointDescriptor {
    role: EndpointRole::Cat,
    required: true,
    shareable_with: &[],
}];

static RADIO: RadioCapabilities = RadioCapabilities {
    model: "CW Gate Test Radio",
    endpoints: EndpointSet::new(ENDPOINTS),
    vfos: VfoCapability {
        count: 1,
        split: false,
        rit_hz: None,
        xit_hz: None,
    },
    modes: MODES,
    tuning_steps_hz: &[10],
    rx_range: FrequencyRange::new(1_800_000, 30_000_000),
    filters: FilterCapability {
        if_shift_hz: None,
        widths_hz: None,
        notch: false,
    },
    meters: MeterSet::new(&[]),
    memory: None,
    menu: None,
    signal: SignalSupport::None,
    cw: Some(CwCapability {
        pitch: Some(HzRange::stepped(400, 1_000, 50)),
        pitch_writable: true,
        keyer_wpm: Some(WpmRange::new(10, 60)),
        keyer_wpm_writable: true,
        break_in: &[BreakInKind::Off, BreakInKind::Semi],
        send_text: Some(CwSendText {
            max_chunk_chars: 24,
            charset: "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789/?",
            pacing: CwPacing::BufferFlag,
            abort: CwAbort::DrainOnly,
            requires_break_in: None,
        }),
        tx_gate: Some(TxGateCapability {
            low_power_raw: 5,
            low_power_label: "5 W",
            power_raw: RawRange::new(5, 100),
        }),
    }),
};

struct Host {
    serves_cw: bool,
    applied: Mutex<Vec<Command>>,
}

impl RadioHost for Host {
    fn capabilities(&self) -> &'static RadioCapabilities {
        &RADIO
    }

    fn state(&self) -> RadioState {
        RadioState {
            vfo_a_hz: 7_030_000,
            vfo_b_hz: 7_030_000,
            mode: ModeId::CwUpper,
            split: false,
            transmitting: false,
            memory_channel: None,
            if_shift_hz: None,
            filter_width_hz: None,
            meters: Vec::new(),
            levels: None,
            cw: None,
        }
    }

    fn serves_cw(&self) -> bool {
        self.serves_cw
    }

    fn apply(&self, command: &Command) -> Result<(), String> {
        self.applied.lock().unwrap().push(command.clone());
        Ok(())
    }
}

fn start(serves_cw: bool) -> (Arc<Host>, Connection) {
    let host = Arc::new(Host {
        serves_cw,
        applied: Mutex::new(Vec::new()),
    });
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = Arc::clone(&host);
    std::thread::spawn(move || serve(listener, served));
    let conn = Connection::connect(("127.0.0.1", port), Streams::none()).expect("connect");
    (host, conn)
}

fn arm() -> Command {
    Command::ArmCwTransmit {
        callsign: "N0CALL".to_string(),
        dummy_load_acknowledged: true,
        key_down_cap_s: None,
    }
}

#[test]
fn a_host_that_does_not_serve_cw_is_never_handed_a_cw_command() {
    let (host, mut conn) = start(false);
    assert_eq!(conn.capabilities().cw, None, "nothing offered");

    for command in [
        Command::SetKeyerSpeed { wpm: 20 },
        arm(),
        Command::SendCwText {
            text: "CQ".to_string(),
        },
        Command::DisarmCw,
    ] {
        match conn.command(command.clone()).unwrap() {
            ServerMessage::Error {
                code: ErrorCode::Unsupported,
                ..
            } => {}
            other => panic!("{command:?}: {other:?}"),
        }
    }
    // Abort is acknowledged -- it is never refused -- but goes nowhere.
    assert_eq!(conn.command(Command::AbortCw).unwrap(), ServerMessage::Ack);
    assert!(host.applied.lock().unwrap().is_empty());
}

#[test]
fn a_host_that_serves_cw_receives_what_validates_and_nothing_else() {
    let (host, mut conn) = start(true);
    assert!(conn.capabilities().cw.is_some());

    assert_eq!(conn.command(Command::AbortCw).unwrap(), ServerMessage::Ack);
    assert_eq!(conn.command(arm()).unwrap(), ServerMessage::Ack);
    match conn
        .command(Command::SendCwText {
            text: "HI!".to_string(),
        })
        .unwrap()
    {
        ServerMessage::Error {
            code: ErrorCode::OutOfRange,
            ..
        } => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(*host.applied.lock().unwrap(), vec![Command::AbortCw, arm()]);
}
