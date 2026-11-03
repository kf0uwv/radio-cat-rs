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

use super::fft::{power_bins, FftBuffer, BINS, WINDOW};
use super::math::{abs, log2_power};
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

/// Copies `src` into `dst` (the same length) as [`classify`] takes each
/// sample, and returns how many it sanitised and clamped. A block whose
/// samples are all finite and within ±[`INPUT_LIMIT`], which `classify`
/// returns unchanged, is checked as a whole and copied; any other block is
/// classified sample by sample.
fn checked_copy(src: &[f32], dst: &mut [f32]) -> (usize, usize) {
    // False for NaN and the infinities; true exactly where `classify`
    // returns `x`. A fold over the whole block rather than `all`: `all`
    // stops at the first failing sample, and a loop with that exit is not
    // vectorised (rustc 1.93.0). Blocks almost always pass.
    if src.iter().fold(true, |ok, &x| ok & (abs(x) <= INPUT_LIMIT)) {
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
        let (sanitised, clamped) = fill(0, &mut self.front.window[..before]);
        self.sanitised += sanitised;
        self.clamped += clamped;
        let mut i = before;
        self.consumed += before;
        while i < n {
            let chunk = (n - i).min(WINDOW - self.filled);
            let (sanitised, clamped) =
                fill(i, &mut self.front.window[self.filled..self.filled + chunk]);
            self.sanitised += sanitised;
            self.clamped += clamped;
            i += chunk;
            self.consumed += chunk;
            self.filled += chunk;
            if self.filled == WINDOW {
                self.finish_row();
                self.front.window.copy_within(HOP.., 0);
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
        power_bins(
            &self.front.window,
            &mut self.front.fft,
            self.rows.band.first_bin(),
            width,
            |i, p| {
                for (level, &p) in row[i..i + p.len()].iter_mut().zip(p) {
                    *level = level_of(log2_power(p));
                }
            },
        );
        self.floored += row.iter().filter(|&&l| l == 0).count();
        self.top += row.iter().filter(|&&l| l == u8::MAX).count();
        self.rows.count += 1;
    }
}

#[cfg(test)]
mod tests;
