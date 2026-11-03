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

//! `osdcal CONFIG RANKS OUTFILE PREFIX FROM TO`: the false-decode
//! calibration of OSD-1 on empty slots, counted rather than estimated.
//! The `empty` set's blocks `PREFIX` followed by FROM..TO (four digits)
//! are decoded with CONFIG, the decoder's OSD always forced off (whatever
//! CONFIG says: the tool measures OSD-1 outside the decoder); after every pass, the
//! decoder's OSD-1 runs on the soft bits of each failed candidate of rank
//! below each subset of RANKS (a comma list, e.g. `10` or `10,25`), and
//! its least-D codeword is counted by Hamming distance dH from the hard
//! decisions: non-zero, CRC passing, and CRC passing and plausible (every
//! such codeword on an empty slot is a false decode). For the non-zero
//! codewords at dH ≤ 30 it also counts the message types and whether the
//! payload is plausible (CRC ignored), the CRC syndrome's bits, and the
//! syndrome's distribution. Belief propagation's own CRC failures and
//! decodes per slot are counted too (the current rate). Aggregates only,
//! one JSON object to OUTFILE, plus one line per accepted codeword.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::time::Instant;

use cat_ft8::crc;
use cat_ft8::decode::osd::{order1, OsdScratch};
use cat_ft8::decode::{
    Candidate, CandidateScratch, Decode, DecodeList, Decoder, FrontEnd, CANDIDATES,
};
use cat_ft8::ldpc::N;
use cat_ft8::message::{unpack, MessageType, NoHashes, Payload};
use serde_json::json;

use crate::common::par_map;
use crate::sets::build;
use crate::{parse_config, provenance, Setup};

/// dH bins 0..=80; 81 above.
const DH: usize = 82;
/// The detailed tail: dH ≤ 30.
const TAIL: usize = 31;
/// Empty-slot kinds, in the set's index order.
const KINDS: [&str; 6] = [
    "noise",
    "carriers",
    "strong",
    "clipped",
    "floored",
    "impulsive",
];
/// Message types counted in the tail.
const TYPES: [&str; 10] = [
    "FreeText",
    "DXpedition",
    "FieldDay",
    "Telemetry",
    "Standard",
    "EuVhfPortable",
    "RttyRoundup",
    "NonStandardCall",
    "EuVhfContest",
    "Reserved",
];

fn type_index(t: MessageType) -> usize {
    match t {
        MessageType::FreeText => 0,
        MessageType::DXpedition => 1,
        MessageType::FieldDay { .. } => 2,
        MessageType::Telemetry => 3,
        MessageType::Standard => 4,
        MessageType::EuVhfPortable => 5,
        MessageType::RttyRoundup => 6,
        MessageType::NonStandardCall => 7,
        MessageType::EuVhfContest => 8,
        MessageType::Reserved { .. } => 9,
    }
}

/// One rank subset's counts.
#[derive(Clone)]
struct Subset {
    runs: u64,
    /// Per pass (1..=3).
    runs_pass: [u64; 3],
    zero: u64,
    nz: Vec<u64>,
    crc: Vec<u64>,
    accepted: Vec<u64>,
    /// [kind][dH ≤ 30]
    crc_kind: Vec<[u64; TAIL]>,
    accepted_kind: Vec<[u64; TAIL]>,
    /// [dH ≤ 30][type]: non-zero codewords, and those with a plausible
    /// payload (CRC ignored).
    types: Vec<[u64; TYPES.len()]>,
    types_plausible: Vec<[u64; TYPES.len()]>,
    /// [dH ≤ 30][bit]: syndrome bits set.
    syn_bits: Vec<[u64; 14]>,
    /// Syndrome values at dH ≤ 24 and at 25..=30.
    syn_lo: Vec<u64>,
    syn_hi: Vec<u64>,
    /// One line per accepted codeword.
    events: String,
}

impl Subset {
    fn new() -> Self {
        Subset {
            runs: 0,
            runs_pass: [0; 3],
            zero: 0,
            nz: vec![0; DH],
            crc: vec![0; DH],
            accepted: vec![0; DH],
            crc_kind: vec![[0; TAIL]; KINDS.len()],
            accepted_kind: vec![[0; TAIL]; KINDS.len()],
            types: vec![[0; TYPES.len()]; TAIL],
            types_plausible: vec![[0; TYPES.len()]; TAIL],
            syn_bits: vec![[0; 14]; TAIL],
            syn_lo: vec![0; 1 << 14],
            syn_hi: vec![0; 1 << 14],
            events: String::new(),
        }
    }

    fn merge(&mut self, o: &Subset) {
        self.runs += o.runs;
        for (a, b) in self.runs_pass.iter_mut().zip(&o.runs_pass) {
            *a += b;
        }
        self.zero += o.zero;
        let add = |a: &mut [u64], b: &[u64]| a.iter_mut().zip(b).for_each(|(x, y)| *x += y);
        add(&mut self.nz, &o.nz);
        add(&mut self.crc, &o.crc);
        add(&mut self.accepted, &o.accepted);
        add(&mut self.syn_lo, &o.syn_lo);
        add(&mut self.syn_hi, &o.syn_hi);
        for (a, b) in self.crc_kind.iter_mut().zip(&o.crc_kind) {
            add(a, b);
        }
        for (a, b) in self.accepted_kind.iter_mut().zip(&o.accepted_kind) {
            add(a, b);
        }
        for (a, b) in self.types.iter_mut().zip(&o.types) {
            add(a, b);
        }
        for (a, b) in self.types_plausible.iter_mut().zip(&o.types_plausible) {
            add(a, b);
        }
        for (a, b) in self.syn_bits.iter_mut().zip(&o.syn_bits) {
            add(a, b);
        }
        self.events.push_str(&o.events);
    }
}

#[derive(Clone)]
struct Cal {
    slots: u64,
    slots_kind: [u64; KINDS.len()],
    /// Belief propagation over every pass: CRC failures and decodes (all
    /// false on an empty slot), and slots that ran more than one pass.
    bp_crc: u64,
    bp_decodes: u64,
    multi_pass: u64,
    subsets: Vec<Subset>,
}

impl Cal {
    fn new(n: usize) -> Self {
        Cal {
            slots: 0,
            slots_kind: [0; KINDS.len()],
            bp_crc: 0,
            bp_decodes: 0,
            multi_pass: 0,
            subsets: vec![Subset::new(); n],
        }
    }

    fn merge(&mut self, o: &Cal) {
        self.slots += o.slots;
        for (a, b) in self.slots_kind.iter_mut().zip(&o.slots_kind) {
            *a += b;
        }
        self.bp_crc += o.bp_crc;
        self.bp_decodes += o.bp_decodes;
        self.multi_pass += o.multi_pass;
        for (a, b) in self.subsets.iter_mut().zip(&o.subsets) {
            a.merge(b);
        }
    }
}

/// Passes run, the list's payloads, CRC failures over every pass, and
/// (rank, soft bits) of the last pass's failed candidates of low rank.
type Top = (u8, Vec<u128>, usize, Vec<(usize, [f32; N])>);

/// Decodes `samples` with `passes` passes; returns the passes run, the
/// list, the CRC failures over every pass, and the soft bits of the last
/// pass's failed candidates of rank below `limit`.
fn decode_top(samples: &[f32], skip: usize, setup: &Setup, passes: u8, limit: usize) -> Top {
    let band = setup.band;
    // The decoder's OSD off whatever the setup says (see `osd_cal`).
    let config = setup.config.with_passes(passes).with_osd(None);
    let mut front = Box::new(FrontEnd::new());
    let mut levels = vec![0u8; band.spectrogram_len()];
    let mut sync_buf = vec![0.0f32; Decoder::sync_scratch_len(band)];
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
    let mut audio = vec![0.0f32; skip];
    audio.extend_from_slice(samples);
    let rep = dec.decode_passes(&mut audio, skip, &mut scratch, &mut list);
    let listed = list
        .finish_payloads()
        .iter()
        .map(|d| d.payload.bits())
        .collect();
    let mut failed = Vec::new();
    for i in 0..dec.candidates().len().min(limit) {
        if let Some(Err(_)) = dec.decode_candidate(i, &mut scratch) {
            failed.push((i, *scratch.soft_bits()));
        }
    }
    (rep.passes_run, listed, rep.report.crc_failed, failed)
}

/// Whether a payload unpacks with no call hashes.
fn plausible(p: &Payload) -> bool {
    unpack(p, &mut NoHashes).is_ok()
}

fn one_slot(
    cal: &mut Cal,
    ranks: &[usize],
    block: &str,
    index: usize,
    setup: &Setup,
    osd: &mut OsdScratch,
) {
    let slot = build("empty", block, index);
    let kind = KINDS
        .iter()
        .position(|k| *k == slot.point)
        .expect("an empty-slot kind");
    cal.slots += 1;
    cal.slots_kind[kind] += 1;
    let limit = *ranks.iter().max().unwrap();
    let (run, listed, crc_failed, last) =
        decode_top(&slot.samples, slot.skip, setup, setup.config.passes, limit);
    cal.bp_crc += crc_failed as u64;
    cal.bp_decodes += listed.len() as u64;
    let mut per_pass = Vec::new();
    if run > 1 {
        cal.multi_pass += 1;
        for k in 1..run {
            per_pass.push((k, decode_top(&slot.samples, slot.skip, setup, k, limit).3));
        }
    }
    per_pass.push((run, last));
    for (pass, failed) in per_pass {
        for (rank, v) in failed {
            let best = order1(&v, osd);
            let dh = (best.distance as usize).min(DH - 1);
            for (s, &r) in cal.subsets.iter_mut().zip(ranks) {
                if rank >= r {
                    continue;
                }
                s.runs += 1;
                s.runs_pass[usize::from(pass) - 1] += 1;
                if best.a91 == 0 {
                    s.zero += 1;
                    continue;
                }
                s.nz[dh] += 1;
                let payload = Payload::from_bits(best.a91 >> 14).expect("77 bits");
                let crc_ok = crc::check(best.a91);
                let ok = crc_ok && plausible(&payload);
                if crc_ok {
                    s.crc[dh] += 1;
                }
                if ok {
                    s.accepted[dh] += 1;
                    let text = unpack(&payload, &mut NoHashes)
                        .map(|t| t.as_str().to_string())
                        .unwrap_or_default();
                    let dup = listed.contains(&payload.bits());
                    let _ = writeln!(
                        s.events,
                        "{block},{index},{},{pass},{rank},{dh},{:020x},{:?},{text:?},{dup}",
                        KINDS[kind],
                        payload.bits(),
                        payload.message_type()
                    );
                }
                if dh < TAIL {
                    if crc_ok {
                        s.crc_kind[kind][dh] += 1;
                    }
                    if ok {
                        s.accepted_kind[kind][dh] += 1;
                    }
                    let t = type_index(payload.message_type());
                    s.types[dh][t] += 1;
                    s.types_plausible[dh][t] += u64::from(plausible(&payload));
                    let syn = crc::crc14(&payload) ^ (best.a91 & 0x3fff) as u16;
                    for (b, c) in s.syn_bits[dh].iter_mut().enumerate() {
                        *c += u64::from((syn >> b) & 1);
                    }
                    if dh <= 24 {
                        s.syn_lo[usize::from(syn)] += 1;
                    } else {
                        s.syn_hi[usize::from(syn)] += 1;
                    }
                }
            }
        }
    }
}

pub fn osd_cal(
    cfg: &str,
    ranks: &str,
    out: &Path,
    prefix: &str,
    from: usize,
    to: usize,
) -> Result<(), String> {
    let mut setup = parse_config(cfg)?;
    // The decoder's own OSD stays off: this tool runs OSD-1 itself.
    setup.config = setup.config.with_osd(None);
    let ranks: Vec<usize> = ranks
        .split(',')
        .map(|r| r.parse().map_err(|e| format!("ranks: {e}")))
        .collect::<Result<_, String>>()?;
    if ranks.is_empty() || from > to || to > 9_999 {
        return Err("RANKS a comma list; FROM ≤ TO ≤ 9999".into());
    }
    let blocks: Vec<String> = (from..=to).map(|b| format!("{prefix}{b:04}")).collect();
    let per = crate::sets::set_size("empty").unwrap();
    let total = blocks.len() * per;
    let t = Instant::now();
    let chunks = 512.min(total);
    let parts = par_map(chunks, |c| {
        let mut cal = Cal::new(ranks.len());
        let mut osd = Box::new(OsdScratch::new());
        for i in (c..total).step_by(chunks) {
            one_slot(
                &mut cal,
                &ranks,
                &blocks[i / per],
                i % per,
                &setup,
                &mut osd,
            );
        }
        cal
    });
    let mut cal = Cal::new(ranks.len());
    for p in &parts {
        cal.merge(p);
    }
    let subsets: Vec<_> = cal
        .subsets
        .iter()
        .zip(&ranks)
        .map(|(s, r)| {
            json!({"ranks": r, "runs": s.runs, "runs_pass": s.runs_pass, "zero": s.zero,
                "nz": s.nz, "crc": s.crc, "accepted": s.accepted,
                "crc_kind": s.crc_kind, "accepted_kind": s.accepted_kind,
                "types": s.types, "types_plausible": s.types_plausible,
                "syn_bits": s.syn_bits, "syn_lo": s.syn_lo, "syn_hi": s.syn_hi,
                "events": s.events})
        })
        .collect();
    let v = json!({"config": cfg, "prefix": prefix, "from": from, "to": to,
        "slots": cal.slots, "slots_kind": cal.slots_kind, "kinds": KINDS, "types": TYPES,
        "bp_crc": cal.bp_crc, "bp_decodes": cal.bp_decodes, "multi_pass": cal.multi_pass,
        "subsets": subsets, "provenance": provenance(),
        "seconds": t.elapsed().as_secs_f64()});
    fs::write(out, v.to_string()).map_err(|e| e.to_string())?;
    let s0 = &cal.subsets[0];
    eprintln!(
        "{cfg} {prefix}{from:04}-{to:04}: {} slots in {:.0} s; BP CRC {} decodes {}; OSD runs {} accepted at dH ≤ 22/23/24: {}/{}/{}",
        cal.slots,
        t.elapsed().as_secs_f64(),
        cal.bp_crc,
        cal.bp_decodes,
        s0.runs,
        s0.accepted[..=22].iter().sum::<u64>(),
        s0.accepted[..=23].iter().sum::<u64>(),
        s0.accepted[..=24].iter().sum::<u64>()
    );
    Ok(())
}
