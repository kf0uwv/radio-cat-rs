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
//! cargo run --release -p cat-ft8 --features profile --example paired_bench -- crc CONFIG [SETS] [BLOCK]
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
//! **crc** measures what continuing past a CRC failure would do, without
//! changing the decoder. It decodes the slots of SETS in BLOCK with CONFIG
//! as `run` does; for every candidate whose belief propagation stopped at
//! a codeword that fails the CRC (or is all zero), it recomputes the
//! candidate's soft bits and runs belief propagation again to the round
//! limit without stopping (`bp::decode_soft_observed`), checking that the
//! re-run first stops where the decoder did. Per point and per set it
//! prints the CRC failures per candidate and per slot; the transmitted
//! messages missing from the output that had a CRC-failing candidate on
//! them (tone 0 within 4 Hz and start within 0.12 s); and what the
//! continued runs reach: the later distinct codewords of each candidate,
//! as transmitted payloads, other payloads passing the CRC, codewords
//! failing it, or all zero; and each candidate's first later codeword
//! that passes the CRC — what a decoder that continued would output — as
//! a gain (a transmitted message missing from the output), a duplicate or
//! a false decode. Every later codeword that passes the CRC is listed.

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
    Band, Candidate, CandidateScratch, CheckRule, Decode, DecodeConfig, DecodeList, Decoder,
    FrontEnd, Stage, CANDIDATES, HOST_DECODES, SMALL_DECODES,
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

/// Builds slot `index` of `set` in `block`.
fn build(set: &str, block: &str, index: usize) -> Slot {
    let seed = slot_seed(set, block, index);
    let mut r = Rng(seed);
    let noise_seed = mix(seed ^ 0x006e_6f69_7365);
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
    for s in &sent {
        add_signal(&mut samples, s);
    }
    add_noise(&mut samples, NOISE_RMS, noise_seed);
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
        o.push_str("}}");
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
        })
    }
}

// --------------------------------------------------------------- decode

/// Decodes one slot.
fn decode_slot(set: &str, block: &str, index: usize, slot: &Slot, setup: &Setup) -> SlotResult {
    let band = setup.band;
    let mut front = Box::new(FrontEnd::new());
    let mut levels = vec![0u8; band.spectrogram_len()];
    let mut sync = vec![0i32; Decoder::sync_scratch_len(band)];
    let mut cands = vec![Candidate::default(); CANDIDATES.max(setup.config.candidates)];
    let mut dec = Decoder::new(
        &mut front,
        &mut levels,
        &mut sync,
        &mut cands,
        band,
        setup.config,
    )
    .unwrap();
    let mut store = vec![Decode::EMPTY; setup.decodes];
    let mut list = DecodeList::new(&mut store);
    let mut scratch = Box::new(CandidateScratch::new());
    let t = Instant::now();
    dec.skip(slot.skip);
    dec.feed(&slot.samples);
    let feed = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let kept = dec.find_candidates().kept;
    let sync_s = t.elapsed().as_secs_f64();
    let mut stages = [0.0f64; Stage::ALL.len()];
    let mut bp_rounds = 0u64;
    for i in 0..kept {
        let mut last = Instant::now();
        let r = dec.decode_candidate_staged(i, &mut scratch, |st| {
            let now = Instant::now();
            stages[st.index()] += (now - last).as_secs_f64();
            last = now;
        });
        bp_rounds += u64::from(scratch.last_rounds());
        if let Some(r) = r {
            list.push(i, &r).unwrap();
        }
    }
    let rep = dec.report(&list);
    let mut seconds = vec![("feed".to_string(), feed), ("sync".to_string(), sync_s)];
    for st in Stage::ALL {
        seconds.push((st.name().to_string(), stages[st.index()]));
    }
    SlotResult {
        set: set.into(),
        block: block.into(),
        index,
        seed: slot_seed(set, block, index),
        point: slot.point.clone(),
        sample_hash: slot.sample_hash(),
        sent: slot.sent.clone(),
        decodes: list
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
            .collect(),
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

// ------------------------------------------------------------------ crc

/// Positional tolerance for "a candidate on a transmitted message": tone 0
/// within this many Hz (a little over one bin) and the start within this
/// many seconds (a little over one row).
const ON_HZ: f32 = 4.0;
const ON_S: f32 = 0.12;

/// What one continued belief-propagation run reached after its first
/// codeword (a CRC or all-zero failure).
#[derive(Default, Clone, Copy)]
struct Continued {
    /// Candidates continued.
    candidates: u64,
    /// Rounds run after the first codeword (the cost of continuing), and
    /// of those, rounds whose hard decisions were a codeword (the same or
    /// another).
    extra_rounds: u64,
    codeword_rounds: u64,
    /// Distinct later codewords other than the first, per candidate.
    codewords: u64,
    /// Of those: passing the CRC with a transmitted payload, with some
    /// other payload, failing the CRC, all zero.
    correct: u64,
    wrong_pass: u64,
    wrong_crc: u64,
    all_zero: u64,
    /// The first CRC-passing later codeword per candidate (what a decoder
    /// that continues would output): a transmitted message missing from
    /// the slot's output (gain, counted once per slot), one already in
    /// it, and a payload not transmitted (a false decode).
    first_gain: u64,
    first_duplicate: u64,
    first_false: u64,
}

impl Continued {
    fn add(&mut self, o: &Continued) {
        self.candidates += o.candidates;
        self.extra_rounds += o.extra_rounds;
        self.codeword_rounds += o.codeword_rounds;
        self.codewords += o.codewords;
        self.correct += o.correct;
        self.wrong_pass += o.wrong_pass;
        self.wrong_crc += o.wrong_crc;
        self.all_zero += o.all_zero;
        self.first_gain += o.first_gain;
        self.first_duplicate += o.first_duplicate;
        self.first_false += o.first_false;
    }
}

/// CRC counts of one slot (or a sum of slots).
#[derive(Default, Clone, Copy)]
struct CrcCounts {
    slots: u64,
    /// Slots with at least one CRC failure, and the most in one slot.
    slots_with_crc: u64,
    max_crc_in_slot: u64,
    candidates: u64,
    decoded: u64,
    crc: u64,
    all_zero: u64,
    not_converged: u64,
    rounds: u64,
    /// Decodes whose position is on their transmitted message (a check of
    /// the positional match).
    decodes_on_message: u64,
    /// CRC failures on a transmitted message by position; of those, on a
    /// message missing from the output.
    crc_on_message: u64,
    crc_on_missing: u64,
    /// Sum of each CRC-failing codeword's distance (bits of 174) to the
    /// nearest transmitted codeword, and how many were within 20 bits.
    crc_distance: u64,
    crc_near: u64,
    /// Transmitted messages; missing from the output; missing with a
    /// CRC-failing candidate on them.
    messages: u64,
    missing: u64,
    missing_with_crc: u64,
    past_crc: Continued,
    past_all_zero: Continued,
}

impl CrcCounts {
    fn add(&mut self, o: &CrcCounts) {
        self.slots += o.slots;
        self.slots_with_crc += o.slots_with_crc;
        self.max_crc_in_slot = self.max_crc_in_slot.max(o.max_crc_in_slot);
        self.candidates += o.candidates;
        self.decoded += o.decoded;
        self.crc += o.crc;
        self.all_zero += o.all_zero;
        self.not_converged += o.not_converged;
        self.rounds += o.rounds;
        self.decodes_on_message += o.decodes_on_message;
        self.crc_on_message += o.crc_on_message;
        self.crc_on_missing += o.crc_on_missing;
        self.crc_distance += o.crc_distance;
        self.crc_near += o.crc_near;
        self.messages += o.messages;
        self.missing += o.missing;
        self.missing_with_crc += o.missing_with_crc;
        self.past_crc.add(&o.past_crc);
        self.past_all_zero.add(&o.past_all_zero);
    }
}

/// Decodes one slot as `decode_slot` does, and for every candidate whose
/// belief propagation stopped at a CRC (or all-zero) failure re-runs it
/// from the same soft bits without stopping, up to the round limit. The
/// decoder's own results are not changed: the re-run is a separate
/// measurement, checked to stop where the decoder stopped. Returns the
/// counts and one line per later codeword that passes the CRC.
fn crc_slot(
    set: &str,
    block: &str,
    index: usize,
    slot: &Slot,
    setup: &Setup,
) -> (CrcCounts, Vec<String>) {
    use cat_ft8::decode::{bp, soft, sync, BpFailure, BIN_HZ};
    use cat_ft8::ldpc::{self, Codeword};
    let band = setup.band;
    let cfg = setup.config;
    let mut front = Box::new(FrontEnd::new());
    let mut levels = vec![0u8; band.spectrogram_len()];
    let mut sync_buf = vec![0i32; Decoder::sync_scratch_len(band)];
    let mut cands = vec![Candidate::default(); CANDIDATES.max(cfg.candidates)];
    let mut dec = Decoder::new(
        &mut front,
        &mut levels,
        &mut sync_buf,
        &mut cands,
        band,
        cfg,
    )
    .unwrap();
    let mut store = vec![Decode::EMPTY; setup.decodes];
    let mut list = DecodeList::new(&mut store);
    let mut scratch = Box::new(CandidateScratch::new());
    dec.skip(slot.skip);
    dec.feed(&slot.samples);
    let kept = dec.find_candidates().kept;
    let mut results = Vec::with_capacity(kept);
    for i in 0..kept {
        let r = dec.decode_candidate(i, &mut scratch).unwrap();
        results.push((r, scratch.last_rounds()));
        list.push(i, &r).unwrap();
    }
    let output: Vec<u128> = list
        .finish_payloads()
        .iter()
        .map(|d| d.payload.bits())
        .collect();
    let sent: Vec<u128> = slot.sent.iter().map(|s| s.payload).collect();
    let sent_cw: Vec<Codeword> = sent
        .iter()
        .map(|&p| ldpc::encode(cat_ft8::crc::append_crc(&Payload::from_bits(p).unwrap())))
        .collect();
    let distance =
        |a: &Codeword, b: &Codeword| (0..ldpc::N).filter(|&i| a.bit(i) != b.bit(i)).count();
    let spec = dec.spectrogram();
    let mut n = CrcCounts {
        slots: 1,
        candidates: kept as u64,
        messages: sent.len() as u64,
        ..CrcCounts::default()
    };
    let mut crc_on: Vec<bool> = vec![false; sent.len()];
    let mut gained: Vec<u128> = Vec::new();
    let mut lines = Vec::new();
    let mut llr = [0.0f32; ldpc::N];
    let mut bps = Box::new(bp::BpScratch::new());
    for (i, &(r, rounds)) in results.iter().enumerate() {
        n.rounds += u64::from(rounds);
        let c = dec.candidates()[i];
        let (frac_row, frac_bin) = sync::refine(spec, &c);
        let freq = ((band.first_bin() + usize::from(c.base)) as f32 + frac_bin) * BIN_HZ;
        let dt = c.dt() + frac_row * sync::ROW_SECONDS;
        let on: Vec<usize> = slot
            .sent
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                (s.freq as f32 - freq).abs() <= ON_HZ && (s.dt as f32 - dt).abs() <= ON_S
            })
            .map(|(k, _)| k)
            .collect();
        match r {
            Ok(d) => {
                n.decoded += 1;
                n.decodes_on_message += u64::from(on.iter().any(|&k| sent[k] == d.payload.bits()));
                continue;
            }
            Err(BpFailure::NotConverged) => {
                n.not_converged += 1;
                continue;
            }
            Err(BpFailure::Crc) => n.crc += 1,
            Err(BpFailure::AllZero) => n.all_zero += 1,
            Err(_) => continue,
        }
        let first_failure = r.err();
        // The decoder's soft bits for this candidate, recomputed.
        soft::soft_bits_at(spec, &c, frac_row, frac_bin, cfg.llr_scale, &mut llr);
        let mut first: Option<(u8, Codeword)> = None;
        let mut seen: Vec<u128> = Vec::new();
        let mut cont = Continued {
            candidates: 1,
            ..Continued::default()
        };
        let mut first_pass: Option<u128> = None;
        let mut events = Vec::new();
        let mut later = 0u64;
        let run = bp::decode_soft_observed(
            &llr,
            cfg.rule,
            cfg.max_rounds,
            &mut bps,
            |round, hard, res| {
                let cw = Codeword::from_bits(hard);
                match first {
                    None => {
                        assert_eq!(
                        (res.err(), round),
                        (first_failure, rounds),
                        "{set}-{block} slot {index} candidate {i}: re-run differs from the decoder"
                    );
                        first = Some((round, cw));
                        seen.push(cw.message());
                    }
                    Some(_) => {
                        later += 1;
                        if !seen.contains(&cw.message()) {
                            seen.push(cw.message());
                            events.push((round, res));
                        }
                    }
                }
                true
            },
        );
        let (first_round, first_cw) = first.expect("the decoder stopped at a codeword");
        cont.extra_rounds = u64::from(run - first_round);
        cont.codeword_rounds = later;
        for (round, res) in events {
            cont.codewords += 1;
            match res {
                Ok(d) => {
                    let p = d.payload.bits();
                    let correct = sent.contains(&p);
                    if correct {
                        cont.correct += 1;
                    } else {
                        cont.wrong_pass += 1;
                    }
                    let fresh = first_pass.is_none();
                    if fresh {
                        first_pass = Some(p);
                        if !correct {
                            cont.first_false += 1;
                        } else if output.contains(&p) || gained.contains(&p) {
                            cont.first_duplicate += 1;
                        } else {
                            cont.first_gain += 1;
                            gained.push(p);
                        }
                    }
                    lines.push(format!(
                        "  {set}-{block} slot {index} ({}) candidate {i} after {:?} at round {first_round}: round {round} {} {:020x}{}",
                        slot.point,
                        first_failure.unwrap(),
                        if correct { "transmitted" } else { "NOT TRANSMITTED" },
                        p,
                        if fresh { if correct && !output.contains(&p) { " (first pass, new)" } else { " (first pass)" } } else { "" },
                    ));
                }
                Err(BpFailure::AllZero) => cont.all_zero += 1,
                Err(_) => cont.wrong_crc += 1,
            }
        }
        if first_failure == Some(BpFailure::Crc) {
            n.past_crc.add(&cont);
            let d = sent_cw
                .iter()
                .map(|s| distance(&first_cw, s))
                .min()
                .unwrap_or(ldpc::N);
            n.crc_distance += d as u64;
            n.crc_near += u64::from(d <= 20);
            if !on.is_empty() {
                n.crc_on_message += 1;
                n.crc_on_missing += u64::from(on.iter().any(|&k| !output.contains(&sent[k])));
            }
            for &k in &on {
                crc_on[k] = true;
            }
        } else {
            n.past_all_zero.add(&cont);
        }
    }
    for (k, p) in sent.iter().enumerate() {
        if !output.contains(p) {
            n.missing += 1;
            n.missing_with_crc += u64::from(crc_on[k]);
        }
    }
    n.slots_with_crc = u64::from(n.crc > 0);
    n.max_crc_in_slot = n.crc;
    (n, lines)
}

fn crc_report(spec: &str, sets: &str, block: &str) -> Result<(), String> {
    let setup = parse_config(spec)?;
    for set in sets.split(',') {
        set_size(set).ok_or_else(|| format!("unknown set {set}"))?;
    }
    println!("config {spec}: {:?}\n{}", setup.config, provenance());
    for set in sets.split(',') {
        let size = set_size(set).unwrap_or(0);
        let per = par_map(size, |i| {
            let slot = build(set, block, i);
            (slot.point.clone(), crc_slot(set, block, i, &slot, &setup))
        });
        let mut points: BTreeMap<String, CrcCounts> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        let mut total = CrcCounts::default();
        let mut lines = Vec::new();
        for (pt, (c, l)) in per {
            if !points.contains_key(&pt) {
                order.push(pt.clone());
            }
            points.entry(pt).or_default().add(&c);
            total.add(&c);
            lines.extend(l);
        }
        println!("\n== {set} ({block}), {size} slots");
        println!(
            "  {:>9} {:>6} {:>7} {:>6} {:>6} {:>6} {:>6} {:>7} {:>6} {:>7} | {:>6} {:>6} {:>6} {:>6} {:>6} {:>5} {:>5} {:>5}",
            "point", "msgs", "missing", "m+crc", "cands", "dec", "crc", "crc/slt", "maxslt", "zero",
            "cont", "cws", "right", "wrongP", "wrongF", "gain", "dup", "false"
        );
        let row = |label: &str, p: &CrcCounts| {
            let k = &p.past_crc;
            println!(
                "  {:>9} {:>6} {:>7} {:>6} {:>6} {:>6} {:>6} {:>7.3} {:>6} {:>7} | {:>6} {:>6} {:>6} {:>6} {:>6} {:>5} {:>5} {:>5}",
                label, p.messages, p.missing, p.missing_with_crc, p.candidates, p.decoded, p.crc,
                p.crc as f64 / p.slots.max(1) as f64, p.max_crc_in_slot, p.all_zero,
                k.candidates, k.codewords, k.correct, k.wrong_pass, k.wrong_crc, k.first_gain, k.first_duplicate, k.first_false
            );
        };
        for pt in &order {
            row(pt, &points[pt]);
        }
        row("set", &total);
        let t = &total;
        println!(
            "  CRC failures: {} of {} candidates ({:.4} per candidate), {:.3} per slot, {} of {} slots with one or more, at most {} in a slot",
            t.crc, t.candidates, t.crc as f64 / t.candidates.max(1) as f64, t.crc as f64 / t.slots.max(1) as f64, t.slots_with_crc, t.slots, t.max_crc_in_slot
        );
        println!(
            "  positional match: {} of {} decodes on their message; CRC failures on a transmitted message {} (on one missing from the output {}); mean distance to the nearest transmitted codeword {:.1} bits, {} within 20 bits",
            t.decodes_on_message, t.decoded, t.crc_on_message, t.crc_on_missing,
            t.crc_distance as f64 / t.crc.max(1) as f64, t.crc_near
        );
        let k = &t.past_crc;
        println!(
            "  past a CRC failure: {} candidates continued, {} extra rounds ({:.2}% of the {} rounds run), {} of them at a codeword; {} later codewords: {} transmitted, {} other passing, {} failing the CRC, {} all-zero; first pass: gain {}, duplicate {}, false {}",
            k.candidates, k.extra_rounds, 100.0 * k.extra_rounds as f64 / t.rounds.max(1) as f64, t.rounds, k.codeword_rounds,
            k.codewords, k.correct, k.wrong_pass, k.wrong_crc, k.all_zero, k.first_gain, k.first_duplicate, k.first_false
        );
        let z = &t.past_all_zero;
        println!(
            "  past an all-zero rejection: {} candidates continued, {} extra rounds, {} of them at a codeword; {} later codewords: {} transmitted, {} other passing, {} failing the CRC, {} all-zero; first pass: gain {}, duplicate {}, false {}",
            z.candidates, z.extra_rounds, z.codeword_rounds, z.codewords, z.correct, z.wrong_pass, z.wrong_crc, z.all_zero, z.first_gain, z.first_duplicate, z.first_false
        );
        println!(
            "  expected false decodes from codewords not transmitted (each passes the 14-bit CRC with probability 1/16 384): as decoded {:.4}; continuing past a CRC failure adds {:.4}",
            t.crc as f64 / 16_384.0,
            (k.wrong_crc + k.wrong_pass) as f64 / 16_384.0
        );
        for l in lines {
            println!("{l}");
        }
    }
    Ok(())
}

const USAGE: &str = "usage: paired_bench run CONFIG OUTDIR [SETS] [BLOCK] | compare OLDDIR NEWDIR [--partial] | cost A B [SLOTS] [RUNS] | crc CONFIG [SETS] [BLOCK]";

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
        (Some("crc"), Some(c), _) => crc_report(
            c,
            arg(2).unwrap_or("sweep,crowded,edges,empty"),
            arg(3).unwrap_or("main"),
        )
        .map(|()| true),
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
