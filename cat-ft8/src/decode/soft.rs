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

use super::frontend::{Level, Spectrogram};
use super::sync::Candidate;
use crate::ldpc::N;
use crate::symbols::{data_index, GRAY_UNMAP};
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
/// erased is `target`. Returns the number of erased symbols.
pub fn soft_bits<L: Level>(
    spec: &Spectrogram<'_, L>,
    candidate: &Candidate,
    target: f32,
    llr: &mut [f32; N],
) -> usize {
    let base = usize::from(candidate.base);
    let mut erased = 0;
    let mut sum = 0.0f32;
    for k in 0..SYMBOL_COUNT {
        let Some(d) = data_index(k) else { continue };
        let row = i32::from(candidate.start_row) + 2 * k as i32;
        let levels = if row >= 0 {
            spec.row(row as usize)
        } else {
            None
        };
        let Some(levels) = levels else {
            llr[3 * d..3 * d + 3].fill(0.0);
            erased += 1;
            continue;
        };
        let mut v = [0i32; 8];
        for (t, x) in v.iter_mut().enumerate() {
            *x = levels[base + 2 * t].value() as i32;
        }
        for (j, &ones) in ONES.iter().enumerate() {
            let (mut max0, mut max1) = (i32::MIN, i32::MIN);
            for (t, &x) in v.iter().enumerate() {
                if ones & (1 << t) != 0 {
                    max1 = max1.max(x);
                } else {
                    max0 = max0.max(x);
                }
            }
            let l = (max0 - max1) as f32;
            llr[3 * d + j] = l;
            sum += if l < 0.0 { -l } else { l };
        }
    }
    let live = (3 * (SYMBOL_COUNT - 21 - erased)) as f32;
    if sum > 0.0 {
        let scale = target * live / sum;
        for x in llr.iter_mut() {
            *x *= scale;
        }
    }
    erased
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
        let mut built = Built::<u8>::new(Band::DEFAULT);
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
