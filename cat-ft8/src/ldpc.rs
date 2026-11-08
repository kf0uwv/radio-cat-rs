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

//! The (174,91) LDPC code (QEX 2020 §3). The code is systematic: a codeword
//! is the 91 message-plus-CRC bits followed by 83 parity bits, parity bit
//! `i` being the mod-2 sum of the message bits selected by generator row
//! `i`. The generator and parity-check tables are protocol constants (see
//! `ldpc_tables.rs` and the crate's `NOTICE`).

use crate::crc::A91_MASK;
use crate::ldpc_tables::{GENERATOR, PARITY_CHECKS};

/// Codeword length, bits.
pub const N: usize = 174;
/// Message-plus-CRC bits.
pub const K: usize = 91;
/// Parity bits.
pub const M: usize = 83;

/// Generator rows as 91-bit masks aligned with a 91-bit word (bit 0 of the
/// word at `>> 90`), built at compile time from the byte table.
pub(crate) const GENERATOR_ROWS: [u128; M] = {
    let mut rows = [0u128; M];
    let mut i = 0;
    while i < M {
        let mut v: u128 = 0;
        let mut j = 0;
        while j < 12 {
            v = (v << 8) | GENERATOR[i][j] as u128;
            j += 1;
        }
        rows[i] = v >> 5; // 96 table bits, of which the first 91 are used
        i += 1;
    }
    rows
};

/// A 174-bit codeword: message-plus-CRC, then parity, stored contiguously
/// MSB first: bit `i` is bit `63 − i % 64` of word `i / 64`. The 18 bits
/// after bit 173 and the fourth word are always zero; the fourth word lets
/// `bits` read any range as one 128-bit window without a bounds fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Codeword {
    words: [u64; 4],
}

impl Codeword {
    /// Builds the codeword from its 91-bit message part and 83-bit parity.
    fn from_parts(a91: u128, parity: u128) -> Self {
        // 91 + 83 = 174 bits = 64 + 64 + 46.
        let w0 = (a91 >> 27) as u64;
        let w1 = (((a91 & ((1 << 27) - 1)) << 37) | (parity >> 46)) as u64;
        let w2 = ((parity & ((1 << 46) - 1)) << 18) as u64;
        Codeword {
            words: [w0, w1, w2, 0],
        }
    }

    /// Codeword bit `i` (0 = first), as 0 or 1. Panics if `i >= 174`.
    pub fn bit(&self, i: usize) -> u8 {
        assert!(i < N, "codeword bit index out of range");
        self.bits(i, 1) as u8
    }

    /// `len` bits (0 to 32) starting at bit `start`, first bit most
    /// significant; 0 for `len == 0`. Panics if they run past bit 173.
    #[inline]
    pub(crate) fn bits(&self, start: usize, len: u32) -> u32 {
        assert!(
            len <= 32 && start + len as usize <= N,
            "codeword bit range out of range"
        );
        if len == 0 {
            return 0;
        }
        // start ≤ 173, so w ≤ 2 and the padding word makes w + 1 valid.
        let w = start / 64;
        let pair = (u128::from(self.words[w]) << 64) | u128::from(self.words[w + 1]);
        ((pair << (start % 64)) >> (128 - len)) as u32
    }

    /// The first 91 bits (payload and CRC), as a 91-bit word.
    pub fn message(&self) -> u128 {
        (u128::from(self.words[0]) << 27) | u128::from(self.words[1] >> 37)
    }

    /// The 83 parity bits, as an 83-bit word.
    pub fn parity(&self) -> u128 {
        (u128::from(self.words[1] & ((1 << 37) - 1)) << 46) | u128::from(self.words[2] >> 18)
    }

    /// Builds a codeword from 174 bits (each 0 or 1; any non-zero is 1).
    pub fn from_bits(bits: &[u8; N]) -> Self {
        let mut words = [0u64; 4];
        for (i, &b) in bits.iter().enumerate() {
            words[i / 64] |= u64::from(b != 0) << (63 - i % 64);
        }
        Codeword { words }
    }

    /// True when every one of the 83 parity checks sums to zero.
    pub fn satisfies_parity_checks(&self) -> bool {
        PARITY_CHECKS.iter().all(|row| {
            row.iter()
                .filter(|&&k| k != 0)
                .fold(0u8, |acc, &k| acc ^ self.bit(usize::from(k) - 1))
                == 0
        })
    }
}

/// Encodes a 91-bit message-plus-CRC word (bits above 91 are ignored).
pub fn encode(a91: u128) -> Codeword {
    let a91 = a91 & A91_MASK;
    let mut parity = 0u128;
    for row in GENERATOR_ROWS.iter() {
        parity = (parity << 1) | u128::from((row & a91).count_ones() & 1);
    }
    Codeword::from_parts(a91, parity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generator_rows_fit_in_91_bits() {
        for row in GENERATOR_ROWS.iter() {
            assert_eq!(row & !A91_MASK, 0);
        }
    }

    #[test]
    fn every_unit_vector_codeword_satisfies_parity_checks() {
        // By linearity, if every basis codeword satisfies H·c = 0, all do.
        for i in 0..K {
            let cw = encode(1u128 << i);
            assert!(cw.satisfies_parity_checks(), "basis vector {i}");
        }
    }

    #[test]
    fn a_flipped_bit_fails_a_parity_check() {
        let cw = encode(0x0123_4567_89ab_cdef_0123_4567);
        for i in 0..N {
            let mut bits = [0u8; N];
            for (k, b) in bits.iter_mut().enumerate() {
                *b = cw.bit(k);
            }
            bits[i] ^= 1;
            assert!(
                !Codeword::from_bits(&bits).satisfies_parity_checks(),
                "bit {i}"
            );
        }
    }

    #[test]
    fn layout_round_trips_and_bits_reads_across_words() {
        let cw = encode(0x0123_4567_89ab_cdef_0123_4567);
        assert_eq!(cw.message(), 0x0123_4567_89ab_cdef_0123_4567 & A91_MASK);
        let mut all = [0u8; N];
        for (i, b) in all.iter_mut().enumerate() {
            *b = cw.bit(i);
        }
        assert_eq!(Codeword::from_bits(&all), cw);
        // Every length 0..=32 at every legal start, against the bits one
        // at a time (read straight from the words, not through `bits`).
        let one = |i: usize| ((cw.words[i / 64] >> (63 - i % 64)) & 1) as u32;
        let mut cases = 0;
        for len in 0..=32u32 {
            for start in 0..=N - len as usize {
                let want = (start..start + len as usize).fold(0u32, |v, i| (v << 1) | one(i));
                assert_eq!(cw.bits(start, len), want, "start {start} len {len}");
                cases += 1;
            }
        }
        assert_eq!(cases, 33 * 175 - (0..=32).sum::<usize>());
        assert_eq!(cw.bits(N, 0), 0);
        assert!((0..N).all(|i| u32::from(cw.bit(i)) == one(i)));
        assert_eq!(cw.parity() >> 83, 0);
    }

    #[test]
    fn parity_check_table_has_column_weight_three() {
        let mut weight = [0u8; N];
        for row in PARITY_CHECKS.iter() {
            for &k in row.iter().filter(|&&k| k != 0) {
                weight[usize::from(k) - 1] += 1;
            }
        }
        assert!(weight.iter().all(|&w| w == 3));
    }
}
