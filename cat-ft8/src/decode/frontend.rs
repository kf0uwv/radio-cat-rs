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
//! [`Band::spectrogram_len`] entries.

use super::fft::{power_bins, FftBuffer, BINS, WINDOW};
use super::math::log2_power;
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

mod sealed {
    pub trait Sealed {}
    impl Sealed for u8 {}
}

/// A quantised spectrogram level: `u8`, in 0.5 dB steps, with 0 dB (a
/// full-scale sine centred in a bin) as the top value. Sealed; `u8` is
/// the only implementation (a 16-bit form measured no better).
pub trait Level: sealed::Sealed + Copy + Default + Ord + core::fmt::Debug + 'static {
    /// Level steps per dB.
    const STEPS_PER_DB: u32;
    /// The top value (0 dB).
    const MAX: u32;
    /// The level of a `log2` power value.
    fn from_log2_power(l2: f32) -> Self;
    /// The level as an integer.
    fn value(self) -> u32;
}

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

impl Level for u8 {
    const STEPS_PER_DB: u32 = 2;
    const MAX: u32 = 255;
    #[inline]
    fn from_log2_power(l2: f32) -> Self {
        quantise(l2 * (DB_PER_LOG2 * 2.0) + 255.0, <Self as Level>::MAX) as u8
    }
    #[inline]
    fn value(self) -> u32 {
        u32::from(self)
    }
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
#[derive(Debug, Clone)]
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

impl Default for FrontEnd {
    fn default() -> Self {
        Self::new()
    }
}

/// A slot's spectrogram, filled block by block from 12 kHz samples. Borrows
/// the caller's [`FrontEnd`] and level slice.
#[derive(Debug)]
pub struct Spectrogram<'a, L: Level> {
    front: &'a mut FrontEnd,
    levels: &'a mut [L],
    band: Band,
    /// Slot samples taken so far (fed or skipped).
    consumed: usize,
    /// Samples in the front end's window.
    filled: usize,
    rows: usize,
    sanitised: usize,
    clamped: usize,
    skipped: usize,
    floored: usize,
    top: usize,
}

impl<'a, L: Level> Spectrogram<'a, L> {
    /// A spectrogram for `band` over the caller's buffers, at the start of
    /// a slot. [`Error::BufferTooSmall`] if `levels` is shorter than
    /// [`Band::spectrogram_len`].
    pub fn new(front: &'a mut FrontEnd, levels: &'a mut [L], band: Band) -> Result<Self, Error> {
        if levels.len() < band.spectrogram_len() {
            return Err(Error::BufferTooSmall);
        }
        Ok(Spectrogram {
            front,
            levels,
            band,
            consumed: 0,
            filled: 0,
            rows: 0,
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
        self.rows = 0;
        self.sanitised = 0;
        self.clamped = 0;
        self.skipped = 0;
        self.floored = 0;
        self.top = 0;
    }

    /// Feeds 12 kHz samples (full scale ±1.0). Returns how many were
    /// taken: all of them, unless the slot's 180 000 samples are reached.
    pub fn feed(&mut self, samples: &[f32]) -> usize {
        self.take(samples.len(), |i| samples[i])
    }

    /// Feeds 16-bit samples (full scale ±32 768).
    pub fn feed_i16(&mut self, samples: &[i16]) -> usize {
        self.take(samples.len(), |i| f32::from(samples[i]) * (1.0 / 32_768.0))
    }

    /// Advances over `n` missing samples, which count as zeros. Returns
    /// how many were taken.
    pub fn skip(&mut self, n: usize) -> usize {
        let taken = self.take(n, |_| 0.0);
        self.skipped += taken;
        taken
    }

    /// Rows complete so far (0..=184).
    pub fn rows_filled(&self) -> usize {
        self.rows
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
        self.band
    }

    /// Row `r`'s levels, one per stored bin, or `None` if the row is not
    /// filled yet.
    pub fn row(&self, r: usize) -> Option<&[L]> {
        if r >= self.rows {
            return None;
        }
        let width = self.band.stored_bins();
        Some(&self.levels[r * width..(r + 1) * width])
    }

    /// The core loop: takes up to `n` samples produced by `sample(i)`.
    fn take<F: Fn(usize) -> f32>(&mut self, n: usize, sample: F) -> usize {
        let n = n.min(SLOT_SAMPLES - self.consumed);
        let mut i = 0;
        // Samples before the row origin belong to no row, but are still
        // checked so the counters cover the whole slot.
        let before = n.min(ROW_ORIGIN.saturating_sub(self.consumed));
        for k in 0..before {
            let x = sample(k);
            if !x.is_finite() {
                self.sanitised += 1;
            } else if !(-INPUT_LIMIT..=INPUT_LIMIT).contains(&x) {
                // (A range, not `f32::abs`: that is in `core` only from
                // Rust 1.85, above the crate's minimum.)
                self.clamped += 1;
            }
        }
        i += before;
        self.consumed += before;
        while i < n {
            let chunk = (n - i).min(WINDOW - self.filled);
            for (k, slot) in self.front.window[self.filled..self.filled + chunk]
                .iter_mut()
                .enumerate()
            {
                let x = sample(i + k);
                *slot = if !x.is_finite() {
                    self.sanitised += 1;
                    0.0
                } else if x > INPUT_LIMIT {
                    self.clamped += 1;
                    INPUT_LIMIT
                } else if x < -INPUT_LIMIT {
                    self.clamped += 1;
                    -INPUT_LIMIT
                } else {
                    x
                };
            }
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
        let width = self.band.stored_bins();
        let row = &mut self.levels[self.rows * width..(self.rows + 1) * width];
        let (mut floored, mut top) = (0, 0);
        power_bins(
            &self.front.window,
            &mut self.front.fft,
            self.band.first_bin(),
            width,
            |i, p| {
                let level = L::from_log2_power(log2_power(p));
                floored += usize::from(level.value() == 0);
                top += usize::from(level.value() == L::MAX);
                row[i] = level;
            },
        );
        self.floored += floored;
        self.top += top;
        self.rows += 1;
    }
}

#[cfg(test)]
mod tests;
