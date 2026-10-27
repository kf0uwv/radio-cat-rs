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

//! Channel symbols (QEX 2020 §4): the 174 codeword bits in groups of three,
//! Gray-mapped to tones 0–7 (Table 3), as 58 data symbols framed by the
//! Costas array 3, 1, 4, 0, 6, 5, 2 at the start, middle and end:
//! `S(7) A(29) S(7) B(29) S(7)` = 79 symbols.

use crate::ldpc::Codeword;
use crate::SYMBOL_COUNT;

/// The 7×7 Costas synchronization array.
pub const COSTAS: [u8; 7] = [3, 1, 4, 0, 6, 5, 2];
/// First symbol of each of the three Costas arrays.
pub const COSTAS_POSITIONS: [usize; 3] = [0, 36, 72];
/// Gray map, three codeword bits (MSB first, as a value 0–7) → tone.
pub const GRAY_MAP: [u8; 8] = [0, 1, 3, 2, 5, 6, 4, 7];
/// Inverse Gray map, tone → three bits.
pub const GRAY_UNMAP: [u8; 8] = {
    let mut inv = [0u8; 8];
    let mut i = 0;
    while i < 8 {
        inv[GRAY_MAP[i] as usize] = i as u8;
        i += 1;
    }
    inv
};
/// Data symbols per transmission.
pub const DATA_SYMBOLS: usize = 58;

/// The two runs of data symbols, as channel-symbol ranges: the gaps
/// between the Costas arrays (the single source of the frame layout).
const DATA_BLOCKS: [core::ops::Range<usize>; 2] = [
    COSTAS_POSITIONS[0] + COSTAS.len()..COSTAS_POSITIONS[1],
    COSTAS_POSITIONS[1] + COSTAS.len()..COSTAS_POSITIONS[2],
];

// The frame layout, checked at compile time: Costas arrays at both ends,
// the two data blocks hold the 58 data symbols, and 58 × 3 bits are the
// whole codeword. `is_sync` and `map` rely on this.
const _: () = assert!(COSTAS_POSITIONS[0] == 0);
const _: () = assert!(COSTAS_POSITIONS[2] + COSTAS.len() == SYMBOL_COUNT);
const _: () = assert!(
    (DATA_BLOCKS[0].end - DATA_BLOCKS[0].start) + (DATA_BLOCKS[1].end - DATA_BLOCKS[1].start)
        == DATA_SYMBOLS
);
const _: () = assert!(3 * DATA_SYMBOLS == crate::ldpc::N);

/// True if channel symbol `k` belongs to a Costas array.
pub fn is_sync(k: usize) -> bool {
    k < SYMBOL_COUNT && data_index(k).is_none()
}

/// Index into the 58 data symbols for channel symbol `k`, if it carries data.
pub fn data_index(k: usize) -> Option<usize> {
    let mut before = 0;
    for block in DATA_BLOCKS.iter() {
        if block.contains(&k) {
            return Some(before + k - block.start);
        }
        before += block.end - block.start;
    }
    None
}

/// The 79 channel symbols of a codeword.
pub fn map(codeword: &Codeword) -> [u8; SYMBOL_COUNT] {
    let mut out = [0u8; SYMBOL_COUNT];
    for p in COSTAS_POSITIONS {
        out[p..p + COSTAS.len()].copy_from_slice(&COSTAS);
    }
    let mut bit = 0;
    for block in DATA_BLOCKS {
        for tone in out[block].iter_mut() {
            *tone = GRAY_MAP[codeword.bits(bit, 3) as usize];
            bit += 3;
        }
    }
    out
}

/// Recovers the codeword from hard channel symbols (the inverse of
/// [`map`]; sync symbols are ignored). Returns `None` for a tone above 7.
pub fn unmap(symbols: &[u8; SYMBOL_COUNT]) -> Option<Codeword> {
    let mut bits = [0u8; crate::ldpc::N];
    for (k, &tone) in symbols.iter().enumerate() {
        if tone > 7 {
            return None;
        }
        if let Some(d) = data_index(k) {
            let v = GRAY_UNMAP[usize::from(tone)];
            bits[3 * d] = (v >> 2) & 1;
            bits[3 * d + 1] = (v >> 1) & 1;
            bits[3 * d + 2] = v & 1;
        }
    }
    Some(Codeword::from_bits(&bits))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn costas_arrays_sit_at_0_36_72() {
        let cw = crate::ldpc::encode(0x05a5_a5a5_a5a5_a5a5_a5a5_a5a5);
        let s = map(&cw);
        for p in COSTAS_POSITIONS {
            assert_eq!(&s[p..p + 7], &COSTAS);
        }
        assert_eq!((0..SYMBOL_COUNT).filter(|&k| is_sync(k)).count(), 21);
        assert_eq!(
            (0..SYMBOL_COUNT).filter_map(data_index).count(),
            DATA_SYMBOLS
        );
    }

    #[test]
    fn costas_array_is_a_permutation_with_distinct_differences() {
        let mut seen = [false; 7];
        for &t in COSTAS.iter() {
            assert!(!seen[usize::from(t)]);
            seen[usize::from(t)] = true;
        }
        // Costas property: for each shift, the tone differences are distinct.
        for shift in 1..7 {
            let mut diffs = [false; 13];
            for i in 0..7 - shift {
                let d = (i32::from(COSTAS[i + shift]) - i32::from(COSTAS[i]) + 6) as usize;
                assert!(!diffs[d], "shift {shift}");
                diffs[d] = true;
            }
        }
    }

    #[test]
    fn gray_map_matches_qex_table_3_and_adjacent_tones_differ_in_one_bit() {
        // Table 3: tone → bits.
        let table3 = [0b000, 0b001, 0b011, 0b010, 0b110, 0b100, 0b101, 0b111];
        for (tone, &bits) in table3.iter().enumerate() {
            assert_eq!(GRAY_MAP[bits] as usize, tone);
            assert_eq!(GRAY_UNMAP[tone] as usize, bits);
        }
        for t in 0..7 {
            assert_eq!((GRAY_UNMAP[t] ^ GRAY_UNMAP[t + 1]).count_ones(), 1);
        }
    }

    #[test]
    fn unmap_inverts_map() {
        let cw = crate::ldpc::encode(0x0012_3456_789a_bcde_f012_3456);
        assert_eq!(unmap(&map(&cw)), Some(cw));
    }
}
