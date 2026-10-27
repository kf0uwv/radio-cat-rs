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

//! `cat-ft8-decode FILE.wav [--config host|small] [--raw] [--profile]`
//!
//! Decodes one FT8 slot from a 12 kHz mono 16-bit WAV file and prints one
//! JSON object per decode, in output format version 0 (see the crate docs,
//! "Decode command output"). The file's first 15 s are the slot: a shorter
//! file is decoded as a partial slot, and anything after 15 s is ignored.
//! Exit status 0 on success (with or without decodes), 1 on an error, 2 on
//! a usage error.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufReader, Write as _};
use std::process::ExitCode;
use std::time::Instant;

use cat_ft8::decode::{
    Band, Candidate, CandidateScratch, Decode, DecodeConfig, DecodeList, DecodeReport, Decoder,
    FrontEnd, CANDIDATES, HOST_DECODES, TABLE_BYTES,
};
use cat_ft8::message::CallTable;
use cat_ft8::{wav, SAMPLE_RATE, SLOT_SAMPLES};

const USAGE: &str = "usage: cat-ft8-decode FILE.wav [--config host|small] [--raw] [--profile]";

/// Calls remembered for hashed-call messages within the file.
const CALLS: usize = 64;

struct Args {
    path: String,
    config_name: &'static str,
    config: DecodeConfig,
    raw: bool,
    profile: bool,
}

fn parse() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut a = Args {
        path: String::new(),
        config_name: "host",
        config: DecodeConfig::HOST,
        raw: false,
        profile: false,
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--config" => match it.next().as_deref() {
                Some("host") => (a.config_name, a.config) = ("host", DecodeConfig::HOST),
                Some("small") => (a.config_name, a.config) = ("small", DecodeConfig::SMALL),
                other => {
                    return Err(format!(
                        "--config needs host or small, not {other:?}\n{USAGE}"
                    ))
                }
            },
            "--raw" => a.raw = true,
            "--profile" => a.profile = true,
            "-h" | "--help" => return Err(USAGE.into()),
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n{USAGE}")),
            s if a.path.is_empty() => a.path = s.to_string(),
            s => return Err(format!("unexpected argument {s}\n{USAGE}")),
        }
    }
    if a.path.is_empty() {
        return Err(USAGE.into());
    }
    Ok(a)
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

/// The fields every decode line ends with.
fn tail_fields(d: &Decode) -> String {
    format!(
        "\"candidate\": {}, \"hard_errors\": {}, \"rounds\": {}, \"score\": {}",
        d.candidate, d.hard_errors, d.rounds, d.score
    )
}

/// One line for an unpacked decode.
fn decode_line(d: &Decode, text: &str) -> String {
    format!(
        "{{{}, \"message\": {}, {}}}",
        common_fields(d),
        json_string(text),
        tail_fields(d)
    )
}

/// One `--raw` line for a decode whose payload did not unpack.
fn raw_line(d: &Decode, error: &str) -> String {
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
        tail_fields(d)
    )
}

/// The `--profile` line.
fn profile_line(
    config: &str,
    band: Band,
    seconds: &[(&str, f64)],
    report: &DecodeReport,
) -> String {
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
            HOST_DECODES * std::mem::size_of::<Decode>(),
        ),
        ("call_table_bytes", std::mem::size_of::<CallTable<CALLS>>()),
    ];
    let peak: usize = memory.iter().map(|m| m.1).sum();
    let mut out = format!(
        "{{\"profile\": {{\"format\": 0, \"config\": {}",
        json_string(config)
    );
    for (k, v) in seconds {
        let _ = write!(out, ", \"{k}_seconds\": {}", json_number(*v, 6));
    }
    for (k, v) in memory {
        let _ = write!(out, ", \"{k}\": {v}");
    }
    let _ = write!(
        out,
        ", \"peak_bytes\": {peak}, \"static_table_bytes\": {TABLE_BYTES}"
    );
    let r = report;
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
    out.push_str("}}");
    out
}

fn run(a: &Args) -> Result<(), String> {
    let t_read = Instant::now();
    let file = File::open(&a.path).map_err(|e| format!("{}: {e}", a.path))?;
    let w = wav::read_wav(BufReader::new(file)).map_err(|e| format!("{}: {e}", a.path))?;
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

    let band = Band::DEFAULT;
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
        a.config,
    )
    .map_err(|e| e.to_string())?;
    let mut scratch = Box::new(CandidateScratch::new());
    let mut entries = vec![Decode::EMPTY; HOST_DECODES];
    let mut list = DecodeList::new(&mut entries);

    // The split API, to time each stage; the same result as `decode()`.
    let t = Instant::now();
    for block in w.samples[..w.samples.len().min(SLOT_SAMPLES)].chunks(1_920) {
        decoder.feed_i16(block);
    }
    let feed = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let kept = decoder.find_candidates().kept;
    let search = t.elapsed().as_secs_f64();
    let t = Instant::now();
    for i in 0..kept {
        if let Some(result) = decoder.decode_candidate(i, &mut scratch) {
            list.push(i, &result).map_err(|e| e.to_string())?;
        }
    }
    let candidates_time = t.elapsed().as_secs_f64();
    let report = decoder.report(&list);

    let t = Instant::now();
    let mut lines = Vec::new();
    let mut table: CallTable<CALLS> = CallTable::new();
    list.finish(&mut table, |d, text| match text {
        Ok(text) => lines.push(decode_line(d, text)),
        Err(e) if a.raw => lines.push(raw_line(d, &e.to_string())),
        Err(_) => {}
    });
    let finish = t.elapsed().as_secs_f64();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in &lines {
        writeln!(out, "{line}").map_err(|e| e.to_string())?;
    }
    if a.profile {
        let seconds = [
            ("read", read),
            ("feed", feed),
            ("sync", search),
            ("candidates", candidates_time),
            ("finish", finish),
            ("total", feed + search + candidates_time + finish),
        ];
        writeln!(
            out,
            "{}",
            profile_line(a.config_name, band, &seconds, &report)
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse() {
        Ok(a) => a,
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
