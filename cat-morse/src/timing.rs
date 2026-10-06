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

//! Speed (PARIS, Farnsworth), operator fists, and keying as on/off times.
//!
//! A [`Keying`] is the key's state over time — the thing a transmitter or
//! a synthesizer consumes. [`key`] builds one from tokens, a [`Speed`] and
//! a [`Fist`]; the fist's randomness comes from a seed, so the same seed
//! keys the same way. Only `+ − × ÷` and rounding are used, so a seed
//! gives bit-identical keying on every platform (ADR 0021).

use std::time::Duration;

use crate::codec::{self, CodecError, Element, Token};
use crate::rng::Rng;

/// Speed, or speed parameters that are not usable.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TimingError {
    /// Words per minute must be finite and in `(0, 100]`.
    #[error("words per minute must be in (0, 100], got {0}")]
    InvalidWpm(f64),
    /// Farnsworth spacing slows the *effective* speed below the character
    /// speed; it cannot raise it.
    #[error("effective speed {effective} wpm exceeds character speed {character} wpm")]
    EffectiveExceedsChar { character: f64, effective: f64 },
    /// A [`Fist`] field out of range.
    #[error("invalid fist: {0}")]
    InvalidFist(&'static str),
    /// The text to key does not encode.
    #[error(transparent)]
    Codec(#[from] CodecError),
}

/// Sending speed: character speed, and the (possibly slower) effective
/// speed that Farnsworth spacing produces.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Speed {
    char_wpm: f64,
    effective_wpm: f64,
}

fn check_wpm(w: f64) -> Result<f64, TimingError> {
    if w.is_finite() && w > 0.0 && w <= 100.0 {
        Ok(w)
    } else {
        Err(TimingError::InvalidWpm(w))
    }
}

fn secs(s: f64) -> Duration {
    Duration::from_nanos((s * 1e9).round().max(0.0) as u64)
}

impl Speed {
    /// Plain PARIS timing: one unit is `1.2 s / wpm`.
    pub fn wpm(wpm: f64) -> Result<Self, TimingError> {
        let w = check_wpm(wpm)?;
        Ok(Speed {
            char_wpm: w,
            effective_wpm: w,
        })
    }

    /// Farnsworth timing: characters at `char_wpm`, letter and word gaps
    /// stretched so that PARIS comes out at `effective_wpm`.
    pub fn farnsworth(char_wpm: f64, effective_wpm: f64) -> Result<Self, TimingError> {
        let c = check_wpm(char_wpm)?;
        let s = check_wpm(effective_wpm)?;
        if s > c {
            return Err(TimingError::EffectiveExceedsChar {
                character: c,
                effective: s,
            });
        }
        Ok(Speed {
            char_wpm: c,
            effective_wpm: s,
        })
    }

    /// Character speed in words per minute.
    pub fn char_wpm(&self) -> f64 {
        self.char_wpm
    }

    /// Effective (overall) speed in words per minute.
    pub fn effective_wpm(&self) -> f64 {
        self.effective_wpm
    }

    /// Whether letter and word gaps are stretched.
    pub fn is_farnsworth(&self) -> bool {
        self.effective_wpm < self.char_wpm
    }

    fn dot_secs(&self) -> f64 {
        1.2 / self.char_wpm
    }

    /// ARRL (KE3Z) Farnsworth delay: the total added per PARIS word,
    /// spread as 3/19 per letter gap and 7/19 per word gap.
    fn farnsworth_ta(&self) -> f64 {
        let (c, s) = (self.char_wpm, self.effective_wpm);
        (60.0 * c - 37.2 * s) / (s * c)
    }

    fn letter_gap_secs(&self) -> f64 {
        if self.is_farnsworth() {
            3.0 * self.farnsworth_ta() / 19.0
        } else {
            3.0 * self.dot_secs()
        }
    }

    fn word_gap_secs(&self) -> f64 {
        if self.is_farnsworth() {
            7.0 * self.farnsworth_ta() / 19.0
        } else {
            7.0 * self.dot_secs()
        }
    }

    /// One unit: the length of a dot.
    pub fn dot(&self) -> Duration {
        secs(self.dot_secs())
    }

    /// The gap between letters.
    pub fn letter_gap(&self) -> Duration {
        secs(self.letter_gap_secs())
    }

    /// The gap between words.
    pub fn word_gap(&self) -> Duration {
        secs(self.word_gap_secs())
    }
}

/// How a (simulated) operator sends: proportions and random variation.
///
/// Jitters are standard deviations. Element jitter is in dot units;
/// gap jitters are fractions of the nominal gap. `weight` lengthens every
/// mark (and shortens every space) by that many dots, as a heavy keyer
/// setting or a heavy hand does.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Fist {
    /// Dash length in dots; 3.0 is textbook. Must be in `[2, 5]`.
    pub dash_ratio: f64,
    /// Mark lengthening in dots, `[-0.4, 0.4]`.
    pub weight: f64,
    /// σ of each mark and each intra-character gap, in dots, `[0, 0.5]`.
    pub element_jitter: f64,
    /// σ of each letter gap as a fraction of it, `[0, 0.6]`.
    pub letter_gap_jitter: f64,
    /// σ of each word gap as a fraction of it, `[0, 0.6]`.
    pub word_gap_jitter: f64,
    /// σ of a slow, per-character speed wander (fraction), `[0, 0.2]`.
    pub speed_drift: f64,
}

impl Default for Fist {
    fn default() -> Self {
        Fist::perfect()
    }
}

impl Fist {
    /// Machine-perfect keying.
    pub fn perfect() -> Self {
        Fist {
            dash_ratio: 3.0,
            weight: 0.0,
            element_jitter: 0.0,
            letter_gap_jitter: 0.0,
            word_gap_jitter: 0.0,
            speed_drift: 0.0,
        }
    }

    /// A good operator on a paddle or a careful straight key.
    pub fn good() -> Self {
        Fist {
            dash_ratio: 3.0,
            weight: 0.0,
            element_jitter: 0.04,
            letter_gap_jitter: 0.06,
            word_gap_jitter: 0.08,
            speed_drift: 0.01,
        }
    }

    /// An ordinary on-air fist.
    pub fn typical() -> Self {
        Fist {
            dash_ratio: 3.15,
            weight: 0.05,
            element_jitter: 0.08,
            letter_gap_jitter: 0.12,
            word_gap_jitter: 0.15,
            speed_drift: 0.02,
        }
    }

    /// Hard copy: short dashes, a heavy hand, loose spacing.
    pub fn sloppy() -> Self {
        Fist {
            dash_ratio: 2.7,
            weight: 0.12,
            element_jitter: 0.14,
            letter_gap_jitter: 0.25,
            word_gap_jitter: 0.3,
            speed_drift: 0.04,
        }
    }

    /// Checks every field is in its documented range.
    pub fn validate(&self) -> Result<(), TimingError> {
        let in_range = |v: f64, lo: f64, hi: f64| v.is_finite() && v >= lo && v <= hi;
        if !in_range(self.dash_ratio, 2.0, 5.0) {
            return Err(TimingError::InvalidFist("dash_ratio must be in [2, 5]"));
        }
        if !in_range(self.weight, -0.4, 0.4) {
            return Err(TimingError::InvalidFist("weight must be in [-0.4, 0.4]"));
        }
        if !in_range(self.element_jitter, 0.0, 0.5) {
            return Err(TimingError::InvalidFist(
                "element_jitter must be in [0, 0.5]",
            ));
        }
        if !in_range(self.letter_gap_jitter, 0.0, 0.6) || !in_range(self.word_gap_jitter, 0.0, 0.6)
        {
            return Err(TimingError::InvalidFist("gap jitter must be in [0, 0.6]"));
        }
        if !in_range(self.speed_drift, 0.0, 0.2) {
            return Err(TimingError::InvalidFist("speed_drift must be in [0, 0.2]"));
        }
        Ok(())
    }
}

/// One stretch of constant key state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Segment {
    /// `true` while the key is down (tone on).
    pub key_down: bool,
    /// How long.
    pub duration: Duration,
}

/// Key state over time: alternating key-down and key-up segments,
/// starting and ending key-down (no leading or trailing silence) unless
/// built otherwise with [`Keying::push`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Keying {
    segments: Vec<Segment>,
}

impl Keying {
    /// No segments.
    pub fn new() -> Self {
        Keying::default()
    }

    /// The segments, in order. Adjacent segments always differ in state
    /// and none has zero length.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Whether there are no segments.
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Total duration.
    pub fn total(&self) -> Duration {
        self.segments.iter().map(|s| s.duration).sum()
    }

    /// Total key-down time.
    pub fn key_down_time(&self) -> Duration {
        self.segments
            .iter()
            .filter(|s| s.key_down)
            .map(|s| s.duration)
            .sum()
    }

    /// Appends a segment, merging it into the last one if the state is
    /// the same. Zero-length segments are ignored.
    pub fn push(&mut self, key_down: bool, duration: Duration) {
        if duration.is_zero() {
            return;
        }
        match self.segments.last_mut() {
            Some(last) if last.key_down == key_down => last.duration += duration,
            _ => self.segments.push(Segment { key_down, duration }),
        }
    }

    /// Appends `other` after a key-up `gap`.
    pub fn append(&mut self, other: &Keying, gap: Duration) {
        self.push(false, gap);
        for s in &other.segments {
            self.push(s.key_down, s.duration);
        }
    }
}

const STREAM_FIST: u64 = 0x4649_5354; // "FIST"

/// Keys `tokens` at `speed` with `fist`. The fist's variation is drawn
/// from `seed`; [`Fist::perfect`] ignores it.
pub fn key(tokens: &[Token], speed: Speed, fist: &Fist, seed: u64) -> Result<Keying, TimingError> {
    fist.validate()?;
    let mut rng = Rng::new(seed).fork(STREAM_FIST);
    let dot = speed.dot_secs();
    let floor = 0.3 * dot;
    let mut out = Keying::new();
    let mut drift = 0.0f64;
    let mut pending_gap: Option<f64> = None;

    for token in tokens {
        let elements: &[Element] = match token {
            Token::WordSpace => {
                if !out.is_empty() {
                    pending_gap = Some(speed.word_gap_secs());
                }
                continue;
            }
            t => codec::pattern(*t).unwrap_or(&[]),
        };
        if elements.is_empty() {
            continue;
        }
        // Slow speed wander, mean-reverting so it never runs away.
        if fist.speed_drift > 0.0 {
            drift = 0.8 * drift + fist.speed_drift * rng.gauss();
            drift = drift.clamp(-0.3, 0.3);
        }
        let scale = 1.0 + drift;
        let d = dot * scale;

        if !out.is_empty() {
            let nominal = pending_gap.take().unwrap_or(speed.letter_gap_secs());
            let jitter = if nominal > speed.letter_gap_secs() * 1.01 {
                fist.word_gap_jitter
            } else {
                fist.letter_gap_jitter
            };
            let g = nominal * scale * (1.0 + jitter * rng.gauss()) - fist.weight * dot;
            out.push(false, secs(g.max(floor)));
        }
        pending_gap = None;

        for (i, el) in elements.iter().enumerate() {
            if i > 0 {
                let g = d * (1.0 - fist.weight + fist.element_jitter * rng.gauss());
                out.push(false, secs(g.max(floor)));
            }
            let units = match el {
                Element::Dit => 1.0,
                Element::Dah => fist.dash_ratio,
            };
            let m = d * (units + fist.weight + fist.element_jitter * rng.gauss());
            out.push(true, secs(m.max(floor)));
        }
    }
    Ok(out)
}

/// Tokenizes `text` and keys it (see [`key`]).
pub fn key_text(text: &str, speed: Speed, fist: &Fist, seed: u64) -> Result<Keying, TimingError> {
    let tokens = codec::tokenize(text)?;
    key(&tokens, speed, fist, seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn twenty_wpm_is_sixty_ms_dot() {
        let s = Speed::wpm(20.0).unwrap();
        assert_eq!(s.dot(), MS(60));
        assert_eq!(s.letter_gap(), MS(180));
        assert_eq!(s.word_gap(), MS(420));
    }

    #[test]
    fn paris_is_exact_at_twenty_wpm() {
        let s = Speed::wpm(20.0).unwrap();
        // PARIS is 50 units including its trailing 7-unit word gap; keying
        // carries no trailing silence, so 43 units = 2.58 s.
        let k = key_text("PARIS", s, &Fist::perfect(), 0).unwrap();
        assert_eq!(k.total(), MS(43 * 60));
        let two = key_text("PARIS PARIS", s, &Fist::perfect(), 0).unwrap();
        assert_eq!(two.total(), MS((50 + 43) * 60));
        // Every segment is a whole number of units.
        for seg in two.segments() {
            assert_eq!(seg.duration.as_nanos() % 60_000_000, 0);
        }
    }

    #[test]
    fn perfect_e_t_shape() {
        let s = Speed::wpm(20.0).unwrap();
        let k = key_text("ET", s, &Fist::perfect(), 99).unwrap();
        let segs: Vec<(bool, Duration)> = k
            .segments()
            .iter()
            .map(|g| (g.key_down, g.duration))
            .collect();
        assert_eq!(
            segs,
            vec![(true, MS(60)), (false, MS(180)), (true, MS(180))]
        );
    }

    #[test]
    fn farnsworth_gaps_follow_arrl_formula() {
        let s = Speed::farnsworth(18.0, 10.0).unwrap();
        let ta = (60.0 * 18.0 - 37.2 * 10.0) / (10.0 * 18.0);
        assert_eq!(s.dot(), secs(1.2 / 18.0));
        assert_eq!(s.letter_gap(), secs(3.0 * ta / 19.0));
        assert_eq!(s.word_gap(), secs(7.0 * ta / 19.0));
        // PARIS + word gap comes out at the effective speed: 60/10 s.
        let k = key_text("PARIS PARIS", s, &Fist::perfect(), 0).unwrap();
        let one_word = k.total() - key_text("PARIS", s, &Fist::perfect(), 0).unwrap().total();
        let err = (one_word.as_secs_f64() - 6.0).abs();
        assert!(err < 1e-6, "word took {one_word:?}");
    }

    #[test]
    fn equal_speeds_are_not_farnsworth() {
        let s = Speed::farnsworth(20.0, 20.0).unwrap();
        assert!(!s.is_farnsworth());
        assert_eq!(s.letter_gap(), MS(180));
    }

    #[test]
    fn invalid_speeds_are_rejected() {
        assert!(matches!(Speed::wpm(0.0), Err(TimingError::InvalidWpm(_))));
        assert!(matches!(
            Speed::wpm(f64::NAN),
            Err(TimingError::InvalidWpm(_))
        ));
        assert!(matches!(
            Speed::farnsworth(10.0, 20.0),
            Err(TimingError::EffectiveExceedsChar { .. })
        ));
        let mut f = Fist::perfect();
        f.dash_ratio = 1.0;
        assert!(key_text("E", Speed::wpm(20.0).unwrap(), &f, 0).is_err());
    }

    #[test]
    fn codec_errors_surface() {
        assert!(matches!(
            key_text("A#", Speed::wpm(20.0).unwrap(), &Fist::perfect(), 0),
            Err(TimingError::Codec(_))
        ));
    }

    #[test]
    fn seeded_fist_is_deterministic() {
        let s = Speed::wpm(25.0).unwrap();
        let text = "CQ CQ DE W1XYZ W1XYZ K";
        let a = key_text(text, s, &Fist::typical(), 5).unwrap();
        let b = key_text(text, s, &Fist::typical(), 5).unwrap();
        let c = key_text(text, s, &Fist::typical(), 6).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn sloppy_fist_respects_floor() {
        let s = Speed::wpm(30.0).unwrap();
        let floor = secs(0.3 * 1.2 / 30.0);
        for seed in 0..50 {
            let k = key_text("THE QUICK BROWN FOX 0123456789", s, &Fist::sloppy(), seed).unwrap();
            for seg in k.segments() {
                assert!(seg.duration >= floor, "{seg:?}");
            }
        }
    }

    #[test]
    fn jittered_dash_ratio_holds_on_average() {
        let s = Speed::wpm(20.0).unwrap();
        let mut f = Fist::good();
        f.speed_drift = 0.0;
        let k = key_text(&"TE ".repeat(3000), s, &f, 1).unwrap();
        let marks: Vec<f64> = k
            .segments()
            .iter()
            .filter(|g| g.key_down)
            .map(|g| g.duration.as_secs_f64())
            .collect();
        let dits: Vec<f64> = marks.iter().skip(1).step_by(2).copied().collect();
        let dahs: Vec<f64> = marks.iter().step_by(2).copied().collect();
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let ratio = mean(&dahs) / mean(&dits);
        assert!((ratio - 3.0).abs() < 0.05, "ratio {ratio}");
    }

    #[test]
    fn push_merges_and_append_inserts_gap() {
        let mut k = Keying::new();
        k.push(true, MS(10));
        k.push(true, MS(5));
        k.push(false, Duration::ZERO);
        assert_eq!(k.segments().len(), 1);
        let mut other = Keying::new();
        other.push(true, MS(20));
        k.append(&other, MS(100));
        assert_eq!(k.segments().len(), 3);
        assert_eq!(k.total(), MS(135));
        assert_eq!(k.key_down_time(), MS(35));
    }
}
