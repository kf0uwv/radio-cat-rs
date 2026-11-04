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

//! Soft bits (ADR 0024, amendment "single-pass decoder"): single-symbol
//! detection with the max-log metric of QEX §6 (N = 1).
//!
//! For each data symbol, the eight tone levels of its row are read; for
//! each of the symbol's three bits, the log-likelihood ratio is the
//! highest level among tones whose Gray-coded bit is 0, minus the highest
//! among tones whose bit is 1. Levels are integers, so a tie gives exactly
//! zero: no tone and no bit value is favoured. A positive value favours 0.
//!
//! Symbols whose row is not filled (before row 0, after the slot's last
//! filled row) are erasures: their three values are exactly zero.
//!
//! The values are then scaled so their mean magnitude (over bits that are
//! not erased) equals a target, which sets the reliability scale belief
//! propagation sees. The mean magnitude is used rather than the standard
//! deviation, so no square root is needed.

use super::frontend::Spectrogram;
use super::math::{log2_power, SCALAR_FORMS};
use super::sync::Candidate;
use super::tables::LEVEL_POWER;
use crate::ldpc::N;
use crate::symbols::{data_index, DATA_SYMBOLS, GRAY_UNMAP};
use crate::SYMBOL_COUNT;

/// For bit j (0 = most significant of the symbol's three), the tones whose
/// Gray-coded bit j is 1, as a bit mask over tones 0–7.
const ONES: [u8; 3] = {
    let mut masks = [0u8; 3];
    let mut j = 0;
    while j < 3 {
        let mut t = 0;
        while t < 8 {
            if (GRAY_UNMAP[t] >> (2 - j)) & 1 == 1 {
                masks[j] |= 1 << t;
            }
            t += 1;
        }
        j += 1;
    }
    masks
};

/// Soft bits for a candidate: `llr[i]` for codeword bit `i` (positive
/// favours 0), scaled so the mean magnitude of the bits that are not
/// erased is `target`. Returns the number of erased symbols. (The
/// decoder uses [`soft_bits_at`]; this grid form is kept for tests and
/// measurement.)
#[cfg(any(test, feature = "profile"))]
pub fn soft_bits(
    spec: &Spectrogram<'_>,
    candidate: &Candidate,
    target: f32,
    llr: &mut [f32; N],
) -> usize {
    let base = usize::from(candidate.base);
    fill_and_scale(target, llr, |k, v| {
        let row = i32::from(candidate.start_row) + 2 * k as i32;
        let Some(levels) = (if row >= 0 {
            spec.row(row as usize)
        } else {
            None
        }) else {
            return false;
        };
        // Integer levels are exact in `f32`, so the metric is the
        // integer level difference.
        for (t, x) in v.iter_mut().enumerate() {
            *x = f32::from(levels[base + 2 * t]);
        }
        true
    })
}

/// The shared tail of both forms: for each data symbol `k`, `fill(k, v)`
/// writes its eight tone values (or returns false for an erased symbol,
/// whose three soft bits are then exactly 0); each bit's value is the
/// highest value among tones whose Gray-coded bit is 0 minus the highest
/// among tones whose bit is 1; then all are scaled so the mean magnitude
/// over bits not erased is `target`. Returns the number of erased symbols.
fn fill_and_scale(
    target: f32,
    llr: &mut [f32; N],
    mut fill: impl FnMut(usize, &mut [f32; 8]) -> bool,
) -> usize {
    let mut erased = 0;
    let mut sum = 0.0f32;
    let mut v = [0.0f32; 8];
    for k in 0..SYMBOL_COUNT {
        let Some(d) = data_index(k) else { continue };
        if !fill(k, &mut v) {
            llr[3 * d..3 * d + 3].fill(0.0);
            erased += 1;
            continue;
        }
        for (j, &ones) in ONES.iter().enumerate() {
            let (mut max0, mut max1) = (f32::MIN, f32::MIN);
            for (t, &x) in v.iter().enumerate() {
                if ones & (1 << t) != 0 {
                    max1 = if x > max1 { x } else { max1 };
                } else {
                    max0 = if x > max0 { x } else { max0 };
                }
            }
            let l = max0 - max1;
            llr[3 * d + j] = l;
            sum += if l < 0.0 { -l } else { l };
        }
    }
    let live = (3 * (DATA_SYMBOLS - erased)) as f32;
    if sum > 0.0 {
        let scale = target * live / sum;
        for x in llr.iter_mut() {
            *x *= scale;
        }
    }
    erased
}

/// Linear power (relative to the top level) of a level value `v`.
#[inline]
pub(crate) fn level_power(v: u32) -> f32 {
    LEVEL_POWER[(v as usize).min(255)]
}

/// The two grid indices around `i + f` (|f| ≤ ½) and their weights.
#[inline]
pub(crate) fn straddle(i: i32, f: f32) -> [(i32, f32); 2] {
    if f < 0.0 {
        [(i - 1, -f), (i, 1.0 + f)]
    } else {
        [(i, 1.0 - f), (i + 1, f)]
    }
}

/// The power of `tone` for the symbol centred on row `centre`,
/// interpolated with the row and bin weights `rows` (offsets from
/// `centre`) and `bins` (base bins); `None` if no row of non-zero weight
/// is filled.
#[inline]
pub(crate) fn interpolated_tone_power(
    spec: &Spectrogram<'_>,
    centre: i32,
    rows: &[(i32, f32); 2],
    bins: &[(i32, f32); 2],
    tone: u8,
    stored: i32,
) -> Option<f32> {
    let mut p = 0.0f32;
    let mut any = false;
    for &(dr, wr) in rows.iter() {
        let row = centre + dr;
        if wr == 0.0 || row < 0 {
            continue;
        }
        let Some(levels) = spec.row(row as usize) else {
            continue;
        };
        any = true;
        for &(b, wb) in bins.iter() {
            let bin = b + 2 * i32::from(tone);
            if (0..stored).contains(&bin) {
                p += wr * wb * level_power(u32::from(levels[bin as usize]));
            }
        }
    }
    any.then_some(p)
}

/// The levels of the rows `rows` (offsets from `centre`, with their
/// weights) that [`interpolated_tone_power`] reads: `None` for a row of
/// zero weight, before the first row or not filled.
#[inline(always)]
fn row_levels<'a>(
    spec: &'a Spectrogram<'_>,
    centre: i32,
    rows: &[(i32, f32); 2],
) -> [Option<&'a [u8]>; 2] {
    let levels = |(dr, wr): (i32, f32)| {
        let row = centre + dr;
        if wr == 0.0 || row < 0 {
            None
        } else {
            spec.row(row as usize)
        }
    };
    // Not `rows.map`: on the Cortex-M33 its iterator was left out of line.
    [levels(rows[0]), levels(rows[1])]
}

/// [`interpolated_tone_power`] from the rows' levels [`row_levels`] took,
/// at least one of them present: the same products, added in the same
/// order.
#[inline(always)]
fn tone_power_of(
    levels: &[Option<&[u8]>; 2],
    rows: &[(i32, f32); 2],
    bins: &[(i32, f32); 2],
    tone: u8,
    stored: i32,
) -> f32 {
    let mut p = 0.0f32;
    for (&(_, wr), levels) in rows.iter().zip(levels) {
        let Some(levels) = levels else {
            continue;
        };
        for &(b, wb) in bins.iter() {
            let bin = b + 2 * i32::from(tone);
            if (0..stored).contains(&bin) {
                p += wr * wb * level_power(u32::from(levels[bin as usize]));
            }
        }
    }
    p
}

/// Soft bits as [`soft_bits`], at a position refined by `frac_row` rows
/// and `frac_bin` bins (each in −½…½) from the candidate's grid point:
/// each tone's power is interpolated bilinearly from the four
/// surrounding grid values (in linear power), and the metric is taken
/// on its logarithm. A symbol is erased if none of its rows is filled.
/// Where the target has no vector unit ([`SCALAR_FORMS`]), the symbol's
/// rows are looked up once for its eight tones ([`row_levels`],
/// [`tone_power_of`]: the soft bits 12% fewer instructions on the
/// Cortex-M33, findings O34); the hosts look them up for each tone, as
/// before, until the Cortex-A76 has timed the change.
pub fn soft_bits_at(
    spec: &Spectrogram<'_>,
    candidate: &Candidate,
    frac_row: f32,
    frac_bin: f32,
    target: f32,
    llr: &mut [f32; N],
) -> usize {
    let stored = spec.band().stored_bins() as i32;
    let rows = straddle(0, frac_row);
    let bins = straddle(i32::from(candidate.base), frac_bin);
    fill_and_scale(target, llr, |k, v| {
        let centre = i32::from(candidate.start_row) + 2 * k as i32;
        if SCALAR_FORMS {
            let levels = row_levels(spec, centre, &rows);
            if levels.iter().all(Option::is_none) {
                return false;
            }
            for (t, x) in v.iter_mut().enumerate() {
                *x = log2_power(tone_power_of(&levels, &rows, &bins, t as u8, stored));
            }
            return true;
        }
        for (t, x) in v.iter_mut().enumerate() {
            match interpolated_tone_power(spec, centre, &rows, &bins, t as u8, stored) {
                Some(p) => *x = log2_power(p),
                None => return false,
            }
        }
        true
    })
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::vec;

    use super::*;
    use crate::decode::frontend::Band;
    use crate::decode::sync::{dt_of, Candidate};
    use crate::decode::testutil::{add_noise, add_signal, empty_slot, symbols_of, Built};
    use crate::message::NoHashes;
    use crate::{crc, ldpc, pack};

    fn codeword_of(message: &str) -> ldpc::Codeword {
        ldpc::encode(crc::append_crc(&pack(message, &mut NoHashes).unwrap()))
    }

    fn llr_for(slot: &[f32], cand: Candidate, target: f32) -> ([f32; N], usize) {
        let mut built = Built::new(Band::DEFAULT);
        built.with(slot, |spec| {
            let mut llr = [9.0f32; N];
            let erased = soft_bits(spec, &cand, target, &mut llr);
            (llr, erased)
        })
    }

    #[test]
    fn the_gray_masks_match_the_gray_map() {
        // Tones with bit 0 (MSB) set: values 4..7 → tones 5, 6, 4, 7.
        assert_eq!(ONES[0], (1 << 4) | (1 << 5) | (1 << 6) | (1 << 7));
        for (j, &m) in ONES.iter().enumerate() {
            assert_eq!(m.count_ones(), 4, "bit {j}");
        }
    }

    #[test]
    fn a_clean_signal_gives_every_sign_right() {
        let msg = "CQ K1ABC FN42";
        let mut slot = empty_slot();
        let cand = Candidate {
            score: 0,
            start_row: 9,
            base: 400,
        };
        add_signal(
            &mut slot,
            &symbols_of(msg),
            (32 + 400) as f32 * 3.125,
            dt_of(9),
            0.3,
        );
        add_noise(&mut slot, 0.001, 4);
        let (llr, erased) = llr_for(&slot, cand, 4.0);
        assert_eq!(erased, 0);
        let cw = codeword_of(msg);
        for (i, &l) in llr.iter().enumerate() {
            let bit = if l > 0.0 { 0 } else { 1 };
            assert!(l != 0.0 && bit == cw.bit(i), "bit {i}: {l}");
        }
        let mean = llr.iter().map(|x| x.abs()).sum::<f32>() / N as f32;
        assert!((mean - 4.0).abs() < 1e-3, "{mean}");
    }

    #[test]
    fn symbols_outside_the_rows_are_erasures() {
        let msg = "K1ABC W9XYZ -11";
        let mut slot = empty_slot();
        // Start row −20: symbols 0–9 have no row (row < 0).
        add_signal(&mut slot, &symbols_of(msg), 1_250.0, dt_of(-20), 0.3);
        add_noise(&mut slot, 0.001, 5);
        let cand = Candidate {
            score: 0,
            start_row: -20,
            base: (1_250.0 / 3.125) as u16 - 32,
        };
        let (llr, erased) = llr_for(&slot, cand, 4.0);
        // Channel symbols 0..10 lie before row 0: 7 Costas + 3 data.
        assert_eq!(erased, 3);
        assert!(llr[..9].iter().all(|&x| x == 0.0));
        let cw = codeword_of(msg);
        for (i, &l) in llr.iter().enumerate().skip(9) {
            assert_eq!(if l > 0.0 { 0 } else { 1 }, cw.bit(i), "bit {i}");
        }
    }

    #[test]
    fn interpolated_bits_at_the_grid_point_follow_the_integer_bits() {
        // At zero offsets the interpolated form reads the same levels: the
        // same signs, the same zeros and the same erasures, at a start row
        // that leaves the first symbols outside the rows.
        let mut slot = empty_slot();
        let cand = Candidate {
            score: 0,
            start_row: -21,
            base: 300,
        };
        add_signal(
            &mut slot,
            &symbols_of("K1ABC W9XYZ -11"),
            (32 + 300) as f32 * 3.125,
            dt_of(-21),
            0.01,
        );
        add_noise(&mut slot, 0.03, 9);
        let mut built = Built::new(Band::DEFAULT);
        built.with(&slot, |spec| {
            let (mut grid, mut interp) = ([0.0f32; N], [0.0f32; N]);
            let e1 = soft_bits(spec, &cand, 4.0, &mut grid);
            let e2 = soft_bits_at(spec, &cand, 0.0, 0.0, 4.0, &mut interp);
            assert_eq!((e1, e2), (4, 4)); // data symbols 7–10
            for (i, (&g, &v)) in grid.iter().zip(interp.iter()).enumerate() {
                assert_eq!(g > 0.0, v > 0.0, "bit {i}");
                assert_eq!(g == 0.0, v == 0.0, "bit {i}");
            }
        });
    }

    /// [`tone_power_of`] over the rows [`row_levels`] looks up once (the
    /// form without a vector unit) equals [`interpolated_tone_power`] bit
    /// for bit, and both have no power where no row is present: at every
    /// tone, row offsets either side of the grid and on it (a row of zero
    /// weight), rows before the first and past the last filled one, base
    /// bins at both edges of the band and inside it, odd and even.
    #[test]
    fn tone_powers_from_rows_looked_up_once_equal_the_interpolated_powers() {
        let mut slot = empty_slot();
        add_signal(
            &mut slot,
            &symbols_of("K1ABC W9XYZ -11"),
            (32 + 300) as f32 * 3.125,
            dt_of(3),
            0.01,
        );
        add_noise(&mut slot, 0.03, 11);
        for samples in [crate::SLOT_SAMPLES, 50_000] {
            let mut built = Built::new(Band::DEFAULT);
            built.with(&slot[..samples], |spec| {
                let stored = spec.band().stored_bins() as i32;
                let filled = spec.rows_filled() as i32;
                let (mut checked, mut none) = (0, 0);
                for centre in (-3..filled + 3).chain([0, 1, filled - 1, filled]) {
                    for frac_row in [-0.5f32, -0.2, 0.0, 0.3, 0.5] {
                        let rows = straddle(0, frac_row);
                        let levels = row_levels(spec, centre, &rows);
                        for base in [0, 1, 300, 301, stored - 16, stored - 15, stored - 1] {
                            for frac_bin in [-0.5f32, 0.0, 0.25, 0.5] {
                                let bins = straddle(base, frac_bin);
                                for tone in 0..8u8 {
                                    let want = interpolated_tone_power(
                                        spec, centre, &rows, &bins, tone, stored,
                                    );
                                    let got = (!levels.iter().all(Option::is_none)).then(|| {
                                        tone_power_of(&levels, &rows, &bins, tone, stored)
                                    });
                                    assert_eq!(
                                        got.map(f32::to_bits),
                                        want.map(f32::to_bits),
                                        "row {centre} {frac_row}, bin {base} {frac_bin}, tone {tone}"
                                    );
                                    checked += 1;
                                    none += usize::from(want.is_none());
                                }
                            }
                        }
                    }
                }
                assert!(none > 0 && none < checked, "{none} of {checked}");
            });
        }
    }

    #[test]
    fn ties_give_exactly_zero() {
        // Silence: every level is the floor, so every comparison ties.
        let slot = vec![0.0f32; crate::SLOT_SAMPLES];
        let cand = Candidate {
            score: 0,
            start_row: 5,
            base: 100,
        };
        let (llr, erased) = llr_for(&slot, cand, 4.0);
        assert_eq!(erased, 0);
        assert!(llr.iter().all(|&x| x == 0.0));
    }
}
