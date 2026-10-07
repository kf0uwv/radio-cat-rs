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

//! ADR 0023 T7: the CW calls a trainer makes, over a real socket.

use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use cat_framework::capabilities::*;
use cat_native::{
    serve, ClientError, Command, Connection, ConsoleId, ErrorCode, HostRefusal, ModeId, RadioHost,
    RadioState, Streams,
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
    model: "CW Client Test Radio",
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
    seen: Mutex<Vec<Command>>,
    refuse: Mutex<Option<HostRefusal>>,
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
    fn apply(&self, _: &Command) -> Result<(), String> {
        unreachable!("apply_as is overridden")
    }
    fn apply_as(&self, _id: ConsoleId, command: &Command) -> Result<(), HostRefusal> {
        self.seen.lock().unwrap().push(command.clone());
        match self.refuse.lock().unwrap().clone() {
            Some(r) => Err(r),
            None => Ok(()),
        }
    }
}

fn start(serves_cw: bool) -> (Arc<Host>, Connection) {
    let host = Arc::new(Host {
        serves_cw,
        seen: Mutex::new(Vec::new()),
        refuse: Mutex::new(None),
    });
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = Arc::clone(&host);
    std::thread::spawn(move || serve(listener, served));
    let conn = Connection::connect(("127.0.0.1", port), Streams::none()).unwrap();
    (host, conn)
}

fn refused(r: Result<(), ClientError>) -> ErrorCode {
    match r {
        Err(ClientError::Refused { code, .. }) => code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_trainer_arms_sends_aborts_and_disarms() {
    let (host, mut conn) = start(true);
    let cw = conn.cw_capability().expect("served").clone();
    assert_eq!(cw.send_text.unwrap().max_chunk_chars, 24);

    conn.set_keyer_speed(20).unwrap();
    conn.arm_cw("N0CALL", true, Some(30)).unwrap();
    conn.send_cw("CQ CQ").unwrap();
    conn.abort_cw().unwrap();
    conn.disarm_cw().unwrap();
    assert_eq!(
        *host.seen.lock().unwrap(),
        vec![
            Command::SetKeyerSpeed { wpm: 20 },
            Command::ArmCwTransmit {
                callsign: "N0CALL".into(),
                dummy_load_acknowledged: true,
                key_down_cap_s: Some(30),
            },
            Command::SendCwText {
                text: "CQ CQ".into()
            },
            Command::AbortCw,
            Command::DisarmCw,
        ]
    );
}

#[test]
fn a_refusal_arrives_with_its_code_and_reason() {
    let (host, mut conn) = start(true);
    *host.refuse.lock().unwrap() = Some(HostRefusal {
        code: ErrorCode::Forbidden,
        message: "another console holds the CW transmit arm".into(),
    });
    match conn.arm_cw("N0CALL", true, None) {
        Err(ClientError::Refused { code, message }) => {
            assert_eq!(code, ErrorCode::Forbidden);
            assert!(message.contains("another console"));
        }
        other => panic!("{other:?}"),
    }
    // The session's own rules come back the same way.
    *host.refuse.lock().unwrap() = None;
    assert_eq!(
        refused(conn.arm_cw("N0CALL", false, None)),
        ErrorCode::Forbidden,
        "no dummy-load acknowledgement"
    );
    assert_eq!(refused(conn.set_keyer_speed(5)), ErrorCode::OutOfRange);
}

#[test]
fn against_a_server_without_cw_nothing_is_sent() {
    // A radio that describes CW behind a host that does not serve it:
    // the Welcome says `cw: None`, and that is the whole answer.
    let (host, mut conn) = start(false);
    assert!(conn.cw_capability().is_none());
    assert_eq!(refused(conn.set_keyer_speed(20)), ErrorCode::Unsupported);
    assert_eq!(
        refused(conn.arm_cw("N0CALL", true, None)),
        ErrorCode::Unsupported
    );
    assert_eq!(refused(conn.send_cw("CQ")), ErrorCode::Unsupported);
    assert_eq!(refused(conn.disarm_cw()), ErrorCode::Unsupported);
    // Nothing to stop, and nothing asked.
    assert!(conn.abort_cw().is_ok());
    assert!(conn.cw_state().unwrap().is_none());
    assert!(host.seen.lock().unwrap().is_empty(), "never probe");
}
