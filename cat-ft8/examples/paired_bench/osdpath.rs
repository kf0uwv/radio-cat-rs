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

//! `osdpath CONFIG C:T [SLOTS] [RUNS] [BLOCK]`: OSD-1's cost per candidate through
//! the decoder, everything the option adds included (the storage switch
//! and its zero-fill, the next candidate's fresh belief-propagation
//! storage, OSD-1 itself and its acceptance tests).
//!
//! Inputs: each pass's input on the slotcost slots (`crowded`, BLOCK,
//! `main` unless given, from index 400): pass 1 the slot, a later pass the residual the
//! decoder's own passes leave (OSD off), searched as that pass searches.
//! Two decoders on each input, OSD off and OSD on (`OsdConfig::new(C,
//! T)`), hold the same candidates; one thread times every candidate's
//! `decode_candidate` on each, alternated, RUNS times (at least 5), and
//! takes each call's median (which decoder goes first alternates from run
//! to run). A candidate's cost is its median with OSD on less its median
//! with OSD off. Printed: the sum of the costs of the candidates OSD ran
//! on and of the candidates after them, over the OSD runs (the mean per
//! run, the following candidates' storage refill included), the cost of each candidate OSD ran on and
//! did not accept (mean, median, largest; the refill is its successor's),
//! the refill (candidates after one OSD ran on), and the accepted ones
//! apart (they also re-encode and estimate the SNR).
//!
//! `osdinput CONFIG SLOT PASS FILE` writes one such input (from the
//! slot's first sample after the skipped ones) as `f32` little-endian, for
//! an emulated target's decoder.

use std::fs;
use std::path::Path;
use std::time::Instant;

use cat_ft8::decode::{
    Candidate, CandidateScratch, Decode, DecodeList, Decoder, FrontEnd, OsdConfig, Stage,
    CANDIDATES,
};

use crate::common::par_map;
use crate::sets::{build, Slot};
use crate::{parse_config, Setup};

/// Caller-owned decoder buffers.
struct Bufs {
    front: Box<FrontEnd>,
    levels: Vec<u8>,
    sync: Vec<f32>,
    cands: Vec<Candidate>,
}

impl Bufs {
    fn new(setup: &Setup) -> Self {
        let band = setup.band;
        Bufs {
            front: Box::new(FrontEnd::new()),
            levels: vec![0; band.spectrogram_len()],
            sync: vec![0.0; Decoder::sync_scratch_len(band)],
            cands: vec![Candidate::default(); CANDIDATES.max(setup.config.candidate_capacity())],
        }
    }

    fn decoder(&mut self, setup: &Setup, osd: Option<OsdConfig>) -> Decoder<'_> {
        Decoder::new(
            &mut self.front,
            &mut self.levels,
            &mut self.sync,
            &mut self.cands,
            setup.band,
            setup.config.with_osd(osd),
        )
        .expect("the configuration is valid")
    }
}

/// The audio pass `pass` of `slot` decodes, OSD off, from slot sample 0
/// (`slot.skip` leading zeros); `None` if the decoder stops before it.
fn pass_input(setup: &Setup, slot: &Slot, pass: u8) -> Option<Vec<f32>> {
    let mut b = Bufs::new(setup);
    let config = setup.config.with_osd(None).with_passes(pass);
    let mut dec = Decoder::new(
        &mut b.front,
        &mut b.levels,
        &mut b.sync,
        &mut b.cands,
        setup.band,
        config,
    )
    .expect("the configuration is valid");
    let mut store = vec![Decode::EMPTY; setup.decodes];
    let mut list = DecodeList::new(&mut store);
    let mut scratch = Box::new(CandidateScratch::new());
    let mut audio = vec![0.0f32; slot.skip];
    audio.extend_from_slice(&slot.samples);
    let rep = dec.decode_passes(&mut audio, slot.skip, &mut scratch, &mut list);
    (rep.passes_run == pass).then_some(audio)
}

/// Feeds `audio` (from slot sample 0, `skip` missing) and searches as pass
/// `pass` does.
fn search(dec: &mut Decoder<'_>, audio: &[f32], skip: usize, pass: u8) -> usize {
    dec.skip(skip);
    dec.feed(&audio[skip..]);
    if pass == 1 {
        dec.find_candidates().kept
    } else {
        dec.find_later_candidates().kept
    }
}

fn parse_osd(spec: &str) -> Result<OsdConfig, String> {
    let (c, t) = spec.split_once(':').ok_or("OSD needs C:T")?;
    let c: u16 = c.parse().map_err(|_| format!("C {c}"))?;
    let t: u8 = t.parse().map_err(|_| format!("T {t}"))?;
    Ok(OsdConfig::new(c, t))
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

pub fn osd_path(
    cfg: &str,
    osd: &str,
    slots: usize,
    runs: usize,
    block: &str,
) -> Result<(), String> {
    let setup = parse_config(cfg)?;
    let osd = parse_osd(osd)?;
    let slots = slots.clamp(1, 200);
    let runs = runs.max(5);
    let passes = setup.config.passes;
    // Every (slot, pass) input, made in parallel; timed on one thread.
    let inputs: Vec<(u8, usize, Vec<f32>)> = par_map(slots, |k| {
        let slot = build("crowded", block, 400 + k);
        (1..=passes)
            .filter_map(|p| pass_input(&setup, &slot, p).map(|a| (p, slot.skip, a)))
            .collect::<Vec<_>>()
    })
    .into_iter()
    .flatten()
    .collect();
    let (mut candidates, mut ran_n, mut accepted_n) = (0usize, 0usize, 0usize);
    let mut sum = 0.0f64;
    let (mut plain, mut refill, mut accepted) = (Vec::new(), Vec::new(), Vec::new());
    let mut by_pass = [0usize; 4];
    for (pass, skip, audio) in &inputs {
        let (mut b_off, mut b_on) = (Bufs::new(&setup), Bufs::new(&setup));
        let mut off = b_off.decoder(&setup, None);
        let mut on = b_on.decoder(&setup, Some(osd));
        let kept = search(&mut off, audio, *skip, *pass);
        if search(&mut on, audio, *skip, *pass) != kept || off.candidates() != on.candidates() {
            return Err("the two decoders' candidates differ".into());
        }
        // Which candidates OSD-1 runs on and accepts (untimed).
        let mut s = Box::new(CandidateScratch::new());
        let mut ran = vec![false; kept];
        let mut ok = vec![false; kept];
        for (i, (r, a)) in ran.iter_mut().zip(ok.iter_mut()).enumerate() {
            let res = on.decode_candidate_staged(i, &mut s, |st| *r |= st == Stage::Osd);
            *a = *r && matches!(res, Some(Ok(_)));
        }
        let (mut s_off, mut s_on) = (
            Box::new(CandidateScratch::new()),
            Box::new(CandidateScratch::new()),
        );
        let mut t_off = vec![Vec::with_capacity(runs); kept];
        let mut t_on = vec![Vec::with_capacity(runs); kept];
        for r in 0..runs {
            for i in 0..kept {
                // Which goes first alternates, so neither gains from the
                // other's warm caches.
                for side in [r % 2, 1 - r % 2] {
                    let (dec, s, t_v) = if side == 0 {
                        (&off, &mut s_off, &mut t_off)
                    } else {
                        (&on, &mut s_on, &mut t_on)
                    };
                    let t = Instant::now();
                    std::hint::black_box(dec.decode_candidate(i, s));
                    t_v[i].push(t.elapsed().as_secs_f64());
                }
            }
        }
        for i in 0..kept {
            // Only these candidates' work differs (an OSD run, or the
            // fresh storage after one); the others' differences are noise.
            if !(ran[i] || i > 0 && ran[i - 1]) {
                continue;
            }
            let d = (median(&mut t_on[i]) - median(&mut t_off[i])) * 1e6;
            sum += d;
            if ok[i] {
                accepted.push(d);
            } else if ran[i] {
                plain.push(d);
            } else if i > 0 && ran[i - 1] {
                refill.push(d);
            }
        }
        candidates += kept;
        let n = ran.iter().filter(|&&r| r).count();
        ran_n += n;
        by_pass[usize::from(*pass)] += n;
        accepted_n += ok.iter().filter(|&&a| a).count();
    }
    println!(
        "{cfg} osd {osd:?}: {slots} slots{}, {} inputs, {candidates} candidates; OSD-1 ran on {ran_n} (pass 1 {}, 2 {}, 3 {}), accepted {accepted_n}",
        if block == "main" {
            String::new()
        } else {
            format!(" ({block})")
        },
        inputs.len(),
        by_pass[1],
        by_pass[2],
        by_pass[3]
    );
    println!(
        "  all candidates' cost over the OSD runs: {:.3} us per run",
        sum / ran_n.max(1) as f64
    );
    for (name, v) in [
        ("ran, not accepted", &mut plain),
        ("refill (next candidate)", &mut refill),
        ("accepted", &mut accepted),
    ] {
        if v.is_empty() {
            println!("  {name}: none");
            continue;
        }
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        let med = median(v);
        println!(
            "  {name}: {} candidates, mean {mean:.3} us, median {med:.3}, max {:.3}",
            v.len(),
            v[v.len() - 1]
        );
    }
    Ok(())
}

pub fn osd_input(cfg: &str, slot: usize, pass: u8, file: &Path) -> Result<(), String> {
    let setup = parse_config(cfg)?;
    let s = build("crowded", "main", 400 + slot);
    let audio = pass_input(&setup, &s, pass).ok_or("the decoder stops before that pass")?;
    let bytes: Vec<u8> = audio[s.skip..]
        .iter()
        .flat_map(|x| x.to_le_bytes())
        .collect();
    fs::write(file, bytes).map_err(|e| e.to_string())?;
    let mut b = Bufs::new(&setup);
    let mut dec = b.decoder(&setup, None);
    let kept = search(&mut dec, &audio, s.skip, pass);
    println!(
        "{cfg} slot {} pass {pass}: skip {}, {} samples, {kept} candidates",
        400 + slot,
        s.skip,
        audio.len() - s.skip
    );
    Ok(())
}
