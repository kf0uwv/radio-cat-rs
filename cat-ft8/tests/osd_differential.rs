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

//! The decoder's OSD-1 against the measurement-grade OSD of the sizing
//! bench (`examples/paired_bench/osd.rs`, written independently): on
//! every input, the same pick, the same D bit for bit, the same codeword
//! and the same Hamming distance, in either form of the elimination.
//! Inputs: noisy codewords over a range of noise, pure noise, values on a
//! coarse grid (many tied magnitudes and tied D), exact ±0, and constant
//! inputs.

#![cfg(feature = "profile")]

#[allow(dead_code)]
#[path = "../examples/paired_bench/osd.rs"]
mod bench_osd;

use bench_osd::{codeword_bits, distance, hard_of, message_part, Generator, Osd};
use cat_ft8::crc;
use cat_ft8::decode::osd::{order1, order1_other_form, OsdScratch};
use cat_ft8::ldpc::{K, N};
use cat_ft8::message::Payload;

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn uniform(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn gauss(&mut self) -> f64 {
        // Irwin–Hall: enough for test inputs.
        (0..12).map(|_| self.uniform()).sum::<f64>() - 6.0
    }
}

#[derive(Default)]
struct Coverage {
    inputs: usize,
    order0: usize,
    /// Inputs whose least D is shared by another list codeword.
    d_ties: usize,
    /// Inputs with at least one exact ±0 value.
    zeros: usize,
    /// Inputs with tied magnitudes inside the most reliable basis.
    basis_ties: usize,
}

fn check(v: &[f32; N], gen: &Generator, bench: &mut Osd, lib: &mut OsdScratch, cov: &mut Coverage) {
    bench.prepare(v, gen);
    let want = bench.search_fast(1, false).a;
    let mut ds = Vec::with_capacity(K + 1);
    bench.search(1, false, |d, _, _| ds.push(d.to_bits()));
    let got = order1(v, lib);
    // The elimination's other form (the other targets' choice) agrees.
    let other = order1_other_form(v, lib);
    assert_eq!((other.pick, other.d.to_bits()), (got.pick, got.d.to_bits()));
    let want_pick = (usize::from(want.j) < K).then_some(want.j);
    assert_eq!(got.pick, want_pick, "pick, input {v:?}");
    assert_eq!(got.d.to_bits(), want.d.to_bits(), "D, input {v:?}");
    let cw = bench.codeword(&want);
    assert_eq!(got.a91, message_part(&cw), "codeword, input {v:?}");
    assert_eq!(
        got.distance,
        distance(&cw, &hard_of(v)),
        "distance, input {v:?}"
    );
    cov.inputs += 1;
    cov.order0 += usize::from(got.pick.is_none());
    cov.d_ties += usize::from(ds.iter().filter(|&&d| d == want.d.to_bits()).count() > 1);
    cov.zeros += usize::from(v.contains(&0.0));
    let mut mags: Vec<u32> = (0..N)
        .filter(|&i| bench_osd::get(&bench.pivot_mask(), i))
        .map(|i| v[i].abs().to_bits())
        .collect();
    let n = mags.len();
    mags.sort_unstable();
    mags.dedup();
    cov.basis_ties += usize::from(mags.len() < n);
}

#[test]
fn library_osd1_equals_the_bench_osd() {
    let gen = Generator::new();
    let mut bench = Box::new(Osd::new());
    let mut lib = Box::new(OsdScratch::new());
    let mut cov = Coverage::default();
    let mut r = Rng(0x05d1_d1ff_0000_0001);
    let trials = if cfg!(debug_assertions) {
        1_500
    } else {
        15_000
    };
    for t in 0..trials {
        let a91 =
            crc::append_crc(&Payload::from_bits(u128::from(r.next()) << 13 ^ t as u128).unwrap());
        let truth = codeword_bits(a91);
        let mut v = [0.0f32; N];
        let sigma = 0.2 + 1.6 * r.uniform();
        let scale = [1.0, 4.0, 1e-3, 37.5][t % 4];
        let kind = t % 6;
        for (i, x) in v.iter_mut().enumerate() {
            let s = if bench_osd::get(&truth, i) { -1.0 } else { 1.0 };
            let y = match kind {
                // Pure noise.
                4 => sigma * r.gauss(),
                _ => s + sigma * r.gauss(),
            };
            *x = (scale * y) as f32;
        }
        match kind {
            // A coarse grid: magnitudes and D tie often, ±0 appears.
            1 => v.iter_mut().for_each(|x| *x = (*x * 2.0).round() / 2.0),
            2 => v.iter_mut().for_each(|x| *x = x.round()),
            // Exact zeros of both signs sprinkled in.
            3 => {
                for _ in 0..20 {
                    let i = (r.next() % N as u64) as usize;
                    v[i] = if r.next() & 1 == 0 { 0.0 } else { -0.0 };
                }
            }
            _ => {}
        }
        check(&v, &gen, &mut bench, &mut lib, &mut cov);
    }
    // Constant inputs: every magnitude tied.
    for c in [0.0f32, -0.0, 1.0, -1.0, 1e-30, -3.5] {
        check(&[c; N], &gen, &mut bench, &mut lib, &mut cov);
    }
    // Alternating signs of one magnitude, and ±0 alternating.
    let alt: [f32; N] = core::array::from_fn(|i| if i % 3 == 0 { -1.0 } else { 1.0 });
    check(&alt, &gen, &mut bench, &mut lib, &mut cov);
    let zeros: [f32; N] = core::array::from_fn(|i| if i % 2 == 0 { 0.0 } else { -0.0 });
    check(&zeros, &gen, &mut bench, &mut lib, &mut cov);
    eprintln!(
        "{} inputs: order-0 picks {}, D ties at the minimum {}, with ±0 {}, tied magnitudes in the basis {}",
        cov.inputs, cov.order0, cov.d_ties, cov.zeros, cov.basis_ties
    );
    // The inputs exercise what they are meant to.
    let n = cov.inputs;
    assert!(cov.d_ties * 20 > n && cov.zeros * 4 > n && cov.basis_ties * 4 > n && cov.order0 > 10);
}
