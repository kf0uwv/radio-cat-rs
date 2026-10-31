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

//! LDPC decoding by belief propagation (ADR 0024, amendment "single-pass
//! decoder"): message passing on the code's parity-check graph (Gallager,
//! 1962; MacKay, 1999) over the crate's parity-check table, with a
//! flooding schedule and a fixed order of every sum, then a CRC check.
//!
//! Two check-node rules (both `f32`-only):
//!
//! - [`CheckRule::SumProduct`]: m = 2 atanh(Π tanh(q / 2)) over the other
//!   edges, with tanh(x/2) = (1 − e⁻ˣ) / (1 + e⁻ˣ) from the in-crate
//!   `exp2` and atanh from the in-crate `log2`;
//! - [`CheckRule::MinSum`]: m = α · (Π sign q) · min |q| over the other
//!   edges (normalised min-sum; Chen and Fossorier, 2002), α = [`ALPHA`].
//!
//! Signs: a positive value favours bit 0. A decode succeeds when every
//! parity check holds, the CRC matches, and the payload is not all zeros:
//! the all-zero codeword passes every check and the CRC (zero initial
//! value), so a carrier or other narrow-band interference could otherwise
//! "decode" as it.

use super::math::{exp2_neg, log2_power, LOG2_E};
use crate::crc;
use crate::ldpc::{Codeword, N};
use crate::ldpc_tables::PARITY_CHECKS;
use crate::message::Payload;

/// Parity checks.
const M: usize = 83;
/// ln 2.
const LN2: f32 = core::f32::consts::LN_2;
/// Largest |tanh product| before atanh, so the message stays finite
/// (≈ 16.6 in magnitude).
const MAX_PRODUCT: f32 = 1.0 - 1.192_092_9e-7;

/// The normalisation factor α of [`CheckRule::MinSum`]. Chosen by
/// measurement on synthetic slots: 0.875 decoded as many signals (a tie
/// within the measurement's noise) and 0.625 fewer; 0.75, the value in use
/// when they were compared, was kept.
pub const ALPHA: f32 = 0.75;

/// The check-node rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckRule {
    /// Sum-product (the tanh rule).
    SumProduct,
    /// Normalised min-sum with scale α = 0.75.
    MinSum,
}

/// Working storage for one decode (3 656 bytes). One per decoding thread;
/// every decode re-initialises it.
#[derive(Debug, Clone)]
pub struct BpScratch {
    /// The totals and the next round's sums. Each round swaps their
    /// roles: after an even number of rounds the totals are in `total`,
    /// after an odd number in `next`.
    total: [f32; N],
    next: [f32; N],
    /// The check-to-variable messages, one per edge, laid out by check
    /// degree ([`check_edges`]).
    msg: [f32; EDGES],
    /// The hard decisions of the last totals whose checks all held.
    hard: [u8; N],
    /// Message-passing rounds run by the last decode, whatever its result.
    rounds: u8,
}

impl BpScratch {
    /// Zeroed storage (usable in a `static` initialiser).
    pub const fn new() -> Self {
        BpScratch {
            total: [0.0; N],
            next: [0.0; N],
            msg: [0.0; EDGES],
            hard: [0; N],
            rounds: 0,
        }
    }

    /// Message-passing rounds the last decode ran, converged or not (for
    /// cost measurement).
    pub fn rounds(&self) -> u8 {
        self.rounds
    }
}

impl Default for BpScratch {
    fn default() -> Self {
        Self::new()
    }
}

/// A successful decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BpDecode {
    /// The 77-bit payload.
    pub payload: Payload,
    /// Message-passing rounds used (0: the channel's own hard decisions
    /// were already a codeword).
    pub rounds: u8,
    /// Codeword bits that differ from the channel's hard decisions,
    /// saturating at 255. Bits whose soft value is exactly 0 (erased or
    /// tied) have no channel decision and are not counted.
    pub hard_errors: u8,
}

/// Why a decode failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BpFailure {
    /// No codeword within the round limit.
    NotConverged,
    /// A codeword whose CRC does not match.
    Crc,
    /// The all-zero codeword (rejected).
    AllZero,
}

/// The parity checks with 0-based variable indices (unused entries 0),
/// derived at compile time from the 1-based, 0-padded `PARITY_CHECKS`,
/// in the same order.
pub(crate) static CHECKS: [[u8; MAX_DEGREE]; M] = {
    let mut out = [[0u8; 7]; M];
    let mut c = 0;
    while c < M {
        let mut n = 0;
        let mut j = 0;
        while j < MAX_DEGREE {
            if PARITY_CHECKS[c][j] != 0 {
                out[c][n] = PARITY_CHECKS[c][j] - 1;
                n += 1;
            }
            j += 1;
        }
        c += 1;
    }
    out
};

/// The number of variables in each check (6 or 7).
pub(crate) static DEGREES: [u8; M] = {
    let mut out = [0u8; M];
    let mut c = 0;
    while c < M {
        let mut j = 0;
        while j < MAX_DEGREE {
            out[c] += (PARITY_CHECKS[c][j] != 0) as u8;
            j += 1;
        }
        c += 1;
    }
    out
};

/// The largest check degree: the width of the parity-check table's rows.
const MAX_DEGREE: usize = 7;

/// One check's values at most (the sum-product products' prefixes have
/// one more entry).
const LANES: usize = 8;
const _: () = assert!(LANES >= MAX_DEGREE);

/// Variable indices (0-based) and count of check `c`.
#[inline]
fn check(c: usize) -> (&'static [u8; MAX_DEGREE], usize) {
    (&CHECKS[c], usize::from(DEGREES[c]))
}

/// Whether the target has a vector unit the round's loops use (SSE2 on
/// x86-64, NEON on AArch64; not the Cortex-M33).
const VECTOR: bool = cfg!(any(target_feature = "sse2", target_feature = "neon"));

/// tanh(x / 2) for any finite x.
#[inline]
fn tanh_half(x: f32) -> f32 {
    let negative = x < 0.0;
    // |x|. Where the round's loops vectorise, by clearing the sign bit:
    // with the select `if negative { -x } else { x }` the compiler moved
    // the select past `exp2_neg` and the division, evaluating the whole
    // chain for both x and −x in every lane and choosing at the end (two
    // divisions per value). Elsewhere by the select, which becomes a
    // branch to one of the two chains: fewer instructions per value than
    // the masked form on the Cortex-M33. The results are the same: the
    // forms differ only for −0.0 and NaN, whose e is 1.0 and 0.0 either
    // way.
    let a = if VECTOR {
        f32::from_bits(x.to_bits() & 0x7fff_ffff)
    } else if negative {
        -x
    } else {
        x
    };
    let e = exp2_neg(-a * LOG2_E); // e^(−|x|)
    let t = (1.0 - e) / (1.0 + e);
    if negative {
        -t
    } else {
        t
    }
}

/// 2 · atanh(p) = ln((1 + p) / (1 − p)), with |p| capped below 1.
#[inline]
fn twice_atanh(p: f32) -> f32 {
    let a = if p < 0.0 { -p } else { p };
    let a = if a > MAX_PRODUCT { MAX_PRODUCT } else { a };
    let m = LN2 * log2_power((1.0 + a) / (1.0 - a));
    if p < 0.0 {
        -m
    } else {
        m
    }
}

/// The hard decision of value `x` at bit `i`. An exact tie (0, e.g. an
/// erased bit no check has informed) goes to the bit's index parity:
/// deterministic, and favouring neither value, so ties do not pull towards
/// the all-zero codeword.
#[inline]
fn hard_bit(x: f32, i: usize) -> u8 {
    u8::from(x < 0.0 || (x == 0.0 && i % 2 == 1))
}

/// True if every parity check holds for the hard decisions of `values`.
/// Each check decides its own bits as it is tested, so a round that fails
/// stops at its first failing check without deciding the other bits.
fn checks_hold(values: &[f32; N]) -> bool {
    (0..M).all(|c| {
        let (v, n) = check(c);
        v[..n].iter().fold(0u8, |acc, &i| {
            let i = usize::from(i);
            acc ^ hard_bit(values[i], i)
        }) == 0
    })
}

/// Hard decisions of `values` into `hard`.
fn hard_decisions(values: &[f32; N], hard: &mut [u8; N]) {
    for (i, (h, &x)) in hard.iter_mut().zip(values.iter()).enumerate() {
        *h = hard_bit(x, i);
    }
}

/// The payload of a codeword in hard decisions, after the CRC and the
/// all-zero test.
fn accept(hard: &[u8; N]) -> Result<Payload, BpFailure> {
    let cw = Codeword::from_bits(hard);
    let a91 = cw.message();
    if a91 == 0 {
        return Err(BpFailure::AllZero);
    }
    if !crc::check(a91) {
        return Err(BpFailure::Crc);
    }
    // Cannot fail: `a91 >> 14` has at most 77 bits. The `Crc` arm only
    // avoids a panic path.
    Payload::from_bits(a91 >> 14).ok_or(BpFailure::Crc)
}

/// Decodes soft bits `llr` (positive favours 0) with at most `max_rounds`
/// rounds of message passing.
pub fn decode_soft(
    llr: &[f32; N],
    rule: CheckRule,
    max_rounds: u8,
    scratch: &mut BpScratch,
) -> Result<BpDecode, BpFailure> {
    let BpScratch {
        total,
        next,
        msg,
        hard,
        rounds: rounds_run,
    } = scratch;
    *rounds_run = 0;
    *total = *llr;
    *msg = [0.0; EDGES];
    // Each round sums into `next`, then the two swap roles: the
    // references swap, not the arrays' contents.
    let (mut total, mut next) = (total, next);
    let hard_errors = |hard: &[u8; N]| {
        let n = hard
            .iter()
            .zip(llr.iter())
            .filter(|(&h, &l)| l != 0.0 && h != u8::from(l < 0.0))
            .count();
        n.min(255) as u8
    };
    // Round 0 is the channel's own hard decisions.
    let mut round = 0;
    loop {
        if checks_hold(total) {
            hard_decisions(total, hard);
            let payload = accept(hard)?;
            return Ok(BpDecode {
                payload,
                rounds: round,
                hard_errors: hard_errors(hard),
            });
        }
        if round == max_rounds {
            return Err(BpFailure::NotConverged);
        }
        round += 1;
        *rounds_run = round;
        *next = *llr;
        if rule == CheckRule::SumProduct {
            sum_product_round(total, next, msg);
        } else {
            min_sum_round(total, next, msg);
        }
        core::mem::swap(&mut total, &mut next);
    }
}

/// Checks of degree 6 (the others have degree 7), from the parity-check
/// table at compile time.
const LOW_CHECKS: usize = {
    let mut count = 0;
    let mut c = 0;
    while c < M {
        let mut n = 0;
        let mut j = 0;
        while j < MAX_DEGREE {
            n += (PARITY_CHECKS[c][j] != 0) as usize;
            j += 1;
        }
        assert!(n == 6 || n == 7);
        count += (n == 6) as usize;
        c += 1;
    }
    count
};
/// The edges of the degree-6 checks, which come first in the messages'
/// layout ([`check_edges`]); then those of the degree-7 checks.
const LOW_EDGES: usize = 6 * LOW_CHECKS;
/// Edges of the parity-check graph (522).
const EDGES: usize = LOW_EDGES + 7 * (M - LOW_CHECKS);

/// Each check in order: its variables (0-based) and the index of its
/// first edge in the messages' layout. The edges are laid out by check
/// degree: the degree-6 checks' edges first, check by check in order, then
/// the degree-7 checks'. Every place the layout is walked takes it from
/// here.
#[inline(always)]
fn check_edges() -> impl Iterator<Item = (&'static [u8], usize)> {
    let (mut low, mut high) = (0, LOW_EDGES);
    (0..M).map(move |c| {
        let (v, n) = check(c);
        let place = if n == 6 { &mut low } else { &mut high };
        let e = *place;
        *place += n;
        (&v[..n], e)
    })
}

/// One sum-product round, in place on the messages: every check's new
/// messages into `msg`, added to `next` check by check in order. Done in
/// phases over all edges (inputs, tanh, products, atanh, then the sums), so
/// the tanh and atanh passes are long plain loops; each value's arithmetic
/// is that of one check at a time. With the edges laid out by check degree
/// ([`check_edges`]), the products run over whole checks of a fixed size.
/// Not inlined, so its frame is measured on its own.
#[inline(never)]
fn sum_product_round(total: &[f32; N], next: &mut [f32; N], msg: &mut [f32; EDGES]) {
    for (v, e) in check_edges() {
        for (m, &i) in msg[e..][..v.len()].iter_mut().zip(v) {
            *m = total[usize::from(i)] - *m;
        }
    }
    each_value::<PASS_LANES>(msg, tanh_half);
    let (t6, t7) = msg.split_at_mut(LOW_EDGES);
    for tc in t6.chunks_exact_mut(6) {
        others_products::<6>(tc);
    }
    for tc in t7.chunks_exact_mut(7) {
        others_products::<7>(tc);
    }
    each_value::<PASS_LANES>(msg, twice_atanh);
    for (v, e) in check_edges() {
        for (&m, &i) in msg[e..][..v.len()].iter().zip(v) {
            next[usize::from(i)] += m;
        }
    }
}

/// Values the tanh and atanh passes take together: several whole vectors
/// where the target has a vector unit, one elsewhere. Each value's
/// operations are the same at any count; only the order of the
/// independent values' instructions changes. With one vector at a time,
/// each value's chain (`exp2_neg` or `log2`, and the divisions) is a long
/// dependent sequence and the Cortex-A76 overlapped few of them; several
/// side by side keep its vector pipes busy. The count was chosen by
/// measurement on the Cortex-A76.
const PASS_LANES: usize = if VECTOR { 24 } else { 1 };

/// `f` applied to every value, `W` values at a time, then the rest.
#[inline(always)]
fn each_value<const W: usize>(values: &mut [f32; EDGES], f: impl Fn(f32) -> f32) {
    let mut blocks = values.chunks_exact_mut(W);
    for block in &mut blocks {
        let block: &mut [f32; W] = block.try_into().unwrap();
        for x in block.iter_mut() {
            *x = f(*x);
        }
    }
    for x in blocks.into_remainder() {
        *x = f(*x);
    }
}

/// One normalised min-sum round, in place on the messages: check by check
/// in order, the inputs (totals less the check's own messages) replace its
/// messages, then its new messages, which are added to `next`. Not
/// inlined, so its frame is measured on its own.
#[inline(never)]
fn min_sum_round(total: &[f32; N], next: &mut [f32; N], msg: &mut [f32; EDGES]) {
    for (v, e) in check_edges() {
        let row = &mut msg[e..][..v.len()];
        // Where the loops vectorise, the check's size as a type, so the
        // compiler drops the vector forms of these short loops (which no
        // check reaches); elsewhere one instance for both sizes.
        match (VECTOR, v.len()) {
            (true, 6) => min_sum_check::<[u8; 6], [f32; 6]>(
                total,
                next,
                v.try_into().unwrap(),
                row.try_into().unwrap(),
            ),
            (true, _) => min_sum_check::<[u8; 7], [f32; 7]>(
                total,
                next,
                v.try_into().unwrap(),
                row.try_into().unwrap(),
            ),
            (false, _) => min_sum_check(total, next, v, row),
        }
    }
}

/// [`min_sum_round`] for one check: its variables `v` and its messages
/// `row`, of the same length.
#[inline(always)]
fn min_sum_check<V, R>(total: &[f32; N], next: &mut [f32; N], v: &V, row: &mut R)
where
    V: AsRef<[u8]> + ?Sized,
    R: AsMut<[f32]> + ?Sized,
{
    let (v, row) = (v.as_ref(), row.as_mut());
    for (m, &i) in row.iter_mut().zip(v) {
        *m = total[usize::from(i)] - *m;
    }
    let (mut min1, mut min2, mut at) = (f32::MAX, f32::MAX, 0);
    let mut negatives = 0u32;
    for (j, &x) in row.iter().enumerate() {
        let a = if x < 0.0 { -x } else { x };
        negatives += u32::from(x < 0.0);
        if a < min1 {
            min2 = min1;
            min1 = a;
            at = j;
        } else if a < min2 {
            min2 = a;
        }
    }
    for (j, x) in row.iter_mut().enumerate() {
        let others_negative = negatives - u32::from(*x < 0.0);
        let mag = ALPHA * if j == at { min2 } else { min1 };
        *x = if others_negative % 2 == 1 { -mag } else { mag };
    }
    for (&m, &i) in row.iter().zip(v) {
        next[usize::from(i)] += m;
    }
}

/// Each value of a check replaced by the product of the others, by prefix
/// and suffix, in place.
#[inline(always)]
fn others_products<const D: usize>(tc: &mut [f32]) {
    let tc: &mut [f32; D] = tc.try_into().unwrap();
    let mut prefix = [1.0f32; LANES + 1];
    for j in 0..D {
        prefix[j + 1] = prefix[j] * tc[j];
    }
    let mut suffix = 1.0f32;
    for j in (0..D).rev() {
        let tj = tc[j];
        tc[j] = prefix[j] * suffix;
        suffix *= tj;
    }
}

#[cfg(test)]
mod tests;
