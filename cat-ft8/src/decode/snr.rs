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
//! - **Noise.** The median level of reference bins just outside the
//!   signal (8 bins below its lowest tone and 8 above its highest, with an
//!   8-bin gap each side), over the signal's symbol rows, converted to a
//!   mean noise power per bin (the median of an exponential distribution
//!   is ln 2 of its mean). The reference is local because a receiver's
//!   audio passband is seldom flat: a whole-band noise figure would be
//!   wrong away from the passband's centre. The median ignores other
//!   signals in part of the reference.
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
/// 500 Hz band from 200 to 2 800 Hz (see the decode bench, mode `snr`).
pub(crate) const CALIBRATION_DB: f32 = 2.3;
/// 1 / ln 2.
const INV_LN2: f32 = core::f32::consts::LOG2_E;
/// Lowest SNR reported, dB.
pub const SNR_FLOOR_DB: f32 = -30.0;
/// Reference bins each side, and the gap between them and the signal's
/// outer tones (25 Hz: nearer, the signal's own spectral skirts raised
/// the reference and compressed the estimate above about 0 dB).
const REF_BINS: i32 = 8;
const REF_GAP: i32 = 8;

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
    *hist = [0; 256];
    let (mut count, mut signal, mut symbols) = (0u32, 0.0f32, 0u32);
    for (k, &tone) in tones.iter().enumerate() {
        let centre = i32::from(c.start_row) + 2 * k as i32;
        if let Some(p) = interpolated_tone_power(spec, centre, &rows, &bins, tone, stored) {
            signal += p;
            symbols += 1;
        }
        let Some(levels) = (if centre >= 0 {
            spec.row(centre as usize)
        } else {
            None
        }) else {
            continue;
        };
        let below = base - REF_GAP - REF_BINS..base - REF_GAP;
        let above = base + 14 + 1 + REF_GAP..base + 14 + 1 + REF_GAP + REF_BINS;
        for b in below.chain(above) {
            if (0..stored).contains(&b) {
                hist[levels[b as usize].value() as usize] += 1;
                count += 1;
            }
        }
    }
    if symbols == 0 || count == 0 {
        return SNR_FLOOR_DB;
    }
    // Median level: the first level whose cumulative count reaches half.
    let half = count.div_ceil(2);
    let (mut acc, mut median) = (0u32, 0usize);
    for (v, &n) in hist.iter().enumerate() {
        acc += u32::from(n);
        if acc >= half {
            median = v;
            break;
        }
    }
    let noise = level_power(median as u32) * INV_LN2;
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
