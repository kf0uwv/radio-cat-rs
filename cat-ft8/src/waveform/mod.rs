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

// The current symbol's pulse is never in the zero prefix, so the loop in
// `Synth::run` reads it without a check.
const _: () = assert!(PULSE_FIRST <= SAMPLES_PER_SYMBOL);
// At most two pulses overlap any sample: the previous symbol's run
// (s ≤ ns − PULSE_FIRST) and the next symbol's (s ≥ PULSE_FIRST) are
// disjoint. (Strict: with equality one sample would need all three.)
const _: () = assert!(2 * PULSE_FIRST > SAMPLES_PER_SYMBOL);

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
/// and sine tables are static. `Clone` forks a generator at its current
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
        let mut k = 0;
        self.run(out.len(), |phase, env| {
            out[k] = amp * env * sin_turns(phase);
            k += 1;
        })
    }

    /// Adds the next samples into `out` (mixing with what is there) and
    /// returns how many. Into a zeroed buffer this equals [`fill`](Self::fill)
    /// except in the sign of zero: `0.0 + (−0.0)` is `+0.0`.
    pub fn mix(&mut self, out: &mut [f32]) -> usize {
        let amp = self.amplitude;
        let mut k = 0;
        self.run(out.len(), |phase, env| {
            out[k] += amp * env * sin_turns(phase);
            k += 1;
        })
    }

    /// The next samples' carrier phase (one turn = 2³², starting at 0 on
    /// the first sample) and envelope, without the sine. This is the same
    /// generator as [`fill`](Self::fill), for building a reference signal
    /// (for example `cos_turns`/`sin_turns` of it, to subtract a decoded
    /// signal). Returns how many; at most the shorter slice's length.
    pub fn fill_phase(&mut self, phase: &mut [u32], envelope: &mut [f32]) -> usize {
        let n = phase.len().min(envelope.len());
        let mut k = 0;
        self.run(n, |p, env| {
            phase[k] = p;
            envelope[k] = env;
            k += 1;
        })
    }

    /// Advances by up to `n` samples without producing them (phase still
    /// advances); returns how many.
    pub fn skip(&mut self, n: usize) -> usize {
        self.run(n, |_, _| {})
    }

    /// The core loop: symbol by symbol, sample by sample, calling `emit`
    /// with each sample's phase and envelope, then advancing the phase.
    fn run<F: FnMut(u32, f32)>(&mut self, n: usize, mut emit: F) -> usize {
        let ns = SAMPLES_PER_SYMBOL;
        let end = self.pos + n.min(self.remaining());
        let last = SYMBOL_COUNT - 1;
        let tone = |m: usize| f32::from(self.symbols[m.min(last)]);
        let mut pos = self.pos;
        let mut phase = self.phase;
        while pos < end {
            let sym = pos / ns;
            let (prev, cur, next) = (tone(sym.saturating_sub(1)), tone(sym), tone(sym + 1));
            let s_start = pos - sym * ns;
            let s_end = ns.min(s_start + (end - pos));
            for s in s_start..s_end {
                // Symbol m's pulse covers samples [(m−1)T, (m+2)T): this
                // sample gets the previous symbol's pulse at 2ns + s, the
                // current one's at ns + s and the next one's at s (mirrored
                // about 2880). The previous symbol's term is exactly zero
                // after sample ns − PULSE_FIRST and the next symbol's before
                // PULSE_FIRST; those terms are skipped, and at most one of
                // them applies (the const assert above). Adding a +0.0 term
                // to these non-negative sums changes nothing, so this is
                // bit-identical to summing all three. (Splitting the symbol
                // into three explicit runs reads more clearly but measured
                // 4–6% slower, so the per-sample checks stay.)
                let mut dev = cur * PULSE_HALF[(ns + s).min(2 * ns - s) - PULSE_FIRST];
                if s + PULSE_FIRST <= ns {
                    dev += prev * PULSE_HALF[ns - s - PULSE_FIRST]; // `+` is commutative in IEEE-754
                }
                if s >= PULSE_FIRST {
                    dev += next * PULSE_HALF[s - PULSE_FIRST];
                }
                let freq = self.base_hz + TONE_SPACING_HZ * dev;
                let j = sym * ns + s;
                emit(phase, envelope(j));
                phase = phase.wrapping_add((freq * TURNS_PER_HZ) as u32);
            }
            pos += s_end - s_start;
        }
        let produced = pos - self.pos;
        self.pos = pos;
        self.phase = phase;
        produced
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
