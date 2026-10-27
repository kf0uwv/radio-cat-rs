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

//! Signal-to-noise estimate in 2500 Hz (the definition of QEX §6), with a
//! local noise reference (ADR 0024, amendment "single-pass decoder").
//!
//! - **Signal.** The mean over the symbols in the filled rows of the power
//!   of the transmitted tone (the decoded payload re-encoded), taken at the
//!   refined position as the soft bits are, less the noise in one bin.
//! - **Noise.** Each side of the signal separately: 24 reference bins
//!   below its lowest tone and 24 above its highest, after an 8-bin gap
//!   (25–100 Hz away), over the signal's symbol rows. A side's noise per
//!   bin is its lower octile level converted to a mean (the lower octile
//!   of an exponential distribution is ln(8/7) of its mean); the noise is
//!   the mean of the two sides (one side at the band's edges).
//!   - *Local*, because a receiver's audio passband is seldom flat: a
//!     band-wide figure would be wrong away from the passband's centre.
//!   - *Per side, then averaged*, so that on a sloping noise floor the two
//!     sides bracket the signal; pooling them would let the quieter side
//!     decide.
//!   - *A low quantile*, because other signals raise some reference bins
//!     (their tones, and their spectral skirts): each FT8 tone is present
//!     in only one row in eight, so most of a side stays at the noise
//!     unless the band is crowded. In a band crowded enough that the whole
//!     reference is raised, the estimate still reads low.
//! - **Scale.** For white noise of variance σ², one 3.125 Hz Hann bin holds
//!   σ²/640 in level units (0 dB = a full-scale sine centred in a bin),
//!   and a sine of amplitude A centred in a bin reads A²; with signal power
//!   A²/2 and noise σ²·2500/6000 in 2500 Hz, SNR = P_tone / P_bin − 27.27
//!   dB. A calibration constant, measured on synthetic slots, accounts for
//!   the tone power lost to the GFSK shaping, the window and the
//!   interpolation.

use super::frontend::{Level, Spectrogram};
use super::math::log2_power;
use super::soft::{interpolated_tone_power, level_power, straddle};
use super::sync::Candidate;
use crate::SYMBOL_COUNT;

/// 10 · log10(2).
const DB_PER_LOG2: f32 = 3.010_3;
/// 10 · log10(640 · 2500 / 6000 / 0.5) = 27.27 dB: per-bin to 2500 Hz.
const BIN_TO_2500_DB: f32 = 27.270_5;
/// Offset (dB) added to the estimate: the mean error of the uncalibrated
/// estimate on synthetic slots was −2.35 dB in white noise and −2.25 dB in
/// noise shaped by a receiver-like passband, for −18…+20 dB and every
/// 500 Hz band from 200 to 2 800 Hz (see the decode bench, mode `snr`;
/// measured with a pooled median reference; with the per-side octile the
/// mean error stays within −0.2…+0.0 dB, so the constant is unchanged).
pub(crate) const CALIBRATION_DB: f32 = 2.3;
/// Lowest SNR reported, dB.
pub const SNR_FLOOR_DB: f32 = -30.0;
/// Reference bins each side, and the gap between them and the signal's
/// outer tones (25 Hz: nearer, the signal's own spectral skirts raised
/// the reference and compressed the estimate above about 0 dB).
const REF_BINS: i32 = 24;
const REF_GAP: i32 = 8;
/// The reference quantile, as a fraction: the lower octile.
const QUANTILE_NUM: u32 = 1;
const QUANTILE_DEN: u32 = 8;
/// Mean over the lower octile for an exponential distribution:
/// 1 / −ln(7/8).
const QUANTILE_TO_MEAN: f32 = 7.488_80;

/// The SNR (dB in 2500 Hz) of the signal with channel symbols `tones` at
/// candidate `c` refined by (`frac_row`, `frac_bin`). `hist` is scratch.
pub(crate) fn estimate<L: Level>(
    spec: &Spectrogram<'_, L>,
    c: &Candidate,
    frac_row: f32,
    frac_bin: f32,
    tones: &[u8; SYMBOL_COUNT],
    hist: &mut [u16; 256],
) -> f32 {
    let stored = spec.band().stored_bins() as i32;
    let base = i32::from(c.base);
    let rows = straddle(0, frac_row);
    let bins = straddle(base, frac_bin);
    let (mut signal, mut symbols) = (0.0f32, 0u32);
    for (k, &tone) in tones.iter().enumerate() {
        let centre = i32::from(c.start_row) + 2 * k as i32;
        if let Some(p) = interpolated_tone_power(spec, centre, &rows, &bins, tone, stored) {
            signal += p;
            symbols += 1;
        }
    }
    if symbols == 0 {
        return SNR_FLOOR_DB;
    }
    // Each side's noise separately, then their mean: on a sloping noise
    // floor the two sides bracket the signal.
    let below = base - REF_GAP - REF_BINS..base - REF_GAP;
    let above = base + 14 + 1 + REF_GAP..base + 14 + 1 + REF_GAP + REF_BINS;
    let mut sides = [0.0f32; 2];
    let mut present = 0u32;
    for (side, range) in sides.iter_mut().zip([below, above]) {
        if let Some(n) = side_noise(spec, c, range, stored, hist) {
            *side = n;
            present += 1;
        }
    }
    if present == 0 {
        return SNR_FLOOR_DB;
    }
    let noise = (sides[0] + sides[1]) / present as f32;
    let tone = signal / symbols as f32 - noise;
    if tone <= 0.0 {
        return SNR_FLOOR_DB;
    }
    let snr =
        DB_PER_LOG2 * (log2_power(tone) - log2_power(noise)) - BIN_TO_2500_DB + CALIBRATION_DB;
    if snr < SNR_FLOOR_DB {
        SNR_FLOOR_DB
    } else {
        snr
    }
}

/// Mean noise power per bin over the bins `range` (those inside the
/// stored spectrum) in the candidate's symbol rows, from the lower
/// octile of their levels; `None` if there are none.
fn side_noise<L: Level>(
    spec: &Spectrogram<'_, L>,
    c: &Candidate,
    range: core::ops::Range<i32>,
    stored: i32,
    hist: &mut [u16; 256],
) -> Option<f32> {
    *hist = [0; 256];
    let mut count = 0u32;
    for k in 0..SYMBOL_COUNT {
        let centre = i32::from(c.start_row) + 2 * k as i32;
        if centre < 0 {
            continue;
        }
        let Some(levels) = spec.row(centre as usize) else {
            continue;
        };
        for b in range.clone() {
            if (0..stored).contains(&b) {
                hist[levels[b as usize].value() as usize] += 1;
                count += 1;
            }
        }
    }
    if count == 0 {
        return None;
    }
    // The first level whose cumulative count reaches the quantile.
    let rank = (count * QUANTILE_NUM).div_ceil(QUANTILE_DEN);
    let mut acc = 0u32;
    for (v, &n) in hist.iter().enumerate() {
        acc += u32::from(n);
        if acc >= rank {
            return Some(level_power(v as u32) * QUANTILE_TO_MEAN);
        }
    }
    None
}
