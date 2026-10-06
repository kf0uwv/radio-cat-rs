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

//! A streaming CW tone decoder over f32 PCM.
//!
//! # Pipeline
//!
//! 1. **Tone detection.** A sliding, Hann-windowed Goertzel bin at the
//!    pitch, evaluated every [`DecoderConfig::hop`]. Its window is, by
//!    default, half the current dot estimate (8–40 ms): long enough to
//!    narrow the bandwidth and reject noise, short enough to keep the
//!    edges. The Hann window keeps a station a few hundred hertz away out.
//! 2. **Envelope.** The bin's amplitude against two trackers — a noise
//!    floor (the 10th percentile of recent key-up windows, scaled to the
//!    noise mean, so a keyed tone cannot drag it up) and a peak (instant
//!    attack, slow release so QSB is followed, but never above the median
//!    of the last few marks' peaks, so one crash cannot deafen it). Input
//!    is clamped to ±4 first. Thresholds are in *amplitude*, where a
//!    sliding window's response to a key edge is linear, so the 50 % point
//!    falls on the true edge and mark lengths come out unbiased.
//! 3. **Hysteresis and debounce.** Separate on/off thresholds, never less
//!    than [`DecoderConfig::squelch_db`] above the floor, and a state change
//!    must persist for about a fifth of a dot before it is taken. The
//!    first 300 ms train the floor and peak from every window and are then
//!    *replayed*, so a station keyed from the very first sample loses
//!    nothing. Each finished mark is then
//!    re-timed to where the envelope crossed half of *its own* peak
//!    (interpolated between hops). A blip under 0.3 dot, or a short mark
//!    far louder than the station's recent marks (a crash), is dropped as
//!    noise; a mark longer than three dahs (a tune-up carrier) ends the
//!    word and is kept out of the speed estimate.
//! 4. **Timing.** Marks are split into dits and dahs by two-cluster
//!    k-means (in log time) over recent marks, so the speed estimate
//!    follows the sender. Spaces are split the same way into letter and
//!    word gaps *by their own clusters*, not fixed 3/7-dot cuts, so
//!    Farnsworth spacing is read correctly.
//! 5. **Output.** A letter is emitted as soon as the gap after it is
//!    longer than an intra-character gap — without waiting for the next
//!    mark — and a word space as soon as the gap is a word gap.
//!
//! With [`PitchMode::Auto`], a bank of Goertzel bins 10 Hz apart finds the
//! strongest steady tone first; audio heard before the lock is buffered
//! and replayed, so nothing at the start is lost. The lock is judged
//! against the band's median *outside* the tone's own skirt, over a bank
//! that extends 100 Hz beyond the configured range, so a narrow range
//! locks too. After two seconds with nothing heard on the locked pitch,
//! a steady tone 30 Hz or more away is re-acquired (the last two seconds
//! are replayed through it). Pitches within [`PITCH_MARGIN_HZ`] of 0 Hz or
//! Nyquist are rejected by validation.

use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

use crate::codec::{self, DecodeStyle, Element, Token};
use crate::dsp::GoertzelBin;

/// A configuration that cannot run.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DecoderError {
    /// The sample rate is zero.
    #[error("sample rate must be non-zero")]
    ZeroSampleRate,
    /// A pitch or pitch range outside `(0, Nyquist)`.
    #[error("pitch {0} Hz is outside (0, Nyquist)")]
    PitchOutOfRange(f32),
    /// Some other setting is out of range.
    #[error("invalid decoder setting: {0}")]
    InvalidConfig(&'static str),
}

/// How the decoder knows the tone's pitch.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PitchMode {
    /// The tone is at this pitch.
    Fixed(f32),
    /// Find the strongest steady tone in `[min_hz, max_hz]`.
    Auto { min_hz: f32, max_hz: f32 },
}

/// Decoder settings. [`DecoderConfig::new`] gives working defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DecoderConfig {
    /// Input sample rate.
    pub sample_rate: u32,
    /// Fixed or auto-detected pitch.
    pub pitch: PitchMode,
    /// Envelope time resolution (default 4 ms).
    pub hop: Duration,
    /// Detection window; `None` (default) adapts to half a dot, 8–40 ms.
    pub window: Option<Duration>,
    /// Key-down threshold, as a fraction of floor→peak amplitude (0.55).
    pub on_threshold: f32,
    /// Key-up threshold (0.40); below `on_threshold` for hysteresis.
    pub off_threshold: f32,
    /// The speed assumed before any marks have been heard (20 wpm).
    pub initial_wpm: f64,
    /// Peak must stand this far above the floor, in dB, to decode (10).
    pub squelch_db: f32,
    /// Tones weaker than this (dBFS, amplitude re 1.0) are never decoded
    /// (−70). Without it, in digital silence the faintest leakage from a
    /// station far off the pitch would be "signal" against a zero floor.
    pub min_level_dbfs: f32,
    /// How a pattern shared by a character and a prosign is written.
    pub style: DecodeStyle,
}

impl DecoderConfig {
    /// Defaults at `sample_rate` with the given pitch mode.
    pub fn new(sample_rate: u32, pitch: PitchMode) -> Self {
        DecoderConfig {
            sample_rate,
            pitch,
            hop: Duration::from_millis(4),
            window: None,
            on_threshold: 0.55,
            off_threshold: 0.40,
            initial_wpm: 20.0,
            squelch_db: 10.0,
            min_level_dbfs: -70.0,
            style: DecodeStyle::default(),
        }
    }

    fn validate(&self) -> Result<(), DecoderError> {
        if self.sample_rate == 0 {
            return Err(DecoderError::ZeroSampleRate);
        }
        let nyquist = self.sample_rate as f32 / 2.0;
        // A pitch within PITCH_MARGIN_HZ of 0 or Nyquist has its own image
        // inside the detector's bin and cannot be decoded.
        let ok = |f: f32| f.is_finite() && f >= PITCH_MARGIN_HZ && f <= nyquist - PITCH_MARGIN_HZ;
        match self.pitch {
            PitchMode::Fixed(f) if !ok(f) => return Err(DecoderError::PitchOutOfRange(f)),
            PitchMode::Auto { min_hz, max_hz } => {
                if !ok(min_hz) {
                    return Err(DecoderError::PitchOutOfRange(min_hz));
                }
                if !ok(max_hz) {
                    return Err(DecoderError::PitchOutOfRange(max_hz));
                }
                if max_hz < min_hz + 20.0 {
                    return Err(DecoderError::InvalidConfig(
                        "auto pitch range must span at least 20 Hz",
                    ));
                }
            }
            _ => {}
        }
        let hop = self.hop.as_secs_f64() * f64::from(self.sample_rate);
        if hop < 1.0 || self.hop > Duration::from_millis(20) {
            return Err(DecoderError::InvalidConfig(
                "hop must be at least one sample and at most 20 ms",
            ));
        }
        if let Some(w) = self.window {
            if w < self.hop || w > Duration::from_millis(200) {
                return Err(DecoderError::InvalidConfig(
                    "window must be in [hop, 200 ms]",
                ));
            }
        }
        let t_ok = |x: f32| x.is_finite() && x > 0.0 && x < 1.0;
        if !t_ok(self.on_threshold)
            || !t_ok(self.off_threshold)
            || self.off_threshold > self.on_threshold
        {
            return Err(DecoderError::InvalidConfig(
                "thresholds must satisfy 0 < off ≤ on < 1",
            ));
        }
        if !(self.initial_wpm.is_finite() && self.initial_wpm >= 3.0 && self.initial_wpm <= 80.0) {
            return Err(DecoderError::InvalidConfig(
                "initial_wpm must be in [3, 80]",
            ));
        }
        if !(self.min_level_dbfs.is_finite() && self.min_level_dbfs <= 0.0) {
            return Err(DecoderError::InvalidConfig("min_level_dbfs must be ≤ 0"));
        }
        if !(self.squelch_db.is_finite() && self.squelch_db >= 0.0) {
            return Err(DecoderError::InvalidConfig("squelch_db must be ≥ 0"));
        }
        Ok(())
    }
}

/// One unit of decoder output.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Decoded {
    /// A recognised character or prosign.
    Token(Token),
    /// An element group that is in no table entry (shown as `*`).
    Unknown(Vec<Element>),
    /// A word space.
    WordSpace,
}

impl fmt::Display for Decoded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Decoded::Token(t) => write!(f, "{t}"),
            Decoded::Unknown(_) => f.write_str("*"),
            Decoded::WordSpace => f.write_str(" "),
        }
    }
}

/// Renders decoder output as text.
pub fn decoded_to_text(items: &[Decoded]) -> String {
    items.iter().map(|d| d.to_string()).collect()
}

const MIN_WINDOW_S: f64 = 0.008;
const MAX_WINDOW_S: f64 = 0.040;
const HISTORY: usize = 24;
const HIST_HOPS: usize = 512;
/// How close a decoder pitch may come to 0 Hz or to Nyquist.
pub const PITCH_MARGIN_HZ: f32 = 100.0;
/// Plausible dot lengths: 60 wpm to 5 wpm.
const DIT_RANGE: (f64, f64) = (1.2 / 60.0, 1.2 / 5.0);
const AUTO_BLOCK_S: f64 = 0.1;
const AUTO_STEP_HZ: f32 = 10.0;
const AUTO_LOCK_DB: f64 = 15.0;
const AUTO_BUFFER_S: f64 = 2.0;

/// Two-cluster k-means on log durations. `None` unless the two centres
/// are at least `sep` apart (as a ratio).
fn two_means(xs: &[f64], sep: f64) -> Option<(f64, f64)> {
    if xs.len() < 2 {
        return None;
    }
    let logs: Vec<f64> = xs.iter().map(|x| x.ln()).collect();
    let mut lo = logs.iter().copied().fold(f64::INFINITY, f64::min);
    let mut hi = logs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if hi - lo < sep.ln() {
        return None;
    }
    for _ in 0..10 {
        let mid = (lo + hi) / 2.0;
        let (mut sl, mut nl, mut sh, mut nh) = (0.0, 0usize, 0.0, 0usize);
        for &x in &logs {
            if x < mid {
                sl += x;
                nl += 1;
            } else {
                sh += x;
                nh += 1;
            }
        }
        if nl == 0 || nh == 0 {
            return None;
        }
        lo = sl / nl as f64;
        hi = sh / nh as f64;
    }
    if hi - lo < sep.ln() {
        None
    } else {
        Some((lo.exp(), hi.exp()))
    }
}

fn push_history(h: &mut Vec<f64>, x: f64) {
    if h.len() == HISTORY {
        h.remove(0);
    }
    h.push(x);
}

/// Timing classifier: element and gap lengths in, tokens out.
#[derive(Debug, Clone)]
struct Classifier {
    style: DecodeStyle,
    dit: f64,
    dah: f64,
    marks: Vec<f64>,
    gaps: Vec<f64>,
    group: Vec<Element>,
    emitted_any: bool,
    last_was_space: bool,
    /// Consecutive marks rejected as far too long for the current speed.
    long_rejects: u32,
    /// The next gap follows a rejected mark: classify it, don't learn it.
    skip_gap_history: bool,
}

impl Classifier {
    fn new(initial_wpm: f64, style: DecodeStyle) -> Self {
        let dit = 1.2 / initial_wpm;
        Classifier {
            style,
            dit,
            dah: 3.0 * dit,
            marks: Vec::new(),
            gaps: Vec::new(),
            group: Vec::new(),
            emitted_any: false,
            last_was_space: false,
            long_rejects: 0,
            skip_gap_history: false,
        }
    }

    /// Forgets the learned speed and spacing (a new station).
    fn forget_timing(&mut self, initial_wpm: f64) {
        self.dit = 1.2 / initial_wpm;
        self.dah = 3.0 * self.dit;
        self.marks.clear();
        self.gaps.clear();
        self.long_rejects = 0;
        self.skip_gap_history = false;
    }

    /// Classifies one mark. A mark far longer than any dah at the current
    /// speed (a tune-up carrier, a stuck key) is not an element: it ends
    /// the word, is kept out of the speed estimate, and `false` is
    /// returned. Three in a row are believed — the speed really changed.
    fn mark(&mut self, len: f64, out: &mut Vec<Decoded>) -> bool {
        if len > 3.0 * self.dah.max(3.0 * self.dit) && self.long_rejects < 2 {
            self.long_rejects += 1;
            self.flush_char(out);
            self.word_space(out);
            self.skip_gap_history = true;
            return false;
        }
        if self.long_rejects >= 2 && len > 3.0 * self.dah {
            // Believed: start the speed estimate again from here.
            self.marks.clear();
            self.gaps.clear();
        }
        self.long_rejects = 0;
        push_history(&mut self.marks, len);
        if let Some((d, a)) = two_means(&self.marks, 1.8) {
            self.dit = d;
            self.dah = a;
        }
        let threshold = (self.dit * self.dah).sqrt();
        let el = if len < threshold {
            Element::Dit
        } else {
            Element::Dah
        };
        if self.marks.len() >= 2 && two_means(&self.marks, 1.8).is_none() {
            // One kind of mark so far: let it set the scale.
            let center = self.marks.iter().map(|x| x.ln()).sum::<f64>() / self.marks.len() as f64;
            let center = center.exp();
            match el {
                Element::Dit => {
                    self.dit = center;
                    self.dah = 3.0 * center;
                }
                Element::Dah => {
                    self.dah = center;
                    self.dit = center / 3.0;
                }
            }
        }
        self.dit = self.dit.clamp(DIT_RANGE.0, DIT_RANGE.1);
        self.dah = self.dah.clamp(2.0 * self.dit, 5.0 * self.dit);
        self.group.push(el);
        true
    }

    fn intra_threshold(&self) -> f64 {
        2.0 * self.dit
    }

    /// The letter/word gap boundary, from recent gaps plus (optionally)
    /// the one being judged, so a lone first gap is read as a letter gap
    /// and Farnsworth spacing is learned rather than assumed.
    fn word_threshold(&self, extra: Option<f64>) -> f64 {
        let mut gaps = self.gaps.clone();
        gaps.extend(extra);
        match two_means(&gaps, 1.6) {
            Some((l, w)) => (l * w).sqrt(),
            None => {
                let base = 4.5 * self.dit;
                if gaps.is_empty() {
                    base
                } else {
                    let c = (gaps.iter().map(|x| x.ln()).sum::<f64>() / gaps.len() as f64).exp();
                    base.max(1.6 * c)
                }
            }
        }
    }

    fn flush_char(&mut self, out: &mut Vec<Decoded>) {
        if self.group.is_empty() {
            return;
        }
        let d = match codec::decode_group_with(&self.group, self.style) {
            Some(t) => Decoded::Token(t),
            None => Decoded::Unknown(self.group.clone()),
        };
        self.group.clear();
        out.push(d);
        self.emitted_any = true;
        self.last_was_space = false;
    }

    fn word_space(&mut self, out: &mut Vec<Decoded>) {
        if self.emitted_any && !self.last_was_space {
            out.push(Decoded::WordSpace);
            self.last_was_space = true;
        }
    }

    /// A gap has been running for `elapsed`; emit what it already proves.
    fn gap_running(&mut self, elapsed: f64, out: &mut Vec<Decoded>) {
        if elapsed > self.intra_threshold() {
            self.flush_char(out);
            if elapsed > self.word_threshold(Some(elapsed)) {
                self.word_space(out);
            }
        }
    }

    /// A gap of `len` has ended (a mark is starting).
    fn gap_ended(&mut self, len: f64, out: &mut Vec<Decoded>) {
        if len <= self.intra_threshold() {
            return;
        }
        self.flush_char(out);
        if len > self.word_threshold(Some(len)) {
            self.word_space(out);
        }
        if std::mem::take(&mut self.skip_gap_history) {
            return;
        }
        // Long silences between transmissions would form a third cluster.
        let keep = match two_means(&self.gaps, 1.6) {
            Some((l, w)) => len < 3.0 * (l * w).sqrt(),
            None => len < 60.0 * self.dit,
        };
        if keep {
            push_history(&mut self.gaps, len);
        }
    }
}

/// The tone detector and envelope for one pitch.
#[derive(Debug, Clone)]
struct Detector {
    sample_rate: f64,
    pitch: f32,
    fixed_window: Option<usize>,
    len: usize,
    bin: GoertzelBin,
    ring: Vec<f32>,
    pos: usize,
    filled: usize,
    hop: usize,
    since_hop: usize,
    samples: u64,
    /// Recent key-up amplitudes; the floor is their 10th percentile.
    amps: VecDeque<f64>,
    scratch: Vec<f64>,
    floor_tick: u32,
    floor: f64,
    peak: f64,
    peak_decay: f64,
    /// Hops buffered during warm-up, replayed once the floor is known.
    warmup: Option<Vec<(f64, f64)>>,
    warmup_hops: usize,
    /// Peaks of the last few accepted marks.
    mark_peaks: VecDeque<f64>,
    last_mark_peak: f64,
    on: bool,
    pending: usize,
    pending_t: f64,
    mark_start_raw: f64,
    last_end: Option<f64>,
    hist: VecDeque<(f64, f64)>,
}

/// The 10th percentile of a Rayleigh-distributed amplitude (band noise in
/// one bin) is 0.366 of its mean, so p10 / 0.366 estimates the noise
/// mean. A low percentile, unlike a mean, is not dragged up by a tone
/// that is keyed for most of the window.
const P10_TO_MEAN: f64 = 0.366;
const FLOOR_HOPS: usize = 500;
const WARMUP_S: f64 = 0.3;
const MARK_PEAKS: usize = 5;

fn median(v: &VecDeque<f64>) -> f64 {
    let mut s: Vec<f64> = v.iter().copied().collect();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    s[s.len() / 2]
}

impl Detector {
    fn new(cfg: &DecoderConfig, pitch: f32, dit: f64) -> Self {
        let sr = f64::from(cfg.sample_rate);
        let hop = (cfg.hop.as_secs_f64() * sr).round().max(1.0) as usize;
        let max_len = ((MAX_WINDOW_S.max(cfg.window.map_or(0.0, |w| w.as_secs_f64()))) * sr).ceil()
            as usize
            + 1;
        let fixed_window = cfg
            .window
            .map(|w| (w.as_secs_f64() * sr).round().max(1.0) as usize);
        let hop_s = hop as f64 / sr;
        let mut d = Detector {
            sample_rate: sr,
            pitch,
            fixed_window,
            len: 0,
            bin: GoertzelBin::new(cfg.sample_rate, pitch, 1),
            ring: vec![0.0; max_len],
            pos: 0,
            filled: 0,
            hop,
            since_hop: 0,
            samples: 0,
            amps: VecDeque::with_capacity(FLOOR_HOPS),
            scratch: Vec::with_capacity(FLOOR_HOPS),
            floor_tick: 0,
            floor: 0.0,
            peak: 0.0,
            // Peak release 6 dB/s.
            peak_decay: 10f64.powf(-6.0 * hop_s / 20.0),
            warmup: Some(Vec::new()),
            warmup_hops: ((WARMUP_S / hop_s).ceil() as usize).max(1),
            mark_peaks: VecDeque::with_capacity(MARK_PEAKS),
            last_mark_peak: 0.0,
            on: false,
            pending: 0,
            pending_t: 0.0,
            mark_start_raw: 0.0,
            last_end: None,
            hist: VecDeque::with_capacity(HIST_HOPS),
        };
        d.set_window_for(dit, cfg.sample_rate);
        d
    }

    fn set_window_for(&mut self, dit: f64, sample_rate: u32) {
        let len = self.fixed_window.unwrap_or_else(|| {
            ((0.5 * dit).clamp(MIN_WINDOW_S, MAX_WINDOW_S) * self.sample_rate).round() as usize
        });
        let len = len.clamp(1, self.ring.len());
        // Only retune for a real change, so the envelope is not jittered.
        if self.len == 0 || (len as f64 - self.len as f64).abs() > 0.15 * self.len as f64 {
            self.len = len;
            self.bin = GoertzelBin::new(sample_rate, self.pitch, len);
        }
    }

    fn amplitude(&self) -> f64 {
        let n = self.len;
        let cap = self.ring.len();
        let start = (self.pos + cap - n) % cap;
        let p = if start + n <= cap {
            self.bin.power(&self.ring[start..start + n], &[])
        } else {
            self.bin
                .power(&self.ring[start..], &self.ring[..(start + n) % cap])
        };
        (2.0 * p).sqrt()
    }

    /// Feeds one sample; returns `Some((time_s, amplitude))` at each hop
    /// once a full window has been heard.
    fn push(&mut self, x: f32) -> Option<(f64, f64)> {
        let cap = self.ring.len();
        self.ring[self.pos] = x;
        self.pos = (self.pos + 1) % cap;
        self.filled = (self.filled + 1).min(cap);
        self.samples += 1;
        self.since_hop += 1;
        if self.since_hop < self.hop {
            return None;
        }
        self.since_hop = 0;
        if self.filled < self.len {
            return None;
        }
        let t = (self.samples as f64 - self.len as f64 / 2.0) / self.sample_rate;
        Some((t, self.amplitude()))
    }

    /// Adds a key-up amplitude to the floor estimate.
    fn learn_floor(&mut self, amp: f64) {
        if self.amps.len() == FLOOR_HOPS {
            self.amps.pop_front();
        }
        self.amps.push_back(amp);
        // Re-estimated every fourth window (and always while there are
        // few): the 10th percentile moves slowly, the selection does not
        // need to run 250 times a second.
        self.floor_tick = self.floor_tick.wrapping_add(1);
        if self.amps.len() > 64 && self.floor_tick % 4 != 0 {
            return;
        }
        self.scratch.clear();
        self.scratch.extend(self.amps.iter().copied());
        let k = self.scratch.len() / 10;
        let (_, p10, _) = self.scratch.select_nth_unstable_by(k, |a, b| {
            a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
        });
        self.floor = *p10 / P10_TO_MEAN;
    }

    /// The signal level thresholds are set against: the instant peak
    /// tracker, but never above the median of the last few marks' peaks —
    /// so one crash cannot deafen the decoder while it decays.
    fn ref_peak(&self) -> f64 {
        if self.mark_peaks.len() >= 3 {
            self.peak.min(median(&self.mark_peaks))
        } else {
            self.peak
        }
    }

    fn remember(&mut self, t: f64, amp: f64) {
        if self.hist.len() == HIST_HOPS {
            self.hist.pop_front();
        }
        self.hist.push_back((t, amp));
    }

    /// Ends the mark that started (by the debounced detector) at
    /// `mark_start_raw` and ended at `end_raw`. Both edges are re-timed
    /// to where the envelope crossed half of *this mark's* peak, with
    /// linear interpolation between hops — the debounced times are late
    /// or early by up to the debounce and threshold offsets, and the
    /// first mark after silence is seen before the peak is known at all.
    /// Returns the re-timed `(start, end)`; the mark's peak is left in
    /// `last_mark_peak`.
    fn close_mark(&mut self, end_raw: f64) -> (f64, f64) {
        let hop = self.hop as f64 / self.sample_rate;
        let window = self.len as f64 / self.sample_rate;
        let start_raw = self.mark_start_raw;
        let peak = self
            .hist
            .iter()
            .filter(|(t, _)| *t >= start_raw && *t <= end_raw)
            .map(|(_, a)| *a)
            .fold(0.0, f64::max);
        self.last_mark_peak = peak;
        let floor = self.floor.max(peak * 0.01);
        let level = floor + 0.5 * (peak - floor);
        let lerp = |(t0, a0): (f64, f64), (t1, a1): (f64, f64)| {
            if (a1 - a0).abs() < 1e-30 {
                t1
            } else {
                t0 + (level - a0) / (a1 - a0) * (t1 - t0)
            }
        };
        let pts: Vec<(f64, f64)> = self
            .hist
            .iter()
            .copied()
            .filter(|(t, _)| *t >= start_raw - window - 2.0 * hop && *t <= end_raw + hop)
            .collect();
        let mut start = start_raw;
        if let Some(i) = pts.iter().position(|(_, a)| *a >= level) {
            if i > 0 {
                start = lerp(pts[i - 1], pts[i]);
            } else {
                start = pts[i].0;
            }
        }
        let mut end = end_raw;
        if let Some(i) = pts.iter().rposition(|(t, a)| *a >= level && *t < end_raw) {
            if i + 1 < pts.len() {
                end = lerp(pts[i], pts[i + 1]);
            }
        }
        if end <= start {
            start = start_raw;
            end = end_raw;
        }
        (start, end)
    }
}

/// A streaming CW decoder: feed PCM blocks of any size, get text back.
#[derive(Debug, Clone)]
pub struct Decoder {
    cfg: DecoderConfig,
    det: Option<Detector>,
    cls: Classifier,
    squelch: f64,
    min_level: f64,
    auto: Option<AutoPitch>,
    /// Samples fed so far, and when the decoder last heard its station.
    total: u64,
    last_activity: u64,
}

/// Bins this far either side of the strongest are its own skirt, not the
/// band: left out of the reference the lock is judged against.
const AUTO_SKIRT_BINS: usize = 3;
/// The bank searches this far beyond the configured range so the lock
/// statistic has band to compare against even when the range is narrow.
const AUTO_MARGIN_HZ: f32 = 100.0;
/// A locked pitch is given up for a new one after this long with nothing
/// heard on it (the "squelch closed for a while" rule).
const REACQUIRE_S: f64 = 2.0;
/// A new pitch must be at least this far from the old to relock.
const REACQUIRE_HZ: f32 = 30.0;

/// Pitch search state for [`PitchMode::Auto`].
#[derive(Debug, Clone)]
struct AutoPitch {
    freqs: Vec<f32>,
    /// Indices of `freqs` inside the configured range.
    candidates: std::ops::Range<usize>,
    block: Vec<f32>,
    block_len: usize,
    smoothed: Vec<f64>,
    buffer: VecDeque<f32>,
    buffer_cap: usize,
}

impl AutoPitch {
    fn new(sample_rate: u32, min_hz: f32, max_hz: f32) -> Self {
        let nyquist = sample_rate as f32 / 2.0;
        let lo = (min_hz - AUTO_MARGIN_HZ).max(PITCH_MARGIN_HZ / 2.0);
        let hi = (max_hz + AUTO_MARGIN_HZ).min(nyquist - PITCH_MARGIN_HZ / 2.0);
        let mut freqs = Vec::new();
        let mut first = None;
        let mut last = 0;
        let mut i = 0usize;
        loop {
            let f = lo + AUTO_STEP_HZ * i as f32;
            if f > hi {
                break;
            }
            if f >= min_hz - 0.01 && f <= max_hz + 0.01 {
                first.get_or_insert(freqs.len());
                last = freqs.len();
            }
            freqs.push(f);
            i += 1;
        }
        let first = first.unwrap_or(0);
        let block_len = (AUTO_BLOCK_S * f64::from(sample_rate)).round() as usize;
        AutoPitch {
            smoothed: vec![0.0; freqs.len()],
            candidates: first..last + 1,
            freqs,
            block: Vec::with_capacity(block_len),
            block_len,
            buffer: VecDeque::new(),
            buffer_cap: (AUTO_BUFFER_S * f64::from(sample_rate)) as usize,
        }
    }

    /// Adds a sample; at the end of each block, returns the pitch of the
    /// strongest steady tone in range if one stands out from the band.
    fn feed(&mut self, sample_rate: u32, x: f32) -> Option<f32> {
        if self.buffer.len() == self.buffer_cap {
            self.buffer.pop_front();
        }
        self.buffer.push_back(x);
        self.block.push(x);
        if self.block.len() < self.block_len {
            return None;
        }
        for (s, &f) in self.smoothed.iter_mut().zip(&self.freqs) {
            let p = f64::from(crate::dsp::goertzel_power(&self.block, sample_rate, f));
            // Peak hold that decays, so a station that has gone quiet
            // stops winning within a second or two.
            *s = p.max(0.7 * *s);
        }
        self.block.clear();
        let cmp = |a: &f64, b: &f64| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal);
        let best = self
            .candidates
            .clone()
            .max_by(|&a, &b| cmp(&self.smoothed[a], &self.smoothed[b]))?;
        let bp = self.smoothed[best];
        // The reference is the band away from the tone's own skirt — not
        // a median a narrow bank's neighbours would dominate.
        let mut band: Vec<f64> = self
            .smoothed
            .iter()
            .enumerate()
            .filter(|(i, _)| i.abs_diff(best) > AUTO_SKIRT_BINS)
            .map(|(_, &p)| p)
            .collect();
        if band.is_empty() {
            return None;
        }
        band.sort_by(cmp);
        let reference = band[band.len() / 2].max(1e-30);
        if 10.0 * (bp / reference).log10() < AUTO_LOCK_DB {
            return None;
        }
        // Parabolic interpolation between neighbouring bins (log power).
        let f = self.freqs[best];
        if best == 0 || best + 1 == self.freqs.len() {
            return Some(f);
        }
        let l = self.smoothed[best - 1].max(1e-30).ln();
        let c = bp.max(1e-30).ln();
        let r = self.smoothed[best + 1].max(1e-30).ln();
        let denom = l - 2.0 * c + r;
        let delta = if denom.abs() > 1e-12 {
            (0.5 * (l - r) / denom).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        Some(f + (delta as f32) * AUTO_STEP_HZ)
    }
}

/// Input samples are clamped to this before detection: well above any
/// real full-scale audio, low enough that one absurd sample (1e30) cannot
/// swamp the envelope.
const INPUT_CLAMP: f32 = 4.0;

impl Decoder {
    /// A decoder with `cfg`.
    pub fn new(cfg: DecoderConfig) -> Result<Self, DecoderError> {
        cfg.validate()?;
        let cls = Classifier::new(cfg.initial_wpm, cfg.style);
        let (det, auto) = match cfg.pitch {
            PitchMode::Fixed(p) => (Some(Detector::new(&cfg, p, cls.dit)), None),
            PitchMode::Auto { min_hz, max_hz } => {
                (None, Some(AutoPitch::new(cfg.sample_rate, min_hz, max_hz)))
            }
        };
        Ok(Decoder {
            squelch: 10f64.powf(f64::from(cfg.squelch_db) / 20.0),
            min_level: 10f64.powf(f64::from(cfg.min_level_dbfs) / 20.0),
            cfg,
            det,
            cls,
            auto,
            total: 0,
            last_activity: 0,
        })
    }

    /// The configuration.
    pub fn config(&self) -> &DecoderConfig {
        &self.cfg
    }

    /// The pitch being decoded, once known.
    pub fn pitch_hz(&self) -> Option<f32> {
        self.det.as_ref().map(|d| d.pitch)
    }

    /// The current speed estimate, once a mark has been heard.
    pub fn wpm_estimate(&self) -> Option<f64> {
        if self.cls.marks.is_empty() {
            None
        } else {
            Some(1.2 / self.cls.dit)
        }
    }

    /// Forgets everything heard, including an auto-detected pitch.
    pub fn reset(&mut self) {
        *self = Decoder::new(self.cfg).expect("config was validated");
    }

    /// Decodes a block of samples; returns whatever it completed.
    pub fn feed(&mut self, samples: &[f32]) -> Vec<Decoded> {
        let mut out = Vec::new();
        let sr = self.cfg.sample_rate;
        let reacquire = (REACQUIRE_S * f64::from(sr)) as u64;
        for &x in samples {
            let x = if x.is_finite() {
                x.clamp(-INPUT_CLAMP, INPUT_CLAMP)
            } else {
                0.0
            };
            self.total += 1;
            if let Some(auto) = self.auto.as_mut() {
                let lock = auto.feed(sr, x);
                match (self.det.as_ref(), lock) {
                    (None, Some(p)) => {
                        self.lock_on(p, &mut out);
                        continue;
                    }
                    (None, None) => continue,
                    (Some(d), Some(p))
                        if (p - d.pitch).abs() >= REACQUIRE_HZ
                            && !d.on
                            && self.total - self.last_activity > reacquire =>
                    {
                        self.lock_on(p, &mut out);
                        continue;
                    }
                    _ => {}
                }
            }
            self.step(x, &mut out);
        }
        out
    }

    /// (Re)locks onto `pitch` and replays the buffered audio through it.
    fn lock_on(&mut self, pitch: f32, out: &mut Vec<Decoded>) {
        if self.det.is_some() {
            // A different station: finish the last one's text, and let
            // the new one's speed be learned afresh.
            self.cls.flush_char(out);
            self.cls.word_space(out);
            self.cls.forget_timing(self.cfg.initial_wpm);
        }
        self.det = Some(Detector::new(&self.cfg, pitch, self.cls.dit));
        let buffered: Vec<f32> = self
            .auto
            .as_ref()
            .map(|a| a.buffer.iter().copied().collect())
            .unwrap_or_default();
        for y in buffered {
            self.step(y, out);
        }
        self.last_activity = self.total;
    }

    /// Ends the input: emits any letter still being formed.
    pub fn flush(&mut self) -> Vec<Decoded> {
        let mut out = Vec::new();
        // A stream shorter than the warm-up is decoded with what it has.
        self.finish_warmup(&mut out);
        let mut closed = None;
        if let Some(det) = &mut self.det {
            if det.on {
                let t = det.samples as f64 / det.sample_rate;
                det.on = false;
                closed = Some(det.close_mark(t));
            }
        }
        if let Some(edges) = closed {
            self.mark_heard(edges, &mut out);
        }
        self.cls.flush_char(&mut out);
        out
    }

    fn finish_warmup(&mut self, out: &mut Vec<Decoded>) {
        let Some(det) = self.det.as_mut() else {
            return;
        };
        let Some(buffered) = det.warmup.take() else {
            return;
        };
        for (t, amp) in buffered {
            self.hop(t, amp, out);
        }
    }

    fn step(&mut self, x: f32, out: &mut Vec<Decoded>) {
        let det = self.det.as_mut().expect("detector");
        let Some((t, amp)) = det.push(x) else {
            return;
        };
        if let Some(buf) = det.warmup.as_mut() {
            // Warm-up: learn the floor from *every* window — a low
            // percentile is not fooled by a tone keyed from t = 0 — and
            // the peak, then replay these hops once both are known, so a
            // station that started with the stream loses nothing.
            buf.push((t, amp));
            let full = buf.len() >= det.warmup_hops;
            det.peak = det.peak.max(amp);
            det.learn_floor(amp);
            if full {
                self.finish_warmup(out);
            }
            return;
        }
        if !det.on {
            det.learn_floor(amp);
        }
        self.hop(t, amp, out);
    }

    /// One envelope hop after warm-up: trackers, thresholds, debounce,
    /// edges.
    fn hop(&mut self, t: f64, amp: f64, out: &mut Vec<Decoded>) {
        let det = self.det.as_mut().expect("detector");
        if amp > det.peak {
            det.peak = amp;
        } else {
            det.peak = (det.peak * det.peak_decay).max(det.floor);
        }
        let peak = det.ref_peak();
        // Keep the floor within 40 dB of the peak so silence (a digital
        // zero) does not put the threshold at the very foot of a ramp.
        let floor = det
            .floor
            .max(peak * 0.01)
            .max(self.min_level / self.squelch);
        let span = (peak - floor).max(0.0);
        let open = peak > floor * self.squelch;
        // Relative to the peak, and never less than the squelch margin
        // above the floor: before the first signal the "peak" is just the
        // loudest noise, and that must not key the decoder.
        let on_level = (floor + f64::from(self.cfg.on_threshold) * span).max(floor * self.squelch);
        let off_level =
            (floor + f64::from(self.cfg.off_threshold) * span).max(floor * self.squelch * 0.7);
        let raw_on = open
            && if det.on {
                amp > off_level
            } else {
                amp > on_level
            };

        det.remember(t, amp);

        // Debounce: a change must hold for ~a fifth of a dot.
        let hop_s = det.hop as f64 / det.sample_rate;
        let need = ((0.2 * self.cls.dit / hop_s).round() as usize).max(1);
        let mut ended = None;
        if raw_on != det.on {
            if det.pending == 0 {
                det.pending_t = t;
            }
            det.pending += 1;
            if det.pending >= need {
                let at = det.pending_t;
                det.on = raw_on;
                det.pending = 0;
                if raw_on {
                    det.mark_start_raw = at;
                } else {
                    ended = Some(det.close_mark(at));
                }
            }
        } else {
            det.pending = 0;
        }

        let on = det.on;
        if on {
            self.last_activity = self.total;
        }
        if let Some(edges) = ended {
            self.mark_heard(edges, out);
        }
        // After `mark_heard`, which may just have moved `last_end`.
        let last_end = self.det.as_ref().and_then(|d| d.last_end);
        if let (false, Some(end)) = (on, last_end) {
            self.cls.gap_running(t - end, out);
        }
    }

    fn mark_heard(&mut self, (start, end): (f64, f64), out: &mut Vec<Decoded>) {
        let len = end - start;
        let Some(det) = self.det.as_mut() else {
            return;
        };
        let peak = det.last_mark_peak;
        // A blip much shorter than a dot is noise or a crash, not keying:
        // drop it without disturbing the gap it fell in or the speed
        // estimate.
        if len < 0.3 * self.cls.dit {
            return;
        }
        // Short and far louder than this station's recent marks: a static
        // crash, not an element. Same treatment. (Only short ones: a long
        // loud mark is a stronger signal, and the noise blips heard before
        // a station starts must not lock its first real marks out.)
        if det.mark_peaks.len() >= 3 && len < self.cls.dit && peak > 3.0 * median(&det.mark_peaks) {
            return;
        }
        if det.mark_peaks.len() == MARK_PEAKS {
            det.mark_peaks.pop_front();
        }
        det.mark_peaks.push_back(peak);
        let gap = det.last_end.map(|e| (start - e).max(0.0));
        det.last_end = Some(end);
        if let Some(g) = gap {
            self.cls.gap_ended(g, out);
        }
        if !self.cls.mark(len, out) {
            return;
        }
        let dit = self.cls.dit;
        let sr = self.cfg.sample_rate;
        if let Some(d) = self.det.as_mut() {
            d.set_window_for(dit, sr);
        }
    }
}

/// Decodes a whole buffer to text (leading/trailing spaces trimmed).
pub fn decode_all(cfg: DecoderConfig, samples: &[f32]) -> Result<String, DecoderError> {
    let mut d = Decoder::new(cfg)?;
    let mut items = d.feed(samples);
    items.extend(d.flush());
    Ok(decoded_to_text(&items).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{render_tone, ToneShape};
    use crate::timing::{key_text, Fist, Speed};

    fn tone(text: &str, wpm: f64, sr: u32) -> Vec<f32> {
        let k = key_text(text, Speed::wpm(wpm).unwrap(), &Fist::perfect(), 0).unwrap();
        let mut pcm = vec![0.0; (sr / 5) as usize];
        pcm.extend(render_tone(&k, sr, 700.0, 0.5, &ToneShape::default()).unwrap());
        pcm.extend(vec![0.0; sr as usize]);
        pcm
    }

    #[test]
    fn two_means_separates_dits_and_dahs() {
        let (a, b) = two_means(&[0.06, 0.18, 0.061, 0.17, 0.059], 1.8).unwrap();
        assert!((a - 0.06).abs() < 0.002 && (b - 0.175).abs() < 0.01);
        assert!(two_means(&[0.06, 0.061, 0.059], 1.8).is_none());
    }

    #[test]
    fn decodes_a_clean_tone() {
        let sr = 8000;
        let cfg = DecoderConfig::new(sr, PitchMode::Fixed(700.0));
        let text = decode_all(cfg, &tone("CQ TEST DE W1AW", 20.0, sr)).unwrap();
        assert_eq!(text, "CQ TEST DE W1AW");
    }

    #[test]
    fn letter_is_emitted_before_the_next_mark() {
        let sr = 8000;
        let mut d = Decoder::new(DecoderConfig::new(sr, PitchMode::Fixed(700.0))).unwrap();
        let out = d.feed(&tone("K", 20.0, sr));
        assert_eq!(decoded_to_text(&out).trim(), "K");
    }

    #[test]
    fn unknown_groups_are_marked() {
        let sr = 8000;
        let mut k = crate::timing::Keying::new();
        for i in 0..7 {
            if i > 0 {
                k.push(false, Duration::from_millis(60));
            }
            k.push(true, Duration::from_millis(180));
        }
        // Teach the decoder what a dit is first.
        let mut pcm = tone("EEEE", 20.0, sr);
        pcm.extend(render_tone(&k, sr, 700.0, 0.5, &ToneShape::default()).unwrap());
        pcm.extend(vec![0.0; sr as usize]);
        let text = decode_all(DecoderConfig::new(sr, PitchMode::Fixed(700.0)), &pcm).unwrap();
        assert!(text.ends_with('*'), "{text}");
    }

    #[test]
    fn style_chooses_prosign_or_character() {
        let sr = 8000;
        let pcm = tone("TU <BT> 73", 20.0, sr);
        let cfg = DecoderConfig::new(sr, PitchMode::Fixed(700.0));
        assert_eq!(decode_all(cfg, &pcm).unwrap(), "TU <BT> 73");
        let mut cfg = cfg;
        cfg.style = DecodeStyle::Characters;
        assert_eq!(decode_all(cfg, &pcm).unwrap(), "TU = 73");
    }

    #[test]
    fn invalid_configs_are_rejected() {
        assert_eq!(
            Decoder::new(DecoderConfig::new(0, PitchMode::Fixed(700.0))).unwrap_err(),
            DecoderError::ZeroSampleRate
        );
        assert!(matches!(
            Decoder::new(DecoderConfig::new(8000, PitchMode::Fixed(4500.0))),
            Err(DecoderError::PitchOutOfRange(_))
        ));
        let mut c = DecoderConfig::new(8000, PitchMode::Fixed(700.0));
        c.off_threshold = 0.9;
        assert!(Decoder::new(c).is_err());
    }

    #[test]
    fn silence_decodes_to_nothing() {
        let cfg = DecoderConfig::new(8000, PitchMode::Fixed(700.0));
        assert_eq!(decode_all(cfg, &vec![0.0; 16_000]).unwrap(), "");
    }
}
