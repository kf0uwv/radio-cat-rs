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

//! The decoder's front end: slot framing and a block-fed log-magnitude
//! spectrogram (ADR 0024, amendment "single-pass decoder").
//!
//! - **Rows.** Row k is the Hann-windowed spectrum of the 3 840 samples
//!   [240 + 960k, 240 + 960k + 3 840) of the slot, k = 0..184: two rows per
//!   symbol. The 240-sample origin puts the symbol centres of a signal
//!   that starts at the nominal 0.5 s on rows 5, 7, 9, …
//! - **Bins.** 3.125 Hz apart (two per tone). A [`Band`] names the base
//!   (tone 0) frequencies searched; the stored spectrum adds the 43.75 Hz
//!   above the top base frequency and one guard bin.
//! - **Levels.** Power in dB relative to a full-scale sine centred in a bin
//!   (0 dB), from the in-crate `log2`, stored as 8-bit levels in 0.5 dB
//!   steps over −127.5…0 dB.
//! - **Feeding.** [`Spectrogram::feed`] accepts blocks of any size; the
//!   rows do not depend on the block sizes. Samples beyond the slot's
//!   180 000 are not taken. Non-finite samples are replaced by 0.0 and
//!   finite samples beyond ±2¹⁶ are clamped to it, both counted;
//!   [`Spectrogram::skip`] advances over a gap as zeros.
//!
//! # Input level
//!
//! Full scale is ±1.0 (`feed`) or ±32 768 (`feed_i16`). No minimum level
//! is enforced, but the fixed range −127.5…0 dB has two consequences:
//!
//! - Levels start to floor when the noise in a 3.125 Hz bin is below about
//!   −105 dB, which is about −80 dBFS RMS for white noise. In practice the
//!   audio codec's own noise floor is the real lower limit.
//! - Nothing reaches the top unless a single tone is near full scale.
//!
//! A typical operating range is −40…−15 dBFS RMS with peaks at or below
//! −6 dBFS. [`Spectrogram::floored_levels`] and
//! [`Spectrogram::top_levels`] count levels at either end.
//!
//! The caller owns every buffer: the [`FrontEnd`] (input window and FFT
//! work, 30 720 bytes, const-constructible) and the level slice of
//! [`Band::spectrogram_len`] entries. The window holds samples from one
//! feed to the next. The FFT work is free between rows, and the decoder's
//! sync search uses it then.

use super::fft::{power_bins, spectrum, spectrum_of_quarters, FftBuffer, BINS, ONE_LANE, WINDOW};
use super::math::{log2_power, POWER_FLOOR, SCALAR_FORMS};
use super::tables::LEVEL_STEPS;
use crate::{Error, SLOT_SAMPLES};

/// Hop between rows: half a symbol.
pub const HOP: usize = 960;
/// Slot sample at which row 0 starts.
pub const ROW_ORIGIN: usize = 240;
/// Rows in a 15 s slot.
pub const ROWS: usize = (SLOT_SAMPLES - ROW_ORIGIN - WINDOW) / HOP + 1;
/// Width of a frequency bin, Hz.
pub const BIN_HZ: f32 = 3.125;
/// Largest input magnitude (2¹⁶): finite samples beyond it are clamped to
/// it and counted, so no power overflows.
pub const INPUT_LIMIT: f32 = 65_536.0;
/// Stored bins beyond the top base bin: tone 7 is 14 bins above tone 0,
/// plus one guard bin.
pub const BINS_ABOVE_BAND: usize = 15;

const _: () = assert!(ROWS == 184);

/// 10 · log10(2): dB per unit of log2 power.
const DB_PER_LOG2: f32 = 3.010_3;

/// Rounds `x` (finite) to the nearest integer in `0..=max`, halves up.
#[inline]
fn quantise(x: f32, max: u32) -> u32 {
    let y = x + 0.5;
    if y <= 0.0 {
        0
    } else if y >= max as f32 {
        max
    } else {
        y as u32 // truncation of a positive value: floor
    }
}

/// The spectrogram level of a `log2` power value: 0.5 dB steps, 255 at
/// 0 dB (a full-scale sine centred in a bin), 0 at the floor.
#[inline]
pub(crate) fn level_of(l2: f32) -> u8 {
    quantise(l2 * (DB_PER_LOG2 * 2.0) + 255.0, 255) as u8
}

/// Levels per unit of a power's bits, scaled by 2⁴⁰ for
/// [`level_by_steps`]' guess: 2⁴⁰ · 2 · `DB_PER_LOG2` / 2²³ (6.0206 levels
/// a unit of log2, 2²³ units of the bits an octave), rounded. A test
/// recomputes it.
const LEVEL_GUESS_SCALE: i64 = 789_132;
/// The guess's offset, scaled likewise: (255.5 − 127 · 2 · `DB_PER_LOG2` −
/// 0.02) · 2⁴⁰, rounded. 127 is the exponent's bias, 255.5 the level of
/// 0 dB and the half that rounds it; the 0.02 keeps the guess below the
/// level where a power's bits give its log2 exactly (at a power of two).
/// A test recomputes it.
const LEVEL_GUESS_OFFSET: i64 = -559_801_149_968_876;

/// The spectrogram level of a power value `p` (any `f32`),
/// `level_of(log2_power(p))`, by a guess from the power's bits and one
/// compare with [`LEVEL_STEPS`]: for the targets without a vector unit
/// ([`levels_by_steps_out_of_line`]), where the logarithm's division and
/// polynomial were most of the front end's work after the transform
/// (findings O30).
///
/// The guess. After the same floor and cap as [`log2_power`], `bits` is a
/// positive normal `f32`'s, exponent e and mantissa fraction m in [0, 1):
/// (bits − 127 · 2²³) / 2²³ is e + m, and log2 p is e + log2(1 + m). The
/// line in the bits is below log2 by log2(1 + m) − m: never negative, 0 at
/// m = 0, at most 0.0861 (at m = 1/ln 2 − 1), so 0 to 0.518 levels (×
/// 6.0206). The offset takes 0.02 of a level more, so the line is the
/// value [`level_of`] truncates (the level unrounded, plus a half) less
/// 0.02 to 0.54 of a level. Its floor, the guess, is never above the level
/// and at most one below it (the clamps to 0..=255 on both sides keep
/// that). The 0.02 covers what is not exact on the logarithm's side
/// (its polynomial, the `f32` roundings in [`level_of`]) where the line is
/// exact, at a power of two; the constants' own rounding is 2⁻⁴⁰ of a level.
///
/// The step. `LEVEL_STEPS[guess]` is the bits of the least power whose
/// level is above the guess, so one compare picks the guess or one more.
/// That relies on the level rising with the power (and on positive `f32`s'
/// bits being in the order of their values): true of the logarithm, but
/// [`log2_power`] is a polynomial and roundings, so it is checked, not
/// derived. The exhaustive test (`level_steps_give_every_level_everywhere`,
/// every `f32`) checks the result, and with it both the bound and the
/// order.
#[inline(always)]
pub(crate) fn level_by_steps(p: f32) -> u8 {
    // Not `clamp`, which keeps a NaN: a quiet NaN goes to the floor, as in
    // `log2_power`. On the Cortex-M33 (`vmaxnm`, `vminnm`) a signalling
    // NaN comes out of `max` quieted, not floored, and `min` caps it to
    // `f32::MAX` (level 255, as `log2_power` gives there). The cap keeps
    // every `bits` below `LEVEL_STEPS[255]` (`u32::MAX`), so a guess of 255
    // is never stepped past: without it, 0xffbf_ffff quieted is
    // 0xffff_ffff, which would step to 256 and wrap to level 0.
    #[allow(clippy::manual_clamp)]
    let bits = p.max(POWER_FLOOR).min(f32::MAX).to_bits();
    debug_assert!(bits < LEVEL_STEPS[255]);
    let line = (i64::from(bits) * LEVEL_GUESS_SCALE + LEVEL_GUESS_OFFSET) >> 40;
    let guess = (line as i32).clamp(0, 255) as u32;
    (guess + u32::from(bits >= LEVEL_STEPS[guess as usize])) as u8
}

/// The base (tone 0) frequencies searched, as 3.125 Hz bins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    lo_bin: u16,
    hi_bin: u16,
}

impl Band {
    /// 100–3100 Hz: 961 base bins, 976 stored.
    pub const DEFAULT: Band = Band {
        lo_bin: 32,
        hi_bin: 992,
    };
    /// 200–2900 Hz: 865 base bins, 880 stored (the small configuration).
    pub const SMALL: Band = Band {
        lo_bin: 64,
        hi_bin: 928,
    };

    /// A band of base frequencies `lo_hz..=hi_hz`. Both must be whole
    /// multiples of 3.125 Hz (for example 100 or 2 900), `lo_hz > 0`,
    /// `lo_hz < hi_hz`, and the stored spectrum must end below Nyquist.
    pub fn from_hz(lo_hz: u32, hi_hz: u32) -> Result<Band, Error> {
        // f / 3.125 = 8 f / 25
        let bin = |hz: u32| (hz.checked_mul(8)?.checked_rem(25) == Some(0)).then(|| hz * 8 / 25);
        let (Some(lo), Some(hi)) = (bin(lo_hz), bin(hi_hz)) else {
            return Err(Error::InvalidBand);
        };
        if lo == 0 || lo >= hi || hi as usize + BINS_ABOVE_BAND >= BINS {
            return Err(Error::InvalidBand);
        }
        Ok(Band {
            lo_bin: lo as u16,
            hi_bin: hi as u16,
        })
    }

    /// First stored bin (the lowest base frequency), in 3.125 Hz units.
    pub const fn first_bin(&self) -> usize {
        self.lo_bin as usize
    }

    /// Number of base (tone 0) bins searched.
    pub const fn base_bins(&self) -> usize {
        (self.hi_bin - self.lo_bin) as usize + 1
    }

    /// Number of bins stored per row.
    pub const fn stored_bins(&self) -> usize {
        self.base_bins() + BINS_ABOVE_BAND
    }

    /// Level entries a spectrogram for this band needs: rows × bins.
    pub const fn spectrogram_len(&self) -> usize {
        ROWS * ((self.hi_bin - self.lo_bin) as usize + 1 + BINS_ABOVE_BAND)
    }

    /// Lowest and highest base frequency, Hz.
    pub fn range_hz(&self) -> (f32, f32) {
        (
            f32::from(self.lo_bin) * BIN_HZ,
            f32::from(self.hi_bin) * BIN_HZ,
        )
    }

    /// For tests: `base_bins` base bins from bin `lo_bin`, any count from
    /// one, below the narrowest band [`Band::from_hz`] makes (nine bins).
    /// [`Band::from_hz`] takes whole multiples of 25 Hz, eight bins, so its
    /// widths are 8k + 1, all odd.
    #[cfg(test)]
    pub(crate) fn of_bins(lo_bin: u16, base_bins: u16) -> Band {
        assert!(base_bins > 0 && lo_bin > 0);
        let hi_bin = usize::from(lo_bin) + usize::from(base_bins) - 1;
        assert!(hi_bin + BINS_ABOVE_BAND < BINS);
        Band {
            lo_bin,
            hi_bin: hi_bin as u16,
        }
    }
}

/// The front end's working buffers: one analysis window of input and the
/// FFT work area (30 720 bytes). Caller-owned and const-constructible, so
/// it can be a `static`.
#[derive(Clone)]
pub struct FrontEnd {
    window: [f32; WINDOW],
    fft: FftBuffer,
}

impl FrontEnd {
    /// Zeroed buffers.
    pub const fn new() -> Self {
        FrontEnd {
            window: [0.0; WINDOW],
            fft: FftBuffer::new(),
        }
    }
}

/// A sample as the front end takes it, counted: a non-finite value
/// becomes 0.0 (`sanitised`); a finite one beyond ±[`INPUT_LIMIT`] becomes
/// the limit (`clamped`).
#[inline]
fn classify(x: f32, sanitised: &mut usize, clamped: &mut usize) -> f32 {
    if !x.is_finite() {
        *sanitised += 1;
        0.0
    } else if x > INPUT_LIMIT {
        *clamped += 1;
        INPUT_LIMIT
    } else if x < -INPUT_LIMIT {
        *clamped += 1;
        -INPUT_LIMIT
    } else {
        x
    }
}

/// Whether every sample of `xs` is finite and within ±[`INPUT_LIMIT`]:
/// exactly the samples that [`classify`] returns unchanged and does not
/// count (and that the subtraction's `input` returns unchanged). A
/// sample's bits without the sign, as an unsigned integer, order the
/// finite magnitudes and put the infinities and NaNs above them. Their
/// bitwise or is at least the largest of them, and for audio within ±1, as
/// a slot's is, it is below the limit's: then that one pass decides.
/// Otherwise their largest decides, in a second pass. (Folds over every
/// sample rather than `all`, whose early exit keeps the loop from
/// vectorising.) Not inlined: one call per block fed and per subtraction,
/// and its loops are checked on their own (`codegen-check.sh`).
#[inline(never)]
pub(crate) fn within_limit(xs: &[f32]) -> bool {
    const MAGNITUDE: u32 = 0x7fff_ffff;
    let limit = INPUT_LIMIT.to_bits();
    let any = xs.iter().fold(0u32, |a, &x| a | x.to_bits());
    (any & MAGNITUDE) <= limit
        || xs.iter().fold(0u32, |m, &x| m.max(x.to_bits() & MAGNITUDE)) <= limit
}

/// Copies `src` into `dst` (the same length) as [`classify`] takes each
/// sample, and returns how many it sanitised and clamped. A block whose
/// samples are all finite and within ±[`INPUT_LIMIT`] ([`within_limit`]),
/// which `classify` returns unchanged, is checked as a whole and copied;
/// any other block is classified sample by sample.
fn checked_copy(src: &[f32], dst: &mut [f32]) -> (usize, usize) {
    if within_limit(src) {
        dst.copy_from_slice(src);
        return (0, 0);
    }
    let (mut sanitised, mut clamped) = (0, 0);
    for (d, &x) in dst.iter_mut().zip(src) {
        *d = classify(x, &mut sanitised, &mut clamped);
    }
    (sanitised, clamped)
}

/// The buffers' contents are not printed.
impl core::fmt::Debug for FrontEnd {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FrontEnd").finish_non_exhaustive()
    }
}

/// The band, progress and counters; the levels are not printed.
impl core::fmt::Debug for Spectrogram<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Spectrogram")
            .field("band", &self.rows.band)
            .field("samples_consumed", &self.consumed)
            .field("rows_filled", &self.rows.count)
            .field("sanitised", &self.sanitised)
            .field("clamped", &self.clamped)
            .field("skipped", &self.skipped)
            .field("floored", &self.floored)
            .field("top", &self.top)
            .finish_non_exhaustive()
    }
}

impl Default for FrontEnd {
    fn default() -> Self {
        Self::new()
    }
}

/// A spectrogram's levels and how many rows of them are filled: apart from
/// the [`FrontEnd`], so that the rows can be read while its FFT work area is
/// used for something else.
pub(crate) struct Rows<'a> {
    levels: &'a mut [u8],
    band: Band,
    /// Rows complete so far.
    count: usize,
}

impl Rows<'_> {
    /// Rows complete so far.
    pub(crate) fn len(&self) -> usize {
        self.count
    }

    /// The band.
    pub(crate) fn band(&self) -> Band {
        self.band
    }

    /// Row `r`'s levels, one per stored bin, or `None` if the row is not
    /// filled yet.
    // Must stay out of line from the M33 sync sum (+3.4%): findings O21, codegen-check `sumloop`.
    pub(crate) fn row(&self, r: usize) -> Option<&[u8]> {
        if r >= self.count {
            return None;
        }
        let width = self.band.stored_bins();
        Some(&self.levels[r * width..(r + 1) * width])
    }
}

/// A slot's spectrogram, filled block by block from 12 kHz samples. Borrows
/// the caller's [`FrontEnd`] and level slice.
pub struct Spectrogram<'a> {
    front: &'a mut FrontEnd,
    rows: Rows<'a>,
    /// Slot samples taken so far (fed or skipped).
    consumed: usize,
    /// Samples in the front end's window.
    filled: usize,
    sanitised: usize,
    clamped: usize,
    skipped: usize,
    floored: usize,
    top: usize,
}

impl<'a> Spectrogram<'a> {
    /// A spectrogram for `band` over the caller's buffers, at the start of
    /// a slot. [`Error::BufferTooSmall`] if `levels` is shorter than
    /// [`Band::spectrogram_len`].
    pub fn new(front: &'a mut FrontEnd, levels: &'a mut [u8], band: Band) -> Result<Self, Error> {
        if levels.len() < band.spectrogram_len() {
            return Err(Error::BufferTooSmall);
        }
        Ok(Spectrogram {
            front,
            rows: Rows {
                levels,
                band,
                count: 0,
            },
            consumed: 0,
            filled: 0,
            sanitised: 0,
            clamped: 0,
            skipped: 0,
            floored: 0,
            top: 0,
        })
    }

    /// Starts a new slot: no rows, nothing consumed. (Earlier rows are
    /// simply overwritten as new ones are filled.)
    pub fn start_slot(&mut self) {
        self.consumed = 0;
        self.filled = 0;
        self.rows.count = 0;
        self.sanitised = 0;
        self.clamped = 0;
        self.skipped = 0;
        self.floored = 0;
        self.top = 0;
    }

    /// The level buffer as working storage for something else (the
    /// subtractions between passes): the spectrogram is emptied first, as
    /// by [`Spectrogram::start_slot`], so no row is read from it until it
    /// is fed again. Every byte of the buffer, at least
    /// [`Band::spectrogram_len`].
    pub(crate) fn idle_levels(&mut self) -> &mut [u8] {
        self.start_slot();
        &mut *self.rows.levels
    }

    /// Feeds 12 kHz samples (full scale ±1.0). Returns how many were
    /// taken: all of them, unless the slot's 180 000 samples are reached.
    pub fn feed(&mut self, samples: &[f32]) -> usize {
        self.take(samples.len(), |i, dst| {
            checked_copy(&samples[i..i + dst.len()], dst)
        })
    }

    /// Feeds 16-bit samples (full scale ±32 768).
    pub fn feed_i16(&mut self, samples: &[i16]) -> usize {
        // Each scaled sample is finite and within ±1, so `classify` would
        // return it unchanged and count nothing.
        self.take(samples.len(), |i, dst| {
            let src = &samples[i..i + dst.len()];
            for (d, &x) in dst.iter_mut().zip(src) {
                *d = f32::from(x) * (1.0 / 32_768.0);
            }
            (0, 0)
        })
    }

    /// Advances over `n` missing samples, which count as zeros. Returns
    /// how many were taken.
    pub fn skip(&mut self, n: usize) -> usize {
        let taken = self.take(n, |_, dst| {
            dst.fill(0.0);
            (0, 0)
        });
        self.skipped += taken;
        taken
    }

    /// Rows complete so far (0..=184).
    pub fn rows_filled(&self) -> usize {
        self.rows.count
    }

    /// Slot samples taken so far, fed or skipped.
    pub fn samples_consumed(&self) -> usize {
        self.consumed
    }

    /// Non-finite samples replaced by 0.0 in this slot.
    pub fn sanitised_samples(&self) -> usize {
        self.sanitised
    }

    /// Finite samples beyond ±2¹⁶ clamped to it in this slot.
    pub fn clamped_samples(&self) -> usize {
        self.clamped
    }

    /// Levels at the floor (−127.5 dB) in the rows filled so far.
    pub fn floored_levels(&self) -> usize {
        self.floored
    }

    /// Levels at the top (0 dB) in the rows filled so far.
    pub fn top_levels(&self) -> usize {
        self.top
    }

    /// Samples skipped (as zeros) in this slot.
    pub fn skipped_samples(&self) -> usize {
        self.skipped
    }

    /// The band.
    pub fn band(&self) -> Band {
        self.rows.band
    }

    /// Row `r`'s levels, one per stored bin, or `None` if the row is not
    /// filled yet.
    pub fn row(&self, r: usize) -> Option<&[u8]> {
        self.rows.row(r)
    }

    /// The filled rows.
    #[cfg(any(test, feature = "profile"))]
    pub(crate) fn rows(&self) -> &Rows<'a> {
        &self.rows
    }

    /// The filled rows and the FFT's work area, free until the next row is
    /// computed (see [`FftBuffer::cells`]). The input window is not part of
    /// it: it holds samples from one feed to the next.
    pub(crate) fn rows_and_work(&mut self) -> (&Rows<'a>, &mut [f32]) {
        (&self.rows, self.front.fft.cells())
    }

    /// The core loop: takes up to `n` samples. `fill(i, dst)` writes
    /// samples `i..i + dst.len()` into `dst` as the front end takes them
    /// (see [`classify`]) and returns how many it sanitised and clamped.
    fn take<F: FnMut(usize, &mut [f32]) -> (usize, usize)>(
        &mut self,
        n: usize,
        mut fill: F,
    ) -> usize {
        let n = n.min(SLOT_SAMPLES - self.consumed);
        // Samples before the row origin belong to no row, but are still
        // checked so the counters cover the whole slot. They pass through
        // the start of the window, which is empty until the row origin
        // (`filled` is 0) and which the first row's samples overwrite.
        let before = n.min(ROW_ORIGIN.saturating_sub(self.consumed));
        // (No row is complete then, so in the ring too the samples are at
        // their own places.)
        debug_assert!(before == 0 || self.rows.count == 0);
        let (sanitised, clamped) = fill(0, &mut self.front.window[..before]);
        self.sanitised += sanitised;
        self.clamped += clamped;
        let mut i = before;
        self.consumed += before;
        while i < n {
            let chunk = (n - i).min(WINDOW - self.filled);
            // Where the transform takes one lane the window is a ring of
            // quarters ([`ring_at`]). A chunk never runs past a quarter's
            // end there but before the first row, when the ring is the
            // window in order: from then on the window holds three
            // quarters at each feed, so the chunk ends with the fourth.
            let at = if ONE_LANE {
                ring_at(self.filled, self.rows.count)
            } else {
                self.filled
            };
            debug_assert!(self.rows.count == 0 || at % HOP + chunk <= HOP);
            let (sanitised, clamped) = fill(i, &mut self.front.window[at..at + chunk]);
            self.sanitised += sanitised;
            self.clamped += clamped;
            i += chunk;
            self.consumed += chunk;
            self.filled += chunk;
            if self.filled == WINDOW {
                self.finish_row();
                // In the ring the quarter just left is the next to fill
                // (`rows.count` has advanced); otherwise the window slides.
                if !ONE_LANE {
                    self.front.window.copy_within(HOP.., 0);
                }
                self.filled = WINDOW - HOP;
            }
        }
        n
    }

    /// Computes the next row from the full window.
    fn finish_row(&mut self) {
        let width = self.rows.band.stored_bins();
        let r = self.rows.count;
        let row = &mut self.rows.levels[r * width..(r + 1) * width];
        let levels = |i: usize, p: &[f32]| {
            // The hosts' logarithm: see `levels_by_steps_out_of_line`.
            if SCALAR_FORMS {
                levels_by_steps_out_of_line(&mut row[i..i + p.len()], p);
            } else {
                for (level, &p) in row[i..i + p.len()].iter_mut().zip(p) {
                    *level = level_of(log2_power(p));
                }
            }
        };
        let (window, fft) = (&self.front.window, &mut self.front.fft);
        let first = self.rows.band.first_bin();
        // The ring where the transform takes one lane, as its quarters in
        // order (`ring`); else the window as it slides, joined as it is
        // (`window`).
        if ONE_LANE {
            let ring = ring_quarters(window, r);
            power_bins(|b| spectrum_of_quarters(ring, b), fft, first, width, levels);
        } else {
            power_bins(|b| spectrum(window, b), fft, first, width, levels);
        }
        let (floored, top) = level_counts(row);
        self.floored += floored;
        self.top += top;
        self.rows.count += 1;
    }
}

/// Quarters of the window: [`HOP`] samples each, the hop between rows.
const QUARTERS: usize = WINDOW / HOP;
const _: () = assert!(QUARTERS * HOP == WINDOW);

/// Where the transform takes one lane ([`ONE_LANE`]) the window is a ring
/// of [`QUARTERS`] quarters rather than sliding by [`HOP`] after each row
/// (a copy of three quarters, 11 520 bytes, a row: on the Cortex-M33 1.6%
/// of a small slot in the library's `memmove`, 1.7% of it after setup;
/// with the ring, and the transform's first stage reading the quarters,
/// the slot took 3.2% fewer instructions: findings O31). Sample `at` of
/// the window, while `rows` rows are complete, is held at the returned
/// index: its quarter turned on by `rows`, so the quarter a row leaves is
/// the one the next row's last quarter fills.
///
/// The rotation is the rows completed mod [`QUARTERS`] by construction:
/// `rows` is the spectrogram's `rows.count`, which `finish_row` advances
/// once a row, as the window fills, and only there, and which is 0 when a
/// slot starts (`start_slot`), when the ring is the window in order. The
/// ring has no phase of its own to fall out of step with it.
#[inline(always)]
fn ring_at(at: usize, rows: usize) -> usize {
    ((at / HOP + rows) % QUARTERS) * HOP + at % HOP
}

/// The window's quarters in order (see [`ring_at`]) while `rows` rows are
/// complete.
#[inline(always)]
fn ring_quarters(window: &[f32; WINDOW], rows: usize) -> [&[f32; HOP]; QUARTERS] {
    core::array::from_fn(|q| {
        let at = ring_at(q * HOP, rows);
        match window[at..at + HOP].try_into() {
            Ok(quarter) => quarter,
            Err(_) => unreachable!(),
        }
    })
}

/// Each power's level into `levels`, of the same length, by
/// [`level_by_steps`]: the front end's row where the target has no vector
/// unit ([`SCALAR_FORMS`]). The hosts keep `level_of(log2_power(p))` only
/// so that their binaries stay as they were while the Cortex-A76 is not
/// available for timing: on x86-64 the steps measured faster (the feed
/// about 17% less wall-clock time), and the steps on every target, if the
/// A76 agrees, are the next candidate (findings O30b). Out of line:
/// inlined into the row, the transform beside it compiled to more
/// instructions on the Cortex-M33 than the levels saved (findings O30).
#[inline(never)]
fn levels_by_steps_out_of_line(levels: &mut [u8], powers: &[f32]) {
    for (level, &p) in levels.iter_mut().zip(powers) {
        *level = level_by_steps(p);
    }
}

/// How many of `levels` (one row: at most [`BINS`], so the counts fit in
/// 16 bits) are at the floor (0) and at the top (255), in one pass. In
/// 16-bit counts the compiler vectorises it as byte compares and widening
/// subtractions (rustc 1.93.0; counts in `usize` took 64-bit lanes, a
/// pass for each). Where the target has no vector unit
/// ([`SCALAR_FORMS`]), [`level_counts_by_words`].
fn level_counts(levels: &[u8]) -> (usize, usize) {
    const _: () = assert!(BINS <= u16::MAX as usize);
    // A row's stored bins end below Nyquist (`Band::from_hz`).
    debug_assert!(levels.len() <= BINS);
    if SCALAR_FORMS {
        return level_counts_by_words(levels);
    }
    let (mut floored, mut top) = (0u16, 0u16);
    for &l in levels {
        floored += u16::from(l == 0);
        top += u16::from(l == u8::MAX);
    }
    (usize::from(floored), usize::from(top))
}

/// [`level_counts`] four levels a word, for the targets without a vector
/// unit: a byte compare and a count a level took 9 instructions on the
/// Cortex-M33. In a word `w`, `zero_bytes(w)` (the classic test for a
/// zero byte in a word) has bit 7 of each byte set where that byte is 0
/// and every other bit clear: a byte's low seven bits plus 0x7F reach bit
/// 7 unless they are all 0, with no carry into the next byte, and its own
/// bit 7 is or-ed in. `!w` has a byte of 0 where `w` has 255. Each word
/// adds those bits, shifted to bit 0, to four byte lanes,
/// which count at most 255 words, so blocks of 255 words are summed
/// across their lanes at the block's end; a block's levels after its last
/// whole word one by one. 13 instructions a word on the Cortex-M33; a
/// last word built with 1s past the levels (neither 0 nor 255), in place
/// of that loop, took 0.14% more of the small slot (findings O34).
fn level_counts_by_words(levels: &[u8]) -> (usize, usize) {
    const LOW: u32 = 0x7f7f_7f7f;
    let zero_bytes = |w: u32| !(((w & LOW) + LOW) | w | LOW);
    let sum_lanes = |x: u32| {
        let pairs = (x & 0x00ff_00ff) + ((x >> 8) & 0x00ff_00ff);
        (pairs & 0xffff) + (pairs >> 16)
    };
    let (mut floored, mut top) = (0u32, 0u32);
    for block in levels.chunks(4 * 255) {
        let (mut f, mut t) = (0u32, 0u32);
        let mut words = block.chunks_exact(4);
        for w in &mut words {
            let w = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
            f += zero_bytes(w) >> 7;
            t += zero_bytes(!w) >> 7;
        }
        floored += sum_lanes(f);
        top += sum_lanes(t);
        // Only the last block can have levels after its last whole word.
        for &l in words.remainder() {
            floored += u32::from(l == 0);
            top += u32::from(l == u8::MAX);
        }
    }
    (floored as usize, top as usize)
}

#[cfg(test)]
mod tests;
