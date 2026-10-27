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

//! The `cat-ft8-decode` command: output format version 0 parsed with
//! `serde_json`, the `--raw`, `--profile` and `--config` options, files
//! shorter and longer than a slot, and the errors for files it cannot use.

#![cfg(feature = "std")]

mod common;

use std::path::PathBuf;
use std::process::{Command, Output};

use cat_ft8::message::Payload;
use cat_ft8::waveform::{synthesize, Placement};
use cat_ft8::{encode_payload, wav, SLOT_SAMPLES};
use common::*;
use serde_json::Value;

/// A file in the temporary directory, unique to this process and `name`.
fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("cat-ft8-decode-{}-{name}", std::process::id()))
}

/// Writes `samples` as a 16-bit WAV at `rate`.
fn write(name: &str, samples: &[f32], rate: u32) -> PathBuf {
    let path = temp(name);
    let file = std::fs::File::create(&path).unwrap();
    wav::write_wav(std::io::BufWriter::new(file), samples, rate).unwrap();
    path
}

fn decode_cmd(path: &PathBuf, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cat-ft8-decode"))
        .arg(path)
        .args(args)
        .output()
        .unwrap()
}

/// stdout as parsed JSON lines (every line must parse).
fn lines(out: &Output) -> Vec<Value> {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout.clone())
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{l}: {e}")))
        .collect()
}

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

const LINE_KEYS: [&str; 8] = [
    "candidate",
    "dt",
    "freq",
    "hard_errors",
    "message",
    "rounds",
    "score",
    "snr",
];

#[test]
fn decodes_print_one_json_object_each() {
    let (s, _) = slot(
        &[
            Signal {
                message: "CQ K1ABC FN42",
                base_hz: 1_000.0,
                dt: 0.0,
                snr_db: -10.0,
            },
            Signal {
                message: "K1ABC W9XYZ -11",
                base_hz: 1_734.4,
                dt: 0.42,
                snr_db: -14.0,
            },
        ],
        5,
    );
    let path = write("two.wav", &s, 12_000);
    for config in [
        &[][..],
        &["--config", "host"][..],
        &["--config", "small"][..],
    ] {
        let mut got = lines(&decode_cmd(&path, config));
        got.sort_by(|a, b| a["freq"].as_f64().partial_cmp(&b["freq"].as_f64()).unwrap());
        assert_eq!(got.len(), 2, "{config:?}");
        for (v, (msg, f, dt, snr)) in got.iter().zip([
            ("CQ K1ABC FN42", 1_000.0, 0.0, -10.0),
            ("K1ABC W9XYZ -11", 1_734.4, 0.42, -14.0),
        ]) {
            assert_eq!(keys(v), LINE_KEYS, "{v}");
            assert_eq!(v["message"], msg);
            assert!((v["freq"].as_f64().unwrap() - f).abs() <= 1.0, "{v}");
            assert!((v["dt"].as_f64().unwrap() - dt).abs() <= 0.03, "{v}");
            assert!((v["snr"].as_f64().unwrap() - snr).abs() <= 2.0, "{v}");
            assert!(v["snr"].as_f64().unwrap().fract() == 0.0, "{v}");
            assert!(
                v["candidate"].is_u64()
                    && v["rounds"].is_u64()
                    && v["hard_errors"].is_u64()
                    && v["score"].is_i64()
            );
        }
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn raw_adds_payloads_that_do_not_unpack() {
    // A valid payload of type 0.3 (ARRL Field Day), which this release
    // does not unpack, beside an ordinary message.
    let mut s = vec![0.0f32; SLOT_SAMPLES];
    let field_day = Payload::from_bits((0x1234_5678u128 << 20) | (3 << 3)).unwrap();
    let mut p = Placement::slot(1_200.0, 0.1).unwrap();
    p.amplitude = amplitude(-8.0, NOISE_RMS);
    synthesize(&encode_payload(&field_day), &p, &mut s).unwrap();
    add(
        &mut s,
        &Signal {
            message: "CQ K1ABC FN42",
            base_hz: 2_000.0,
            dt: 0.0,
            snr_db: -8.0,
        },
    );
    add_noise(&mut s, NOISE_RMS, 8);
    let path = write("raw.wav", &s, 12_000);

    let plain = lines(&decode_cmd(&path, &[]));
    assert_eq!(plain.len(), 1);
    assert_eq!(plain[0]["message"], "CQ K1ABC FN42");

    let raw = lines(&decode_cmd(&path, &["--raw"]));
    assert_eq!(raw.len(), 2);
    let r = raw
        .iter()
        .find(|v| v.get("unpacked").is_some())
        .expect("raw line");
    assert_eq!(r["unpacked"], false);
    assert_eq!(r["type"], "0.3");
    let hex: String = field_day
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(r["payload"], hex.as_str());
    assert!(
        r["error"].as_str().unwrap().contains("not supported"),
        "{r}"
    );
    assert!((r["freq"].as_f64().unwrap() - 1_200.0).abs() <= 1.0);
    assert!(r.get("message").is_none());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn profile_adds_one_line_of_timings_memory_and_counts() {
    let (s, _) = slot(
        &[Signal {
            message: "CQ K1ABC FN42",
            base_hz: 1_500.0,
            dt: 0.0,
            snr_db: -10.0,
        }],
        6,
    );
    let path = write("profile.wav", &s, 12_000);
    let got = lines(&decode_cmd(&path, &["--profile", "--config", "small"]));
    assert_eq!(got.len(), 2);
    assert_eq!(got[0]["message"], "CQ K1ABC FN42");
    let p = &got[1]["profile"];
    assert_eq!(keys(&got[1]), ["profile"]);
    assert_eq!(p["format"], 0);
    assert_eq!(p["config"], "small");
    for stage in ["read", "feed", "sync", "candidates", "finish", "total"] {
        assert!(
            p[format!("{stage}_seconds")].as_f64().unwrap() >= 0.0,
            "{stage}"
        );
    }
    let parts: u64 = p
        .as_object()
        .unwrap()
        .iter()
        .filter(|(k, _)| k.ends_with("_bytes") && *k != "peak_bytes" && *k != "static_table_bytes")
        .map(|(_, v)| v.as_u64().unwrap())
        .sum();
    assert_eq!(p["peak_bytes"].as_u64().unwrap(), parts);
    assert_eq!(p["spectrogram_bytes"], 179_584);
    for count in [
        "candidates_found",
        "candidates_kept",
        "candidates_dropped",
        "decoded",
        "decodes_truncated",
        "duplicates",
        "all_zero_rejected",
        "crc_failed",
        "not_converged",
        "rows_filled",
        "sanitised_samples",
        "clamped_samples",
        "skipped_samples",
        "floored_levels",
        "top_levels",
    ] {
        assert!(p[count].is_u64(), "{count}");
    }
    assert_eq!(p["decoded"], 1);
    assert_eq!(p["rows_filled"], 184);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn short_files_are_partial_slots_and_long_files_use_the_first_slot() {
    let first = Signal {
        message: "CQ K1ABC FN42",
        base_hz: 1_000.0,
        dt: 0.0,
        snr_db: -10.0,
    };
    let (s, _) = slot(&[first], 10);
    // 14 s: the signal (ending at 13.1 s) is all there.
    let short = write("short.wav", &s[..14 * 12_000], 12_000);
    let got = lines(&decode_cmd(&short, &["--profile"]));
    assert_eq!(got[0]["message"], "CQ K1ABC FN42");
    assert!(got[1]["profile"]["rows_filled"].as_u64().unwrap() < 184);
    // 30 s: a second signal in the second 15 s is not decoded.
    let mut long = s.clone();
    let mut second = vec![0.0f32; SLOT_SAMPLES];
    add(
        &mut second,
        &Signal {
            message: "K1ABC W9XYZ -11",
            base_hz: 1_800.0,
            dt: 0.0,
            snr_db: 0.0,
        },
    );
    long.extend_from_slice(&second);
    let long = write("long.wav", &long, 12_000);
    let got = lines(&decode_cmd(&long, &[]));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["message"], "CQ K1ABC FN42");
    std::fs::remove_file(short).unwrap();
    std::fs::remove_file(long).unwrap();
}

#[test]
fn unusable_files_and_arguments_are_errors() {
    let fail = |out: Output, code: i32, needle: &str| {
        assert_eq!(out.status.code(), Some(code));
        assert!(out.stdout.is_empty());
        let err = String::from_utf8(out.stderr).unwrap();
        assert!(err.contains(needle), "{err}");
    };
    let rate = write("rate.wav", &[0.0; 4_800], 48_000);
    fail(decode_cmd(&rate, &[]), 1, "needs 12000 Hz");
    // A stereo file, by hand.
    let mut stereo = Vec::new();
    stereo.extend_from_slice(b"RIFF");
    stereo.extend_from_slice(&44u32.to_le_bytes());
    stereo.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 1 | (2 << 16), 12_000, 48_000, 4 | (16 << 16)] {
        stereo.extend_from_slice(&v.to_le_bytes());
    }
    stereo.extend_from_slice(b"data");
    stereo.extend_from_slice(&8u32.to_le_bytes());
    stereo.extend_from_slice(&[0; 8]);
    let stereo_path = temp("stereo.wav");
    std::fs::write(&stereo_path, stereo).unwrap();
    fail(decode_cmd(&stereo_path, &[]), 1, "needs mono");
    let text = temp("text.wav");
    std::fs::write(&text, b"not audio").unwrap();
    fail(decode_cmd(&text, &[]), 1, "RIFF");
    fail(decode_cmd(&temp("missing.wav"), &[]), 1, "missing.wav");
    fail(decode_cmd(&rate, &["--config", "fast"]), 2, "host or small");
    fail(decode_cmd(&rate, &["--bogus"]), 2, "unknown option");
    for p in [rate, stereo_path, text] {
        std::fs::remove_file(p).unwrap();
    }
}
