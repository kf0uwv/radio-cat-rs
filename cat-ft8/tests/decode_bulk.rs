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

//! Bulk false-decode run: thousands of slots with no signal — white
//! noise only, and white noise with steady carriers — must give no
//! decodes, with either configuration.
//!
//! Slow in a debug build, so ignored by default; CI runs it in release in
//! its own job:
//!
//! ```text
//! cargo test --release -p cat-ft8 --test decode_bulk -- --ignored --nocapture
//! ```

mod common;

use cat_ft8::decode::DecodeConfig;
use cat_ft8::SLOT_SAMPLES;
use common::*;

/// Noise-only slots and carrier-plus-noise slots per configuration.
const NOISE_SLOTS: usize = 2_000;
const CARRIER_SLOTS: usize = 1_000;

/// Slot `i`: white noise, plus (if `carriers`) one to four steady
/// carriers 0–20 dB above the noise in 2500 Hz at random frequencies.
fn empty_slot(i: usize, carriers: bool) -> Vec<f32> {
    let seed = 1_000_000 + i as u64 * 2 + u64::from(carriers);
    let mut s = vec![0.0f32; SLOT_SAMPLES];
    if carriers {
        let mut r = Rng(seed ^ 0x00c0_ffee);
        let count = 1 + (r.uniform() * 4.0) as usize;
        for _ in 0..count {
            let f = 100.0 + 3_000.0 * r.uniform();
            let a = f64::from(amplitude(20.0 * r.uniform(), NOISE_RMS));
            let phase = 2.0 * core::f64::consts::PI * r.uniform();
            for (n, x) in s.iter_mut().enumerate() {
                *x += (a * libm::cos(2.0 * core::f64::consts::PI * f * n as f64 / 12_000.0 + phase))
                    as f32;
            }
        }
    }
    add_noise(&mut s, NOISE_RMS, seed);
    s
}

#[test]
#[ignore = "bulk run: release build, its own CI job"]
fn no_decodes_in_noise_or_carriers() {
    for (name, config) in [("HOST", DecodeConfig::HOST), ("SMALL", DecodeConfig::SMALL)] {
        for (kind, carriers, n) in [
            ("noise only", false, NOISE_SLOTS),
            ("carriers + noise", true, CARRIER_SLOTS),
        ] {
            let results = par_map(n, |i| {
                let out = decode(&empty_slot(i, carriers), config);
                (out.decodes.len(), out.report.all_zero_rejected, out.texts)
            });
            let decodes: usize = results.iter().map(|r| r.0).sum();
            let all_zero: usize = results.iter().map(|r| r.1).sum();
            println!("{name} {kind}: {n} slots, {decodes} decodes, {all_zero} all-zero codewords rejected");
            for (i, r) in results.iter().enumerate().filter(|r| r.1 .0 > 0) {
                println!("  slot {i}: {:?}", r.2);
            }
            assert_eq!(decodes, 0, "{name} {kind}");
        }
    }
}
