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

//! `cat-ft8-decode FILE.wav [--config host|small[,passes=N]] [--raw] [--profile]`
//!
//! Decodes one FT8 slot from a 12 kHz mono 16-bit WAV file and prints one
//! JSON object per decode, in output format version 0 (see the crate docs,
//! "Decode command output"). The file's first 15 s are the slot: a shorter
//! file is decoded as a partial slot, and anything after 15 s is ignored.
//! The configuration's pass count is used (`passes=N` sets it, 1–3): after
//! each pass but the last, its decodes are subtracted from the audio and
//! the residual decoded again; the output lists every decode once, in the
//! order found, with the pass that found it. `--profile` adds per-pass keys
//! when more than one pass is configured.
//! Exit status 0 on success (with or without decodes), 1 on an error, 2 on
//! a usage error.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufReader, Write as _};
use std::ops::ControlFlow;
use std::process::ExitCode;
use std::time::Instant;

use cat_ft8::decode::{
    Band, Candidate, CandidateScratch, Decode, DecodeConfig, DecodeList, Decoder, FrontEnd,
    PassEvent, PassReport, PassStep, PassesReport, Stage, CANDIDATES, HOST_DECODES, SMALL_DECODES,
    TABLE_BYTES,
};
use cat_ft8::message::CallTable;
use cat_ft8::{wav, SAMPLE_RATE, SLOT_SAMPLES};

const USAGE: &str =
    "usage: cat-ft8-decode FILE.wav [--config host|small[,passes=N]] [--raw] [--profile]";

/// Calls remembered for hashed-call messages within the file.
const CALLS: usize = 64;

/// A configuration as the command uses it: decoder settings, band and
/// decode capacity.
#[derive(Clone, Copy)]
struct Setup {
    name: &'static str,
    config: DecodeConfig,
    band: Band,
    decodes: usize,
}

/// The host configuration: 100–3100 Hz, sum-product, 128 decodes.
const HOST: Setup = Setup {
    name: "host",
    config: DecodeConfig::HOST,
    band: Band::DEFAULT,
    decodes: HOST_DECODES,
};

/// The small-target configuration: 200–2900 Hz, min-sum, 64 decodes.
const SMALL: Setup = Setup {
    name: "small",
    config: DecodeConfig::SMALL,
    band: Band::SMALL,
    decodes: SMALL_DECODES,
};

struct Args {
    path: String,
    setup: Setup,
    raw: bool,
    profile: bool,
}

/// What the command line asks for.
enum Parsed {
    Run(Args),
    Help,
}

fn parse() -> Result<Parsed, String> {
    let mut it = std::env::args().skip(1);
    let mut a = Args {
        path: String::new(),
        setup: HOST,
        raw: false,
        profile: false,
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--config" => a.setup = parse_config(it.next().as_deref())?,
            "--raw" => a.raw = true,
            "--profile" => a.profile = true,
            "-h" | "--help" => return Ok(Parsed::Help),
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n{USAGE}")),
            s if a.path.is_empty() => a.path = s.to_string(),
            s => return Err(format!("unexpected argument {s}\n{USAGE}")),
        }
    }
    if a.path.is_empty() {
        return Err(USAGE.into());
    }
    Ok(Parsed::Run(a))
}

/// `host` or `small`, optionally followed by `,passes=N` (1–3).
fn parse_config(spec: Option<&str>) -> Result<Setup, String> {
    let bad = || format!("--config needs host or small[,passes=N], not {spec:?}\n{USAGE}");
    let mut parts = spec.ok_or_else(bad)?.split(',');
    let mut setup = match parts.next() {
        Some("host") => HOST,
        Some("small") => SMALL,
        _ => return Err(bad()),
    };
    for kv in parts {
        match kv.split_once('=') {
            Some(("passes", n)) => {
                let n: u8 = n.parse().map_err(|_| bad())?;
                if !(1..=DecodeConfig::MAX_PASSES).contains(&n) {
                    return Err(format!(
                        "passes must be 1 to {}, not {n}\n{USAGE}",
                        DecodeConfig::MAX_PASSES
                    ));
                }
                setup.config = setup.config.with_passes(n);
            }
            _ => return Err(bad()),
        }
    }
    Ok(setup)
}

/// `s` as a JSON string literal.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `x` with `decimals` places, as a JSON number (never `-0`; `null` if not
/// finite).
fn json_number(x: f64, decimals: usize) -> String {
    if !x.is_finite() {
        return "null".into();
    }
    let s = format!("{x:.decimals$}");
    if s.trim_start_matches('-')
        .chars()
        .all(|c| c == '0' || c == '.')
    {
        s.trim_start_matches('-').to_string()
    } else {
        s
    }
}

/// The fields every decode line starts with.
fn common_fields(d: &Decode) -> String {
    format!(
        "\"freq\": {}, \"dt\": {}, \"snr\": {}",
        json_number(f64::from(d.freq_hz), 1),
        json_number(f64::from(d.dt), 2),
        json_number(f64::from(d.snr_db).round(), 0),
    )
}

/// The fields every decode line ends with; `pass` is the pass that found
/// it (from 1).
fn tail_fields(d: &Decode, pass: u8) -> String {
    format!(
        "\"pass\": {pass}, \"candidate\": {}, \"hard_errors\": {}, \"rounds\": {}, \"score\": {}",
        d.candidate, d.hard_errors, d.rounds, d.score
    )
}

/// One line for an unpacked decode.
fn decode_line(d: &Decode, pass: u8, text: &str) -> String {
    format!(
        "{{{}, \"message\": {}, {}}}",
        common_fields(d),
        json_string(text),
        tail_fields(d, pass)
    )
}

/// One `--raw` line for a decode whose payload did not unpack.
fn raw_line(d: &Decode, pass: u8, error: &str) -> String {
    let hex: String = d
        .payload
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let (i3, n3) = (d.payload.i3(), d.payload.n3());
    let kind = if i3 == 0 {
        format!("0.{n3}")
    } else {
        format!("{i3}")
    };
    format!(
        "{{{}, \"payload\": \"{hex}\", \"type\": \"{kind}\", \"unpacked\": false, \"error\": {}, {}}}",
        common_fields(d),
        json_string(error),
        tail_fields(d, pass)
    )
}

/// The steps of a pass as the profile names them, in the order of
/// [`Times::steps`].
const STEPS: [&str; 4] = ["subtract", "feed", "sync", "candidates"];

/// Most passes.
const PASSES: usize = DecodeConfig::MAX_PASSES as usize;

/// Seconds booked to each step of each pass and to each candidate stage
/// (over every pass).
#[derive(Default)]
struct Times {
    /// Per pass (from pass 1), per step of [`STEPS`].
    steps: [[f64; STEPS.len()]; PASSES],
    /// Per [`Stage`], by [`Stage::index`].
    stages: [f64; Stage::ALL.len()],
}

impl Times {
    /// Books `t` seconds to `step` of pass `pass`.
    fn add(&mut self, pass: u8, step: PassStep, t: f64) {
        let k = match step {
            PassStep::Subtract => 0,
            PassStep::Feed => 1,
            PassStep::Sync => 2,
            PassStep::Candidate(stage) => {
                self.stages[stage.index()] += t;
                3
            }
        };
        self.steps[usize::from(pass) - 1][k] += t;
    }

    /// Step `k`'s seconds over every pass.
    fn step(&self, k: usize) -> f64 {
        self.steps.iter().map(|p| p[k]).sum()
    }
}

/// The `--profile` line: per-pass keys follow the others when more than one
/// pass is configured. The subtractions between passes take their working
/// storage from the spectrogram's buffer, so `peak_bytes` covers them.
fn profile_line(
    setup: &Setup,
    seconds: &[(String, f64)],
    slot: &PassesReport,
    passes: &[PassReport],
) -> String {
    let band = setup.band;
    let memory = [
        ("front_end_bytes", std::mem::size_of::<FrontEnd>()),
        ("spectrogram_bytes", band.spectrogram_len()),
        ("sync_scratch_bytes", Decoder::sync_scratch_len(band) * 4),
        (
            "candidates_bytes",
            CANDIDATES * std::mem::size_of::<Candidate>(),
        ),
        (
            "candidate_scratch_bytes",
            std::mem::size_of::<CandidateScratch>(),
        ),
        (
            "decode_list_bytes",
            setup.decodes * std::mem::size_of::<Decode>(),
        ),
        ("call_table_bytes", std::mem::size_of::<CallTable<CALLS>>()),
    ];
    let mut peak: usize = memory.iter().map(|m| m.1).sum();
    let multi = setup.config.passes > 1;
    // The slot's samples as `f32`, overwritten with the residual.
    let residual = SLOT_SAMPLES * std::mem::size_of::<f32>();
    if multi {
        peak += residual;
    }
    let (lo, hi) = band.range_hz();
    let mut out = format!(
        "{{\"profile\": {{\"format\": 0, \"config\": {}, \"band_lo_hz\": {}, \"band_hi_hz\": {}, \"decode_capacity\": {}",
        json_string(setup.name),
        json_number(f64::from(lo), 1),
        json_number(f64::from(hi), 1),
        setup.decodes
    );
    for (k, v) in seconds {
        let _ = write!(out, ", \"{k}_seconds\": {}", json_number(*v, 6));
    }
    for (k, v) in memory {
        let _ = write!(out, ", \"{k}\": {v}");
    }
    if multi {
        let _ = write!(out, ", \"residual_bytes\": {residual}");
    }
    let _ = write!(
        out,
        ", \"peak_bytes\": {peak}, \"static_table_bytes\": {TABLE_BYTES}"
    );
    let r = &slot.report;
    for (k, v) in [
        ("candidates_found", r.candidates_found),
        ("candidates_kept", r.candidates_kept),
        ("candidates_dropped", r.candidates_dropped),
        ("decoded", r.decoded),
        ("decodes_truncated", r.decodes_truncated),
        ("duplicates", r.duplicates),
        ("all_zero_rejected", r.all_zero_rejected),
        ("crc_failed", r.crc_failed),
        ("not_converged", r.not_converged),
        ("rows_filled", r.rows_filled),
        ("sanitised_samples", r.sanitised_samples),
        ("clamped_samples", r.clamped_samples),
        ("skipped_samples", r.skipped_samples),
        ("floored_levels", r.floored_levels),
        ("top_levels", r.top_levels),
    ] {
        let _ = write!(out, ", \"{k}\": {v}");
    }
    if multi {
        let _ = write!(
            out,
            ", \"passes\": {}, \"passes_run\": {}, \"passes_end\": {}",
            setup.config.passes,
            slot.passes_run,
            json_string(slot.end.name())
        );
        for p in passes {
            let r = &p.report;
            for (k, v) in [
                ("subtracted", p.subtracted),
                ("candidates_found", r.candidates_found),
                ("candidates_kept", r.candidates_kept),
                ("candidates_dropped", r.candidates_dropped),
                ("decoded", r.decoded),
                ("decodes_truncated", r.decodes_truncated),
                ("duplicates", r.duplicates),
                ("earlier_pass_duplicates", p.earlier_duplicates),
                ("all_zero_rejected", r.all_zero_rejected),
                ("crc_failed", r.crc_failed),
                ("not_converged", r.not_converged),
                ("bp_rounds", p.bp_rounds),
            ] {
                let _ = write!(out, ", \"pass{}_{k}\": {v}", p.pass);
            }
        }
    }
    out.push_str("}}");
    out
}

fn run(a: &Args) -> Result<(), String> {
    let t_read = Instant::now();
    let file = File::open(&a.path).map_err(|e| format!("{}: {e}", a.path))?;
    // Only the first slot's samples are read.
    let w = wav::read_wav_prefix(BufReader::new(file), SLOT_SAMPLES)
        .map_err(|e| format!("{}: {e}", a.path))?;
    if w.sample_rate != SAMPLE_RATE {
        return Err(format!(
            "{}: sample rate {} Hz; cat-ft8-decode needs {SAMPLE_RATE} Hz (resample first)",
            a.path, w.sample_rate
        ));
    }
    if w.channels != 1 {
        return Err(format!(
            "{}: {} channels; cat-ft8-decode needs mono",
            a.path, w.channels
        ));
    }
    let read = t_read.elapsed().as_secs_f64();

    let band = a.setup.band;
    let mut front = Box::new(FrontEnd::new());
    let mut levels = vec![0u8; band.spectrogram_len()];
    let mut sync = vec![0i32; Decoder::sync_scratch_len(band)];
    let mut candidates = vec![Candidate::default(); CANDIDATES];
    let mut decoder = Decoder::new(
        &mut front,
        &mut levels,
        &mut sync,
        &mut candidates,
        band,
        a.setup.config,
    )
    .map_err(|e| e.to_string())?;
    let mut scratch = Box::new(CandidateScratch::new());
    let mut entries = vec![Decode::EMPTY; a.setup.decodes];
    let mut list = DecodeList::new(&mut entries);

    // Every pass, timed step by step (`PassStep`); one pass is `feed` and
    // `decode`. The samples as `f32` (exactly what `feed_i16` takes), which
    // further passes overwrite with the residual.
    let mut audio: Vec<f32> = w
        .samples
        .iter()
        .map(|&x| f32::from(x) * (1.0 / 32_768.0))
        .collect();
    let mut times = Times::default();
    let mut passes: Vec<PassReport> = Vec::new();
    let mut last = Instant::now();
    let slot = decoder.decode_passes_staged(&mut audio, 0, &mut scratch, &mut list, |e| {
        let now = Instant::now();
        match e {
            PassEvent::Step { pass, step } => times.add(pass, step, (now - last).as_secs_f64()),
            PassEvent::Done(r) => passes.push(*r),
            _ => {}
        }
        last = Instant::now();
        ControlFlow::Continue(())
    });
    let t = Instant::now();
    let mut lines = Vec::new();
    let mut table: CallTable<CALLS> = CallTable::new();
    // The pass of list entry `i`: the last pass that began at or before it.
    let pass_of = |i: usize| {
        passes
            .iter()
            .rev()
            .find(|p| p.first <= i)
            .map_or(1, |p| p.pass)
    };
    let mut i = 0;
    list.finish(&mut table, |d, text| {
        let pass = pass_of(i);
        i += 1;
        match text {
            Ok(text) => lines.push(decode_line(d, pass, text)),
            Err(e) if a.raw => lines.push(raw_line(d, pass, &e.to_string())),
            Err(_) => {}
        }
    });
    let finish = t.elapsed().as_secs_f64();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in &lines {
        writeln!(out, "{line}").map_err(|e| e.to_string())?;
    }
    if a.profile {
        // The fixed keys first, in the order of format version 0; then
        // `subtract` and the per-pass keys.
        let [subtract, feed, search, cands] = [0, 1, 2, 3].map(|k| times.step(k));
        let mut keys: Vec<(String, f64)> = vec![
            ("read".into(), read),
            ("feed".into(), feed),
            ("sync".into(), search),
            ("candidates".into(), cands),
        ];
        // The candidate stage split up; these sum to about `candidates`.
        for stage in Stage::ALL {
            keys.push((stage.name().into(), times.stages[stage.index()]));
        }
        keys.push(("finish".into(), finish));
        keys.push(("total".into(), feed + search + cands + subtract + finish));
        if a.setup.config.passes > 1 {
            keys.push(("subtract".into(), subtract));
            for p in 1..=passes.len() {
                for (k, step) in STEPS.iter().enumerate() {
                    if p > 1 || k > 0 {
                        keys.push((format!("pass{p}_{step}"), times.steps[p - 1][k]));
                    }
                }
            }
        }
        writeln!(out, "{}", profile_line(&a.setup, &keys, &slot, &passes))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse() {
        Ok(Parsed::Run(a)) => a,
        Ok(Parsed::Help) => {
            println!("{USAGE}");
            println!("  --config host   100–3100 Hz, sum-product, 128 decodes (the default)");
            println!("  --config small  200–2900 Hz, min-sum, 64 decodes");
            println!("  ,passes=N       after either: decode in N passes (1–3), subtracting each pass's decodes");
            println!("  --raw           also print decodes whose payload did not unpack");
            println!("  --profile       add one line of timings, memory and counts");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cat-ft8-decode: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_are_escaped() {
        assert_eq!(json_string("CQ K1ABC FN42"), "\"CQ K1ABC FN42\"");
        assert_eq!(json_string("a\"b\\c\nd\u{1}"), "\"a\\\"b\\\\c\\nd\\u0001\"");
        let parsed: String = serde_json::from_str(&json_string("<...> \"x\" \t\u{7f}")).unwrap();
        assert_eq!(parsed, "<...> \"x\" \t\u{7f}");
    }

    #[test]
    fn numbers_never_print_negative_zero() {
        assert_eq!(json_number(-0.001, 2), "0.00");
        assert_eq!(json_number(-0.4, 0), "0");
        assert_eq!(json_number(-12.6, 0), "-13");
        assert_eq!(json_number(1500.04, 1), "1500.0");
        assert_eq!(json_number(f64::NAN, 1), "null");
    }
}
