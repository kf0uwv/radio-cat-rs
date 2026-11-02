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

extern crate std;

use super::*;
use crate::decode::testutil::Noise;
use crate::message::{pack, NoHashes};
use crate::{crc, ldpc};

const RULES: [CheckRule; 2] = [CheckRule::SumProduct, CheckRule::MinSum];

/// The parity checks with 0-based variable indices (unused entries 0),
/// derived at compile time from the 1-based, 0-padded `PARITY_CHECKS`,
/// in the same order: the tests' own form of the graph, independent of
/// the edge layout the decoder walks.
static CHECKS: [[u8; MAX_DEGREE]; M] = {
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

/// The graph's tables in the layout of groups of one and of four
/// ([`place`]): the decoder's statics for its own group size, these for the
/// other ([`tables`]).
static VAR_EDGES_1: [[u16; 3]; N] = var_edges(1);
static EDGE_VARS_1: [u8; EDGES] = edge_vars(&VAR_EDGES_1);
static VAR_EDGES_4: [[u16; 3]; N] = var_edges(4);
static EDGE_VARS_4: [u8; EDGES] = edge_vars(&VAR_EDGES_4);

/// The tables for groups of `g` (one or four).
pub(super) fn tables_for(g: usize) -> (&'static [u8; EDGES], &'static [[u16; 3]; N]) {
    match g {
        1 => (&EDGE_VARS_1, &VAR_EDGES_1),
        4 => (&EDGE_VARS_4, &VAR_EDGES_4),
        _ => panic!("no tables for groups of {g}"),
    }
}

/// Each check's edges in the layout of groups of `G`, in check order.
fn edges_of<const G: usize>() -> std::vec::Vec<std::vec::Vec<usize>> {
    check_edges::<G>()
        .map(|(e, n, k)| (0..n).map(|j| e + j * k).collect())
        .collect()
}

/// Variable indices (0-based) and count of check `c`.
fn check(c: usize) -> (&'static [u8; MAX_DEGREE], usize) {
    (&CHECKS[c], usize::from(DEGREES[c]))
}

fn codeword(message: &str) -> (Payload, Codeword) {
    let p = pack(message, &mut NoHashes).unwrap();
    (p, ldpc::encode(crc::append_crc(&p)))
}

/// Channel values for a codeword: ±mean (positive for bit 0) plus Gaussian
/// noise of `sigma`.
fn channel(cw: &Codeword, mean: f32, sigma: f64, seed: u64) -> [f32; N] {
    let mut n = Noise(seed);
    let mut llr = [0.0f32; N];
    for (i, l) in llr.iter_mut().enumerate() {
        let s = if cw.bit(i) == 0 { mean } else { -mean };
        *l = s + (sigma * n.gauss()) as f32;
    }
    llr
}

#[test]
fn a_clean_codeword_decodes_in_zero_rounds() {
    let (p, cw) = codeword("CQ K1ABC FN42");
    let llr = channel(&cw, 4.0, 0.0, 1);
    for rule in RULES {
        let d = decode_soft(&llr, rule, 30, &mut BpScratch::new()).unwrap();
        assert_eq!((d.payload, d.rounds, d.hard_errors), (p, 0, 0));
    }
}

#[test]
fn the_all_zero_codeword_is_rejected() {
    let llr = [4.0f32; N]; // every bit 0
    for rule in RULES {
        assert_eq!(
            decode_soft(&llr, rule, 30, &mut BpScratch::new()),
            Err(BpFailure::AllZero)
        );
    }
}

#[test]
fn hard_decisions_split_exact_ties_by_index() {
    let mut hard = [9u8; N];
    for tie in [0.0f32, -0.0] {
        // Every value an exact tie (−0.0 included): bit i goes to i % 2.
        hard_decisions(&[tie; N], &mut hard);
        for (i, &h) in hard.iter().enumerate() {
            assert_eq!(usize::from(h), i % 2, "tie {tie:?} at {i}");
        }
    }
    // Signs decide everywhere else, whatever the index.
    let mut values = [0.0f32; N];
    for (i, v) in values.iter_mut().enumerate() {
        *v = if i % 3 == 0 { -1.0e-30 } else { 2.5 };
    }
    hard_decisions(&values, &mut hard);
    for (i, &h) in hard.iter().enumerate() {
        assert_eq!(h, u8::from(i % 3 == 0), "at {i}");
    }
}

/// The stopping test as first written: every bit decided, then the
/// checks over the decisions.
fn hard_decide_first_form(values: &[f32; N], hard: &mut [u8; N]) -> bool {
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

#[test]
fn the_stopping_test_matches_its_first_form() {
    // Codewords with a few bits flipped or erased (±0), so both outcomes
    // occur, and random states. Most first failures are at low checks
    // here; `the_stopping_test_fails_first_at_every_check` covers every
    // position.
    let (_, cw) = codeword("K1ABC W9XYZ RR73");
    let clean = channel(&cw, 2.0, 0.0, 1);
    let mut value = random_values(0x57_0b);
    let mut word = 0x9e37_79b9_u64;
    let mut held = 0;
    for state in 0..20_000 {
        let mut values = clean;
        if state % 2 == 0 {
            values.iter_mut().for_each(|x| *x = value());
        } else {
            for _ in 0..state % 5 {
                word = word.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                let i = (word >> 33) as usize % N;
                values[i] = match (word >> 20) % 3 {
                    0 => -values[i],
                    1 => 0.0,
                    _ => -0.0,
                };
            }
        }
        let mut want = [0u8; N];
        let holds = hard_decide_first_form(&values, &mut want);
        assert_eq!(checks_hold::<1>(&values), holds, "state {state}");
        assert_eq!(checks_hold::<4>(&values), holds, "state {state}");
        held += usize::from(holds);
        let mut hard = [9u8; N];
        hard_decisions(&values, &mut hard);
        assert_eq!(hard, want, "state {state}");
    }
    assert!(held > 1_000, "{held}");
}

/// The first check that fails for hard decisions `hard`, if any.
fn first_failing_check(hard: &[u8; N]) -> Option<usize> {
    (0..M).find(|&c| {
        let (v, n) = check(c);
        v[..n]
            .iter()
            .fold(0u8, |acc, &i| acc ^ hard[usize::from(i)])
            != 0
    })
}

/// Bits to flip in a codeword so that exactly the checks of `syndrome`
/// (bit c for check c) fail, by elimination over GF(2): the 83 checks are
/// independent (91 = 174 − 83 message bits), so every syndrome has a
/// solution.
fn flips_for(syndrome: u128) -> std::vec::Vec<usize> {
    type Bits = [u64; 3];
    let flip = |b: &mut Bits, o: &Bits| b.iter_mut().zip(o).for_each(|(x, y)| *x ^= y);
    // basis[p]: a set of bits whose syndrome's lowest check is p.
    let mut basis: [Option<(u128, Bits)>; M] = [None; M];
    for i in 0..N {
        let mut s = 0u128;
        for c in 0..M {
            let (v, n) = check(c);
            if v[..n].contains(&(i as u8)) {
                s |= 1 << c;
            }
        }
        let mut bits: Bits = [0; 3];
        bits[i / 64] |= 1 << (i % 64);
        while s != 0 {
            let p = s.trailing_zeros() as usize;
            match &basis[p] {
                Some((bs, bb)) => {
                    s ^= bs;
                    flip(&mut bits, bb);
                }
                None => {
                    basis[p] = Some((s, bits));
                    break;
                }
            }
        }
    }
    let (mut s, mut bits) = (syndrome, [0u64; 3]);
    while s != 0 {
        let (bs, bb) = basis[s.trailing_zeros() as usize].expect("full rank");
        s ^= bs;
        flip(&mut bits, &bb);
    }
    (0..N)
        .filter(|&i| bits[i / 64] >> (i % 64) & 1 == 1)
        .collect()
}

#[test]
fn the_stopping_test_fails_first_at_every_check() {
    // For every check c, codewords with bits flipped so that c is the
    // first check to fail: c alone, and c with random later checks. Some
    // states also erase bits to ±0 where the tie rule keeps the decision,
    // or scale the values, so ties and magnitudes vary too.
    let (_, cw) = codeword("K1ABC W9XYZ RR73");
    let clean = channel(&cw, 2.0, 0.0, 1);
    let mut word = 0x2545_f491_4f6c_dd1d_u64;
    let mut next_word = move || {
        word ^= word << 13;
        word ^= word >> 7;
        word ^= word << 17;
        word
    };
    let mut seen = [0u32; M];
    for (c, seen) in seen.iter_mut().enumerate() {
        let above = !0u128 << (c + 1) & ((1u128 << M) - 1);
        for variant in 0..12 {
            let later = if variant % 2 == 0 {
                0
            } else {
                (u128::from(next_word()) << 64 | u128::from(next_word())) & above
            };
            let mut values = clean;
            for i in flips_for(1 << c | later) {
                values[i] = -values[i];
            }
            if variant >= 4 {
                let scale = (next_word() % 1_000) as f32 / 100.0 + 0.01;
                for (i, x) in values.iter_mut().enumerate() {
                    let r = next_word();
                    if r % 4 == 0 && u8::from(*x < 0.0) == (i % 2) as u8 {
                        *x = if r & 16 == 0 { 0.0 } else { -0.0 };
                    } else {
                        *x *= scale;
                    }
                }
            }
            let mut want = [0u8; N];
            let holds = hard_decide_first_form(&values, &mut want);
            assert_eq!(first_failing_check(&want), Some(c), "check {c}");
            assert!(!holds);
            assert_eq!(
                checks_hold::<1>(&values),
                holds,
                "check {c}, variant {variant}"
            );
            assert_eq!(
                checks_hold::<4>(&values),
                holds,
                "check {c}, variant {variant}"
            );
            *seen += 1;
        }
        // The codeword itself holds.
        assert!(checks_hold::<1>(&clean) && checks_hold::<4>(&clean));
    }
    assert!(seen.iter().all(|&n| n == 12), "{seen:?}");
}

#[test]
fn hard_errors_skip_erased_bits() {
    // A clean codeword with its first 24 bits erased: the decode
    // restores them but none had a channel decision to be wrong about.
    let (p, cw) = codeword("CQ DX K1ABC FN42");
    let mut llr = channel(&cw, 4.0, 0.0, 1);
    for x in llr[..24].iter_mut() {
        *x = 0.0;
    }
    assert!((0..24).any(|i| cw.bit(i) == 1), "needs some erased 1 bits");
    for rule in RULES {
        let d = decode_soft(&llr, rule, 30, &mut BpScratch::new()).unwrap();
        assert_eq!((d.payload, d.hard_errors), (p, 0), "{rule:?}");
    }
    // One flipped (not erased) bit is counted.
    llr[100] = -llr[100];
    for rule in RULES {
        let d = decode_soft(&llr, rule, 30, &mut BpScratch::new()).unwrap();
        assert_eq!((d.payload, d.hard_errors), (p, 1), "{rule:?}");
    }
}

#[test]
fn an_erased_input_never_decodes() {
    // Every soft bit 0: ties split by index, so not the all-zero word.
    let llr = [0.0f32; N];
    for rule in RULES {
        assert_eq!(
            decode_soft(&llr, rule, 30, &mut BpScratch::new()),
            Err(BpFailure::NotConverged)
        );
    }
}

#[test]
fn noisy_channels_decode_with_both_rules() {
    // Mean 2, σ 1.4 (Eb/N0 about 1.3 dB at this code rate): most decode.
    let (p, cw) = codeword("K1ABC W9XYZ -11");
    for rule in RULES {
        let mut ok = 0;
        let mut scratch = BpScratch::new();
        for seed in 0..100 {
            let llr = channel(&cw, 2.0, 1.4, 100 + seed);
            if let Ok(d) = decode_soft(&llr, rule, 30, &mut scratch) {
                assert_eq!(d.payload, p);
                assert!(d.rounds > 0 && d.hard_errors > 0);
                ok += 1;
            }
        }
        std::println!("{rule:?}: {ok}/100 decoded");
        assert!(ok >= 80, "{rule:?}: {ok}");
    }
}

#[test]
fn erasures_are_recovered() {
    let (p, cw) = codeword("CQ DX K1ABC FN42");
    let mut llr = channel(&cw, 3.0, 0.8, 7);
    for x in llr[..24].iter_mut() {
        *x = 0.0; // the first eight symbols erased
    }
    for rule in RULES {
        assert_eq!(
            decode_soft(&llr, rule, 30, &mut BpScratch::new())
                .unwrap()
                .payload,
            p
        );
    }
}

#[test]
fn pure_noise_never_passes() {
    let mut n = Noise(42);
    let mut scratch = BpScratch::new();
    for rule in RULES {
        let mut passed = 0;
        for _ in 0..300 {
            let mut llr = [0.0f32; N];
            for l in llr.iter_mut() {
                *l = (2.0 * n.gauss()) as f32;
            }
            passed += usize::from(decode_soft(&llr, rule, 30, &mut scratch).is_ok());
        }
        assert_eq!(passed, 0, "{rule:?}");
    }
}

#[test]
fn a_used_scratch_gives_the_same_result() {
    let (_, cw) = codeword("W9XYZ K1ABC R-09");
    let llr = channel(&cw, 2.0, 1.3, 9);
    for rule in RULES {
        let fresh = decode_soft(&llr, rule, 30, &mut BpScratch::new());
        let mut used = BpScratch::new();
        let _ = decode_soft(&channel(&cw, 1.0, 2.0, 10), rule, 30, &mut used);
        assert_eq!(decode_soft(&llr, rule, 30, &mut used), fresh);
    }
}

#[test]
fn tanh_and_atanh_helpers() {
    for x in [-30.0f32, -5.0, -0.5, 0.0, 0.3, 2.0, 9.0, 40.0] {
        let exact = libm::tanh(f64::from(x) / 2.0);
        for t in [tanh_half::<true>(x), tanh_half::<false>(x)] {
            assert!((f64::from(t) - exact).abs() < 2e-6, "{x}");
        }
    }
    for p in [-0.999f32, -0.5, 0.0, 0.25, 0.9, 0.999_99] {
        let exact = 2.0 * libm::atanh(f64::from(p));
        assert!(
            (f64::from(twice_atanh(p)) - exact).abs() < 1e-4 * (1.0 + exact.abs()),
            "{p}"
        );
    }
    assert!(twice_atanh(1.0).is_finite() && twice_atanh(1.0) > 16.0);
}

/// `tanh_half` as first written (the magnitude by a select): the
/// reference the current form must match bit for bit.
fn tanh_half_first_form(x: f32) -> f32 {
    let negative = x < 0.0;
    let a = if negative { -x } else { x };
    let e = exp2_neg(-a * LOG2_E);
    let t = (1.0 - e) / (1.0 + e);
    if negative {
        -t
    } else {
        t
    }
}

/// Every 2⁻ᵏ step of the bit patterns plus the special values, or (with
/// `step` 1) all 2³² of them; both targets' forms.
fn tanh_half_matches_its_first_form(step: usize) {
    let specials = [
        0.0f32,
        -0.0,
        f32::MIN_POSITIVE,
        -f32::MIN_POSITIVE,
        f32::from_bits(1),
        f32::from_bits(0x8000_0001),
        f32::MAX,
        f32::MIN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        -f32::NAN,
        f32::from_bits(0x7f80_0001),
        f32::from_bits(0xffc0_0001),
        87.336_54,
        -87.336_54,
        175.0,
        -175.0,
    ];
    let patterns = (0..=u32::MAX).step_by(step).map(f32::from_bits);
    for x in specials.into_iter().chain(patterns) {
        let want = tanh_half_first_form(x).to_bits();
        for (form, got) in [
            ("vector", tanh_half::<true>(x)),
            ("scalar", tanh_half::<false>(x)),
        ] {
            assert_eq!(got.to_bits(), want, "{form}: {x:e} ({:#010x})", x.to_bits());
        }
    }
}

#[test]
fn tanh_half_is_its_first_form_bit_for_bit() {
    tanh_half_matches_its_first_form(4_099);
}

/// All 2³² inputs, both forms (about two minutes in release).
#[test]
#[ignore]
fn tanh_half_is_its_first_form_bit_for_bit_everywhere() {
    tanh_half_matches_its_first_form(1);
}

#[test]
fn the_check_table_is_the_parity_check_matrix_0_based() {
    for (c, row) in PARITY_CHECKS.iter().enumerate() {
        let want: std::vec::Vec<u8> = row.iter().filter(|&&k| k != 0).map(|&k| k - 1).collect();
        let (v, n) = check(c);
        assert_eq!(&v[..n], &want[..], "check {c}");
        assert!(n == 6 || n == 7);
        assert!(v[n..].iter().all(|&x| x == 0));
    }
}

/// Messages in the layout of groups of `G` as rows by check, in check
/// order (unused lanes 0.0).
fn rows_of<const G: usize>(msg: &[f32; EDGES]) -> [[f32; MAX_DEGREE]; M] {
    let mut rows = [[0.0f32; MAX_DEGREE]; M];
    for (row, edges) in rows.iter_mut().zip(edges_of::<G>()) {
        for (r, e) in row.iter_mut().zip(edges) {
            *r = msg[e];
        }
    }
    rows
}

/// Rows by check written back into messages in the layout of groups of
/// `G`.
fn rows_into<const G: usize>(rows: &[[f32; MAX_DEGREE]; M], msg: &mut [f32; EDGES]) {
    for (row, edges) in rows.iter().zip(edges_of::<G>()) {
        for (&r, e) in row.iter().zip(edges) {
            msg[e] = r;
        }
    }
}

/// FNV-1a over the bits of the scratch's totals and messages (as rows by
/// check, so the hash does not depend on the messages' layout), whichever
/// array the last round left the totals in.
fn state_hash(s: &BpScratch) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let rows = rows_of::<GROUP>(&s.msg);
    let totals = if s.rounds % 2 == 0 { &s.total } else { &s.next };
    let words = totals.iter().chain(rows.iter().flatten());
    for x in words {
        for b in x.to_bits().to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

#[test]
fn message_passing_state_is_pinned_bit_for_bit() {
    // A fixed soft-bit vector that is no codeword and does not converge in
    // five rounds, so every round runs; the final totals and messages are
    // hashed bit for bit. Any change in the arithmetic of a round, the
    // check-node rules or `exp2`/`log2` (even one ulp) changes the hash.
    let mut llr = [0.0f32; N];
    for (i, l) in llr.iter_mut().enumerate() {
        *l = ((i * 7_919) % 1_000) as f32 / 125.0 - 4.0;
    }
    let pinned = [
        (CheckRule::SumProduct, 0x59dd_5a55_fda7_edc7u64),
        (CheckRule::MinSum, 0xc981_1426_8889_7698u64),
    ];
    for (rule, want) in pinned {
        let mut scratch = BpScratch::new();
        assert_eq!(
            decode_soft(&llr, rule, 5, &mut scratch),
            Err(BpFailure::NotConverged)
        );
        let got = state_hash(&scratch);
        assert_eq!(got, want, "{rule:?}: {got:#018x}");
    }
}

#[test]
fn the_scratch_records_the_rounds_run() {
    let (_, cw) = codeword("CQ K1ABC FN42");
    let mut scratch = BpScratch::new();
    let clean = channel(&cw, 4.0, 0.0, 1);
    assert_eq!(
        decode_soft(&clean, CheckRule::SumProduct, 30, &mut scratch)
            .unwrap()
            .rounds,
        0
    );
    assert_eq!(scratch.rounds(), 0);
    let mut llr = [0.0f32; N];
    for (i, l) in llr.iter_mut().enumerate() {
        *l = ((i * 7_919) % 1_000) as f32 / 125.0 - 4.0;
    }
    for rule in RULES {
        assert_eq!(
            decode_soft(&llr, rule, 7, &mut scratch),
            Err(BpFailure::NotConverged)
        );
        assert_eq!(scratch.rounds(), 7);
    }
    let noisy = channel(&cw, 2.0, 1.4, 105);
    if let Ok(d) = decode_soft(&noisy, CheckRule::SumProduct, 30, &mut scratch) {
        assert_eq!(scratch.rounds(), d.rounds);
    }
}

#[test]
fn the_edge_layout_places_every_edge_once_for_both_group_sizes() {
    assert_eq!((LOW_CHECKS, LOW_EDGES, EDGES), (59, 354, 522));
    assert_eq!(tables_for(GROUP), (&EDGE_VARS, &VAR_EDGES));
    assert!(core::ptr::eq(tables::<GROUP>().0, &EDGE_VARS));
    edge_layout_is::<1>();
    edge_layout_is::<4>();
}

/// The layout of groups of `G` ([`place`]), checked from the tables of
/// [`tables`] and [`check_edges`] against the parity-check table.
fn edge_layout_is<const G: usize>() {
    let (edge_vars, var_edges) = tables::<G>();
    let mut used = [0u8; EDGES];
    let mut per_variable = [0u8; N];
    // Per degree: the checks so far, and the first edge of their region.
    let mut seen = [0usize; 2];
    for (c, (e, n, k)) in check_edges::<G>().enumerate() {
        assert_eq!(n, check(c).1, "check {c}");
        let (d, first, count) = if n == 6 {
            (0, 0, LOW_CHECKS)
        } else {
            (1, LOW_EDGES, M - LOW_CHECKS)
        };
        let index = seen[d];
        seen[d] += 1;
        // Checks in whole groups are interleaved: edge j of the group's
        // k-th check at the group's first edge + j·G + k. The rest are
        // laid out one after another.
        let whole = count / G * G;
        let want = if index < whole {
            (first + index / G * G * n + index % G, G)
        } else {
            (first + index * n, 1)
        };
        assert_eq!((e, k), want, "check {c}");
        let edges: std::vec::Vec<usize> = (0..n).map(|j| e + j * k).collect();
        let vars: std::vec::Vec<u8> = edges.iter().map(|&x| edge_vars[x]).collect();
        assert_eq!(vars, &check(c).0[..n], "check {c}");
        for &x in &edges {
            assert!(x >= first && x < first + count * n, "check {c} at {x}");
            used[x] += 1;
            per_variable[usize::from(edge_vars[x])] += 1;
        }
    }
    assert_eq!(seen, [LOW_CHECKS, M - LOW_CHECKS]);
    assert!(used.iter().all(|&u| u == 1), "{used:?}");
    assert!(per_variable.iter().all(|&d| d == 3), "{per_variable:?}");
    // Each variable's edges, in check order.
    for (i, ve) in var_edges.iter().enumerate() {
        let want: std::vec::Vec<u16> = edges_of::<G>()
            .iter()
            .enumerate()
            .flat_map(|(c, edges)| {
                let (v, n) = check(c);
                (0..n)
                    .filter(move |&j| usize::from(v[j]) == i)
                    .map(move |j| edges[j] as u16)
            })
            .collect();
        assert_eq!(&ve[..], &want[..], "variable {i}");
    }
}

/// Values over a wide range, with ±0, subnormals and values up to ±1e6
/// mixed in (splitmix64 words from `seed`).
fn random_values(mut seed: u64) -> impl FnMut() -> f32 {
    let mut word = move || {
        seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    move || {
        let w = word();
        let sign = if w & 16 == 0 { 1.0 } else { -1.0 };
        match w % 16 {
            0 => 0.0,
            1 => -0.0,
            2 => sign * f32::from_bits((w >> 8) as u32 & 0x007f_ffff),
            3 => ((w >> 8) % 2_000_001) as f32 - 1.0e6,
            _ => ((w >> 8) % 400_001) as f32 / 10_000.0 - 20.0,
        }
    }
}

/// The sum-product round in its first form, check by check over rows of
/// the parity-check table: the reference the edge-layout round must match
/// bit for bit.
fn reference_round(total: &[f32; N], next: &mut [f32; N], rows: &mut [[f32; MAX_DEGREE]; M]) {
    for (c, row) in rows.iter_mut().enumerate() {
        let (v, n) = check(c);
        let mut t = [0.0f32; MAX_DEGREE];
        for j in 0..n {
            t[j] = tanh_half_first_form(total[usize::from(v[j])] - row[j]);
        }
        let mut prefix = [1.0f32; MAX_DEGREE + 1];
        for j in 0..n {
            prefix[j + 1] = prefix[j] * t[j];
        }
        let mut suffix = 1.0f32;
        for j in (0..n).rev() {
            row[j] = twice_atanh(prefix[j] * suffix);
            suffix *= t[j];
        }
        for j in 0..n {
            next[usize::from(v[j])] += row[j];
        }
    }
}

#[test]
fn the_sum_product_round_matches_the_check_by_check_form_bit_for_bit() {
    let mut value = random_values(0x5eed);
    let bits =
        |x: &mut dyn Iterator<Item = &f32>| x.map(|v| v.to_bits()).collect::<std::vec::Vec<_>>();
    for state in 0..20_000 {
        let mut total = [0.0f32; N];
        let mut next = [0.0f32; N];
        let mut msg = [0.0f32; EDGES];
        total.iter_mut().for_each(|x| *x = value());
        next.iter_mut().for_each(|x| *x = value());
        msg.iter_mut().for_each(|x| *x = value());
        let given = rows_of::<GROUP>(&msg);
        let mut rows = given;
        let mut want_next = next;
        reference_round(&total, &mut want_next, &mut rows);
        // Both targets' forms: the decoder's, the other one, and each
        // with the other layout. Each takes the same messages in its own
        // layout.
        let forms: [(&str, Round, usize); 5] = [
            (
                "decoder",
                sum_product_round::<VECTOR, PASS_LANES, GROUP>,
                GROUP,
            ),
            ("vector", sum_product_round::<true, 24, 4>, 4),
            ("scalar", sum_product_round::<false, 1, 1>, 1),
            ("vector, groups of one", sum_product_round::<true, 24, 1>, 1),
            (
                "scalar, groups of four",
                sum_product_round::<false, 1, 4>,
                4,
            ),
        ];
        for (form, round, group) in forms {
            let mut msg = [f32::NAN; EDGES];
            let mut got_next = [f32::NAN; N];
            let got_rows = if group == 1 {
                rows_into::<1>(&given, &mut msg);
                round(&next, &total, &mut got_next, &mut msg);
                rows_of::<1>(&msg)
            } else {
                rows_into::<4>(&given, &mut msg);
                round(&next, &total, &mut got_next, &mut msg);
                rows_of::<4>(&msg)
            };
            assert_eq!(
                bits(&mut got_next.iter()),
                bits(&mut want_next.iter()),
                "state {state}, {form}: sums"
            );
            assert_eq!(
                bits(&mut got_rows.iter().flatten()),
                bits(&mut rows.iter().flatten()),
                "state {state}, {form}: messages"
            );
        }
    }
}

/// A round: the soft bits, the totals, the next totals, the messages.
type Round = fn(&[f32; N], &[f32; N], &mut [f32; N], &mut [f32; EDGES]);

/// The min-sum round as first written (inside `decode_soft`, the inputs
/// in a separate row), over rows by check: the reference the in-place
/// round must match bit for bit in either layout.
fn reference_min_sum_round(
    total: &[f32; N],
    next: &mut [f32; N],
    rows: &mut [[f32; MAX_DEGREE]; M],
) {
    for (c, row) in rows.iter_mut().enumerate() {
        let (v, n) = check(c);
        let mut q = [0.0f32; LANES];
        for j in 0..n {
            q[j] = total[usize::from(v[j])] - row[j];
        }
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
        for j in 0..n {
            next[usize::from(v[j])] += row[j];
        }
    }
}

#[test]
fn the_min_sum_round_matches_its_first_form_bit_for_bit() {
    let mut value = random_values(0x31d_5eed);
    for state in 0..20_000 {
        let mut total = [0.0f32; N];
        let mut next = [0.0f32; N];
        let mut msg = [0.0f32; EDGES];
        total.iter_mut().for_each(|x| *x = value());
        next.iter_mut().for_each(|x| *x = value());
        msg.iter_mut().for_each(|x| *x = value());
        // Ties: some states with repeated magnitudes in every check.
        if state % 4 == 0 {
            msg.iter_mut().step_by(3).for_each(|x| *x = 0.0);
            total.iter_mut().step_by(2).for_each(|x| *x = 1.5);
        }
        // Special values: NaNs of both signs, infinities, −0 and the
        // largest finite values, in the totals, the messages and the soft
        // bits, so that the selects meet comparisons that are all false.
        if state % 4 == 1 {
            let special = [
                f32::NAN,
                -f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
                -0.0,
                f32::MAX,
                -f32::MAX,
            ];
            let mut k = state;
            for x in total.iter_mut().step_by(5).chain(msg.iter_mut().step_by(7)) {
                *x = special[k % special.len()];
                k += 3;
            }
            next.iter_mut().step_by(11).for_each(|x| {
                *x = special[k % special.len()];
                k += 1;
            });
        }
        let given = rows_of::<GROUP>(&msg);
        let (mut want_next, mut want_rows) = (next, given);
        reference_min_sum_round(&total, &mut want_next, &mut want_rows);
        let forms: [(&str, Round, usize); 5] = [
            ("decoder", min_sum_round::<VECTOR, GROUP>, GROUP),
            ("vector", min_sum_round::<true, 4>, 4),
            ("scalar", min_sum_round::<false, 1>, 1),
            ("vector, groups of one", min_sum_round::<true, 1>, 1),
            ("scalar, groups of four", min_sum_round::<false, 4>, 4),
        ];
        for (form, round, group) in forms {
            let mut msg = [f32::NAN; EDGES];
            let mut got_next = [f32::NAN; N];
            let got_rows = if group == 1 {
                rows_into::<1>(&given, &mut msg);
                round(&next, &total, &mut got_next, &mut msg);
                rows_of::<1>(&msg)
            } else {
                rows_into::<4>(&given, &mut msg);
                round(&next, &total, &mut got_next, &mut msg);
                rows_of::<4>(&msg)
            };
            assert_eq!(
                got_next.map(f32::to_bits),
                want_next.map(f32::to_bits),
                "state {state}, {form}: sums"
            );
            assert_eq!(
                got_rows.map(|r| r.map(f32::to_bits)),
                want_rows.map(|r| r.map(f32::to_bits)),
                "state {state}, {form}: messages"
            );
        }
    }
}

/// The inputs' form: the totals, the messages, the edges' variables.
type Inputs = fn(&[f32; N], &mut [f32; EDGES], &[u8; EDGES]);

#[test]
fn the_inputs_are_the_same_in_blocks_of_any_size() {
    let mut value = random_values(0x1a_b10c);
    for state in 0..2_000 {
        let mut total = [0.0f32; N];
        let mut msg = [0.0f32; EDGES];
        total.iter_mut().for_each(|x| *x = value());
        msg.iter_mut().for_each(|x| *x = value());
        let edge_vars = tables_for(if state % 2 == 0 { 1 } else { 4 }).0;
        let mut want = msg;
        for (m, &i) in want.iter_mut().zip(edge_vars) {
            *m = total[usize::from(i)] - *m;
        }
        let blocks: [Inputs; 5] = [
            inputs::<1>,
            inputs::<3>,
            inputs::<8>,
            inputs::<16>,
            inputs::<INPUT_BLOCK>,
        ];
        for (k, f) in blocks.iter().enumerate() {
            let mut got = msg;
            f(&total, &mut got, edge_vars);
            assert_eq!(
                got.map(f32::to_bits),
                want.map(f32::to_bits),
                "state {state}, block form {k}"
            );
        }
    }
}

#[test]
fn each_value_gives_the_same_bits_at_any_lane_count() {
    let mut seed = 0x1a9e_u64;
    let mut value = move || {
        seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        match z % 8 {
            0 => f32::from_bits(z as u32),
            1 => ((z >> 8) % 2_000_001) as f32 / 1_000_000.0 - 1.0,
            _ => ((z >> 8) % 400_001) as f32 / 10_000.0 - 20.0,
        }
    };
    fn lanes<const W: usize>(xs: &[f32; EDGES], f: fn(f32) -> f32) -> [u32; EDGES] {
        let mut v = *xs;
        each_value::<W>(&mut v, f);
        v.map(f32::to_bits)
    }
    for _ in 0..200 {
        let mut xs = [0.0f32; EDGES];
        xs.iter_mut().for_each(|x| *x = value());
        for f in [
            tanh_half::<true> as fn(f32) -> f32,
            tanh_half::<false>,
            twice_atanh,
        ] {
            let want = xs.map(|x| f(x).to_bits());
            for got in [
                lanes::<1>(&xs, f),
                lanes::<4>(&xs, f),
                lanes::<8>(&xs, f),
                lanes::<16>(&xs, f),
                lanes::<24>(&xs, f),
                lanes::<PASS_LANES>(&xs, f),
            ] {
                assert_eq!(got, want);
            }
        }
    }
}

#[test]
fn the_scratch_holds_one_message_per_edge() {
    assert_eq!(
        core::mem::size_of::<BpScratch>(),
        4 * (2 * N + EDGES) + N + 1 + 1
    );
}

/// `decode_soft` as at its first exit-per-path form: round 0's test
/// before the loop, then each round's test after it, the totals' arrays
/// swapped by contents, every bit decided before the checks, and the
/// rounds in their first forms. The messages are kept in the decoder's
/// layout. Returns the result, the rounds run, the totals and the
/// messages.
fn decode_soft_first_form(
    llr: &[f32; N],
    rule: CheckRule,
    max_rounds: u8,
) -> (Result<BpDecode, BpFailure>, u8, [f32; N], [f32; EDGES]) {
    let (mut total, mut msg, mut hard) = (*llr, [0.0f32; EDGES], [0u8; N]);
    let hard_errors = |hard: &[u8; N]| {
        let n = hard
            .iter()
            .zip(llr.iter())
            .filter(|(&h, &l)| l != 0.0 && h != u8::from(l < 0.0))
            .count();
        n.min(255) as u8
    };
    let done = |round: u8, hard: &[u8; N]| {
        accept(hard).map(|payload| BpDecode {
            payload,
            rounds: round,
            hard_errors: hard_errors(hard),
        })
    };
    if hard_decide_first_form(&total, &mut hard) {
        return (done(0, &hard), 0, total, msg);
    }
    for round in 1..=max_rounds {
        let mut next = *llr;
        let mut rows = rows_of::<GROUP>(&msg);
        if rule == CheckRule::SumProduct {
            reference_round(&total, &mut next, &mut rows);
        } else {
            reference_min_sum_round(&total, &mut next, &mut rows);
        }
        rows_into::<GROUP>(&rows, &mut msg);
        core::mem::swap(&mut total, &mut next);
        if hard_decide_first_form(&total, &mut hard) {
            return (done(round, &hard), round, total, msg);
        }
    }
    (Err(BpFailure::NotConverged), max_rounds, total, msg)
}

/// `decode_soft` against its first form over `vectors` soft-bit vectors
/// (noisy codewords of four messages, of one with a wrong CRC and of the
/// all-zero word, σ 0.3–2.9, some with bits erased to ±0, some random
/// values, one all zero), both rules, round limits 0,
/// 1, 2, 3, 7, 30 and 255 and, for each vector that converges at round
/// r > 0, the limits r and r − 1: the result, the rounds run, and the
/// totals' and messages' bits. Returns the outcomes: decodes at round 0,
/// exactly at their limit (> 0), at other rounds, then the failures not
/// converged, by CRC and all zero.
fn decode_soft_against_its_first_form(vectors: usize) -> [usize; 6] {
    let messages = [
        "CQ K1ABC FN42",
        "K1ABC W9XYZ -11",
        "W9XYZ K1ABC R-09",
        "CQ DX K1ABC FN42",
    ];
    let mut words: std::vec::Vec<_> = messages.iter().map(|m| codeword(m).1).collect();
    words.push(ldpc::encode(crc::append_crc(&codeword(messages[0]).0) ^ 1));
    words.push(ldpc::encode(0));
    let mut value = random_values(0xd1ff);
    let mut scratch = BpScratch::new();
    let mut outcomes = [0usize; 6];
    for k in 0..vectors {
        let sigma = 0.3 + 2.6 * (k % 53) as f64 / 52.0;
        let mut llr = channel(&words[k % words.len()], 2.0, sigma, 0x5107 + k as u64);
        match k % 11 {
            0 => llr.iter_mut().for_each(|x| *x = value()),
            3 | 7 => {
                for (i, x) in llr.iter_mut().enumerate() {
                    if (i * 31 + k) % 9 == 0 {
                        *x = if i % 2 == 0 { 0.0 } else { -0.0 };
                    }
                }
            }
            _ => {}
        }
        if k == 1 {
            llr = [0.0; N];
        }
        for rule in RULES {
            let mut limits = std::vec![0u8, 1, 2, 3, 7, 30, 255];
            if let Ok(d) = decode_soft(&llr, rule, 30, &mut scratch) {
                if d.rounds > 0 {
                    limits.extend([d.rounds, d.rounds - 1]);
                }
            }
            for max_rounds in limits {
                let (want, want_rounds, want_total, want_msg) =
                    decode_soft_first_form(&llr, rule, max_rounds);
                let got = decode_soft(&llr, rule, max_rounds, &mut scratch);
                let at = std::format!("vector {k}, {rule:?}, limit {max_rounds}");
                assert_eq!(got, want, "{at}");
                assert_eq!(scratch.rounds(), want_rounds, "{at}");
                let totals = if scratch.rounds % 2 == 0 {
                    &scratch.total
                } else {
                    &scratch.next
                };
                assert_eq!(
                    totals.map(f32::to_bits),
                    want_total.map(f32::to_bits),
                    "{at}"
                );
                assert_eq!(
                    scratch.msg.map(f32::to_bits),
                    want_msg.map(f32::to_bits),
                    "{at}"
                );
                outcomes[match got {
                    Ok(d) if d.rounds == 0 => 0,
                    Ok(d) if d.rounds == max_rounds => 1,
                    Ok(_) => 2,
                    Err(BpFailure::NotConverged) => 3,
                    Err(BpFailure::Crc) => 4,
                    Err(_) => 5,
                }] += 1;
            }
        }
    }
    std::println!("outcomes {outcomes:?}");
    outcomes
}

#[test]
fn decode_soft_matches_its_first_form() {
    let outcomes = decode_soft_against_its_first_form(120);
    assert!(outcomes.iter().all(|&n| n > 0), "{outcomes:?}");
    assert!(outcomes[1] >= 40, "{outcomes:?}");
}

/// The same over 3 000 vectors (about ten seconds in release).
#[test]
#[ignore]
fn decode_soft_matches_its_first_form_widely() {
    let outcomes = decode_soft_against_its_first_form(3_000);
    assert!(outcomes.iter().all(|&n| n > 0), "{outcomes:?}");
    assert!(outcomes[1] >= 1_000, "{outcomes:?}");
}
