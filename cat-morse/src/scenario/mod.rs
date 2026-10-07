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

//! Seeded scripts of realistic on-air contacts.
//!
//! [`generate`] turns a [`ScenarioConfig`] — the kind of contact, the
//! operator's own callsign, a [`Difficulty`] and a seed — into a
//! [`Script`]: who the other stations are and every transmission in
//! order, each tagged as sent by one of *them* or by *you*. A *them*
//! line comes with that station's [`SignalProfile`] (pitch offset, speed,
//! fist and strength) so a caller can key and render it; a *you* line is
//! what the operator would send, for sending practice.
//!
//! The same config gives the same script on every platform: generation
//! uses only the in-crate [`Rng`](crate::rng::Rng) and integer or basic
//! float arithmetic. Every transmission's text encodes.
//!
//! | Kind | What happens |
//! |------|--------------|
//! | [`ScenarioKind::RagChew`] | a CQ is answered; RST, name, QTH, rig, WX; 73 |
//! | [`ScenarioKind::Contest`] | search and pounce: `CQ TEST`, `5NN` + zone or serial |
//! | [`ScenarioKind::Pota`] | a park activator (`US-1234`), state exchange, optional P2P |
//! | [`ScenarioKind::Sota`] | a summit activator (`W0C/FR-001`), optional S2S |
//! | [`ScenarioKind::DxPileup`] | you run the DX; callers overlap at different offsets |
//!
//! [`band_channel`] wraps an over's signals in the band
//! [`Difficulty::band`] describes: noise, fading, static and QRM.
//!
//! Any kind may include repeat requests (`QRZ?`, `AGN?`, `PSE RPT`) at
//! the rate [`Difficulty::repeat_requests`] sets.

mod band;
mod callsign;
mod data;
mod qso;
mod world;

use std::time::Duration;

pub use band::{band_channel, BandConditions, BandReport, QrmOdds, QrmReport, QrnOdds, QsbOdds};
pub use callsign::Callsign;

use crate::codec;
use crate::rng::Rng;
use crate::timing::{Fist, Speed, TimingError};

/// A scenario that cannot be generated.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ScenarioError {
    /// Not a usable callsign.
    #[error("invalid callsign {0:?}")]
    InvalidCallsign(String),
    /// A [`Difficulty`] or kind parameter out of range.
    #[error("invalid scenario setting: {0}")]
    InvalidSetting(&'static str),
    /// A generated line did not encode — a bug in the generator.
    #[error("generated text {text:?} does not encode: {source}")]
    Unencodable {
        text: String,
        source: codec::CodecError,
    },
}

impl From<TimingError> for ScenarioError {
    fn from(_: TimingError) -> Self {
        ScenarioError::InvalidSetting("speed or fist out of range")
    }
}

/// The number sent after `5NN` in a contest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ContestExchange {
    /// CQ zone (CQ WW style).
    CqZone,
    /// Serial number.
    Serial,
}

/// What kind of contact to script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ScenarioKind {
    /// A conversational QSO.
    RagChew,
    /// A run of quick contest exchanges.
    Contest {
        /// Zone or serial.
        exchange: ContestExchange,
        /// Send `T` for 0 and `N` for 9 in the exchange, as contesters do.
        cut_numbers: bool,
    },
    /// Hunting a Parks on the Air activator.
    Pota {
        /// You are in a park too.
        park_to_park: bool,
    },
    /// Chasing a Summits on the Air activator.
    Sota {
        /// You are on a summit too.
        summit_to_summit: bool,
    },
    /// Running a pileup as the DX station.
    DxPileup {
        /// How many stations call (1–12).
        callers: u8,
    },
}

/// How hard the other stations are to copy.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Difficulty {
    /// Character speeds are drawn from this range (whole wpm, 5–60).
    pub wpm: (u32, u32),
    /// If set and slower than the drawn character speed, Farnsworth
    /// spacing at this effective speed.
    pub farnsworth_effective: Option<f64>,
    /// Fists the stations are drawn from (non-empty).
    pub fists: Vec<Fist>,
    /// Signal strength range in dB, relative to the synth reference.
    pub strength_db: (f32, f32),
    /// Pitch offset range in Hz, relative to the operator's pitch.
    pub pitch_offset_hz: (f32, f32),
    /// Probability (0–1) of a repeat request at each opportunity.
    pub repeat_requests: f64,
    /// The band the stations are copied through (see [`band_channel`]).
    pub band: BandConditions,
}

impl Default for Difficulty {
    fn default() -> Self {
        Difficulty::intermediate()
    }
}

impl Difficulty {
    /// Slow, clean, Farnsworth-spaced, few repeats.
    pub fn beginner() -> Self {
        Difficulty {
            wpm: (15, 18),
            farnsworth_effective: Some(10.0),
            fists: vec![Fist::perfect(), Fist::good()],
            strength_db: (-3.0, 0.0),
            pitch_offset_hz: (-30.0, 30.0),
            repeat_requests: 0.05,
            band: BandConditions::beginner(),
        }
    }

    /// Ordinary band conditions.
    pub fn intermediate() -> Self {
        Difficulty {
            wpm: (18, 25),
            farnsworth_effective: None,
            fists: vec![Fist::good(), Fist::typical()],
            strength_db: (-12.0, 0.0),
            pitch_offset_hz: (-100.0, 100.0),
            repeat_requests: 0.15,
            band: BandConditions::intermediate(),
        }
    }

    /// Fast, varied fists, weak signals spread across the passband.
    pub fn advanced() -> Self {
        Difficulty {
            wpm: (25, 35),
            farnsworth_effective: None,
            fists: vec![Fist::good(), Fist::typical(), Fist::sloppy()],
            strength_db: (-20.0, 0.0),
            pitch_offset_hz: (-250.0, 250.0),
            repeat_requests: 0.25,
            band: BandConditions::advanced(),
        }
    }

    fn validate(&self) -> Result<(), ScenarioError> {
        let (lo, hi) = self.wpm;
        if lo < 5 || hi > 60 || lo > hi {
            return Err(ScenarioError::InvalidSetting(
                "wpm range must be within 5..=60",
            ));
        }
        if let Some(e) = self.farnsworth_effective {
            if !(e.is_finite() && e >= 3.0) {
                return Err(ScenarioError::InvalidSetting(
                    "Farnsworth speed must be ≥ 3 wpm",
                ));
            }
        }
        if self.fists.is_empty() {
            return Err(ScenarioError::InvalidSetting("at least one fist is needed"));
        }
        for f in &self.fists {
            f.validate()?;
        }
        let range_ok = |(a, b): (f32, f32)| a.is_finite() && b.is_finite() && a <= b;
        if !range_ok(self.strength_db) || !range_ok(self.pitch_offset_hz) {
            return Err(ScenarioError::InvalidSetting(
                "ranges must be finite with lo ≤ hi",
            ));
        }
        if !(0.0..=1.0).contains(&self.repeat_requests) {
            return Err(ScenarioError::InvalidSetting(
                "repeat_requests must be in [0, 1]",
            ));
        }
        self.band.validate()
    }
}

/// Everything needed to generate a script.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScenarioConfig {
    /// The kind of contact.
    pub kind: ScenarioKind,
    /// The operator's own callsign.
    pub my_call: Callsign,
    /// How hard.
    pub difficulty: Difficulty,
    /// The seed: same seed, same script.
    pub seed: u64,
}

/// How a station sounds.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SignalProfile {
    /// Offset from the operator's chosen pitch, Hz.
    pub pitch_offset_hz: f32,
    /// Sending speed.
    pub speed: Speed,
    /// Fist.
    pub fist: Fist,
    /// Seed for this station's fist (see [`crate::timing::key`]).
    pub fist_seed: u64,
    /// Strength in dB relative to the synth reference.
    pub strength_db: f32,
}

/// Who an operator is.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Persona {
    /// Callsign as sent (may carry `/P`).
    pub call: Callsign,
    /// First name.
    pub name: String,
    /// Town.
    pub qth: String,
    /// State/province code (`CO`) or country name.
    pub region: String,
    /// CQ zone.
    pub zone: u8,
    /// Radio.
    pub rig: String,
    /// Power in watts.
    pub power_w: u32,
    /// Antenna.
    pub antenna: String,
    /// Park or summit reference, if activating.
    pub reference: Option<String>,
}

/// Another station in the script.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Station {
    /// Who.
    pub persona: Persona,
    /// How they sound.
    pub signal: SignalProfile,
}

/// Who sends a transmission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Party {
    /// The station at this index in [`Script::stations`].
    Them(usize),
    /// The operator.
    You,
}

/// When a transmission starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Start {
    /// This long after the previous transmission ends.
    AfterPrevious {
        /// The pause.
        gap: Duration,
    },
    /// This long after the previous transmission *starts*: it overlaps.
    WithPrevious {
        /// The stagger.
        delay: Duration,
    },
}

/// One transmission.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Transmission {
    /// Who sends it.
    pub from: Party,
    /// What is sent; always encodes.
    pub text: String,
    /// When.
    pub start: Start,
}

/// A generated contact.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Script {
    /// The kind it was generated as.
    pub kind: ScenarioKind,
    /// The operator.
    pub me: Persona,
    /// Everyone else.
    pub stations: Vec<Station>,
    /// Every transmission, in order.
    pub transmissions: Vec<Transmission>,
}

impl Script {
    /// The signal profile for a *them* transmission; `None` for *you*.
    pub fn signal_for(&self, t: &Transmission) -> Option<&SignalProfile> {
        match t.from {
            Party::Them(i) => self.stations.get(i).map(|s| &s.signal),
            Party::You => None,
        }
    }
}

/// Generates the script for `cfg`.
pub fn generate(cfg: &ScenarioConfig) -> Result<Script, ScenarioError> {
    cfg.difficulty.validate()?;
    if let ScenarioKind::DxPileup { callers } = cfg.kind {
        if !(1..=12).contains(&callers) {
            return Err(ScenarioError::InvalidSetting(
                "a pileup needs 1 to 12 callers",
            ));
        }
    }
    let mut rng = Rng::new(cfg.seed);
    let script = qso::build(cfg, &mut rng)?;
    for t in &script.transmissions {
        codec::tokenize(&t.text).map_err(|source| ScenarioError::Unencodable {
            text: t.text.clone(),
            source,
        })?;
    }
    Ok(script)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds() -> Vec<ScenarioKind> {
        vec![
            ScenarioKind::RagChew,
            ScenarioKind::Contest {
                exchange: ContestExchange::CqZone,
                cut_numbers: false,
            },
            ScenarioKind::Contest {
                exchange: ContestExchange::Serial,
                cut_numbers: true,
            },
            ScenarioKind::Pota {
                park_to_park: false,
            },
            ScenarioKind::Pota { park_to_park: true },
            ScenarioKind::Sota {
                summit_to_summit: false,
            },
            ScenarioKind::Sota {
                summit_to_summit: true,
            },
            ScenarioKind::DxPileup { callers: 6 },
        ]
    }

    fn cfg(kind: ScenarioKind, seed: u64) -> ScenarioConfig {
        ScenarioConfig {
            kind,
            my_call: Callsign::parse("KF0UWV").unwrap(),
            difficulty: Difficulty::advanced(),
            seed,
        }
    }

    fn is_pota_ref(s: &str) -> bool {
        let b = s.as_bytes();
        b.len() >= 7
            && b.len() <= 8
            && b[0].is_ascii_uppercase()
            && b[1].is_ascii_uppercase()
            && b[2] == b'-'
            && b[3..].iter().all(u8::is_ascii_digit)
    }

    fn is_sota_ref(s: &str) -> bool {
        let Some((assoc, rest)) = s.split_once('/') else {
            return false;
        };
        let Some((region, num)) = rest.split_once('-') else {
            return false;
        };
        !assoc.is_empty()
            && assoc
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            && region.len() == 2
            && region.bytes().all(|b| b.is_ascii_uppercase())
            && num.len() == 3
            && num.bytes().all(|b| b.is_ascii_digit())
    }

    #[test]
    fn same_seed_same_script() {
        for kind in kinds() {
            let a = generate(&cfg(kind, 42)).unwrap();
            let b = generate(&cfg(kind, 42)).unwrap();
            let c = generate(&cfg(kind, 43)).unwrap();
            assert_eq!(a, b);
            assert_ne!(a, c, "{kind:?}");
        }
    }

    #[test]
    fn every_line_encodes_across_many_seeds() {
        for kind in kinds() {
            for seed in 0..1000 {
                let s = generate(&cfg(kind, seed)).unwrap();
                assert!(!s.transmissions.is_empty());
                for t in &s.transmissions {
                    assert!(codec::tokenize(&t.text).is_ok(), "{}", t.text);
                    assert!(!t.text.trim().is_empty());
                    if let Party::Them(i) = t.from {
                        assert!(i < s.stations.len());
                    }
                }
                for st in &s.stations {
                    assert!(Callsign::parse(st.persona.call.as_str()).is_ok());
                    assert_ne!(st.persona.call.base(), "KF0UWV");
                    if let Some(r) = &st.persona.reference {
                        match kind {
                            ScenarioKind::Pota { .. } => assert!(is_pota_ref(r), "{r}"),
                            ScenarioKind::Sota { .. } => assert!(is_sota_ref(r), "{r}"),
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn pota_and_sota_activators_carry_references() {
        let p = generate(&cfg(ScenarioKind::Pota { park_to_park: true }, 1)).unwrap();
        let r = p.stations[0].persona.reference.as_deref().unwrap();
        assert!(is_pota_ref(r), "{r}");
        assert!(!r.starts_with("K-"));
        assert!(is_pota_ref(p.me.reference.as_deref().unwrap()));
        let s = generate(&cfg(
            ScenarioKind::Sota {
                summit_to_summit: true,
            },
            1,
        ))
        .unwrap();
        assert!(is_sota_ref(
            s.stations[0].persona.reference.as_deref().unwrap()
        ));
        assert!(is_sota_ref(s.me.reference.as_deref().unwrap()));
    }

    #[test]
    fn pileup_overlaps_with_distinct_signals() {
        let s = generate(&cfg(ScenarioKind::DxPileup { callers: 6 }, 7)).unwrap();
        let overlaps = s
            .transmissions
            .iter()
            .filter(|t| matches!(t.start, Start::WithPrevious { .. }))
            .count();
        assert!(overlaps >= 2, "{overlaps}");
        let mut pitches: Vec<i32> = s
            .stations
            .iter()
            .map(|st| st.signal.pitch_offset_hz.round() as i32)
            .collect();
        pitches.sort();
        pitches.dedup();
        assert!(pitches.len() >= 2);
    }

    #[test]
    fn them_lines_have_signals_and_you_lines_do_not() {
        let s = generate(&cfg(ScenarioKind::RagChew, 3)).unwrap();
        assert!(s.transmissions.iter().any(|t| t.from == Party::You));
        for t in &s.transmissions {
            assert_eq!(s.signal_for(t).is_some(), matches!(t.from, Party::Them(_)));
        }
        assert!(s.transmissions[0].text.contains("CQ"));
        assert!(s.transmissions.iter().any(|t| t.text.contains("KF0UWV")));
    }

    #[test]
    fn contest_exchange_has_rst_and_number() {
        let s = generate(&cfg(
            ScenarioKind::Contest {
                exchange: ContestExchange::CqZone,
                cut_numbers: false,
            },
            5,
        ))
        .unwrap();
        assert!(s.transmissions.iter().any(|t| t.text.contains("5NN")));
        assert!(s.transmissions.iter().any(|t| t.text.contains("TEST")));
    }

    #[test]
    fn invalid_settings_are_errors() {
        assert!(matches!(
            Callsign::parse("NOTACALL"),
            Err(ScenarioError::InvalidCallsign(_))
        ));
        let mut c = cfg(ScenarioKind::DxPileup { callers: 0 }, 0);
        assert!(generate(&c).is_err());
        c.kind = ScenarioKind::RagChew;
        c.difficulty.wpm = (30, 20);
        assert!(generate(&c).is_err());
        c.difficulty = Difficulty::default();
        c.difficulty.fists.clear();
        assert!(generate(&c).is_err());
    }

    #[test]
    fn farnsworth_is_applied_when_slower() {
        let mut c = cfg(ScenarioKind::RagChew, 9);
        c.difficulty = Difficulty::beginner();
        let s = generate(&c).unwrap();
        for st in &s.stations {
            assert!(st.signal.speed.is_farnsworth());
            assert_eq!(st.signal.speed.effective_wpm(), 10.0);
        }
    }

    #[test]
    fn dx_operator_gets_a_dx_home() {
        let mut c = cfg(ScenarioKind::RagChew, 2);
        c.my_call = Callsign::parse("DL1ABC").unwrap();
        let s = generate(&c).unwrap();
        assert_eq!(s.me.zone, 14);
        assert_eq!(s.me.region, "GERMANY");
    }

    #[test]
    fn operators_have_names_from_their_part_of_the_world() {
        use super::data::Names;
        let names = |call: &str| -> Vec<String> {
            (0..20)
                .map(|seed| {
                    let mut c = cfg(ScenarioKind::RagChew, seed);
                    c.my_call = Callsign::parse(call).unwrap();
                    generate(&c).unwrap().me.name
                })
                .collect()
        };
        for (call, pool) in [
            ("JA1ABC", Names::Japanese),
            ("DL1ABC", Names::German),
            ("UA3ABC", Names::EastSlavic),
            ("LU1ABC", Names::LatAm),
            ("KF0UWV", Names::Us),
        ] {
            for n in names(call) {
                assert!(pool.pool().contains(&n.as_str()), "{call}: {n}");
            }
        }
    }

    #[test]
    fn call_area_operators_get_their_area_zone_and_region() {
        let me = |call: &str| {
            let mut c = cfg(ScenarioKind::RagChew, 2);
            c.my_call = Callsign::parse(call).unwrap();
            generate(&c).unwrap().me
        };
        // A Canadian sends a two-letter province, as a POTA exchange does.
        let ve = me("VE7ABC");
        assert_eq!((ve.zone, ve.region.as_str()), (3, "BC"));
        assert_eq!(me("VE3ABC").zone, 4);
        // Elsewhere the region is the entity; the zone is the call area's.
        let ja = me("JA8ABC");
        assert_eq!((ja.zone, ja.region.as_str()), (25, "JAPAN"));
        let ua = me("UA0ABC");
        assert_eq!((ua.zone, ua.region.as_str()), (19, "ASIATIC RUSSIA"));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn script_round_trips_through_json() {
        let s = generate(&cfg(ScenarioKind::DxPileup { callers: 3 }, 4)).unwrap();
        let json = serde_json::to_string(&s).unwrap();
        let back: Script = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
}
