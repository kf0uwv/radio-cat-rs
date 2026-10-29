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

//! Continuous-phase GFSK synthesis (QEX 2020 §5), streamed into caller
//! buffers of any size.
//!
//! The instantaneous frequency is `f0 + Δf · Σ bₙ g(t − nT)`, where `bₙ`
//! is the tone index of symbol `n`, `Δf = 6.25 Hz`, `T = 0.160 s`, and `g`
//! is the rectangular pulse smoothed by a Gaussian filter with `BT = 2`
//! (Equation 3, scaled by `T` so that a constant tone gives exactly `bΔf`):
//!
//! `g(t) = ½ [erf(k·BT·(t/T + ½)) − erf(k·BT·(t/T − ½))]`, `k = π√(2/ln 2)`.
//!
//! `g` is negligible beyond ±1.5 T, and in `f32` it is exactly zero more
//! than about 0.87 T from its centre, so each sample sums at most two
//! pulses: the current symbol's and one neighbour's. Before the first symbol and after
//! the last, the tone sequence is extended with the first and last tone so
//! the edges stay on-tone; the paper does not say what lies outside the
//! transmission, and this follows `ft8_lib`. The envelope rises over the
//! first T/8 (240 samples, 20 ms) with the raised cosine `½(1 − cos(8πt/T))`
//! of QEX §5 and falls with its mirror image over the last T/8.
//!
//! # Arithmetic
//!
//! Only `f32` `+ − ×` and integer operations run here; no transcendental
//! function and no `f64`. The pulse `g` and a quarter-wave sine are
//! tables in static data (`tables.rs`, generated and checked bit-for-bit
//! by a test against `libm`). Phase is a 32-bit fixed-point accumulator
//! (one turn = 2³²), so it is continuous and exact over the transmission
//! and across blocks. The sine is read from the table by the top phase bits
//! and linearly interpolated with the next 20 bits (error below 4 × 10⁻⁷;
//! measured 3.5 × 10⁻⁷).
//! Rust never fuses `×` and `+` into FMA, and IEEE-754 binary32 arithmetic
//! with round-to-nearest gives every operation one exact result, so the
//! same symbols, frequency, amplitude and position give bit-identical
//! samples on every platform that provides it: every target with SSE2 or a
//! hardware single-precision FPU, and soft-float targets. (x87-only
//! targets such as i586, which round through extended precision, are
//! excluded.)
//!
//! # Blocks
//!
//! [`Synth`] keeps its phase and position, so a waveform can be rendered
//! in blocks of any size with the same result as in one call:
//!
//! ```
//! use cat_ft8::{encode, message::NoHashes, waveform::Synth, WAVEFORM_SAMPLES};
//!
//! let symbols = encode("CQ K1ABC FN42", &mut NoHashes).unwrap().symbols;
//! let mut synth = Synth::new(&symbols, 1500.0, 0.5).unwrap();
//! let mut block = [0.0f32; 480]; // e.g. one 40 ms audio buffer
//! let mut total = 0;
//! loop {
//!     let n = synth.fill(&mut block);
//!     // ... play or send block[..n] ...
//!     total += n;
//!     if n < block.len() {
//!         break;
//!     }
//! }
//! assert_eq!(total, WAVEFORM_SAMPLES);
//! ```

mod tables;

use crate::{
    Error, NOMINAL_START_S, SAMPLES_PER_SYMBOL, SAMPLE_RATE, SYMBOL_COUNT, TONE_SPACING_HZ,
    WAVEFORM_SAMPLES,
};
use tables::{PULSE_FIRST, PULSE_HALF, QUARTER_SINE};

/// Gaussian filter bandwidth-time product for FT8.
pub const BT: f32 = 2.0;
/// Length of the pulse over three symbols, samples.
pub const PULSE_SAMPLES: usize = 3 * SAMPLES_PER_SYMBOL;
/// Ramp length at each end (T/8).
pub const RAMP_SAMPLES: usize = SAMPLES_PER_SYMBOL / 8;
/// Entries in the stored half pulse (the pulse is symmetric about its
/// centre, sample 2880 of 5760).
pub const PULSE_HALF_LEN: usize = PULSE_SAMPLES / 2 + 1;
/// Quarter-wave sine table: 1024 segments plus the end point.
pub const SINE_TABLE_LEN: usize = 1025;
/// Bytes of static table data the synthesizer uses. The pulse is stored
/// from its first non-zero sample: the 1 215 samples before it are exactly
/// zero in `f32` and are not stored.
pub const TABLE_BYTES: usize =
    (PULSE_HALF_LEN - tables::PULSE_FIRST + SINE_TABLE_LEN) * core::mem::size_of::<f32>();

/// The size of the actual table statics (checked against [`TABLE_BYTES`]).
#[cfg(test)]
pub(crate) fn table_bytes_of_statics() -> usize {
    core::mem::size_of_val(&PULSE_HALF) + core::mem::size_of_val(&QUARTER_SINE)
}

/// One turn of phase, 2³², as `f32` (exactly representable).
const TURN: f32 = 4_294_967_296.0;
/// Phase increment per hertz per sample.
const TURNS_PER_HZ: f32 = TURN / SAMPLE_RATE as f32;

// The current symbol's pulse is never in the zero prefix, so
// `Synth::increments` reads it in every run.
const _: () = assert!(PULSE_FIRST <= SAMPLES_PER_SYMBOL);
// At most two pulses overlap any sample: the previous symbol's run
// (s ≤ ns − PULSE_FIRST) and the next symbol's (s ≥ PULSE_FIRST) are
// disjoint. (Strict: with equality one sample would need all three.)
const _: () = assert!(2 * PULSE_FIRST > SAMPLES_PER_SYMBOL);
// The ramps lie within the first and last symbols.
const _: () = assert!(RAMP_SAMPLES <= SAMPLES_PER_SYMBOL);
/// Within a symbol, the previous symbol's pulse is non-zero before this
/// sample (`s + PULSE_FIRST <= ns`) ...
const PREV_END: usize = SAMPLES_PER_SYMBOL + 1 - PULSE_FIRST;
/// ... the current one's is read mirrored from this sample on
/// (`ns + s > 2·ns − s`), and the next one's is non-zero from
/// `PULSE_FIRST` on: the runs of `Synth::increments`.
const MIRROR: usize = SAMPLES_PER_SYMBOL / 2 + 1;
const _: () = assert!(PREV_END <= MIRROR && MIRROR <= PULSE_FIRST);

/// The pulse `g` at sample `i` of `0..PULSE_SAMPLES` (centre at 2880).
#[cfg(test)]
#[inline]
fn pulse(i: usize) -> f32 {
    // Mirror about the centre, then skip the stored table's zero prefix.
    match i.min(PULSE_SAMPLES - i).checked_sub(PULSE_FIRST) {
        Some(j) => PULSE_HALF[j],
        None => 0.0,
    }
}

/// `sin(2π · phase / 2³²)`, from the quarter-wave table.
#[inline]
pub fn sin_turns(phase: u32) -> f32 {
    let quadrant = phase >> 30;
    let x = phase & 0x3fff_ffff;
    // Quadrants 1 and 3 read the quarter wave backwards.
    let x = if quadrant & 1 == 0 { x } else { (1 << 30) - x };
    let idx = (x >> 20) as usize; // 0..=1024
    let v = if idx >= SINE_TABLE_LEN - 1 {
        QUARTER_SINE[SINE_TABLE_LEN - 1]
    } else {
        let frac = (x & 0xf_ffff) as f32 * (1.0 / 1_048_576.0);
        let a = QUARTER_SINE[idx];
        a + (QUARTER_SINE[idx + 1] - a) * frac
    };
    if quadrant & 2 == 0 {
        v
    } else {
        -v
    }
}

/// `cos(2π · phase / 2³²)`.
#[inline]
pub fn cos_turns(phase: u32) -> f32 {
    sin_turns(phase.wrapping_add(1 << 30))
}

/// The amplitude envelope at waveform sample `j`: the raised-cosine ramp
/// at both ends, 1 elsewhere.
pub fn envelope(j: usize) -> f32 {
    let r = j.min(WAVEFORM_SAMPLES.saturating_sub(1 + j));
    if r >= RAMP_SAMPLES {
        return 1.0;
    }
    // cos(π r / 240) = cos of r/480 of a turn; exact integer phase.
    let phase = ((r as u64) << 32) / (2 * RAMP_SAMPLES as u64);
    0.5 * (1.0 - cos_turns(phase as u32))
}

/// A resumable FT8 waveform generator: phase and position persist between
/// calls, so blocks of any size give the same samples as one call.
///
/// Memory: the 79 symbols and a few scalars (about 100 bytes); the pulse
/// and sine tables are static. [`fill`](Self::fill), [`mix`](Self::mix) and
/// [`skip`](Self::skip) work through a stack chunk of 64 samples (about
/// 0.8 KB at the deepest on thumbv8m). `Clone` forks a generator at its current
/// position (for example to build a reference signal from part-way in).
#[derive(Debug, Clone)]
pub struct Synth {
    symbols: [u8; SYMBOL_COUNT],
    base_hz: f32,
    amplitude: f32,
    phase: u32,
    pos: usize,
}

impl Synth {
    /// A generator for `symbols` with tone 0 at `base_hz` and peak
    /// `amplitude`.
    ///
    /// Errors: a tone above 7, a non-finite amplitude, or a base frequency
    /// that is not finite, not positive, or puts the top tone (`7·Δf`
    /// above it) at or above half the sample rate.
    pub fn new(symbols: &[u8; SYMBOL_COUNT], base_hz: f32, amplitude: f32) -> Result<Self, Error> {
        if symbols.iter().any(|&s| s > 7) {
            return Err(Error::InvalidSymbol);
        }
        if !amplitude.is_finite() {
            return Err(Error::NonFinite);
        }
        let nyquist = SAMPLE_RATE as f32 / 2.0;
        if !base_hz.is_finite() || base_hz <= 0.0 || base_hz + 7.0 * TONE_SPACING_HZ >= nyquist {
            return Err(Error::FrequencyOutOfRange);
        }
        Ok(Synth {
            symbols: *symbols,
            base_hz,
            amplitude,
            phase: 0,
            pos: 0,
        })
    }

    /// Samples produced so far (index of the next sample).
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Samples still to come.
    pub fn remaining(&self) -> usize {
        WAVEFORM_SAMPLES - self.pos
    }

    /// Writes the next samples into `out` (overwriting) and returns how
    /// many; fewer than `out.len()` only at the end of the waveform.
    pub fn fill(&mut self, out: &mut [f32]) -> usize {
        let amp = self.amplitude;
        self.render(out, |y, phase, env| *y = amp * env * sin_turns(phase))
    }

    /// Adds the next samples into `out` (mixing with what is there) and
    /// returns how many. Into a zeroed buffer this equals [`fill`](Self::fill)
    /// except in the sign of zero: `0.0 + (−0.0)` is `+0.0`.
    pub fn mix(&mut self, out: &mut [f32]) -> usize {
        let amp = self.amplitude;
        self.render(out, |y, phase, env| *y += amp * env * sin_turns(phase))
    }

    /// The samples of [`fill`](Self::fill) and [`mix`](Self::mix): the
    /// phase and envelope of [`fill_phase`](Self::fill_phase) a chunk of
    /// [`CHUNK`] samples at a time (in a stack buffer), each sample then
    /// written by `put(sample, phase, envelope)`.
    #[inline(always)]
    fn render(&mut self, out: &mut [f32], put: impl Fn(&mut f32, u32, f32)) -> usize {
        let mut phase = [0u32; CHUNK];
        let mut env = [0.0f32; CHUNK];
        let mut done = 0;
        for block in out.chunks_mut(CHUNK) {
            let k = block.len();
            let n = self.fill_phase(&mut phase[..k], &mut env[..k]);
            for ((y, &p), &e) in block[..n].iter_mut().zip(&phase[..n]).zip(&env[..n]) {
                put(y, p, e);
            }
            done += n;
            if n < k {
                break;
            }
        }
        done
    }

    /// The next samples' carrier phase (one turn = 2³², starting at 0 on
    /// the first sample) and envelope, without the sine. This is the same
    /// generator as [`fill`](Self::fill), for building a reference signal
    /// (for example `cos_turns`/`sin_turns` of it, to subtract a decoded
    /// signal). Returns how many; at most the shorter slice's length.
    pub fn fill_phase(&mut self, phase: &mut [u32], envelope: &mut [f32]) -> usize {
        let n = phase.len().min(envelope.len()).min(self.remaining());
        let (phase, envelope) = (&mut phase[..n], &mut envelope[..n]);
        let pos = self.pos;
        // The increments first, then their running sum (the phase before
        // each sample).
        self.increments(pos, phase);
        let mut p = self.phase;
        for x in phase.iter_mut() {
            let step = *x;
            *x = p;
            p = p.wrapping_add(step);
        }
        self.phase = p;
        self.pos = pos + n;
        // `envelope(j)` on the ramps, exactly 1.0 elsewhere.
        envelope.fill(1.0);
        let ramps = [
            0..RAMP_SAMPLES,
            WAVEFORM_SAMPLES - RAMP_SAMPLES..WAVEFORM_SAMPLES,
        ];
        for r in ramps {
            for j in r.start.max(pos)..r.end.min(pos + n) {
                envelope[j - pos] = self::envelope(j);
            }
        }
        n
    }

    /// [`skip`](Self::skip) over up to `buf.len()` samples, with `buf` as
    /// working storage (its contents are overwritten): the same phase,
    /// through the generator of [`fill_phase`](Self::fill_phase).
    pub(crate) fn skip_with(&mut self, buf: &mut [u32]) -> usize {
        let n = buf.len().min(self.remaining());
        let buf = &mut buf[..n];
        self.increments(self.pos, buf);
        // Integer addition modulo 2³²: any order gives the same sum.
        let step = buf.iter().fold(0u32, |a, &b| a.wrapping_add(b));
        self.phase = self.phase.wrapping_add(step);
        self.pos += n;
        n
    }

    /// The phase increments of waveform samples `pos..pos + out.len()`
    /// (within the waveform) into `out`: [`increment`] of
    /// `(base + Δf · dev) · TURNS_PER_HZ` at each sample.
    ///
    /// Symbol m's pulse covers samples [(m−1)T, (m+2)T), so sample s of
    /// symbol m gets the current symbol's pulse at NS + s and the previous
    /// and next symbols' at 2·NS + s and s (the stored half read mirrored
    /// about its centre). The previous symbol's term is exactly zero from
    /// `PREV_END` on and the next one's before `PULSE_FIRST`, so at most
    /// one of them applies (the const asserts above) and `dev` is
    /// `cur · g` plus that term, if any: adding a +0.0 term to these
    /// non-negative sums would change nothing, so this is bit-identical to
    /// summing all three. The loops run over each symbol's parts where the
    /// same pulses overlap (the previous symbol's and the current one's
    /// rising half, the current one's rising half alone, its falling half
    /// alone, its falling half and the next symbol's), with no per-sample
    /// branch or bounds check, so they vectorise.
    #[inline]
    fn increments(&self, pos: usize, out: &mut [u32]) {
        const NS: usize = SAMPLES_PER_SYMBOL;
        const PF: usize = PULSE_FIRST;
        let last = SYMBOL_COUNT - 1;
        let tone = |m: usize| f32::from(self.symbols[m.min(last)]);
        let base = self.base_hz;
        let step = |dev: f32| increment((base + TONE_SPACING_HZ * dev) * TURNS_PER_HZ);
        let pulse = &PULSE_HALF[..];
        let end = pos + out.len();
        let mut p = pos;
        let mut out = out;
        while p < end {
            let sym = p / NS;
            let (prev, cur, next) = (tone(sym.saturating_sub(1)), tone(sym), tone(sym + 1));
            let s0 = p - sym * NS;
            let s1 = NS.min(s0 + (end - p));
            let cut = |x: usize| x.clamp(s0, s1);
            let (a, b, c) = (cut(PREV_END), cut(MIRROR), cut(PF));
            let (o1, rest) = core::mem::take(&mut out).split_at_mut(a - s0);
            let (o2, rest) = rest.split_at_mut(b - a);
            let (o3, rest) = rest.split_at_mut(c - b);
            let (o4, rest) = rest.split_at_mut(s1 - c);
            out = rest;
            // s in s0..a: the current pulse at NS + s, the previous one's
            // at NS − s.
            if a > s0 {
                let g = &pulse[NS + s0 - PF..NS + a - PF];
                let h = &pulse[NS + 1 - a - PF..NS + 1 - s0 - PF];
                for ((y, &g), &h) in o1.iter_mut().zip(g).zip(h.iter().rev()) {
                    *y = step(cur * g + prev * h);
                }
            }
            // s in a..b: the current pulse at NS + s alone.
            if b > a {
                for (y, &g) in o2.iter_mut().zip(&pulse[NS + a - PF..NS + b - PF]) {
                    *y = step(cur * g);
                }
            }
            // s in b..c: the current pulse mirrored, at 2·NS − s, alone.
            if c > b {
                let g = &pulse[2 * NS + 1 - c - PF..2 * NS + 1 - b - PF];
                for (y, &g) in o3.iter_mut().zip(g.iter().rev()) {
                    *y = step(cur * g);
                }
            }
            // s in c..s1: the current pulse mirrored and the next one's at s.
            if s1 > c {
                let g = &pulse[2 * NS + 1 - s1 - PF..2 * NS + 1 - c - PF];
                let h = &pulse[c - PF..s1 - PF];
                for ((y, &g), &h) in o4.iter_mut().zip(g.iter().rev()).zip(h) {
                    *y = step(cur * g + next * h);
                }
            }
            p += s1 - s0;
        }
    }

    /// Advances by up to `n` samples without producing them (phase still
    /// advances); returns how many.
    pub fn skip(&mut self, n: usize) -> usize {
        let mut buf = [0u32; CHUNK];
        let n = n.min(self.remaining());
        let mut left = n;
        while left > 0 {
            left -= self.skip_with(&mut buf[..left.min(CHUNK)]);
        }
        n
    }
}

/// Samples per chunk in [`Synth::fill`], [`Synth::mix`] and
/// [`Synth::skip`]: their stack buffers take 8 bytes per sample in `fill`
/// and `mix` (phase and envelope), 4 in `skip`.
const CHUNK: usize = 64;

/// A phase increment `x`, in [0, 2³¹), as the `u32` it truncates to (as
/// `x as i64 as u32` on every target). `x` is positive because `freq` is
/// positive and below half the sample rate (checked in [`Synth::new`]).
///
/// On x86-64, where the saturating `as` conversion of a vectorised loop
/// goes a lane at a time (SSE2), without a float-to-integer conversion:
/// the top 15 bits are ⌊x / 2¹⁶⌋ and the low 16 bits ⌊x − 2¹⁶·⌊x / 2¹⁶⌋⌋,
/// each step exact (the scaling by 2¹⁶ either way, and the difference, a
/// multiple of x's spacing below 2¹⁶). Elsewhere through `i32`: one
/// instruction on AArch64 (lanes at once) and on Cortex-M, where `i64`
/// would be a library call.
#[inline(always)]
fn increment(x: f32) -> u32 {
    // The `as` forms agree only below 2³¹: at 2³¹ `as i32` saturates to
    // 2³¹ − 1 while `as i64 as u32` gives 2³¹. The margin: `new` keeps the
    // top tone below 6 000 Hz and `TURNS_PER_HZ` is 357 913.9375 (2³² /
    // 12 000 rounded down), so the largest increment, at the largest base
    // accepted and any tones, is 2³¹ − 256 (tested), two `f32` steps
    // below the limit.
    debug_assert!(
        (0.0..2_147_483_648.0).contains(&x),
        "phase increment {x} outside [0, 2^31)"
    );
    #[cfg(target_arch = "x86_64")]
    {
        use crate::decode::math::floor_small;
        let hi = floor_small(x * (1.0 / 65_536.0));
        let lo = floor_small(x - hi as f32 * 65_536.0);
        ((hi as u32) << 16).wrapping_add(lo as u32)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        x as i32 as u32
    }
}

/// Where and how loud to put a whole waveform in a buffer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Frequency of tone 0, Hz. Tone 7 is 43.75 Hz above it.
    pub base_hz: f32,
    /// Index in the output buffer of the waveform's first sample. May be
    /// negative or run past the end; samples outside the buffer are
    /// dropped (phase still advances through them).
    pub start: i64,
    /// Peak amplitude.
    pub amplitude: f32,
}

impl Placement {
    /// A signal in a 15 s slot buffer that starts at the nominal 0.5 s plus
    /// `dt` seconds (rounded to the nearest sample), at amplitude 1.
    /// [`Error::NonFinite`] if `dt` is NaN or infinite.
    pub fn slot(base_hz: f32, dt: f32) -> Result<Self, Error> {
        if !dt.is_finite() {
            return Err(Error::NonFinite);
        }
        let start = (NOMINAL_START_S + dt) * SAMPLE_RATE as f32;
        Ok(Placement {
            base_hz,
            start: round_to_i64(start),
            amplitude: 1.0,
        })
    }
}

/// Rounds half away from zero; saturates beyond the `i64` range.
fn round_to_i64(x: f32) -> i64 {
    let t = x as i64; // truncates toward zero, saturating
    let frac = x - t as f32;
    if frac >= 0.5 {
        t.saturating_add(1)
    } else if frac <= -0.5 {
        t.saturating_sub(1)
    } else {
        t
    }
}

/// Adds the whole waveform of `symbols` into `out` at `placement`.
pub fn synthesize(
    symbols: &[u8; SYMBOL_COUNT],
    placement: &Placement,
    out: &mut [f32],
) -> Result<(), Error> {
    let mut synth = Synth::new(symbols, placement.base_hz, placement.amplitude)?;
    let first = if placement.start < 0 {
        let before = usize::try_from(placement.start.unsigned_abs()).unwrap_or(usize::MAX);
        synth.skip(before);
        0
    } else {
        usize::try_from(placement.start).unwrap_or(usize::MAX)
    };
    if let Some(dst) = out.get_mut(first..) {
        synth.mix(dst);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
