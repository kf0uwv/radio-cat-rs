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

use super::math::{exp2_neg, log2_power, LOG2_E, VECTOR};
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
    /// degree in groups ([`place`]).
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

/// tanh(x / 2) for any finite x, in the form for targets where the loops
/// vectorise (`V`) or not; the decoder uses [`VECTOR`]'s, the tests both.
#[inline]
fn tanh_half<const V: bool>(x: f32) -> f32 {
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
    let a = if V {
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
/// stops at its first failing check without deciding the other bits. `G`
/// is the layout's group size; the decoder uses [`GROUP`], the tests
/// both.
fn checks_hold<const G: usize>(values: &[f32; N]) -> bool {
    let (edge_vars, _) = tables::<G>();
    let decide = |acc: u8, &i: &u8| {
        let i = usize::from(i);
        acc ^ hard_bit(values[i], i)
    };
    check_edges::<G>().all(|(e, n, k)| {
        let parity = if k == 1 {
            edge_vars[e..][..n].iter().fold(0u8, decide)
        } else {
            edge_vars[e..].iter().step_by(k).take(n).fold(0u8, decide)
        };
        parity == 0
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
        if checks_hold::<GROUP>(total) {
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
        if rule == CheckRule::SumProduct {
            sum_product_round::<VECTOR, PASS_LANES, GROUP>(llr, total, next, msg);
        } else {
            min_sum_round::<VECTOR, GROUP>(llr, total, next, msg);
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
/// layout ([`place`]); then those of the degree-7 checks.
const LOW_EDGES: usize = 6 * LOW_CHECKS;
/// Edges of the parity-check graph (522).
const EDGES: usize = LOW_EDGES + 7 * (M - LOW_CHECKS);

/// Checks of one degree whose edges are interleaved in the messages'
/// layout ([`place`]): four where the round's loops vectorise, so that
/// the sum-product products run over four checks in the lanes of one
/// vector; one elsewhere, check by check.
const GROUP: usize = if VECTOR { 4 } else { 1 };

/// The messages' layout, in groups of `group` checks: the first edge of
/// check `index` among the checks of `degree` (6 or 7), and the stride
/// between its edges. The degree-6 checks' edges come first, then the
/// degree-7 checks', each degree's checks in check order. They are taken
/// `group` at a time, and within a group edge j of its k-th check is at
/// the group's first edge + j·group + k; the checks left over after the
/// last whole group are laid out one after another, stride 1. With a
/// group of one this is check by check.
///
/// The tables ([`var_edges`]) and the walk in check order
/// ([`check_edges`]) are built from this function; its `group == 1` case
/// is unrolled there (each degree's next check starts where the last one
/// ended), which keeps the Cortex-M33's code as it was. The rounds' group
/// loops ([`products`], [`min_sum_round`]) do not call it: they rely on
/// its shape, each degree's edges as contiguous blocks of D·G values for
/// the whole groups followed by the leftover checks of D values each.
#[inline(always)]
const fn place(degree: usize, index: usize, group: usize) -> (usize, usize) {
    let (first, count) = if degree == 6 {
        (0, LOW_CHECKS)
    } else {
        (LOW_EDGES, M - LOW_CHECKS)
    };
    if index < count / group * group {
        (
            first + index / group * group * degree + index % group,
            group,
        )
    } else {
        (first + index * degree, 1)
    }
}

/// Each check in order: its first edge in the messages' layout of
/// groups of `G` ([`place`]), its degree, and the stride between its
/// edges. A check's variables are its edges' entries of [`EDGE_VARS`],
/// so the graph is stored once. With groups of one, each degree's next
/// check starts where the last one ended, so the walk keeps each
/// degree's next edge rather than its count of checks.
#[inline(always)]
fn check_edges<const G: usize>() -> impl Iterator<Item = (usize, usize, usize)> {
    let (mut low, mut high) = (0, if G == 1 { LOW_EDGES } else { 0 });
    (0..M).map(move |c| {
        let n = usize::from(DEGREES[c]);
        let at = if n == 6 { &mut low } else { &mut high };
        if G == 1 {
            let e = *at;
            *at += n;
            (e, n, 1)
        } else {
            let (e, k) = place(n, *at, G);
            *at += 1;
            (e, n, k)
        }
    })
}

/// The graph's tables in the layout of groups of `G`: the decoder's
/// statics for [`GROUP`]; in the tests, their copies for the other group
/// size, so that both layouts are tested on any host.
#[inline(always)]
fn tables<const G: usize>() -> (&'static [u8; EDGES], &'static [[u16; 3]; N]) {
    #[cfg(test)]
    if G != GROUP {
        return tests::tables_for(G);
    }
    assert!(G == GROUP);
    (&EDGE_VARS, &VAR_EDGES)
}

/// One sum-product round, in place on the messages: every check's new
/// messages into `msg`, added to `next` check by check in order. Done in
/// phases over all edges (inputs, tanh, products, atanh, then the sums), so
/// the tanh and atanh passes are long plain loops; each value's arithmetic
/// is that of one check at a time. With the edges laid out by check degree
/// in groups of `G` ([`place`]), the products run over whole groups of a
/// fixed size. `V` picks the form of `tanh_half`, `W` the values the tanh
/// and atanh passes take together and `G` the layout; the decoder uses
/// [`VECTOR`], [`PASS_LANES`] and [`GROUP`], the tests both targets' forms.
/// Not inlined, so its frame is measured on its own.
#[inline(never)]
fn sum_product_round<const V: bool, const W: usize, const G: usize>(
    llr: &[f32; N],
    total: &[f32; N],
    next: &mut [f32; N],
    msg: &mut [f32; EDGES],
) {
    let (edge_vars, var_edges) = tables::<G>();
    if INPUT_BLOCK == 1 {
        for (m, &i) in msg.iter_mut().zip(edge_vars) {
            *m = total[usize::from(i)] - *m;
        }
    } else {
        inputs::<INPUT_BLOCK>(total, msg, edge_vars);
    }
    each_value::<W>(msg, tanh_half::<V>);
    let (t6, t7) = msg.split_at_mut(LOW_EDGES);
    products::<6, G>(t6);
    products::<7, G>(t7);
    each_value::<W>(msg, twice_atanh);
    sums(llr, next, msg, var_edges);
}

/// Edges whose inputs [`inputs`] takes together: eight on AArch64, one
/// elsewhere. The gather is a scalar loop bound by the instructions it
/// issues on the Cortex-A76; blocks issue one count and branch per eight
/// edges. On x86-64 blocks did not help (findings O17).
const INPUT_BLOCK: usize = if cfg!(target_arch = "aarch64") { 8 } else { 1 };

/// The inputs: each edge's message replaced by its variable's total
/// (`edge_vars`) less the message, `U` edges at a time, then the rest.
/// Each edge's subtraction is the same at any `U`; where `INPUT_BLOCK` is
/// one, the round has the plain loop instead. `INPUT_BLOCK` is not a
/// parameter of the round, so the round with blocked inputs is tested
/// only where it is built, on AArch64; elsewhere the tests compare
/// `inputs` itself at several block sizes with the plain loop.
#[inline(always)]
fn inputs<const U: usize>(total: &[f32; N], msg: &mut [f32; EDGES], edge_vars: &[u8; EDGES]) {
    let mut blocks = msg.chunks_exact_mut(U);
    let mut vars = edge_vars.chunks_exact(U);
    for (block, v) in (&mut blocks).zip(&mut vars) {
        let block: &mut [f32; U] = block.try_into().unwrap();
        let v: &[u8; U] = v.try_into().unwrap();
        for k in 0..U {
            block[k] = total[usize::from(v[k])] - block[k];
        }
    }
    for (m, &i) in blocks.into_remainder().iter_mut().zip(vars.remainder()) {
        *m = total[usize::from(i)] - *m;
    }
}

/// The products over the edges of one degree's checks in the layout of
/// groups of `G` ([`place`]): with groups of one, check by check; else the
/// whole groups, then the checks left over one by one.
#[inline(always)]
fn products<const D: usize, const G: usize>(edges: &mut [f32]) {
    let rest = if G == 1 {
        edges
    } else {
        let mut groups = edges.chunks_exact_mut(D * G);
        for group in &mut groups {
            group_products::<D, G>(group);
        }
        groups.into_remainder()
    };
    for tc in rest.chunks_exact_mut(D) {
        others_products::<D>(tc);
    }
}

/// Each variable's three edges in the messages' layout ([`place`]), in
/// check order: every variable is in exactly three checks.
pub(crate) static VAR_EDGES: [[u16; 3]; N] = var_edges(GROUP);

/// Each edge's variable, in the messages' layout ([`place`]).
pub(crate) static EDGE_VARS: [u8; EDGES] = edge_vars(&VAR_EDGES);

/// [`VAR_EDGES`] in the layout of groups of `group`.
const fn var_edges(group: usize) -> [[u16; 3]; N] {
    let mut out = [[0u16; 3]; N];
    let mut seen = [0usize; N];
    let (mut low, mut high) = (0, 0);
    let mut c = 0;
    while c < M {
        let mut n = 0;
        let mut j = 0;
        while j < MAX_DEGREE {
            n += (PARITY_CHECKS[c][j] != 0) as usize;
            j += 1;
        }
        let (first, stride) = place(n, if n == 6 { low } else { high }, group);
        let (mut j, mut k) = (0, 0);
        while j < MAX_DEGREE {
            if PARITY_CHECKS[c][j] != 0 {
                let i = (PARITY_CHECKS[c][j] - 1) as usize;
                assert!(seen[i] < 3);
                out[i][seen[i]] = (first + k * stride) as u16;
                seen[i] += 1;
                k += 1;
            }
            j += 1;
        }
        if n == 6 {
            low += 1;
        } else {
            high += 1;
        }
        c += 1;
    }
    let mut i = 0;
    while i < N {
        assert!(seen[i] == 3);
        i += 1;
    }
    out
}

/// [`EDGE_VARS`] from the variables' edges `var_edges`.
const fn edge_vars(var_edges: &[[u16; 3]; N]) -> [u8; EDGES] {
    let mut out = [0u8; EDGES];
    let mut i = 0;
    while i < N {
        let mut k = 0;
        while k < 3 {
            out[var_edges[i][k] as usize] = i as u8;
            k += 1;
        }
        i += 1;
    }
    out
}

/// The next totals: each variable's soft bit plus its three checks'
/// messages (at `var_edges`), added in check order.
#[inline(always)]
fn sums(llr: &[f32; N], next: &mut [f32; N], msg: &[f32; EDGES], var_edges: &[[u16; 3]; N]) {
    for ((x, &l), e) in next.iter_mut().zip(llr).zip(var_edges) {
        *x = l + msg[usize::from(e[0])] + msg[usize::from(e[1])] + msg[usize::from(e[2])];
    }
}

/// Values the tanh and atanh passes take together: several whole vectors
/// where the target has a vector unit, one elsewhere. Each value's
/// operations are the same at any count; only the order of the
/// independent values' instructions changes. With one vector at a time,
/// each value's chain (`exp2_neg` or `log2`, and the divisions) is a long
/// dependent sequence and the Cortex-A76 overlapped few of them; several
/// side by side keep its vector pipes busy. The count was chosen by
/// measurement on the Cortex-A76. The gain depends on the compiler fully
/// unrolling the block (it did not unroll 32 values), so a toolchain
/// change can lose it with every test passing: re-measure the round after
/// one.
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
/// messages, then its new messages, which are added to `next`. `V` picks
/// the form for targets where the loops vectorise or not and `G` the
/// layout (the decoder uses [`VECTOR`] and [`GROUP`], the tests both).
/// With groups of more than one ([`place`]), each degree's whole groups
/// come first ([`group_min_sum`]), then its checks left over; the checks
/// are independent, so their order does not matter. Not inlined, so its
/// frame is measured on its own.
#[inline(never)]
fn min_sum_round<const V: bool, const G: usize>(
    llr: &[f32; N],
    total: &[f32; N],
    next: &mut [f32; N],
    msg: &mut [f32; EDGES],
) {
    let (edge_vars, var_edges) = tables::<G>();
    if G == 1 {
        for (e, n, _) in check_edges::<G>() {
            let (v, row) = (&edge_vars[e..][..n], &mut msg[e..][..n]);
            min_sum_check_sized::<V>(total, v, row);
        }
    } else {
        let (m6, m7) = msg.split_at_mut(LOW_EDGES);
        let (v6, v7) = edge_vars.split_at(LOW_EDGES);
        min_sum_groups::<V, 6, G>(total, v6, m6);
        min_sum_groups::<V, 7, G>(total, v7, m7);
    }
    sums(llr, next, msg, var_edges);
}

/// [`min_sum_round`]'s checks of degree `D` in the layout of groups of
/// `G` ([`place`]): their variables `vars` and messages `edges`, the whole
/// groups, then the checks left over one by one.
#[inline(always)]
fn min_sum_groups<const V: bool, const D: usize, const G: usize>(
    total: &[f32; N],
    vars: &[u8],
    edges: &mut [f32],
) {
    let mut groups = edges.chunks_exact_mut(D * G);
    let mut group_vars = vars.chunks_exact(D * G);
    for (group, v) in (&mut groups).zip(&mut group_vars) {
        group_min_sum::<D, G>(total, v, group);
    }
    let rest = groups.into_remainder().chunks_exact_mut(D);
    for (row, v) in rest.zip(group_vars.remainder().chunks_exact(D)) {
        min_sum_check_sized::<V>(total, v, row);
    }
}

/// [`min_sum_check`] on one check's variables `v` and messages `row`.
/// Where the loops vectorise, the check's size as a constant (the slices
/// cut to 6 or 7 once this is inlined), so the compiler drops the vector
/// forms of these short loops, which no check reaches; elsewhere one form
/// for both sizes, its inputs taken in the loop that finds the two
/// smallest magnitudes.
#[inline(always)]
fn min_sum_check_sized<const V: bool>(total: &[f32; N], v: &[u8], row: &mut [f32]) {
    match (V, v.len()) {
        (true, 6) => min_sum_check::<false>(total, &v[..6], &mut row[..6]),
        (true, _) => min_sum_check::<false>(total, &v[..7], &mut row[..7]),
        (false, _) => min_sum_check::<true>(total, v, row),
    }
}

/// [`min_sum_round`] for one check: its variables `v` and its messages
/// `row`, of the same length. With `ONE_PASS` each input (the variable's
/// total less the message) is taken in the loop that finds the two
/// smallest magnitudes; otherwise all of them first, in a loop of their
/// own. Each value goes through the same operations in the same order
/// either way.
#[inline(always)]
fn min_sum_check<const ONE_PASS: bool>(total: &[f32; N], v: &[u8], row: &mut [f32]) {
    let (mut min1, mut min2, mut at) = (f32::MAX, f32::MAX, 0);
    let mut negatives = 0u32;
    // One input's magnitude and sign into the two smallest and the count.
    // (A macro rather than a closure: the hosts' two loops compile as they
    // did before, their binaries unchanged.)
    macro_rules! take {
        ($j:expr, $x:expr) => {{
            let (j, x): (usize, f32) = ($j, $x);
            let a = if x < 0.0 { -x } else { x };
            negatives += u32::from(x < 0.0);
            if a < min1 {
                min2 = min1;
                min1 = a;
                at = j;
            } else if a < min2 {
                min2 = a;
            }
        }};
    }
    if ONE_PASS {
        for (j, (m, &i)) in row.iter_mut().zip(v).enumerate() {
            *m = total[usize::from(i)] - *m;
            take!(j, *m);
        }
    } else {
        for (m, &i) in row.iter_mut().zip(v) {
            *m = total[usize::from(i)] - *m;
        }
        for (j, &x) in row.iter().enumerate() {
            take!(j, x);
        }
    }
    for (j, x) in row.iter_mut().enumerate() {
        let others_negative = negatives - u32::from(*x < 0.0);
        let mag = ALPHA * if j == at { min2 } else { min1 };
        *x = if others_negative % 2 == 1 { -mag } else { mag };
    }
}

/// [`min_sum_check`] for a group of `K` checks of degree `D` laid out as
/// in [`place`] (edge j of the group's k-th check at j·K + k), with their
/// variables `vars`: each check's comparisons, counts and messages are
/// those of one check alone, and the group's K checks take the K lanes of
/// each step. The branches that keep the two smallest magnitudes are
/// selects here, which choose the same values.
#[inline(always)]
fn group_min_sum<const D: usize, const K: usize>(total: &[f32; N], vars: &[u8], group: &mut [f32]) {
    let mut x = [[0.0f32; K]; MAX_DEGREE];
    for (j, xj) in x.iter_mut().enumerate().take(D) {
        for l in 0..K {
            xj[l] = total[usize::from(vars[j * K + l])] - group[j * K + l];
        }
    }
    let (mut min1, mut min2) = ([f32::MAX; K], [f32::MAX; K]);
    let (mut at, mut negatives) = ([0u32; K], [0u32; K]);
    for (j, xj) in x.iter().enumerate().take(D) {
        for l in 0..K {
            let v = xj[l];
            let a = if v < 0.0 { -v } else { v };
            negatives[l] += u32::from(v < 0.0);
            let (below1, below2) = (a < min1[l], a < min2[l]);
            min2[l] = if below1 {
                min1[l]
            } else if below2 {
                a
            } else {
                min2[l]
            };
            min1[l] = if below1 { a } else { min1[l] };
            at[l] = if below1 { j as u32 } else { at[l] };
        }
    }
    for (j, xj) in x.iter().enumerate().take(D) {
        for l in 0..K {
            let others_negative = negatives[l] - u32::from(xj[l] < 0.0);
            let mag = ALPHA * if at[l] == j as u32 { min2[l] } else { min1[l] };
            group[j * K + l] = if others_negative % 2 == 1 { -mag } else { mag };
        }
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

/// [`others_products`] for a group of `K` checks of degree `D` laid out
/// as in [`place`] (edge j of the group's k-th check at j·K + k): each
/// check's products are those of one check alone, in the same order, and
/// the group's K checks take the K lanes of each step. The group's values
/// are all loaded before any is stored, so the compiler keeps the lanes
/// together; single checks keep the in-place [`others_products`], which
/// is the better form on the Cortex-M33 (findings O17).
#[inline(always)]
fn group_products<const D: usize, const K: usize>(group: &mut [f32]) {
    let mut t = [[0.0f32; K]; D];
    for (j, tj) in t.iter_mut().enumerate() {
        *tj = group[j * K..][..K].try_into().unwrap();
    }
    let mut prefix = [[1.0f32; K]; LANES + 1];
    for j in 0..D {
        for l in 0..K {
            prefix[j + 1][l] = prefix[j][l] * t[j][l];
        }
    }
    let mut suffix = [1.0f32; K];
    for j in (0..D).rev() {
        let mut out = [0.0f32; K];
        for l in 0..K {
            out[l] = prefix[j][l] * suffix[l];
        }
        for l in 0..K {
            suffix[l] *= t[j][l];
        }
        group[j * K..][..K].copy_from_slice(&out);
    }
}

#[cfg(test)]
mod tests;

/// Measurement only (`profile`): not part of the crate's API.
#[cfg(feature = "profile")]
impl BpScratch {
    /// The totals after the last decode's final round (the channel's
    /// soft bits if it ran none), whatever its result.
    pub fn last_totals(&self) -> &[f32; N] {
        if self.rounds % 2 == 0 {
            &self.total
        } else {
            &self.next
        }
    }
}
