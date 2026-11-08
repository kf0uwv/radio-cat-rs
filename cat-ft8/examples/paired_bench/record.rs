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

//! Decoding a slot, classifying what became of each transmitted message,
//! and the per-slot record `run` writes and `compare` reads.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::ops::ControlFlow;
use std::path::Path;
use std::time::Instant;

use cat_ft8::decode::{
    sync, BpFailure, Candidate, CandidateScratch, Decode, DecodeList, Decoder, FrontEnd, PassEvent,
    PassStep, Spectrogram, Stage, BIN_HZ, CANDIDATES,
};
use serde_json::Value;

use crate::channel::Channel;
use crate::common::par_map;
use crate::sets::{build, set_size, slot_seed, Sent, Slot};
use crate::{parse_config, provenance, Setup};

/// One decode as saved.
#[derive(Clone, PartialEq)]
pub struct DecodeRecord {
    pub payload: u128,
    pub freq: f32,
    pub dt: f32,
    pub snr: f32,
    pub score: i32,
    pub candidate: u16,
    pub hard_errors: u8,
    pub rounds: u8,
    /// Decoded by OSD-1 after belief propagation failed (the candidate's
    /// `Stage::Osd` reported, joined on pass and candidate): saved as
    /// `"osd":true`, and left out otherwise, so records without the
    /// option are as before.
    pub osd: bool,
}

/// The report counts saved per slot.
#[derive(Clone, Copy, Default)]
pub struct Counts {
    pub found: u64,
    pub kept: u64,
    pub dropped: u64,
    pub decoded: u64,
    pub truncated: u64,
    pub duplicates: u64,
    pub all_zero: u64,
    pub crc_failed: u64,
    pub not_converged: u64,
    pub rows_filled: u64,
}

/// One pass's counts, saved for multi-pass runs only.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct PassRecord {
    pub pass: u64,
    /// The decode list's length when the pass began (its first new entry).
    pub first: u64,
    pub subtracted: u64,
    pub found: u64,
    pub kept: u64,
    pub dropped: u64,
    /// New entries plus truncated.
    pub decoded: u64,
    pub truncated: u64,
    pub duplicates: u64,
    /// Duplicates of a payload an earlier pass listed.
    pub earlier: u64,
    pub all_zero: u64,
    pub crc_failed: u64,
    pub not_converged: u64,
    pub bp_rounds: u64,
}

impl PassRecord {
    const KEYS: [&'static str; 14] = [
        "pass",
        "first",
        "subtracted",
        "candidates_found",
        "candidates_kept",
        "candidates_dropped",
        "decoded",
        "decodes_truncated",
        "duplicates",
        "earlier_pass_duplicates",
        "all_zero_rejected",
        "crc_failed",
        "not_converged",
        "bp_rounds",
    ];

    fn values(&self) -> [u64; 14] {
        [
            self.pass,
            self.first,
            self.subtracted,
            self.found,
            self.kept,
            self.dropped,
            self.decoded,
            self.truncated,
            self.duplicates,
            self.earlier,
            self.all_zero,
            self.crc_failed,
            self.not_converged,
            self.bp_rounds,
        ]
    }

    fn from_values(v: [u64; 14]) -> Self {
        PassRecord {
            pass: v[0],
            first: v[1],
            subtracted: v[2],
            found: v[3],
            kept: v[4],
            dropped: v[5],
            decoded: v[6],
            truncated: v[7],
            duplicates: v[8],
            earlier: v[9],
            all_zero: v[10],
            crc_failed: v[11],
            not_converged: v[12],
            bp_rounds: v[13],
        }
    }
}

/// One slot's result: the record `run` writes and `compare` reads.
pub struct SlotResult {
    pub set: String,
    pub block: String,
    pub index: usize,
    pub seed: u64,
    pub point: String,
    pub sample_hash: u64,
    pub sent: Vec<Sent>,
    pub decodes: Vec<DecodeRecord>,
    pub counts: Counts,
    pub bp_rounds: u64,
    /// (stage, seconds): feed, sync, then every `Stage`.
    pub seconds: Vec<(String, f64)>,
    /// Per transmitted message; `None` in runs recorded before outcomes
    /// were saved (so an old run of a set without messages is told apart
    /// from a new one).
    pub outcomes: Option<Vec<Outcome>>,
    /// Per pass, for multi-pass configurations only (single-pass records
    /// are unchanged).
    pub passes: Option<Vec<PassRecord>>,
    /// Candidates OSD-1 ran on (`"osd_runs"`, saved only when not 0, so
    /// records without the option are as before).
    pub osd_runs: u64,
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
    pub fn to_json(&self) -> String {
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
                "{}{{\"payload\":\"{:020x}\",\"text\":{},\"freq\":{:?},\"dt\":{:?},\"snr\":{:?}",
                if i > 0 { "," } else { "" }, s.payload, json_str(&s.text), s.freq, s.dt, s.snr
            );
            // Only the `fading` set has a channel, so other sets' records
            // are unchanged.
            if let Some(c) = &s.channel {
                let _ = write!(
                    o,
                    ",\"channel\":{{\"fade_hz\":{:?},\"fade_depth\":{:?},\"fade_phase\":{:?},\"drift\":{:?}}}",
                    c.fade_hz, c.fade_depth, c.fade_phase, c.drift
                );
            }
            o.push('}');
        }
        o.push_str("],\"decodes\":[");
        for (i, d) in self.decodes.iter().enumerate() {
            let _ = write!(
                o,
                "{}{{\"payload\":\"{:020x}\",\"freq\":{:?},\"dt\":{:?},\"snr\":{:?},\"score\":{},\"candidate\":{},\"hard_errors\":{},\"rounds\":{}}}",
                if i > 0 { "," } else { "" }, d.payload, d.freq, d.dt, d.snr, d.score, d.candidate, d.hard_errors, d.rounds
            );
            if d.osd {
                o.truncate(o.len() - 1);
                o.push_str(",\"osd\":true}");
            }
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
        o.push('}');
        if let Some(outcomes) = &self.outcomes {
            o.push_str(",\"outcomes\":[");
            for (i, x) in outcomes.iter().enumerate() {
                let _ = write!(o, "{}\"{}\"", if i > 0 { "," } else { "" }, x.code());
            }
            o.push(']');
        }
        if let Some(passes) = &self.passes {
            o.push_str(",\"passes\":[");
            for (i, p) in passes.iter().enumerate() {
                o.push_str(if i > 0 { ",{" } else { "{" });
                for (j, (k, v)) in PassRecord::KEYS.iter().zip(p.values()).enumerate() {
                    let _ = write!(o, "{}\"{k}\":{v}", if j > 0 { "," } else { "" });
                }
                o.push('}');
            }
            o.push(']');
        }
        if self.osd_runs > 0 {
            let _ = write!(o, ",\"osd_runs\":{}", self.osd_runs);
        }
        o.push('}');
        o
    }

    /// Reads a line written by [`SlotResult::to_json`].
    pub fn from_json(v: &Value) -> Option<SlotResult> {
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
                        channel: match s.get("channel") {
                            None => None,
                            Some(c) => Some(Channel {
                                fade_hz: c["fade_hz"].as_f64()?,
                                fade_depth: c["fade_depth"].as_f64()?,
                                fade_phase: c["fade_phase"].as_f64()?,
                                drift: c["drift"].as_f64()?,
                            }),
                        },
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
                        osd: d.get("osd").and_then(Value::as_bool).unwrap_or(false),
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
                None => None,
                Some(x) => Some(
                    x.as_array()?
                        .iter()
                        .map(|o| Outcome::from_code(o.as_str()?))
                        .collect::<Option<_>>()?,
                ),
            },
            osd_runs: v.get("osd_runs").and_then(Value::as_u64).unwrap_or(0),
            passes: match v.get("passes") {
                None => None,
                Some(x) => Some(
                    x.as_array()?
                        .iter()
                        .map(|p| {
                            let mut v = [0u64; 14];
                            for (k, slot) in PassRecord::KEYS.iter().zip(v.iter_mut()) {
                                *slot = u(&p[*k])?;
                            }
                            Some(PassRecord::from_values(v))
                        })
                        .collect::<Option<_>>()?,
                ),
            },
        })
    }
}

// -------------------------------------------------------------- outcome

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
/// failing the CRC, then the all-zero codeword, then another failure, then
/// no codeword within the round limit. With no candidate on it, the sync
/// search is run again without the candidate limit: a sync peak on the
/// message that the limit dropped makes it [`Outcome::Dropped`], none
/// [`Outcome::NoPeak`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Outcome {
    /// No sync peak on the message (at the minimum score).
    NoPeak,
    /// A sync peak on the message, dropped by the candidate limit.
    Dropped,
    /// No candidate on the message: runs recorded before the two above
    /// were told apart (read only, never produced).
    NoCandidate,
    NotConverged,
    /// A belief-propagation failure of a kind this bench does not know.
    OtherFailure,
    AllZero,
    Crc,
    Decoded,
}

impl Outcome {
    /// The reasons a message is missed, in report order.
    pub const MISSES: [Outcome; 7] = [
        Outcome::NoPeak,
        Outcome::Dropped,
        Outcome::NoCandidate,
        Outcome::NotConverged,
        Outcome::Crc,
        Outcome::AllZero,
        Outcome::OtherFailure,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Outcome::Decoded => "decoded",
            Outcome::NoPeak => "no_peak",
            Outcome::Dropped => "dropped",
            Outcome::NoCandidate => "no_candidate",
            Outcome::NotConverged => "not_converged",
            Outcome::OtherFailure => "other_failure",
            Outcome::Crc => "crc",
            Outcome::AllZero => "all_zero",
        }
    }

    /// The words for a miss in a report line.
    pub fn words(self) -> &'static str {
        match self {
            Outcome::Decoded => "decoded",
            Outcome::NoPeak => "no sync peak",
            Outcome::Dropped => "peak dropped by the candidate limit",
            Outcome::NoCandidate => "no candidate (older run, not split)",
            Outcome::NotConverged => "not converged",
            Outcome::OtherFailure => "other failure",
            Outcome::Crc => "CRC failure",
            Outcome::AllZero => "all-zero",
        }
    }

    pub fn from_code(s: &str) -> Option<Outcome> {
        Outcome::MISSES
            .into_iter()
            .chain([Outcome::Decoded])
            .find(|o| o.code() == s)
    }

    /// The outcome a failed candidate gives a message it is on.
    fn of_failure(f: BpFailure) -> Outcome {
        match f {
            BpFailure::NotConverged => Outcome::NotConverged,
            BpFailure::Crc => Outcome::Crc,
            BpFailure::AllZero => Outcome::AllZero,
            _ => Outcome::OtherFailure,
        }
    }
}

/// Counts of missed messages by [`Outcome::MISSES`].
pub fn miss_counts<'a>(outcomes: impl IntoIterator<Item = &'a Outcome>) -> [u64; 7] {
    let mut n = [0u64; 7];
    for o in outcomes {
        if let Some(k) = Outcome::MISSES.iter().position(|m| m == o) {
            n[k] += 1;
        }
    }
    n
}

/// "no sync peak 3, peak dropped by the candidate limit 1, …": every miss
/// cause, except that the cause of older runs and unknown failures are
/// left out when there are none.
pub fn misses_text(n: &[u64; 7]) -> String {
    Outcome::MISSES
        .iter()
        .zip(n)
        .filter(|(o, &k)| k > 0 || !matches!(o, Outcome::NoCandidate | Outcome::OtherFailure))
        .map(|(o, k)| format!("{} {k}", o.words()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A candidate's refined tone 0 (Hz) and start (s), as `decode_candidate`
/// computes them.
pub fn position(spec: &Spectrogram<'_>, c: &Candidate) -> (f32, f32) {
    let (frac_row, frac_bin) = sync::refine(spec, c);
    let freq = ((spec.band().first_bin() + usize::from(c.base)) as f32 + frac_bin) * BIN_HZ;
    (freq, c.dt() + frac_row * sync::ROW_SECONDS)
}

/// Whether a candidate at `(freq, dt)` is on message `s`.
pub fn on(s: &Sent, (freq, dt): (f32, f32)) -> bool {
    (s.freq as f32 - freq).abs() <= ON_HZ && (s.dt as f32 - dt).abs() <= ON_S
}

// --------------------------------------------------------------- decode

/// One single-pass decode of some input.
pub struct Decoded {
    pub decodes: Vec<DecodeRecord>,
    pub counts: Counts,
    pub bp_rounds: u64,
    /// (stage, seconds): feed, sync, then every `Stage`.
    pub seconds: Vec<(String, f64)>,
    /// Per transmitted message, in order.
    pub outcomes: Vec<Outcome>,
    /// Per pass (multi-pass configurations only).
    pub passes: Option<Vec<PassRecord>>,
    /// Candidates OSD-1 ran on, over every pass.
    pub osd_runs: u64,
}

/// Decodes `samples` after `skip` missing ones with `setup`, and classifies
/// each of the `sent` messages. Only the decoder's own calls are timed.
/// With more than one pass configured, `decode_passes` runs every pass
/// (see [`decode_passes_input`]).
pub fn decode_input(samples: &[f32], skip: usize, sent: &[Sent], setup: &Setup) -> Decoded {
    if setup.config.passes > 1 {
        return decode_passes_input(samples, skip, sent, setup);
    }
    let band = setup.band;
    let mut front = Box::new(FrontEnd::new());
    let mut levels = vec![0u8; band.spectrogram_len()];
    let mut sync_buf = vec![0.0f32; Decoder::sync_scratch_len(band)];
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
    // The candidates OSD-1 ran on (any decode of theirs is OSD-1's: it
    // runs only after belief propagation failed).
    let mut osd_ran: BTreeSet<(u8, u16)> = BTreeSet::new();
    for i in 0..kept {
        let mut last = Instant::now();
        let mut ran = false;
        let r = dec.decode_candidate_staged(i, &mut scratch, |st| {
            let now = Instant::now();
            stages[st.index()] += (now - last).as_secs_f64();
            ran |= st == Stage::Osd;
            last = now;
        });
        if ran {
            osd_ran.insert((1, i as u16));
        }
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
        .map(|d| record(d, osd_ran.contains(&(1, d.candidate))))
        .collect();
    let outcomes = classify(&dec, rep.candidates_found, sent, &decodes, &failed, setup);
    Decoded {
        decodes,
        passes: None,
        osd_runs: osd_ran.len() as u64,
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

/// A decode's record; `osd` its origin.
fn record(d: &Decode, osd: bool) -> DecodeRecord {
    DecodeRecord {
        payload: d.payload.bits(),
        freq: d.freq_hz,
        dt: d.dt,
        snr: d.snr_db,
        score: d.score,
        candidate: d.candidate,
        hard_errors: d.hard_errors,
        rounds: d.rounds,
        osd,
    }
}

/// [`decode_input`] for several passes: `decode_passes_staged` over the
/// slot (the skipped region zero), timing each pass's steps. Seconds:
/// `feed`, `sync` and the candidate stages summed over passes, then
/// `subtract`, and per pass `pass<N>_<step>`. The counts are the slot's
/// totals over every pass (`PassesReport`), each pass's own in `passes`.
/// Misses are classified on
/// the last pass run (its spectrogram and candidates; its failures are
/// found again outside the timed calls).
pub fn decode_passes_input(samples: &[f32], skip: usize, sent: &[Sent], setup: &Setup) -> Decoded {
    let band = setup.band;
    let mut front = Box::new(FrontEnd::new());
    let mut levels = vec![0u8; band.spectrogram_len()];
    let mut sync_buf = vec![0.0f32; Decoder::sync_scratch_len(band)];
    let mut cands = vec![Candidate::default(); CANDIDATES.max(setup.config.candidate_capacity())];
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
    let mut audio = vec![0.0f32; skip];
    audio.extend_from_slice(samples);
    let mut seconds: BTreeMap<String, f64> = BTreeMap::new();
    let mut passes: Vec<PassRecord> = Vec::new();
    // (pass, candidate) of every candidate OSD-1 ran on: a pass reports
    // each candidate's stages in turn, each beginning with `Refine`.
    let mut osd_ran: BTreeSet<(u8, u16)> = BTreeSet::new();
    let mut at: Option<u16> = None;
    let mut last = Instant::now();
    let slot = dec.decode_passes_staged(&mut audio, skip, &mut scratch, &mut list, |e| {
        let now = Instant::now();
        match e {
            PassEvent::Step { pass, step } => {
                match step {
                    PassStep::Sync => at = None,
                    PassStep::Candidate(Stage::Refine) => at = Some(at.map_or(0, |i| i + 1)),
                    PassStep::Candidate(Stage::Osd) => {
                        osd_ran.insert((pass, at.expect("a candidate's first stage")));
                    }
                    _ => {}
                }
                let t = (now - last).as_secs_f64();
                let name = match step {
                    PassStep::Subtract => "subtract",
                    PassStep::Feed => "feed",
                    PassStep::Sync => "sync",
                    PassStep::Candidate(st) => st.name(),
                };
                *seconds.entry(name.to_string()).or_default() += t;
                *seconds.entry(format!("pass{pass}_{name}")).or_default() += t;
            }
            PassEvent::Done(r) => {
                let c = &r.report;
                // The join's premise: every kept candidate of the pass began
                // with one `Refine`. Checked in release builds too (the
                // bench runs only in them).
                assert_eq!(
                    at.map_or(0, |i| usize::from(i) + 1),
                    c.candidates_kept,
                    "pass {}: candidates counted from Refine",
                    r.pass
                );
                passes.push(PassRecord {
                    pass: u64::from(r.pass),
                    first: r.first as u64,
                    subtracted: r.subtracted as u64,
                    found: c.candidates_found as u64,
                    kept: c.candidates_kept as u64,
                    dropped: c.candidates_dropped as u64,
                    decoded: c.decoded as u64,
                    truncated: c.decodes_truncated as u64,
                    duplicates: c.duplicates as u64,
                    earlier: r.earlier_duplicates as u64,
                    all_zero: c.all_zero_rejected as u64,
                    crc_failed: c.crc_failed as u64,
                    not_converged: c.not_converged as u64,
                    bp_rounds: r.bp_rounds as u64,
                });
            }
            _ => {}
        }
        last = Instant::now();
        ControlFlow::Continue(())
    });
    // The slot's totals over every pass.
    let rep = slot.report;
    // The pass of list entry `i`: the last pass that began at or before it.
    let pass_of = |i: usize| {
        passes
            .iter()
            .rev()
            .find(|p| p.first as usize <= i)
            .map_or(1, |p| p.pass as u8)
    };
    let decodes: Vec<DecodeRecord> = list
        .finish_payloads()
        .iter()
        .enumerate()
        .map(|(i, d)| record(d, osd_ran.contains(&(pass_of(i), d.candidate))))
        .collect();
    // The last pass's failures, found again (deterministic, not timed).
    let mut failed: Vec<(usize, BpFailure)> = Vec::new();
    for i in 0..dec.candidates().len() {
        if let Some(Err(f)) = dec.decode_candidate(i, &mut scratch) {
            failed.push((i, f));
        }
    }
    let found = passes.last().map_or(0, |p| p.found as usize);
    let outcomes = classify(&dec, found, sent, &decodes, &failed, setup);
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
        bp_rounds: passes.iter().map(|p| p.bp_rounds).sum(),
        seconds: seconds.into_iter().collect(),
        outcomes,
        passes: Some(passes),
        osd_runs: osd_ran.len() as u64,
    }
}

/// The [`Outcome`] of each of the `sent` messages, outside the timed
/// calls; `found` is the sync peaks the search found.
fn classify(
    dec: &Decoder<'_>,
    found: usize,
    sent: &[Sent],
    decodes: &[DecodeRecord],
    failed: &[(usize, BpFailure)],
    setup: &Setup,
) -> Vec<Outcome> {
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
    for &(i, f) in failed {
        let at = position(spec, &dec.candidates()[i]);
        for (s, o) in sent.iter().zip(outcomes.iter_mut()) {
            if on(s, at) {
                *o = (*o).max(Outcome::of_failure(f));
            }
        }
    }
    // No kept candidate on these: was there a sync peak the limit dropped?
    // Every kept candidate either failed (above) or decoded a payload
    // (left out), so only the peaks beyond the kept list count.
    if !outcomes.contains(&Outcome::NoCandidate) {
        return outcomes;
    }
    let mut dropped_peaks: Vec<(f32, f32)> = Vec::new();
    if found > dec.candidates().len() {
        let mut scratch = vec![0.0f32; sync::scratch_len(setup.band.base_bins())];
        let mut all = vec![Candidate::default(); found];
        let rep = sync::find_candidates(spec, setup.config.min_score, &mut scratch, &mut all)
            .expect("sync scratch and list are sized");
        dropped_peaks = all[..rep.kept]
            .iter()
            .filter(|c| !dec.candidates().contains(c))
            .map(|c| position(spec, c))
            .collect();
    }
    for (s, o) in sent.iter().zip(outcomes.iter_mut()) {
        if *o == Outcome::NoCandidate {
            *o = if dropped_peaks.iter().any(|&at| on(s, at)) {
                Outcome::Dropped
            } else {
                Outcome::NoPeak
            };
        }
    }
    outcomes
}

/// Decodes one slot of a set.
pub fn decode_slot(set: &str, block: &str, index: usize, slot: &Slot, setup: &Setup) -> SlotResult {
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
        outcomes: Some(d.outcomes),
        passes: d.passes,
        osd_runs: d.osd_runs,
    }
}

// ------------------------------------------------------------------ run

pub fn run(spec: &str, out: &Path, sets: &str, block: &str) -> Result<(), String> {
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
