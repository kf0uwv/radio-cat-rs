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
        hard_decide(&[tie; N], &mut hard);
        for (i, &h) in hard.iter().enumerate() {
            assert_eq!(usize::from(h), i % 2, "tie {tie:?} at {i}");
        }
    }
    // Signs decide everywhere else, whatever the index.
    let mut values = [0.0f32; N];
    for (i, v) in values.iter_mut().enumerate() {
        *v = if i % 3 == 0 { -1.0e-30 } else { 2.5 };
    }
    hard_decide(&values, &mut hard);
    for (i, &h) in hard.iter().enumerate() {
        assert_eq!(h, u8::from(i % 3 == 0), "at {i}");
    }
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
        assert!((f64::from(tanh_half(x)) - exact).abs() < 2e-6, "{x}");
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

/// Messages in the edge layout as rows by check, in check order (unused
/// lanes 0.0).
fn rows_of(msg: &[f32; EDGES]) -> [[f32; MAX_DEGREE]; M] {
    let mut rows = [[0.0f32; MAX_DEGREE]; M];
    for (row, (v, e)) in rows.iter_mut().zip(check_edges()) {
        row[..v.len()].copy_from_slice(&msg[e..][..v.len()]);
    }
    rows
}

/// FNV-1a over the bits of the scratch's totals and messages (as rows by
/// check, so the hash does not depend on the messages' layout).
fn state_hash(s: &BpScratch) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let rows = rows_of(&s.msg);
    let words = s.total.iter().chain(rows.iter().flatten());
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
fn the_edge_layout_uses_every_edge_once_in_aligned_blocks() {
    let mut used = [0u8; EDGES];
    let mut per_variable = [0u8; N];
    let (mut low, mut high) = (0, LOW_EDGES);
    for (c, (v, e)) in check_edges().enumerate() {
        let n = v.len();
        assert_eq!(v, &check(c).0[..check(c).1], "check {c}");
        // Each degree's checks are consecutive blocks of their own size,
        // in check order: degree 6 in [0, LOW_EDGES), 7 in [LOW_EDGES, EDGES).
        if n == 6 {
            assert_eq!(e, low, "check {c}");
            low += 6;
            assert!(e + 6 <= LOW_EDGES && e % 6 == 0, "check {c} at {e}");
        } else {
            assert_eq!(n, 7, "check {c}");
            assert_eq!(e, high, "check {c}");
            high += 7;
            assert!(e >= LOW_EDGES && e + 7 <= EDGES, "check {c} at {e}");
            assert_eq!((e - LOW_EDGES) % 7, 0, "check {c} at {e}");
        }
        for (u, &i) in used[e..e + n].iter_mut().zip(v) {
            *u += 1;
            per_variable[usize::from(i)] += 1;
        }
    }
    assert_eq!((low, high), (LOW_EDGES, EDGES));
    assert_eq!((LOW_EDGES, EDGES), (354, 522));
    assert!(used.iter().all(|&u| u == 1), "{used:?}");
    assert!(per_variable.iter().all(|&d| d == 3), "{per_variable:?}");
}

/// The sum-product round in its first form, check by check over rows of
/// the parity-check table: the reference the edge-layout round must match
/// bit for bit.
fn reference_round(total: &[f32; N], next: &mut [f32; N], rows: &mut [[f32; MAX_DEGREE]; M]) {
    for (c, row) in rows.iter_mut().enumerate() {
        let (v, n) = check(c);
        let mut t = [0.0f32; MAX_DEGREE];
        for j in 0..n {
            t[j] = tanh_half(total[usize::from(v[j])] - row[j]);
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
    // splitmix64 words.
    let mut seed = 0x5eed_u64;
    let mut word = move || {
        seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    // Values over a wide range, with ±0, subnormals and values up to ±1e6
    // mixed in.
    let mut value = move || {
        let w = word();
        let sign = if w & 16 == 0 { 1.0 } else { -1.0 };
        match w % 16 {
            0 => 0.0,
            1 => -0.0,
            2 => sign * f32::from_bits((w >> 8) as u32 & 0x007f_ffff),
            3 => ((w >> 8) % 2_000_001) as f32 - 1.0e6,
            _ => ((w >> 8) % 400_001) as f32 / 10_000.0 - 20.0,
        }
    };
    let bits =
        |x: &mut dyn Iterator<Item = &f32>| x.map(|v| v.to_bits()).collect::<std::vec::Vec<_>>();
    for state in 0..20_000 {
        let mut total = [0.0f32; N];
        let mut next = [0.0f32; N];
        let mut msg = [0.0f32; EDGES];
        total.iter_mut().for_each(|x| *x = value());
        next.iter_mut().for_each(|x| *x = value());
        msg.iter_mut().for_each(|x| *x = value());
        let mut rows = rows_of(&msg);
        let mut want_next = next;
        reference_round(&total, &mut want_next, &mut rows);
        sum_product_round(&total, &mut next, &mut msg);
        assert_eq!(
            bits(&mut next.iter()),
            bits(&mut want_next.iter()),
            "state {state}: sums"
        );
        assert_eq!(
            bits(&mut rows_of(&msg).iter().flatten()),
            bits(&mut rows.iter().flatten()),
            "state {state}: messages"
        );
    }
}

#[test]
fn the_scratch_holds_one_message_per_edge() {
    assert_eq!(
        core::mem::size_of::<BpScratch>(),
        4 * (2 * N + EDGES) + N + 1 + 1
    );
}
