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
//! The decoder here is the single pass, with sub-grid refinement and no
//! subtraction (the `grid` and `refine` modes compare it with soft bits
//! taken on the coarse half-symbol, half-tone grid).
//!
//! Each slot holds one signal at a random off-grid frequency (200–2800 Hz)
//! and time offset (−0.3…+0.7 s), from the encoder vector messages, with
//! white noise at the stated SNR in 2500 Hz. A slot counts as decoded if
//! the transmitted payload is among the decodes; any other decoded payload
//! is a false decode. Slots run in parallel threads; each slot's result
//! depends only on its seed.

use std::time::Instant;

use cat_ft8::decode::bp::{decode_soft, BpFailure, BpScratch};
use cat_ft8::decode::soft::{soft_bits, soft_bits_at};
use cat_ft8::decode::sync::{self, find_candidates};
use cat_ft8::decode::{
    Band, Candidate, CandidateScratch, CheckRule, Decode, DecodeConfig, DecodeList, Decoder,
    FrontEnd, Spectrogram, HOST_DECODES,
};
use cat_ft8::message::{pack, NoHashes, Payload};
use cat_ft8::waveform::{synthesize, Placement};
use cat_ft8::{encode, SLOT_SAMPLES};

const NOISE_RMS: f64 = 0.03;
const CANDIDATES: usize = 200;

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
    /// SNR, then the offset from the analysis grid in bins and rows
    /// (0 = on the grid, 0.5 = half-way between grid points).
    Placed(f64, f64, f64),
    /// A signal at this SNR against the local noise density, in noise
    /// shaped by a receiver-like passband ([`passband`]).
    Shaped(f64),
    NoiseOnly,
    Carriers,
}

/// The transmitted signal of a slot.
#[derive(Clone, Copy)]
struct Truth {
    payload: Payload,
    snr: f64,
    base_hz: f64,
    dt: f64,
}

/// Builds slot `seed`: returns samples and the transmitted signal.
fn make_slot(messages: &[String], content: Content, seed: u64) -> (Vec<f32>, Option<Truth>) {
    let mut rng = Noise(seed.wrapping_mul(0x2545_f491_4f6c_dd1d));
    let mut out = vec![0.0f32; SLOT_SAMPLES];
    let mut payload = None;
    match content {
        Content::Signal(snr) | Content::Placed(snr, ..) | Content::Shaped(snr) => {
            let msg = &messages[(rng.uniform() * messages.len() as f64) as usize % messages.len()];
            let enc = encode(msg, &mut NoHashes).unwrap();
            let (mut base, mut dt) = (200.0 + 2_600.0 * rng.uniform(), -0.3 + rng.uniform());
            if let Content::Placed(_, dbin, drow) = content {
                let bin = 64 + (rng.uniform() * 832.0) as i32;
                let row = -3 + (rng.uniform() * 12.0) as i32;
                base = (f64::from(bin) + dbin) * 3.125;
                dt = (f64::from(row) + drow) * 0.08;
            }
            let mut noise_2500 = NOISE_RMS * NOISE_RMS * 2500.0 / 6000.0;
            if let Content::Shaped(_) = content {
                noise_2500 *= passband_gain2(base + 21.875);
            }
            let mut p = Placement::slot(base as f32, dt as f32).unwrap();
            p.amplitude = (2.0 * noise_2500 * 10f64.powf(snr / 10.0)).sqrt() as f32;
            synthesize(&enc.symbols, &p, &mut out).unwrap();
            payload = Some(Truth {
                payload: enc.payload,
                snr,
                base_hz: base,
                dt,
            });
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
    if let Content::Shaped(_) = content {
        let mut filters = passband();
        for x in out.iter_mut() {
            let mut v = NOISE_RMS * n.gauss();
            for f in filters.iter_mut() {
                v = f.run(v);
            }
            *x += v as f32;
        }
    } else {
        for x in out.iter_mut() {
            *x += (NOISE_RMS * n.gauss()) as f32;
        }
    }
    (out, payload)
}

/// A second-order section (RBJ cookbook form), f64, for test noise only.
#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    fn new(high: bool, f0: f64, q: f64) -> Self {
        let w = std::f64::consts::TAU * f0 / 12_000.0;
        let (sn, cs) = w.sin_cos();
        let alpha = sn / (2.0 * q);
        let a0 = 1.0 + alpha;
        let b = if high {
            [(1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0]
        } else {
            [(1.0 - cs) / 2.0, 1.0 - cs, (1.0 - cs) / 2.0]
        };
        Biquad {
            b: [b[0] / a0, b[1] / a0, b[2] / a0],
            a: [-2.0 * cs / a0, (1.0 - alpha) / a0],
            z: [0.0; 2],
        }
    }
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
    fn gain2(&self, f: f64) -> f64 {
        let w = std::f64::consts::TAU * f / 12_000.0;
        let z1 = (w.cos(), -w.sin());
        let z2 = ((2.0 * w).cos(), -(2.0 * w).sin());
        let num = (
            self.b[0] + self.b[1] * z1.0 + self.b[2] * z2.0,
            self.b[1] * z1.1 + self.b[2] * z2.1,
        );
        let den = (
            1.0 + self.a[0] * z1.0 + self.a[1] * z2.0,
            self.a[0] * z1.1 + self.a[1] * z2.1,
        );
        (num.0 * num.0 + num.1 * num.1) / (den.0 * den.0 + den.1 * den.1)
    }
}

/// A receiver-like audio passband: 2nd-order high-pass at 400 Hz and
/// 4th-order low-pass at 1800 Hz (about −1 dB at 1 kHz, −15 dB at
/// 2 800 Hz, −13 dB at 200 Hz).
fn passband() -> [Biquad; 3] {
    [
        Biquad::new(true, 400.0, std::f64::consts::FRAC_1_SQRT_2),
        Biquad::new(false, 1_800.0, 0.541_196),
        Biquad::new(false, 1_800.0, 1.306_563),
    ]
}

/// Power gain of [`passband`] at `f` Hz.
fn passband_gain2(f: f64) -> f64 {
    passband().iter().map(|b| b.gain2(f)).product()
}

/// A crowded slot: `signals` signals at random frequencies (200–2 800 Hz),
/// time offsets (−0.3…+0.7 s) and SNRs (−16…+6 dB) in white noise. With
/// `neighbour`, instead one signal at −10 dB and a second 10 dB stronger
/// 30–60 Hz above it.
fn make_busy(
    messages: &[String],
    signals: usize,
    neighbour: bool,
    seed: u64,
) -> (Vec<f32>, Vec<Truth>) {
    let mut rng = Noise(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0x5555);
    let mut out = vec![0.0f32; SLOT_SAMPLES];
    let mut truths = Vec::new();
    let noise_2500 = NOISE_RMS * NOISE_RMS * 2500.0 / 6000.0;
    let count = if neighbour { 2 } else { signals };
    let first_base = 300.0 + 2_300.0 * rng.uniform();
    for i in 0..count {
        let msg = &messages[(rng.uniform() * messages.len() as f64) as usize % messages.len()];
        let enc = encode(msg, &mut NoHashes).unwrap();
        if truths.iter().any(|t: &Truth| t.payload == enc.payload) {
            continue;
        }
        let (base, dt, snr) = if neighbour {
            if i == 0 {
                (first_base, -0.3 + rng.uniform(), -10.0)
            } else {
                (
                    first_base + 30.0 + 30.0 * rng.uniform(),
                    -0.3 + rng.uniform(),
                    0.0,
                )
            }
        } else {
            (
                200.0 + 2_600.0 * rng.uniform(),
                -0.3 + rng.uniform(),
                -16.0 + 22.0 * rng.uniform(),
            )
        };
        let mut p = Placement::slot(base as f32, dt as f32).unwrap();
        p.amplitude = (2.0 * noise_2500 * 10f64.powf(snr / 10.0)).sqrt() as f32;
        synthesize(&enc.symbols, &p, &mut out).unwrap();
        truths.push(Truth {
            payload: enc.payload,
            snr,
            base_hz: base,
            dt,
        });
    }
    let mut n = Noise(seed);
    for x in out.iter_mut() {
        *x += (NOISE_RMS * n.gauss()) as f32;
    }
    (out, truths)
}

/// SNR errors (estimate − truth) of the decoded signals of a busy slot,
/// and the count of decodes that match no signal.
fn busy_errors(
    samples: &[f32],
    truths: &[Truth],
    cfg: DecodeConfig,
    only_first: bool,
) -> (Vec<f64>, u32) {
    let band = Band::DEFAULT;
    let mut front = Box::new(FrontEnd::new());
    let mut levels = vec![0u8; band.spectrogram_len()];
    let mut scratch = vec![0i32; Decoder::sync_scratch_len(band)];
    let mut cands = vec![Candidate::default(); cfg.candidates];
    let mut dec =
        Decoder::new(&mut front, &mut levels, &mut scratch, &mut cands, band, cfg).unwrap();
    dec.feed(samples);
    let mut store = vec![Decode::EMPTY; HOST_DECODES];
    let mut list = DecodeList::new(&mut store);
    dec.decode(&mut Box::new(CandidateScratch::new()), &mut list);
    let mut errs = Vec::new();
    let mut false_decodes = 0;
    for d in list.finish_payloads() {
        match truths.iter().position(|t| t.payload == d.payload) {
            Some(i) if !only_first || i == 0 => errs.push(f64::from(d.snr_db) - truths[i].snr),
            Some(_) => {}
            None => false_decodes += 1,
        }
    }
    (errs, false_decodes)
}

/// Counts summed over slots.
#[derive(Default, Clone, Copy)]
struct Tally {
    decoded: u32,
    false_decodes: u32,
    all_zero_rejected: u32,
    candidates: u32,
    /// Per 500 Hz band from 200 Hz (signal tone 0): count, sum and sum of
    /// squares of the SNR error (estimate − truth, dB).
    snr_err: [(u32, f64, f64); 6],
    /// Sums of squared frequency (Hz) and time (s) errors, and maxima.
    freq_err2: f64,
    dt_err2: f64,
    freq_err_max: f64,
    dt_err_max: f64,
}

impl Tally {
    fn add(&mut self, o: &Tally) {
        self.decoded += o.decoded;
        self.false_decodes += o.false_decodes;
        self.all_zero_rejected += o.all_zero_rejected;
        self.candidates += o.candidates;
        for (a, b) in self.snr_err.iter_mut().zip(o.snr_err.iter()) {
            a.0 += b.0;
            a.1 += b.1;
            a.2 += b.2;
        }
        self.freq_err2 += o.freq_err2;
        self.dt_err2 += o.dt_err2;
        self.freq_err_max = self.freq_err_max.max(o.freq_err_max);
        self.dt_err_max = self.dt_err_max.max(o.dt_err_max);
    }
}

/// Decodes one slot with the decoder API.
fn decode_slot(samples: &[f32], truth: Option<Truth>, cfg: DecodeConfig) -> Tally {
    let snr_true = truth.map_or(0.0, |t| t.snr);
    let band = Band::DEFAULT;
    let mut front = Box::new(FrontEnd::new());
    let mut levels = vec![0u8; band.spectrogram_len()];
    let mut scratch = vec![0i32; Decoder::sync_scratch_len(band)];
    let mut cands = vec![Candidate::default(); cfg.candidates];
    let mut dec =
        Decoder::new(&mut front, &mut levels, &mut scratch, &mut cands, band, cfg).unwrap();
    dec.feed(samples);
    let mut store = vec![Decode::EMPTY; HOST_DECODES];
    let mut list = DecodeList::new(&mut store);
    let report = dec.decode(&mut Box::new(CandidateScratch::new()), &mut list);
    let mut out = Tally {
        candidates: report.candidates_kept as u32,
        all_zero_rejected: report.all_zero_rejected as u32,
        ..Tally::default()
    };
    for d in list.finish_payloads() {
        if let Some(t) = truth.filter(|t| t.payload == d.payload) {
            out.decoded = 1;
            let band = (((t.base_hz - 200.0) / 500.0) as usize).min(5);
            let e = f64::from(d.snr_db) - snr_true;
            out.snr_err[band] = (1, e, e * e);
            let (fe, te) = (f64::from(d.freq_hz) - t.base_hz, f64::from(d.dt) - t.dt);
            out.freq_err2 = fe * fe;
            out.dt_err2 = te * te;
            out.freq_err_max = fe.abs();
            out.dt_err_max = te.abs();
        } else {
            out.false_decodes += 1;
        }
    }
    out
}

/// How soft bits are taken in [`decode_low`].
#[derive(Clone, Copy, PartialEq)]
enum Path {
    /// The decoder API.
    Api,
    /// Integer levels at the grid point.
    Grid,
    /// Interpolated at offsets estimated from the sync scores.
    Estimated,
    /// Interpolated at the true offsets (for candidates within half a
    /// grid step of the signal; others at the grid point).
    Oracle,
}

/// Decodes one slot with the stages directly.
fn decode_low(samples: &[f32], truth: Option<Truth>, cfg: DecodeConfig, path: Path) -> Tally {
    if path == Path::Api {
        return decode_slot(samples, truth, cfg);
    }
    let band = Band::DEFAULT;
    let mut front = Box::new(FrontEnd::new());
    let mut levels = vec![0u8; band.spectrogram_len()];
    let mut spec = Spectrogram::new(&mut front, &mut levels, band).unwrap();
    spec.feed(samples);
    let mut scratch = vec![0i32; sync::scratch_len(band.base_bins())];
    let mut cands = vec![Candidate::default(); cfg.candidates];
    let rep = find_candidates(&spec, cfg.min_score, &mut scratch, &mut cands).unwrap();
    let mut bp = Box::new(BpScratch::new());
    let mut seen: Vec<Payload> = Vec::new();
    let mut out = Tally {
        candidates: rep.kept as u32,
        ..Tally::default()
    };
    let mut llr = [0.0f32; 174];
    for c in &cands[..rep.kept] {
        match path {
            Path::Api | Path::Grid => {
                soft_bits(&spec, c, cfg.llr_scale, &mut llr);
            }
            Path::Estimated => {
                let (fr, fb) = sync::refine(&spec, c);
                soft_bits_at(&spec, c, fr, fb, cfg.llr_scale, &mut llr);
            }
            Path::Oracle => {
                let (mut fr, mut fb) = (0.0, 0.0);
                if let Some(t) = truth {
                    let row = 5.0 + t.dt / 0.08 - f64::from(c.start_row);
                    let bin = t.base_hz / 3.125 - band.first_bin() as f64 - f64::from(c.base);
                    if row.abs() <= 0.5 && bin.abs() <= 0.5 {
                        (fr, fb) = (row as f32, bin as f32);
                    }
                }
                soft_bits_at(&spec, c, fr, fb, cfg.llr_scale, &mut llr);
            }
        }
        match decode_soft(&llr, cfg.rule, cfg.max_rounds, &mut bp) {
            Ok(d) => {
                if seen.contains(&d.payload) {
                    continue;
                }
                seen.push(d.payload);
                if Some(d.payload) == truth.map(|t| t.payload) {
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
fn run(messages: &[String], content: Content, n: u64, seed0: u64, cfg: DecodeConfig) -> Tally {
    run_path(messages, content, n, seed0, cfg, Path::Api)
}

/// [`run`] with soft bits taken along `path`.
fn run_path(
    messages: &[String],
    content: Content,
    n: u64,
    seed0: u64,
    cfg: DecodeConfig,
    path: Path,
) -> Tally {
    let threads = std::thread::available_parallelism().map_or(4, |x| x.get()) as u64;
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                s.spawn(move || {
                    let mut acc = Tally::default();
                    let mut i = t;
                    while i < n {
                        let (samples, truth) = make_slot(messages, content, seed0 + i);
                        acc.add(&decode_low(&samples, truth, cfg, path));
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
    let base = DecodeConfig::HOST;
    match mode.as_str() {
        "sweep" => {
            let rule = args.next().unwrap_or_else(|| "sp".into());
            let target: f32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(4.0);
            let cfg = base
                .with_rule(if rule == "ms" {
                    CheckRule::MinSum
                } else {
                    CheckRule::SumProduct
                })
                .with_llr_scale(target);
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
                    let cfg = base.with_rule(rule).with_llr_scale(target);
                    let mut line = format!("  {name:<14} target {target:<3}:");
                    for snr in [-20.0, -19.0, -18.0] {
                        let o = run(&msgs, Content::Signal(snr), n, (snr * -1000.0) as u64, cfg);
                        line += &format!("  {snr}: {:>4} ({})", o.decoded, o.false_decodes);
                    }
                    println!("{line}");
                }
            }
        }
        "grid" => {
            println!("decoded / {n} slots by offset from the analysis grid (bins, rows)");
            for (db, dr) in [(0.0, 0.0), (0.5, 0.0), (0.0, 0.5), (0.5, 0.5), (0.25, 0.25)] {
                let mut line = format!("  offset ({db}, {dr}):");
                for snr in [-21.0, -20.0, -19.0, -18.0] {
                    let o = run(
                        &msgs,
                        Content::Placed(snr, db, dr),
                        n,
                        (snr * -1000.0) as u64,
                        base,
                    );
                    line += &format!("  {snr}: {:>4} ({})", o.decoded, o.false_decodes);
                }
                println!("{line}");
            }
        }
        "refine" => {
            println!("decoded / {n} slots (false decodes) by soft-bit path");
            let offsets = [(0.0, 0.0), (0.5, 0.0), (0.0, 0.5), (0.5, 0.5), (0.25, 0.25)];
            for path in [Path::Grid, Path::Estimated, Path::Oracle] {
                let name = match path {
                    Path::Grid => "grid",
                    Path::Estimated => "estimated",
                    _ => "oracle",
                };
                for (db, dr) in offsets {
                    let mut line = format!("  {name:<9} offset ({db}, {dr}):");
                    for snr in [-20.0, -19.0, -18.0] {
                        let c = Content::Placed(snr, db, dr);
                        let o = run_path(&msgs, c, n, (snr * -1000.0) as u64, base, path);
                        line += &format!("  {snr}: {:>4} ({})", o.decoded, o.false_decodes);
                    }
                    println!("{line}");
                }
                let mut line = format!("  {name:<9} random offsets:");
                for snr in [-20.0, -19.0, -18.0] {
                    let o = run_path(
                        &msgs,
                        Content::Signal(snr),
                        n,
                        (snr * -1000.0) as u64,
                        base,
                        path,
                    );
                    line += &format!("  {snr}: {:>4} ({})", o.decoded, o.false_decodes);
                }
                println!("{line}");
            }
        }
        "snr" => {
            for f in [
                100.0, 200.0, 500.0, 1_000.0, 1_500.0, 2_000.0, 2_500.0, 2_800.0, 3_100.0,
            ] {
                print!(" {f}: {:+.1} dB", 10.0 * passband_gain2(f).log10());
            }
            println!(" (shaped-noise passband)");
            println!("SNR estimate error (dB, estimate − truth) by signal frequency; {n} slots per point");
            let names = [
                "200-700",
                "700-1200",
                "1200-1700",
                "1700-2200",
                "2200-2700",
                "2700-2800",
            ];
            for (noise, shaped) in [("white", false), ("shaped", true)] {
                println!("  {noise} noise: mean (RMS) error per band, then decoded; freq / dt RMS and max error");
                for snr in [-20.0, -18.0, -15.0, -10.0, -5.0, 0.0, 10.0, 20.0] {
                    let content = if shaped {
                        Content::Shaped(snr)
                    } else {
                        Content::Signal(snr)
                    };
                    let o = run(&msgs, content, n, (snr * -1000.0 + 77.0) as u64, base);
                    let mut line = format!("    SNR {snr:>5}:");
                    for (name, e) in names.iter().zip(o.snr_err.iter()) {
                        if e.0 > 0 {
                            let mean = e.1 / f64::from(e.0);
                            let rms = (e.2 / f64::from(e.0)).sqrt();
                            line += &format!(" {name} {mean:+5.1} ({rms:3.1})");
                        } else {
                            line += &format!(" {name}   -  ");
                        }
                    }
                    let k = f64::from(o.decoded.max(1));
                    line += &format!(
                        "  dec {:>4}  f {:.2}/{:.2} Hz  dt {:.3}/{:.3} s",
                        o.decoded,
                        (o.freq_err2 / k).sqrt(),
                        o.freq_err_max,
                        (o.dt_err2 / k).sqrt(),
                        o.dt_err_max
                    );
                    println!("{line}");
                }
            }
        }
        "busy" => {
            println!("SNR error in busy slots (estimate − truth, dB); {n} slots per row");
            for (name, signals, neighbour) in [
                ("one neighbour +10 dB, 30–60 Hz above", 2, true),
                ("10 signals", 10, false),
                ("20 signals", 20, false),
                ("40 signals", 40, false),
            ] {
                let threads = std::thread::available_parallelism().map_or(4, |x| x.get()) as u64;
                let msgs = &msgs;
                let (errs, falses) = std::thread::scope(|s| {
                    let hs: Vec<_> = (0..threads)
                        .map(|t| {
                            s.spawn(move || {
                                let (mut e, mut f) = (Vec::new(), 0u32);
                                let mut i = t;
                                while i < n {
                                    let (slot, truths) =
                                        make_busy(msgs, signals, neighbour, 4_000 + i);
                                    let (ee, ff) = busy_errors(&slot, &truths, base, neighbour);
                                    e.extend(ee);
                                    f += ff;
                                    i += threads;
                                }
                                (e, f)
                            })
                        })
                        .collect();
                    let mut e = Vec::new();
                    let mut f = 0;
                    for h in hs {
                        let (ee, ff) = h.join().unwrap();
                        e.extend(ee);
                        f += ff;
                    }
                    (e, f)
                });
                let k = errs.len().max(1) as f64;
                let mean = errs.iter().sum::<f64>() / k;
                let rms = (errs.iter().map(|e| e * e).sum::<f64>() / k).sqrt();
                let min = errs.iter().copied().fold(f64::MAX, f64::min);
                let max = errs.iter().copied().fold(f64::MIN, f64::max);
                let low3 = errs.iter().filter(|&&e| e < -3.0).count();
                println!(
                    "  {name:<38}: {} decodes, mean {mean:+.2}, RMS {rms:.2}, min {min:+.1}, max {max:+.1}, below −3 dB {low3}; false {falses}",
                    errs.len()
                );
            }
        }
        "stage" => {
            // One stage only, repeated, for instruction counts with
            // `perf stat -e instructions:u` (args: stage name, repeats).
            let which = args.next().unwrap_or_else(|| "feed".into());
            let (samples, _) = make_slot(&msgs, Content::Signal(-20.0), 7);
            let band = Band::DEFAULT;
            let mut front = Box::new(FrontEnd::new());
            let mut levels = vec![0u8; band.spectrogram_len()];
            let mut spec = Spectrogram::new(&mut front, &mut levels, band).unwrap();
            spec.feed(&samples);
            let mut scratch = vec![0i32; sync::scratch_len(band.base_bins())];
            let mut cands = vec![Candidate::default(); CANDIDATES];
            let kept = find_candidates(&spec, 0, &mut scratch, &mut cands)
                .unwrap()
                .kept;
            let mut bp = Box::new(BpScratch::new());
            let mut llr = [0.0f32; 174];
            let mut sink = 0usize;
            let t = Instant::now();
            for _ in 0..n {
                match which.as_str() {
                    "feed" => {
                        spec.start_slot();
                        sink += spec.feed(std::hint::black_box(&samples));
                    }
                    "sync" => {
                        sink += find_candidates(&spec, 0, &mut scratch, &mut cands)
                            .unwrap()
                            .kept;
                    }
                    "bp" => {
                        for c in &cands[..kept] {
                            let (fr, fb) = sync::refine(&spec, c);
                            soft_bits_at(&spec, c, fr, fb, 4.0, &mut llr);
                            sink += usize::from(
                                decode_soft(&llr, CheckRule::SumProduct, 30, &mut bp).is_ok(),
                            );
                        }
                    }
                    other => panic!("unknown stage {other}"),
                }
            }
            println!(
                "stage {which}: {n} repeats, {:.3} ms each ({sink})",
                t.elapsed().as_secs_f64() * 1e3 / n as f64
            );
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
            let mut llr = [0.0f32; 174];
            let t = Instant::now();
            for _ in 0..reps {
                for c in &cands[..kept] {
                    soft_bits(&spec, c, 4.0, &mut llr);
                }
            }
            let t_grid = t.elapsed().as_secs_f64() / f64::from(reps) / kept as f64;
            let t = Instant::now();
            for _ in 0..reps {
                for c in &cands[..kept] {
                    let (fr, fb) = sync::refine(&spec, c);
                    soft_bits_at(&spec, c, fr, fb, 4.0, &mut llr);
                }
            }
            let t_refined = t.elapsed().as_secs_f64() / f64::from(reps) / kept as f64;
            println!(
                "  soft bits per candidate: grid {:.1} µs, refined (offsets + interpolation) {:.1} µs",
                t_grid * 1e6,
                t_refined * 1e6
            );
            let mut bp = Box::new(BpScratch::new());
            for (name, rule) in [
                ("sum-product", CheckRule::SumProduct),
                ("min-sum", CheckRule::MinSum),
            ] {
                let t = Instant::now();
                // Rounds are known for converged candidates only: a failure
                // reports why, not after how many rounds (a candidate that
                // never converges runs all 30).
                let (mut rounds, mut converged, mut failed) = (0u64, 0u64, 0u64);
                for _ in 0..reps {
                    for c in &cands[..kept] {
                        soft_bits(&spec, c, 4.0, &mut llr);
                        match decode_soft(&llr, rule, 30, &mut bp) {
                            Ok(d) => {
                                rounds += u64::from(d.rounds);
                                converged += 1;
                            }
                            Err(_) => failed += 1,
                        }
                    }
                }
                let per = t.elapsed().as_secs_f64() / f64::from(reps) / kept as f64;
                println!(
                    "  {name}: {:.1} µs per candidate (soft bits + BP), {:.1} ms per slot of {kept} candidates; \
                     {converged} converged (mean {} rounds), {failed} failed",
                    per * 1e6,
                    per * kept as f64 * 1e3,
                    if converged == 0 {
                        "n/a".to_string()
                    } else {
                        format!("{:.1}", rounds as f64 / converged as f64)
                    }
                );
            }
            println!(
                "  feed (whole slot): {:.2} ms; sync search: {:.2} ms",
                t_feed * 1e3,
                t_sync * 1e3
            );
            // The whole decoder, one thread: feed, then decode().
            for (name, cfg) in [("HOST", DecodeConfig::HOST), ("SMALL", DecodeConfig::SMALL)] {
                let mut front = Box::new(FrontEnd::new());
                let mut levels = vec![0u8; band.spectrogram_len()];
                let mut scratch = vec![0i32; Decoder::sync_scratch_len(band)];
                let mut cands = vec![Candidate::default(); cfg.candidates];
                let mut dec =
                    Decoder::new(&mut front, &mut levels, &mut scratch, &mut cands, band, cfg)
                        .unwrap();
                let mut store = vec![Decode::EMPTY; HOST_DECODES];
                let mut list = DecodeList::new(&mut store);
                let mut cs = Box::new(CandidateScratch::new());
                let t = Instant::now();
                for _ in 0..reps {
                    dec.start_slot();
                    dec.feed(&samples);
                    dec.decode(&mut cs, &mut list);
                }
                let per = t.elapsed().as_secs_f64() / f64::from(reps);
                println!(
                    "  decoder {name} (feed + decode, one thread): {:.1} ms per slot",
                    per * 1e3
                );
            }
        }
        other => eprintln!("unknown mode {other}"),
    }
}
