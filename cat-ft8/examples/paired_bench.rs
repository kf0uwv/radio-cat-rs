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
//! cargo run --release -p cat-ft8 --features profile --example paired_bench -- compare OLDDIR NEWDIR
//! cargo run --release -p cat-ft8 --features profile --example paired_bench -- cost CONFIG_A CONFIG_B [SLOTS] [RUNS]
//! ```
//!
//! **Sets** (encoder-made, seeded; each slot's seed comes from its set,
//! block and index, so two runs decode bit-identical input):
//!
//! - `sweep`: one signal per slot at random offsets (200–2 800 Hz,
//!   −0.3…+0.7 s), −22…−15 dB in 0.5 dB steps, 1 000 slots per point;
//! - `crowded`: 10, 20 and 40 distinct messages per slot (−16…+6 dB, same
//!   offsets, overlaps allowed), 200 slots per size;
//! - `edges`: one signal at −18 dB (near the decode threshold, so a loss
//!   shows), 100 slots per case: starts at
//!   ±1.6/±2.0/±2.4 s, tone 0 near each end of both bands, input cut at
//!   13.5 s, and input starting 1 s late;
//! - `empty`: no signal — noise; noise with weak (0–20 dB) or strong
//!   (20–50 dB) carriers; clipped full-scale noise; floored low-level
//!   noise; impulsive noise.
//!
//! SETS is a comma list (default `sweep,crowded,edges,empty`); BLOCK is
//! `main` (default) or `fresh` (other seeds, for confirming a chosen
//! value). Signal SNRs are in 2500 Hz against white noise of RMS 0.03.
//!
//! **CONFIG** is `host` or `small` (the small setup: 200–2900 Hz, min-sum,
//! 64 decodes) followed by optional `,key=value` changes: `rule=sp|ms`,
//! `rounds=N`, `scale=X`, `candidates=N`, `min_score=N`. Example:
//! `host,candidates=100`.
//!
//! **run** decodes every slot (all threads; results do not depend on the
//! thread count) and writes `OUTDIR/<set>-<block>.jsonl`: one JSON object
//! per slot with its set, block, index, seed and point, the transmitted
//! messages, the decodes (payload in hex and every field), the report
//! counts and the slot's stage times; `OUTDIR/config.txt` records the
//! configuration.
//!
//! **compare** pairs two runs slot by slot and message by message and
//! prints, per point: transmitted messages, decoded by both, by the old
//! run only (b), by the new only (c), false decodes of each, and the
//! verdicts — no loss beyond noise (b − c ≤ 3·√(b + c) at every point and
//! ≤ 2·√(b + c) over a set), false decodes (zero on `empty`; elsewhere new
//! ≤ old at every point) and, for `sweep`, each run's 50% point with a
//! bootstrap 95% interval and the paired shift (it fails if the new 50%
//! point is more than 0.05 dB worse). It also prints the
//! deterministic cost counts (candidates through belief propagation and
//! rounds per candidate) and the summed stage times.
//!
//! **cost** times two configurations interleaved, one thread, on the first
//! SLOTS crowded slots (default 40) for RUNS rounds (default 5, at least
//! 5), and prints each stage's median.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use cat_ft8::decode::{
    Band, Candidate, CandidateScratch, CheckRule, Decode, DecodeConfig, DecodeList, Decoder,
    FrontEnd, Stage, CANDIDATES, HOST_DECODES, SMALL_DECODES,
};
use cat_ft8::message::{pack, NoHashes, Payload};
use cat_ft8::waveform::{synthesize, Placement};
use cat_ft8::{encode_payload, SLOT_SAMPLES};
use serde_json::{json, Value};

const NOISE_RMS: f64 = 0.03;

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

/// splitmix64 with Box–Muller normals through `libm`, so slots are the
/// same bits on every platform.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mix(self.0)
    }
    fn uniform(&mut self) -> f64 {
        ((self.next() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }
    fn below(&mut self, n: usize) -> usize {
        (self.uniform() * n as f64) as usize % n
    }
    fn gauss(&mut self) -> f64 {
        let (u, v) = (self.uniform(), self.uniform());
        libm::sqrt(-2.0 * libm::log(u)) * libm::cos(2.0 * core::f64::consts::PI * v)
    }
}

// ------------------------------------------------------------- messages

/// A random standard callsign (prefix, digit, suffix).
fn callsign(r: &mut Rng) -> String {
    const L: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut s = String::new();
    s.push(L[r.below(26)] as char);
    if r.below(2) == 0 {
        s.push(L[r.below(26)] as char);
    }
    s.push((b'0' + r.below(10) as u8) as char);
    for _ in 0..1 + r.below(3) {
        s.push(L[r.below(26)] as char);
    }
    s
}

/// A random grid square.
fn grid(r: &mut Rng) -> String {
    const F: &[u8] = b"ABCDEFGHIJKLMNOPQR";
    format!(
        "{}{}{}{}",
        F[r.below(18)] as char,
        F[r.below(18)] as char,
        r.below(10),
        r.below(10)
    )
}

/// A random packable message, its payload not in `avoid`.
fn message(r: &mut Rng, avoid: &[Payload]) -> (String, Payload) {
    loop {
        let (a, b) = (callsign(r), callsign(r));
        let text = match r.below(6) {
            0 => format!("CQ {a} {}", grid(r)),
            1 => format!("{a} {b} {}", grid(r)),
            2 => format!("{a} {b} {:+03}", r.below(51) as i32 - 30),
            3 => format!("{a} {b} R{:+03}", r.below(51) as i32 - 30),
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
#[derive(Clone)]
struct Sent {
    text: String,
    payload: Payload,
    base_hz: f64,
    dt: f64,
    snr: f64,
}

/// A slot: its point label, the samples to feed (after `skip` missing
/// samples), and what was sent.
struct Slot {
    point: String,
    samples: Vec<f32>,
    skip: usize,
    sent: Vec<Sent>,
}

fn amplitude(snr: f64) -> f32 {
    let n2500 = NOISE_RMS * NOISE_RMS * 2500.0 / 6000.0;
    libm::sqrt(2.0 * n2500 * libm::pow(10.0, snr / 10.0)) as f32
}

fn add_signal(out: &mut [f32], s: &Sent) {
    let mut p = Placement::slot(s.base_hz as f32, s.dt as f32).unwrap();
    p.amplitude = amplitude(s.snr);
    synthesize(&encode_payload(&s.payload), &p, out).unwrap();
}

fn add_noise(out: &mut [f32], rms: f64, r: &mut Rng) {
    for x in out.iter_mut() {
        *x += (rms * r.gauss()) as f32;
    }
}

fn add_carriers(out: &mut [f32], r: &mut Rng, lo_db: f64, hi_db: f64) {
    for _ in 0..1 + r.below(4) {
        let f = 100.0 + 3_000.0 * r.uniform();
        let a = f64::from(amplitude(lo_db + (hi_db - lo_db) * r.uniform()));
        let ph = 2.0 * core::f64::consts::PI * r.uniform();
        for (n, x) in out.iter_mut().enumerate() {
            *x +=
                (a * libm::cos(2.0 * core::f64::consts::PI * f * n as f64 / 12_000.0 + ph)) as f32;
        }
    }
}

/// The slots of a set: (count, builder of slot `index` from its RNG).
fn set_size(set: &str) -> usize {
    match set {
        "sweep" => 15 * 1_000,
        "crowded" => 3 * 200,
        "edges" => EDGE_CASES.len() * 100,
        "empty" => 2_300,
        _ => panic!("unknown set {set}"),
    }
}

/// Edge cases: (label, dt, base Hz or None for random, cut at sample, skip).
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

fn build(set: &str, block: &str, index: usize) -> Slot {
    let mut r = Rng(slot_seed(set, block, index));
    let mut samples = vec![0.0f32; SLOT_SAMPLES];
    let mut sent = Vec::new();
    let one = |r: &mut Rng, snr: f64, dt: Option<f64>, f: Option<f64>| {
        let (text, payload) = message(r, &[]);
        let base_hz = f.unwrap_or_else(|| 200.0 + 2_600.0 * r.uniform());
        let dt = dt.unwrap_or_else(|| -0.3 + r.uniform());
        Sent {
            text,
            payload,
            base_hz,
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
                let payloads: Vec<Payload> = sent.iter().map(|s: &Sent| s.payload).collect();
                let (text, payload) = message(&mut r, &payloads);
                sent.push(Sent {
                    text,
                    payload,
                    base_hz: 200.0 + 2_600.0 * r.uniform(),
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
            add_noise(&mut samples, rms, &mut r);
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
    add_noise(&mut samples, NOISE_RMS, &mut r);
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

fn parse_config(spec: &str) -> Setup {
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
        other => panic!("config must start with host or small, not {other:?}"),
    };
    for kv in parts {
        let (k, v) = kv
            .split_once('=')
            .unwrap_or_else(|| panic!("bad setting {kv}"));
        s.config = match k {
            "rule" => s.config.with_rule(match v {
                "sp" => CheckRule::SumProduct,
                "ms" => CheckRule::MinSum,
                _ => panic!("rule must be sp or ms"),
            }),
            "rounds" => s.config.with_max_rounds(v.parse().unwrap()),
            "scale" => s.config.with_llr_scale(v.parse().unwrap()),
            "candidates" => s.config.with_candidates(v.parse().unwrap()),
            "min_score" => s.config.with_min_score(v.parse().unwrap()),
            _ => panic!("unknown setting {k}"),
        };
    }
    s
}

// --------------------------------------------------------------- decode

/// One slot's decode: the output lines' fields, report and stage times.
fn decode_slot(slot: &Slot, setup: &Setup) -> Value {
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
    let mut rounds = 0u64;
    for i in 0..kept {
        let mut last = Instant::now();
        let r = dec.decode_candidate_staged(i, &mut scratch, |st| {
            let now = Instant::now();
            stages[st.index()] += (now - last).as_secs_f64();
            last = now;
        });
        rounds += u64::from(scratch.last_rounds());
        if let Some(r) = r {
            list.push(i, &r).unwrap();
        }
    }
    let rep = dec.report(&list);
    let decodes: Vec<Value> = list
        .finish_payloads()
        .iter()
        .map(|d| {
            json!({
                "payload": format!("{:020x}", d.payload.bits()),
                "freq": d.freq_hz, "dt": d.dt, "snr": d.snr_db, "score": d.score,
                "candidate": d.candidate, "hard_errors": d.hard_errors, "rounds": d.rounds,
            })
        })
        .collect();
    let mut times = serde_json::Map::new();
    times.insert("feed".into(), json!(feed));
    times.insert("sync".into(), json!(sync_s));
    for st in Stage::ALL {
        times.insert(st.name().into(), json!(stages[st.index()]));
    }
    json!({
        "decodes": decodes,
        "report": {
            "candidates_found": rep.candidates_found, "candidates_kept": rep.candidates_kept,
            "decoded": rep.decoded, "duplicates": rep.duplicates,
            "all_zero_rejected": rep.all_zero_rejected, "crc_failed": rep.crc_failed,
            "not_converged": rep.not_converged, "rows_filled": rep.rows_filled,
        },
        "bp_rounds": rounds,
        "seconds": Value::Object(times),
    })
}

/// `f(i)` for `i` in `0..n` on all threads, in index order.
fn par_map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism()
        .map_or(4, |x| x.get())
        .min(n.max(1));
    let f = &f;
    let mut out: Vec<(usize, T)> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|t| {
                s.spawn(move || {
                    (t..n)
                        .step_by(threads)
                        .map(|i| (i, f(i)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    out.sort_by_key(|p| p.0);
    out.into_iter().map(|p| p.1).collect()
}

fn run(spec: &str, out: &Path, sets: &str, block: &str) {
    let setup = parse_config(spec);
    fs::create_dir_all(out).unwrap();
    fs::write(
        out.join("config.txt"),
        format!(
            "{spec}\n{:?}\nband {:?} decodes {}\n",
            setup.config,
            setup.band.range_hz(),
            setup.decodes
        ),
    )
    .unwrap();
    for set in sets.split(',') {
        let t = Instant::now();
        let n = set_size(set);
        let lines = par_map(n, |i| {
            let slot = build(set, block, i);
            let mut v = decode_slot(&slot, &setup);
            let sent: Vec<Value> = slot
                .sent
                .iter()
                .map(|s| json!({"payload": format!("{:020x}", s.payload.bits()), "text": s.text, "freq": s.base_hz, "dt": s.dt, "snr": s.snr}))
                .collect();
            let o = v.as_object_mut().unwrap();
            o.insert("set".into(), json!(set));
            o.insert("block".into(), json!(block));
            o.insert("index".into(), json!(i));
            o.insert(
                "seed".into(),
                json!(format!("{:016x}", slot_seed(set, block, i))),
            );
            o.insert("point".into(), json!(slot.point));
            o.insert("sent".into(), Value::Array(sent));
            v.to_string()
        });
        let path = out.join(format!("{set}-{block}.jsonl"));
        let mut f = std::io::BufWriter::new(fs::File::create(&path).unwrap());
        for l in &lines {
            writeln!(f, "{l}").unwrap();
        }
        eprintln!(
            "{set}-{block}: {n} slots in {:.1} s -> {}",
            t.elapsed().as_secs_f64(),
            path.display()
        );
    }
}

// -------------------------------------------------------------- compare

/// A saved run's slots by (set, block, index).
fn load(dir: &Path) -> BTreeMap<(String, String), Vec<Value>> {
    let mut out = BTreeMap::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "jsonl") {
            let stem = p.file_stem().unwrap().to_string_lossy().to_string();
            let (set, block) = stem.split_once('-').unwrap();
            let v: Vec<Value> = fs::read_to_string(&p)
                .unwrap()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
            out.insert((set.to_string(), block.to_string()), v);
        }
    }
    out
}

fn decoded(slot: &Value) -> Vec<&str> {
    slot["decodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["payload"].as_str().unwrap())
        .collect()
}

fn sent(slot: &Value) -> Vec<&str> {
    slot["sent"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["payload"].as_str().unwrap())
        .collect()
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

/// The SNR at which the decoded fraction first reaches 0.5, by linear
/// interpolation between points (`None` if it never does).
fn half_point(points: &[(f64, f64)]) -> Option<f64> {
    for w in points.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if y0 < 0.5 && y1 >= 0.5 {
            return Some(x0 + (0.5 - y0) * (x1 - x0) / (y1 - y0));
        }
    }
    None
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn compare(old: &Path, new: &Path) {
    let (a, b) = (load(old), load(new));
    println!(
        "old: {}",
        fs::read_to_string(old.join("config.txt"))
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or("")
    );
    println!(
        "new: {}",
        fs::read_to_string(new.join("config.txt"))
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or("")
    );
    let mut all_pass = true;
    for (key, sa) in &a {
        let Some(sb) = b.get(key) else { continue };
        assert_eq!(sa.len(), sb.len(), "{key:?}: different slot counts");
        let (set, block) = key;
        let mut points: BTreeMap<String, Pair> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        // Per slot (sweep): decoded or not, for the bootstrap.
        let mut sweep: BTreeMap<String, Vec<(bool, bool)>> = BTreeMap::new();
        let (mut cand_a, mut cand_b, mut rounds_a, mut rounds_b) = (0u64, 0u64, 0u64, 0u64);
        let mut secs_a: BTreeMap<String, f64> = BTreeMap::new();
        let mut secs_b: BTreeMap<String, f64> = BTreeMap::new();
        for (x, y) in sa.iter().zip(sb) {
            assert_eq!(x["seed"], y["seed"], "{key:?}: slots differ");
            assert_eq!(x["sent"], y["sent"], "{key:?}: transmitted messages differ");
            let point = x["point"].as_str().unwrap().to_string();
            if !points.contains_key(&point) {
                order.push(point.clone());
            }
            let p = points.entry(point.clone()).or_default();
            let (s, da, db) = (sent(x), decoded(x), decoded(y));
            for m in &s {
                let (ia, ib) = (da.contains(m), db.contains(m));
                p.messages += 1;
                p.both += u64::from(ia && ib);
                p.old_only += u64::from(ia && !ib);
                p.new_only += u64::from(!ia && ib);
                if set == "sweep" {
                    sweep.entry(point.clone()).or_default().push((ia, ib));
                }
            }
            p.false_old += da.iter().filter(|d| !s.contains(d)).count() as u64;
            p.false_new += db.iter().filter(|d| !s.contains(d)).count() as u64;
            for (v, cand, rounds, secs) in [
                (x, &mut cand_a, &mut rounds_a, &mut secs_a),
                (y, &mut cand_b, &mut rounds_b, &mut secs_b),
            ] {
                *cand += v["report"]["candidates_kept"].as_u64().unwrap();
                *rounds += v["bp_rounds"].as_u64().unwrap();
                for (k, t) in v["seconds"].as_object().unwrap() {
                    *secs.entry(k.clone()).or_default() += t.as_f64().unwrap();
                }
            }
        }
        println!("\n== {set} ({block}), {} slots", sa.len());
        println!(
            "  {:>10} {:>8} {:>8} {:>8} {:>6} {:>6} {:>8} {:>8}  verdict",
            "point", "messages", "dec_old", "dec_new", "b", "c", "false_o", "false_n"
        );
        let (mut tb, mut tc) = (0u64, 0u64);
        let mut pass = true;
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
        // ("both" column above is the old run's decoded count.)
        if set == "sweep" {
            let curve =
                |pick: fn(&(bool, bool)) -> bool, idx: Option<&[Vec<usize>]>| -> Option<f64> {
                    let pts: Vec<(f64, f64)> = sweep
                        .iter()
                        .enumerate()
                        .map(|(j, (k, v))| {
                            let snr: f64 = k.parse().unwrap();
                            let hits = match idx {
                                Some(ix) => ix[j].iter().filter(|&&i| pick(&v[i])).count(),
                                None => v.iter().filter(|e| pick(e)).count(),
                            };
                            let n = idx.map_or(v.len(), |ix| ix[j].len());
                            (snr, hits as f64 / n as f64)
                        })
                        .collect();
                    let mut pts = pts;
                    pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                    half_point(&pts)
                };
            let (h_old, h_new) = (curve(|e| e.0, None), curve(|e| e.1, None));
            // Paired bootstrap: resample slots within each point.
            let mut r = Rng(12_345);
            let (mut bo, mut bn, mut bd) = (Vec::new(), Vec::new(), Vec::new());
            for _ in 0..1_000 {
                let ix: Vec<Vec<usize>> = sweep
                    .values()
                    .map(|v| (0..v.len()).map(|_| r.below(v.len())).collect())
                    .collect();
                if let (Some(o), Some(n)) = (curve(|e| e.0, Some(&ix)), curve(|e| e.1, Some(&ix))) {
                    bo.push(o);
                    bn.push(n);
                    bd.push(n - o);
                }
            }
            if let (Some(o), Some(n)) = (h_old, h_new) {
                // The new 50% point may be at most 0.05 dB worse (higher);
                // the interval is reported for judgement.
                let shift_ok = n - o <= 0.05;
                println!(
                    "  50% point: old {o:.3} dB [{:.3}, {:.3}], new {n:.3} dB [{:.3}, {:.3}], shift {:+.3} dB [{:+.3}, {:+.3}]: {}",
                    percentile(&mut bo, 0.025), percentile(&mut bo.clone(), 0.975),
                    percentile(&mut bn, 0.025), percentile(&mut bn.clone(), 0.975),
                    n - o, percentile(&mut bd, 0.025), percentile(&mut bd.clone(), 0.975),
                    if shift_ok { "ok" } else { "SHIFT" }
                );
                pass &= shift_ok;
            }
        }
        println!(
            "  cost: candidates through BP {cand_a} -> {cand_b}; rounds per candidate {:.2} -> {:.2}",
            rounds_a as f64 / cand_a.max(1) as f64,
            rounds_b as f64 / cand_b.max(1) as f64
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
    println!("\noverall: {}", if all_pass { "PASS" } else { "FAIL" });
}

// ----------------------------------------------------------------- cost

fn cost(spec_a: &str, spec_b: &str, slots: usize, runs: usize) {
    let (a, b) = (parse_config(spec_a), parse_config(spec_b));
    let input: Vec<Slot> = (0..slots).map(|i| build("crowded", "main", i)).collect();
    let mut per: [BTreeMap<String, Vec<f64>>; 2] = Default::default();
    for r in 0..runs.max(5) {
        let order = if r % 2 == 0 { [0, 1] } else { [1, 0] };
        for which in order {
            let setup = if which == 0 { &a } else { &b };
            let mut tot: BTreeMap<String, f64> = BTreeMap::new();
            for s in &input {
                let v = decode_slot(s, setup);
                for (k, t) in v["seconds"].as_object().unwrap() {
                    *tot.entry(k.clone()).or_default() += t.as_f64().unwrap();
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
        "median of {} interleaved runs over {slots} crowded slots, one thread (ms per slot):",
        runs.max(5)
    );
    for (k, va) in &per[0] {
        let ma = percentile(&mut va.clone(), 0.5);
        let mb = percentile(&mut per[1][k].clone(), 0.5);
        println!(
            "  {k:<20} A {:8.3}  B {:8.3}  {:+6.1}%",
            ma * 1e3,
            mb * 1e3,
            (mb - ma) / ma * 100.0
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    match arg(0) {
        Some("run") => run(
            arg(1).expect("config"),
            &PathBuf::from(arg(2).expect("outdir")),
            arg(3).unwrap_or("sweep,crowded,edges,empty"),
            arg(4).unwrap_or("main"),
        ),
        Some("compare") => compare(Path::new(arg(1).expect("old dir")), Path::new(arg(2).expect("new dir"))),
        Some("cost") => cost(
            arg(1).expect("config a"),
            arg(2).expect("config b"),
            arg(3).map_or(40, |s| s.parse().unwrap()),
            arg(4).map_or(5, |s| s.parse().unwrap()),
        ),
        _ => eprintln!("usage: paired_bench run CONFIG OUTDIR [SETS] [BLOCK] | compare OLDDIR NEWDIR | cost A B [SLOTS] [RUNS]"),
    }
}
