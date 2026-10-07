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

//! ADR 0023 T6: the supervisor wired into `NativeShared` and the pump.
//!
//! The supervisor's own rules are tested in `cw_tests.rs`. These test the
//! wiring: who may opt in, that abort overtakes the queue, that a live
//! send owns the link, that rigctl `T 1` is refused, and that nothing a
//! console gives up on is left keyed or clamped.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cat_framework::capabilities::*;
use cat_native::{Command, ConsoleId, ErrorCode, RadioHost, RadioState, SenderState};

use super::*;
use crate::ptt_gate::PttGate;
use crate::RigctlRadio;

const OWNER: ConsoleId = ConsoleId::from_raw(101);
const OTHER: ConsoleId = ConsoleId::from_raw(102);

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

const CW: CwCapability = CwCapability {
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
};

static CW_RADIO: RadioCapabilities = RadioCapabilities {
    model: "CW Host Test Radio",
    endpoints: EndpointSet::new(ENDPOINTS),
    vfos: VfoCapability {
        count: 2,
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
    cw: Some(CW),
};

static PLAIN_RADIO: RadioCapabilities = RadioCapabilities {
    cw: None,
    ..CW_RADIO
};

/// A radio for the pump: records every call, in order.
struct Fake {
    has_cw: bool,
    log: Vec<String>,
    power: u16,
    wpm: u16,
    /// `ready()` answers `false` this many times after each chunk.
    busy_after_send: u32,
    busy: u32,
    /// Another console aborts while this radio is applying a command.
    abort_during_apply: Option<Arc<NativeShared>>,
}

impl Fake {
    fn new(has_cw: bool) -> Self {
        Fake {
            has_cw,
            log: Vec::new(),
            power: 100,
            wpm: 20,
            busy_after_send: 0,
            busy: 0,
            abort_during_apply: None,
        }
    }

    fn pos(&self, prefix: &str) -> Option<usize> {
        self.log.iter().position(|l| l.starts_with(prefix))
    }

    fn any(&self, prefix: &str) -> bool {
        self.pos(prefix).is_some()
    }
}

#[async_trait(?Send)]
impl NativeRadio for Fake {
    async fn state(&mut self) -> Option<RadioState> {
        self.log.push("state".into());
        Some(RadioState {
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
        })
    }

    async fn apply(&mut self, command: &Command) -> Result<(), String> {
        self.log.push(format!("apply {command:?}"));
        if let Some(shared) = self.abort_during_apply.take() {
            let _ = shared.apply_as(OTHER, &Command::AbortCw);
        }
        Ok(())
    }

    fn cw(&mut self) -> Option<&mut dyn NativeCw> {
        if self.has_cw {
            Some(self)
        } else {
            None
        }
    }
}

#[async_trait(?Send)]
impl NativeCw for Fake {
    async fn status(&mut self) -> Result<CwStatus, String> {
        self.log.push("status".into());
        Ok(CwStatus {
            cw_mode: true,
            transmitting: false,
            wpm: Some(self.wpm),
        })
    }
    async fn settings(&mut self) -> Option<CwSettings> {
        self.log.push("settings".into());
        Some(CwSettings {
            pitch_hz: Some(600),
            keyer_wpm: Some(self.wpm),
            break_in: Some(BreakInKind::Off),
        })
    }
    async fn set_keyer_wpm(&mut self, wpm: u16) -> Result<(), String> {
        self.log.push(format!("set_wpm {wpm}"));
        self.wpm = wpm;
        Ok(())
    }
    async fn power_raw(&mut self) -> Result<u16, String> {
        self.log.push("power".into());
        Ok(self.power)
    }
    async fn set_power_raw(&mut self, raw: u16) -> Result<(), String> {
        self.log.push(format!("set_power {raw}"));
        self.power = raw;
        Ok(())
    }
    async fn break_in(&mut self) -> Result<BreakInKind, String> {
        self.log.push("break_in".into());
        Ok(BreakInKind::Off)
    }
    async fn set_break_in(&mut self, kind: BreakInKind) -> Result<(), String> {
        self.log.push(format!("set_break_in {kind:?}"));
        Ok(())
    }
    async fn ready(&mut self) -> Result<bool, String> {
        self.log.push("ready".into());
        if self.busy > 0 {
            self.busy -= 1;
            Ok(false)
        } else {
            Ok(true)
        }
    }
    async fn send_chunk(&mut self, text: &str) -> Result<(), String> {
        self.log.push(format!("send {text}"));
        self.busy = self.busy_after_send;
        Ok(())
    }
    async fn stop(&mut self) -> Result<(), String> {
        self.log.push("stop".into());
        Ok(())
    }
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    cat_server::block_on::block_on(f)
}

struct Rig {
    shared: Arc<NativeShared>,
    radio: Fake,
    pump: PumpState,
}

impl Rig {
    fn new(caps: &'static RadioCapabilities, has_cw: bool) -> Self {
        Rig {
            shared: NativeShared::new(caps),
            radio: Fake::new(has_cw),
            pump: PumpState::default(),
        }
    }

    fn cycle(&mut self) -> Duration {
        block_on(pump_once(
            &self.shared,
            &mut self.radio,
            &mut self.pump,
            Duration::from_millis(200),
        ))
    }

    /// Run `f` on a listener-like thread while the pump turns, as `serve`
    /// and the pump do for real.
    fn on_listener<T: Send + 'static>(
        &mut self,
        f: impl FnOnce(Arc<NativeShared>) -> T + Send + 'static,
    ) -> T {
        let shared = Arc::clone(&self.shared);
        let handle = std::thread::spawn(move || f(shared));
        for _ in 0..4_000 {
            if handle.is_finished() {
                return handle.join().expect("listener thread");
            }
            self.cycle();
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("the listener never got its answer");
    }

    fn arm(&mut self) -> Result<(), HostRefusal> {
        self.cycle();
        self.on_listener(|s| {
            s.apply_as(
                OWNER,
                &Command::ArmCwTransmit {
                    callsign: "N0CALL".into(),
                    dummy_load_acknowledged: true,
                    key_down_cap_s: None,
                },
            )
        })
    }

    fn send(&mut self, text: &str) -> Result<(), HostRefusal> {
        self.shared.apply_as(
            OWNER,
            &Command::SendCwText {
                text: text.to_string(),
            },
        )
    }

    /// `n` cycles a CW tick apart, as the real pump runs while CW is
    /// live -- the supervisor's waits are on the real clock here.
    fn cycles(&mut self, n: usize) {
        for _ in 0..n {
            self.cycle();
            std::thread::sleep(CW_TICK);
        }
    }
}

// ---------------------------------------------------------------------
// The opt-in
// ---------------------------------------------------------------------

#[test]
fn cw_is_offered_only_once_the_pump_has_found_a_cw_radio() {
    let mut rig = Rig::new(&CW_RADIO, true);
    assert!(!rig.shared.serves_cw(), "not before the pump has looked");
    rig.cycle();
    assert!(rig.shared.serves_cw());

    // A model that describes CW, behind a radio crate that did not
    // implement it: nothing to supervise with, so nothing offered.
    let mut rig = Rig::new(&CW_RADIO, false);
    rig.cycle();
    assert!(!rig.shared.serves_cw());

    // And the other way round.
    let mut rig = Rig::new(&PLAIN_RADIO, true);
    rig.cycle();
    assert!(!rig.shared.serves_cw());
}

#[test]
fn a_description_that_does_not_validate_is_never_served() {
    static UNGATED: RadioCapabilities = RadioCapabilities {
        cw: Some(CwCapability {
            tx_gate: None,
            ..CW
        }),
        ..CW_RADIO
    };
    let mut rig = Rig::new(&UNGATED, true);
    rig.cycle();
    assert!(!rig.shared.serves_cw());
}

#[test]
fn without_cw_an_abort_is_a_harmless_no_op() {
    let mut rig = Rig::new(&PLAIN_RADIO, false);
    rig.cycle();
    assert_eq!(rig.shared.apply_as(OWNER, &Command::AbortCw), Ok(()));
}

// ---------------------------------------------------------------------
// Arming and state
// ---------------------------------------------------------------------

#[test]
fn arming_through_the_host_clamps_power_and_shows_who_holds_it() {
    let mut rig = Rig::new(&CW_RADIO, true);
    assert_eq!(rig.arm(), Ok(()));
    assert_eq!(rig.radio.power, 5);
    let set = rig.radio.pos("set_power 5").unwrap();
    assert_eq!(rig.radio.log[set + 1], "power", "read back after the set");

    let mine = rig.shared.state_as(OWNER).cw.expect("cw state");
    assert!(mine.armed.as_ref().unwrap().mine);
    assert_eq!(mine.armed.unwrap().callsign, "N0CALL");
    let theirs = rig.shared.state_as(OTHER).cw.unwrap();
    assert!(!theirs.armed.unwrap().mine);
}

#[test]
fn cw_settings_ride_the_levels_clock() {
    let mut rig = Rig::new(&CW_RADIO, true);
    rig.cycle();
    assert!(rig.radio.any("settings"));
    let cw = rig.shared.state_as(OWNER).cw.expect("cw state");
    assert_eq!(cw.pitch_hz, Some(600));
    assert_eq!(cw.keyer_wpm, Some(20));

    // Not again until the levels clock comes round.
    rig.radio.log.clear();
    rig.cycle();
    assert!(!rig.radio.any("settings"));

    let mut plain = Rig::new(&PLAIN_RADIO, true);
    plain.cycle();
    assert!(!plain.radio.any("settings"));
    assert!(plain.shared.state_as(OWNER).cw.is_none());
}

// ---------------------------------------------------------------------
// Sending, abort, and the link
// ---------------------------------------------------------------------

#[test]
fn a_live_send_owns_the_link_and_the_fast_tick() {
    let mut rig = Rig::new(&CW_RADIO, true);
    rig.arm().unwrap();
    rig.radio.busy_after_send = 1_000;
    rig.send("CQ CQ CQ").unwrap();
    rig.radio.log.clear();
    let wait = rig.cycle();
    assert!(rig.radio.any("send "));
    assert!(!rig.radio.any("state"), "no state poll while keying");
    assert_eq!(wait, CW_TICK);
    rig.radio.log.clear();
    let wait = rig.cycle();
    assert!(!rig.radio.any("state"));
    assert_eq!(wait, CW_TICK);
}

#[test]
fn abort_goes_out_before_anything_queued_and_nothing_follows_it() {
    let mut rig = Rig::new(&CW_RADIO, true);
    rig.arm().unwrap();
    rig.radio.busy_after_send = 1_000;
    rig.send("CQ CQ CQ DE N0CALL").unwrap();
    rig.cycle();
    let sent = rig
        .radio
        .log
        .iter()
        .filter(|l| l.starts_with("send "))
        .count();
    assert!(sent >= 1);

    // A console's set is waiting in the queue when another console
    // aborts. The abort is not queued behind it.
    let _reply = rig
        .shared
        .enqueue(&Command::SetFrequency {
            vfo: 0,
            hz: 7_031_000,
        })
        .unwrap();
    assert_eq!(rig.shared.apply_as(OTHER, &Command::AbortCw), Ok(()));
    rig.radio.log.clear();
    rig.cycles(20);
    let stop = rig.radio.pos("stop").expect("stopped");
    let set = rig.radio.pos("apply SetFrequency").expect("set applied");
    assert!(stop < set, "{:?}", rig.radio.log);
    assert!(!rig.radio.any("send "), "{:?}", rig.radio.log);
    // An abort does not disarm.
    assert!(rig.shared.state_as(OWNER).cw.unwrap().armed.is_some());
}

#[test]
fn only_the_owner_may_send() {
    let mut rig = Rig::new(&CW_RADIO, true);
    rig.arm().unwrap();
    let r = rig
        .shared
        .apply_as(OTHER, &Command::SendCwText { text: "CQ".into() });
    assert_eq!(r.unwrap_err().code, ErrorCode::Forbidden);
}

#[test]
fn keyer_speed_goes_to_the_radio_but_not_mid_send() {
    let mut rig = Rig::new(&CW_RADIO, true);
    rig.cycle();
    let r = rig.on_listener(|s| s.apply_as(OTHER, &Command::SetKeyerSpeed { wpm: 25 }));
    assert_eq!(r, Ok(()));
    assert!(rig.radio.any("set_wpm 25"));

    rig.arm().unwrap();
    rig.radio.busy_after_send = 1_000;
    rig.send("CQ CQ").unwrap();
    rig.cycle();
    let r = rig
        .shared
        .apply_as(OWNER, &Command::SetKeyerSpeed { wpm: 30 });
    assert_eq!(r.unwrap_err().code, ErrorCode::Forbidden);
}

// ---------------------------------------------------------------------
// Every way out restores
// ---------------------------------------------------------------------

#[test]
fn the_owner_hanging_up_restores_power() {
    let mut rig = Rig::new(&CW_RADIO, true);
    rig.arm().unwrap();
    rig.shared.console_detached_as(OWNER);
    rig.cycles(5);
    assert_eq!(rig.radio.power, 100);
    assert!(rig.shared.state_as(OWNER).cw.unwrap().armed.is_none());
}

#[test]
fn disarm_through_the_host_restores_and_answers_after_the_read_back() {
    let mut rig = Rig::new(&CW_RADIO, true);
    rig.arm().unwrap();
    let r = rig.on_listener(|s| s.apply_as(OWNER, &Command::DisarmCw));
    assert_eq!(r, Ok(()));
    assert_eq!(rig.radio.power, 100);
    let set = rig
        .radio
        .log
        .iter()
        .rposition(|l| l == "set_power 100")
        .unwrap();
    assert_eq!(rig.radio.log[set + 1], "power");
}

#[test]
fn an_arm_its_console_gave_up_on_is_dropped_not_left_clamped() {
    let mut rig = Rig::new(&CW_RADIO, true);
    rig.cycle();
    rig.shared.set_cw_reply_timeout(Duration::ZERO);
    // The console times out before the pump has even started arming.
    let r = rig.shared.apply_as(
        OWNER,
        &Command::ArmCwTransmit {
            callsign: "N0CALL".into(),
            dummy_load_acknowledged: true,
            key_down_cap_s: None,
        },
    );
    assert!(r.is_err());
    rig.cycles(10);
    assert_eq!(rig.radio.power, 100, "clamped, then restored");
    assert!(rig.radio.any("set_power 5"));
    assert!(rig.shared.state_as(OWNER).cw.unwrap().armed.is_none());
    assert!(rig.shared.cw_settled());
}

#[test]
fn shutdown_restores_and_then_reports_settled() {
    let mut rig = Rig::new(&CW_RADIO, true);
    rig.arm().unwrap();
    rig.radio.busy_after_send = 3;
    rig.send("CQ").unwrap();
    rig.cycle();
    assert!(!rig.shared.cw_settled());
    rig.shared.shutdown_cw();
    rig.cycles(20);
    assert!(rig.radio.any("stop"));
    assert_eq!(rig.radio.power, 100);
    assert!(rig.shared.cw_settled());
    assert!(matches!(
        rig.shared.state_as(OWNER).cw.unwrap().sender,
        SenderState::Stopped { .. }
    ));
}

// ---------------------------------------------------------------------
// rigctl
// ---------------------------------------------------------------------

#[derive(Default)]
struct Hamlib {
    keyed: u32,
}

#[async_trait(?Send)]
impl RigctlRadio for Hamlib {
    type Mode = ModeId;
    type Error = std::io::Error;
    fn unsupported() -> Self::Error {
        std::io::Error::other("unsupported")
    }
    async fn get_vfo_a_hz(&mut self) -> Result<u64, Self::Error> {
        Ok(7_030_000)
    }
    async fn set_vfo_a_hz(&mut self, _hz: u64) -> Result<(), Self::Error> {
        Ok(())
    }
    async fn get_mode(&mut self) -> Result<Self::Mode, Self::Error> {
        Ok(ModeId::CwUpper)
    }
    async fn set_mode(&mut self, _mode: Self::Mode) -> Result<(), Self::Error> {
        Ok(())
    }
    async fn get_transmitting(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    async fn transmit(&mut self) -> Result<(), Self::Error> {
        self.keyed += 1;
        Ok(())
    }
    async fn receive(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn hamlib_mode_name(_mode: Self::Mode) -> &'static str {
        "CW"
    }
    fn hamlib_mode_from_name(_name: &str) -> Option<Self::Mode> {
        Some(ModeId::CwUpper)
    }
    fn freq_range_hz() -> (u64, u64) {
        (1_800_000, 30_000_000)
    }
}

#[test]
fn rigctl_cannot_key_the_radio_while_a_console_holds_the_cw_arm() {
    let mut rig = Rig::new(&CW_RADIO, true);
    let mut gate = PttGate::new(Hamlib::default(), Arc::clone(&rig.shared));
    rig.cycle();
    assert!(
        block_on(gate.transmit()).is_ok(),
        "unarmed: WSJT-X as usual"
    );
    assert_eq!(gate.inner().keyed, 1);

    rig.arm().unwrap();
    assert!(block_on(gate.transmit()).is_err());
    assert_eq!(gate.inner().keyed, 1, "the radio never heard it");
    // Releasing is always allowed.
    assert!(block_on(gate.receive()).is_ok());

    rig.on_listener(|s| s.apply_as(OWNER, &Command::DisarmCw))
        .unwrap();
    assert!(block_on(gate.transmit()).is_ok());
    assert_eq!(gate.inner().keyed, 2);
}

#[test]
fn the_gate_passes_everything_else_through() {
    let rig = Rig::new(&CW_RADIO, true);
    let mut gate = PttGate::new(Hamlib::default(), Arc::clone(&rig.shared));
    assert_eq!(block_on(gate.get_vfo_a_hz()).unwrap(), 7_030_000);
    assert_eq!(block_on(gate.get_mode()).unwrap(), ModeId::CwUpper);
    assert_eq!(PttGate::<Hamlib>::freq_range_hz(), (1_800_000, 30_000_000));
}

#[test]
fn an_abort_arriving_mid_queue_waits_for_one_command_at_most() {
    let mut rig = Rig::new(&CW_RADIO, true);
    rig.arm().unwrap();
    rig.radio.busy_after_send = 1_000;
    rig.send("CQ CQ CQ").unwrap();
    rig.cycle();
    let _a = rig
        .shared
        .enqueue(&Command::SetSplit { enabled: true })
        .unwrap();
    let _b = rig
        .shared
        .enqueue(&Command::SetFrequency {
            vfo: 0,
            hz: 7_031_000,
        })
        .unwrap();
    rig.radio.abort_during_apply = Some(Arc::clone(&rig.shared));
    rig.radio.log.clear();
    rig.cycle();
    let first = rig.radio.pos("apply SetSplit").unwrap();
    let stop = rig.radio.pos("stop").expect("stopped");
    let second = rig.radio.pos("apply SetFrequency").unwrap();
    assert!(first < stop && stop < second, "{:?}", rig.radio.log);
}
