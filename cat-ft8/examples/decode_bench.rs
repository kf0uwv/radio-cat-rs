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

//! Single-pass decoding measurements on synthetic slots made with this
//! crate's encoder and seeded white noise (no external tools):
//!
//! ```text
//! cargo run --release -p cat-ft8 --features profile --example decode_bench -- MODE [SLOTS] [sp|ms] [SCALE]
//! ```
//!
//! MODE: `sweep` (decode rate against SNR, plus noise-only and
//! carrier-plus-noise false decodes, for one check-node rule and LLR
//! scale), `compare` (the two check-node rules and LLR scales near the
//! 50% point) or `cost` (host time per stage). SLOTS is per point.
//!
//! The decoder here is the first pass only: candidates on the coarse
//! (half-symbol, half-tone) grid, without refinement or subtraction.
//!
//! Each slot holds one signal at a random off-grid frequency (200–2800 Hz)
//! and time offset (−0.3…+0.7 s), from the encoder vector messages, with
//! white noise at the stated SNR in 2500 Hz. A slot counts as decoded if
//! the transmitted payload is among the decodes; any other decoded payload
//! is a false decode. Slots run in parallel threads; each slot's result
//! depends only on its seed.

use std::time::Instant;

use cat_ft8::decode::{
    decode_soft, find_candidates, soft_bits, sync, Band, BpFailure, BpScratch, Candidate,
    CheckRule, FrontEnd, Spectrogram,
};
use cat_ft8::message::{pack, NoHashes, Payload};
use cat_ft8::waveform::{synthesize, Placement};
use cat_ft8::{encode, SLOT_SAMPLES};

const NOISE_RMS: f64 = 0.03;
const CANDIDATES: usize = 200;

#[derive(Clone, Copy, Debug)]
struct Config {
    rule: CheckRule,
    target: f32,
    rounds: u8,
    min_score: i32,
}

struct Noise(u64);

impl Noise {
    fn uniform(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64 + 1e-300
    }
    fn gauss(&mut self) -> f64 {
        let (u, v) = (self.uniform(), self.uniform());
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

/// What lies in a synthetic slot.
#[derive(Clone, Copy)]
enum Content {
    Signal(f64),
    NoiseOnly,
    Carriers,
}

/// Builds slot `seed`: returns samples and the transmitted payload.
fn make_slot(messages: &[String], content: Content, seed: u64) -> (Vec<f32>, Option<Payload>) {
    let mut rng = Noise(seed.wrapping_mul(0x2545_f491_4f6c_dd1d));
    let mut out = vec![0.0f32; SLOT_SAMPLES];
    let mut payload = None;
    match content {
        Content::Signal(snr) => {
            let msg = &messages[(rng.uniform() * messages.len() as f64) as usize % messages.len()];
            let enc = encode(msg, &mut NoHashes).unwrap();
            payload = Some(enc.payload);
            let base = 200.0 + 2_600.0 * rng.uniform();
            let dt = -0.3 + rng.uniform();
            let noise_2500 = NOISE_RMS * NOISE_RMS * 2500.0 / 6000.0;
            let mut p = Placement::slot(base as f32, dt as f32).unwrap();
            p.amplitude = (2.0 * noise_2500 * 10f64.powf(snr / 10.0)).sqrt() as f32;
            synthesize(&enc.symbols, &p, &mut out).unwrap();
        }
        Content::NoiseOnly => {}
        Content::Carriers => {
            // Three steady carriers 10 dB above the noise in 2500 Hz, at
            // random frequencies.
            for _ in 0..3 {
                let f = 200.0 + 2_700.0 * rng.uniform();
                let noise_2500 = NOISE_RMS * NOISE_RMS * 2500.0 / 6000.0;
                let a = (2.0 * noise_2500 * 10.0).sqrt();
                let ph = rng.uniform() * std::f64::consts::TAU;
                for (n, x) in out.iter_mut().enumerate() {
                    *x += (a * (2.0 * std::f64::consts::PI * f * n as f64 / 12_000.0 + ph).cos())
                        as f32;
                }
            }
        }
    }
    let mut n = Noise(seed);
    for x in out.iter_mut() {
        *x += (NOISE_RMS * n.gauss()) as f32;
    }
    (out, payload)
}

/// Counts summed over slots.
#[derive(Default, Clone, Copy)]
struct Tally {
    decoded: u32,
    false_decodes: u32,
    all_zero_rejected: u32,
    candidates: u32,
}

impl Tally {
    fn add(&mut self, o: &Tally) {
        self.decoded += o.decoded;
        self.false_decodes += o.false_decodes;
        self.all_zero_rejected += o.all_zero_rejected;
        self.candidates += o.candidates;
    }
}

/// Decodes one slot.
fn decode_slot(samples: &[f32], truth: Option<Payload>, cfg: Config) -> Tally {
    let band = Band::DEFAULT;
    let mut front = Box::new(FrontEnd::new());
    let mut levels = vec![0u8; band.spectrogram_len()];
    let mut spec = Spectrogram::new(&mut front, &mut levels, band).unwrap();
    spec.feed(samples);
    let mut scratch = vec![0i32; sync::scratch_len(band.base_bins())];
    let mut cands = vec![Candidate::default(); CANDIDATES];
    let rep = find_candidates(&spec, cfg.min_score, &mut scratch, &mut cands).unwrap();
    let mut bp = Box::new(BpScratch::new());
    let mut seen: Vec<Payload> = Vec::new();
    let mut out = Tally {
        candidates: rep.kept as u32,
        ..Tally::default()
    };
    let mut llr = [0.0f32; 174];
    for c in &cands[..rep.kept] {
        soft_bits(&spec, c, cfg.target, &mut llr);
        match decode_soft(&llr, cfg.rule, cfg.rounds, &mut bp) {
            Ok(d) => {
                if seen.contains(&d.payload) {
                    continue;
                }
                seen.push(d.payload);
                if Some(d.payload) == truth {
                    out.decoded = 1;
                } else {
                    out.false_decodes += 1;
                }
            }
            Err(BpFailure::AllZero) => out.all_zero_rejected += 1,
            Err(_) => {}
        }
    }
    out
}

/// Runs `n` slots of `content` in parallel threads; sums the tallies.
fn run(messages: &[String], content: Content, n: u64, seed0: u64, cfg: Config) -> Tally {
    let threads = std::thread::available_parallelism().map_or(4, |x| x.get()) as u64;
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                s.spawn(move || {
                    let mut acc = Tally::default();
                    let mut i = t;
                    while i < n {
                        let (samples, truth) = make_slot(messages, content, seed0 + i);
                        acc.add(&decode_slot(&samples, truth, cfg));
                        i += threads;
                    }
                    acc
                })
            })
            .collect();
        let mut total = Tally::default();
        for h in handles {
            total.add(&h.join().unwrap());
        }
        total
    })
}

fn messages() -> Vec<String> {
    include_str!("../tests/data/encoder-vectors.jsonl")
        .lines()
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["message"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .filter(|m| pack(m, &mut NoHashes).is_ok())
        .collect()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "sweep".into());
    let n: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(100);
    let msgs = messages();
    let base = Config {
        rule: CheckRule::SumProduct,
        target: 4.0,
        rounds: 30,
        min_score: 0,
    };
    match mode.as_str() {
        "sweep" => {
            let rule = args.next().unwrap_or_else(|| "sp".into());
            let target: f32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(4.0);
            let cfg = Config {
                rule: if rule == "ms" {
                    CheckRule::MinSum
                } else {
                    CheckRule::SumProduct
                },
                target,
                ..base
            };
            println!("config {cfg:?}, {n} slots per point, 8-bit levels, band 100–3100 Hz");
            for snr in [
                -24.0, -22.0, -21.0, -20.0, -19.5, -19.0, -18.5, -18.0, -17.0, -16.0, -14.0,
            ] {
                let o = run(&msgs, Content::Signal(snr), n, (snr * -1000.0) as u64, cfg);
                println!(
                    "  SNR {snr:>5}: decoded {:>4}/{n}  false {}  all-zero rejected {}",
                    o.decoded, o.false_decodes, o.all_zero_rejected
                );
            }
            for (name, content) in [
                ("noise only", Content::NoiseOnly),
                ("carriers + noise", Content::Carriers),
            ] {
                let o = run(&msgs, content, n * 10, 900_000, cfg);
                println!(
                    "  {name}: {} slots, false decodes {}, all-zero rejected {}, mean candidates {:.0}",
                    n * 10,
                    o.false_decodes,
                    o.all_zero_rejected,
                    f64::from(o.candidates) / (n * 10) as f64
                );
            }
        }
        "compare" => {
            println!("decoded / {n} slots per SNR (false decodes)");
            let rules = [
                ("sum-product", CheckRule::SumProduct),
                ("min-sum", CheckRule::MinSum),
            ];
            for (name, rule) in rules {
                for target in [1.5f32, 2.0, 3.0, 4.0, 6.0] {
                    if rule == CheckRule::MinSum && target != 4.0 {
                        continue; // min-sum is scale-free apart from rounding
                    }
                    let cfg = Config {
                        rule,
                        target,
                        ..base
                    };
                    let mut line = format!("  {name:<14} target {target:<3}:");
                    for snr in [-20.0, -19.0, -18.0] {
                        let o = run(&msgs, Content::Signal(snr), n, (snr * -1000.0) as u64, cfg);
                        line += &format!("  {snr}: {:>4} ({})", o.decoded, o.false_decodes);
                    }
                    println!("{line}");
                }
            }
        }
        "cost" => {
            let (samples, _) = make_slot(&msgs, Content::Signal(-20.0), 7);
            let band = Band::DEFAULT;
            let reps = n.max(5) as u32;
            let mut front = Box::new(FrontEnd::new());
            let mut levels = vec![0u8; band.spectrogram_len()];
            let mut spec = Spectrogram::new(&mut front, &mut levels, band).unwrap();
            let t = Instant::now();
            for _ in 0..reps {
                spec.start_slot();
                spec.feed(&samples);
            }
            let t_feed = t.elapsed().as_secs_f64() / f64::from(reps);
            let mut scratch = vec![0i32; sync::scratch_len(band.base_bins())];
            let mut cands = vec![Candidate::default(); CANDIDATES];
            let t = Instant::now();
            let mut kept = 0;
            for _ in 0..reps {
                kept = find_candidates(&spec, 0, &mut scratch, &mut cands)
                    .unwrap()
                    .kept;
            }
            let t_sync = t.elapsed().as_secs_f64() / f64::from(reps);
            let mut bp = Box::new(BpScratch::new());
            let mut llr = [0.0f32; 174];
            for (name, rule) in [
                ("sum-product", CheckRule::SumProduct),
                ("min-sum", CheckRule::MinSum),
            ] {
                let t = Instant::now();
                let mut rounds = 0u64;
                for _ in 0..reps {
                    for c in &cands[..kept] {
                        soft_bits(&spec, c, 4.0, &mut llr);
                        match decode_soft(&llr, rule, 30, &mut bp) {
                            Ok(d) => rounds += u64::from(d.rounds),
                            Err(_) => rounds += 30,
                        }
                    }
                }
                let per = t.elapsed().as_secs_f64() / f64::from(reps) / kept as f64;
                println!(
                    "  {name}: {:.1} µs per candidate (soft bits + BP; mean {:.1} rounds), {:.1} ms per slot of {kept} candidates",
                    per * 1e6,
                    rounds as f64 / (f64::from(reps) * kept as f64),
                    per * kept as f64 * 1e3
                );
            }
            println!(
                "  feed (whole slot): {:.2} ms; sync search: {:.2} ms",
                t_feed * 1e3,
                t_sync * 1e3
            );
        }
        other => eprintln!("unknown mode {other}"),
    }
}
