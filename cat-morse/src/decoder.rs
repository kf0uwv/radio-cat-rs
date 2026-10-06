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
//!    floor (followed only while key-up) and a peak (instant attack, slow
//!    release, so QSB is followed). Thresholds are in *amplitude*, where a
//!    sliding window's response to a key edge is linear, so the 50 % point
//!    falls on the true edge and mark lengths come out unbiased.
//! 3. **Hysteresis and debounce.** Separate on/off thresholds, never less
//!    than [`DecoderConfig::squelch_db`] above the floor, and a state change
//!    must persist for about a fifth of a dot before it is taken. The
//!    first ~60 ms only train the floor. Each finished mark is then
//!    re-timed to where the envelope crossed half of *its own* peak
//!    (interpolated between hops), and a blip under 0.3 dot is dropped as
//!    noise.
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
//! and replayed, so nothing at the start is lost. The lock holds until
//! [`Decoder::reset`].

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
    /// Peak must stand this far above the floor, in dB, to decode (13).
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
            squelch_db: 13.0,
            min_level_dbfs: -70.0,
            style: DecodeStyle::default(),
        }
    }

    fn validate(&self) -> Result<(), DecoderError> {
        if self.sample_rate == 0 {
            return Err(DecoderError::ZeroSampleRate);
        }
        let nyquist = self.sample_rate as f32 / 2.0;
        let ok = |f: f32| f.is_finite() && f > 0.0 && f < nyquist;
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
/// Hops (of a full window) before the decoder may key: lets the noise
/// floor settle so the first noise peak is not taken for a signal.
const WARMUP_HOPS: u32 = 15;
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
        }
    }

    fn mark(&mut self, len: f64) {
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
    floor: f64,
    peak: f64,
    peak_decay: f64,
    floor_alpha: f64,
    floor_creep: f64,
    floor_n: u32,
    hops: u32,
    on: bool,
    pending: usize,
    pending_t: f64,
    mark_start_raw: f64,
    last_end: Option<f64>,
    hist: VecDeque<(f64, f64)>,
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
            floor: 0.0,
            peak: 0.0,
            // Peak release 6 dB/s; floor time constant ~0.25 s.
            peak_decay: 10f64.powf(-6.0 * hop_s / 20.0),
            floor_alpha: (hop_s / 0.25).min(1.0),
            floor_creep: 10f64.powf(hop_s / 20.0),
            floor_n: 0,
            hops: 0,
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
        if self.filled < self.len {
            return 0.0;
        }
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

    /// Feeds one sample; returns `Some(time_s)` at each hop.
    fn push(&mut self, x: f32) -> Option<f64> {
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
        Some((self.samples as f64 - self.len.min(self.filled) as f64 / 2.0) / self.sample_rate)
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
    /// Returns the re-timed `(start, end)`.
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
}

/// Pitch search state for [`PitchMode::Auto`].
#[derive(Debug, Clone)]
struct AutoPitch {
    freqs: Vec<f32>,
    block: Vec<f32>,
    block_len: usize,
    smoothed: Vec<f64>,
    buffer: Vec<f32>,
    buffer_cap: usize,
}

impl AutoPitch {
    fn new(sample_rate: u32, min_hz: f32, max_hz: f32) -> Self {
        let mut freqs = Vec::new();
        let mut f = min_hz;
        while f <= max_hz {
            freqs.push(f);
            f += AUTO_STEP_HZ;
        }
        let block_len = (AUTO_BLOCK_S * f64::from(sample_rate)).round() as usize;
        AutoPitch {
            smoothed: vec![0.0; freqs.len()],
            freqs,
            block: Vec::with_capacity(block_len),
            block_len,
            buffer: Vec::new(),
            buffer_cap: (AUTO_BUFFER_S * f64::from(sample_rate)) as usize,
        }
    }

    /// Adds samples; returns a pitch once one stands out.
    fn feed(&mut self, sample_rate: u32, x: f32) -> Option<f32> {
        if self.buffer.len() == self.buffer_cap {
            self.buffer.drain(..self.block_len.min(self.buffer.len()));
        }
        self.buffer.push(x);
        self.block.push(x);
        if self.block.len() < self.block_len {
            return None;
        }
        let powers: Vec<f64> = self
            .freqs
            .iter()
            .map(|&f| f64::from(crate::dsp::goertzel_power(&self.block, sample_rate, f)))
            .collect();
        self.block.clear();
        for (s, p) in self.smoothed.iter_mut().zip(&powers) {
            *s = s.max(*p).max(0.5 * *s + 0.5 * p);
        }
        let mut sorted = self.smoothed.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = sorted[sorted.len() / 2].max(1e-30);
        let (best, &bp) = self
            .smoothed
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))?;
        if 10.0 * (bp / median).log10() < AUTO_LOCK_DB {
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
        for &x in samples {
            if self.det.is_none() {
                let sr = self.cfg.sample_rate;
                let auto = self.auto.as_mut().expect("auto mode without detector");
                if let Some(p) = auto.feed(sr, x) {
                    let buffered = std::mem::take(&mut auto.buffer);
                    self.det = Some(Detector::new(&self.cfg, p, self.cls.dit));
                    for &y in &buffered {
                        self.step(y, &mut out);
                    }
                }
                continue;
            }
            self.step(x, &mut out);
        }
        out
    }

    /// Ends the input: emits any letter still being formed.
    pub fn flush(&mut self) -> Vec<Decoded> {
        let mut out = Vec::new();
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

    fn step(&mut self, x: f32, out: &mut Vec<Decoded>) {
        let det = self.det.as_mut().expect("detector");
        let Some(t) = det.push(x) else {
            return;
        };
        let amp = det.amplitude();

        // Trackers.
        if amp > det.peak {
            det.peak = amp;
        } else {
            det.peak = (det.peak * det.peak_decay).max(det.floor);
        }
        if det.filled >= det.len && !det.on {
            // The noise floor: a mean of key-up amplitude. For its first
            // 25 windows it is a plain running mean, so it is right within
            // ~100 ms; after that it ignores spikes and mark tails (more
            // than 2.5x the floor) and instead creeps up ~1 dB/s while
            // everything is above it, so a rising band noise is followed.
            // Digital silence leaves it at zero, which the clamps below
            // handle.
            if det.floor_n < 25 || amp < 2.5 * det.floor + 1e-12 {
                det.floor_n += 1;
                let a = det.floor_alpha.max(1.0 / det.floor_n as f64);
                det.floor += a * (amp - det.floor);
            } else {
                det.floor *= det.floor_creep;
            }
        }
        // Keep the floor within 40 dB of the peak so silence (a digital
        // zero) does not put the threshold at the very foot of a ramp.
        let floor = det
            .floor
            .max(det.peak * 0.01)
            .max(self.min_level / self.squelch);
        let span = det.peak - floor;
        let open = det.peak > floor * self.squelch;
        // Relative to the peak, and never less than the squelch margin
        // above the floor: before the first signal the "peak" is just the
        // loudest noise, and that must not key the decoder.
        let on_level = (floor + f64::from(self.cfg.on_threshold) * span).max(floor * self.squelch);
        let off_level =
            (floor + f64::from(self.cfg.off_threshold) * span).max(floor * self.squelch * 0.7);
        // No keying until the floor has had a moment to settle.
        if det.filled >= det.len {
            det.hops = det.hops.saturating_add(1);
        }
        let warm = det.hops > WARMUP_HOPS;
        let raw_on = warm
            && open
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
        // A blip much shorter than a dot is noise or a crash, not keying:
        // drop it without disturbing the gap it fell in or the speed
        // estimate.
        if len < 0.3 * self.cls.dit {
            return;
        }
        let gap = self
            .det
            .as_ref()
            .and_then(|d| d.last_end)
            .map(|e| (start - e).max(0.0));
        if let Some(d) = self.det.as_mut() {
            d.last_end = Some(end);
        }
        if let Some(g) = gap {
            self.cls.gap_ended(g, out);
        }
        self.cls.mark(len);
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
