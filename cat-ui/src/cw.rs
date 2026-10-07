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

//! What a console shows about CW (ADR 0023 decision 12), for both
//! renderers alike (ADR 0013).
//!
//! Text and a tone per line, and nothing about how either is drawn. The
//! one thing a console must never get wrong is the failed-restore alert:
//! a radio whose power or break-in the server could not put back is not
//! the radio the operator left, and [`CwPanel::status`] puts that ahead of
//! everything else the status strip could say.

use cat_native::{CwCapabilityWire, CwState, SenderState, StopReason};

/// How a line should read, which each renderer turns into its own colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tone {
    /// Nothing happening.
    Quiet,
    Normal,
    /// This console is armed or keying.
    Active,
    /// Someone else holds the arm, or a send ended badly.
    Warning,
    /// The radio is not as the operator left it.
    Alarm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CwLine {
    pub label: &'static str,
    pub text: String,
    pub tone: Tone,
}

/// Everything a CW panel draws.
#[derive(Debug, Clone, PartialEq)]
pub struct CwPanel {
    /// Pitch, keyer speed and break-in.
    pub settings: CwLine,
    /// Who holds the transmit arm.
    pub arm: CwLine,
    /// What the sender is doing.
    pub sender: CwLine,
    /// How far through the current send, 0.0-1.0, while sending.
    pub progress: Option<f32>,
    /// A send is keying or draining right now.
    pub live: bool,
    /// A failed restore, worded to be read once and acted on.
    pub alert: Option<String>,
}

impl CwPanel {
    /// The panel for a server that reported CW. `cap` supplies the power
    /// label; without it the raw value is shown.
    pub fn new(state: &CwState, cap: Option<&CwCapabilityWire>) -> Self {
        let settings = CwLine {
            label: "CW",
            text: format!(
                "{} · {} · break-in {}",
                state
                    .pitch_hz
                    .map_or("— Hz".to_string(), |hz| format!("{hz} Hz")),
                state
                    .keyer_wpm
                    .map_or("— wpm".to_string(), |w| format!("{w} wpm")),
                state
                    .break_in
                    .map_or("—".to_string(), |b| format!("{b:?}").to_lowercase()),
            ),
            tone: Tone::Normal,
        };

        let arm = match &state.armed {
            None => CwLine {
                label: "TX",
                text: "not armed".to_string(),
                tone: Tone::Quiet,
            },
            Some(armed) => {
                let power = match cap.and_then(|c| c.tx_gate.as_ref()) {
                    Some(gate) if gate.low_power_raw == armed.power_raw => {
                        gate.low_power_label.clone()
                    }
                    _ => format!("power {}", armed.power_raw),
                };
                if armed.mine {
                    CwLine {
                        label: "TX",
                        text: format!("ARMED by this console · {} · {power}", armed.callsign),
                        tone: Tone::Active,
                    }
                } else {
                    CwLine {
                        label: "TX",
                        text: format!("armed by another console · {} · {power}", armed.callsign),
                        tone: Tone::Warning,
                    }
                }
            }
        };

        let live = matches!(
            state.sender,
            SenderState::Sending { .. } | SenderState::Draining { .. }
        );
        let (sender, progress) = match &state.sender {
            SenderState::Idle => (
                CwLine {
                    label: "SEND",
                    text: "idle".to_string(),
                    tone: Tone::Quiet,
                },
                None,
            ),
            SenderState::Sending {
                job,
                sent_chars,
                total_chars,
            } => {
                let fraction = if *total_chars == 0 {
                    0.0
                } else {
                    (*sent_chars as f32 / *total_chars as f32).clamp(0.0, 1.0)
                };
                (
                    CwLine {
                        label: "SEND",
                        text: format!("sending #{job} · {sent_chars}/{total_chars}"),
                        tone: Tone::Active,
                    },
                    Some(fraction),
                )
            }
            SenderState::Draining { job } => (
                CwLine {
                    label: "SEND",
                    text: format!("stopping #{job} · the radio is finishing its buffer"),
                    tone: Tone::Warning,
                },
                None,
            ),
            SenderState::Stopped {
                job,
                reason,
                message,
            } => {
                let (what, tone) = match reason {
                    StopReason::Completed => ("sent", Tone::Normal),
                    StopReason::Aborted => ("aborted", Tone::Normal),
                    StopReason::KeyDownCap => ("stopped by the key-down cap", Tone::Warning),
                    StopReason::Disconnected => {
                        ("stopped: its console disconnected", Tone::Warning)
                    }
                    StopReason::Error => ("stopped by an error", Tone::Warning),
                };
                let text = match message {
                    Some(m) => format!("#{job} {what}: {m}"),
                    None => format!("#{job} {what}"),
                };
                (
                    CwLine {
                        label: "SEND",
                        text,
                        tone,
                    },
                    None,
                )
            }
        };

        let alert = state.restore.as_ref().map(|r| {
            format!(
                "CW RESTORE FAILED ({} {}): {} — power or break-in is NOT back as \
                 it was; check the radio before transmitting. The server keeps retrying.",
                r.attempts,
                if r.attempts == 1 {
                    "attempt"
                } else {
                    "attempts"
                },
                r.last_error
            )
        });

        CwPanel {
            settings,
            arm,
            sender,
            progress,
            live,
            alert,
        }
    }

    /// The one line the status strip should show for CW, if any, by
    /// priority: a failed restore, then a send in progress, then an arm.
    /// `None` when CW has nothing to say, so the strip keeps its own text.
    pub fn status(&self) -> Option<(String, Tone)> {
        if let Some(alert) = &self.alert {
            return Some((alert.clone(), Tone::Alarm));
        }
        if self.live {
            return Some((format!("CW {}", self.sender.text), self.sender.tone));
        }
        if self.arm.tone != Tone::Quiet {
            return Some((format!("CW {}", self.arm.text), self.arm.tone));
        }
        None
    }

    /// The lines in drawing order.
    pub fn lines(&self) -> [&CwLine; 3] {
        [&self.settings, &self.arm, &self.sender]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_native::{ArmedState, BreakInKind, CwPacing, RestoreStatus, TxGateWire};

    fn cap() -> CwCapabilityWire {
        CwCapabilityWire {
            pitch: None,
            pitch_writable: false,
            keyer_wpm: None,
            keyer_wpm_writable: false,
            break_in: vec![BreakInKind::Off, BreakInKind::Semi],
            send_text: Some(cat_native::CwSendTextWire {
                max_chunk_chars: 24,
                charset: "ABC".into(),
                pacing: CwPacing::BufferFlag,
                abort: cat_native::CwAbort::DrainOnly,
                requires_break_in: None,
            }),
            tx_gate: Some(TxGateWire {
                low_power_raw: 5,
                low_power_label: "5 W".into(),
                power_raw: cat_native::RawRange::new(5, 100),
            }),
        }
    }

    fn state() -> CwState {
        CwState {
            pitch_hz: Some(600),
            keyer_wpm: Some(20),
            break_in: Some(BreakInKind::Semi),
            armed: None,
            sender: SenderState::Idle,
            restore: None,
        }
    }

    fn armed(mine: bool) -> Option<ArmedState> {
        Some(ArmedState {
            callsign: "N0CALL".into(),
            power_raw: 5,
            mine,
        })
    }

    #[test]
    fn settings_read_as_an_operator_says_them() {
        let p = CwPanel::new(&state(), Some(&cap()));
        assert_eq!(p.settings.text, "600 Hz · 20 wpm · break-in semi");
        let unknown = CwPanel::new(
            &CwState {
                pitch_hz: None,
                keyer_wpm: None,
                break_in: None,
                ..state()
            },
            None,
        );
        assert_eq!(unknown.settings.text, "— Hz · — wpm · break-in —");
    }

    #[test]
    fn an_idle_unarmed_radio_has_nothing_for_the_status_strip() {
        let p = CwPanel::new(&state(), Some(&cap()));
        assert_eq!(p.arm.tone, Tone::Quiet);
        assert_eq!(p.status(), None);
    }

    #[test]
    fn the_arm_says_whose_it_is_and_at_what_power() {
        let mine = CwPanel::new(
            &CwState {
                armed: armed(true),
                ..state()
            },
            Some(&cap()),
        );
        assert_eq!(mine.arm.text, "ARMED by this console · N0CALL · 5 W");
        assert_eq!(mine.arm.tone, Tone::Active);
        assert_eq!(mine.status().unwrap().1, Tone::Active);

        let theirs = CwPanel::new(
            &CwState {
                armed: armed(false),
                ..state()
            },
            Some(&cap()),
        );
        assert!(theirs.arm.text.starts_with("armed by another console"));
        assert_eq!(theirs.arm.tone, Tone::Warning);

        // Without the capability's label, the raw value -- never a guess.
        let raw = CwPanel::new(
            &CwState {
                armed: armed(true),
                ..state()
            },
            None,
        );
        assert!(raw.arm.text.ends_with("power 5"), "{}", raw.arm.text);
    }

    #[test]
    fn a_send_in_progress_shows_how_far_it_has_got() {
        let p = CwPanel::new(
            &CwState {
                armed: armed(true),
                sender: SenderState::Sending {
                    job: 3,
                    sent_chars: 5,
                    total_chars: 20,
                },
                ..state()
            },
            Some(&cap()),
        );
        assert_eq!(p.sender.text, "sending #3 · 5/20");
        assert_eq!(p.progress, Some(0.25));
        // Sending outranks armed in the strip.
        assert_eq!(p.status().unwrap().0, "CW sending #3 · 5/20");
    }

    #[test]
    fn every_ending_is_named() {
        for (reason, word, tone) in [
            (StopReason::Completed, "sent", Tone::Normal),
            (StopReason::Aborted, "aborted", Tone::Normal),
            (StopReason::KeyDownCap, "key-down cap", Tone::Warning),
            (StopReason::Disconnected, "disconnected", Tone::Warning),
            (StopReason::Error, "error", Tone::Warning),
        ] {
            let p = CwPanel::new(
                &CwState {
                    sender: SenderState::Stopped {
                        job: 4,
                        reason,
                        message: Some("why".into()),
                    },
                    ..state()
                },
                None,
            );
            assert!(p.sender.text.contains(word), "{:?}", p.sender.text);
            assert!(p.sender.text.ends_with(": why"));
            assert_eq!(p.sender.tone, tone);
        }
        let draining = CwPanel::new(
            &CwState {
                sender: SenderState::Draining { job: 4 },
                ..state()
            },
            None,
        );
        assert!(draining.sender.text.starts_with("stopping #4"));
        assert!(draining.status().is_some(), "a drain is worth the strip");
    }

    #[test]
    fn a_failed_restore_outranks_everything() {
        let p = CwPanel::new(
            &CwState {
                armed: armed(true),
                sender: SenderState::Sending {
                    job: 1,
                    sent_chars: 1,
                    total_chars: 2,
                },
                restore: Some(RestoreStatus {
                    attempts: 2,
                    last_error: "power read back 5, wanted 100".into(),
                }),
                ..state()
            },
            Some(&cap()),
        );
        let (text, tone) = p.status().unwrap();
        assert_eq!(tone, Tone::Alarm);
        assert!(text.starts_with("CW RESTORE FAILED (2 attempts)"), "{text}");
        assert!(text.contains("power read back 5, wanted 100"));
        assert!(text.contains("check the radio before transmitting"));
    }
}
