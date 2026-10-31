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

//! Ordered-statistics decoding of the (174, 91) code, measurement grade
//! (M. P. C. Fossorier and S. Lin, "Soft-decision decoding of linear block
//! codes based on ordered statistics", IEEE Trans. Inf. Theory 41(5),
//! 1995). Not part of the decoder: the sizing round's bench and its
//! Cortex-M33 image use it. `no_std`, no heap, `f32` additions only.
//!
//! Positions are ordered by reliability (|value|, largest first, ties to
//! the lower index). Gauss–Jordan elimination over GF(2) on the
//! generator's 91 rows takes each position in that order as a pivot when
//! it is independent of the pivots so far: the 91 pivots are the most
//! reliable basis (MRB), and row j of the reduced generator is the
//! codeword with a 1 at pivot j and 0 at the other pivots. The order-0
//! codeword takes the hard decisions on the basis; order i also lists
//! every flip of up to i basis bits. The discrepancy D of a codeword is
//! the sum of |value| over the positions where it differs from the hard
//! decisions. With d0 the order-0 codeword's differences and
//! s = |value| where d0 is 0, −|value| where it is 1, a flip set's
//! discrepancy is D(d0) + Σ over its rows' XOR of s; for a pair (j, k),
//! whose rows share no pivot, that is D0 + S_j + S_k − 2 Σ_{R_j ∧ R_k} s.

use cat_ft8::crc;
use cat_ft8::ldpc::{encode, K, N};

/// A word of the bit vectors: the target's own width (64 bits on 64-bit
/// hosts, 32 on the Cortex-M33). Every sum is taken in an order that does
/// not depend on it, so results are the same on every target.
#[cfg(target_pointer_width = "64")]
pub type Word = u64;
#[cfg(not(target_pointer_width = "64"))]
pub type Word = u32;
const WB: usize = Word::BITS as usize;
/// Words of 174 bits.
pub const WORDS: usize = N.div_ceil(WB);

/// 174 bits, bit i at word i / WB, bit i % WB (LSB first).
pub type Bits = [Word; WORDS];

/// Bit i.
#[inline(always)]
pub fn get(b: &Bits, i: usize) -> bool {
    (b[i / WB] >> (i % WB)) & 1 == 1
}

#[inline(always)]
fn xor(a: &Bits, b: &Bits) -> Bits {
    let mut c = *a;
    for (x, y) in c.iter_mut().zip(b) {
        *x ^= y;
    }
    c
}

#[inline(always)]
fn and(a: &Bits, b: &Bits) -> Bits {
    let mut c = *a;
    for (x, y) in c.iter_mut().zip(b) {
        *x &= y;
    }
    c
}

#[inline(always)]
fn ones(x: &Bits) -> u32 {
    x.iter().map(|w| w.count_ones()).sum()
}

/// The generator's rows: row k is the codeword of message bit k (bit k of
/// the 91-bit message part, first bit first).
pub struct Generator {
    pub rows: [Bits; K],
}

impl Generator {
    pub fn new() -> Self {
        let mut rows = [[0; WORDS]; K];
        for (k, row) in rows.iter_mut().enumerate() {
            *row = codeword_bits(1u128 << (K - 1 - k));
        }
        Generator { rows }
    }
}

/// The codeword of a 91-bit message-plus-CRC word, as [`Bits`].
pub fn codeword_bits(a91: u128) -> Bits {
    let cw = encode(a91);
    let mut b = [0; WORDS];
    for i in 0..N {
        b[i / WB] |= Word::from(cw.bit(i)) << (i % WB);
    }
    b
}

/// The 91-bit message part (positions 0..91, first bit most significant).
#[cfg(target_pointer_width = "64")]
#[inline(always)]
pub fn message_part(b: &Bits) -> u128 {
    // Positions 0..64 then 64..91, each reversed so the first is highest.
    (u128::from(b[0].reverse_bits()) << 27)
        | u128::from((b[1] & ((1 << 27) - 1)).reverse_bits() >> 37)
}

/// The 91-bit message part (positions 0..91, first bit most significant).
#[cfg(not(target_pointer_width = "64"))]
#[inline(always)]
pub fn message_part(b: &Bits) -> u128 {
    // Positions 0..32, 32..64 and 64..91, each reversed.
    (u128::from(b[0].reverse_bits()) << 59)
        | (u128::from(b[1].reverse_bits()) << 27)
        | u128::from((b[2] & ((1 << 27) - 1)).reverse_bits() >> 5)
}

/// One list codeword: the flipped basis rows (`K` for none).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Pick {
    pub j: u8,
    pub k: u8,
    pub d: f32,
}

/// The search's result: the least-D codeword (form A) and the least-D
/// codeword whose message part is non-zero and passes the CRC (form B).
#[derive(Clone, Copy, Debug)]
pub struct Found {
    pub a: Pick,
    pub b: Option<Pick>,
}

/// Working storage (about 8.9 KB).
pub struct Osd {
    rows: [Bits; K],
    /// Each reduced row's message part (positions 0..91).
    msg: [u128; K],
    pivot: [u8; K],
    order: [u8; N],
    w: [f32; N],
    hard: Bits,
    /// Order-0 codeword and its differences from the hard decisions.
    c0: Bits,
    d0: Bits,
    m0: u128,
    /// Signed weights: |value| where d0 is 0, −|value| where it is 1.
    s: [f32; N],
    /// D of the order-0 codeword, and each row's Σ s.
    big_d0: f32,
    row_s: [f32; K],
    /// Hamming forms: |d0|, and each row's Σ (1 − 2·d0).
    h0: i32,
    row_h: [i32; K],
    pub total_w: f32,
    /// Σ s over each nibble's set bits, per nibble of the 176-bit form.
    nib: [[[f32; 16]; 4]; NIBBLES / 4],
}

/// Nibbles of the words (176 or 192 bits; those past bit 173 are always 0,
/// and their tables hold +0.0, which leaves a sum unchanged).
const NIBBLES: usize = WORDS * WB / 4;

impl Osd {
    pub const fn new() -> Self {
        Osd {
            rows: [[0; WORDS]; K],
            msg: [0; K],
            pivot: [0; K],
            order: [0; N],
            w: [0.0; N],
            hard: [0; WORDS],
            c0: [0; WORDS],
            d0: [0; WORDS],
            m0: 0,
            s: [0.0; N],
            big_d0: 0.0,
            row_s: [0.0; K],
            h0: 0,
            row_h: [0; K],
            total_w: 0.0,
            nib: [[[0.0; 16]; 4]; NIBBLES / 4],
        }
    }

    /// The hard decisions (1 where the value is negative).
    pub fn hard(&self) -> &Bits {
        &self.hard
    }

    /// The basis positions as a mask.
    pub fn pivot_mask(&self) -> Bits {
        let mut m = [0; WORDS];
        for &p in &self.pivot {
            m[usize::from(p) / WB] |= 1 << (usize::from(p) % WB);
        }
        m
    }

    /// Reliability order and elimination for `values` (positive favours 0).
    pub fn prepare(&mut self, values: &[f32; N], gen: &Generator) {
        let mut hard = [0; WORDS];
        let mut total = 0.0f32;
        for (i, &v) in values.iter().enumerate() {
            let a = v.abs();
            self.w[i] = a;
            total += a;
            hard[i / WB] |= Word::from(v < 0.0) << (i % WB);
        }
        self.total_w = total;
        self.hard = hard;
        for (i, o) in self.order.iter_mut().enumerate() {
            *o = i as u8;
        }
        let w = &self.w;
        self.order.sort_unstable_by(|&x, &y| {
            w[usize::from(y)]
                .total_cmp(&w[usize::from(x)])
                .then(x.cmp(&y))
        });
        self.rows = gen.rows;
        let rows = &mut self.rows;
        let mut rank = 0;
        for &p in self.order.iter() {
            let (wi, bi) = (usize::from(p) / WB, usize::from(p) % WB);
            let mut r = rank;
            while r < K && (rows[r][wi] >> bi) & 1 == 0 {
                r += 1;
            }
            if r == K {
                continue;
            }
            rows.swap(rank, r);
            let piv = rows[rank];
            // Every other row with a 1 in the pivot's column: branch-free
            // on 64-bit hosts (deep pipelines), a branch on the M33.
            for (j, row) in rows.iter_mut().enumerate() {
                if cfg!(target_pointer_width = "64") {
                    let m = Word::wrapping_sub(0, (row[wi] >> bi) & 1 & Word::from(j != rank));
                    for (x, y) in row.iter_mut().zip(&piv) {
                        *x ^= y & m;
                    }
                } else if j != rank && (row[wi] >> bi) & 1 == 1 {
                    for (x, y) in row.iter_mut().zip(&piv) {
                        *x ^= y;
                    }
                }
            }
            self.pivot[rank] = p;
            rank += 1;
            if rank == K {
                break;
            }
        }
        debug_assert_eq!(rank, K);
        // Order 0: the hard decisions on the basis, re-encoded.
        let mut c0 = [0; WORDS];
        for j in 0..K {
            if get(&hard, usize::from(self.pivot[j])) {
                c0 = xor(&c0, &self.rows[j]);
            }
        }
        self.c0 = c0;
        let d0 = xor(&c0, &hard);
        self.d0 = d0;
        self.m0 = message_part(&c0);
        for j in 0..K {
            self.msg[j] = message_part(&self.rows[j]);
        }
        let mut big_d0 = 0.0f32;
        let mut h0 = 0;
        for i in 0..N {
            if get(&d0, i) {
                self.s[i] = -self.w[i];
                big_d0 += self.w[i];
                h0 += 1;
            } else {
                self.s[i] = self.w[i];
            }
        }
        self.big_d0 = big_d0;
        self.h0 = h0;
        for (q, t) in self.nib.iter_mut().flatten().enumerate() {
            t[0] = 0.0;
            for v in 1..16usize {
                let low = v.trailing_zeros() as usize;
                let i = 4 * q + low;
                t[v] = t[v & (v - 1)] + if i < N { self.s[i] } else { 0.0 };
            }
        }
        for j in 0..K {
            let row = self.rows[j];
            let (sj, x) = self.masked_sum(&row);
            self.row_s[j] = sj;
            self.row_h[j] = hamming_form(&x, &d0);
        }
    }

    /// Σ s over the positions set in `x`: one sum per 64 positions (in
    /// position order), then (g0 + g1) + g2. Also returns `x`.
    #[inline(always)]
    fn masked_sum(&self, x: &Bits) -> (f32, Bits) {
        let mut sums = [0.0f32; 3];
        for (wi, &word) in x.iter().enumerate() {
            let sum = &mut sums[wi * WB / 64];
            let s = &self.s[wi * WB..];
            let mut b = word;
            while b != 0 {
                *sum += s[b.trailing_zeros() as usize];
                b &= b - 1;
            }
        }
        ((sums[0] + sums[1]) + sums[2], *x)
    }

    /// Σ s over rows j and k's shared positions (by nibble tables: four
    /// running sums over nibbles q ≡ 0, 1, 2, 3 mod 4, then
    /// (r0 + r1) + (r2 + r3)), and those positions.
    #[inline(always)]
    fn shared(&self, j: usize, k: usize) -> (f32, Bits) {
        let x = and(&self.rows[j], &self.rows[k]);
        let (mut r0, mut r1, mut r2, mut r3) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for (wi, &w) in x.iter().enumerate() {
            // Four nibbles at a time: nibble q goes to sum q mod 4.
            for g in 0..WB / 16 {
                let n = (w >> (16 * g)) as usize;
                let q = &self.nib[wi * (WB / 16) + g];
                r0 += q[0][n & 15];
                r1 += q[1][(n >> 4) & 15];
                r2 += q[2][(n >> 8) & 15];
                r3 += q[3][(n >> 12) & 15];
            }
        }
        let r = [r0, r1, r2, r3];
        ((r[0] + r[1]) + (r[2] + r[3]), x)
    }

    /// The list of order `order` (0, 1 or 2) after [`Osd::prepare`].
    /// `each(d, h, nz)` sees every list codeword's D, Hamming distance from
    /// the hard decisions and whether its message part is non-zero (measurement only; [`Osd::search_fast`] is the
    /// timed form). With `form_b`, also the least-D codeword passing the CRC.
    pub fn search(&self, order: u8, form_b: bool, each: impl FnMut(f32, i32, bool)) -> Found {
        self.search_impl::<true>(order, form_b, each)
    }

    /// [`Osd::search`] without the per-codeword measurement (the timed
    /// form): the same result.
    pub fn search_fast(&self, order: u8, form_b: bool) -> Found {
        self.search_impl::<false>(order, form_b, |_, _, _| {})
    }

    #[allow(clippy::needless_range_loop)]
    fn search_impl<const H: bool>(
        &self,
        order: u8,
        form_b: bool,
        mut each: impl FnMut(f32, i32, bool),
    ) -> Found {
        let none = K as u8;
        let mut a = Pick {
            j: none,
            k: none,
            d: self.big_d0,
        };
        let mut b: Option<Pick> = None;
        // The CRC-14 has a zero start and no final XOR, so its syndrome
        // (the CRC of the payload part XOR the CRC part) is linear: a list
        // codeword's is the XOR of the order-0 codeword's and its rows'.
        let mut syn = [0u16; K];
        let mut syn0 = 0;
        if form_b {
            syn0 = syndrome(self.m0);
            for (s, &m) in syn.iter_mut().zip(&self.msg) {
                *s = syndrome(m);
            }
        }
        let try_b = |s: u16, m: u128, p: Pick, b: &mut Option<Pick>| {
            if s == 0 && b.map_or(true, |q| p.d < q.d) && m != 0 {
                *b = Some(p);
            }
        };
        if H {
            each(self.big_d0, self.h0, self.m0 != 0);
        }
        if form_b {
            try_b(syn0, self.m0, a, &mut b);
        }
        if order >= 1 {
            for j in 0..K {
                let d = self.big_d0 + self.row_s[j];
                if H {
                    each(d, self.h0 + self.row_h[j], self.m0 != self.msg[j]);
                }
                let p = Pick {
                    j: j as u8,
                    k: none,
                    d,
                };
                if d < a.d {
                    a = p;
                }
                if form_b {
                    try_b(syn0 ^ syn[j], self.m0 ^ self.msg[j], p, &mut b);
                }
            }
        }
        if order >= 2 {
            for j in 0..K {
                let base = self.big_d0 + self.row_s[j];
                let hb = self.h0 + self.row_h[j];
                let sj = syn0 ^ syn[j];
                for k in j + 1..K {
                    let (sh, x) = self.shared(j, k);
                    let d = (base + self.row_s[k]) - (sh + sh);
                    if H {
                        let h = hb + self.row_h[k] - 2 * hamming_form(&x, &self.d0);
                        each(d, h, self.m0 ^ self.msg[j] != self.msg[k]);
                    }
                    let p = Pick {
                        j: j as u8,
                        k: k as u8,
                        d,
                    };
                    if d < a.d {
                        a = p;
                    }
                    if form_b && sj == syn[k] {
                        try_b(0, self.m0 ^ self.msg[j] ^ self.msg[k], p, &mut b);
                    }
                }
            }
        }
        Found { a, b }
    }

    /// The codeword of a list entry.
    pub fn codeword(&self, p: &Pick) -> Bits {
        let mut c = self.c0;
        if usize::from(p.j) < K {
            c = xor(&c, &self.rows[usize::from(p.j)]);
        }
        if usize::from(p.k) < K {
            c = xor(&c, &self.rows[usize::from(p.k)]);
        }
        c
    }

    /// D (summed directly, in position order) and the Hamming distance of
    /// `c` from the hard decisions.
    pub fn metrics(&self, c: &Bits) -> (f32, u32) {
        let x = xor(c, &self.hard);
        let mut d = 0.0f32;
        for i in 0..N {
            if get(&x, i) {
                d += self.w[i];
            }
        }
        (d, ones(&x))
    }
}

/// The CRC-14 syndrome of a 91-bit message-plus-CRC word: zero exactly
/// when the CRC holds; linear in the word.
#[inline]
pub fn syndrome(m: u128) -> u16 {
    let p = cat_ft8::message::Payload::from_bits((m >> 14) & ((1 << 77) - 1))
        .expect("77 bits always form a payload");
    crc::crc14(&p) ^ (m & 0x3fff) as u16
}

/// Σ (1 − 2·d0) over the positions set in `x`.
#[inline(always)]
fn hamming_form(x: &Bits, d0: &Bits) -> i32 {
    ones(x) as i32 - 2 * ones(&and(x, d0)) as i32
}

/// Positions where `a` and `b` differ.
pub fn distance(a: &Bits, b: &Bits) -> u32 {
    ones(&xor(a, b))
}

/// Positions in `mask` where `a` and `b` differ.
pub fn distance_in(a: &Bits, b: &Bits, mask: &Bits) -> u32 {
    ones(&and(&xor(a, b), mask))
}

/// Hard decisions of `values` (1 where negative).
pub fn hard_of(values: &[f32; N]) -> Bits {
    let mut b = [0; WORDS];
    for (i, &v) in values.iter().enumerate() {
        b[i / WB] |= Word::from(v < 0.0) << (i % WB);
    }
    b
}
