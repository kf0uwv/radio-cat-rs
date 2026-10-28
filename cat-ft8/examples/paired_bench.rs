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

//! Paired measurement of decoder changes that alter results:
//!
//! ```text
//! cargo run --release -p cat-ft8 --features profile --example paired_bench -- run CONFIG OUTDIR [SETS] [BLOCK]
//! cargo run --release -p cat-ft8 --features profile --example paired_bench -- compare OLDDIR NEWDIR [--partial]
//! cargo run --release -p cat-ft8 --features profile --example paired_bench -- cost CONFIG_A CONFIG_B [SLOTS] [RUNS]
//! ```
//!
//! **Sets** (encoder-made; each slot's seed comes from its set, block and
//! index, and the noise from `libm`, so runs decode bit-identical input on
//! every platform):
//!
//! - `sweep`: one signal per slot at random offsets (200–2 800 Hz,
//!   −0.3…+0.7 s), −22…−15 dB in 0.5 dB steps, 1 000 slots per point;
//! - `crowded`: 10, 20 and 40 distinct messages per slot (−16…+6 dB, same
//!   offsets, overlaps allowed), 200 slots per size;
//! - `edges`: one signal at −18 dB (near the decode threshold, so a loss
//!   shows), 100 slots per case: starts at ±1.6/±2.0/±2.4 s, tone 0 near
//!   each end of both bands, input cut at 13.5 s, input starting 1 s late;
//! - `empty`: no signal — noise; noise with weak (0–20 dB) or strong
//!   (20–50 dB) carriers; clipped full-scale noise; floored low-level
//!   noise; impulsive noise.
//!
//! SETS is a comma list (default `sweep,crowded,edges,empty`); BLOCK is
//! `main` (default) or `fresh` (other seeds, for confirming a chosen
//! value); any other name is a further block of seeds (for larger runs of
//! the `empty` set). Signal SNRs are in 2500 Hz against white noise of RMS
//! 0.03.
//!
//! **CONFIG** is `host` or `small` (the small setup: 200–2900 Hz, min-sum,
//! 64 decodes) followed by optional `,key=value` changes: `rule=sp|ms`,
//! `rounds=N`, `scale=X`, `candidates=N`, `min_score=N`. Example:
//! `host,candidates=100`.
//!
//! **run** decodes every slot (all threads; results do not depend on the
//! thread count) and writes `OUTDIR/<set>-<block>.jsonl`: one JSON object
//! per slot with its set, block, index, seed, point and a hash of its input
//! samples, the transmitted messages, the decodes, the report counts, the
//! belief-propagation rounds run and the stage times. `OUTDIR/config.txt`
//! records the configuration, the commit, rustc and the CPU.
//!
//! **compare** pairs two runs slot by slot and message by message. Every
//! (set, block) must be in both runs (one present on one side only is
//! MISSING and fails), something must be compared, each pair of slots must
//! have the same seed, sample hash and transmitted messages, and — unless
//! `--partial` — all four sets of the main block must be there. It exits
//! with status 1 on any failure.
//!
//! **Statistics.** For each transmitted message the pair (old decoded?, new
//! decoded?) is one observation; b counts old-only and c new-only pairs.
//! Under "no difference" each discordant pair is old-only with probability
//! ½, so b − c has mean 0 and standard deviation √(b + c) (McNemar's test,
//! normal approximation). The no-loss gate is one-sided — only the new
//! decoder losing fails it: b − c ≤ 3·√(b + c) at every point and ≤
//! 2·√(b + c) over a set. The false-decode gate is one-sided too: zero on
//! `empty`, elsewhere the new decoder's false decodes ≤ the old's at every
//! point. For `sweep`, the 50% point is the SNR where the decoded fraction
//! first reaches ½ (linear interpolation between points); its 95% interval
//! comes from a paired bootstrap — the same resampled slots for both runs,
//! 1 000 times — and the shift gate fails if the point estimate is more
//! than 0.05 dB worse or if a 50% point is missing. The interval is
//! reported for judgement; real losses are caught by the sign tests.
//!
//! **cost** times two configurations interleaved, one thread, on SLOTS
//! crowded slots (default 42) taken evenly from the 10-, 20- and 40-message
//! sizes, for RUNS rounds (default and minimum 5), and prints each stage's
//! median.
//!
//! **Misses by cause.** `run` also saves, per transmitted message, its
//! outcome: decoded (its payload is in the output) or, judged by position,
//! why not — no candidate on it, a candidate on it that did not converge,
//! one that reached a codeword failing the CRC, or one that reached the
//! all-zero codeword (a candidate is on a message when its refined tone 0
//! is within 4 Hz and its refined start within 0.12 s; candidates that
//! decoded some payload are left out; the furthest any candidate got
//! decides). `compare` prints, for each side, the misses by cause, the CRC
//! failures per slot and the expected false decodes they imply (a codeword
//! that was not transmitted passes the 14-bit CRC with probability
//! 2⁻¹⁴, so the estimate is the CRC failures per slot × 2⁻¹⁴).

#[path = "../tests/common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use cat_ft8::decode::{
    sync, Band, BpFailure, Candidate, CandidateScratch, CheckRule, Decode, DecodeConfig,
    DecodeList, Decoder, FrontEnd, Stage, BIN_HZ, CANDIDATES, HOST_DECODES, SMALL_DECODES,
};
use cat_ft8::message::{pack, NoHashes, Payload};
use cat_ft8::waveform::{synthesize, Placement};
use cat_ft8::{encode_payload, SLOT_SAMPLES};
use common::{add_noise, amplitude, fnv1a, par_map, Rng, NOISE_RMS};
use serde_json::Value;

// ---------------------------------------------------------------- seeds

/// splitmix64 finaliser.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The seed of slot `index` of `set` in `block`.
fn slot_seed(set: &str, block: &str, index: usize) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in set.bytes().chain([0]).chain(block.bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    mix(h ^ (index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

/// A uniform index below `n`.
fn below(r: &mut Rng, n: usize) -> usize {
    (r.uniform() * n as f64) as usize % n
}

// ------------------------------------------------------------- messages

/// A random standard callsign (prefix, digit, suffix).
fn callsign(r: &mut Rng) -> String {
    const L: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut s = String::new();
    s.push(L[below(r, 26)] as char);
    if below(r, 2) == 0 {
        s.push(L[below(r, 26)] as char);
    }
    s.push((b'0' + below(r, 10) as u8) as char);
    for _ in 0..1 + below(r, 3) {
        s.push(L[below(r, 26)] as char);
    }
    s
}

/// A random grid square.
fn grid(r: &mut Rng) -> String {
    const F: &[u8] = b"ABCDEFGHIJKLMNOPQR";
    let (a, b) = (F[below(r, 18)] as char, F[below(r, 18)] as char);
    format!("{a}{b}{}{}", below(r, 10), below(r, 10))
}

/// A random packable message whose payload is not in `avoid`.
fn message(r: &mut Rng, avoid: &[Payload]) -> (String, Payload) {
    loop {
        let (a, b) = (callsign(r), callsign(r));
        let text = match below(r, 6) {
            0 => format!("CQ {a} {}", grid(r)),
            1 => format!("{a} {b} {}", grid(r)),
            2 => format!("{a} {b} {:+03}", below(r, 51) as i32 - 30),
            3 => format!("{a} {b} R{:+03}", below(r, 51) as i32 - 30),
            4 => format!("{a} {b} RR73"),
            _ => format!("{a} {b} 73"),
        };
        if let Ok(p) = pack(&text, &mut NoHashes) {
            if !avoid.contains(&p) {
                return (text, p);
            }
        }
    }
}

// ---------------------------------------------------------------- slots

/// One transmitted signal.
#[derive(Clone, PartialEq)]
struct Sent {
    text: String,
    payload: u128,
    freq: f64,
    dt: f64,
    snr: f64,
}

/// A slot: its point label, the samples to feed after `skip` missing ones,
/// and what was sent.
struct Slot {
    point: String,
    samples: Vec<f32>,
    skip: usize,
    sent: Vec<Sent>,
}

impl Slot {
    /// FNV-1a over the skip count and every sample's bits.
    fn sample_hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        fnv1a(&mut h, &(self.skip as u64).to_le_bytes());
        for x in &self.samples {
            fnv1a(&mut h, &x.to_bits().to_le_bytes());
        }
        h
    }
}

fn add_signal(out: &mut [f32], s: &Sent) {
    let mut p = Placement::slot(s.freq as f32, s.dt as f32).unwrap();
    p.amplitude = amplitude(s.snr, NOISE_RMS);
    let payload = Payload::from_bits(s.payload).unwrap();
    synthesize(&encode_payload(&payload), &p, out).unwrap();
}

fn add_carriers(out: &mut [f32], r: &mut Rng, lo_db: f64, hi_db: f64) {
    for _ in 0..1 + below(r, 4) {
        let f = 100.0 + 3_000.0 * r.uniform();
        let a = f64::from(amplitude(lo_db + (hi_db - lo_db) * r.uniform(), NOISE_RMS));
        let ph = 2.0 * core::f64::consts::PI * r.uniform();
        for (n, x) in out.iter_mut().enumerate() {
            *x +=
                (a * libm::cos(2.0 * core::f64::consts::PI * f * n as f64 / 12_000.0 + ph)) as f32;
        }
    }
}

/// Slots in each set.
fn set_size(set: &str) -> Option<usize> {
    match set {
        "sweep" => Some(15 * 1_000),
        "crowded" => Some(3 * 200),
        "edges" => Some(EDGE_CASES.len() * 100),
        "empty" => Some(2_300),
        _ => None,
    }
}

/// Edge cases: (label, dt, tone 0 in Hz or random, samples kept, samples
/// missing at the start).
const EDGE_CASES: [(&str, f64, Option<f64>, usize, usize); 13] = [
    ("dt-2.4", -2.4, None, SLOT_SAMPLES, 0),
    ("dt-2.0", -2.0, None, SLOT_SAMPLES, 0),
    ("dt-1.6", -1.6, None, SLOT_SAMPLES, 0),
    ("dt+1.6", 1.6, None, SLOT_SAMPLES, 0),
    ("dt+2.0", 2.0, None, SLOT_SAMPLES, 0),
    ("dt+2.4", 2.4, None, SLOT_SAMPLES, 0),
    ("f100", 0.2, Some(100.0), SLOT_SAMPLES, 0),
    ("f3090", 0.2, Some(3_090.0), SLOT_SAMPLES, 0),
    ("f200", 0.2, Some(200.0), SLOT_SAMPLES, 0),
    ("f2890", 0.2, Some(2_890.0), SLOT_SAMPLES, 0),
    ("cut13.5", 0.0, None, 162_000, 0),
    ("late1s", 0.0, None, SLOT_SAMPLES, 12_000),
    ("dt+0", 0.0, None, SLOT_SAMPLES, 0),
];

/// The noise seed of the slot whose seed is `seed`.
fn noise_seed(seed: u64) -> u64 {
    mix(seed ^ 0x006e_6f69_7365)
}

/// A full slot of `sent` (in order) plus white noise of RMS [`NOISE_RMS`]
/// from `noise_seed`: the input of every set with signals.
fn render(sent: &[Sent], noise_seed: u64) -> Vec<f32> {
    let mut samples = vec![0.0f32; SLOT_SAMPLES];
    for s in sent {
        add_signal(&mut samples, s);
    }
    add_noise(&mut samples, NOISE_RMS, noise_seed);
    samples
}

/// Builds slot `index` of `set` in `block`.
fn build(set: &str, block: &str, index: usize) -> Slot {
    let seed = slot_seed(set, block, index);
    let mut r = Rng(seed);
    let noise_seed = noise_seed(seed);
    let mut samples = vec![0.0f32; SLOT_SAMPLES];
    let mut sent: Vec<Sent> = Vec::new();
    let one = |r: &mut Rng, snr: f64, dt: Option<f64>, f: Option<f64>| {
        let (text, payload) = message(r, &[]);
        let freq = f.unwrap_or_else(|| 200.0 + 2_600.0 * r.uniform());
        let dt = dt.unwrap_or_else(|| -0.3 + r.uniform());
        Sent {
            text,
            payload: payload.bits(),
            freq,
            dt,
            snr,
        }
    };
    let (point, skip, cut) = match set {
        "sweep" => {
            let snr = -22.0 + 0.5 * (index / 1_000) as f64;
            sent.push(one(&mut r, snr, None, None));
            (format!("{snr:.1}"), 0, SLOT_SAMPLES)
        }
        "crowded" => {
            let n = [10, 20, 40][index / 200];
            for _ in 0..n {
                let avoid: Vec<Payload> = sent
                    .iter()
                    .map(|s| Payload::from_bits(s.payload).unwrap())
                    .collect();
                let (text, payload) = message(&mut r, &avoid);
                sent.push(Sent {
                    text,
                    payload: payload.bits(),
                    freq: 200.0 + 2_600.0 * r.uniform(),
                    dt: -0.3 + r.uniform(),
                    snr: -16.0 + 22.0 * r.uniform(),
                });
            }
            (format!("{n}"), 0, SLOT_SAMPLES)
        }
        "edges" => {
            let (label, dt, f, cut, skip) = EDGE_CASES[index / 100];
            sent.push(one(&mut r, -18.0, Some(dt), f));
            (label.to_string(), skip, cut)
        }
        "empty" => {
            let kind = match index {
                0..=999 => "noise",
                1_000..=1_499 => "carriers",
                1_500..=1_699 => "strong",
                1_700..=1_899 => "clipped",
                1_900..=2_099 => "floored",
                _ => "impulsive",
            };
            match kind {
                "carriers" => add_carriers(&mut samples, &mut r, 0.0, 20.0),
                "strong" => add_carriers(&mut samples, &mut r, 20.0, 50.0),
                "clipped" => {
                    let f = 200.0 + 2_800.0 * r.uniform();
                    for (n, x) in samples.iter_mut().enumerate() {
                        *x = (0.8
                            * libm::cos(2.0 * core::f64::consts::PI * f * n as f64 / 12_000.0))
                            as f32;
                    }
                }
                _ => {}
            }
            let rms = match kind {
                "clipped" => 0.5,
                "floored" => 1e-5,
                _ => NOISE_RMS,
            };
            add_noise(&mut samples, rms, noise_seed);
            if kind == "clipped" {
                for x in samples.iter_mut() {
                    *x = x.clamp(-1.0, 1.0);
                }
            }
            if kind == "impulsive" {
                for x in samples.iter_mut() {
                    if r.uniform() < 5e-4 {
                        let a = (0.3 + 0.7 * r.uniform()) as f32;
                        *x += if r.uniform() < 0.5 { a } else { -a };
                    }
                }
            }
            return Slot {
                point: kind.to_string(),
                samples,
                skip: 0,
                sent,
            };
        }
        _ => unreachable!(),
    };
    let mut samples = render(&sent, noise_seed);
    samples.truncate(cut);
    let samples = samples.split_off(skip.min(samples.len()));
    Slot {
        point,
        samples,
        skip,
        sent,
    }
}

// --------------------------------------------------------------- config

/// A decoder setup.
#[derive(Clone, Copy)]
struct Setup {
    band: Band,
    config: DecodeConfig,
    decodes: usize,
}

fn parse_config(spec: &str) -> Result<Setup, String> {
    let mut parts = spec.split(',');
    let mut s = match parts.next() {
        Some("host") => Setup {
            band: Band::DEFAULT,
            config: DecodeConfig::HOST,
            decodes: HOST_DECODES,
        },
        Some("small") => Setup {
            band: Band::SMALL,
            config: DecodeConfig::SMALL,
            decodes: SMALL_DECODES,
        },
        other => {
            return Err(format!(
                "config must start with host or small, not {other:?}"
            ))
        }
    };
    for kv in parts {
        let (k, v) = kv
            .split_once('=')
            .ok_or_else(|| format!("bad setting {kv}"))?;
        let num = |v: &str| v.parse::<f64>().map_err(|e| format!("{k}: {e}"));
        s.config = match k {
            "rule" => s.config.with_rule(match v {
                "sp" => CheckRule::SumProduct,
                "ms" => CheckRule::MinSum,
                _ => return Err("rule must be sp or ms".into()),
            }),
            "rounds" => s.config.with_max_rounds(num(v)? as u8),
            "scale" => s.config.with_llr_scale(num(v)? as f32),
            "candidates" => s.config.with_candidates(num(v)? as usize),
            "min_score" => s.config.with_min_score(num(v)? as i32),
            _ => return Err(format!("unknown setting {k}")),
        };
    }
    Ok(s)
}

// --------------------------------------------------------------- record

/// One decode as saved.
#[derive(Clone, PartialEq)]
struct DecodeRecord {
    payload: u128,
    freq: f32,
    dt: f32,
    snr: f32,
    score: i32,
    candidate: u16,
    hard_errors: u8,
    rounds: u8,
}

/// The report counts saved per slot.
#[derive(Clone, Copy, Default)]
struct Counts {
    found: u64,
    kept: u64,
    dropped: u64,
    decoded: u64,
    truncated: u64,
    duplicates: u64,
    all_zero: u64,
    crc_failed: u64,
    not_converged: u64,
    rows_filled: u64,
}

/// One slot's result: the record `run` writes and `compare` reads.
struct SlotResult {
    set: String,
    block: String,
    index: usize,
    seed: u64,
    point: String,
    sample_hash: u64,
    sent: Vec<Sent>,
    decodes: Vec<DecodeRecord>,
    counts: Counts,
    bp_rounds: u64,
    /// (stage, seconds): feed, sync, then every `Stage`.
    seconds: Vec<(String, f64)>,
    /// Per transmitted message (empty in runs recorded before outcomes
    /// were saved).
    outcomes: Vec<Outcome>,
}

/// `s` as a JSON string.
fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl SlotResult {
    /// One line of JSON (floats in Rust's shortest round-trip form).
    fn to_json(&self) -> String {
        let mut o = String::new();
        let _ = write!(
            o,
            "{{\"set\":{},\"block\":{},\"index\":{},\"seed\":\"{:016x}\",\"point\":{},\"sample_hash\":\"{:016x}\",\"sent\":[",
            json_str(&self.set), json_str(&self.block), self.index, self.seed, json_str(&self.point), self.sample_hash
        );
        for (i, s) in self.sent.iter().enumerate() {
            let _ =
                write!(
                o,
                "{}{{\"payload\":\"{:020x}\",\"text\":{},\"freq\":{:?},\"dt\":{:?},\"snr\":{:?}}}",
                if i > 0 { "," } else { "" }, s.payload, json_str(&s.text), s.freq, s.dt, s.snr
            );
        }
        o.push_str("],\"decodes\":[");
        for (i, d) in self.decodes.iter().enumerate() {
            let _ = write!(
                o,
                "{}{{\"payload\":\"{:020x}\",\"freq\":{:?},\"dt\":{:?},\"snr\":{:?},\"score\":{},\"candidate\":{},\"hard_errors\":{},\"rounds\":{}}}",
                if i > 0 { "," } else { "" }, d.payload, d.freq, d.dt, d.snr, d.score, d.candidate, d.hard_errors, d.rounds
            );
        }
        let c = &self.counts;
        let _ = write!(
            o,
            "],\"report\":{{\"candidates_found\":{},\"candidates_kept\":{},\"candidates_dropped\":{},\"decoded\":{},\"decodes_truncated\":{},\"duplicates\":{},\"all_zero_rejected\":{},\"crc_failed\":{},\"not_converged\":{},\"rows_filled\":{}}},\"bp_rounds\":{},\"seconds\":{{",
            c.found, c.kept, c.dropped, c.decoded, c.truncated, c.duplicates, c.all_zero, c.crc_failed, c.not_converged, c.rows_filled, self.bp_rounds
        );
        for (i, (k, v)) in self.seconds.iter().enumerate() {
            let _ = write!(o, "{}{}:{:?}", if i > 0 { "," } else { "" }, json_str(k), v);
        }
        o.push_str("},\"outcomes\":[");
        for (i, x) in self.outcomes.iter().enumerate() {
            let _ = write!(o, "{}\"{}\"", if i > 0 { "," } else { "" }, x.code());
        }
        o.push_str("]}");
        o
    }

    /// Reads a line written by [`SlotResult::to_json`].
    fn from_json(v: &Value) -> Option<SlotResult> {
        let hex = |x: &Value| u128::from_str_radix(x.as_str()?, 16).ok();
        let u = |x: &Value| x.as_u64();
        let r = &v["report"];
        Some(SlotResult {
            set: v["set"].as_str()?.into(),
            block: v["block"].as_str()?.into(),
            index: u(&v["index"])? as usize,
            seed: hex(&v["seed"])? as u64,
            point: v["point"].as_str()?.into(),
            sample_hash: hex(&v["sample_hash"])? as u64,
            sent: v["sent"]
                .as_array()?
                .iter()
                .map(|s| {
                    Some(Sent {
                        text: s["text"].as_str()?.into(),
                        payload: hex(&s["payload"])?,
                        freq: s["freq"].as_f64()?,
                        dt: s["dt"].as_f64()?,
                        snr: s["snr"].as_f64()?,
                    })
                })
                .collect::<Option<_>>()?,
            decodes: v["decodes"]
                .as_array()?
                .iter()
                .map(|d| {
                    Some(DecodeRecord {
                        payload: hex(&d["payload"])?,
                        freq: d["freq"].as_f64()? as f32,
                        dt: d["dt"].as_f64()? as f32,
                        snr: d["snr"].as_f64()? as f32,
                        score: d["score"].as_i64()? as i32,
                        candidate: u(&d["candidate"])? as u16,
                        hard_errors: u(&d["hard_errors"])? as u8,
                        rounds: u(&d["rounds"])? as u8,
                    })
                })
                .collect::<Option<_>>()?,
            counts: Counts {
                found: u(&r["candidates_found"])?,
                kept: u(&r["candidates_kept"])?,
                dropped: u(&r["candidates_dropped"])?,
                decoded: u(&r["decoded"])?,
                truncated: u(&r["decodes_truncated"])?,
                duplicates: u(&r["duplicates"])?,
                all_zero: u(&r["all_zero_rejected"])?,
                crc_failed: u(&r["crc_failed"])?,
                not_converged: u(&r["not_converged"])?,
                rows_filled: u(&r["rows_filled"])?,
            },
            bp_rounds: u(&v["bp_rounds"])?,
            seconds: v["seconds"]
                .as_object()?
                .iter()
                .map(|(k, x)| Some((k.clone(), x.as_f64()?)))
                .collect::<Option<_>>()?,
            outcomes: match v.get("outcomes") {
                None => Vec::new(),
                Some(x) => x
                    .as_array()?
                    .iter()
                    .map(|o| Outcome::from_code(o.as_str()?))
                    .collect::<Option<_>>()?,
            },
        })
    }
}

// --------------------------------------------------------------- decode

/// Positional tolerance for "a candidate on a transmitted message": the
/// candidate's refined tone 0 within this many Hz of the message's (a
/// little over one bin) and its refined start within this many seconds (a
/// little over one row).
const ON_HZ: f32 = 4.0;
const ON_S: f32 = 0.12;

/// What became of one transmitted message: decoded (its payload is in the
/// output), or why not, judged by position. The candidates "on" the
/// message are those within [`ON_HZ`] and [`ON_S`] of it, leaving out any
/// that decoded a payload (that candidate belongs to another signal, or is
/// a false decode). Of those, the furthest any got decides: a codeword
/// failing the CRC, then the all-zero codeword, then no codeword within the
/// round limit; with none, there was no candidate on it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Outcome {
    NoCandidate,
    NotConverged,
    AllZero,
    Crc,
    Decoded,
}

impl Outcome {
    /// The reasons a message is missed, in report order.
    const MISSES: [Outcome; 4] = [
        Outcome::NoCandidate,
        Outcome::NotConverged,
        Outcome::Crc,
        Outcome::AllZero,
    ];

    fn code(self) -> &'static str {
        match self {
            Outcome::Decoded => "decoded",
            Outcome::NoCandidate => "no_candidate",
            Outcome::NotConverged => "not_converged",
            Outcome::Crc => "crc",
            Outcome::AllZero => "all_zero",
        }
    }

    fn from_code(s: &str) -> Option<Outcome> {
        [
            Outcome::Decoded,
            Outcome::NoCandidate,
            Outcome::NotConverged,
            Outcome::Crc,
            Outcome::AllZero,
        ]
        .into_iter()
        .find(|o| o.code() == s)
    }

    /// The outcome a failed candidate gives a message it is on.
    fn of_failure(f: BpFailure) -> Outcome {
        match f {
            BpFailure::Crc => Outcome::Crc,
            BpFailure::AllZero => Outcome::AllZero,
            _ => Outcome::NotConverged,
        }
    }
}

/// Counts of missed messages by [`Outcome::MISSES`].
fn miss_counts<'a>(outcomes: impl IntoIterator<Item = &'a Outcome>) -> [u64; 4] {
    let mut n = [0u64; 4];
    for o in outcomes {
        if let Some(k) = Outcome::MISSES.iter().position(|m| m == o) {
            n[k] += 1;
        }
    }
    n
}

/// One single-pass decode of some input.
struct Decoded {
    decodes: Vec<DecodeRecord>,
    counts: Counts,
    bp_rounds: u64,
    /// (stage, seconds): feed, sync, then every `Stage`.
    seconds: Vec<(String, f64)>,
    /// Per transmitted message, in order.
    outcomes: Vec<Outcome>,
}

/// Decodes `samples` after `skip` missing ones with `setup`, and classifies
/// each of the `sent` messages. Only the decoder's own calls are timed.
fn decode_input(samples: &[f32], skip: usize, sent: &[Sent], setup: &Setup) -> Decoded {
    let band = setup.band;
    let mut front = Box::new(FrontEnd::new());
    let mut levels = vec![0u8; band.spectrogram_len()];
    let mut sync_buf = vec![0i32; Decoder::sync_scratch_len(band)];
    let mut cands = vec![Candidate::default(); CANDIDATES.max(setup.config.candidates)];
    let mut dec = Decoder::new(
        &mut front,
        &mut levels,
        &mut sync_buf,
        &mut cands,
        band,
        setup.config,
    )
    .unwrap();
    let mut store = vec![Decode::EMPTY; setup.decodes];
    let mut list = DecodeList::new(&mut store);
    let mut scratch = Box::new(CandidateScratch::new());
    let t = Instant::now();
    dec.skip(skip);
    dec.feed(samples);
    let feed = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let kept = dec.find_candidates().kept;
    let sync_s = t.elapsed().as_secs_f64();
    let mut stages = [0.0f64; Stage::ALL.len()];
    let mut bp_rounds = 0u64;
    let mut failed: Vec<(usize, BpFailure)> = Vec::new();
    for i in 0..kept {
        let mut last = Instant::now();
        let r = dec.decode_candidate_staged(i, &mut scratch, |st| {
            let now = Instant::now();
            stages[st.index()] += (now - last).as_secs_f64();
            last = now;
        });
        bp_rounds += u64::from(scratch.last_rounds());
        if let Some(r) = r {
            if let Err(f) = r {
                failed.push((i, f));
            }
            list.push(i, &r).unwrap();
        }
    }
    let rep = dec.report(&list);
    let mut seconds = vec![("feed".to_string(), feed), ("sync".to_string(), sync_s)];
    for st in Stage::ALL {
        seconds.push((st.name().to_string(), stages[st.index()]));
    }
    let decodes: Vec<DecodeRecord> = list
        .finish_payloads()
        .iter()
        .map(|d| DecodeRecord {
            payload: d.payload.bits(),
            freq: d.freq_hz,
            dt: d.dt,
            snr: d.snr_db,
            score: d.score,
            candidate: d.candidate,
            hard_errors: d.hard_errors,
            rounds: d.rounds,
        })
        .collect();
    // Outside the timed calls: each failed candidate's refined position,
    // as `decode_candidate` computes it.
    let spec = dec.spectrogram();
    let mut outcomes: Vec<Outcome> = sent
        .iter()
        .map(|s| {
            if decodes.iter().any(|d| d.payload == s.payload) {
                Outcome::Decoded
            } else {
                Outcome::NoCandidate
            }
        })
        .collect();
    for &(i, f) in &failed {
        let c = dec.candidates()[i];
        let (frac_row, frac_bin) = sync::refine(spec, &c);
        let freq = ((band.first_bin() + usize::from(c.base)) as f32 + frac_bin) * BIN_HZ;
        let dt = c.dt() + frac_row * sync::ROW_SECONDS;
        for (s, o) in sent.iter().zip(outcomes.iter_mut()) {
            if (s.freq as f32 - freq).abs() <= ON_HZ && (s.dt as f32 - dt).abs() <= ON_S {
                *o = (*o).max(Outcome::of_failure(f));
            }
        }
    }
    Decoded {
        decodes,
        counts: Counts {
            found: rep.candidates_found as u64,
            kept: rep.candidates_kept as u64,
            dropped: rep.candidates_dropped as u64,
            decoded: rep.decoded as u64,
            truncated: rep.decodes_truncated as u64,
            duplicates: rep.duplicates as u64,
            all_zero: rep.all_zero_rejected as u64,
            crc_failed: rep.crc_failed as u64,
            not_converged: rep.not_converged as u64,
            rows_filled: rep.rows_filled as u64,
        },
        bp_rounds,
        seconds,
        outcomes,
    }
}

/// Decodes one slot of a set.
fn decode_slot(set: &str, block: &str, index: usize, slot: &Slot, setup: &Setup) -> SlotResult {
    let d = decode_input(&slot.samples, slot.skip, &slot.sent, setup);
    SlotResult {
        set: set.into(),
        block: block.into(),
        index,
        seed: slot_seed(set, block, index),
        point: slot.point.clone(),
        sample_hash: slot.sample_hash(),
        sent: slot.sent.clone(),
        decodes: d.decodes,
        counts: d.counts,
        bp_rounds: d.bp_rounds,
        seconds: d.seconds,
        outcomes: d.outcomes,
    }
}

/// The output of a command, or "unknown".
fn command(cmd: &str, args: &[&str]) -> String {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

/// Commit (with "+dirty" for uncommitted changes), rustc and CPU.
fn provenance() -> String {
    let dir = env!("CARGO_MANIFEST_DIR");
    let commit = command("git", &["-C", dir, "rev-parse", "HEAD"]);
    let dirty = command("git", &["-C", dir, "status", "--porcelain"]);
    let cpu = fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into());
    format!(
        "commit {commit}{}\nrustc {}\ncpu {cpu}\n",
        if dirty.is_empty() || dirty == "unknown" {
            ""
        } else {
            "+dirty"
        },
        command("rustc", &["--version"])
    )
}

fn run(spec: &str, out: &Path, sets: &str, block: &str) -> Result<(), String> {
    let setup = parse_config(spec)?;
    for set in sets.split(',') {
        set_size(set).ok_or_else(|| format!("unknown set {set}"))?;
    }
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    fs::write(
        out.join("config.txt"),
        format!(
            "{spec}\n{:?}\nband {:?} decodes {}\n{}",
            setup.config,
            setup.band.range_hz(),
            setup.decodes,
            provenance()
        ),
    )
    .map_err(|e| e.to_string())?;
    for set in sets.split(',') {
        let t = Instant::now();
        let n = set_size(set).unwrap_or(0);
        let lines = par_map(n, |i| {
            decode_slot(set, block, i, &build(set, block, i), &setup).to_json()
        });
        let path = out.join(format!("{set}-{block}.jsonl"));
        let mut f = std::io::BufWriter::new(fs::File::create(&path).map_err(|e| e.to_string())?);
        for l in &lines {
            writeln!(f, "{l}").map_err(|e| e.to_string())?;
        }
        eprintln!(
            "{set}-{block}: {n} slots in {:.1} s -> {}",
            t.elapsed().as_secs_f64(),
            path.display()
        );
    }
    Ok(())
}

// -------------------------------------------------------------- compare

/// A saved run's slots by (set, block).
fn load(dir: &Path) -> Result<BTreeMap<(String, String), Vec<SlotResult>>, String> {
    let mut out = BTreeMap::new();
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for e in entries {
        let p = e.map_err(|e| e.to_string())?.path();
        if p.extension().is_some_and(|x| x == "jsonl") {
            let stem = p
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let (set, block) = stem
                .split_once('-')
                .ok_or_else(|| format!("unexpected file {}", p.display()))?;
            let text = fs::read_to_string(&p).map_err(|e| e.to_string())?;
            let mut slots = Vec::new();
            for (n, l) in text.lines().enumerate() {
                let v: Value = serde_json::from_str(l)
                    .map_err(|e| format!("{}:{}: {e}", p.display(), n + 1))?;
                slots.push(
                    SlotResult::from_json(&v)
                        .ok_or_else(|| format!("{}:{}: not a slot record", p.display(), n + 1))?,
                );
            }
            out.insert((set.to_string(), block.to_string()), slots);
        }
    }
    Ok(out)
}

/// Per-point paired counts.
#[derive(Default, Clone, Copy)]
struct Pair {
    messages: u64,
    both: u64,
    old_only: u64,
    new_only: u64,
    false_old: u64,
    false_new: u64,
}

/// The SNR at which the decoded fraction first reaches ½, by linear
/// interpolation between points sorted by SNR (`None` if it never does).
fn half_point(points: &[(f64, f64)]) -> Option<f64> {
    for w in points.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if y0 < 0.5 && y1 >= 0.5 {
            return Some(x0 + (0.5 - y0) * (x1 - x0) / (y1 - y0));
        }
    }
    None
}

/// The `p` quantile of already sorted values (`None` if empty).
fn quantile(sorted: &[f64], p: f64) -> Option<f64> {
    (!sorted.is_empty()).then(|| sorted[((sorted.len() - 1) as f64 * p).round() as usize])
}

/// Sorts and returns the 2.5% and 97.5% quantiles.
fn interval(v: &mut [f64]) -> (f64, f64) {
    v.sort_by(f64::total_cmp);
    (
        quantile(v, 0.025).unwrap_or(f64::NAN),
        quantile(v, 0.975).unwrap_or(f64::NAN),
    )
}

/// The sweep's 50% point from per-point (old, new) outcomes, using all of
/// each point's slots or the resampled `idx`. Points are visited in SNR
/// order: the map is keyed by the SNR as a sortable integer (tenths of a
/// dB), so its order is the SNR order the interpolation needs.
fn curve(
    sweep: &BTreeMap<i64, Vec<(bool, bool)>>,
    new: bool,
    idx: Option<&[Vec<usize>]>,
) -> Option<f64> {
    let pts: Vec<(f64, f64)> = sweep
        .iter()
        .enumerate()
        .map(|(j, (tenths, v))| {
            let pick = |i: usize| if new { v[i].1 } else { v[i].0 };
            let (hits, n) = match idx {
                Some(ix) => (ix[j].iter().filter(|&&i| pick(i)).count(), ix[j].len()),
                None => ((0..v.len()).filter(|&i| pick(i)).count(), v.len()),
            };
            (*tenths as f64 / 10.0, hits as f64 / n as f64)
        })
        .collect();
    half_point(&pts)
}

fn compare(old: &Path, new: &Path, partial: bool) -> Result<bool, String> {
    let (a, b) = (load(old)?, load(new)?);
    let first = |d: &Path| {
        fs::read_to_string(d.join("config.txt"))
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or("")
            .to_string()
    };
    println!("old: {}\nnew: {}", first(old), first(new));
    let mut all_pass = true;
    let keys: std::collections::BTreeSet<&(String, String)> = a.keys().chain(b.keys()).collect();
    let mut compared = 0;
    for key in &keys {
        let (Some(sa), Some(sb)) = (a.get(*key), b.get(*key)) else {
            println!(
                "\n== {} ({}): MISSING from the {} run",
                key.0,
                key.1,
                if a.contains_key(*key) { "new" } else { "old" }
            );
            all_pass = false;
            continue;
        };
        compared += 1;
        let (set, block) = (&key.0, &key.1);
        let mut pass = true;
        if sa.len() != sb.len() {
            println!(
                "\n== {set} ({block}): different slot counts {} and {}",
                sa.len(),
                sb.len()
            );
            all_pass = false;
            continue;
        }
        let mut points: BTreeMap<String, Pair> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        let mut sweep: BTreeMap<i64, Vec<(bool, bool)>> = BTreeMap::new();
        let (mut cost_a, mut cost_b) = ([0u64; 4], [0u64; 4]);
        let mut secs_a: BTreeMap<String, f64> = BTreeMap::new();
        let mut secs_b: BTreeMap<String, f64> = BTreeMap::new();
        let mut input_differs = 0;
        for (x, y) in sa.iter().zip(sb) {
            if x.index != y.index
                || x.seed != y.seed
                || x.sample_hash != y.sample_hash
                || x.sent != y.sent
            {
                input_differs += 1;
                continue;
            }
            if !points.contains_key(&x.point) {
                order.push(x.point.clone());
            }
            let p = points.entry(x.point.clone()).or_default();
            let da: Vec<u128> = x.decodes.iter().map(|d| d.payload).collect();
            let db: Vec<u128> = y.decodes.iter().map(|d| d.payload).collect();
            let sent: Vec<u128> = x.sent.iter().map(|s| s.payload).collect();
            for m in &sent {
                let (ia, ib) = (da.contains(m), db.contains(m));
                p.messages += 1;
                p.both += u64::from(ia && ib);
                p.old_only += u64::from(ia && !ib);
                p.new_only += u64::from(!ia && ib);
                if set == "sweep" {
                    let tenths = (x.point.parse::<f64>().unwrap_or(0.0) * 10.0).round() as i64;
                    sweep.entry(tenths).or_default().push((ia, ib));
                }
            }
            p.false_old += da.iter().filter(|d| !sent.contains(d)).count() as u64;
            p.false_new += db.iter().filter(|d| !sent.contains(d)).count() as u64;
            for (r, cost, secs) in [(x, &mut cost_a, &mut secs_a), (y, &mut cost_b, &mut secs_b)] {
                cost[0] += r.counts.kept;
                cost[1] += r.bp_rounds;
                cost[2] += r.counts.truncated;
                cost[3] += r.counts.dropped;
                for (k, t) in &r.seconds {
                    *secs.entry(k.clone()).or_default() += t;
                }
            }
        }
        println!("\n== {set} ({block}), {} slots", sa.len());
        if input_differs > 0 {
            println!("  INPUT DIFFERS in {input_differs} slots (seed, sample hash or messages): not comparable");
            pass = false;
        }
        println!(
            "  {:>10} {:>8} {:>8} {:>8} {:>6} {:>6} {:>8} {:>8}  verdict",
            "point", "messages", "dec_old", "dec_new", "b", "c", "false_o", "false_n"
        );
        let (mut tb, mut tc) = (0u64, 0u64);
        for pt in &order {
            let p = points[pt];
            let (bb, cc) = (p.old_only as f64, p.new_only as f64);
            let loss_ok = bb - cc <= 3.0 * (bb + cc).sqrt();
            let false_ok = if set == "empty" {
                p.false_new == 0
            } else {
                p.false_new <= p.false_old
            };
            pass &= loss_ok && false_ok;
            tb += p.old_only;
            tc += p.new_only;
            println!(
                "  {:>10} {:>8} {:>8} {:>8} {:>6} {:>6} {:>8} {:>8}  {}{}",
                pt,
                p.messages,
                p.both + p.old_only,
                p.both + p.new_only,
                p.old_only,
                p.new_only,
                p.false_old,
                p.false_new,
                if loss_ok { "" } else { "LOSS " },
                if false_ok { "ok" } else { "FALSE" }
            );
        }
        let (bb, cc) = (tb as f64, tc as f64);
        let set_ok = bb - cc <= 2.0 * (bb + cc).sqrt();
        pass &= set_ok;
        println!(
            "  set: b {tb}, c {tc}, b − c = {} (limit {:.1}): {}",
            tb as i64 - tc as i64,
            2.0 * (bb + cc).sqrt(),
            if set_ok { "ok" } else { "LOSS" }
        );
        if set == "sweep" {
            let (h_old, h_new) = (curve(&sweep, false, None), curve(&sweep, true, None));
            // Paired bootstrap: the same resampled slots for both runs.
            let mut r = Rng(12_345);
            let (mut bo, mut bn, mut bd) = (Vec::new(), Vec::new(), Vec::new());
            let mut dropped = 0;
            for _ in 0..1_000 {
                let ix: Vec<Vec<usize>> = sweep
                    .values()
                    .map(|v| (0..v.len()).map(|_| below(&mut r, v.len())).collect())
                    .collect();
                match (
                    curve(&sweep, false, Some(&ix)),
                    curve(&sweep, true, Some(&ix)),
                ) {
                    (Some(o), Some(n)) => {
                        bo.push(o);
                        bn.push(n);
                        bd.push(n - o);
                    }
                    _ => dropped += 1,
                }
            }
            match (h_old, h_new) {
                (Some(o), Some(n)) => {
                    let shift_ok = n - o <= 0.05;
                    pass &= shift_ok;
                    let ((ol, oh), (nl, nh), (dl, dh)) =
                        (interval(&mut bo), interval(&mut bn), interval(&mut bd));
                    println!(
                        "  50% point: old {o:.3} dB [{ol:.3}, {oh:.3}], new {n:.3} dB [{nl:.3}, {nh:.3}], shift {:+.3} dB [{dl:+.3}, {dh:+.3}] ({dropped} of 1 000 resamples without a 50% point): {}",
                        n - o,
                        if shift_ok { "ok" } else { "SHIFT" }
                    );
                }
                _ => {
                    println!("  50% point: MISSING (old {h_old:?}, new {h_new:?})");
                    pass = false;
                }
            }
        }
        println!(
            "  cost: candidates through BP {} -> {}; rounds per candidate {:.2} -> {:.2}; decodes truncated {} -> {}; candidates dropped {} -> {}",
            cost_a[0],
            cost_b[0],
            cost_a[1] as f64 / cost_a[0].max(1) as f64,
            cost_b[1] as f64 / cost_b[0].max(1) as f64,
            cost_a[2],
            cost_b[2],
            cost_a[3],
            cost_b[3]
        );
        for (side, run) in [("old", sa), ("new", sb)] {
            let crc: u64 = run.iter().map(|r| r.counts.crc_failed).sum();
            let per_slot = crc as f64 / run.len().max(1) as f64;
            let most = run.iter().map(|r| r.counts.crc_failed).max().unwrap_or(0);
            let misses = if run.iter().all(|r| r.outcomes.len() == r.sent.len()) {
                let m = miss_counts(run.iter().flat_map(|r| &r.outcomes));
                format!(
                    "missed: no candidate {}, not converged {}, CRC failure {}, all-zero {}",
                    m[0], m[1], m[2], m[3]
                )
            } else {
                "missed by cause: not recorded".to_string()
            };
            println!(
                "  {side}: CRC failures {crc} ({per_slot:.4} per slot, at most {most}); expected false decodes {:.2e} per slot; {misses}",
                per_slot / 16_384.0
            );
        }
        let mut line = String::from("  summed stage seconds (all threads):");
        for (k, va) in &secs_a {
            let vb = secs_b.get(k).copied().unwrap_or(0.0);
            let _ = write!(line, " {k} {va:.1} -> {vb:.1};");
        }
        println!("{line}");
        println!("  verdict: {}", if pass { "PASS" } else { "FAIL" });
        all_pass &= pass;
    }
    if compared == 0 {
        println!("\nnothing to compare");
        all_pass = false;
    }
    if !partial {
        for set in ["sweep", "crowded", "edges", "empty"] {
            let key = (set.to_string(), "main".to_string());
            if !a.contains_key(&key) && !b.contains_key(&key) {
                println!(
                    "\n== {set} (main): MISSING from both runs (use --partial to compare a subset)"
                );
                all_pass = false;
            }
        }
    }
    println!("\noverall: {}", if all_pass { "PASS" } else { "FAIL" });
    Ok(all_pass)
}

// ----------------------------------------------------------------- cost

fn cost(spec_a: &str, spec_b: &str, slots: usize, runs: usize) -> Result<(), String> {
    let (a, b) = (parse_config(spec_a)?, parse_config(spec_b)?);
    // Evenly over the three sizes: indices 0.., 200.., 400.. in turn.
    let indices: Vec<usize> = (0..slots).map(|k| (k % 3) * 200 + k / 3).collect();
    let input: Vec<Slot> = indices
        .iter()
        .map(|&i| build("crowded", "main", i))
        .collect();
    let runs = runs.max(5);
    let mut per: [BTreeMap<String, Vec<f64>>; 2] = Default::default();
    for r in 0..runs {
        let order = if r % 2 == 0 { [0, 1] } else { [1, 0] };
        for which in order {
            let setup = if which == 0 { &a } else { &b };
            let mut tot: BTreeMap<String, f64> = BTreeMap::new();
            for (s, &i) in input.iter().zip(&indices) {
                for (k, t) in decode_slot("crowded", "main", i, s, setup).seconds {
                    *tot.entry(k).or_default() += t;
                }
            }
            let total: f64 = tot.values().sum();
            tot.insert("total".into(), total);
            for (k, t) in tot {
                per[which].entry(k).or_default().push(t / slots as f64);
            }
        }
    }
    println!(
        "median of {runs} interleaved runs over {slots} crowded slots ({} each of 10, 20 and 40 messages), one thread (ms per slot):",
        slots / 3
    );
    for (k, va) in &per[0] {
        let (mut sa, mut sb) = (va.clone(), per[1][k].clone());
        sa.sort_by(f64::total_cmp);
        sb.sort_by(f64::total_cmp);
        let (ma, mb) = (
            quantile(&sa, 0.5).unwrap_or(f64::NAN),
            quantile(&sb, 0.5).unwrap_or(f64::NAN),
        );
        println!(
            "  {k:<20} A {:8.3}  B {:8.3}  {:+6.1}%",
            ma * 1e3,
            mb * 1e3,
            (mb - ma) / ma * 100.0
        );
    }
    Ok(())
}

const USAGE: &str = "usage: paired_bench run CONFIG OUTDIR [SETS] [BLOCK] | compare OLDDIR NEWDIR [--partial] | cost A B [SLOTS] [RUNS]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    let num =
        |i: usize, d: usize| arg(i).map_or(Ok(d), |s| s.parse().map_err(|e| format!("{s}: {e}")));
    let result = match (arg(0), arg(1), arg(2)) {
        (Some("run"), Some(c), Some(o)) => run(
            c,
            &PathBuf::from(o),
            arg(3).unwrap_or("sweep,crowded,edges,empty"),
            arg(4).unwrap_or("main"),
        )
        .map(|()| true),
        (Some("compare"), Some(o), Some(n)) => {
            compare(Path::new(o), Path::new(n), arg(3) == Some("--partial"))
        }
        (Some("cost"), Some(x), Some(y)) => num(3, 42)
            .and_then(|s| num(4, 5).and_then(|r| cost(x, y, s, r)))
            .map(|()| true),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("paired_bench: {e}");
            ExitCode::from(2)
        }
    }
}
