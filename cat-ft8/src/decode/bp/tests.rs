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
