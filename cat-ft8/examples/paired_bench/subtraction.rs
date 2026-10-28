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

//! `suppression` and `passes`: how well `decode::subtract` removes one
//! decoded signal, and what decoding the residual again gives.

use std::collections::BTreeMap;
use std::time::Instant;

use cat_ft8::decode::{subtract, subtract_fixed, Decode, Fixed, SubtractConfig, Subtraction};
use cat_ft8::message::Payload;
use cat_ft8::{encode_payload, SAMPLE_RATE, SLOT_SAMPLES, WAVEFORM_SAMPLES};

use crate::channel::{self, check_against_synth, Channel};
use crate::common::{add_noise, amplitude, par_map, Rng, NOISE_RMS};
use crate::compare::quantile;
use crate::record::{decode_input, DecodeRecord, Outcome};
use crate::sets::{build, message, noise_seed, render, set_size, slot_seed, Sent};
use crate::{parse_config, provenance, Setup};

/// Parses `seg=N,drift=0|1` (either part optional; empty for the default).
pub fn parse_subtract(spec: &str) -> Result<SubtractConfig, String> {
    let mut c = SubtractConfig::DEFAULT;
    for kv in spec.split(',').filter(|s| !s.is_empty()) {
        let (k, v) = kv
            .split_once('=')
            .ok_or_else(|| format!("bad setting {kv}"))?;
        c = match k {
            "seg" => {
                let n: usize = v.parse().map_err(|e| format!("seg: {e}"))?;
                let ok = (SubtractConfig::MIN_SEGMENT..=WAVEFORM_SAMPLES).contains(&n)
                    && n % SubtractConfig::SEGMENT_STEP == 0;
                if !ok {
                    return Err(format!(
                        "seg must be a multiple of {} from {} to {WAVEFORM_SAMPLES}, not {n}",
                        SubtractConfig::SEGMENT_STEP,
                        SubtractConfig::MIN_SEGMENT
                    ));
                }
                c.with_segment(n)
            }
            "drift" => c.with_drift(match v {
                "0" => false,
                "1" => true,
                _ => return Err(format!("drift must be 0 or 1, not {v}")),
            }),
            _ => return Err(format!("unknown subtraction setting {k}")),
        };
    }
    Ok(c)
}

/// Every combination of a grid of settings: `key=v1|v2|…` per key,
/// comma-separated; the first key varies slowest. Several grids separated
/// by `;` are run one after the other. Empty: `default`.
pub fn expand_grid(spec: &str, default: &str) -> Vec<String> {
    let spec = if spec.is_empty() { default } else { spec };
    let mut all = Vec::new();
    for grid in spec.split(';') {
        let mut out = vec![String::new()];
        for kv in grid.split(',').filter(|s| !s.is_empty()) {
            let (k, vs) = kv.split_once('=').unwrap_or((kv, ""));
            out = out
                .iter()
                .flat_map(|prefix| {
                    vs.split('|').map(move |v| {
                        let sep = if prefix.is_empty() { "" } else { "," };
                        format!("{prefix}{sep}{k}={v}")
                    })
                })
                .collect();
        }
        all.extend(out);
    }
    all
}

/// The subtraction settings of a grid (see [`expand_grid`]).
pub fn parse_grid(spec: &str, default: &str) -> Result<Vec<SubtractConfig>, String> {
    expand_grid(spec, default)
        .iter()
        .map(|s| parse_subtract(s))
        .collect()
}

fn label(c: &SubtractConfig) -> String {
    format!("seg={}{}", c.segment, if c.drift { "" } else { ",nodrift" })
}

/// The decode a record describes.
fn to_decode(d: &DecodeRecord) -> Decode {
    let mut x = Decode::EMPTY;
    x.payload = Payload::from_bits(d.payload).unwrap();
    x.freq_hz = d.freq;
    x.dt = d.dt;
    x.snr_db = d.snr;
    x.score = d.score;
    x.candidate = d.candidate;
    x.hard_errors = d.hard_errors;
    x.rounds = d.rounds;
    x
}

fn energy(x: &[f32]) -> f64 {
    x.iter().map(|&v| f64::from(v) * f64::from(v)).sum()
}

fn db(a: f64, b: f64) -> f64 {
    10.0 * (a.max(1e-300) / b).log10()
}

/// Median, 10% and 90% quantiles, as text.
fn spread(v: &mut [f64], digits: usize) -> String {
    if v.is_empty() {
        return "-".into();
    }
    v.sort_by(f64::total_cmp);
    let q = |p| quantile(v, p).unwrap_or(f64::NAN);
    format!(
        "{:.d$} [{:.d$}, {:.d$}]",
        q(0.5),
        q(0.1),
        q(0.9),
        d = digits
    )
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    quantile(v, 0.5).unwrap_or(f64::NAN)
}

// ---------------------------------------------------------- suppression

/// One condition of the `suppression` mode. `None` fields are drawn per
/// trial: frequency 200–2 800 Hz, dt −0.3…+0.7 s, start fraction 0…1.
/// The input is the slot's samples `skip..cut` (the first `skip` missing).
#[derive(Clone)]
struct Condition {
    group: &'static str,
    label: String,
    snr: f64,
    freq: Option<f64>,
    dt: Option<f64>,
    frac: Option<f64>,
    channel: Channel,
    skip: usize,
    cut: usize,
}

fn conditions() -> Vec<Condition> {
    let base = |group, label: String, snr| Condition {
        group,
        label,
        snr,
        freq: None,
        dt: None,
        frac: None,
        channel: Channel::default(),
        skip: 0,
        cut: SLOT_SAMPLES,
    };
    let mut v = Vec::new();
    for snr in [-20.0, -16.0, -12.0, -8.0, -4.0, 0.0, 5.0, 10.0, 20.0] {
        v.push(base("snr", format!("{snr:+} dB"), snr));
    }
    for dt in [-2.0, -1.0, 0.0, 1.0, 2.0, 2.4, 2.5] {
        let mut c = base("dt", format!("dt {dt:+.1} s"), 10.0);
        c.dt = Some(dt);
        v.push(c);
    }
    for frac in [0.0, 0.25, 0.5] {
        let mut c = base("start", format!("start +{frac} sample"), 10.0);
        c.frac = Some(frac);
        v.push(c);
    }
    for f in [
        100.0,
        101.5625,
        200.0,
        1_500.0,
        1_500.781_25,
        1_501.562_5,
        2_850.0,
        3_050.0,
    ] {
        let mut c = base("freq", format!("{f} Hz"), 10.0);
        c.freq = Some(f);
        v.push(c);
    }
    for rate in [0.05, 0.1, 0.2, 0.5, 1.0, 2.0] {
        let mut c = base("fade", format!("fade {rate} Hz"), 10.0);
        c.channel.fade_hz = rate;
        c.channel.fade_depth = 0.9;
        v.push(c);
    }
    for drift in [0.02, 0.05, 0.1, 0.2, 0.5] {
        let mut c = base("drift", format!("drift {drift} Hz/s"), 10.0);
        c.channel.drift = drift;
        v.push(c);
    }
    // Input cut at 13.5 s, or starting 1 s late, as in the `edges` set.
    for snr in [10.0, 0.0] {
        let mut c = base("input", format!("cut 13.5 s, {snr:+} dB"), snr);
        c.cut = 162_000;
        v.push(c);
        let mut c = base("input", format!("late 1 s, {snr:+} dB"), snr);
        c.skip = 12_000;
        v.push(c);
    }
    v
}

/// The `suppression` measurements of one trial.
#[derive(Default, Clone)]
struct Trial {
    decoded: bool,
    /// Signal left after subtraction, dB relative to the signal: with
    /// everything estimated; with the true start (rounded) imposed; and
    /// on the noiseless signal, with everything estimated and with the
    /// true start, frequency and drift imposed (amplitude estimated).
    full: f64,
    true_start: f64,
    noiseless: f64,
    timing: f64,
    /// The `timing` measurement on the same signal started on a whole
    /// sample, and the difference (energies), dB: the sub-sample start's
    /// own share of the residue.
    whole: f64,
    timing_part: f64,
    /// `subtract`'s own estimate of the signal left, and its model part
    /// (residue without the noise's share), dB.
    reported: f64,
    reported_model: f64,
    /// Fine estimate errors: start (samples, before rounding), whole-sample
    /// start, frequency (Hz), drift (Hz/s); the decoder's start (samples)
    /// and frequency (Hz) errors.
    start_err: f64,
    start_int_err: i64,
    freq_err: f64,
    drift_err: f64,
    dec_start_err: f64,
    dec_freq_err: f64,
    seconds: f64,
    /// With each configuration of the segment sweep.
    sweep: Vec<f64>,
}

/// Signal left in `residual` (`audio` = `clean` + rest), dB re `clean`.
fn left_db(residual: &[f32], audio: &[f32], clean: &[f32]) -> f64 {
    let e: f64 = residual
        .iter()
        .zip(audio)
        .zip(clean)
        .map(|((&r, &a), &c)| {
            let x = f64::from(r) - (f64::from(a) - f64::from(c));
            x * x
        })
        .sum();
    db(e, energy(clean))
}

fn trial(
    cond: &Condition,
    index: usize,
    setup: &Setup,
    config: &SubtractConfig,
    sweep: &[SubtractConfig],
) -> Trial {
    let seed = slot_seed("suppression", &cond.label, index);
    let mut r = Rng(seed);
    let (text, payload) = message(&mut r, &[]);
    let freq = cond.freq.unwrap_or_else(|| 200.0 + 2_600.0 * r.uniform());
    let dt = cond.dt.unwrap_or_else(|| -0.3 + r.uniform());
    let frac = cond.frac.unwrap_or_else(|| r.uniform());
    let fs = f64::from(SAMPLE_RATE);
    let start = ((0.5 + dt) * fs).round() + frac;
    let mut ch = cond.channel;
    if ch.fade_depth > 0.0 {
        ch.fade_phase = r.uniform();
    }
    let sent = Sent {
        text,
        payload: payload.bits(),
        freq,
        dt: start / fs - 0.5,
        snr: cond.snr,
        channel: Some(ch),
    };
    let mut clean = vec![0.0f32; SLOT_SAMPLES];
    let amp = f64::from(amplitude(cond.snr, NOISE_RMS));
    channel::render(&mut clean, &encode_payload(&payload), freq, start, amp, &ch);
    let mut audio = clean.clone();
    add_noise(&mut audio, NOISE_RMS, noise_seed(seed));
    // The input: samples skip..cut. The skipped region is zero in every
    // buffer (it is not input), so it adds nothing to the signal left.
    let skip = cond.skip;
    let usable = |mut x: Vec<f32>| {
        x.truncate(cond.cut);
        x[..skip].fill(0.0);
        x
    };
    let (clean, audio) = (usable(clean), usable(audio));
    let d = decode_input(&audio[skip..], skip, std::slice::from_ref(&sent), setup);
    let Some(rec) = d.decodes.iter().find(|x| x.payload == sent.payload) else {
        return Trial::default();
    };
    let dec = to_decode(rec);
    let true_start = start.round() as i64;
    let run = |input: &[f32], cfg: &SubtractConfig, fixed: Fixed| -> (Vec<f32>, Subtraction) {
        let mut res = input.to_vec();
        let s = subtract_fixed(&mut res, &dec, skip, cfg, fixed).unwrap();
        (res, s)
    };
    let t = Instant::now();
    let mut res = audio.clone();
    let s = subtract(&mut res, &dec, skip, config).unwrap();
    let seconds = t.elapsed().as_secs_f64();
    let full = left_db(&res, &audio, &clean);
    let at_start = Fixed {
        start: Some(true_start),
        freq: None,
    };
    let (res_t, _) = run(&audio, config, at_start);
    let (res_n, _) = run(&clean, config, Fixed::default());
    let truth = Fixed {
        start: Some(true_start),
        freq: Some(((freq - f64::from(dec.freq_hz)) as f32, ch.drift as f32)),
    };
    let (res_timing, _) = run(&clean, config, truth);
    let mut clean_whole = vec![0.0f32; SLOT_SAMPLES];
    channel::render(
        &mut clean_whole,
        &encode_payload(&payload),
        freq,
        start.round(),
        amp,
        &ch,
    );
    let clean_whole = usable(clean_whole);
    let (res_whole, _) = run(&clean_whole, config, truth);
    let timing = left_db(&res_timing, &clean, &clean);
    let whole = left_db(&res_whole, &clean_whole, &clean_whole);
    let timing_part = {
        let p = 10f64.powf(timing / 10.0) - 10f64.powf(whole / 10.0);
        10.0 * p.max(1e-30).log10()
    };
    let reported = f64::from(s.left_db().unwrap_or(f32::NAN));
    let reported_model = f64::from(s.residue_db().unwrap_or(f32::NAN));
    let sweep = sweep
        .iter()
        .map(|c| {
            let mut x = audio.clone();
            subtract(&mut x, &dec, skip, c).unwrap();
            left_db(&x, &audio, &clean)
        })
        .collect();
    Trial {
        decoded: true,
        full,
        true_start: left_db(&res_t, &audio, &clean),
        noiseless: left_db(&res_n, &clean, &clean),
        timing,
        whole,
        timing_part,
        reported,
        reported_model,
        start_err: (f64::from(s.dt) - sent.dt) * fs,
        start_int_err: s.start - true_start,
        freq_err: f64::from(s.freq_hz) - freq,
        drift_err: f64::from(s.drift_hz_per_s) - ch.drift,
        dec_start_err: (f64::from(dec.dt) - sent.dt) * fs,
        dec_freq_err: f64::from(dec.freq_hz) - freq,
        seconds,
        sweep,
    }
}

/// `suppression`'s grid when none is given: the default first (the
/// detailed tables), then the segment lengths with drift on and off.
const SUPPRESSION_GRID: &str = "seg=1920|960|3840|7680|151680,drift=1|0";

/// The largest difference allowed between the bench's generator and
/// `Synth` at whole-sample starts (measured 6.6 × 10⁻⁴ of full scale).
const GENERATOR_TOLERANCE: f64 = 1e-3;

/// Fails unless the bench's generator matches `Synth`.
fn checked_generator() -> Result<f64, String> {
    let worst = check_against_synth();
    println!(
        "bench generator vs Synth (whole-sample starts): largest difference {worst:.2e} of full scale (limit {GENERATOR_TOLERANCE:.0e})"
    );
    // False for NaN as well.
    if worst <= GENERATOR_TOLERANCE {
        Ok(worst)
    } else {
        Err(format!(
            "the bench generator differs from Synth by {worst:.2e} (limit {GENERATOR_TOLERANCE:.0e})"
        ))
    }
}

/// Residual suppression of single signals across conditions with the
/// first setting of GRID, and the signal left with every setting of it.
pub fn suppression(spec: &str, grid: &str, trials: usize) -> Result<(), String> {
    let setup = parse_config(spec)?;
    let sweep = parse_grid(grid, SUPPRESSION_GRID)?;
    let config = sweep[0];
    println!(
        "config {spec}: {:?}\nsubtraction {} (grid {})\n{}",
        setup.config,
        label(&config),
        if grid.is_empty() {
            SUPPRESSION_GRID
        } else {
            grid
        },
        provenance()
    );
    checked_generator()?;
    let conds = conditions();
    let t = Instant::now();
    let results: Vec<Vec<Trial>> = conds
        .iter()
        .map(|c| par_map(trials, |i| trial(c, i, &setup, &config, &sweep)))
        .collect();
    eprintln!("suppression: {:.0} s", t.elapsed().as_secs_f64());
    println!("\n== signal left after subtraction, dB re the signal: median [10%, 90%] over decoded trials");
    println!("   full: everything estimated, in noise; start: the true start (rounded) imposed; noiseless: everything estimated, no noise;");
    println!("   timing: no noise, true start (rounded), frequency and drift imposed, amplitude estimated; whole: the same signal started on a sample;");
    println!("   sub-sample: timing less whole (energies), median and largest: the residue a start between samples leaves (−300: none);");
    println!("   reported: subtract's own estimate of the signal left (left_db) / its model part (residue_db), dB re the energy removed (medians)");
    println!(
        "  {:<6} {:<22} {:>5} {:>22} {:>8} {:>10} {:>22} {:>8} {:>14} {:>13}",
        "group",
        "condition",
        "dec",
        "full",
        "start",
        "noiseless",
        "timing",
        "whole",
        "sub-sample",
        "reported"
    );
    for (c, r) in conds.iter().zip(&results) {
        let ok: Vec<&Trial> = r.iter().filter(|t| t.decoded).collect();
        let col = |f: &dyn Fn(&Trial) -> f64| -> Vec<f64> { ok.iter().map(|t| f(t)).collect() };
        println!(
            "  {:<6} {:<22} {:>5} {:>22} {:>8.1} {:>10.1} {:>22} {:>8.1} {:>14} {:>13}",
            c.group,
            c.label,
            ok.len(),
            spread(&mut col(&|t| t.full), 1),
            median(&mut col(&|t| t.true_start)),
            median(&mut col(&|t| t.noiseless)),
            spread(&mut col(&|t| t.timing), 1),
            median(&mut col(&|t| t.whole)),
            {
                let mut v = col(&|t| t.timing_part);
                let m = median(&mut v);
                format!("{m:.1} / {:.1}", v.last().copied().unwrap_or(f64::NAN))
            },
            format!(
                "{:.1}/{:.1}",
                median(&mut col(&|t| t.reported)),
                median(&mut col(&|t| t.reported_model))
            ),
        );
    }
    println!("\n== estimate errors: |error| median [90%]; the decoder's own in brackets");
    println!(
        "  {:<6} {:<22} {:>24} {:>9} {:>24} {:>14}",
        "group", "condition", "start, samples (dec)", "int ok %", "freq, Hz (dec)", "drift, Hz/s"
    );
    let mq = |v: &mut Vec<f64>| -> (f64, f64) {
        v.sort_by(f64::total_cmp);
        (
            quantile(v, 0.5).unwrap_or(f64::NAN),
            quantile(v, 0.9).unwrap_or(f64::NAN),
        )
    };
    for (c, r) in conds.iter().zip(&results) {
        let ok: Vec<&Trial> = r.iter().filter(|t| t.decoded).collect();
        let col =
            |f: &dyn Fn(&Trial) -> f64| -> Vec<f64> { ok.iter().map(|t| f(t).abs()).collect() };
        let (s, s9) = mq(&mut col(&|t| t.start_err));
        let (ds, ds9) = mq(&mut col(&|t| t.dec_start_err));
        let (f, f9) = mq(&mut col(&|t| t.freq_err));
        let (df, df9) = mq(&mut col(&|t| t.dec_freq_err));
        let (g, g9) = mq(&mut col(&|t| t.drift_err));
        let exact = ok.iter().filter(|t| t.start_int_err == 0).count();
        println!(
            "  {:<6} {:<22} {:>24} {:>9.0} {:>24} {:>14}",
            c.group,
            c.label,
            format!("{s:.2} [{s9:.2}] ({ds:.0} [{ds9:.0}])"),
            100.0 * exact as f64 / ok.len().max(1) as f64,
            format!("{f:.3} [{f9:.3}] ({df:.2} [{df9:.2}])"),
            format!("{g:.3} [{g9:.3}]"),
        );
    }
    let mut secs: Vec<f64> = results
        .iter()
        .flatten()
        .filter(|t| t.decoded)
        .map(|t| t.seconds)
        .collect();
    println!(
        "\nsubtract, one call (all threads busy): median {:.2} ms",
        1e3 * median(&mut secs)
    );
    println!("\n== the grid: signal left (full), median dB, per setting");
    let mut head = format!("  {:<6} {:<22}", "group", "condition");
    for c in &sweep {
        head.push_str(&format!(" {:>16}", label(c)));
    }
    println!("{head}");
    for (c, r) in conds.iter().zip(&results) {
        let ok: Vec<&Trial> = r.iter().filter(|t| t.decoded).collect();
        let mut line = format!("  {:<6} {:<22}", c.group, c.label);
        for k in 0..sweep.len() {
            let mut v: Vec<f64> = ok.iter().map(|t| t.sweep[k]).collect();
            line.push_str(&format!(" {:>16.1}", median(&mut v)));
        }
        println!("{line}");
    }
    Ok(())
}

// --------------------------------------------------------------- passes

/// Further passes after the single pass (passes 2 and 3).
const PASSES: usize = 2;

/// One setting's further passes on one slot.
#[derive(Default, Clone)]
struct Real {
    /// Decoded by pass 2 or 3 at the latest, of the sent messages.
    real: [usize; PASSES],
    /// Decoded by a real pass and not by the oracle pass, by pass.
    real_not_oracle: [usize; PASSES],
    /// Per pass: payloads of earlier passes decoded again (duplicates), not
    /// transmitted (false decodes), decodes subtracted before the pass.
    duplicates: [usize; PASSES],
    false_decodes: [usize; PASSES],
    subtracted: [usize; PASSES],
    /// SNRs of the duplicated messages.
    duplicate_snrs: Vec<f64>,
    /// Seconds per subtraction (all threads busy), and the further passes'
    /// decode seconds.
    subtract_seconds: Vec<f64>,
    decode_seconds: [f64; PASSES],
}

/// One slot of the `passes` mode: the single pass, the oracle passes and
/// each setting's real passes.
#[derive(Default)]
struct PassSlot {
    size: usize,
    sent: usize,
    /// Decoded by pass 1, and by oracle pass 2 or 3 at the latest.
    single: usize,
    oracle: [usize; PASSES],
    real: Vec<Real>,
}

/// The real passes of one setting with the bench's own loop.
fn real_passes(
    slot: &crate::sets::Slot,
    p1: &crate::record::Decoded,
    setup: &Setup,
    cfg: &SubtractConfig,
) -> (Real, Vec<Vec<bool>>) {
    let sent = &slot.sent;
    let skip = slot.skip;
    let mut out = Real::default();
    let mut residual = vec![0.0f32; skip];
    residual.extend_from_slice(&slot.samples);
    let mut known: Vec<u128> = p1.decodes.iter().map(|d| d.payload).collect();
    let mut found: Vec<bool> = p1.outcomes.iter().map(|&o| o == Outcome::Decoded).collect();
    let mut new: Vec<DecodeRecord> = p1.decodes.clone();
    let mut found_by_pass = Vec::new();
    for pass in 0..PASSES {
        for d in &new {
            let t = Instant::now();
            subtract(&mut residual, &to_decode(d), skip, cfg).unwrap();
            out.subtract_seconds.push(t.elapsed().as_secs_f64());
        }
        out.subtracted[pass] = new.len();
        let t = Instant::now();
        let d = decode_input(&residual[skip..], skip, sent, setup);
        out.decode_seconds[pass] = t.elapsed().as_secs_f64();
        new.clear();
        for x in &d.decodes {
            if known.contains(&x.payload) {
                out.duplicates[pass] += 1;
                if let Some(s) = sent.iter().find(|s| s.payload == x.payload) {
                    out.duplicate_snrs.push(s.snr);
                }
                continue;
            }
            match sent.iter().position(|s| s.payload == x.payload) {
                Some(k) => found[k] = true,
                None => out.false_decodes[pass] += 1,
            }
            known.push(x.payload);
            new.push(x.clone());
        }
        out.real[pass] = found.iter().filter(|&&f| f).count();
        found_by_pass.push(found.clone());
    }
    (out, found_by_pass)
}

fn pass_slot(
    set: &str,
    block: &str,
    index: usize,
    setup: &Setup,
    grid: &[SubtractConfig],
) -> PassSlot {
    let slot = build(set, block, index);
    let sent = &slot.sent;
    let skip = slot.skip;
    let mut out = PassSlot {
        size: sent.len(),
        sent: sent.len(),
        ..PassSlot::default()
    };
    let p1 = decode_input(&slot.samples, skip, sent, setup);
    out.single = p1
        .outcomes
        .iter()
        .filter(|&&o| o == Outcome::Decoded)
        .count();
    // Oracle passes: the slot rebuilt without every message decoded so far
    // (true waveforms, same noise).
    let ns = noise_seed(slot_seed(set, block, index));
    let mut found_o: Vec<bool> = p1.outcomes.iter().map(|&o| o == Outcome::Decoded).collect();
    let mut oracle_by_pass = Vec::new();
    for pass in 0..PASSES {
        let left: Vec<Sent> = sent
            .iter()
            .zip(&found_o)
            .filter(|(_, f)| !**f)
            .map(|(s, _)| s.clone())
            .collect();
        let d = decode_input(&render(&left, ns)[skip..], skip, &left, setup);
        for x in &d.decodes {
            if let Some(k) = sent.iter().position(|s| s.payload == x.payload) {
                found_o[k] = true;
            }
        }
        out.oracle[pass] = found_o.iter().filter(|&&f| f).count();
        oracle_by_pass.push(found_o.clone());
    }
    for cfg in grid {
        let (mut real, found_by_pass) = real_passes(&slot, &p1, setup, cfg);
        for pass in 0..PASSES {
            real.real_not_oracle[pass] = found_by_pass[pass]
                .iter()
                .zip(&oracle_by_pass[pass])
                .filter(|(r, o)| **r && !**o)
                .count();
        }
        out.real.push(real);
    }
    out
}

/// Further passes on a crowded set with the bench's own loop, for every
/// setting of GRID: subtraction of each pass's new decodes in list order,
/// then the residual decoded again with the same configuration; beside the
/// oracle passes (computed once).
pub fn passes(spec: &str, block: &str, set: &str, grid: &str) -> Result<(), String> {
    if !["crowded", "wide", "fading"].contains(&set) {
        return Err(format!("passes takes crowded, wide or fading, not {set}"));
    }
    let setup = parse_config(spec)?;
    let grid = parse_grid(grid, "")?;
    println!(
        "config {spec}: {:?}\nsubtraction {}\n{}",
        setup.config,
        grid.iter().map(label).collect::<Vec<_>>().join("; "),
        provenance()
    );
    let t = Instant::now();
    let per = par_map(set_size(set).unwrap_or(0), |i| {
        pass_slot(set, block, i, &setup, &grid)
    });
    eprintln!("passes {set}-{block}: {:.0} s", t.elapsed().as_secs_f64());
    for (g, cfg) in grid.iter().enumerate() {
        println!("\n######## subtraction {}", label(cfg));
        report_passes(set, block, &per, g);
    }
    if grid.len() > 1 {
        println!("\n== summary ({set}, {block}): share of the oracle's gain +1 / +2, duplicates p2 / p3, false p2 / p3");
        let all: Vec<&PassSlot> = per.iter().collect();
        let single: usize = all.iter().map(|p| p.single).sum();
        let oracle: Vec<usize> = (0..PASSES)
            .map(|k| all.iter().map(|p| p.oracle[k]).sum())
            .collect();
        for (g, cfg) in grid.iter().enumerate() {
            let sum = |f: &dyn Fn(&Real) -> usize| all.iter().map(|p| f(&p.real[g])).sum::<usize>();
            let share = |k: usize| {
                let real = sum(&|r| r.real[k]);
                100.0 * (real as f64 - single as f64) / (oracle[k] as f64 - single as f64).max(1.0)
            };
            println!(
                "  {:<22} {:>6.1}% / {:>6.1}%   {:>4} / {:>4}   {} / {}",
                label(cfg),
                share(0),
                share(1),
                sum(&|r| r.duplicates[0]),
                sum(&|r| r.duplicates[1]),
                sum(&|r| r.false_decodes[0]),
                sum(&|r| r.false_decodes[1]),
            );
        }
    }
    Ok(())
}

/// The `passes` tables of setting `g`.
fn report_passes(set: &str, block: &str, per: &[PassSlot], g: usize) {
    let mut sizes: BTreeMap<usize, Vec<&PassSlot>> = BTreeMap::new();
    for p in per {
        sizes.entry(p.size).or_default().push(p);
    }
    let all: Vec<&PassSlot> = per.iter().collect();
    let groups: Vec<(String, Vec<&PassSlot>)> = sizes
        .iter()
        .map(|(z, v)| (z.to_string(), v.clone()))
        .chain([("all".to_string(), all)])
        .collect();
    let sum =
        |v: &[&PassSlot], f: &dyn Fn(&PassSlot) -> usize| v.iter().map(|p| f(p)).sum::<usize>();
    let pct = |a: usize, b: usize| 100.0 * a as f64 / b.max(1) as f64;
    println!("\n== {set} ({block}): messages decoded, % of transmitted (real pass / oracle pass)");
    println!(
        "  {:>5} {:>6} {:>8} {:>16} {:>16} {:>10} {:>10}",
        "size", "sent", "single", "+1 real/oracle", "+2 real/oracle", "share +1", "share +2"
    );
    for (name, v) in &groups {
        let sent = sum(v, &|p| p.sent);
        let single = sum(v, &|p| p.single);
        let real: Vec<usize> = (0..PASSES)
            .map(|k| sum(v, &|p| p.real[g].real[k]))
            .collect();
        let oracle: Vec<usize> = (0..PASSES).map(|k| sum(v, &|p| p.oracle[k])).collect();
        let share = |k: usize| {
            100.0 * (real[k] as f64 - single as f64) / (oracle[k] as f64 - single as f64).max(1.0)
        };
        println!(
            "  {:>5} {:>6} {:>8} {:>16} {:>16} {:>9.1}% {:>9.1}%",
            name,
            sent,
            format!("{:.1}", pct(single, sent)),
            format!("{:.1}/{:.1}", pct(real[0], sent), pct(oracle[0], sent)),
            format!("{:.1}/{:.1}", pct(real[1], sent), pct(oracle[1], sent)),
            share(0),
            share(1)
        );
    }
    let last = &groups.last().unwrap().1;
    println!("  counts (all): single {}, real +1 {}, +2 {}; oracle +1 {}, +2 {}; decoded by a real pass but not the oracle: +1 {}, +2 {}",
        sum(last, &|p| p.single),
        sum(last, &|p| p.real[g].real[0]),
        sum(last, &|p| p.real[g].real[1]),
        sum(last, &|p| p.oracle[0]),
        sum(last, &|p| p.oracle[1]),
        sum(last, &|p| p.real[g].real_not_oracle[0]),
        sum(last, &|p| p.real[g].real_not_oracle[1]),
    );
    println!("  (share +k: real gain over the single pass / oracle gain, same configuration; may exceed 100%)");
    println!(
        "\n== duplicates (earlier payloads decoded again) and false decodes, per further pass"
    );
    println!(
        "  {:>5} {:>12} {:>12} {:>12} {:>12} {:>8} {:>8}",
        "size", "subtr. p2", "dup p2", "subtr. p3", "dup p3", "false p2", "false p3"
    );
    for (name, v) in &groups {
        let s2 = sum(v, &|p| p.real[g].subtracted[0]);
        let d2 = sum(v, &|p| p.real[g].duplicates[0]);
        let s3 = sum(v, &|p| p.real[g].subtracted[1]);
        let d3 = sum(v, &|p| p.real[g].duplicates[1]);
        println!(
            "  {:>5} {:>12} {:>12} {:>12} {:>12} {:>8} {:>8}",
            name,
            s2,
            format!("{d2} ({:.2}%)", pct(d2, s2)),
            s3,
            format!("{d3} ({:.2}%)", pct(d3, s3)),
            sum(v, &|p| p.real[g].false_decodes[0]),
            sum(v, &|p| p.real[g].false_decodes[1]),
        );
    }
    let mut snrs: Vec<f64> = per
        .iter()
        .flat_map(|p| p.real[g].duplicate_snrs.iter().copied())
        .collect();
    snrs.sort_by(f64::total_cmp);
    let mut hist: BTreeMap<i64, usize> = BTreeMap::new();
    for s in &snrs {
        *hist.entry((s / 5.0).floor() as i64 * 5).or_default() += 1;
    }
    println!("  duplicates by the message's SNR (5 dB bins, lower edge): {hist:?}");
    let mut sub: Vec<f64> = per
        .iter()
        .flat_map(|p| p.real[g].subtract_seconds.iter().copied())
        .collect();
    let total: f64 = sub.iter().sum();
    let slots = per.len().max(1) as f64;
    let mut dec2: Vec<f64> = per.iter().map(|p| p.real[g].decode_seconds[0]).collect();
    println!(
        "\ncost (all threads busy): subtraction median {:.2} ms per call, {:.1} ms per slot on average; pass-2 decode median {:.1} ms",
        1e3 * median(&mut sub),
        1e3 * total / slots,
        1e3 * median(&mut dec2)
    );
}

/// Single-thread cost of `subtract`: the pass-1 decodes of `slots`
/// 40-message crowded slots, subtracted in list order, `runs` times; the
/// median over runs of each slot's time, reported per call.
pub fn subtract_cost(spec: &str, sub: &str, slots: usize, runs: usize) -> Result<(), String> {
    let setup = parse_config(spec)?;
    let cfg = parse_subtract(sub)?;
    let picked: Vec<(Vec<f32>, Vec<Decode>)> = par_map(slots, |i| {
        let slot = build("crowded", "main", 400 + i);
        let d = decode_input(&slot.samples, 0, &slot.sent, &setup);
        (slot.samples, d.decodes.iter().map(to_decode).collect())
    });
    let mut per_call: Vec<f64> = Vec::new();
    let mut calls = 0usize;
    for (audio, decodes) in &picked {
        let mut times = Vec::new();
        for _ in 0..runs.max(5) {
            let mut res = audio.clone();
            let t = Instant::now();
            for d in decodes {
                subtract(&mut res, d, 0, &cfg).unwrap();
            }
            times.push(t.elapsed().as_secs_f64());
        }
        calls += decodes.len();
        per_call.push(median(&mut times) / decodes.len().max(1) as f64);
    }
    println!(
        "subtract ({}), one thread, {} slots of 40 messages, {} calls: median per call {:.3} ms (per-slot medians: {})",
        label(&cfg),
        picked.len(),
        calls,
        1e3 * median(&mut per_call.clone()),
        spread(&mut per_call.iter().map(|x| x * 1e3).collect::<Vec<_>>(), 3)
    );
    Ok(())
}
