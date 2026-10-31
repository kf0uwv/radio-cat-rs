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

//! The OSD sizing round's measurements (`osd`, `osdcost`, `osdplaus`,
//! `osddump`): the decoder is unchanged; every failed candidate of every
//! pass is decoded again outside any timing and its soft bits and belief
//! propagation's last totals handed to the measurement-grade OSD in
//! `osd.rs`.

use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Instant;

use cat_ft8::crc;
use cat_ft8::decode::{
    BpFailure, Candidate, CandidateScratch, Decode, DecodeList, Decoder, FrontEnd, CANDIDATES,
};
use cat_ft8::ldpc::{Codeword, N};
use cat_ft8::message::{unpack, NoHashes, Payload};
use serde_json::{json, Value};

use crate::common::{fnv1a, par_map, Rng};
use crate::osd::{
    codeword_bits, distance, distance_in, get, hard_of, message_part, Bits, Generator, Osd,
};
use crate::record::{on, position};
use crate::sets::{build, set_size, Sent, Slot};
use crate::{parse_config, provenance, Setup};

/// Dn bins: bin b holds (b − 1)·STEP < Dn ≤ b·STEP, b ≤ 120; 121 above.
pub const STEP: f64 = 0.0025;
pub const DN_BINS: usize = 122;
/// dH bins 0..=80; 81 above.
pub const DH_BINS: usize = 82;

fn dn_bin(dn: f64) -> usize {
    let b = (dn / STEP - 1e-9).ceil().max(0.0);
    if b <= 120.0 {
        b as usize
    } else {
        121
    }
}

fn dh_bin(dh: u32) -> usize {
    (dh as usize).min(81)
}

fn generator() -> &'static Generator {
    static G: OnceLock<Generator> = OnceLock::new();
    G.get_or_init(Generator::new)
}

/// Whether a payload unpacks (the plausibility test).
pub fn plausible(m: u128) -> bool {
    Payload::from_bits(m >> 14).is_some_and(|p| unpack(&p, &mut NoHashes).is_ok())
}

// ------------------------------------------------------------ the pool

/// One failed candidate of one pass, with both inputs.
pub struct Failed {
    pub pass: u8,
    pub rank: usize,
    pub kind: BpFailure,
    pub at: (f32, f32),
    pub ch: [f32; N],
    pub bp: [f32; N],
}

/// A slot's passes: the full decode's list, each pass's list length and
/// new decodes, and every pass's failed candidates.
pub struct Pools {
    pub final_list: Vec<u128>,
    /// Per pass run: the list's payloads after it.
    pub after: Vec<Vec<u128>>,
    pub failed: Vec<Failed>,
    /// Per pass: candidates kept.
    pub kept: Vec<usize>,
}

fn decode_k(slot: &Slot, setup: &Setup, k: u8) -> (Vec<u128>, u8, Vec<Failed>, usize) {
    let band = setup.band;
    let config = setup.config.with_passes(k);
    let mut front = Box::new(FrontEnd::new());
    let mut levels = vec![0u8; band.spectrogram_len()];
    let mut sync_buf = vec![0i32; Decoder::sync_scratch_len(band)];
    let mut cands = vec![Candidate::default(); CANDIDATES.max(config.candidate_capacity())];
    let mut dec = Decoder::new(
        &mut front,
        &mut levels,
        &mut sync_buf,
        &mut cands,
        band,
        config,
    )
    .unwrap();
    let mut store = vec![Decode::EMPTY; setup.decodes];
    let mut list = DecodeList::new(&mut store);
    let mut scratch = Box::new(CandidateScratch::new());
    let mut audio = vec![0.0f32; slot.skip];
    audio.extend_from_slice(&slot.samples);
    let rep = dec.decode_passes(&mut audio, slot.skip, &mut scratch, &mut list);
    let listed: Vec<u128> = list
        .finish_payloads()
        .iter()
        .map(|d| d.payload.bits())
        .collect();
    let mut failed = Vec::new();
    let n = dec.candidates().len();
    for i in 0..n {
        if let Some(Err(kind)) = dec.decode_candidate(i, &mut scratch) {
            let bp = *scratch.bp_totals();
            if kind == BpFailure::Crc {
                // The accessor's check: a CRC failure's last totals are a codeword.
                let mut bits = [0u8; N];
                for (b, &v) in bits.iter_mut().zip(bp.iter()) {
                    *b = u8::from(v < 0.0);
                }
                assert!(Codeword::from_bits(&bits).satisfies_parity_checks());
            }
            failed.push(Failed {
                pass: rep.passes_run,
                rank: i,
                kind,
                at: position(dec.spectrogram(), &dec.candidates()[i]),
                ch: *scratch.soft_bits(),
                bp,
            });
        }
    }
    (listed, rep.passes_run, failed, n)
}

pub fn pools(slot: &Slot, setup: &Setup) -> Pools {
    let (final_list, run, mut failed_last, n_last) = decode_k(slot, setup, setup.config.passes);
    let mut after = Vec::new();
    let mut failed = Vec::new();
    let mut kept = Vec::new();
    for k in 1..=run {
        let (l, r, f, n) = if k == run {
            (
                final_list.clone(),
                run,
                std::mem::take(&mut failed_last),
                n_last,
            )
        } else {
            decode_k(slot, setup, k)
        };
        assert_eq!(r, k);
        assert_eq!(
            &final_list[..l.len()],
            &l[..],
            "pass {k} list is not a prefix"
        );
        after.push(l);
        failed.extend(f);
        kept.push(n);
    }
    Pools {
        final_list,
        after,
        failed,
        kept,
    }
}

// ------------------------------------------------------------ aggregates

#[derive(Clone)]
pub struct Hists {
    pub dn: Vec<u64>,
    pub dh: Vec<u64>,
}

impl Hists {
    fn new() -> Self {
        Hists {
            dn: vec![0; DN_BINS],
            dh: vec![0; DH_BINS],
        }
    }
    fn add(&mut self, dn: f64, dh: u32) {
        self.dn[dn_bin(dn)] += 1;
        self.dh[dh_bin(dh)] += 1;
    }
    fn merge(&mut self, o: &Hists) {
        for (a, b) in self.dn.iter_mut().zip(&o.dn) {
            *a += b;
        }
        for (a, b) in self.dh.iter_mut().zip(&o.dh) {
            *a += b;
        }
    }
    fn json(&self) -> Value {
        json!({"dn": self.dn, "dh": self.dh})
    }
}

/// Off-message (or empty-slot) failed candidates of one pass, rank
/// subset, input and order.
#[derive(Clone)]
pub struct Noise {
    pub n: u64,
    /// The least-D codeword's metrics (CRC ignored).
    pub best: Hists,
    /// The same, counting only a non-zero least-D codeword (form A rejects
    /// the zero one).
    pub best_nz: Hists,
    /// Every list codeword's metrics.
    pub list: Hists,
    /// Every list codeword with a non-zero message part.
    pub list_nz: Hists,
    /// Accepted (non-zero, CRC, plausible) and not transmitted: false.
    pub false_a: Hists,
    pub false_b: Hists,
    /// Accepted and transmitted (another message of the slot).
    pub other_a: u64,
    pub other_b: u64,
    /// Form A's best passing the CRC (non-zero), plausible or not.
    pub crc_a: u64,
    /// Form B found a CRC-passing codeword (any D), and it was plausible.
    pub found_b: u64,
    pub plaus_b: u64,
}

impl Noise {
    fn new() -> Self {
        Noise {
            n: 0,
            best: Hists::new(),
            best_nz: Hists::new(),
            list: Hists::new(),
            list_nz: Hists::new(),
            false_a: Hists::new(),
            false_b: Hists::new(),
            other_a: 0,
            other_b: 0,
            crc_a: 0,
            found_b: 0,
            plaus_b: 0,
        }
    }
    fn merge(&mut self, o: &Noise) {
        self.n += o.n;
        self.best.merge(&o.best);
        self.best_nz.merge(&o.best_nz);
        self.list_nz.merge(&o.list_nz);
        self.list.merge(&o.list);
        self.false_a.merge(&o.false_a);
        self.false_b.merge(&o.false_b);
        self.other_a += o.other_a;
        self.other_b += o.other_b;
        self.crc_a += o.crc_a;
        self.found_b += o.found_b;
        self.plaus_b += o.plaus_b;
    }
    fn json(&self) -> Value {
        json!({"n": self.n, "best": self.best.json(), "list": self.list.json(),
            "best_nz": self.best_nz.json(), "list_nz": self.list_nz.json(),
            "false_a": self.false_a.json(), "false_b": self.false_b.json(),
            "other_a": self.other_a, "other_b": self.other_b, "crc_a": self.crc_a,
            "found_b": self.found_b, "plaus_b": self.plaus_b})
    }
}

const PASSES: usize = 3;
const RANKS: [usize; 3] = [10, 25, usize::MAX];
/// [pass][rank subset][input][order − 1]
type NoiseAgg = Vec<Noise>;
fn noise_index(pass: usize, r: usize, input: usize, order: usize) -> usize {
    ((pass * 3 + r) * 2 + input) * 2 + order
}

#[derive(Clone, Default)]
pub struct PassStats {
    pub slots: u64,
    pub kept: u64,
    pub not_converged: u64,
    pub crc: u64,
    pub all_zero: u64,
    pub on_message: u64,
    pub new_decodes: u64,
}

pub struct Agg {
    pub slots: u64,
    pub noise: NoiseAgg,
    pub passes: Vec<PassStats>,
    pub rows: String,
    pub acc_rows: String,
    pub hash: u64,
}

impl Agg {
    fn new() -> Self {
        Agg {
            slots: 0,
            noise: vec![Noise::new(); PASSES * 3 * 2 * 2],
            passes: vec![PassStats::default(); PASSES],
            rows: String::new(),
            acc_rows: String::new(),
            hash: 0xcbf2_9ce4_8422_2325,
        }
    }
    fn merge(&mut self, o: Agg) {
        self.slots += o.slots;
        for (a, b) in self.noise.iter_mut().zip(&o.noise) {
            a.merge(b);
        }
        for (a, b) in self.passes.iter_mut().zip(&o.passes) {
            a.slots += b.slots;
            a.kept += b.kept;
            a.not_converged += b.not_converged;
            a.crc += b.crc;
            a.all_zero += b.all_zero;
            a.on_message += b.on_message;
            a.new_decodes += b.new_decodes;
        }
        self.rows.push_str(&o.rows);
        self.acc_rows.push_str(&o.acc_rows);
        // Chunks are merged in index order, so this is deterministic.
        fnv1a(&mut self.hash, &o.hash.to_le_bytes());
    }
}

/// The class of an accepted codeword: T (the candidate's own message),
/// O (another transmitted message), F (false), I (CRC passes but not
/// plausible), R (zero or CRC fails).
fn class(c: &Bits, own: Option<&Bits>, sent: &[Sent]) -> char {
    if own == Some(c) {
        return 'T';
    }
    let m = message_part(c);
    if m == 0 || !crc::check(m) {
        return 'R';
    }
    if sent.iter().any(|s| s.payload == m >> 14) {
        return 'O';
    }
    if plausible(m) {
        'F'
    } else {
        'I'
    }
}

#[allow(non_snake_case)]
fn KIND(k: BpFailure) -> char {
    match k {
        BpFailure::NotConverged => 'n',
        BpFailure::Crc => 'c',
        BpFailure::AllZero => 'z',
        _ => '?',
    }
}

/// One slot into the aggregate.
fn one_slot(agg: &mut Agg, cfg: &str, set: &str, block: &str, index: usize, setup: &Setup) {
    let slot = build(set, block, index);
    let p = pools(&slot, setup);
    agg.slots += 1;
    let gen = generator();
    let truth: Vec<Bits> = slot
        .sent
        .iter()
        .map(|s| codeword_bits(crc::append_crc(&Payload::from_bits(s.payload).unwrap())))
        .collect();
    for (k, after) in p.after.iter().enumerate() {
        let st = &mut agg.passes[k];
        st.slots += 1;
        st.kept += p.kept[k] as u64;
        let before = if k == 0 { 0 } else { p.after[k - 1].len() };
        st.new_decodes += (after.len() - before) as u64;
    }
    let mut osd = Box::new(Osd::new());
    for f in &p.failed {
        let k = usize::from(f.pass) - 1;
        let st = &mut agg.passes[k];
        match f.kind {
            BpFailure::NotConverged => st.not_converged += 1,
            BpFailure::Crc => st.crc += 1,
            _ => st.all_zero += 1,
        }
        // On a message: the nearest (by channel hard decisions) of those in range.
        let hard_ch = hard_of(&f.ch);
        let msg = slot
            .sent
            .iter()
            .enumerate()
            .filter(|(_, s)| on(s, f.at))
            .min_by_key(|(i, _)| (distance(&hard_ch, &truth[*i]), *i))
            .map(|(i, _)| i);
        if msg.is_some() {
            st.on_message += 1;
        }
        let mut row = String::new();
        if let Some(i) = msg {
            let s = &slot.sent[i];
            let hard_bp = hard_of(&f.bp);
            let _ = write!(
                row,
                "{cfg},{set},{block},{index},{},{},{},{i},{:.1},{},{},{},{}",
                f.pass,
                f.rank,
                KIND(f.kind),
                s.snr,
                u8::from(p.after[k].contains(&s.payload)),
                u8::from(p.final_list.contains(&s.payload)),
                distance(&hard_ch, &truth[i]),
                distance(&hard_bp, &truth[i]),
            );
        }
        for (input, values) in [&f.ch, &f.bp].into_iter().enumerate() {
            osd.prepare(values, gen);
            fnv1a(&mut agg.hash, &osd.total_w.to_bits().to_le_bytes());
            if let Some(i) = msg {
                let e = distance_in(osd.hard(), &truth[i], &osd.pivot_mask());
                let _ = write!(row, ",{e}");
            }
            for order in 1..=2u8 {
                let mut list = Hists::new();
                let mut list_nz = Hists::new();
                let total = f64::from(osd.total_w);
                let found = if msg.is_none() {
                    osd.search(order, true, |d, h, nz| {
                        let (dn, h) = (f64::from(d) / total, h.max(0) as u32);
                        list.add(dn, h);
                        if nz {
                            list_nz.add(dn, h);
                        }
                    })
                } else {
                    osd.search_fast(order, true)
                };
                let own = msg.map(|i| &truth[i]);
                let ca = osd.codeword(&found.a);
                let (da, ha) = osd.metrics(&ca);
                let dna = f64::from(da) / total;
                let cla = class(&ca, own, &slot.sent);
                let b = found.b.map(|pb| {
                    let cb = osd.codeword(&pb);
                    let (db, hb) = osd.metrics(&cb);
                    (class(&cb, own, &slot.sent), f64::from(db) / total, hb)
                });
                for v in [
                    u64::from(found.a.j),
                    u64::from(found.a.k),
                    u64::from(da.to_bits()),
                    found
                        .b
                        .map_or(999, |q| u64::from(q.j) << 8 | u64::from(q.k)),
                ] {
                    fnv1a(&mut agg.hash, &v.to_le_bytes());
                }
                if msg.is_some() {
                    let _ = write!(row, ",{cla},{ha},{dna:.5}");
                    match b {
                        Some((c, dn, h)) => {
                            let _ = write!(row, ",{c},{h},{dn:.5}");
                        }
                        None => row.push_str(",N,,"),
                    }
                    continue;
                }
                for (r, &lim) in RANKS.iter().enumerate() {
                    if f.rank >= lim {
                        continue;
                    }
                    let nz = &mut agg.noise[noise_index(k, r, input, usize::from(order) - 1)];
                    nz.n += 1;
                    nz.best.add(dna, ha);
                    if message_part(&ca) != 0 {
                        nz.best_nz.add(dna, ha);
                    }
                    nz.list.merge(&list);
                    nz.list_nz.merge(&list_nz);
                    match cla {
                        'F' => nz.false_a.add(dna, ha),
                        'O' => nz.other_a += 1,
                        _ => {}
                    }
                    if matches!(cla, 'F' | 'O' | 'I') {
                        nz.crc_a += 1;
                    }
                    if let Some((c, dn, h)) = b {
                        nz.found_b += 1;
                        match c {
                            'F' => {
                                nz.false_b.add(dn, h);
                                nz.plaus_b += 1;
                            }
                            'O' => {
                                nz.other_b += 1;
                                nz.plaus_b += 1;
                            }
                            _ => {}
                        }
                    }
                }
                if matches!(cla, 'F' | 'O') {
                    let m = message_part(&ca) >> 14;
                    let mt = Payload::from_bits(m).map(|p| format!("{:?}", p.message_type()));
                    let _ = writeln!(
                        agg.acc_rows,
                        "{cfg},{set},{block},{index},{},{},{},{input},{order},A,{cla},{ha},{dna:.5},{}",
                        f.pass,
                        f.rank,
                        KIND(f.kind),
                        mt.unwrap_or_default().replace(',', ";")
                    );
                }
            }
        }
        if msg.is_some() {
            row.push('\n');
            agg.rows.push_str(&row);
        }
    }
}

/// `osd CONFIG OUTDIR SETS BLOCKS`: every slot of each set and block.
pub fn osd_run(cfg: &str, out: &Path, sets: &str, blocks: &str) -> Result<(), String> {
    let setup = parse_config(cfg)?;
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    fs::write(
        out.join("config.txt"),
        format!("{cfg}\n{:?}\n{}", setup.config, provenance()),
    )
    .map_err(|e| e.to_string())?;
    for block in blocks.split(',') {
        for set in sets.split(',') {
            let t = Instant::now();
            let n = set_size(set).ok_or_else(|| format!("unknown set {set}"))?;
            let chunks = 64.min(n);
            let parts = par_map(chunks, |c| {
                let mut a = Agg::new();
                for i in (c..n).step_by(chunks) {
                    one_slot(&mut a, cfg, set, block, i, &setup);
                }
                a
            });
            let mut agg = Agg::new();
            for p in parts {
                agg.merge(p);
            }
            let tag = format!("{}-{set}-{block}", cfg.replace([',', '='], "_"));
            let mut f =
                fs::File::create(out.join(format!("{tag}.csv"))).map_err(|e| e.to_string())?;
            f.write_all(agg.rows.as_bytes())
                .map_err(|e| e.to_string())?;
            fs::write(out.join(format!("{tag}-acc.csv")), &agg.acc_rows)
                .map_err(|e| e.to_string())?;
            let noise: Vec<Value> = (0..PASSES)
                .flat_map(|k| (0..3).map(move |r| (k, r)))
                .flat_map(|(k, r)| (0..2).map(move |i| (k, r, i)))
                .flat_map(|(k, r, i)| (0..2).map(move |o| (k, r, i, o)))
                .map(|(k, r, i, o)| {
                    let mut v = agg.noise[noise_index(k, r, i, o)].json();
                    v["pass"] = json!(k + 1);
                    v["rank"] = json!(if RANKS[r] == usize::MAX { 0 } else { RANKS[r] });
                    v["input"] = json!(["ch", "bp"][i]);
                    v["order"] = json!(o + 1);
                    v
                })
                .collect();
            let passes: Vec<Value> = agg
                .passes
                .iter()
                .enumerate()
                .map(|(k, p)| {
                    json!({"pass": k + 1, "slots": p.slots, "kept": p.kept,
                        "not_converged": p.not_converged, "crc": p.crc, "all_zero": p.all_zero,
                        "on_message": p.on_message, "new_decodes": p.new_decodes})
                })
                .collect();
            let v = json!({"config": cfg, "set": set, "block": block, "slots": agg.slots,
                "hash": format!("{:016x}", agg.hash), "passes": passes, "noise": noise});
            fs::write(out.join(format!("{tag}.json")), v.to_string()).map_err(|e| e.to_string())?;
            eprintln!(
                "{tag}: {n} slots in {:.1} s, hash {:016x}",
                t.elapsed().as_secs_f64(),
                agg.hash
            );
        }
    }
    Ok(())
}

/// `osdplaus N`: the plausibility rate of N uniformly random 77-bit
/// payloads (fixed seed), by message type.
pub fn osd_plaus(n: usize) -> Result<(), String> {
    let mut r = Rng(0x05d0_91a7_51ab_1e00);
    let mut ok = 0u64;
    let mut by: std::collections::BTreeMap<String, (u64, u64)> = Default::default();
    for _ in 0..n {
        let hi = (r.uniform() * (1u64 << 40) as f64) as u128;
        let lo = (r.uniform() * (1u64 << 40) as f64) as u128;
        let m = ((hi << 40) | lo) & ((1u128 << 77) - 1);
        let p = Payload::from_bits(m).unwrap();
        let good = unpack(&p, &mut NoHashes).is_ok();
        ok += u64::from(good);
        let e = by.entry(format!("{:?}", p.message_type())).or_default();
        e.0 += 1;
        e.1 += u64::from(good);
    }
    println!("plausible {ok} of {n} = {:.6}", ok as f64 / n as f64);
    for (t, (a, b)) in by {
        println!("  {t}: {b} of {a}");
    }
    Ok(())
}

// ------------------------------------------------------------ cost

/// The failed candidates of host's passes on the slotcost slots.
fn cost_inputs(setup: &Setup, slots: usize) -> Vec<(u8, [f32; N])> {
    let per: Vec<Vec<(u8, [f32; N])>> = par_map(slots, |k| {
        let slot = build("crowded", "main", 400 + k);
        pools(&slot, setup)
            .failed
            .into_iter()
            .map(|f| (f.pass, f.ch))
            .collect()
    });
    per.into_iter().flatten().collect()
}

/// `osdcost CONFIG [SLOTS] [RUNS]`: one thread, per failed candidate
/// (channel soft bits) of every pass on the slotcost slots: preparation
/// (sort and elimination), then preparation with OSD-1 and OSD-2 in forms
/// A and B. Prints the median µs per candidate over RUNS and a hash of
/// the results.
pub fn osd_cost(cfg: &str, slots: usize, runs: usize) -> Result<(), String> {
    let setup = parse_config(cfg)?;
    let slots = slots.clamp(1, 200);
    let input = cost_inputs(&setup, slots);
    let gen = generator();
    let mut by_pass = [0usize; 4];
    for (p, _) in &input {
        by_pass[usize::from(*p)] += 1;
    }
    println!(
        "{cfg}: {slots} slots, {} failed candidates (pass 1 {}, 2 {}, 3 {})",
        input.len(),
        by_pass[1],
        by_pass[2],
        by_pass[3]
    );
    let mut osd = Box::new(Osd::new());
    let modes: [(&str, Option<(u8, bool)>); 5] = [
        ("prepare", None),
        ("osd1 A", Some((1, false))),
        ("osd1 B", Some((1, true))),
        ("osd2 A", Some((2, false))),
        ("osd2 B", Some((2, true))),
    ];
    let runs = runs.max(5);
    let mut times = vec![Vec::new(); modes.len()];
    let mut hashes = vec![0u64; modes.len()];
    for r in 0..runs {
        for (m, (_, mode)) in modes.iter().enumerate() {
            let mut h = 0xcbf2_9ce4_8422_2325u64;
            let t = Instant::now();
            for (_, v) in &input {
                osd.prepare(std::hint::black_box(v), gen);
                if let Some((order, b)) = *mode {
                    let f = osd.search_fast(order, b);
                    fnv1a(&mut h, &[f.a.j, f.a.k]);
                    fnv1a(&mut h, &f.a.d.to_bits().to_le_bytes());
                    if let Some(q) = f.b {
                        fnv1a(&mut h, &[q.j, q.k]);
                    }
                } else {
                    fnv1a(&mut h, &osd.total_w.to_bits().to_le_bytes());
                }
            }
            times[m].push(t.elapsed().as_secs_f64() * 1e6 / input.len() as f64);
            if r == 0 {
                hashes[m] = h;
            } else if hashes[m] != h {
                return Err(format!("{} results differ between runs", modes[m].0));
            }
        }
    }
    for (m, (name, _)) in modes.iter().enumerate() {
        let mut t = times[m].clone();
        t.sort_by(f64::total_cmp);
        println!(
            "{name:8} median {:.3} us/candidate (min {:.3}, max {:.3}) hash {:016x}",
            t[t.len() / 2],
            t[0],
            t[t.len() - 1],
            hashes[m]
        );
    }
    Ok(())
}

/// `osddump CONFIG FILE`: every 50th pass-1 failed candidate's soft bits
/// on the slotcost slots (16 of them), as `f32` little-endian bits, and the
/// x86-64 results of OSD-1 and OSD-2 (forms A and B) for each, as text.
pub fn osd_dump(cfg: &str, file: &Path) -> Result<(), String> {
    let setup = parse_config(cfg)?;
    let input = cost_inputs(&setup, 200);
    let picked: Vec<&[f32; N]> = input
        .iter()
        .filter(|(p, _)| *p == 1)
        .map(|(_, v)| v)
        .step_by(50)
        .take(16)
        .collect();
    let mut bytes = Vec::new();
    for v in &picked {
        for x in v.iter() {
            bytes.extend_from_slice(&x.to_bits().to_le_bytes());
        }
    }
    fs::write(file, &bytes).map_err(|e| e.to_string())?;
    let gen = generator();
    let mut osd = Box::new(Osd::new());
    for (i, v) in picked.iter().enumerate() {
        osd.prepare(v, gen);
        let mut s = format!("{i}");
        for order in 1..=2 {
            let f = osd.search_fast(order, true);
            let _ = write!(
                s,
                " o{order} A {} {} {:08x} B {:?}",
                f.a.j,
                f.a.k,
                f.a.d.to_bits(),
                f.b.map(|q| (q.j, q.k, format!("{:08x}", q.d.to_bits())))
            );
        }
        println!("{s}");
    }
    Ok(())
}

/// `osdcheck`: the OSD's own checks on seeded inputs: every reduced row is
/// a codeword with a 1 at its pivot only among the pivots; the summed D
/// of every listed codeword matches the direct sum (to 10⁻⁴ relative) and
/// its Hamming distance exactly; order i finds the true codeword whenever
/// the basis holds at most i errors and no listed codeword is closer.
pub fn osd_check(trials: usize) -> Result<(), String> {
    let gen = generator();
    let mut osd = Box::new(Osd::new());
    let mut r = Rng(0x05d0_c4ec_0000_0001);
    let mut found_truth = [0u64; 3];
    let mut reach = [0u64; 3];
    for t in 0..trials {
        let a91 = crc::append_crc(
            &Payload::from_bits(((r.uniform() * 2f64.powi(52)) as u128) << 25 | t as u128).unwrap(),
        );
        let truth = codeword_bits(a91);
        let sigma = 0.4 + 0.8 * r.uniform();
        let mut v = [0.0f32; N];
        for (i, x) in v.iter_mut().enumerate() {
            let s = if get(&truth, i) { -1.0 } else { 1.0 };
            *x = (s + sigma * r.gauss()) as f32;
        }
        osd.prepare(&v, gen);
        let piv = osd.pivot_mask();
        if piv.iter().map(|w| w.count_ones()).sum::<u32>() != 91 {
            return Err("basis not 91 positions".into());
        }
        let e = distance_in(osd.hard(), &truth, &piv);
        for order in 0..=2u8 {
            let mut bad = None;
            let f = osd.search(order, true, |d, h, _| {
                if d < -1e-3 || h < 0 {
                    bad = Some((d, h));
                }
            });
            if let Some(b) = bad {
                return Err(format!("negative metric {b:?}"));
            }
            let c = osd.codeword(&f.a);
            let mut bits = [0u8; N];
            for (i, b) in bits.iter_mut().enumerate() {
                *b = u8::from(get(&c, i));
            }
            if !Codeword::from_bits(&bits).satisfies_parity_checks() {
                return Err("chosen word is not a codeword".into());
            }
            let (d, _) = osd.metrics(&c);
            if (d - f.a.d).abs() > 1e-4 * osd.total_w.max(1.0) {
                return Err(format!("D {} vs direct {d}", f.a.d));
            }
            if let Some(b) = f.b {
                let cb = osd.codeword(&b);
                let m = message_part(&cb);
                if m == 0 || !crc::check(m) || b.d + 1e-4 * osd.total_w < f.a.d {
                    return Err("form B pick wrong".into());
                }
            }
            if e <= u32::from(order) {
                reach[usize::from(order)] += 1;
                let (dt, _) = osd.metrics(&truth);
                if c == truth {
                    found_truth[usize::from(order)] += 1;
                } else if d > dt + 1e-4 * osd.total_w {
                    return Err(format!(
                        "order {order}, e {e}: chose D {d} over the truth's {dt}"
                    ));
                }
                // Form B must then be at most the truth's D too.
                if f.b.map_or(true, |b| b.d > dt + 1e-4 * osd.total_w) {
                    return Err("form B missed the truth in reach".into());
                }
            }
        }
    }
    println!(
        "osdcheck: {trials} trials pass; in reach (e ≤ i) per order {reach:?}, chose the truth {found_truth:?}"
    );
    Ok(())
}
