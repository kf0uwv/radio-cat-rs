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

//! The CW transmit supervisor: ADR 0023 §3-§5, written once for every
//! radio.
//!
//! # Why it is a state machine with no I/O
//!
//! Everything that ends, bounds or reverses a transmission lives here,
//! because the server outlives the client. That makes this the code whose
//! bugs key a transmitter, so it is built to be tested exhaustively: it
//! never touches a radio, a socket, a thread or a clock. The pump asks
//! [`CwSupervisor::next_action`] what to do, does it through the radio's
//! own `NativeCw` methods, and reports the answer with
//! [`CwSupervisor::complete`]. Time is a parameter. The same tests run on
//! Linux and Windows and cannot be flaky.
//!
//! One action is outstanding at a time. A request (`arm`, `send`, `abort`,
//! `disarm`, a console hanging up, shutdown) only changes the plan; the
//! radio hears about it on the next `next_action`. An abort replaces the
//! plan with a stop, so the stop is the very next thing on the wire after
//! whatever exchange is already in flight.
//!
//! # The rules, and where each is enforced
//!
//! - **Arm**: the radio must already be in a CW mode and not transmitting
//!   (decision 7). Power and (if the radio needs it) break-in are read,
//!   set, and read back; a read-back that disagrees refuses the arm and
//!   restores what was changed.
//! - **Send**: owner only, one at a time, under the key-down cap including
//!   the ID the server appends (decisions 4-5). Before the first chunk the
//!   mode and power are read again; power above the clamp stops the send
//!   and disarms.
//! - **ID**: `" DE <CALL>"` is appended when no ID has been sent this arm,
//!   or ten minutes have passed since the last. It counts as sent only
//!   once the chunk carrying it has been handed to the radio.
//! - **Watchdog**: a send that outlives its estimate plus margin is stopped
//!   and the arm dropped. The deadline lives in the job, so a stale one
//!   cannot touch the next job.
//! - **Restore**: on disarm, owner hang-up, watchdog, idle expiry, power
//!   found above the clamp, and shutdown. Only what was changed is written
//!   back, never under a keyed chunk, and every write is read back. A
//!   restore that fails is reported, keeps the arm locked, and is retried
//!   until a read-back proves it: this code never reports a radio restored
//!   that it has not seen restored.
//! - **Abort**: from anyone, never refused.
//! - **rigctl**: [`CwSupervisor::blocks_external_ptt`] is true from the
//!   moment an arm is requested until a restore is verified (decision 8).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use cat_framework::capabilities::{
    BreakInKind, CwAbort, CwCapability, CwPacing, CwSendText, TxGateCapability,
};
use cat_morse::{timing, Fist, Speed};
use cat_native::{
    ArmedState, ConsoleId, ErrorCode, HostRefusal, RestoreStatus, SenderState, StopReason,
};

/// The supervisor's policy numbers. Defaults are ADR 0023's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CwConfig {
    /// Ceiling on one send's estimated keying time (decision 5).
    pub key_down_cap: Duration,
    /// Longest a `DrainOnly` radio may be left keying after an abort,
    /// as a target for joining words into a chunk (§4).
    pub drain_bound: Duration,
    /// An arm with no send for this long is dropped and restored.
    pub idle_ttl: Duration,
    /// How often the ID is owed while armed (decision 4).
    pub id_interval: Duration,
    /// How soon to ask a busy keyer again.
    pub tick: Duration,
    /// Added to each estimated chunk before an `Estimated` radio is fed
    /// again.
    pub pacing_margin: Duration,
    /// Added to a job's estimate (after a 20% allowance) for its watchdog.
    pub watchdog_margin: Duration,
    /// How long to wait before trying a failed restore again.
    pub restore_retry: Duration,
}

impl Default for CwConfig {
    fn default() -> Self {
        Self {
            key_down_cap: Duration::from_secs(60),
            drain_bound: Duration::from_secs(3),
            idle_ttl: Duration::from_secs(600),
            id_interval: Duration::from_secs(600),
            tick: Duration::from_millis(50),
            pacing_margin: Duration::from_millis(250),
            watchdog_margin: Duration::from_secs(3),
            restore_retry: Duration::from_secs(5),
        }
    }
}

/// Names the answer to an [`CwSupervisor::arm`] or
/// [`CwSupervisor::disarm`], which arrives later as [`CwNotice::Reply`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ticket(u64);

/// One thing to do to the radio, through its `NativeCw` implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwAction {
    /// Mode, transmit state and keyer speed -> [`CwOutcome::Status`].
    ReadStatus,
    ReadPower,
    SetPower(u16),
    ReadBreakIn,
    SetBreakIn(BreakInKind),
    /// `CwPacing::BufferFlag` only -> [`CwOutcome::Ready`].
    Ready,
    /// Already upper case, within the charset and the radio's chunk size.
    SendChunk(String),
    /// Stop the keyer. A `DrainOnly` radio answers `Done` and keeps
    /// draining; the supervisor waits that out.
    Stop,
}

/// The radio's answer to a [`CwAction`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwOutcome {
    /// A write or a send was accepted.
    Done,
    Status {
        cw_mode: bool,
        transmitting: bool,
        /// `None` if the radio cannot report it; the slowest speed it
        /// supports is then assumed, which overestimates durations.
        wpm: Option<u16>,
    },
    Power(u16),
    BreakIn(BreakInKind),
    Ready(bool),
    /// The exchange failed. The text is shown to the operator.
    Failed(String),
}

/// Why an arm ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisarmReason {
    Requested,
    Disconnected,
    KeyDownCap,
    PowerAboveClamp,
    IdleTimeout,
    Shutdown,
    /// Arming itself failed after something had been changed.
    ArmFailed,
}

/// What happened, for the pump to log and to answer consoles with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwNotice {
    /// The answer to an `arm` or a `disarm`.
    Reply {
        ticket: Ticket,
        result: Result<(), HostRefusal>,
    },
    /// Logged with the peer address: the dummy-load acknowledgement is an
    /// attestation, and this is its record.
    Armed {
        owner: ConsoleId,
        callsign: String,
        power_raw: u16,
    },
    JobEnded {
        job: u32,
        reason: StopReason,
        message: Option<String>,
    },
    /// A restore write failed or read back wrong. Retried until it works.
    RestoreFailed { attempts: u32, error: String },
    /// Every changed setting has been written back **and read back**.
    Restored { reason: DisarmReason },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    Status,
    SnapPower,
    SnapBreakIn,
    SetPower(u16),
    VerifyPower(u16),
    SetBreakIn(BreakInKind),
    VerifyBreakIn(BreakInKind),
    PowerAtMost(u16),
    WaitReady,
    WaitUntil(Instant),
    /// The chunk, and whether it is the job's last.
    Send(String, bool),
    Stop,
}

impl Step {
    fn action(&self) -> Option<CwAction> {
        Some(match self {
            Step::Status => CwAction::ReadStatus,
            Step::SnapPower | Step::VerifyPower(_) | Step::PowerAtMost(_) => CwAction::ReadPower,
            Step::SnapBreakIn | Step::VerifyBreakIn(_) => CwAction::ReadBreakIn,
            Step::SetPower(raw) => CwAction::SetPower(*raw),
            Step::SetBreakIn(kind) => CwAction::SetBreakIn(*kind),
            Step::WaitReady => CwAction::Ready,
            Step::Send(text, _) => CwAction::SendChunk(text.clone()),
            Step::Stop => CwAction::Stop,
            Step::WaitUntil(_) => return None,
        })
    }
}

/// What the radio was before the arm, and what the arm changed.
#[derive(Debug, Clone, Copy, Default)]
struct Snapshot {
    power: Option<u16>,
    break_in: Option<BreakInKind>,
    power_changed: bool,
    break_in_changed: bool,
}

#[derive(Debug, Clone)]
struct Arm {
    owner: ConsoleId,
    callsign: String,
    cap: Duration,
    power_raw: u16,
    wpm: u16,
    last_activity: Instant,
    last_id: Option<Instant>,
}

#[derive(Debug, Clone)]
struct Job {
    id: u32,
    /// Operator text and appended ID together, for estimates.
    text: String,
    /// What the operator asked to send.
    operator: String,
    /// `DE <CALL>`, when owed: always a chunk of its own, never split.
    id_text: Option<String>,
    total: u32,
    sent: u32,
    started: Instant,
    deadline: Instant,
    has_id: bool,
    chunk_in_radio: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    StayArmed,
    Restore(DisarmReason, Option<Ticket>),
}

#[derive(Debug, Clone)]
enum Activity {
    Idle,
    Arming {
        ticket: Ticket,
        owner: ConsoleId,
        callsign: String,
        cap: Duration,
        wpm: u16,
    },
    Sending(Job),
    Stopping {
        job: u32,
        reason: StopReason,
        message: Option<String>,
        then: After,
        /// Give up waiting for a `DrainOnly` radio to go idle at this
        /// point, so a wedged keyer cannot hold a restore off forever.
        drain_deadline: Instant,
    },
    Restoring {
        reason: DisarmReason,
        ticket: Option<Ticket>,
        attempts: u32,
        last_error: Option<String>,
        retry_at: Option<Instant>,
    },
}

/// See the module documentation.
pub struct CwSupervisor {
    send_text: Option<CwSendText>,
    gate: Option<TxGateCapability>,
    slowest_wpm: u16,
    config: CwConfig,
    arm: Option<Arm>,
    activity: Activity,
    plan: VecDeque<Step>,
    outstanding: Option<Step>,
    ready_retry_at: Option<Instant>,
    snap: Snapshot,
    last_chunk_end: Option<Instant>,
    notices: Vec<CwNotice>,
    next_ticket: u64,
    next_job: u32,
    last_sender: SenderState,
    shutting_down: bool,
}

impl CwSupervisor {
    pub fn new(capability: CwCapability, config: CwConfig) -> Self {
        Self {
            send_text: capability.send_text,
            gate: capability.tx_gate,
            slowest_wpm: capability.keyer_wpm.map_or(5, |r| r.min.max(1)),
            config,
            arm: None,
            activity: Activity::Idle,
            plan: VecDeque::new(),
            outstanding: None,
            ready_retry_at: None,
            snap: Snapshot::default(),
            last_chunk_end: None,
            notices: Vec::new(),
            next_ticket: 1,
            next_job: 1,
            last_sender: SenderState::Idle,
            shutting_down: false,
        }
    }

    // -----------------------------------------------------------------
    // Requests
    // -----------------------------------------------------------------

    /// Ask to take the transmit arm. Refused now, or answered later with
    /// [`CwNotice::Reply`] once power is clamped and read back.
    ///
    /// `callsign` has been checked by the session; it is upper-cased here.
    pub fn arm(
        &mut self,
        owner: ConsoleId,
        callsign: &str,
        key_down_cap_s: Option<u16>,
        _now: Instant,
    ) -> Result<Ticket, HostRefusal> {
        let (Some(send), Some(gate)) = (self.send_text, self.gate) else {
            return Err(refusal(
                ErrorCode::Unsupported,
                "this radio cannot send CW text over CAT",
            ));
        };
        if self.shutting_down {
            return Err(refusal(ErrorCode::Forbidden, "the server is shutting down"));
        }
        if let Activity::Restoring { last_error, .. } = &self.activity {
            return Err(refusal(
                ErrorCode::Forbidden,
                &match last_error {
                    Some(e) => format!("the last arm's restore has not been verified: {e}"),
                    None => "the last arm is still being restored".to_string(),
                },
            ));
        }
        let held_by = match (&self.arm, &self.activity) {
            (Some(arm), _) => Some(arm.owner),
            (None, Activity::Arming { owner, .. }) => Some(*owner),
            _ => None,
        };
        if let Some(holder) = held_by {
            return Err(refusal(
                ErrorCode::Forbidden,
                if holder == owner {
                    "this console is already armed"
                } else {
                    "another console holds the CW transmit arm"
                },
            ));
        }

        let id_len = 3 + callsign.chars().count();
        if id_len > usize::from(send.max_chunk_chars) {
            return Err(refusal(
                ErrorCode::Forbidden,
                &format!(
                    "\"DE {callsign}\" is {id_len} characters; this radio sends at most {} at once, \
                     and the ID is never split",
                    send.max_chunk_chars
                ),
            ));
        }

        let cap = key_down_cap_s
            .map(|s| Duration::from_secs(u64::from(s)))
            .unwrap_or(self.config.key_down_cap)
            .min(self.config.key_down_cap);
        let ticket = self.ticket();
        self.snap = Snapshot::default();
        self.plan.clear();
        self.plan.push_back(Step::Status);
        self.plan.push_back(Step::SnapPower);
        if send.requires_break_in.is_some() {
            self.plan.push_back(Step::SnapBreakIn);
        }
        self.plan.push_back(Step::SetPower(gate.low_power_raw));
        self.plan.push_back(Step::VerifyPower(gate.low_power_raw));
        if let Some(kind) = send.requires_break_in {
            self.plan.push_back(Step::SetBreakIn(kind));
            self.plan.push_back(Step::VerifyBreakIn(kind));
        }
        self.activity = Activity::Arming {
            ticket,
            owner,
            callsign: callsign.to_ascii_uppercase(),
            cap,
            wpm: self.slowest_wpm,
        };
        Ok(ticket)
    }

    /// Queue text. `Ok(job)` means queued; progress is [`Self::sender`].
    pub fn send(&mut self, owner: ConsoleId, text: &str, now: Instant) -> Result<u32, HostRefusal> {
        let Some(send) = self.send_text else {
            return Err(refusal(
                ErrorCode::Unsupported,
                "this radio cannot send CW text over CAT",
            ));
        };
        let Some(arm) = &self.arm else {
            return Err(refusal(ErrorCode::Forbidden, "not armed"));
        };
        if arm.owner != owner {
            return Err(refusal(
                ErrorCode::Forbidden,
                "another console holds the CW transmit arm",
            ));
        }
        if !matches!(self.activity, Activity::Idle) {
            return Err(refusal(
                ErrorCode::Forbidden,
                "a send is already in progress",
            ));
        }
        let words: Vec<String> = text
            .split_whitespace()
            .map(str::to_ascii_uppercase)
            .collect();
        if words.is_empty() {
            return Err(refusal(ErrorCode::OutOfRange, "nothing to send"));
        }
        let operator = words.join(" ");
        if let Some(c) = send.rejected_char(&operator) {
            return Err(refusal(
                ErrorCode::OutOfRange,
                &format!("this keyer cannot send {c:?}"),
            ));
        }
        let id_due = arm.last_id.map_or(true, |t| {
            now.saturating_duration_since(t) >= self.config.id_interval
        });
        let id_text = id_due.then(|| format!("DE {}", arm.callsign));
        let full = match &id_text {
            Some(id) => format!("{operator} {id}"),
            None => operator.clone(),
        };
        let est = estimate(&full, arm.wpm);
        if est > arm.cap {
            return Err(refusal(
                ErrorCode::OutOfRange,
                &format!(
                    "about {:.1} s at {} wpm{}; the key-down cap is {} s",
                    est.as_secs_f64(),
                    arm.wpm,
                    if id_due { ", with the ID" } else { "" },
                    arm.cap.as_secs()
                ),
            ));
        }

        let id = self.next_job;
        self.next_job = self.next_job.wrapping_add(1);
        let job = Job {
            id,
            total: full.chars().count() as u32,
            text: full,
            operator,
            id_text,
            sent: 0,
            started: now,
            deadline: now + self.watchdog_for(est),
            has_id: id_due,
            chunk_in_radio: false,
        };
        self.plan.clear();
        self.plan.push_back(Step::Status);
        self.plan
            .push_back(Step::PowerAtMost(self.gate.map_or(0, |g| g.low_power_raw)));
        self.activity = Activity::Sending(job);
        if let Some(arm) = &mut self.arm {
            arm.last_activity = now;
        }
        Ok(id)
    }

    /// Stop any send now. From any console, armed or not; never refused.
    pub fn abort(&mut self, now: Instant) {
        if matches!(self.activity, Activity::Sending(_)) {
            self.begin_stop(StopReason::Aborted, None, After::StayArmed, now);
        }
    }

    /// Release the arm. Answered with [`CwNotice::Reply`] once the
    /// restore is verified -- or `Err` the first time it fails.
    pub fn disarm(&mut self, owner: ConsoleId, now: Instant) -> Result<Ticket, HostRefusal> {
        match &self.arm {
            Some(arm) if arm.owner == owner => {}
            Some(_) => {
                return Err(refusal(
                    ErrorCode::Forbidden,
                    "another console holds the CW transmit arm",
                ))
            }
            None => {
                return Err(refusal(
                    ErrorCode::Forbidden,
                    if matches!(self.activity, Activity::Arming { .. }) {
                        "arming is still in progress"
                    } else {
                        "not armed"
                    },
                ))
            }
        }
        if let Activity::Stopping {
            then: After::Restore(_, Some(_)),
            ..
        } = self.activity
        {
            return Err(refusal(ErrorCode::Forbidden, "already disarming"));
        }
        let ticket = self.ticket();
        self.end_arm(
            DisarmReason::Requested,
            StopReason::Aborted,
            Some(ticket),
            now,
        );
        Ok(ticket)
    }

    /// A console went away. If it held the arm, stop and restore.
    pub fn detached(&mut self, id: ConsoleId, now: Instant) {
        if let Activity::Arming { owner, .. } = &self.activity {
            if *owner == id {
                self.refuse_arm(ErrorCode::Forbidden, "the console hung up");
            }
            return;
        }
        if self.arm.as_ref().is_some_and(|a| a.owner == id) {
            self.end_arm(
                DisarmReason::Disconnected,
                StopReason::Disconnected,
                None,
                now,
            );
        }
    }

    /// The server is stopping: stop and restore. Poll [`Self::is_settled`]
    /// to know when the radio is back.
    pub fn shutdown(&mut self, now: Instant) {
        self.shutting_down = true;
        if let Activity::Arming { .. } = self.activity {
            self.refuse_arm(ErrorCode::Forbidden, "the server is shutting down");
        } else if self.arm.is_some() {
            self.end_arm(DisarmReason::Shutdown, StopReason::Aborted, None, now);
        }
    }

    // -----------------------------------------------------------------
    // Driving
    // -----------------------------------------------------------------

    /// The next thing to do to the radio, if anything is due at `now`.
    pub fn next_action(&mut self, now: Instant) -> Option<CwAction> {
        if self.outstanding.is_some() {
            return None;
        }
        self.check_timers(now);
        for _ in 0..64 {
            match self.plan.front().cloned() {
                None => {
                    if !self.finish(now) {
                        return None;
                    }
                }
                Some(Step::WaitUntil(t)) => {
                    if now < t {
                        return None;
                    }
                    self.plan.pop_front();
                }
                Some(Step::WaitReady) => {
                    if self.ready_retry_at.is_some_and(|t| now < t) {
                        return None;
                    }
                    self.ready_retry_at = None;
                    self.plan.pop_front();
                    self.outstanding = Some(Step::WaitReady);
                    return Some(CwAction::Ready);
                }
                Some(step) => {
                    self.plan.pop_front();
                    // Counted as changed when the write is *issued*: a
                    // write that timed out may still have landed, and a
                    // restore of an unchanged setting costs nothing.
                    match step {
                        Step::SetPower(_) => self.snap.power_changed = true,
                        Step::SetBreakIn(_) => self.snap.break_in_changed = true,
                        _ => {}
                    }
                    let action = step.action();
                    self.outstanding = Some(step);
                    return action;
                }
            }
        }
        None
    }

    /// The radio's answer to the last action.
    pub fn complete(&mut self, outcome: CwOutcome, now: Instant) {
        let Some(step) = self.outstanding.take() else {
            return;
        };
        if let CwOutcome::Failed(error) = outcome {
            self.fail(error, now);
            return;
        }
        match (step, outcome) {
            (
                Step::Status,
                CwOutcome::Status {
                    cw_mode,
                    transmitting,
                    wpm,
                },
            ) => self.on_status(cw_mode, transmitting, wpm, now),
            (Step::SnapPower, CwOutcome::Power(p)) => self.snap.power = Some(p),
            (Step::VerifyPower(want), CwOutcome::Power(p)) => {
                if p != want {
                    self.fail(format!("power read back {p}, wanted {want}"), now);
                }
            }
            (Step::PowerAtMost(max), CwOutcome::Power(p)) => {
                if p > max {
                    self.begin_stop(
                        StopReason::Error,
                        Some(format!(
                            "power reads {p}, above the {max} it was clamped to"
                        )),
                        After::Restore(DisarmReason::PowerAboveClamp, None),
                        now,
                    );
                }
            }
            (Step::SnapBreakIn, CwOutcome::BreakIn(k)) => {
                self.snap.break_in = Some(k);
                let wanted = self.send_text.and_then(|s| s.requires_break_in);
                if wanted == Some(k) {
                    // Already as needed: nothing to set, nothing to undo.
                    self.plan
                        .retain(|s| !matches!(s, Step::SetBreakIn(_) | Step::VerifyBreakIn(_)));
                }
            }
            (Step::VerifyBreakIn(want), CwOutcome::BreakIn(k)) => {
                if k != want {
                    self.fail(format!("break-in read back {k:?}, wanted {want:?}"), now);
                }
            }
            (Step::SetPower(_) | Step::SetBreakIn(_) | Step::Stop, CwOutcome::Done) => {}
            (Step::WaitReady, CwOutcome::Ready(true)) => {}
            (Step::WaitReady, CwOutcome::Ready(false)) => {
                self.plan.push_front(Step::WaitReady);
                self.ready_retry_at = Some(now + self.config.tick);
            }
            (Step::Send(text, last), CwOutcome::Done) => self.on_sent(&text, last, now),
            (step, other) => self.fail(format!("unexpected answer {other:?} to {step:?}"), now),
        }
    }

    /// Everything that has happened since the last call.
    pub fn take_notices(&mut self) -> Vec<CwNotice> {
        std::mem::take(&mut self.notices)
    }

    // -----------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------

    /// Whether rigctl `T 1` must be refused: from the moment an arm is
    /// asked for until its restore has been verified.
    pub fn blocks_external_ptt(&self) -> bool {
        self.arm.is_some() || !matches!(self.activity, Activity::Idle)
    }

    /// Whether the pump should run on the CW tick rather than its poll
    /// interval.
    pub fn is_live(&self) -> bool {
        !matches!(self.activity, Activity::Idle)
            || !self.plan.is_empty()
            || self.outstanding.is_some()
    }

    /// Nothing armed, nothing to undo, nothing in flight.
    pub fn is_settled(&self) -> bool {
        self.arm.is_none() && !self.is_live()
    }

    /// The arm as `viewer` should see it.
    pub fn armed_state(&self, viewer: ConsoleId) -> Option<ArmedState> {
        self.arm.as_ref().map(|a| ArmedState {
            callsign: a.callsign.clone(),
            power_raw: a.power_raw,
            mine: a.owner == viewer,
        })
    }

    pub fn sender(&self) -> SenderState {
        match &self.activity {
            Activity::Sending(job) => SenderState::Sending {
                job: job.id,
                sent_chars: job.sent,
                total_chars: job.total,
            },
            Activity::Stopping { job, .. } => SenderState::Draining { job: *job },
            _ => self.last_sender.clone(),
        }
    }

    /// The alert a console must show while a restore is unverified.
    pub fn restore_status(&self) -> Option<RestoreStatus> {
        match &self.activity {
            Activity::Restoring {
                attempts,
                last_error: Some(e),
                ..
            } if *attempts > 0 => Some(RestoreStatus {
                attempts: *attempts,
                last_error: e.clone(),
            }),
            _ => None,
        }
    }

    // -----------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------

    fn ticket(&mut self) -> Ticket {
        let t = Ticket(self.next_ticket);
        self.next_ticket += 1;
        t
    }

    fn watchdog_for(&self, estimate: Duration) -> Duration {
        estimate + estimate / 5 + self.config.watchdog_margin
    }

    fn reply(&mut self, ticket: Ticket, result: Result<(), HostRefusal>) {
        self.notices.push(CwNotice::Reply { ticket, result });
    }

    fn check_timers(&mut self, now: Instant) {
        match &self.activity {
            Activity::Sending(job) if now >= job.deadline => {
                let message = format!(
                    "still keying {:.1} s after it started; the watchdog stopped it",
                    now.saturating_duration_since(job.started).as_secs_f64()
                );
                self.begin_stop(
                    StopReason::KeyDownCap,
                    Some(message),
                    After::Restore(DisarmReason::KeyDownCap, None),
                    now,
                );
            }
            Activity::Stopping { drain_deadline, .. } if now >= *drain_deadline => {
                // The radio never said it was idle. Stop waiting; the
                // restore still reads every write back.
                self.plan
                    .retain(|s| !matches!(s, Step::WaitReady | Step::WaitUntil(_)));
                self.ready_retry_at = None;
            }
            Activity::Idle => {
                if let Some(arm) = &self.arm {
                    if now.saturating_duration_since(arm.last_activity) >= self.config.idle_ttl {
                        self.start_restore(DisarmReason::IdleTimeout, None);
                    }
                }
            }
            _ => {}
        }
    }

    /// The plan has run out. Returns whether there may be more to do.
    fn finish(&mut self, now: Instant) -> bool {
        match std::mem::replace(&mut self.activity, Activity::Idle) {
            Activity::Idle => false,
            Activity::Arming {
                ticket,
                owner,
                callsign,
                cap,
                wpm,
            } => {
                let power_raw = self.gate.map_or(0, |g| g.low_power_raw);
                self.notices.push(CwNotice::Armed {
                    owner,
                    callsign: callsign.clone(),
                    power_raw,
                });
                self.arm = Some(Arm {
                    owner,
                    callsign,
                    cap,
                    power_raw,
                    wpm,
                    last_activity: now,
                    last_id: None,
                });
                self.reply(ticket, Ok(()));
                false
            }
            Activity::Sending(job) => {
                self.end_job(job.id, StopReason::Completed, None);
                if let Some(arm) = &mut self.arm {
                    arm.last_activity = now;
                }
                true
            }
            Activity::Stopping {
                job,
                reason,
                message,
                then,
                ..
            } => {
                self.end_job(job, reason, message);
                match then {
                    After::StayArmed => {
                        if let Some(arm) = &mut self.arm {
                            arm.last_activity = now;
                        }
                    }
                    After::Restore(reason, ticket) => self.start_restore(reason, ticket),
                }
                true
            }
            Activity::Restoring {
                reason,
                ticket,
                attempts,
                last_error,
                retry_at,
            } => {
                if let Some(at) = retry_at {
                    // Waiting to try a failed restore again.
                    let due = now >= at;
                    self.activity = Activity::Restoring {
                        reason,
                        ticket,
                        attempts,
                        last_error,
                        retry_at: if due { None } else { Some(at) },
                    };
                    if due {
                        self.plan_restore();
                    }
                    return due;
                }
                // Every write in the plan was read back and matched.
                self.snap = Snapshot::default();
                self.arm = None;
                self.notices.push(CwNotice::Restored { reason });
                if let Some(ticket) = ticket {
                    self.reply(ticket, Ok(()));
                }
                false
            }
        }
    }

    fn end_job(&mut self, job: u32, reason: StopReason, message: Option<String>) {
        self.notices.push(CwNotice::JobEnded {
            job,
            reason,
            message: message.clone(),
        });
        self.last_sender = SenderState::Stopped {
            job,
            reason,
            message,
        };
        self.last_chunk_end = None;
    }

    fn on_status(&mut self, cw_mode: bool, transmitting: bool, wpm: Option<u16>, now: Instant) {
        let wpm = wpm.unwrap_or(self.slowest_wpm).max(1);
        match &mut self.activity {
            Activity::Arming { wpm: w, .. } => {
                if !cw_mode {
                    self.refuse_arm(
                        ErrorCode::Forbidden,
                        "the radio is not in a CW mode; select CW on the radio first",
                    );
                } else if transmitting {
                    self.refuse_arm(ErrorCode::Forbidden, "the radio is transmitting");
                } else {
                    *w = wpm;
                }
            }
            Activity::Sending(job) => {
                let est = estimate(&job.text, wpm);
                let cap = self.arm.as_ref().map_or(Duration::ZERO, |a| a.cap);
                let problem = if !cw_mode {
                    Some("the radio left CW mode".to_string())
                } else if transmitting {
                    Some("the radio is already transmitting".to_string())
                } else if est > cap {
                    Some(format!(
                        "at the radio's {wpm} wpm this takes {:.1} s, over the {} s cap",
                        est.as_secs_f64(),
                        cap.as_secs()
                    ))
                } else {
                    None
                };
                if let Some(message) = problem {
                    self.begin_stop(StopReason::Error, Some(message), After::StayArmed, now);
                    return;
                }
                job.deadline = job.started + est + est / 5 + self.config.watchdog_margin;
                let (operator, id_text) = (job.operator.clone(), job.id_text.clone());
                if let Some(arm) = &mut self.arm {
                    arm.wpm = wpm;
                }
                self.plan_chunks(&operator, id_text, wpm);
            }
            _ => {}
        }
    }

    fn plan_chunks(&mut self, operator: &str, id_text: Option<String>, wpm: u16) {
        let Some(send) = self.send_text else {
            return;
        };
        let bound = (send.abort == CwAbort::DrainOnly).then_some(self.config.drain_bound);
        let mut pieces = chunks(operator, send.max_chunk_chars, bound, wpm);
        // Whole and alone: a split or merged ID is not a clean ID, and
        // `arm` already checked it fits one chunk.
        pieces.extend(id_text);
        let n = pieces.len();
        for (i, piece) in pieces.into_iter().enumerate() {
            if send.pacing == CwPacing::BufferFlag {
                self.plan.push_back(Step::WaitReady);
            }
            self.plan.push_back(Step::Send(piece, i + 1 == n));
        }
        if send.pacing == CwPacing::BufferFlag {
            // Done when the keyer is idle again.
            self.plan.push_back(Step::WaitReady);
        }
    }

    fn on_sent(&mut self, text: &str, last: bool, now: Instant) {
        let wpm = self.arm.as_ref().map_or(self.slowest_wpm, |a| a.wpm);
        let end = now + estimate(text, wpm);
        self.last_chunk_end = Some(self.last_chunk_end.map_or(end, |e| e.max(end)));
        if let Activity::Sending(job) = &mut self.activity {
            job.chunk_in_radio = true;
            job.sent = if last {
                job.total
            } else {
                (job.sent + text.chars().count() as u32 + 1).min(job.total)
            };
            if last && job.has_id {
                if let Some(arm) = &mut self.arm {
                    arm.last_id = Some(now);
                }
            }
            if self.send_text.map(|s| s.pacing) == Some(CwPacing::Estimated) {
                self.plan
                    .push_front(Step::WaitUntil(end + self.config.pacing_margin));
            }
        }
    }

    /// End the current job: the stop goes first, nothing more is sent.
    fn begin_stop(
        &mut self,
        reason: StopReason,
        message: Option<String>,
        then: After,
        now: Instant,
    ) {
        let job = match &self.activity {
            Activity::Sending(job) => job.clone(),
            _ => {
                if let After::Restore(r, t) = then {
                    self.start_restore(r, t);
                }
                return;
            }
        };
        // A chunk whose completion has not come back yet is in the radio.
        let in_flight = match &self.outstanding {
            Some(Step::Send(text, _)) => {
                let wpm = self.arm.as_ref().map_or(self.slowest_wpm, |a| a.wpm);
                Some(now + estimate(text, wpm))
            }
            _ => None,
        };
        let in_radio = job.chunk_in_radio || in_flight.is_some();
        self.plan.clear();
        self.ready_retry_at = None;
        // Always: cheap, and a radio that took a chunk we lost track of
        // is exactly the case it covers.
        self.plan.push_back(Step::Stop);
        let mut drain_deadline = now;
        if let (true, Some(send)) = (in_radio, self.send_text) {
            if send.abort == CwAbort::DrainOnly {
                let drain_end = [self.last_chunk_end, in_flight]
                    .into_iter()
                    .flatten()
                    .max()
                    .unwrap_or(now);
                match send.pacing {
                    CwPacing::BufferFlag => self.plan.push_back(Step::WaitReady),
                    CwPacing::Estimated => self
                        .plan
                        .push_back(Step::WaitUntil(drain_end + self.config.pacing_margin)),
                }
                let longest = drain_end
                    .saturating_duration_since(now)
                    .max(self.config.drain_bound);
                drain_deadline = now + longest + self.config.watchdog_margin;
            }
        }
        self.activity = Activity::Stopping {
            job: job.id,
            reason,
            message,
            then,
            drain_deadline,
        };
    }

    /// Stop any send, then restore. The common end of every arm.
    fn end_arm(
        &mut self,
        reason: DisarmReason,
        stop: StopReason,
        ticket: Option<Ticket>,
        now: Instant,
    ) {
        match &mut self.activity {
            Activity::Sending(_) => {
                self.begin_stop(stop, None, After::Restore(reason, ticket), now)
            }
            Activity::Stopping { then, .. } => match then {
                After::StayArmed => *then = After::Restore(reason, ticket),
                After::Restore(_, existing) => {
                    if existing.is_none() {
                        *existing = ticket;
                    }
                }
            },
            Activity::Idle => self.start_restore(reason, ticket),
            Activity::Arming { .. } | Activity::Restoring { .. } => {}
        }
    }

    fn refuse_arm(&mut self, code: ErrorCode, message: &str) {
        if let Activity::Arming { ticket, .. } = self.activity {
            self.reply(ticket, Err(refusal(code, message)));
        }
        self.plan.clear();
        self.activity = Activity::Idle;
        if self.snap.power_changed || self.snap.break_in_changed {
            self.start_restore(DisarmReason::ArmFailed, None);
        } else {
            self.snap = Snapshot::default();
        }
    }

    fn start_restore(&mut self, reason: DisarmReason, ticket: Option<Ticket>) {
        // Sends are over from here; the alert, not the arm, is what a
        // console sees until the read-back proves the radio is back.
        self.arm = None;
        self.ready_retry_at = None;
        self.activity = Activity::Restoring {
            reason,
            ticket,
            attempts: 0,
            last_error: None,
            retry_at: None,
        };
        self.plan_restore();
    }

    fn plan_restore(&mut self) {
        self.plan.clear();
        if self.snap.power_changed {
            if let Some(p) = self.snap.power {
                self.plan.push_back(Step::SetPower(p));
                self.plan.push_back(Step::VerifyPower(p));
            }
        }
        if self.snap.break_in_changed {
            if let Some(k) = self.snap.break_in {
                self.plan.push_back(Step::SetBreakIn(k));
                self.plan.push_back(Step::VerifyBreakIn(k));
            }
        }
    }

    fn fail(&mut self, error: String, now: Instant) {
        match &mut self.activity {
            Activity::Idle => {}
            Activity::Arming { .. } => self.refuse_arm(ErrorCode::OutOfRange, &error),
            Activity::Sending(_) => {
                self.begin_stop(StopReason::Error, Some(error), After::StayArmed, now)
            }
            Activity::Stopping { .. } => {
                // The stop or a drain poll failed. Wait out the longest
                // a chunk can take instead, then carry on.
                self.plan.clear();
                self.ready_retry_at = None;
                self.plan.push_back(Step::WaitUntil(
                    now + self.config.drain_bound + self.config.pacing_margin,
                ));
            }
            Activity::Restoring {
                ticket,
                attempts,
                last_error,
                retry_at,
                ..
            } => {
                *attempts += 1;
                *last_error = Some(error.clone());
                *retry_at = Some(now + self.config.restore_retry);
                let attempts = *attempts;
                let unanswered = ticket.take();
                self.plan.clear();
                self.notices.push(CwNotice::RestoreFailed {
                    attempts,
                    error: error.clone(),
                });
                if let Some(t) = unanswered {
                    self.reply(
                        t,
                        Err(refusal(
                            ErrorCode::OutOfRange,
                            &format!("restore failed ({error}); the server keeps retrying"),
                        )),
                    );
                }
            }
        }
    }
}

fn refusal(code: ErrorCode, message: &str) -> HostRefusal {
    HostRefusal {
        code,
        message: message.to_string(),
    }
}

/// How long `text` takes to key at `wpm`, PARIS timing, no trailing gap.
///
/// Text the Morse codec cannot tokenize is costed at 22 units a
/// character -- longer than any real character plus its gap -- so an
/// unknown character can only make an estimate pessimistic.
pub fn estimate(text: &str, wpm: u16) -> Duration {
    let wpm = f64::from(wpm.clamp(3, 100));
    let speed = Speed::wpm(wpm).expect("clamped into cat-morse's range");
    match timing::key_text(text.trim(), speed, &Fist::perfect(), 0) {
        Ok(keying) => keying.total(),
        Err(_) => speed.dot() * (22 * text.chars().count() as u32),
    }
}

/// Split operator `text` into chunks of at most `max_chars` that each key
/// within `max_duration` (the drain bound).
///
/// Word boundaries are preferred, but the bound wins: abort latency on a
/// `DrainOnly` radio is the safety property, and a 24-character string
/// with no spaces would otherwise sit in the radio for ~25 s at 10 wpm. A
/// single character always goes, even one longer than the bound -- there
/// is nothing smaller to send.
///
/// The server's own `DE <call>` never comes through here: it is its own
/// chunk (see [`CwSupervisor::send`]).
pub fn chunks(text: &str, max_chars: u16, max_duration: Option<Duration>, wpm: u16) -> Vec<String> {
    let max = usize::from(max_chars.max(1));
    let fits =
        |s: &str| s.chars().count() <= max && max_duration.map_or(true, |d| estimate(s, wpm) <= d);

    let mut out = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() {
            let joined = format!("{current} {word}");
            if fits(&joined) {
                current = joined;
                continue;
            }
            out.push(std::mem::take(&mut current));
        }
        if fits(word) {
            current = word.to_string();
            continue;
        }
        let mut piece = String::new();
        for c in word.chars() {
            let mut longer = piece.clone();
            longer.push(c);
            if piece.is_empty() || fits(&longer) {
                piece = longer;
            } else {
                out.push(std::mem::replace(&mut piece, c.to_string()));
            }
        }
        current = piece;
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
#[path = "cw_tests.rs"]
mod tests;
