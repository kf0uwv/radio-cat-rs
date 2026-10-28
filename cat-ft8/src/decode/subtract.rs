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

//! Subtracting one decoded signal from a slot's audio (ADR 0024,
//! amendment "subtraction and further passes").
//!
//! [`subtract`] rebuilds a decode's waveform at unit amplitude with
//! [`Synth::fill_phase`] (a whole-sample start), estimates where and how
//! the signal actually sits in the audio, and subtracts the scaled
//! reference in place. It reads only the audio, the decode and its
//! arguments: no spectrogram and no decoder state. Its working storage is
//! on the stack and bounded (`SUBTRACT_STACK_BYTES`); it allocates nothing.
//!
//! 1. **Fine time.** For each symbol the audio is correlated with the
//!    unit reference over the symbol's middle half, giving one complex
//!    value per symbol. Between symbols k and k+1 a start error of e
//!    samples turns the phase by −2π·m·e/1920, where m is the tone step
//!    (each 6.25 Hz tone is 1/1920 of a turn per sample), plus a common
//!    term from any frequency error. The products of adjacent symbols'
//!    values are summed by tone step within six groups of 13 pairs
//!    (about 2 s each), and each trial e in ±½ symbol is scored by the
//!    groups' aligned magnitudes, added without their phases (so drift
//!    and fading need only hold the phase within a group): every 16
//!    samples, then every sample near the best, then a parabola through
//!    the best three for the fraction. The start moves by the
//!    whole-sample part; only if that move is larger than ⅛ symbol (so the
//!    signal's tone changes may have reached the symbols' middle halves)
//!    is the step repeated at the moved start, at most three times in all.
//! 2. **Fine frequency.** Each group's aligned phase is its frequency
//!    error times one symbol. With [`SubtractConfig::drift`] on, a line
//!    through the groups' errors (weighted by their squared magnitudes,
//!    which do not depend on the phase) gives the
//!    error at the middle of the transmission and a linear drift;
//!    otherwise the phase of the groups' sum gives one error.
//! 3. **Amplitude.** Per segment of [`SubtractConfig::segment`] samples,
//!    least squares of the audio on the frequency-corrected reference's
//!    cosine and sine: a complex amplitude per segment (no bias from the
//!    reference's double-frequency term, which matters near 0 Hz).
//! 4. **Subtraction.** The amplitude is interpolated linearly between
//!    segment centres (held constant before the first and after the
//!    last) and the scaled reference subtracted. A segment with too few
//!    usable samples takes its neighbour's amplitude.
//!
//! Only the audio from `skip` to the end of the buffer is read or
//! written: the skipped region (missing input, zero by convention) stays
//! as it is, and a buffer shorter than a slot is a slot cut short. Samples
//! are read as the decoder's front end takes them (a non-finite value as
//! 0.0, a finite one beyond ±2¹⁶ clamped), and a sample the subtraction
//! changes is written as that value less the estimate, so the residual is
//! what the next pass would see, less the signal.
//!
//! All arithmetic is `f32` `+ − × ÷`, comparisons, integer phase and the
//! crate's polynomial helpers, in a fixed order, so the result is
//! deterministic (bit for bit on every platform the crate's promise
//! covers).

use super::decoder::Decode;
use super::frontend::INPUT_LIMIT;
use super::math::{atan2_turns, log2_power};
use crate::waveform::{cos_turns, sin_turns, Placement, Synth};
use crate::{
    encode_payload, Error, SAMPLES_PER_SYMBOL, SAMPLE_RATE, SYMBOL_COUNT, WAVEFORM_SAMPLES,
};

/// Samples per symbol.
const T: usize = SAMPLES_PER_SYMBOL;
/// Reference samples generated at a time.
const BLOCK: usize = 240;
// Symbol cores ([T/4, 3T/4)) and segments are whole blocks.
const _: () = assert!(T % BLOCK == 0 && (T / 4) % BLOCK == 0 && WAVEFORM_SAMPLES % BLOCK == 0);

/// Most segments in one waveform (`run` uses `div_ceil` too).
const MAX_SEGMENTS: usize = WAVEFORM_SAMPLES.div_ceil(SubtractConfig::MIN_SEGMENT);
/// Timing steps at most (each one pass over the signal).
const TIMING_PASSES: usize = 3;
/// The coarse grid of the time search, samples.
const COARSE_STEP: i32 = 16;
/// One turn of phase (2³²) as `f32`.
const TURN: f32 = 4_294_967_296.0;
/// Phase increment per hertz per sample.
const TURNS_PER_HZ: f32 = TURN / SAMPLE_RATE as f32;
/// Hertz per turn of phase between adjacent symbols (1 / 0.16 s).
const HZ_PER_SYMBOL_TURN: f32 = SAMPLE_RATE as f32 / T as f32;
/// The middle of the waveform, samples.
const MIDDLE: f32 = WAVEFORM_SAMPLES as f32 / 2.0;

/// Upper bound on [`subtract`]'s own stack storage, bytes: the per-symbol
/// correlations, one set of per-group step sums, the per-segment
/// amplitudes and one block of reference phase and envelope. (The
/// compiler's frame adds scalars, temporaries and call overhead; the
/// embedded link check reports it.)
pub const STACK_BYTES: usize = SYMBOL_COUNT * 8 + GROUPS * 15 * 8 + MAX_SEGMENTS * 8 + BLOCK * 8;

/// Settings of [`subtract`]. Start from [`SubtractConfig::DEFAULT`] and
/// change fields with the `with_*` setters (the type is non-exhaustive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SubtractConfig {
    /// Samples per amplitude segment: a multiple of
    /// [`SubtractConfig::SEGMENT_STEP`] from [`SubtractConfig::MIN_SEGMENT`]
    /// to the waveform's length. Shorter follows fading
    /// and frequency error better; longer averages more noise.
    pub segment: usize,
    /// Estimate a linear frequency drift as well as the offset.
    pub drift: bool,
}

impl SubtractConfig {
    /// Segment lengths are a multiple of this many samples (20 ms).
    pub const SEGMENT_STEP: usize = BLOCK;
    /// The shortest segment, samples (half a symbol; shorter segments
    /// measured worse in every condition).
    pub const MIN_SEGMENT: usize = 4 * BLOCK;

    /// The measured default: one symbol (1 920 samples) per segment,
    /// drift on.
    pub const DEFAULT: SubtractConfig = SubtractConfig {
        segment: T,
        drift: true,
    };

    /// With `segment` samples per amplitude segment.
    pub const fn with_segment(self, segment: usize) -> Self {
        SubtractConfig { segment, ..self }
    }

    /// With drift estimation on or off.
    pub const fn with_drift(self, drift: bool) -> Self {
        SubtractConfig { drift, ..self }
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        let s = self.segment;
        if !(Self::MIN_SEGMENT..=WAVEFORM_SAMPLES).contains(&s) || s % Self::SEGMENT_STEP != 0 {
            return Err(Error::InvalidConfig);
        }
        Ok(())
    }
}

impl Default for SubtractConfig {
    fn default() -> Self {
        SubtractConfig::DEFAULT
    }
}

/// What [`subtract`] estimated and removed. Non-exhaustive: further
/// estimates may be added.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[non_exhaustive]
pub struct Subtraction {
    /// Slot sample of the subtracted waveform's first sample (may be
    /// negative or beyond the buffer; only its overlap with the buffer
    /// after `skip` is changed).
    pub start: i64,
    /// The estimated start before rounding to a whole sample, relative to
    /// the nominal 0.5 s, seconds.
    pub dt: f32,
    /// Estimated frequency of tone 0 at the middle of the transmission, Hz.
    pub freq_hz: f32,
    /// Estimated frequency drift, Hz per second (0 with drift off).
    pub drift_hz_per_s: f32,
    /// Samples changed.
    pub samples: usize,
    /// Energy of the subtracted waveform (sum of squared samples).
    pub energy: f32,
    /// Energy the reference still sees after subtraction: per segment, the
    /// least-squares projection of the residual on the reference, summed.
    /// This is what the model left behind (interpolation between
    /// segments, a fade or drift it did not follow); other signals in the
    /// segment add to it. Noise in the amplitude estimate does not show
    /// here, since the fit removes the noise's own projection.
    pub residue: f32,
    /// The error the noise is expected to leave in the amplitude estimate:
    /// per segment, two samples' worth (the fit's two dimensions) of the
    /// residual's mean power, summed. Other signals in the segment count
    /// as noise here, so in a crowded slot this overstates it.
    pub residue_noise: f32,
}

impl Subtraction {
    /// The estimated signal left after subtraction, dB relative to the
    /// energy removed: [`Subtraction::residue`] plus
    /// [`Subtraction::residue_noise`] (`None` if nothing was subtracted).
    /// Measured on single signals in white noise: within about 2.5 dB of
    /// the true value, about 2 dB high where noise dominates and up to
    /// 2.4 dB low for fades faster than the segments follow.
    ///
    /// It sees the residual only along the reference waveform (each
    /// segment's cosine and sine): a mismatch orthogonal to the reference
    /// is invisible to it. A signal from another generator whose ramps are
    /// a sample or two away from this crate's, for example, leaves a
    /// residue `left_db` does not report; only a comparison with the true
    /// slot measures that.
    pub fn left_db(&self) -> Option<f32> {
        ratio_db(self.residue + self.residue_noise, self.energy)
    }

    /// [`Subtraction::residue`] relative to [`Subtraction::energy`], dB
    /// (`None` if nothing was subtracted).
    pub fn residue_db(&self) -> Option<f32> {
        ratio_db(self.residue, self.energy)
    }

    /// [`Subtraction::residue_noise`] relative to [`Subtraction::energy`],
    /// dB (`None` if nothing was subtracted).
    pub fn residue_noise_db(&self) -> Option<f32> {
        ratio_db(self.residue_noise, self.energy)
    }
}

/// 10·log10(a / b), or `None` unless b > 0.
fn ratio_db(a: f32, b: f32) -> Option<f32> {
    // 10 / log2(10).
    const DB_PER_OCTAVE: f32 = 3.010_3;
    (b > 0.0).then(|| DB_PER_OCTAVE * (log2_power(a) - log2_power(b)))
}

/// Subtracts the signal of `decode` from `audio` in place and returns the
/// estimates.
///
/// `audio[0]` is slot sample 0 (the nominal start is sample 6 000); the
/// first `skip` samples are missing input and are neither read nor
/// written. A buffer shorter than a slot is a slot cut short; one longer
/// is fine too. The reference starts at the decode's time offset and
/// frequency and is refined against the audio (see the module docs).
///
/// Errors: [`Error::InvalidConfig`] for a segment length out of range,
/// [`Error::NonFinite`] for a non-finite time offset, and
/// [`Error::FrequencyOutOfRange`] for a frequency the waveform cannot
/// have. A decode whose waveform does not overlap the usable audio
/// changes nothing (`samples` is 0).
pub fn subtract(
    audio: &mut [f32],
    decode: &Decode,
    skip: usize,
    config: &SubtractConfig,
) -> Result<Subtraction, Error> {
    run(audio, decode, skip, config, Fixed::default())
}

/// Estimates to impose instead of measuring them (measurement only).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Fixed {
    /// The waveform's first slot sample; the time search is skipped.
    pub start: Option<i64>,
    /// Frequency offset from the decode's (Hz, at the middle) and drift
    /// (Hz/s); the frequency estimate is skipped.
    pub freq: Option<(f32, f32)>,
}

/// [`subtract`] with some estimates imposed: for measuring how much of the
/// residue each estimate leaves. UNSTABLE: `profile` feature only.
#[cfg(feature = "profile")]
#[doc(hidden)]
pub fn subtract_fixed(
    audio: &mut [f32],
    decode: &Decode,
    skip: usize,
    config: &SubtractConfig,
    fixed: Fixed,
) -> Result<Subtraction, Error> {
    run(audio, decode, skip, config, fixed)
}

/// The usable audio: slot samples `lo..hi`.
#[derive(Clone, Copy)]
struct Usable {
    lo: i64,
    hi: i64,
}

impl Usable {
    /// The reference samples `j` in `j0..j0 + n` whose slot sample
    /// `start + j` is usable.
    fn range(self, start: i64, j0: usize, n: usize) -> (usize, usize) {
        let (a, b) = (j0 as i64, (j0 + n) as i64);
        let lo = (self.lo - start).clamp(a, b);
        let hi = (self.hi - start).clamp(lo, b);
        (lo as usize, hi as usize)
    }
}

/// Runs the reference generator from its first sample in blocks, calling
/// `f(j0, phase, envelope)` for the blocks where `want(j0)` holds (the
/// others are skipped, phase advancing), until `end` samples.
fn blocks(
    synth: &Synth,
    end: usize,
    want: impl Fn(usize) -> bool,
    mut f: impl FnMut(usize, &[u32], &[f32]),
) {
    let mut s = synth.clone();
    let mut phase = [0u32; BLOCK];
    let mut env = [0.0f32; BLOCK];
    let mut j0 = 0;
    while j0 < end {
        if want(j0) {
            let n = s.fill_phase(&mut phase, &mut env);
            if n == 0 {
                break;
            }
            f(j0, &phase[..n], &env[..n]);
            j0 += n;
        } else {
            let n = s.skip(BLOCK);
            if n == 0 {
                break;
            }
            j0 += n;
        }
    }
}

/// Per symbol, the audio times the conjugate unit reference starting at
/// `start`, summed over the symbol's middle half.
fn symbol_correlations(
    audio: &[f32],
    usable: Usable,
    synth: &Synth,
    start: i64,
) -> [(f32, f32); SYMBOL_COUNT] {
    let mut c = [(0.0f32, 0.0f32); SYMBOL_COUNT];
    let end = usable.range(start, 0, WAVEFORM_SAMPLES).1;
    let core = |j0: usize| (T / 4..3 * T / 4).contains(&(j0 % T));
    blocks(synth, end, core, |j0, phase, env| {
        let (a, b) = usable.range(start, j0, phase.len());
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for j in a..b {
            let i = j - j0;
            let w = input(audio[(start + j as i64) as usize]) * env[i];
            re += w * cos_turns(phase[i]);
            im -= w * sin_turns(phase[i]);
        }
        let k = j0 / T;
        c[k].0 += re;
        c[k].1 += im;
    });
    c
}

/// A sample as the decoder's front end takes it: a non-finite value is
/// 0.0 and a finite one beyond ±[`INPUT_LIMIT`] is clamped to it.
#[inline]
fn input(x: f32) -> f32 {
    if x.is_finite() {
        x.clamp(-INPUT_LIMIT, INPUT_LIMIT)
    } else {
        0.0
    }
}

/// `a · conj(b)`.
fn mul_conj(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    (a.0 * b.0 + a.1 * b.1, a.1 * b.0 - a.0 * b.1)
}

/// `z` turned by `phase` (2³² = one turn).
fn rotate(z: (f32, f32), phase: u32) -> (f32, f32) {
    let (c, s) = (cos_turns(phase), sin_turns(phase));
    (z.0 * c - z.1 * s, z.0 * s + z.1 * c)
}

/// `turns` (any real number) as a phase.
fn phase_of_turns(turns: f32) -> u32 {
    // Saturating cast, then modulo 2³² (the integer part does not matter).
    (turns * TURN) as i64 as u32
}

/// The phase of `m · e / 1920` turns for whole samples `e`, exactly.
fn step_phase(m: i32, e: i32) -> u32 {
    let r = (m * e).rem_euclid(T as i32) as u64;
    ((r << 32) / T as u64) as u32
}

/// Pairs of adjacent symbols.
const PAIRS: usize = SYMBOL_COUNT - 1;
/// Groups of consecutive pairs (13 pairs, about 2 s, each): the time
/// search adds the groups' alignments without their phases, so a drifting
/// or fading signal's phase need only hold within a group.
const GROUPS: usize = 6;
const _: () = assert!(PAIRS % GROUPS == 0);
/// Pairs per group.
const PER_GROUP: usize = PAIRS / GROUPS;

/// Per group, products of adjacent symbols' correlations summed by tone
/// step (`m + 7` for m = −7..=7).
type StepSums = [[(f32, f32); 15]; GROUPS];

/// The tone step from symbol `k` to `k + 1`.
fn tone_step(tones: &[u8; SYMBOL_COUNT], k: usize) -> i32 {
    i32::from(tones[k + 1]) - i32::from(tones[k])
}

fn step_sums(c: &[(f32, f32); SYMBOL_COUNT], tones: &[u8; SYMBOL_COUNT]) -> StepSums {
    let mut d = [[(0.0f32, 0.0f32); 15]; GROUPS];
    for k in 0..PAIRS {
        let p = mul_conj(c[k + 1], c[k]);
        let slot = &mut d[k / PER_GROUP][(tone_step(tones, k) + 7) as usize];
        slot.0 += p.0;
        slot.1 += p.1;
    }
    d
}

/// One group's sum, aligned for a start error whose phase per tone step
/// is `phase(m)`.
fn aligned(g: &[(f32, f32); 15], phase: impl Fn(i32) -> u32) -> (f32, f32) {
    let (mut re, mut im) = (0.0f32, 0.0f32);
    for (k, &z) in g.iter().enumerate() {
        let r = rotate(z, phase(k as i32 - 7));
        re += r.0;
        im += r.1;
    }
    (re, im)
}

/// The alignment score of a start error of `e` samples.
fn score(d: &StepSums, e: i32) -> f32 {
    let mut total = 0.0f32;
    for g in d {
        let (re, im) = aligned(g, |m| step_phase(m, e));
        total += re * re + im * im;
    }
    total
}

/// The start error (whole samples, and the fraction from a parabola
/// through the best three) that best aligns the adjacent products, within
/// ±½ symbol; `None` if the audio carries nothing of the reference.
fn align(d: &StepSums) -> Option<(i32, f32)> {
    let half = T as i32 / 2;
    let best_of = |from: i32, to: i32, step: i32| -> (i32, f32) {
        let mut best = (from, score(d, from));
        let mut e = from + step;
        while e <= to {
            let s = score(d, e);
            if s > best.1 {
                best = (e, s);
            }
            e += step;
        }
        best
    };
    let (coarse, s) = best_of(-half, half - COARSE_STEP, COARSE_STEP);
    // False for NaN as well.
    let found = s > 0.0 && s.is_finite();
    if !found {
        return None;
    }
    let (e, s0) = best_of(coarse - COARSE_STEP + 1, coarse + COARSE_STEP - 1, 1);
    let (a, b) = (score(d, e - 1), score(d, e + 1));
    let den = a - 2.0 * s0 + b;
    let frac = if den < 0.0 {
        ((a - b) / (2.0 * den)).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    Some((e, frac))
}

/// Frequency offset at the middle (Hz) and drift (Hz/s) from the step
/// sums of correlations taken at a start `e` samples (fractional) before
/// the signal's: each group's aligned phase over one symbol is its
/// frequency error; with `drift`, a line through the groups' errors
/// (weighted by their squared magnitudes), otherwise the phase of their
/// sum.
fn frequency(d: &StepSums, e: f32, drift: bool) -> (f32, f32) {
    let hz = |z: (f32, f32)| atan2_turns(z.1, z.0) * HZ_PER_SYMBOL_TURN;
    let mut z = [(0.0f32, 0.0f32); GROUPS];
    for (zg, g) in z.iter_mut().zip(d) {
        *zg = aligned(g, |m| phase_of_turns(m as f32 * e / T as f32));
    }
    if !drift {
        let sum = z.iter().fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
        return (hz(sum), 0.0);
    }
    // Group g's pairs lie between symbol centres, at (k + 1)·T for k in
    // the group: centred on (g·PER_GROUP + (PER_GROUP + 1)/2)·T, here in
    // seconds from the middle of the waveform.
    let at = |g: usize| {
        ((g * PER_GROUP) as f32 + (PER_GROUP + 1) as f32 * 0.5)
            * T as f32
            * (1.0 / SAMPLE_RATE as f32)
            - MIDDLE * (1.0 / SAMPLE_RATE as f32)
    };
    let (mut w, mut wt, mut wf) = (0.0f32, 0.0f32, 0.0f32);
    for (g, zg) in z.iter().enumerate() {
        let wg = zg.0 * zg.0 + zg.1 * zg.1;
        w += wg;
        wt += wg * at(g);
        wf += wg * hz(*zg);
    }
    let weighed = w > 0.0; // false for NaN as well
    if !weighed {
        return (0.0, 0.0);
    }
    let (tm, fm) = (wt / w, wf / w);
    let (mut stt, mut stf) = (0.0f32, 0.0f32);
    for (g, zg) in z.iter().enumerate() {
        let wg = zg.0 * zg.0 + zg.1 * zg.1;
        let dt = at(g) - tm;
        stt += wg * dt * dt;
        stf += wg * dt * (hz(*zg) - fm);
    }
    let slope = if stt > 0.0 { stf / stt } else { 0.0 };
    (fm - slope * tm, slope)
}

/// Least-squares sums of one segment.
#[derive(Clone, Copy, Default)]
struct Sums {
    xc: f32,
    xs: f32,
    cc: f32,
    cs: f32,
    ss: f32,
    xx: f32,
    n: usize,
}

impl Sums {
    fn add(&mut self, x: f32, c: f32, s: f32) {
        self.xc += x * c;
        self.xs += x * s;
        self.cc += c * c;
        self.cs += c * s;
        self.ss += s * s;
        self.xx += x * x;
        self.n += 1;
    }

    /// The Gram determinant, if the segment holds enough of the reference
    /// (an eighth of a full segment's energy) and is well conditioned.
    fn det(&self, segment: usize) -> Option<f32> {
        let e = self.cc + self.ss;
        let det = self.cc * self.ss - self.cs * self.cs;
        (e >= segment as f32 * 0.125 && det > 0.1 * e * e && det.is_finite()).then_some(det)
    }

    /// The amplitude (a, b) of `x ≈ a·c + b·s`.
    fn solve(&self, segment: usize) -> Option<(f32, f32)> {
        let det = self.det(segment)?;
        let a = (self.xc * self.ss - self.xs * self.cs) / det;
        let b = (self.xs * self.cc - self.xc * self.cs) / det;
        (a.is_finite() && b.is_finite()).then_some((a, b))
    }

    /// The energy of `x`'s projection on the reference, and what white
    /// noise of `x`'s mean power would give.
    fn projection(&self, segment: usize) -> Option<(f32, f32)> {
        let det = self.det(segment)?;
        let e = (self.xc * self.xc * self.ss - 2.0 * self.xc * self.xs * self.cs
            + self.xs * self.xs * self.cc)
            / det;
        let noise = 2.0 * self.xx / self.n.max(1) as f32;
        (e.is_finite() && noise.is_finite()).then_some((e, noise))
    }
}

fn run(
    audio: &mut [f32],
    decode: &Decode,
    skip: usize,
    config: &SubtractConfig,
    fixed: Fixed,
) -> Result<Subtraction, Error> {
    config.validate()?;
    let tones = encode_payload(&decode.payload);
    let synth = Synth::new(&tones, decode.freq_hz, 1.0)?;
    let nominal = Placement::slot(decode.freq_hz, decode.dt)?.start;
    let usable = Usable {
        lo: skip as i64,
        hi: audio.len() as i64,
    };
    let mut out = Subtraction {
        start: nominal,
        dt: decode.dt,
        freq_hz: decode.freq_hz,
        ..Subtraction::default()
    };
    if usable.lo >= usable.hi {
        return Ok(out);
    }

    // 1. Fine time. A start error within an eighth of a symbol keeps the
    // signal's tone changes out of the symbols' middle halves, so the
    // estimate is used as it is; a larger one is measured again at the
    // moved start.
    let mut start = fixed.start.unwrap_or(nominal);
    let sums_at = |at: i64| step_sums(&symbol_correlations(audio, usable, &synth, at), &tones);
    // The sums of the last step, taken at `taken`; the estimate `e` found
    // in sums taken at `at`.
    let mut d: StepSums = [[(0.0, 0.0); 15]; GROUPS];
    let mut taken = start;
    let mut timing: Option<(i64, f32)> = None;
    for _ in 0..TIMING_PASSES {
        d = sums_at(start);
        taken = start;
        let Some((e, frac)) = align(&d) else {
            break;
        };
        timing = Some((start, e as f32 + frac));
        if fixed.start.is_some() {
            break;
        }
        start += i64::from(e);
        if e.unsigned_abs() as usize <= T / 8 {
            break;
        }
    }
    let Some((at, e)) = timing else {
        return Ok(out);
    };
    if taken != at {
        // A repeated step found nothing at the moved start (rare): the
        // frequency is measured from the sums the estimate came from.
        d = sums_at(at);
    }
    out.start = start;
    out.dt = (at as f32 + e) * (1.0 / SAMPLE_RATE as f32) - crate::NOMINAL_START_S;

    // 2. Fine frequency.
    let (offset, drift) = fixed.freq.unwrap_or_else(|| frequency(&d, e, config.drift));
    out.freq_hz = decode.freq_hz + offset;
    out.drift_hz_per_s = drift;
    let correction = Correction::new(offset, drift);

    // 3. Amplitude per segment.
    let seg = config.segment;
    let segments = WAVEFORM_SAMPLES.div_ceil(seg);
    let mut amp = [(0.0f32, 0.0f32); MAX_SEGMENTS];
    let mut first_valid: Option<usize> = None;
    let end = usable.range(start, 0, WAVEFORM_SAMPLES).1;
    {
        let mut corr = correction;
        let mut sums = Sums::default();
        blocks(
            &synth,
            end,
            |_| true,
            |j0, phase, env| {
                let n = phase.len();
                let (a, b) = usable.range(start, j0, n);
                let (a, b) = (a - j0, b - j0);
                if a == b {
                    corr.skip(n);
                } else {
                    corr.skip(a);
                    let xs = &audio[(start + (j0 + a) as i64) as usize..][..b - a];
                    for ((&p, &w), &x) in phase[a..b].iter().zip(&env[a..b]).zip(xs) {
                        let q = p.wrapping_add(corr.next());
                        sums.add(input(x), w * cos_turns(q), w * sin_turns(q));
                    }
                    corr.skip(n - b);
                }
                let next = j0 + n;
                if next % seg == 0 || next >= end {
                    // A segment without an estimate takes the previous
                    // one's (those before the first estimate are filled
                    // below).
                    let k = j0 / seg;
                    match sums.solve(seg.min(WAVEFORM_SAMPLES - k * seg)) {
                        Some(ab) => {
                            amp[k] = ab;
                            first_valid.get_or_insert(k);
                        }
                        None if k > 0 => amp[k] = amp[k - 1],
                        None => {}
                    }
                    sums = Sums::default();
                }
            },
        );
    }
    // Segments before the first estimate take it; those after the usable
    // audio (never visited) the last one.
    let Some(first_valid) = first_valid else {
        return Ok(out);
    };
    for k in 0..first_valid {
        amp[k] = amp[first_valid];
    }
    for k in end.div_ceil(seg)..segments {
        amp[k] = amp[k - 1];
    }

    // 4. Subtraction, with the residue's projection per segment. The
    // amplitude is linear between segment centres (whole samples, since
    // segments are whole blocks): amp[k] + (j − centre k)·slope.
    let centre = |k: usize| k * seg + seg.min(WAVEFORM_SAMPLES - k * seg) / 2;
    let line = |k: usize| -> (usize, (f32, f32), (f32, f32)) {
        if k + 1 < segments {
            let span = 1.0 / (centre(k + 1) - centre(k)) as f32;
            let slope = (
                (amp[k + 1].0 - amp[k].0) * span,
                (amp[k + 1].1 - amp[k].1) * span,
            );
            (centre(k + 1), amp[k], slope)
        } else {
            (usize::MAX, amp[k], (0.0, 0.0))
        }
    };
    // Before the first centre the amplitude is constant; `k` is then the
    // interval's left centre once inside one.
    let mut k: Option<usize> = None;
    let (mut from, mut base, mut slope, mut until) = (0, amp[0], (0.0f32, 0.0f32), centre(0));
    let mut corr = correction;
    let mut sums = Sums::default();
    let mut energy = 0.0f32;
    let (mut residue, mut residue_noise) = (0.0f32, 0.0f32);
    let mut changed = 0usize;
    blocks(
        &synth,
        end,
        |_| true,
        |j0, phase, env| {
            let n = phase.len();
            let (a, b) = usable.range(start, j0, n);
            let (a, b) = (a - j0, b - j0);
            if a == b {
                corr.skip(n);
            } else {
                corr.skip(a);
                let xs = &mut audio[(start + (j0 + a) as i64) as usize..][..b - a];
                for (i, ((&p, &w), x)) in phase[a..b].iter().zip(&env[a..b]).zip(xs).enumerate() {
                    let j = j0 + a + i;
                    while j >= until {
                        let next = k.map_or(0, |k| k + 1);
                        k = Some(next);
                        let (end_of, v, sl) = line(next);
                        (from, base, slope, until) = (centre(next), v, sl, end_of);
                    }
                    let t = if j > from { (j - from) as f32 } else { 0.0 };
                    let (ra, rb) = (base.0 + t * slope.0, base.1 + t * slope.1);
                    let q = p.wrapping_add(corr.next());
                    let (rc, rs) = (w * cos_turns(q), w * sin_turns(q));
                    let est = ra * rc + rb * rs;
                    *x = input(*x) - est;
                    energy += est * est;
                    sums.add(*x, rc, rs);
                }
                changed += b - a;
                corr.skip(n - b);
            }
            let next = j0 + n;
            if next % seg == 0 || next >= end {
                let s = j0 / seg;
                if let Some((e, noise)) = sums.projection(seg.min(WAVEFORM_SAMPLES - s * seg)) {
                    residue += e;
                    residue_noise += noise;
                }
                sums = Sums::default();
            }
        },
    );
    out.samples = changed;
    out.energy = energy;
    out.residue = residue;
    out.residue_noise = residue_noise;
    Ok(out)
}

/// The frequency correction's phase, sample by sample: an offset and a
/// linear drift (zero at the middle), as a quadratic phase in integer
/// arithmetic (2⁻⁶⁴ turn units), so it is exact and the same everywhere.
#[derive(Clone, Copy)]
struct Correction {
    /// Phase before the next sample.
    phase: u64,
    /// Phase step to the next sample, and its change per sample.
    step: i64,
    accel: i64,
}

impl Correction {
    fn new(offset_hz: f32, drift_hz_per_s: f32) -> Self {
        const UNITS_PER_HZ: f32 = TURNS_PER_HZ * TURN;
        let per_sample = drift_hz_per_s * (1.0 / SAMPLE_RATE as f32);
        Correction {
            phase: 0,
            step: ((offset_hz - per_sample * MIDDLE) * UNITS_PER_HZ) as i64,
            accel: (per_sample * UNITS_PER_HZ) as i64,
        }
    }

    /// The phase at this sample (2³² = one turn); moves to the next.
    #[inline]
    fn next(&mut self) -> u32 {
        let p = (self.phase >> 32) as u32;
        self.phase = self.phase.wrapping_add(self.step as u64);
        self.step = self.step.wrapping_add(self.accel);
        p
    }

    /// Moves over `n` samples.
    fn skip(&mut self, n: usize) {
        for _ in 0..n {
            self.next();
        }
    }
}

#[cfg(test)]
mod tests;
