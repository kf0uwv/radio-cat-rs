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

//! The `:` command line.
//!
//! The complete path to every capability, and the reason option 3 can hold
//! ADR 0013's parity: anything reachable here is reachable identically in
//! the TUI, because it is the same grammar over the same commands.
//!
//! Discoverability is the quick bar's job, not this one's. These two are
//! deliberately redundant.
//!
//! # Frequencies are parsed the way an operator says them
//!
//! `:f 14.074` is 14.074 MHz, `:f 14074000` is the same frequency in Hz.
//! Guessing by magnitude is the sort of cleverness that eventually tunes
//! somebody to 14 Hz, so the rule is explicit: a decimal point means MHz,
//! a bare integer means Hz.

use cat_native::{CapabilitiesWire, Command};

/// What a typed line asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Send this to the radio.
    Radio(Command),
    /// Switch workspace by 1-based index, as the digit keys do.
    SelectTab(usize),
    Quit,
}

/// Why a line was not understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    UnknownVerb(String),
    /// The verb was right but the argument was not a number.
    BadArgument {
        verb: &'static str,
        got: String,
    },
    MissingArgument(&'static str),
    /// The radio does not have what was asked for. Named separately from a
    /// syntax error because the fix is different: the operator did not
    /// mistype, this radio simply cannot.
    Unsupported(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Empty => write!(f, "nothing to do"),
            ParseError::UnknownVerb(v) => write!(f, "unknown command: {v}"),
            ParseError::BadArgument { verb, got } => {
                write!(f, "{verb}: {got:?} is not a number")
            }
            ParseError::MissingArgument(v) => write!(f, "{v}: needs a value"),
            ParseError::Unsupported(what) => write!(f, "this radio has no {what}"),
        }
    }
}

/// Parse one command line against what the radio can do.
///
/// Taking capabilities is the point: `:mode c4fm` on a TS-570D is refused
/// here, with a message saying the radio has no such mode, rather than
/// travelling to the server to be refused there. Same answer, but the
/// operator gets it while still looking at what they typed.
pub fn parse(line: &str, caps: &CapabilitiesWire) -> Result<Action, ParseError> {
    let line = line.trim();
    let mut parts = line.split_whitespace();
    let Some(verb) = parts.next() else {
        return Err(ParseError::Empty);
    };
    let rest: Vec<&str> = parts.collect();
    let arg = rest.first().copied();

    match verb.to_ascii_lowercase().as_str() {
        "q" | "quit" => Ok(Action::Quit),

        "f" | "freq" => {
            let arg = arg.ok_or(ParseError::MissingArgument("freq"))?;
            let hz = parse_frequency(arg).ok_or_else(|| ParseError::BadArgument {
                verb: "freq",
                got: arg.to_string(),
            })?;
            Ok(Action::Radio(Command::SetFrequency { vfo: 0, hz }))
        }

        // Retune moves the dial *and* any IF-tap source with it. Distinct
        // from `freq` because on a radio with a tap they are not the same
        // gesture, and collapsing them would hide that.
        "t" | "tune" => {
            let arg = arg.ok_or(ParseError::MissingArgument("tune"))?;
            let hz = parse_frequency(arg).ok_or_else(|| ParseError::BadArgument {
                verb: "tune",
                got: arg.to_string(),
            })?;
            Ok(Action::Radio(Command::Retune { hz }))
        }

        "m" | "mode" => {
            let arg = arg.ok_or(ParseError::MissingArgument("mode"))?;
            let wanted = arg.to_ascii_uppercase();
            let mode = caps
                .modes
                .iter()
                .find(|m| m.label.eq_ignore_ascii_case(&wanted))
                .map(|m| m.id)
                .ok_or_else(|| ParseError::Unsupported(format!("{wanted} mode")))?;
            Ok(Action::Radio(Command::SetMode { mode }))
        }

        "mem" | "memory" => {
            let memory = caps
                .memory
                .ok_or_else(|| ParseError::Unsupported("memory".to_string()))?;
            let arg = arg.ok_or(ParseError::MissingArgument("memory"))?;
            let channel: u16 = arg.parse().map_err(|_| ParseError::BadArgument {
                verb: "memory",
                got: arg.to_string(),
            })?;
            if channel < memory.channels.min || channel > memory.channels.max {
                return Err(ParseError::Unsupported(format!(
                    "memory channel {channel} (it has {}–{})",
                    memory.channels.min, memory.channels.max
                )));
            }
            Ok(Action::Radio(Command::SetMemoryChannel { channel }))
        }

        "shift" => {
            let limit = caps
                .filters
                .if_shift_hz
                .ok_or_else(|| ParseError::Unsupported("IF shift".to_string()))?;
            let arg = arg.ok_or(ParseError::MissingArgument("shift"))?;
            let hz: i32 = arg.parse().map_err(|_| ParseError::BadArgument {
                verb: "shift",
                got: arg.to_string(),
            })?;
            Ok(Action::Radio(Command::SetIfShift {
                hz: crate::quick::clamp_shift(hz, limit),
            }))
        }

        "split" => {
            if !caps.vfos.split {
                return Err(ParseError::Unsupported("split".to_string()));
            }
            let enabled = !matches!(arg, Some("off") | Some("0"));
            Ok(Action::Radio(Command::SetSplit { enabled }))
        }

        digits if digits.chars().all(|c| c.is_ascii_digit()) && !digits.is_empty() => {
            Ok(Action::SelectTab(digits.parse().unwrap_or(0)))
        }

        other => Err(ParseError::UnknownVerb(other.to_string())),
    }
}

/// `14.074` is MHz; `14074000` is Hz. Never guessed by magnitude.
fn parse_frequency(text: &str) -> Option<u64> {
    let text = text.replace('_', "");
    if text.contains('.') {
        let mhz: f64 = text.parse().ok()?;
        if !mhz.is_finite() || mhz < 0.0 {
            return None;
        }
        Some((mhz * 1_000_000.0).round() as u64)
    } else {
        text.parse().ok()
    }
}

/// Every verb, for the hint line. Kept beside `parse` so the two cannot
/// drift into advertising something that does not work.
pub const VERBS: &[(&str, &str)] = &[
    ("f <mhz|hz>", "set VFO A frequency"),
    (
        "t <mhz|hz>",
        "retune, moving any IF-tap source with the dial",
    ),
    ("m <mode>", "set mode"),
    ("mem <n>", "recall memory channel"),
    ("shift <hz>", "set IF shift"),
    ("split [off]", "split on or off"),
    ("<n>", "switch to workspace n"),
    ("q", "quit"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::{bare as caps_bare, full as caps_ts570d};
    use cat_native::ModeId;

    fn parse_ts(line: &str) -> Result<Action, ParseError> {
        parse(line, &caps_ts570d())
    }

    #[test]
    fn a_decimal_frequency_is_megahertz_and_an_integer_is_hertz() {
        // Never guessed by magnitude. Guessing is how somebody eventually
        // gets tuned to 14 Hz.
        assert_eq!(
            parse_ts("f 14.074"),
            Ok(Action::Radio(Command::SetFrequency {
                vfo: 0,
                hz: 14_074_000
            }))
        );
        assert_eq!(
            parse_ts("f 14074000"),
            Ok(Action::Radio(Command::SetFrequency {
                vfo: 0,
                hz: 14_074_000
            }))
        );
    }

    #[test]
    fn retune_is_a_different_verb_from_setting_a_frequency() {
        // On a radio with an IF tap these are not the same gesture: one
        // moves the dial and the spectrum source with it. Collapsing them
        // would hide that.
        assert_eq!(
            parse_ts("t 14.074"),
            Ok(Action::Radio(Command::Retune { hz: 14_074_000 }))
        );
    }

    #[test]
    fn a_mode_this_radio_does_not_have_is_refused_before_it_reaches_the_wire() {
        // Same answer the server would give, but the operator gets it
        // while still looking at what they typed.
        let err = parse_ts("m c4fm").unwrap_err();
        assert_eq!(err, ParseError::Unsupported("C4FM mode".to_string()));
        assert!(err.to_string().contains("no C4FM mode"));
    }

    #[test]
    fn a_mode_it_does_have_is_accepted_in_any_case() {
        assert_eq!(
            parse_ts("m usb"),
            Ok(Action::Radio(Command::SetMode { mode: ModeId::Usb }))
        );
        assert_eq!(parse_ts("m USB"), parse_ts("mode Usb"));
    }

    #[test]
    fn a_memory_channel_outside_the_radios_numbering_is_refused_with_the_range() {
        // "0-99" is the useful half of the message.
        let err = parse_ts("mem 200").unwrap_err();
        assert!(err.to_string().contains("0–99"), "got {err}");
    }

    #[test]
    fn a_capability_the_radio_lacks_is_refused_as_unsupported_not_as_a_typo() {
        // The fix is different, so the error must be. The operator did not
        // mistype; this radio simply cannot.
        let bare = caps_bare();
        assert_eq!(
            parse("mem 3", &bare),
            Err(ParseError::Unsupported("memory".to_string()))
        );
        assert_eq!(
            parse("shift 200", &bare),
            Err(ParseError::Unsupported("IF shift".to_string()))
        );
        assert_eq!(
            parse("split", &bare),
            Err(ParseError::Unsupported("split".to_string()))
        );
    }

    #[test]
    fn shift_is_clamped_to_the_radios_limit_rather_than_refused() {
        // Distinct from the memory case on purpose: a too-large shift has
        // an obvious intent, and the radio's own limit is the answer.
        assert_eq!(
            parse_ts("shift 99999"),
            Ok(Action::Radio(Command::SetIfShift { hz: 1_000 }))
        );
    }

    #[test]
    fn split_defaults_to_on_and_takes_off_explicitly() {
        assert_eq!(
            parse_ts("split"),
            Ok(Action::Radio(Command::SetSplit { enabled: true }))
        );
        assert_eq!(
            parse_ts("split off"),
            Ok(Action::Radio(Command::SetSplit { enabled: false }))
        );
        assert_eq!(parse_ts("split 0"), parse_ts("split off"));
    }

    #[test]
    fn a_bare_number_selects_a_workspace_like_the_digit_keys_do() {
        assert_eq!(parse_ts("2"), Ok(Action::SelectTab(2)));
    }

    #[test]
    fn a_verb_with_no_argument_says_what_it_wanted() {
        assert_eq!(parse_ts("f"), Err(ParseError::MissingArgument("freq")));
        assert!(parse_ts("m")
            .unwrap_err()
            .to_string()
            .contains("needs a value"));
    }

    #[test]
    fn an_argument_that_is_not_a_number_says_so_with_what_was_typed() {
        let err = parse_ts("f banana").unwrap_err();
        assert!(err.to_string().contains("banana"), "got {err}");
    }

    #[test]
    fn nonsense_is_an_unknown_verb_and_not_a_panic() {
        assert!(matches!(
            parse_ts("wibble"),
            Err(ParseError::UnknownVerb(_))
        ));
        assert_eq!(parse_ts("   "), Err(ParseError::Empty));
        assert_eq!(parse_ts(""), Err(ParseError::Empty));
    }

    #[test]
    fn a_negative_or_absurd_frequency_is_rejected_rather_than_wrapped() {
        // `as u64` on a negative float saturates to 0 rather than
        // wrapping, but relying on that silently is how a console tunes to
        // DC. It is rejected explicitly.
        assert!(parse_ts("f -1.5").is_err());
        assert!(parse_ts("f nan").is_err());
    }

    #[test]
    fn every_advertised_verb_actually_parses() {
        // The hint line and the parser drift apart otherwise, and the
        // console ends up advertising something that does nothing.
        let caps = caps_ts570d();
        for (usage, _) in VERBS {
            let mut words = usage.split_whitespace();
            let verb = words.next().unwrap();
            // The sample argument is derived from the usage string's own
            // placeholder rather than kept in a parallel list, so the two
            // cannot drift into disagreeing about what a verb takes.
            let argument = match words.next() {
                None => String::new(),
                Some("<mhz|hz>") => "14.074".to_string(),
                Some("<mode>") => caps.modes[0].label.clone(),
                Some("<n>") => "1".to_string(),
                Some("<hz>") => "200".to_string(),
                Some("[off]") => String::new(),
                Some(other) => panic!("unrecognised placeholder {other:?} in {usage:?}"),
            };
            let line = if verb == "<n>" {
                "1".to_string()
            } else {
                format!("{verb} {argument}")
            };
            let result = parse(line.trim(), &caps);
            assert!(
                result.is_ok(),
                "advertised {usage:?} but {line:?} gave {result:?}"
            );
        }
    }
}
