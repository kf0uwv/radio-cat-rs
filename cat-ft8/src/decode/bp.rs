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

/// Working storage for one decode (about 4.4 KB). One per decoding thread;
/// every decode re-initialises it.
#[derive(Debug, Clone)]
pub struct BpScratch {
    total: [f32; N],
    next: [f32; N],
    msg: [[f32; 7]; M],
    hard: [u8; N],
}

impl BpScratch {
    /// Zeroed storage (usable in a `static` initialiser).
    pub const fn new() -> Self {
        BpScratch {
            total: [0.0; N],
            next: [0.0; N],
            msg: [[0.0; 7]; M],
            hard: [0; N],
        }
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
pub(crate) static CHECKS: [[u8; 7]; M] = {
    let mut out = [[0u8; 7]; M];
    let mut c = 0;
    while c < M {
        let mut n = 0;
        let mut j = 0;
        while j < 7 {
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
        while j < 7 {
            out[c] += (PARITY_CHECKS[c][j] != 0) as u8;
            j += 1;
        }
        c += 1;
    }
    out
};

/// Variable indices (0-based) and count of check `c`.
#[inline]
fn check(c: usize) -> (&'static [u8; 7], usize) {
    (&CHECKS[c], usize::from(DEGREES[c]))
}

/// tanh(x / 2) for any finite x.
#[inline]
fn tanh_half(x: f32) -> f32 {
    let negative = x < 0.0;
    let a = if negative { -x } else { x };
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

/// Hard decisions of `values` into `hard`; true if every check holds.
fn hard_decide(values: &[f32; N], hard: &mut [u8; N]) -> bool {
    // An exact tie (0, e.g. an erased bit no check has informed) goes to
    // the bit's index parity: deterministic, and favouring neither value,
    // so ties do not pull towards the all-zero codeword.
    for (i, (h, &x)) in hard.iter_mut().zip(values.iter()).enumerate() {
        *h = u8::from(x < 0.0 || (x == 0.0 && i % 2 == 1));
    }
    (0..M).all(|c| {
        let (v, n) = check(c);
        v[..n]
            .iter()
            .fold(0u8, |acc, &i| acc ^ hard[usize::from(i)])
            == 0
    })
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
    } = scratch;
    *total = *llr;
    for row in msg.iter_mut() {
        *row = [0.0; 7];
    }
    let hard_errors = |hard: &[u8; N]| {
        let n = hard
            .iter()
            .zip(llr.iter())
            .filter(|(&h, &l)| l != 0.0 && h != u8::from(l < 0.0))
            .count();
        n.min(255) as u8
    };
    if hard_decide(total, hard) {
        let payload = accept(hard)?;
        return Ok(BpDecode {
            payload,
            rounds: 0,
            hard_errors: hard_errors(hard),
        });
    }
    for round in 1..=max_rounds {
        *next = *llr;
        for (c, row) in msg.iter_mut().enumerate() {
            let (v, n) = check(c);
            let mut q = [0.0f32; 7];
            for j in 0..n {
                q[j] = total[usize::from(v[j])] - row[j];
            }
            match rule {
                CheckRule::SumProduct => {
                    // Fixed eight lanes (unused ones hold 0.0 and are
                    // discarded) so the tanh and atanh passes vectorise;
                    // each used lane's arithmetic is unchanged.
                    let mut t = [0.0f32; 8];
                    let mut qq = [0.0f32; 8];
                    qq[..7].copy_from_slice(&q);
                    for (tj, &x) in t.iter_mut().zip(qq.iter()) {
                        *tj = tanh_half(x);
                    }
                    // Products of all the others, by prefix and suffix, in
                    // the same order as before.
                    let mut prefix = [1.0f32; 8];
                    for j in 0..n {
                        prefix[j + 1] = prefix[j] * t[j];
                    }
                    let mut others = [0.0f32; 8];
                    let mut suffix = 1.0f32;
                    for j in (0..n).rev() {
                        others[j] = prefix[j] * suffix;
                        suffix *= t[j];
                    }
                    let mut m = [0.0f32; 8];
                    for (mj, &p) in m.iter_mut().zip(others.iter()) {
                        *mj = twice_atanh(p);
                    }
                    row[..n].copy_from_slice(&m[..n]);
                }
                CheckRule::MinSum => {
                    let (mut min1, mut min2, mut at) = (f32::MAX, f32::MAX, 0);
                    let mut negatives = 0u32;
                    for (j, &x) in q.iter().enumerate().take(n) {
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
                    for (j, &x) in q.iter().enumerate().take(n) {
                        let others_negative = negatives - u32::from(x < 0.0);
                        let mag = ALPHA * if j == at { min2 } else { min1 };
                        row[j] = if others_negative % 2 == 1 { -mag } else { mag };
                    }
                }
            }
            for j in 0..n {
                next[usize::from(v[j])] += row[j];
            }
        }
        core::mem::swap(total, next);
        if hard_decide(total, hard) {
            let payload = accept(hard)?;
            return Ok(BpDecode {
                payload,
                rounds: round,
                hard_errors: hard_errors(hard),
            });
        }
    }
    Err(BpFailure::NotConverged)
}

#[cfg(test)]
mod tests;
