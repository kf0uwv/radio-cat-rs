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
//! cargo run --release -p cat-ft8 --features profile --example paired_bench -- slotcost CONFIG_A CONFIG_B [SLOTS] [RUNS]
//! cargo run --release -p cat-ft8 --features profile --example paired_bench -- ceiling CONFIG [BLOCK] [SET]
//! cargo run --release -p cat-ft8 --features profile --example paired_bench -- suppression CONFIG [GRID] [TRIALS]
//! cargo run --release -p cat-ft8 --features profile --example paired_bench -- passes CONFIG [BLOCK] [SET] [GRID]
//! cargo run --release -p cat-ft8 --features profile --example paired_bench -- subcost CONFIG [SUB] [SLOTS] [RUNS]
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
//! - `wide`: crowded slots over a wider SNR range, −22…+5 dB, so that some
//!   messages are near or below the single-signal decode threshold: 10,
//!   20, 30 and 40 distinct messages per slot (same offsets, overlaps
//!   allowed), 200 slots per size; its own seeds (not in the default
//!   SETS);
//! - `fading`: crowded slots (10, 20 and 40 messages, −16…+6 dB, the same
//!   offsets) whose signals fade, drift and start between samples: each
//!   has a sinusoidal amplitude fade at 0.05–1 Hz with a depth of 0–0.9
//!   (the SNR is the unfaded peak's), a linear drift of −0.2…+0.2 Hz/s
//!   (zero at the middle) and a start at exactly (0.5 + dt)·12 000
//!   samples, made by the bench's own `f64` generator (`channel.rs`);
//!   its own seeds (not in the default SETS);
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
//! `rounds=N`, `scale=X`, `candidates=N`, `min_score=N`, and for further
//! passes `passes=N` (1–3), `later=N` (candidates in every pass after the
//! first), `seg=N` and `drift=0|1` (the subtraction). Examples:
//! `host,candidates=100`, `host,passes=3,later=150,seg=2880`. With more
//! than one pass, `run` decodes with `decode_passes` and saves the slot's
//! totals over every pass as its report, and each pass's own counts
//! (single-pass records are unchanged).
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
//! **slotcost** is the multi-pass cost statistic: every 40-message slot of
//! the crowded set's main block (SLOTS, default and at most 200), each
//! decoded by A and B interleaved, one thread, RUNS times (at least 5);
//! per slot the median of its runs' total; it prints the largest and the
//! mean of those medians, each step's mean (summed over passes and per
//! pass), and the work per pass (candidates, rounds, subtractions).
//!
//! **Misses by cause.** `run` also saves, per transmitted message, its
//! outcome: decoded (its payload is in the output) or, judged by position,
//! why not. A candidate is on a message when its refined tone 0 is within
//! 4 Hz and its refined start within 0.12 s; candidates that decoded some
//! payload are left out, and the furthest any candidate got decides: a
//! codeword failing the CRC, the all-zero codeword, another failure, no
//! codeword within the round limit. With no candidate on it, the sync
//! search is repeated without the candidate limit (outside the timed
//! calls): the message had a sync peak on it that the limit dropped, or
//! no sync peak at all. `compare` prints, for each side, the misses by
//! cause ("not recorded" for runs saved before outcomes were; older runs
//! that did not split the no-candidate cause show it undivided), the CRC
//! failures per slot (summed over passes) and the expected false decodes
//! they imply (a
//! codeword that was not transmitted passes the 14-bit CRC with
//! probability 2⁻¹⁴, so the estimate is the CRC failures per slot × 2⁻¹⁴).
//!
//! **ceiling** measures what subtraction could gain on a crowded set (SET:
//! `crowded`, the default, `wide` or `fading`) of BLOCK (default `main`), without changing the
//! decoder. Each transmitted message is also decoded alone, in a slot with
//! the same noise and the other signals left out. The oracle passes then
//! decode the slot rebuilt without every message decoded so far (true
//! waveforms, same noise), twice, with the later passes' candidate count
//! (CONFIG's `later`): a same-configuration reference for what
//! further passes could gain with perfect reference waveforms (not a
//! bound: a real pass decodes different input). The "ceiling" column
//! counts a message decoded by the single pass, alone or by an oracle
//! pass. It prints the decoded counts per size, false decodes and oracle
//! duplicates, the single-pass misses by cause and what recovers them,
//! and the counts by SNR relative to the single-pass 50% point of the
//! sweep set in the same block (computed first).
//!
//! **SUB** is a subtraction setting for `decode::subtract`: empty for the
//! default, or `seg=N` (samples per amplitude segment, a multiple of 240
//! from 960 to the waveform's 151 680) and/or `drift=0|1`, comma-separated.
//! **GRID** is a set of them: each key may list values separated by `|`,
//! and every combination is run (the first key varies slowest), so
//! `seg=1920|3840,drift=1|0` is four settings; several grids may be joined
//! with `;` (`seg=960|1920;seg=1920,drift=0` is three).
//!
//! **suppression** measures how much of one decoded signal `subtract`
//! removes, over TRIALS (default 100) seeded slots per condition: SNR
//! (−20…+20 dB), start offset (−2…+2.5 s, and starts a quarter and half a
//! sample off the grid), frequency (band edges, fractions of a bin), fade
//! rate (depth 0.9) and drift. Each signal comes from the bench's own
//! generator and is decoded by the single pass first; the decode is what
//! is subtracted. It reports the signal left (residual less noise, over
//! the signal's energy) with every estimate made, with the true start
//! imposed, on the noiseless signal, and on the noiseless signal with the
//! true start (rounded to a sample), frequency and drift imposed (the
//! whole-sample start's share), and the fine estimates' errors beside the
//! decoder's, all with GRID's first setting; then the signal left with
//! every setting of GRID (default `seg=1920|960|3840|7680|151680,drift=1|0`).
//! Input cut at 13.5 s and starting 1 s late are conditions too. It fails
//! if the bench's generator differs from `Synth` by more than 10⁻³.
//!
//! **passes** runs `Decoder::decode_passes` with three passes on a crowded
//! set (SET `crowded`, `wide` or `fading`; BLOCK as above) for every
//! setting of GRID (keys `seg`, `drift` and `later`, the later passes'
//! candidate count; default: CONFIG's own); the single pass and the oracle
//! passes (at each later count) are run once for all settings. It reports
//! the decodes gained by passes 2 and 3 beside the oracle passes (and the
//! share of the oracle's gain recovered), earlier payloads decoded again
//! (duplicates: the main sign of a poor subtraction), false decodes and
//! list truncation per pass, and the cost.
//!
//! **subcost** times `subtract` on one thread: the single-pass decodes of
//! SLOTS (default 20) 40-message crowded slots subtracted in list order,
//! RUNS times (at least 5); the median per call.

#[path = "../../tests/common/mod.rs"]
mod common;

mod ceiling;
mod channel;
mod compare;
mod cost;
mod record;
mod sets;
mod subtraction;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cat_ft8::decode::{Band, CheckRule, DecodeConfig, HOST_DECODES, SMALL_DECODES};

// --------------------------------------------------------------- config

/// A decoder setup.
#[derive(Clone, Copy)]
pub struct Setup {
    pub band: Band,
    pub config: DecodeConfig,
    pub decodes: usize,
}

impl Setup {
    /// The same setup decoding one pass.
    pub fn single(&self) -> Setup {
        Setup {
            config: self.config.with_passes(1),
            ..*self
        }
    }

    /// One pass with the later passes' candidate count: what an oracle
    /// pass after the first decodes with.
    pub fn later(&self) -> Setup {
        Setup {
            config: self
                .config
                .with_passes(1)
                .with_candidates(self.config.later_candidates),
            ..*self
        }
    }
}

pub fn parse_config(spec: &str) -> Result<Setup, String> {
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
            "passes" => s.config.with_passes(num(v)? as u8),
            "later" => s.config.with_later_candidates(num(v)? as usize),
            "seg" | "drift" => s
                .config
                .with_subtract(subtraction::merge_subtract(s.config.subtract, kv)?),
            _ => return Err(format!("unknown setting {k}")),
        };
    }
    Ok(s)
}

// ----------------------------------------------------------- provenance

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
pub fn provenance() -> String {
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

// ----------------------------------------------------------------- main

const USAGE: &str = "usage: paired_bench run CONFIG OUTDIR [SETS] [BLOCK] | compare OLDDIR NEWDIR [--partial] | cost A B [SLOTS] [RUNS] | slotcost A B [SLOTS] [RUNS] | ceiling CONFIG [BLOCK] [SET] | suppression CONFIG [GRID] [TRIALS] | passes CONFIG [BLOCK] [SET] [GRID] | subcost CONFIG [SUB] [SLOTS] [RUNS]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    let num =
        |i: usize, d: usize| arg(i).map_or(Ok(d), |s| s.parse().map_err(|e| format!("{s}: {e}")));
    let result = match (arg(0), arg(1), arg(2)) {
        (Some("run"), Some(c), Some(o)) => record::run(
            c,
            &PathBuf::from(o),
            arg(3).unwrap_or("sweep,crowded,edges,empty"),
            arg(4).unwrap_or("main"),
        )
        .map(|()| true),
        (Some("compare"), Some(o), Some(n)) => {
            compare::compare(Path::new(o), Path::new(n), arg(3) == Some("--partial"))
        }
        (Some("ceiling"), Some(c), _) => {
            ceiling::ceiling(c, arg(2).unwrap_or("main"), arg(3).unwrap_or("crowded"))
                .map(|()| true)
        }
        (Some("suppression"), Some(c), _) => num(3, 100)
            .and_then(|n| subtraction::suppression(c, arg(2).unwrap_or(""), n))
            .map(|()| true),
        (Some("passes"), Some(c), _) => subtraction::passes(
            c,
            arg(2).unwrap_or("main"),
            arg(3).unwrap_or("crowded"),
            arg(4).unwrap_or(""),
        )
        .map(|()| true),
        (Some("subcost"), Some(c), _) => num(3, 20)
            .and_then(|s| {
                num(4, 5).and_then(|r| subtraction::subtract_cost(c, arg(2).unwrap_or(""), s, r))
            })
            .map(|()| true),
        (Some("slotcost"), Some(x), Some(y)) => num(3, 200)
            .and_then(|s| num(4, 5).and_then(|r| cost::slot_cost(x, y, s, r)))
            .map(|()| true),
        (Some("cost"), Some(x), Some(y)) => num(3, 42)
            .and_then(|s| num(4, 5).and_then(|r| cost::cost(x, y, s, r)))
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
