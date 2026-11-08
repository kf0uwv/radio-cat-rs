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

//! Bulk false-decode run: thousands of slots with no signal must give no
//! decodes, with either configuration. The inputs: white noise; noise with
//! steady carriers, weak and strong; full-scale noise clipped at ±1;
//! noise so low that most levels floor; and noise with impulses.
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

/// What a slot holds (none of it a signal).
#[derive(Debug, Clone, Copy)]
enum Kind {
    /// White noise.
    Noise,
    /// One to four steady carriers 0–20 dB above the noise in 2500 Hz.
    Carriers,
    /// One to four carriers 20–50 dB above the noise.
    StrongCarriers,
    /// Noise of RMS 0.5 and a carrier of amplitude 0.8, clipped at ±1.
    Clipped,
    /// Noise of RMS 10⁻⁵ (−100 dBFS): most levels at the floor.
    Floored,
    /// White noise with random impulses of amplitude 0.3–1.0 (about one
    /// in 2 000 samples) and two 20 ms bursts.
    Impulsive,
}

/// (kind, slots per configuration).
const RUNS: [(Kind, usize); 6] = [
    (Kind::Noise, 2_000),
    (Kind::Carriers, 1_000),
    (Kind::StrongCarriers, 250),
    (Kind::Clipped, 250),
    (Kind::Floored, 250),
    (Kind::Impulsive, 250),
];

fn add_carriers(s: &mut [f32], r: &mut Rng, lo_db: f64, hi_db: f64) {
    let count = 1 + (r.uniform() * 4.0) as usize;
    for _ in 0..count {
        let f = 100.0 + 3_000.0 * r.uniform();
        let a = f64::from(amplitude(lo_db + (hi_db - lo_db) * r.uniform(), NOISE_RMS));
        let phase = 2.0 * core::f64::consts::PI * r.uniform();
        for (n, x) in s.iter_mut().enumerate() {
            *x += (a * libm::cos(2.0 * core::f64::consts::PI * f * n as f64 / 12_000.0 + phase))
                as f32;
        }
    }
}

/// Slot `i` of `kind`.
fn empty_slot(i: usize, kind: Kind) -> Vec<f32> {
    let seed = 1_000_000 + i as u64 * 8 + kind as u64;
    let mut r = Rng(seed ^ 0x00c0_ffee);
    let mut s = vec![0.0f32; SLOT_SAMPLES];
    match kind {
        Kind::Noise => add_noise(&mut s, NOISE_RMS, seed),
        Kind::Carriers => {
            add_carriers(&mut s, &mut r, 0.0, 20.0);
            add_noise(&mut s, NOISE_RMS, seed);
        }
        Kind::StrongCarriers => {
            add_carriers(&mut s, &mut r, 20.0, 50.0);
            add_noise(&mut s, NOISE_RMS, seed);
        }
        Kind::Clipped => {
            let f = 200.0 + 2_800.0 * r.uniform();
            for (n, x) in s.iter_mut().enumerate() {
                *x =
                    (0.8 * libm::cos(2.0 * core::f64::consts::PI * f * n as f64 / 12_000.0)) as f32;
            }
            add_noise(&mut s, 0.5, seed);
            for x in s.iter_mut() {
                *x = x.clamp(-1.0, 1.0);
            }
        }
        Kind::Floored => add_noise(&mut s, 1e-5, seed),
        Kind::Impulsive => {
            add_noise(&mut s, NOISE_RMS, seed);
            for x in s.iter_mut() {
                if r.uniform() < 5e-4 {
                    let a = 0.3 + 0.7 * r.uniform();
                    *x += if r.uniform() < 0.5 {
                        a as f32
                    } else {
                        -a as f32
                    };
                }
            }
            for _ in 0..2 {
                let start = (r.uniform() * (SLOT_SAMPLES - 240) as f64) as usize;
                for x in &mut s[start..start + 240] {
                    *x += (0.5 * (2.0 * r.uniform() - 1.0)) as f32;
                }
            }
        }
    }
    s
}

#[test]
#[ignore = "bulk run: release build, its own CI job"]
fn no_decodes_without_a_signal() {
    let mut failures = Vec::new();
    for (name, config) in [("HOST", DecodeConfig::HOST), ("SMALL", DecodeConfig::SMALL)] {
        for (kind, n) in RUNS {
            let results = par_map(n, |i| {
                let out = decode(&empty_slot(i, kind), config);
                (out.decodes.len(), out.report.all_zero_rejected, out.texts)
            });
            let decodes: usize = results.iter().map(|r| r.0).sum();
            let all_zero: usize = results.iter().map(|r| r.1).sum();
            println!("{name} {kind:?}: {n} slots, {decodes} decodes, {all_zero} all-zero codewords rejected");
            for (i, r) in results.iter().enumerate().filter(|r| r.1 .0 > 0) {
                println!("  slot {i}: {:?}", r.2);
            }
            if decodes > 0 {
                failures.push(format!("{name} {kind:?}: {decodes}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "decodes without a signal: {failures:?}"
    );
}
