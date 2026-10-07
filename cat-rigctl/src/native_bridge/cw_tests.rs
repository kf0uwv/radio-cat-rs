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

//! Tests for [`super::CwSupervisor`]. Every one runs on an injected clock
//! against a scripted radio -- no threads, no sockets, no sleeps.

use std::time::{Duration, Instant};

use cat_framework::capabilities::{
    BreakInKind, CwAbort, CwCapability, CwPacing, CwSendText, HzRange, RawRange, TxGateCapability,
    WpmRange,
};
use cat_native::{ConsoleId, ErrorCode, SenderState, StopReason};

use super::*;

const OWNER: ConsoleId = ConsoleId::from_raw(1);
const OTHER: ConsoleId = ConsoleId::from_raw(2);

const CHARSET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789/?=+.,";

/// TS-570D-shaped: a busy flag, no stop, 24 characters, 5-100 W.
const KENWOOD: CwCapability = CwCapability {
    pitch: Some(HzRange::stepped(400, 1_000, 50)),
    pitch_writable: true,
    keyer_wpm: Some(WpmRange::new(10, 60)),
    keyer_wpm_writable: true,
    break_in: &[BreakInKind::Off, BreakInKind::Semi],
    send_text: Some(CwSendText {
        max_chunk_chars: 24,
        charset: CHARSET,
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

/// IC-7100-shaped: no busy flag, a real stop, needs break-in to send.
const ICOM: CwCapability = CwCapability {
    pitch: Some(HzRange::continuous(300, 900)),
    pitch_writable: true,
    keyer_wpm: Some(WpmRange::new(6, 48)),
    keyer_wpm_writable: true,
    break_in: &[BreakInKind::Off, BreakInKind::Semi, BreakInKind::Full],
    send_text: Some(CwSendText {
        max_chunk_chars: 30,
        charset: CHARSET,
        pacing: CwPacing::Estimated,
        abort: CwAbort::RadioStop,
        requires_break_in: Some(BreakInKind::Semi),
    }),
    tx_gate: Some(TxGateCapability {
        low_power_raw: 0,
        low_power_label: "minimum",
        power_raw: RawRange::new(0, 255),
    }),
};

/// A scripted radio. Answers every action the way the real one would,
/// with switches for each way a real one goes wrong.
struct Radio {
    cw_mode: bool,
    transmitting: bool,
    wpm: Option<u16>,
    power: u16,
    break_in: BreakInKind,
    /// `Ready(false)` this many times before each `Ready(true)`.
    busy_polls: u32,
    busy_left: u32,
    /// Never report ready: a keyer that has wedged.
    never_ready: bool,
    /// Fail this many power writes, then behave.
    fail_power_writes: u32,
    /// Accept power writes but leave the power where it was.
    ignores_power: bool,
    fail_break_in_writes: u32,
    log: Vec<CwAction>,
    /// When the radio will finish keying everything it has been handed,
    /// keyed one after another at its own speed. `None`: silent.
    keyed_until: Option<Instant>,
    /// A `RadioStop` keyer: `Stop` silences it at once.
    stops: bool,
    /// Answer `Ready` as the TS-570D's `KY` does: buffer space, not idle
    /// -- true while it is still keying what it holds.
    buffer_space_ready: bool,
    /// Anything that must never happen, recorded rather than panicked so
    /// a test can say which.
    violations: Vec<String>,
}

impl Radio {
    fn new() -> Self {
        Radio {
            cw_mode: true,
            transmitting: false,
            wpm: Some(20),
            power: 100,
            break_in: BreakInKind::Off,
            busy_polls: 0,
            busy_left: 0,
            never_ready: false,
            fail_power_writes: 0,
            ignores_power: false,
            fail_break_in_writes: 0,
            log: Vec::new(),
            keyed_until: None,
            stops: false,
            buffer_space_ready: false,
            violations: Vec::new(),
        }
    }

    fn keying_at(&self, now: Instant) -> bool {
        self.keyed_until.is_some_and(|t| now < t)
    }

    fn answer(&mut self, action: &CwAction, now: Instant) -> CwOutcome {
        self.log.push(action.clone());
        if let CwAction::SetPower(raw) = action {
            if *raw > self.power && self.keying_at(now) {
                self.violations.push(format!(
                    "power raised to {raw} with {:?} still to key",
                    self.keyed_until.unwrap() - now
                ));
            }
        }
        match action {
            CwAction::ReadStatus => CwOutcome::Status {
                cw_mode: self.cw_mode,
                transmitting: self.transmitting,
                wpm: self.wpm,
            },
            CwAction::ReadPower => CwOutcome::Power(self.power),
            CwAction::SetPower(raw) => {
                if self.fail_power_writes > 0 {
                    self.fail_power_writes -= 1;
                    return CwOutcome::Failed("PC write timed out".to_string());
                }
                if !self.ignores_power {
                    self.power = *raw;
                }
                CwOutcome::Done
            }
            CwAction::ReadBreakIn => CwOutcome::BreakIn(self.break_in),
            CwAction::SetBreakIn(kind) => {
                if self.fail_break_in_writes > 0 {
                    self.fail_break_in_writes -= 1;
                    return CwOutcome::Failed("break-in write refused".to_string());
                }
                self.break_in = *kind;
                CwOutcome::Done
            }
            CwAction::Ready => {
                if self.buffer_space_ready {
                    return CwOutcome::Ready(true);
                }
                if self.never_ready {
                    return CwOutcome::Ready(false);
                }
                if self.busy_left > 0 {
                    self.busy_left -= 1;
                    CwOutcome::Ready(false)
                } else {
                    self.busy_left = self.busy_polls;
                    CwOutcome::Ready(true)
                }
            }
            CwAction::SendChunk(text) => {
                self.busy_left = self.busy_polls;
                let start = self.keyed_until.filter(|t| *t > now).unwrap_or(now);
                self.keyed_until = Some(start + estimate(text, self.wpm.unwrap_or(10)));
                CwOutcome::Done
            }
            CwAction::Stop => {
                if self.stops {
                    self.keyed_until = None;
                }
                CwOutcome::Done
            }
        }
    }

    fn sent(&self) -> Vec<String> {
        self.log
            .iter()
            .filter_map(|a| match a {
                CwAction::SendChunk(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    fn count(&self, pred: impl Fn(&CwAction) -> bool) -> usize {
        self.log.iter().filter(|a| pred(a)).count()
    }
}

/// A supervisor, a radio and a clock.
struct Bench {
    sup: CwSupervisor,
    radio: Radio,
    t0: Instant,
    now: Instant,
    notices: Vec<CwNotice>,
}

impl Bench {
    fn new(cap: CwCapability) -> Self {
        let t0 = Instant::now();
        Bench {
            sup: CwSupervisor::new(cap, CwConfig::default()),
            radio: Radio::new(),
            t0,
            now: t0,
            notices: Vec::new(),
        }
    }

    /// Run every action that is due now.
    fn drive(&mut self) {
        for _ in 0..10_000 {
            match self.sup.next_action(self.now) {
                Some(action) => {
                    let outcome = self.radio.answer(&action, self.now);
                    self.sup.complete(outcome, self.now);
                }
                None => break,
            }
        }
        self.notices.extend(self.sup.take_notices());
    }

    /// Advance the clock in CW ticks, driving at each.
    fn run_for(&mut self, d: Duration) {
        let end = self.now + d;
        while self.now < end {
            self.now += Duration::from_millis(50);
            self.drive();
        }
    }

    fn elapsed(&self) -> Duration {
        self.now - self.t0
    }

    fn reply(&self, ticket: Ticket) -> Option<Result<(), HostRefusal>> {
        self.notices.iter().find_map(|n| match n {
            CwNotice::Reply { ticket: t, result } if *t == ticket => Some(result.clone()),
            _ => None,
        })
    }

    fn arm(&mut self) -> Result<(), HostRefusal> {
        self.arm_as(OWNER, None)
    }

    fn arm_as(&mut self, owner: ConsoleId, cap_s: Option<u16>) -> Result<(), HostRefusal> {
        let ticket = self.sup.arm(owner, "n0call", cap_s, self.now)?;
        self.drive();
        self.reply(ticket).expect("arm answered once the radio has")
    }

    fn restored(&self) -> bool {
        self.notices
            .iter()
            .any(|n| matches!(n, CwNotice::Restored { .. }))
    }

    fn restore_failures(&self) -> usize {
        self.notices
            .iter()
            .filter(|n| matches!(n, CwNotice::RestoreFailed { .. }))
            .count()
    }
}

fn code(r: Result<impl std::fmt::Debug, HostRefusal>) -> ErrorCode {
    match r {
        Err(refusal) => refusal.code,
        Ok(v) => panic!("expected a refusal, got Ok({v:?})"),
    }
}

// ---------------------------------------------------------------------
// Arming
// ---------------------------------------------------------------------

#[test]
fn arming_clamps_power_and_reads_it_back_before_answering() {
    let mut b = Bench::new(KENWOOD);
    assert_eq!(b.arm(), Ok(()));
    assert_eq!(b.radio.power, 5);
    let armed = b.sup.armed_state(OWNER).expect("armed");
    assert_eq!(armed.callsign, "N0CALL");
    assert_eq!(armed.power_raw, 5);
    assert!(armed.mine);
    assert!(!b.sup.armed_state(OTHER).unwrap().mine);
    // Set, then read back: the read after the write is what armed it.
    let set = b
        .radio
        .log
        .iter()
        .position(|a| *a == CwAction::SetPower(5))
        .unwrap();
    assert!(b.radio.log[set + 1..].contains(&CwAction::ReadPower));
}

#[test]
fn a_power_read_back_mismatch_refuses_the_arm_and_restores() {
    let mut b = Bench::new(KENWOOD);
    b.radio.ignores_power = true;
    b.radio.power = 100;
    let refusal = b.arm().unwrap_err();
    assert!(refusal.message.contains("100"), "{refusal:?}");
    b.drive();
    assert!(b.sup.armed_state(OWNER).is_none());
    // It wrote 5, so it writes back what it found, and verifies.
    assert_eq!(b.radio.count(|a| *a == CwAction::SetPower(100)), 1);
    assert!(b.restored());
    assert!(b.sup.is_settled());
}

#[test]
fn arming_is_refused_outside_cw_mode_and_nothing_is_touched() {
    let mut b = Bench::new(KENWOOD);
    b.radio.cw_mode = false;
    assert_eq!(code(b.arm()), ErrorCode::Forbidden);
    assert_eq!(b.radio.power, 100);
    assert!(b
        .radio
        .log
        .iter()
        .all(|a| !matches!(a, CwAction::SetPower(_))));
    assert!(b.sup.is_settled());
}

#[test]
fn arming_is_refused_while_the_radio_is_transmitting() {
    let mut b = Bench::new(KENWOOD);
    b.radio.transmitting = true;
    assert_eq!(code(b.arm()), ErrorCode::Forbidden);
    assert!(b.sup.is_settled());
}

#[test]
fn only_one_console_holds_the_arm() {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    assert_eq!(
        code(b.sup.arm(OTHER, "K9X", None, b.now)),
        ErrorCode::Forbidden
    );
    assert_eq!(
        code(b.sup.arm(OWNER, "K9X", None, b.now)),
        ErrorCode::Forbidden
    );
}

#[test]
fn a_radio_needing_break_in_gets_it_and_gets_it_back() {
    let mut b = Bench::new(ICOM);
    b.radio.power = 200;
    b.radio.break_in = BreakInKind::Off;
    b.arm().unwrap();
    assert_eq!(b.radio.break_in, BreakInKind::Semi);
    assert_eq!(b.radio.power, 0);

    let ticket = b.sup.disarm(OWNER, b.now).unwrap();
    b.drive();
    assert_eq!(b.reply(ticket), Some(Ok(())));
    assert_eq!(b.radio.power, 200, "power restored");
    assert_eq!(b.radio.break_in, BreakInKind::Off, "break-in restored");
    assert!(b.restored());
    assert!(b.sup.is_settled());
}

#[test]
fn a_radio_that_cannot_send_cannot_be_armed() {
    let read_only = CwCapability {
        send_text: None,
        tx_gate: None,
        ..KENWOOD
    };
    let mut b = Bench::new(read_only);
    assert_eq!(
        code(b.sup.arm(OWNER, "N0CALL", None, b.now)),
        ErrorCode::Unsupported
    );
}

// ---------------------------------------------------------------------
// Restore, on every way out
// ---------------------------------------------------------------------

#[test]
fn disarm_restores_power_and_reads_it_back() {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    let ticket = b.sup.disarm(OWNER, b.now).unwrap();
    b.drive();
    assert_eq!(b.reply(ticket), Some(Ok(())));
    assert_eq!(b.radio.power, 100);
    let last_set = b
        .radio
        .log
        .iter()
        .rposition(|a| *a == CwAction::SetPower(100))
        .unwrap();
    assert_eq!(b.radio.log[last_set + 1], CwAction::ReadPower);
    assert!(b.sup.armed_state(OWNER).is_none());
    assert!(b.sup.is_settled());
}

#[test]
fn only_the_owner_may_disarm() {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    assert_eq!(code(b.sup.disarm(OTHER, b.now)), ErrorCode::Forbidden);
    assert!(b.sup.armed_state(OWNER).is_some());
}

#[test]
fn the_owner_hanging_up_restores_and_a_bystander_hanging_up_does_not() {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    b.sup.detached(OTHER, b.now);
    b.drive();
    assert!(b.sup.armed_state(OWNER).is_some());
    assert_eq!(b.radio.power, 5);

    b.sup.detached(OWNER, b.now);
    b.drive();
    assert!(b.sup.armed_state(OWNER).is_none());
    assert_eq!(b.radio.power, 100);
    assert!(b.restored());
}

#[test]
fn the_owner_hanging_up_mid_send_stops_then_restores() {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    b.radio.busy_polls = 3;
    b.sup
        .send(OWNER, "CQ CQ CQ DE N0CALL N0CALL K", b.now)
        .unwrap();
    b.drive();
    let sent_before = b.radio.sent().len();
    assert!(sent_before >= 1);

    b.sup.detached(OWNER, b.now);
    b.drive();
    b.run_for(Duration::from_secs(5));
    let stop = b
        .radio
        .log
        .iter()
        .rposition(|a| *a == CwAction::Stop)
        .unwrap();
    let restore = b
        .radio
        .log
        .iter()
        .rposition(|a| *a == CwAction::SetPower(100))
        .unwrap();
    assert!(stop < restore, "never raise power under a keyed chunk");
    assert_eq!(b.radio.sent().len(), sent_before, "nothing sent after");
    assert!(matches!(
        b.sup.sender(),
        SenderState::Stopped {
            reason: StopReason::Disconnected,
            ..
        }
    ));
    assert!(b.sup.is_settled());
}

#[test]
fn shutdown_stops_and_restores() {
    let mut b = Bench::new(ICOM);
    b.radio.power = 128;
    b.arm().unwrap();
    b.sup.send(OWNER, "TEST TEST", b.now).unwrap();
    b.drive();
    b.sup.shutdown(b.now);
    b.drive();
    b.run_for(Duration::from_secs(2));
    assert!(b.radio.log.contains(&CwAction::Stop));
    assert_eq!(b.radio.power, 128);
    assert_eq!(b.radio.break_in, BreakInKind::Off);
    assert!(b.sup.is_settled());
}

#[test]
fn an_idle_arm_expires_and_restores() {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    b.run_for(CwConfig::default().idle_ttl - Duration::from_secs(1));
    assert!(b.sup.armed_state(OWNER).is_some());
    b.run_for(Duration::from_secs(2));
    assert!(b.sup.armed_state(OWNER).is_none());
    assert_eq!(b.radio.power, 100);
}

/// Each way an arm ends, with the restore's first power write failing.
#[derive(Debug, Clone, Copy)]
enum Ending {
    Disarm,
    Detach,
    Watchdog,
    Shutdown,
}

fn end_with_a_failing_restore(ending: Ending) {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    let mut ticket = None;
    match ending {
        Ending::Disarm => ticket = Some(b.sup.disarm(OWNER, b.now).unwrap()),
        Ending::Detach => b.sup.detached(OWNER, b.now),
        Ending::Shutdown => b.sup.shutdown(b.now),
        Ending::Watchdog => {
            b.radio.never_ready = true;
            b.sup.send(OWNER, "CQ", b.now).unwrap();
            b.drive();
        }
    }
    b.radio.fail_power_writes = 3;
    b.drive();
    if let Ending::Watchdog = ending {
        // Past the job's deadline (estimate x 1.2 + margin, ~12 s here).
        b.run_for(Duration::from_secs(20));
        b.radio.never_ready = false;
        b.drive();
    }

    // Failed, said so, and has not claimed otherwise.
    assert!(b.restore_failures() >= 1, "{ending:?}: failure reported");
    assert!(!b.restored(), "{ending:?}: never claims restored");
    assert!(!b.sup.is_settled(), "{ending:?}");
    assert!(b.sup.blocks_external_ptt(), "{ending:?}: still locked");
    let status = b.sup.restore_status().expect("alert visible");
    assert!(status.attempts >= 1);
    assert!(status.last_error.contains("timed out"), "{status:?}");
    assert_eq!(b.radio.power, 5, "{ending:?}: still low, the safe way");
    if let Some(t) = ticket {
        let reply = b.reply(t).expect("the disarm is answered");
        assert!(reply.is_err(), "a failed restore is not Ok: {reply:?}");
    }
    // Nobody can arm over an unrestored radio.
    assert_eq!(
        code(b.sup.arm(OTHER, "K9X", None, b.now)),
        ErrorCode::Forbidden
    );

    // It keeps trying, and only a verified read-back ends the alert.
    b.run_for(CwConfig::default().restore_retry * 4);
    assert_eq!(b.radio.power, 100, "{ending:?}");
    assert!(b.restored(), "{ending:?}");
    assert!(b.sup.restore_status().is_none());
    assert!(b.sup.is_settled(), "{ending:?}");
    assert!(!b.sup.blocks_external_ptt());
    // Every retry wrote, and every write was read back.
    assert!(b.radio.count(|a| *a == CwAction::SetPower(100)) >= 4);
}

#[test]
fn a_failing_restore_after_disarm_is_reported_and_retried() {
    end_with_a_failing_restore(Ending::Disarm);
}

#[test]
fn a_failing_restore_after_disconnect_is_reported_and_retried() {
    end_with_a_failing_restore(Ending::Detach);
}

#[test]
fn a_failing_restore_after_the_watchdog_is_reported_and_retried() {
    end_with_a_failing_restore(Ending::Watchdog);
}

#[test]
fn a_failing_restore_at_shutdown_is_reported_and_retried() {
    end_with_a_failing_restore(Ending::Shutdown);
}

#[test]
fn a_restore_that_reads_back_wrong_is_a_failure_not_a_success() {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    b.radio.ignores_power = true;
    b.sup.disarm(OWNER, b.now).unwrap();
    b.drive();
    assert!(!b.restored());
    assert!(b.restore_failures() >= 1);
    assert!(b.sup.restore_status().unwrap().last_error.contains('5'));
}

#[test]
fn a_failing_break_in_restore_is_retried_too() {
    let mut b = Bench::new(ICOM);
    b.arm().unwrap();
    b.radio.fail_break_in_writes = 1;
    b.sup.disarm(OWNER, b.now).unwrap();
    b.drive();
    assert!(!b.restored());
    b.run_for(CwConfig::default().restore_retry * 2);
    assert!(b.restored());
    assert_eq!(b.radio.break_in, BreakInKind::Off);
}

// ---------------------------------------------------------------------
// Sending
// ---------------------------------------------------------------------

#[test]
fn only_the_owner_may_send() {
    let mut b = Bench::new(KENWOOD);
    assert_eq!(code(b.sup.send(OWNER, "CQ", b.now)), ErrorCode::Forbidden);
    b.arm().unwrap();
    assert_eq!(code(b.sup.send(OTHER, "CQ", b.now)), ErrorCode::Forbidden);
    assert!(b.sup.send(OWNER, "CQ", b.now).is_ok());
}

#[test]
fn one_send_at_a_time() {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    b.radio.never_ready = true;
    b.sup.send(OWNER, "CQ", b.now).unwrap();
    b.drive();
    assert_eq!(code(b.sup.send(OWNER, "CQ", b.now)), ErrorCode::Forbidden);
}

#[test]
fn the_first_send_carries_the_id_and_later_ones_every_ten_minutes() {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    b.sup.send(OWNER, "cq test", b.now).unwrap();
    b.run_for(Duration::from_secs(20));
    let first = b.radio.sent().concat().replace(' ', "");
    assert!(first.ends_with("DEN0CALL"), "{first:?}");
    assert!(first.starts_with("CQTEST"), "{first:?}");

    let before = b.radio.sent().len();
    b.sup.send(OWNER, "TEST", b.now).unwrap();
    b.run_for(Duration::from_secs(20));
    let second = b.radio.sent()[before..].join(" ");
    assert_eq!(second, "TEST", "no ID inside ten minutes");

    // Keep the arm alive past ten minutes from the first ID.
    for _ in 0..4 {
        b.run_for(Duration::from_secs(150));
        let n = b.radio.sent().len();
        b.sup.send(OWNER, "E", b.now).unwrap();
        b.run_for(Duration::from_secs(5));
        if b.radio.sent()[n..]
            .concat()
            .replace(' ', "")
            .contains("DEN0CALL")
        {
            assert!(b.elapsed() >= Duration::from_secs(600));
            return;
        }
    }
    panic!("no ID after ten minutes");
}

#[test]
fn an_aborted_id_is_owed_again() {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    b.radio.never_ready = true;
    b.sup.send(OWNER, "CQ", b.now).unwrap();
    b.drive();
    b.sup.abort(b.now);
    b.run_for(Duration::from_secs(5));
    b.radio.never_ready = false;
    let n = b.radio.sent().len();
    b.sup.send(OWNER, "CQ", b.now).unwrap();
    b.run_for(Duration::from_secs(10));
    assert!(b.radio.sent()[n..]
        .concat()
        .replace(' ', "")
        .contains("DEN0CALL"));
}

#[test]
fn a_send_over_the_cap_is_refused_and_the_id_counts() {
    let mut b = Bench::new(KENWOOD);
    b.radio.wpm = Some(10);
    b.arm().unwrap();
    // ~60 s at 10 wpm is about 50 characters.
    let long = "PARIS ".repeat(12);
    assert_eq!(code(b.sup.send(OWNER, &long, b.now)), ErrorCode::OutOfRange);
    assert!(b.radio.sent().is_empty());
    // Under the cap without the ID, over it with: refused.
    let text = "PARIS ".repeat(9);
    let bare = estimate(&text, 10);
    let with_id = estimate(&format!("{text} DE N0CALL"), 10);
    assert!(bare < Duration::from_secs(60) && with_id > Duration::from_secs(60));
    assert_eq!(code(b.sup.send(OWNER, &text, b.now)), ErrorCode::OutOfRange);
}

#[test]
fn a_client_may_lower_the_cap_but_not_raise_it() {
    let mut b = Bench::new(KENWOOD);
    b.radio.wpm = Some(20);
    b.arm_as(OWNER, Some(5)).unwrap();
    assert_eq!(
        code(b.sup.send(OWNER, "CQ CQ CQ CQ", b.now)),
        ErrorCode::OutOfRange
    );

    let mut b = Bench::new(KENWOOD);
    b.radio.wpm = Some(10);
    b.arm_as(OWNER, Some(600)).unwrap();
    let long = "PARIS ".repeat(12);
    assert_eq!(code(b.sup.send(OWNER, &long, b.now)), ErrorCode::OutOfRange);
}

/// The ID the server appends, unsplit (as a radio with no drain bound
/// sends it).
const ID: &str = "DE N0CALL";

/// Split `chunks` into the operator's and the ID's, failing if any one
/// chunk carries both. `id_first` says which comes first.
fn split_id(chunks: &[String], operator: &str, id_first: bool) -> (Vec<String>, Vec<String>) {
    let squash = |c: &[String]| c.concat().replace(' ', "");
    let (first_text, _) = if id_first {
        ("DEN0CALL".to_string(), ())
    } else {
        (operator.replace(' ', ""), ())
    };
    let mut n = 0;
    while n < chunks.len() && squash(&chunks[..n]).len() < first_text.len() {
        n += 1;
    }
    assert_eq!(
        squash(&chunks[..n]),
        first_text,
        "a chunk carries both the operator's text and the ID: {chunks:?}"
    );
    let (a, b) = (chunks[..n].to_vec(), chunks[n..].to_vec());
    let (op, id) = if id_first { (b, a) } else { (a, b) };
    assert_eq!(squash(&op), operator.replace(' ', ""), "{chunks:?}");
    assert_eq!(squash(&id), "DEN0CALL", "{chunks:?}");
    (op, id)
}

/// Abort latency is the safety property (architect, 2026-10-07): every
/// chunk -- operator text and ID alike -- is split at the drain bound.
#[test]
fn a_drain_only_radio_never_holds_more_than_the_drain_bound() {
    let mut b = Bench::new(KENWOOD);
    b.radio.wpm = Some(10);
    b.arm().unwrap();
    b.sup.send(OWNER, "CQ CQ DE K9X", b.now).unwrap();
    b.run_for(Duration::from_secs(60));
    let chunks = b.radio.sent();
    split_id(&chunks, "CQ CQ DE K9X", false);
    for c in &chunks {
        assert!(c.len() <= 24, "{c:?}");
        assert!(
            estimate(c, 10) <= CwConfig::default().drain_bound,
            "{c:?} takes {:?}",
            estimate(c, 10)
        );
    }
}

#[test]
fn long_text_with_no_spaces_is_chunked_within_the_bound() {
    let mut b = Bench::new(KENWOOD);
    b.radio.wpm = Some(10);
    b.arm().unwrap();
    // A full TS-570D buffer with nowhere to break: ~25 s in one chunk if
    // word boundaries ruled.
    let text = "ABCDEFGHIJKLMNOPQRSTUVWX";
    b.sup.send(OWNER, text, b.now).unwrap();
    b.run_for(Duration::from_secs(60));
    let chunks = b.radio.sent();
    let (operator, _) = split_id(&chunks, text, false);
    assert!(operator.len() > 1);
    for c in &chunks {
        assert!(
            estimate(c, 10) <= CwConfig::default().drain_bound,
            "{c:?} takes {:?}",
            estimate(c, 10)
        );
    }
}

#[test]
fn the_id_never_shares_a_chunk_with_operator_text() {
    // Slow: the ID is split at the bound like anything else, and still
    // kept apart from what the operator typed.
    let mut b = Bench::new(KENWOOD);
    b.radio.wpm = Some(10);
    b.arm().unwrap();
    assert!(estimate(ID, 10) > CwConfig::default().drain_bound);
    b.sup.send(OWNER, "TEST", b.now).unwrap();
    b.run_for(Duration::from_secs(60));
    let (_, id) = split_id(&b.radio.sent(), "TEST", false);
    assert!(id.len() > 1, "split at the bound: {id:?}");

    // Fast, with room to spare: still not joined to the operator's text.
    let mut b = Bench::new(ICOM);
    b.radio.wpm = Some(30);
    b.arm().unwrap();
    b.sup.send(OWNER, "CQ CQ", b.now).unwrap();
    b.run_for(Duration::from_secs(60));
    assert_eq!(b.radio.sent(), vec!["CQ CQ", ID]);

    // Operator text that happens to contain the call is the operator's.
    let mut b = Bench::new(ICOM);
    b.arm().unwrap();
    b.sup.send(OWNER, "N0CALL TEST", b.now).unwrap();
    b.run_for(Duration::from_secs(60));
    assert_eq!(b.radio.sent(), vec!["N0CALL TEST", ID]);
}

#[test]
fn an_id_cut_by_an_abort_goes_first_on_the_next_send() {
    let mut b = Bench::new(KENWOOD);
    b.radio.wpm = Some(20);
    b.radio.busy_polls = 1_000;
    b.arm().unwrap();
    b.sup.send(OWNER, "E", b.now).unwrap();
    // Feed chunks one at a time until the ID has started, then abort.
    for _ in 0..50 {
        b.radio.busy_left = 0;
        b.drive();
        if b.radio.sent().iter().any(|c| c.starts_with("DE")) {
            break;
        }
    }
    let before = b.radio.sent();
    assert!(
        before.concat().replace(' ', "") != "EDEN0CALL",
        "cut short: {before:?}"
    );
    b.sup.abort(b.now);
    b.run_for(Duration::from_secs(8));
    b.radio.busy_polls = 0;
    b.radio.busy_left = 0;

    let n = b.radio.sent().len();
    b.sup.send(OWNER, "TEST", b.now).unwrap();
    b.run_for(Duration::from_secs(20));
    split_id(&b.radio.sent()[n..], "TEST", true);
}

#[test]
fn a_long_call_arms_on_a_small_chunk_radio_and_its_id_is_split() {
    let tiny = CwCapability {
        send_text: Some(CwSendText {
            max_chunk_chars: 4,
            ..KENWOOD.send_text.unwrap()
        }),
        ..KENWOOD
    };
    let mut b = Bench::new(tiny);
    assert_eq!(b.arm(), Ok(()));
    b.sup.send(OWNER, "TEST", b.now).unwrap();
    b.run_for(Duration::from_secs(60));
    let chunks = b.radio.sent();
    assert!(chunks.iter().all(|c| c.chars().count() <= 4), "{chunks:?}");
    split_id(&chunks, "TEST", false);
}

#[test]
fn chunks_prefer_word_boundaries_but_split_at_the_bound() {
    let c = chunks("CQ CQ DE N0CALL", 24, None, 20);
    assert_eq!(c, vec!["CQ CQ DE N0CALL"]);
    let c = chunks("ABCDEFGHIJ KLM", 6, None, 20);
    assert_eq!(c, vec!["ABCDEF", "GHIJ", "KLM"]);
    let c = chunks("AA BB CC", 5, None, 20);
    assert_eq!(c, vec!["AA BB", "CC"]);
    let bound = Duration::from_secs(3);
    let c = chunks("CQ CQ DE N0CALL", 24, Some(bound), 10);
    assert!(c.iter().all(|p| estimate(p, 10) <= bound), "{c:?}");
    assert_eq!(c.concat().replace(' ', ""), "CQCQDEN0CALL");
    // One character always goes, even one longer than the bound: there is
    // nothing smaller to send.
    let c = chunks("0", 24, Some(Duration::from_millis(1)), 10);
    assert_eq!(c, vec!["0"]);
}

#[test]
fn a_radio_stop_radio_sends_full_chunks() {
    let mut b = Bench::new(ICOM);
    b.radio.wpm = Some(10);
    b.arm().unwrap();
    b.sup.send(OWNER, "CQ CQ CQ", b.now).unwrap();
    b.run_for(Duration::from_secs(60));
    assert_eq!(b.radio.sent(), vec!["CQ CQ CQ", ID]);
}

#[test]
fn a_busy_flag_radio_is_fed_only_when_ready() {
    let mut b = Bench::new(KENWOOD);
    b.radio.wpm = Some(10);
    b.radio.busy_polls = 5;
    b.arm().unwrap();
    b.sup.send(OWNER, "AAAA BBBB CCCC", b.now).unwrap();
    b.run_for(Duration::from_secs(30));
    // Between any two chunks the radio said it was ready.
    let log = &b.radio.log;
    let sends: Vec<usize> = log
        .iter()
        .enumerate()
        .filter(|(_, a)| matches!(a, CwAction::SendChunk(_)))
        .map(|(i, _)| i)
        .collect();
    assert!(sends.len() >= 2);
    for w in sends.windows(2) {
        assert!(log[w[0]..w[1]].contains(&CwAction::Ready));
    }
    assert!(matches!(
        b.sup.sender(),
        SenderState::Stopped {
            reason: StopReason::Completed,
            ..
        }
    ));
}

#[test]
fn an_estimated_radio_waits_out_each_chunk() {
    let mut b = Bench::new(ICOM);
    b.radio.wpm = Some(20);
    b.arm().unwrap();
    let text = "PARIS PARIS PARIS PARIS PARIS PARIS";
    b.sup.send(OWNER, text, b.now).unwrap();
    b.drive();
    assert_eq!(b.radio.sent().len(), 1);
    b.run_for(Duration::from_millis(500));
    assert_eq!(b.radio.sent().len(), 1, "still keying the first chunk");
    assert!(matches!(b.sup.sender(), SenderState::Sending { .. }));
    b.run_for(Duration::from_secs(60));
    assert!(b.radio.sent().len() >= 2);
    assert!(matches!(
        b.sup.sender(),
        SenderState::Stopped {
            reason: StopReason::Completed,
            ..
        }
    ));
}

#[test]
fn a_send_is_refused_by_the_radio_outside_cw_mode() {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    b.radio.cw_mode = false;
    b.sup.send(OWNER, "CQ", b.now).unwrap();
    b.drive();
    assert!(b.radio.sent().is_empty());
    assert!(matches!(
        b.sup.sender(),
        SenderState::Stopped {
            reason: StopReason::Error,
            ..
        }
    ));
}

#[test]
fn power_found_above_the_clamp_stops_the_send_and_disarms() {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    b.radio.power = 50; // someone turned the knob
    b.sup.send(OWNER, "CQ", b.now).unwrap();
    b.drive();
    assert!(b.radio.sent().is_empty());
    assert!(b.sup.armed_state(OWNER).is_none());
    assert!(matches!(
        b.sup.sender(),
        SenderState::Stopped {
            reason: StopReason::Error,
            ..
        }
    ));
}

// ---------------------------------------------------------------------
// Abort and the watchdog
// ---------------------------------------------------------------------

#[test]
fn any_console_may_abort_and_the_stop_goes_first() {
    let mut b = Bench::new(KENWOOD);
    b.radio.wpm = Some(10);
    b.radio.busy_polls = 2;
    b.arm().unwrap();
    b.sup
        .send(OWNER, "CQ CQ CQ DE N0CALL N0CALL K", b.now)
        .unwrap();
    b.drive();
    let sent = b.radio.sent().len();
    let log_len = b.radio.log.len();

    b.sup.abort(b.now); // from anyone: no console named at all
    let next = b.sup.next_action(b.now);
    assert_eq!(next, Some(CwAction::Stop), "the stop goes first");
    b.sup.complete(CwOutcome::Done, b.now);
    b.run_for(Duration::from_secs(30));
    assert_eq!(b.radio.sent().len(), sent, "no chunk after abort");
    assert!(b.radio.log[log_len..]
        .iter()
        .all(|a| !matches!(a, CwAction::SendChunk(_))));
    assert!(matches!(
        b.sup.sender(),
        SenderState::Stopped {
            reason: StopReason::Aborted,
            ..
        }
    ));
    // An abort does not disarm.
    assert!(b.sup.armed_state(OWNER).is_some());
}

#[test]
fn abort_while_a_chunk_is_on_the_wire_still_stops_next() {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    b.sup.send(OWNER, "CQ CQ", b.now).unwrap();
    // Run to the chunk, but abort before its completion arrives.
    loop {
        let a = b.sup.next_action(b.now).unwrap();
        let outcome = b.radio.answer(&a, b.now);
        if matches!(a, CwAction::SendChunk(_)) {
            b.sup.abort(b.now);
            b.sup.complete(outcome, b.now);
            break;
        }
        b.sup.complete(outcome, b.now);
    }
    assert_eq!(b.sup.next_action(b.now), Some(CwAction::Stop));
}

#[test]
fn a_drain_only_abort_reports_draining_until_the_radio_is_idle() {
    let mut b = Bench::new(KENWOOD);
    b.radio.busy_polls = 10;
    b.arm().unwrap();
    b.sup.send(OWNER, "CQ CQ CQ CQ", b.now).unwrap();
    b.drive();
    b.sup.abort(b.now);
    b.drive();
    assert!(matches!(b.sup.sender(), SenderState::Draining { .. }));
    // Until the chunk already handed over has had time to key.
    b.run_for(Duration::from_secs(10));
    assert!(matches!(
        b.sup.sender(),
        SenderState::Stopped {
            reason: StopReason::Aborted,
            ..
        }
    ));
}

#[test]
fn abort_with_nothing_sending_is_harmless() {
    let mut b = Bench::new(KENWOOD);
    b.sup.abort(b.now);
    b.drive();
    assert!(b.radio.log.is_empty());
    b.arm().unwrap();
    let n = b.radio.log.len();
    b.sup.abort(b.now);
    b.drive();
    assert_eq!(b.radio.log.len(), n);
}

#[test]
fn a_send_that_outlives_its_estimate_is_stopped_and_disarmed() {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    b.radio.never_ready = true; // the keyer has wedged
    b.sup.send(OWNER, "CQ", b.now).unwrap();
    b.run_for(Duration::from_secs(30));
    assert!(b.radio.log.contains(&CwAction::Stop));
    assert!(matches!(
        b.sup.sender(),
        SenderState::Stopped {
            reason: StopReason::KeyDownCap,
            ..
        }
    ));
    // Disarm waits on the drain; once the radio is idle it restores.
    b.radio.never_ready = false;
    b.run_for(Duration::from_secs(5));
    assert!(b.sup.armed_state(OWNER).is_none());
    assert_eq!(b.radio.power, 100);
}

#[test]
fn a_stale_watchdog_cannot_stop_the_next_send() {
    let mut b = Bench::new(KENWOOD);
    b.radio.wpm = Some(20);
    b.arm().unwrap();
    b.sup.send(OWNER, "E", b.now).unwrap();
    b.run_for(Duration::from_secs(10));
    assert!(matches!(
        b.sup.sender(),
        SenderState::Stopped {
            reason: StopReason::Completed,
            ..
        }
    ));
    // A long second job, running past where the first one's deadline
    // would have fallen.
    b.sup
        .send(OWNER, "PARIS PARIS PARIS PARIS PARIS PARIS PARIS", b.now)
        .unwrap();
    b.radio.busy_polls = 40; // ~2 s busy per chunk
    b.run_for(Duration::from_secs(8));
    assert!(
        !matches!(
            b.sup.sender(),
            SenderState::Stopped {
                reason: StopReason::KeyDownCap,
                ..
            }
        ),
        "{:?}",
        b.sup.sender()
    );
}

// ---------------------------------------------------------------------
// rigctl, liveness
// ---------------------------------------------------------------------

#[test]
fn rigctl_ptt_is_blocked_from_arming_until_the_restore_is_verified() {
    let mut b = Bench::new(KENWOOD);
    assert!(!b.sup.blocks_external_ptt());
    let ticket = b.sup.arm(OWNER, "N0CALL", None, b.now).unwrap();
    assert!(b.sup.blocks_external_ptt(), "while arming");
    b.drive();
    assert_eq!(b.reply(ticket), Some(Ok(())));
    assert!(b.sup.blocks_external_ptt(), "while armed");
    b.sup.disarm(OWNER, b.now).unwrap();
    assert!(b.sup.blocks_external_ptt(), "while restoring");
    b.drive();
    assert!(!b.sup.blocks_external_ptt(), "after a verified restore");
}

#[test]
fn the_pump_runs_fast_only_while_something_is_live() {
    let mut b = Bench::new(KENWOOD);
    assert!(!b.sup.is_live());
    b.arm().unwrap();
    assert!(!b.sup.is_live(), "an idle arm needs no fast tick");
    b.radio.never_ready = true;
    b.sup.send(OWNER, "CQ", b.now).unwrap();
    b.drive();
    assert!(b.sup.is_live());
}

#[test]
fn arming_is_announced_for_the_log() {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    assert!(b.notices.iter().any(|n| matches!(
        n,
        CwNotice::Armed { owner, callsign, power_raw: 5 } if *owner == OWNER && callsign == "N0CALL"
    )));
}

#[test]
fn an_id_cut_off_by_an_abort_after_the_first_chunk_is_still_owed() {
    let mut b = Bench::new(KENWOOD);
    b.radio.wpm = Some(10);
    b.radio.busy_polls = 1_000; // the keyer stays busy after chunk one
    b.arm().unwrap();
    b.sup.send(OWNER, "CQ CQ", b.now).unwrap();
    b.drive();
    let out = b.radio.sent();
    assert_eq!(out.len(), 1, "only the first chunk is out: {out:?}");
    assert!(!out[0].contains("DE"));
    b.sup.abort(b.now);
    b.run_for(Duration::from_secs(10));
    b.radio.busy_polls = 0;
    b.radio.busy_left = 0;
    let n = b.radio.sent().len();
    b.sup.send(OWNER, "CQ", b.now).unwrap();
    b.run_for(Duration::from_secs(20));
    assert!(
        b.radio.sent()[n..]
            .concat()
            .replace(' ', "")
            .contains("DEN0CALL"),
        "{:?}",
        b.radio.sent()
    );
}

/// Pins ADR 0023 §4: on a `DrainOnly` radio the worst-case drain after
/// an abort is the 3 s bound -- for operator text and ID alike, even for
/// the longest valid callsign (`DE` + nine `0`s + a 13-unit letter, 27.5 s
/// unsplit) at the TS-570D's slowest keyer speed. A single character is
/// never split; the longest (`0`, 2.28 s at 10 wpm) is inside the bound.
#[test]
fn the_worst_case_drain_is_the_bound_in_the_adr() {
    assert_eq!(CwConfig::default().drain_bound, Duration::from_secs(3));
    // At the conservative 4:1 dash weighting: five 4-unit dashes and
    // four gaps, 24 units of 120 ms.
    assert_eq!(estimate("0", 10), Duration::from_millis(2_880));
    let mut b = Bench::new(KENWOOD);
    b.radio.wpm = Some(10); // KENWOOD's slowest
    let ticket = b.sup.arm(OWNER, "000000000Q", None, b.now).unwrap();
    b.drive();
    assert_eq!(b.reply(ticket), Some(Ok(())));
    b.sup.send(OWNER, "ABCDEFGHIJKL", b.now).unwrap();
    b.run_for(Duration::from_secs(90));
    let chunks = b.radio.sent();
    assert!(chunks.concat().contains("000000000Q"), "{chunks:?}");
    let longest = chunks.iter().map(|c| estimate(c, 10)).max().unwrap();
    assert!(longest <= Duration::from_secs(3), "{longest:?}");
}

// ---------------------------------------------------------------------
// Ready is buffer space, not idle (safety review, 2026-10-07)
// ---------------------------------------------------------------------

/// A TS-570D-shaped radio whose `KY;` says "buffer space" and so answers
/// ready while it is still keying.
fn buffer_space_bench(wpm: u16) -> Bench {
    let mut b = Bench::new(KENWOOD);
    b.radio.wpm = Some(wpm);
    b.radio.buffer_space_ready = true;
    b
}

#[test]
fn a_ready_buffer_is_not_fed_ahead_of_what_it_is_keying() {
    // The reviewer's scenario: a 10 wpm job of some fourteen chunks used
    // to be handed over entirely at t=0.
    let mut b = buffer_space_bench(10);
    b.arm().unwrap();
    let text = "ABCDE FGHIJ KLMNO PQRST";
    b.sup.send(OWNER, text, b.now).unwrap();
    b.drive();
    assert_eq!(
        b.radio.sent().len(),
        1,
        "one chunk at a time: {:?}",
        b.radio.sent()
    );
    // At any moment the radio holds no more than the drain bound.
    for _ in 0..600 {
        b.run_for(Duration::from_millis(100));
        if let Some(t) = b.radio.keyed_until {
            let left = t.saturating_duration_since(b.now);
            assert!(
                left <= CwConfig::default().drain_bound + Duration::from_millis(100),
                "{left:?} buffered at {:?}",
                b.elapsed()
            );
        }
    }
    assert!(b.radio.violations.is_empty(), "{:?}", b.radio.violations);
}

#[test]
fn abort_and_disarm_never_raise_power_under_buffered_text() {
    let mut b = buffer_space_bench(10);
    b.arm().unwrap();
    b.sup.send(OWNER, "ABCDE FGHIJ KLMNO PQRST", b.now).unwrap();
    b.drive();
    b.sup.abort(b.now);
    b.sup.disarm(OWNER, b.now).unwrap();
    b.drive();
    assert_eq!(
        b.radio.power, 5,
        "not while the first chunk is still keying"
    );
    b.run_for(Duration::from_secs(10));
    assert_eq!(b.radio.power, 100, "restored once it has finished");
    assert!(b.radio.violations.is_empty(), "{:?}", b.radio.violations);
}

#[test]
fn a_job_is_complete_only_when_its_text_has_had_time_to_key() {
    let mut b = buffer_space_bench(10);
    b.arm().unwrap();
    b.sup.send(OWNER, "E", b.now).unwrap();
    b.drive();
    // "E DE N0CALL" at 10 wpm is several seconds; the buffer said ready
    // at once.
    b.run_for(Duration::from_millis(500));
    assert!(
        matches!(b.sup.sender(), SenderState::Sending { .. }),
        "{:?}",
        b.sup.sender()
    );
    b.run_for(Duration::from_secs(30));
    assert!(matches!(
        b.sup.sender(),
        SenderState::Stopped {
            reason: StopReason::Completed,
            ..
        }
    ));
    // And a disarm right after completion restores at once, safely.
    b.sup.disarm(OWNER, b.now).unwrap();
    b.drive();
    assert_eq!(b.radio.power, 100);
    assert!(b.radio.violations.is_empty(), "{:?}", b.radio.violations);
}

#[test]
fn every_way_out_waits_for_keyed_text_before_restoring() {
    for ending in ["detach", "shutdown", "watchdog"] {
        let mut b = buffer_space_bench(10);
        b.arm().unwrap();
        b.sup.send(OWNER, "ABCDE FGHIJ KLMNO PQRST", b.now).unwrap();
        b.drive();
        match ending {
            "detach" => b.sup.detached(OWNER, b.now),
            "shutdown" => b.sup.shutdown(b.now),
            _ => {
                // Wedge: the radio stops reporting buffer space.
                b.radio.buffer_space_ready = false;
                b.radio.never_ready = true;
            }
        }
        b.run_for(Duration::from_secs(90));
        assert_eq!(b.radio.power, 100, "{ending}");
        assert!(
            b.radio.violations.is_empty(),
            "{ending}: {:?}",
            b.radio.violations
        );
    }
}

#[test]
fn a_radio_stop_keyer_may_be_restored_once_stopped() {
    let mut b = Bench::new(ICOM);
    b.radio.stops = true;
    b.radio.power = 200;
    b.arm().unwrap();
    b.sup.send(OWNER, "CQ CQ CQ", b.now).unwrap();
    b.drive();
    b.sup.abort(b.now);
    b.sup.disarm(OWNER, b.now).unwrap();
    b.run_for(Duration::from_secs(1));
    assert_eq!(b.radio.power, 200);
    assert!(b.radio.violations.is_empty(), "{:?}", b.radio.violations);
}

// ---------------------------------------------------------------------
// Keyer speed can change under a send (safety review, 2026-10-07)
// ---------------------------------------------------------------------

#[test]
fn a_slower_knob_mid_send_replans_within_the_drain_bound() {
    let mut b = buffer_space_bench(30);
    b.arm().unwrap();
    b.sup
        .send(OWNER, "ABCDE FGHIJ KLMNO PQRST UVWXY", b.now)
        .unwrap();
    b.drive();
    assert_eq!(b.radio.sent().len(), 1);
    // The operator turns the speed knob down on the front panel.
    b.radio.wpm = Some(10);
    let first = b.radio.sent().len();
    b.run_for(Duration::from_secs(120));
    let later = &b.radio.sent()[first..];
    assert!(!later.is_empty());
    for c in later {
        assert!(
            estimate(c, 10) <= CwConfig::default().drain_bound,
            "{c:?} takes {:?} at the new speed",
            estimate(c, 10)
        );
    }
    assert!(b.radio.violations.is_empty(), "{:?}", b.radio.violations);
    assert!(
        !matches!(
            b.sup.sender(),
            SenderState::Stopped {
                reason: StopReason::KeyDownCap,
                ..
            }
        ),
        "the watchdog follows the new speed: {:?}",
        b.sup.sender()
    );
}

#[test]
fn speed_is_read_before_every_chunk_on_a_drain_only_radio() {
    let mut b = buffer_space_bench(20);
    b.arm().unwrap();
    b.radio.log.clear();
    b.sup.send(OWNER, "ABCDE FGHIJ", b.now).unwrap();
    b.run_for(Duration::from_secs(60));
    let log = &b.radio.log;
    let sends: Vec<usize> = log
        .iter()
        .enumerate()
        .filter(|(_, a)| matches!(a, CwAction::SendChunk(_)))
        .map(|(i, _)| i)
        .collect();
    assert!(sends.len() > 1);
    let mut from = 0;
    for i in sends {
        assert!(
            log[from..i].contains(&CwAction::ReadStatus),
            "no speed read before chunk at {i}: {log:?}"
        );
        from = i + 1;
    }
}

// ---------------------------------------------------------------------
// An owed ID is reported, never sent on the way out (safety review)
// ---------------------------------------------------------------------

fn owed_notice(b: &Bench) -> Option<String> {
    b.notices.iter().find_map(|n| match n {
        CwNotice::IdOwed { callsign, .. } => Some(callsign.clone()),
        _ => None,
    })
}

/// Send, let the first chunk key, abort before the ID: returns the bench.
fn keyed_without_its_id() -> Bench {
    let mut b = buffer_space_bench(20);
    b.arm().unwrap();
    b.sup.send(OWNER, "ABCDE FGHIJ KLMNO", b.now).unwrap();
    b.drive();
    assert_eq!(b.radio.sent().len(), 1);
    b.sup.abort(b.now);
    b.run_for(Duration::from_secs(5));
    b
}

#[test]
fn ending_an_arm_that_owes_its_id_reports_it_and_sends_nothing() {
    for ending in ["disarm", "detach", "idle"] {
        let mut b = keyed_without_its_id();
        let sent = b.radio.sent().len();
        assert_eq!(
            b.sup.id_owed().as_deref(),
            Some("N0CALL"),
            "owed while armed"
        );
        match ending {
            "disarm" => {
                b.sup.disarm(OWNER, b.now).unwrap();
            }
            "detach" => b.sup.detached(OWNER, b.now),
            _ => b.run_for(CwConfig::default().idle_ttl + Duration::from_secs(1)),
        }
        b.run_for(Duration::from_secs(5));
        assert!(b.sup.armed_state(OWNER).is_none(), "{ending}");
        assert_eq!(b.radio.sent().len(), sent, "{ending}: abort means stop");
        assert_eq!(owed_notice(&b).as_deref(), Some("N0CALL"), "{ending}");
        assert_eq!(
            b.sup.id_owed().as_deref(),
            Some("N0CALL"),
            "{ending}: still shown"
        );
    }
}

#[test]
fn nothing_is_owed_if_nothing_keyed() {
    let mut b = Bench::new(KENWOOD);
    b.arm().unwrap();
    b.radio.never_ready = true;
    b.sup.send(OWNER, "CQ", b.now).unwrap();
    b.drive();
    b.sup.abort(b.now);
    b.sup.disarm(OWNER, b.now).unwrap();
    b.run_for(Duration::from_secs(5));
    assert!(b.radio.sent().is_empty());
    assert_eq!(b.sup.id_owed(), None);
    assert_eq!(owed_notice(&b), None);
}

#[test]
fn an_owed_id_is_cleared_once_an_id_is_sent() {
    let mut b = keyed_without_its_id();
    b.sup.disarm(OWNER, b.now).unwrap();
    b.run_for(Duration::from_secs(5));
    assert!(b.sup.id_owed().is_some());
    b.arm().unwrap();
    b.sup.send(OWNER, "E", b.now).unwrap();
    b.run_for(Duration::from_secs(30));
    assert_eq!(b.sup.id_owed(), None);
}

#[test]
fn estimates_assume_the_heaviest_weighting_a_keyer_may_use() {
    // Safety review 6: a TS-570D's keyer weighting can stretch a dash to
    // 4:1. Every bound -- chunk split, drain, watchdog, cap -- is computed
    // from `estimate`, so it errs long: a 4-unit dash, not the textbook 3.
    assert_eq!(estimate("E", 20), Duration::from_millis(60));
    assert_eq!(estimate("T", 20), Duration::from_millis(240));
    // A character the codec cannot key is costed at least as long as the
    // longest real one with its gap (`0`: 24 + 3 units).
    assert!(estimate("\u{2603}", 20) >= Duration::from_millis(27 * 60));
}
