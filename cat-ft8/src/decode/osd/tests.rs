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

use std::boxed::Box;
use std::vec::Vec;

use super::*;
use crate::crc;
use crate::decode::testutil::Noise;
use crate::ldpc::{encode, Codeword};
use crate::message::Payload;

fn bits_of(cw: &Codeword) -> Bits {
    let mut b = [0; WORDS];
    for i in 0..N {
        b[i / WB] |= Word::from(cw.bit(i)) << (i % WB);
    }
    b
}

fn get(b: &Bits, i: usize) -> bool {
    (b[i / WB] >> (i % WB)) & 1 == 1
}

fn is_codeword(b: &Bits) -> bool {
    let mut bits = [0u8; N];
    for (i, x) in bits.iter_mut().enumerate() {
        *x = u8::from(get(b, i));
    }
    Codeword::from_bits(&bits).satisfies_parity_checks()
}

/// Noisy soft bits of a random codeword (some exactly ±0, some tied).
fn noisy(r: &mut Noise, sigma: f64) -> (Bits, [f32; N]) {
    let payload = Payload::from_bits(((r.uniform() * 2f64.powi(52)) as u128) << 25).unwrap();
    let truth = bits_of(&encode(crc::append_crc(&payload)));
    let mut v = [0.0f32; N];
    for (i, x) in v.iter_mut().enumerate() {
        let s = if get(&truth, i) { -1.0 } else { 1.0 };
        *x = (s + sigma * r.gauss()) as f32;
        match (r.uniform() * 40.0) as u32 {
            0 => *x = 0.0,
            1 => *x = -0.0,
            2 => *x = x.signum() * 0.5,
            _ => {}
        }
    }
    (truth, v)
}

#[test]
fn rows_are_the_unit_messages_codewords() {
    for (k, row) in OSD_ROWS.iter().enumerate() {
        assert_eq!(*row, bits_of(&encode(1u128 << (K - 1 - k))), "row {k}");
    }
}

#[test]
fn the_order_is_by_magnitude_then_index() {
    let mut r = Noise(7);
    let mut s = Box::new(OsdScratch::new());
    for t in 0..200 {
        let (_, mut v) = noisy(&mut r, 0.3 + t as f64 * 0.01);
        if t % 10 == 0 {
            // Few distinct magnitudes: long runs of ties.
            for x in v.iter_mut() {
                *x = (*x * 2.0).round() / 2.0;
            }
        }
        reliability_order(&v, &mut s.order, &mut s.spare, &mut s.count);
        let mut want: Vec<u8> = (0..N as u8).collect();
        want.sort_by(|&a, &b| {
            v[usize::from(b)]
                .abs()
                .total_cmp(&v[usize::from(a)].abs())
                .then(a.cmp(&b))
        });
        assert_eq!(&s.order[..], &want[..], "trial {t}");
    }
}

#[test]
fn elimination_gives_the_systematic_form_on_the_basis() {
    let mut r = Noise(11);
    let mut s = Box::new(OsdScratch::new());
    for t in 0..100 {
        let (_, v) = noisy(&mut r, 0.4 + t as f64 * 0.01);
        let OsdScratch {
            rows,
            order,
            spare,
            count,
            pivot,
            ..
        } = &mut *s;
        reliability_order(&v, order, spare, count);
        *rows = OSD_ROWS;
        eliminate::<TARGET_BRANCH>(rows, pivot, order);
        // Each row is a codeword with a 1 at its own pivot and 0 at the
        // others, and the pivots are the first independent positions.
        for (j, row) in rows.iter().enumerate() {
            assert!(is_codeword(row), "trial {t} row {j}");
            for (k, &p) in pivot.iter().enumerate() {
                assert_eq!(
                    get(row, usize::from(p)),
                    j == k,
                    "trial {t} row {j} pivot {k}"
                );
            }
        }
        let rank_of = |p: u8| order.iter().position(|&o| o == p).unwrap();
        assert!(pivot.windows(2).all(|w| rank_of(w[0]) < rank_of(w[1])));
    }
}

#[test]
fn order1_finds_the_least_discrepancy_flip() {
    let mut r = Noise(13);
    let mut s = Box::new(OsdScratch::new());
    let mut truth_found = 0;
    for t in 0..300 {
        let (truth, v) = noisy(&mut r, 0.3 + t as f64 * 0.004);
        let best = order1(&v, &mut s);
        let mut hard = [0; WORDS];
        for (i, &x) in v.iter().enumerate() {
            hard[i / WB] |= Word::from(x < 0.0) << (i % WB);
        }
        // Rebuild the chosen codeword from the message part.
        let cw = bits_of(&encode(best.a91));
        assert!(is_codeword(&cw));
        let diff = xor(&cw, &hard);
        assert_eq!(
            best.distance,
            diff.iter().map(|w| w.count_ones()).sum::<u32>()
        );
        let direct: f32 = (0..N).filter(|&i| get(&diff, i)).map(|i| v[i].abs()).sum();
        assert!(
            (direct - best.d).abs() <= 1e-4 * direct.max(1.0),
            "trial {t}"
        );
        // Basis errors at most 1: the truth or something closer.
        let errors = s
            .pivot
            .iter()
            .filter(|&&p| get(&truth, usize::from(p)) != get(&hard, usize::from(p)))
            .count();
        if errors <= 1 {
            if cw == truth {
                truth_found += 1;
            } else {
                let dt: f32 = (0..N)
                    .filter(|&i| get(&truth, i) != get(&hard, i))
                    .map(|i| v[i].abs())
                    .sum();
                assert!(best.d <= dt + 1e-4 * dt.max(1.0), "trial {t}");
            }
        }
    }
    assert!(truth_found > 100, "{truth_found}");
}

#[test]
fn zero_input_gives_the_zero_codeword() {
    let mut s = Box::new(OsdScratch::new());
    for z in [0.0f32, -0.0] {
        let best = order1(&[z; N], &mut s);
        assert_eq!(
            (best.pick, best.d, best.a91, best.distance),
            (None, 0.0, 0, 0)
        );
    }
}
