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

//! Ordered-statistics decoding of order 1 (OSD-1) of the (174, 91) code,
//! the least-discrepancy codeword only (M. P. C. Fossorier and S. Lin,
//! "Soft-decision decoding of linear block codes based on ordered
//! statistics", IEEE Trans. Inf. Theory 41(5), 1995). The decoder runs it
//! on a candidate belief propagation has failed on, when
//! [`DecodeConfig::osd`](super::DecodeConfig::osd) asks for it.
//!
//! Positions are ordered by reliability (|value|, largest first, ties to
//! the lower index). Gauss–Jordan elimination over GF(2) on the
//! generator's 91 rows takes each position in that order as a pivot when
//! it is independent of the pivots so far: the 91 pivots are the most
//! reliable basis, and row j of the reduced generator is the codeword with
//! a 1 at pivot j and 0 at the other pivots. The order-0 codeword takes the
//! hard decisions on the basis; order 1 also lists every one-bit flip of
//! the basis. A codeword's discrepancy D is the sum of |value| over the
//! positions where it differs from the hard decisions. With d0 the order-0
//! codeword's differences and s = |value| where d0 is 0, −|value| where it
//! is 1, flipping basis bit j gives D(d0) + Σ s over row j.
//!
//! `no_std`, no heap, `f32` additions only; every sum is taken in one
//! order on every target, so the result is the same everywhere. Working
//! storage is [`OsdScratch`] (3.6 KB), which the decoder keeps in the
//! candidate's belief-propagation storage, idle once belief propagation
//! has failed.

use super::math::abs;
use crate::ldpc::{GENERATOR_ROWS, K, M, N};

/// A word of the bit vectors: the target's own width (64 bits on 64-bit
/// hosts, 32 on a 32-bit microcontroller).
#[cfg(target_pointer_width = "64")]
type Word = u64;
#[cfg(not(target_pointer_width = "64"))]
type Word = u32;
const WB: usize = Word::BITS as usize;
/// Words of 174 bits.
const WORDS: usize = N.div_ceil(WB);
/// Words per 64 positions (the row sums' groups).
const PER_GROUP: usize = 64 / WB;
const _: () = assert!(WORDS == 3 * PER_GROUP && WORDS * WB >= N);

/// 174 bits, bit i at word i / WB, bit i % WB (LSB first); the bits past
/// 173 are always 0.
type Bits = [Word; WORDS];

/// The generator's rows, in flash: row k is the codeword of message bit k
/// (bit k of the 91-bit message part, first bit first): a 1 at position k
/// and the parity bits `ldpc::encode` gives that bit.
pub(crate) static OSD_ROWS: [Bits; K] = {
    let mut rows = [[0; WORDS]; K];
    let mut k = 0;
    while k < K {
        rows[k][k / WB] |= 1 << (k % WB);
        let mut i = 0;
        while i < M {
            // Parity bit i is position K + i; message bit k is bit
            // K − 1 − k of the 91-bit word.
            if (GENERATOR_ROWS[i] >> (K - 1 - k)) & 1 == 1 {
                let p = K + i;
                rows[k][p / WB] |= 1 << (p % WB);
            }
            i += 1;
        }
        k += 1;
    }
    rows
};

/// OSD-1's working storage (3 576 bytes; the decoder holds it in the
/// candidate's belief-propagation storage).
#[derive(Clone)]
pub struct OsdScratch {
    /// The reduced generator.
    rows: [Bits; K],
    /// Signed weights: |value| where the order-0 codeword agrees with the
    /// hard decisions, −|value| where it differs.
    s: [f32; N],
    /// Positions by reliability, and the radix sort's other buffer.
    order: [u8; N],
    spare: [u8; N],
    /// The basis: row j's pivot position.
    pivot: [u8; K],
    /// The radix sort's digit counts.
    count: [u8; 256],
    /// Belief propagation's rounds for the candidate, kept for
    /// `CandidateScratch::last_rounds`.
    pub(crate) bp_rounds: u8,
}

impl OsdScratch {
    /// Zeroed storage (usable in a `static` initialiser).
    pub const fn new() -> Self {
        OsdScratch {
            rows: [[0; WORDS]; K],
            s: [0.0; N],
            order: [0; N],
            spare: [0; N],
            pivot: [0; K],
            count: [0; 256],
            bp_rounds: 0,
        }
    }
}

impl Default for OsdScratch {
    fn default() -> Self {
        Self::new()
    }
}

/// OSD-1's result: the least-D codeword of the order-0 codeword and its 91
/// one-bit flips.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Order1 {
    /// The flipped basis row (`None`: the order-0 codeword). Ties go to
    /// the first in list order (order 0, then rows 0, 1, …).
    pub pick: Option<u8>,
    /// Its discrepancy D.
    pub d: f32,
    /// Its 91-bit message part (payload and CRC), first bit most
    /// significant.
    pub a91: u128,
    /// Positions where it differs from the hard decisions.
    pub distance: u32,
}

/// Whether the elimination tests each row's bit with a branch (32-bit
/// targets) or masks the XOR (64-bit hosts); see `clear`.
const TARGET_BRANCH: bool = cfg!(not(target_pointer_width = "64"));

/// OSD-1 on `values` (positive favours 0): the least-D codeword.
pub fn order1(values: &[f32; N], scratch: &mut OsdScratch) -> Order1 {
    order1_form::<TARGET_BRANCH>(values, scratch)
}

/// [`order1`] with the elimination's other form (measurement only: the
/// same result, for timing the per-target choice).
#[cfg(feature = "profile")]
pub fn order1_other_form(values: &[f32; N], scratch: &mut OsdScratch) -> Order1 {
    order1_form::<{ !TARGET_BRANCH }>(values, scratch)
}

#[inline(always)]
fn order1_form<const BRANCH: bool>(values: &[f32; N], scratch: &mut OsdScratch) -> Order1 {
    let OsdScratch {
        rows,
        s,
        order,
        spare,
        pivot,
        count,
        ..
    } = scratch;
    reliability_order(values, order, spare, count);
    // The hard decisions: 1 where the value is negative. A value of ±0 is
    // decided 0 here (belief propagation's `hard_bit` splits exact ties by
    // index parity instead): its weight is 0, so D is the same either way,
    // and only the order-0 codeword's bit there (should a zero be in the
    // basis) and the Hamming distance can differ.
    let mut hard = [0; WORDS];
    for (i, &v) in values.iter().enumerate() {
        hard[i / WB] |= Word::from(v < 0.0) << (i % WB);
    }
    *rows = OSD_ROWS;
    eliminate::<BRANCH>(rows, pivot, order);
    // Order 0: the hard decisions on the basis, re-encoded.
    let mut cw0 = [0; WORDS];
    for (row, &p) in rows.iter().zip(pivot.iter()) {
        let p = usize::from(p);
        let m = mask((hard[p / WB] >> (p % WB)) & 1);
        xor_masked(&mut cw0, row, m);
    }
    let d0 = xor(&cw0, &hard);
    let mut d_0 = 0.0f32;
    for (i, (si, &v)) in s.iter_mut().zip(values.iter()).enumerate() {
        let w = abs(v);
        if (d0[i / WB] >> (i % WB)) & 1 == 1 {
            *si = -w;
            d_0 += w;
        } else {
            *si = w;
        }
    }
    // Order 1: each row's flip, least D first in list order.
    let mut pick = None;
    let mut d = d_0;
    for (j, row) in rows.iter().enumerate() {
        let dj = d_0 + row_sum(s, row);
        if dj < d {
            d = dj;
            pick = Some(j as u8);
        }
    }
    // The winner's message part and distance, once.
    let (cw, diff) = match pick {
        Some(j) => {
            let r = &rows[usize::from(j)];
            (xor(&cw0, r), xor(&d0, r))
        }
        None => (cw0, d0),
    };
    Order1 {
        pick,
        d,
        a91: message_part(&cw),
        distance: diff.iter().map(|w| w.count_ones()).sum(),
    }
}

/// `order` = the positions by |value|, largest first, ties to the lower
/// index: a stable least-significant-digit radix sort (four 8-bit digits)
/// on the complement of |value|'s bits, which orders non-negative floats
/// as their values do. `spare` and `count` are its working storage.
fn reliability_order(
    values: &[f32; N],
    order: &mut [u8; N],
    spare: &mut [u8; N],
    count: &mut [u8; 256],
) {
    for (i, o) in order.iter_mut().enumerate() {
        *o = i as u8;
    }
    let key = |i: u8| !(values[usize::from(i)].to_bits() & 0x7fff_ffff);
    // An even number of passes, so the result ends in `order`. Counts and
    // offsets are at most N = 174, so `u8` holds them.
    let (mut src, mut dst) = (order, spare);
    for shift in [0, 8, 16, 24] {
        *count = [0; 256];
        for &i in src.iter() {
            count[usize::from((key(i) >> shift) as u8)] += 1;
        }
        let mut at = 0u8;
        for c in count.iter_mut() {
            let n = *c;
            *c = at;
            at += n;
        }
        for &i in src.iter() {
            let c = &mut count[usize::from((key(i) >> shift) as u8)];
            dst[usize::from(*c)] = i;
            *c += 1;
        }
        core::mem::swap(&mut src, &mut dst);
    }
}

/// Gauss–Jordan elimination of `rows` for the most reliable basis: each
/// position of `order` in turn becomes the next pivot if some row from the
/// current rank has a 1 there; that row moves to the rank and is cleared
/// from every other row with a 1 in its column. `pivot[j]` is row j's.
fn eliminate<const BRANCH: bool>(rows: &mut [Bits; K], pivot: &mut [u8; K], order: &[u8; N]) {
    // The message columns any cleared pivot row has had a 1 in. The
    // generator is systematic, so a message column outside this set still
    // holds its single 1, and pivoting there clears nothing.
    let mut touched: Bits = [0; WORDS];
    let mut rank = 0;
    for &p in order.iter() {
        let (wi, bi) = (usize::from(p) / WB, usize::from(p) % WB);
        let Some(r) = (rank..K).find(|&r| (rows[r][wi] >> bi) & 1 == 1) else {
            continue;
        };
        rows.swap(rank, r);
        if usize::from(p) >= K || (touched[wi] >> bi) & 1 == 1 {
            let piv = rows[rank];
            // Every row with a 1 in the column, the pivot row included
            // (put back after); see `clear` for the per-target form.
            for row in rows.iter_mut() {
                clear::<BRANCH>(row, &piv, wi, bi);
            }
            rows[rank] = piv;
            for (t, x) in touched.iter_mut().zip(&piv) {
                *t |= x;
            }
        }
        pivot[rank] = p;
        rank += 1;
        if rank == K {
            break;
        }
    }
    // The generator has full rank, so every row finds a pivot.
    debug_assert_eq!(rank, K);
}

/// `row ^= piv` when `row` has a 1 at word `wi`, bit `bi`: with `BRANCH`
/// a test and a jump (32-bit targets, where skipping the XOR saves its six
/// words), otherwise a masked XOR (64-bit hosts, whose deep pipelines
/// mispredict the random test). The choice per target is measured with
/// `paired_bench osdlib`.
#[inline(always)]
fn clear<const BRANCH: bool>(row: &mut Bits, piv: &Bits, wi: usize, bi: usize) {
    let bit = (row[wi] >> bi) & 1;
    if !BRANCH {
        xor_masked(row, piv, mask(bit));
    } else if bit == 1 {
        xor_masked(row, piv, Word::MAX);
    }
}

/// All ones for 1, zero for 0.
#[inline(always)]
fn mask(bit: Word) -> Word {
    bit.wrapping_neg()
}

/// `row ^= piv & m`, written out (the release profile for small targets,
/// `opt-level = "s"`, does not unroll the loop).
#[inline(always)]
fn xor_masked(row: &mut Bits, piv: &Bits, m: Word) {
    #[cfg(target_pointer_width = "64")]
    {
        row[0] ^= piv[0] & m;
        row[1] ^= piv[1] & m;
        row[2] ^= piv[2] & m;
    }
    #[cfg(not(target_pointer_width = "64"))]
    {
        row[0] ^= piv[0] & m;
        row[1] ^= piv[1] & m;
        row[2] ^= piv[2] & m;
        row[3] ^= piv[3] & m;
        row[4] ^= piv[4] & m;
        row[5] ^= piv[5] & m;
    }
}

#[inline(always)]
fn xor(a: &Bits, b: &Bits) -> Bits {
    let mut c = *a;
    xor_masked(&mut c, b, Word::MAX);
    c
}

/// Σ s over the positions set in `row`: one running sum per 64 positions,
/// each in position order, then (g0 + g1) + g2.
#[inline(always)]
fn row_sum(s: &[f32; N], row: &Bits) -> f32 {
    let group = |g: usize| {
        let mut acc = 0.0f32;
        for k in 0..PER_GROUP {
            let base = (g * PER_GROUP + k) * WB;
            let mut b = row[g * PER_GROUP + k];
            while b != 0 {
                acc += s[base + b.trailing_zeros() as usize];
                b &= b - 1;
            }
        }
        acc
    };
    (group(0) + group(1)) + group(2)
}

/// The 91-bit message part (positions 0..91, first bit most significant).
#[cfg(target_pointer_width = "64")]
#[inline(always)]
fn message_part(b: &Bits) -> u128 {
    // Positions 0..64 then 64..91, each reversed so the first is highest.
    (u128::from(b[0].reverse_bits()) << 27)
        | u128::from((b[1] & ((1 << 27) - 1)).reverse_bits() >> 37)
}

/// The 91-bit message part (positions 0..91, first bit most significant).
#[cfg(not(target_pointer_width = "64"))]
#[inline(always)]
fn message_part(b: &Bits) -> u128 {
    // Positions 0..32, 32..64 and 64..91, each reversed.
    (u128::from(b[0].reverse_bits()) << 59)
        | (u128::from(b[1].reverse_bits()) << 27)
        | u128::from((b[2] & ((1 << 27) - 1)).reverse_bits() >> 5)
}

#[cfg(test)]
mod tests;
