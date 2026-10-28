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

//! `ceiling`: what subtraction could gain on a crowded set, without
//! changing the decoder.

use std::collections::BTreeMap;
use std::time::Instant;

use crate::common::par_map;
use crate::compare::{half_point_of, tenths, Sweep};
use crate::record::{decode_input, miss_counts, misses_text, Decoded, Outcome};
use crate::sets::{build, noise_seed, render, set_size, slot_seed, Sent};
use crate::{parse_config, provenance, Setup};

/// Oracle passes after the single pass (passes 2 and 3).
const ORACLE_PASSES: usize = 2;

/// The sets `ceiling` accepts.
pub const CEILING_SETS: [&str; 3] = ["crowded", "wide", "fading"];

/// One transmitted message of a crowded slot in the `ceiling` mode.
struct CeilingMessage {
    /// Messages in its slot, and its SNR.
    size: usize,
    snr: f64,
    /// Single pass on the crowded slot.
    single: Outcome,
    /// Single pass on the slot with only this message (same noise).
    alone: Outcome,
    /// The pass that first decoded it: 1 for the single pass, 2 and 3 for
    /// the oracle passes; `None` if none did.
    pass: Option<u8>,
}

impl CeilingMessage {
    /// Decoded by pass `k` or an earlier one.
    fn by_pass(&self, k: u8) -> bool {
        self.pass.is_some_and(|p| p <= k)
    }

    /// Decoded alone.
    fn alone(&self) -> bool {
        self.alone == Outcome::Decoded
    }

    /// Decoded by any pass or alone.
    fn ceiling(&self) -> bool {
        self.pass.is_some() || self.alone()
    }
}

/// Per-slot counts of the `ceiling` mode.
#[derive(Default, Clone, Copy)]
struct CeilingCounts {
    /// Decodes not transmitted: single pass, all the slot's alone decodes,
    /// oracle passes 2 and 3.
    false_single: u64,
    false_alone: u64,
    false_oracle: [u64; ORACLE_PASSES],
    /// Oracle-pass decodes of a message already decoded by an earlier pass.
    duplicate_oracle: [u64; ORACLE_PASSES],
}

impl CeilingCounts {
    fn add(&mut self, c: &CeilingCounts) {
        self.false_single += c.false_single;
        self.false_alone += c.false_alone;
        for p in 0..ORACLE_PASSES {
            self.false_oracle[p] += c.false_oracle[p];
            self.duplicate_oracle[p] += c.duplicate_oracle[p];
        }
    }
}

/// The `ceiling` mode on slot `index` of `set` in `block`.
fn ceiling_slot(
    set: &str,
    block: &str,
    index: usize,
    setup: &Setup,
) -> (Vec<CeilingMessage>, CeilingCounts) {
    let slot = build(set, block, index);
    let ns = noise_seed(slot_seed(set, block, index));
    let sent = &slot.sent;
    let mut n = CeilingCounts::default();
    let not_sent = |d: &Decoded| {
        d.decodes
            .iter()
            .filter(|x| !sent.iter().any(|s| s.payload == x.payload))
            .count() as u64
    };
    let single = decode_input(&slot.samples, 0, sent, setup);
    n.false_single = not_sent(&single);
    let alone: Vec<Outcome> = sent
        .iter()
        .map(|s| {
            let one = std::slice::from_ref(s);
            let d = decode_input(&render(one, ns), 0, one, setup);
            n.false_alone += d.decodes.iter().filter(|x| x.payload != s.payload).count() as u64;
            d.outcomes[0]
        })
        .collect();
    // Oracle: remove every message decoded so far with its true waveform
    // (the slot is rebuilt without them, same noise), then decode again.
    let mut found: Vec<Option<u8>> = single
        .outcomes
        .iter()
        .map(|&o| (o == Outcome::Decoded).then_some(1))
        .collect();
    for pass in 0..ORACLE_PASSES {
        let left: Vec<Sent> = sent
            .iter()
            .zip(&found)
            .filter(|(_, f)| f.is_none())
            .map(|(s, _)| s.clone())
            .collect();
        let d = decode_input(&render(&left, ns), 0, &left, setup);
        n.false_oracle[pass] += not_sent(&d);
        for x in &d.decodes {
            if let Some(k) = sent.iter().position(|s| s.payload == x.payload) {
                if found[k].is_none() {
                    found[k] = Some(pass as u8 + 2);
                } else {
                    n.duplicate_oracle[pass] += 1;
                }
            }
        }
    }
    let messages = sent
        .iter()
        .enumerate()
        .map(|(k, s)| CeilingMessage {
            size: sent.len(),
            snr: s.snr,
            single: single.outcomes[k],
            alone: alone[k],
            pass: found[k],
        })
        .collect();
    (messages, n)
}

/// The single-pass 50% point of the sweep set in `block`.
fn sweep_half_point(block: &str, setup: &Setup) -> Option<f64> {
    let sweep: Vec<(String, bool)> = par_map(set_size("sweep").unwrap_or(0), |i| {
        let s = build("sweep", block, i);
        let d = decode_input(&s.samples, s.skip, &s.sent, setup);
        (s.point, d.outcomes[0] == Outcome::Decoded)
    });
    let mut points: Sweep<bool> = BTreeMap::new();
    for (pt, ok) in sweep {
        points.entry(tenths(&pt)).or_default().push(ok);
    }
    half_point_of(&points)
}

/// The miss causes a fresh run can give, in report order.
fn causes(msgs: &[CeilingMessage]) -> Vec<Outcome> {
    Outcome::MISSES
        .into_iter()
        .filter(|&o| match o {
            Outcome::NoCandidate => false,
            Outcome::OtherFailure => msgs.iter().any(|m| m.single == o),
            _ => true,
        })
        .collect()
}

/// The most that subtraction could gain on a crowded set: for each
/// transmitted message, the single pass on the crowded slot, the single
/// pass on a slot with that message alone (same noise), and oracle passes
/// that remove what earlier passes decoded with the true waveforms. SNRs
/// are also given relative to the single-pass 50% point of the sweep set
/// in the same block.
pub fn ceiling(spec: &str, block: &str, set: &str) -> Result<(), String> {
    if !CEILING_SETS.contains(&set) {
        return Err(format!(
            "ceiling takes a set of {CEILING_SETS:?}, not {set}"
        ));
    }
    let setup = parse_config(spec)?;
    println!("config {spec}: {:?}\n{}", setup.config, provenance());
    let t = Instant::now();
    let p50 = sweep_half_point(block, &setup).ok_or("the sweep has no 50% point")?;
    println!(
        "single-pass 50% point (sweep, {block}): {p50:.3} dB ({:.0} s)",
        t.elapsed().as_secs_f64()
    );
    let t = Instant::now();
    let per = par_map(set_size(set).unwrap_or(0), |i| {
        ceiling_slot(set, block, i, &setup)
    });
    eprintln!("{set}-{block}: {:.0} s", t.elapsed().as_secs_f64());
    let mut msgs: Vec<CeilingMessage> = Vec::new();
    let mut counts: BTreeMap<usize, CeilingCounts> = BTreeMap::new();
    for (m, c) in per {
        counts.entry(m[0].size).or_default().add(&c);
        msgs.extend(m);
    }
    let pct = |a: usize, b: usize| 100.0 * a as f64 / b.max(1) as f64;
    let sizes: Vec<Option<usize>> = counts.keys().map(|&z| Some(z)).chain([None]).collect();
    let label = |z: Option<usize>| z.map_or("all".to_string(), |z| z.to_string());
    let of_size = |z: Option<usize>| -> Vec<&CeilingMessage> {
        msgs.iter()
            .filter(|m| z.map_or(true, |z| m.size == z))
            .collect()
    };
    let count = |m: &[&CeilingMessage], f: &dyn Fn(&CeilingMessage) -> bool| {
        m.iter().filter(|m| f(m)).count()
    };
    println!("\n== {set} ({block}): messages decoded (% of transmitted)");
    println!(
        "  {:>5} {:>6} {:>14} {:>14} {:>14} {:>14} {:>14}",
        "size", "sent", "single pass", "oracle +1", "oracle +2", "alone", "ceiling"
    );
    for &z in &sizes {
        let m = of_size(z);
        let cell = |x: usize| format!("{x} ({:.1}%)", pct(x, m.len()));
        println!(
            "  {:>5} {:>6} {:>14} {:>14} {:>14} {:>14} {:>14}",
            label(z),
            m.len(),
            cell(count(&m, &|m| m.by_pass(1))),
            cell(count(&m, &|m| m.by_pass(2))),
            cell(count(&m, &|m| m.by_pass(3))),
            cell(count(&m, &|m| m.alone())),
            cell(count(&m, &|m| m.ceiling()))
        );
    }
    println!("  (oracle +k: k further passes, each on the slot without every message decoded so far; ceiling: decoded by the single pass, alone or an oracle pass)");
    println!("  false decodes (single pass, alone, oracle +1, oracle +2) and oracle duplicates:");
    for (z, c) in &counts {
        println!(
            "  {:>5} {} {} {} {}; duplicates {} {}",
            z,
            c.false_single,
            c.false_alone,
            c.false_oracle[0],
            c.false_oracle[1],
            c.duplicate_oracle[0],
            c.duplicate_oracle[1]
        );
    }
    println!("\n== single-pass misses by cause, and what recovers them");
    println!(
        "  {:>5} {:>14} {:>7} {:>11} {:>10} {:>10}",
        "size", "cause", "missed", "alone dec", "oracle +1", "oracle +2"
    );
    let causes = causes(&msgs);
    for &z in &sizes {
        for &cause in &causes {
            let m: Vec<&CeilingMessage> = of_size(z)
                .into_iter()
                .filter(|m| m.single == cause)
                .collect();
            println!(
                "  {:>5} {:>14} {:>7} {:>11} {:>10} {:>10}",
                label(z),
                cause.code(),
                m.len(),
                count(&m, &|m| m.alone()),
                count(&m, &|m| m.pass == Some(2)),
                count(&m, &|m| m.pass == Some(3))
            );
        }
    }
    let alone_miss = miss_counts(msgs.iter().map(|m| &m.alone));
    println!(
        "  missed even alone (all sizes): {}",
        misses_text(&alone_miss)
    );
    println!("\n== by SNR relative to the single-pass 50% point ({p50:.2} dB), all sizes; right: single-pass misses by cause");
    let mut head = format!(
        "  {:>12} {:>6} {:>8} {:>9} {:>8} {:>8} |",
        "rel dB", "sent", "single", "oracle+1", "alone", "ceiling"
    );
    for c in &causes {
        head.push_str(&format!(" {:>7}", short(*c)));
    }
    println!("{head}");
    let lowest = msgs
        .iter()
        .map(|m| ((m.snr - p50) / 3.0).floor() as i64 * 3)
        .min()
        .unwrap_or(0);
    let mut lo = lowest;
    while msgs.iter().any(|m| m.snr - p50 >= lo as f64) {
        let (l, h) = (lo as f64, (lo + 3) as f64);
        lo += 3;
        let m: Vec<&CeilingMessage> = msgs
            .iter()
            .filter(|m| (l..h).contains(&(m.snr - p50)))
            .collect();
        if m.is_empty() {
            continue;
        }
        let miss = miss_counts(m.iter().map(|m| &m.single));
        let mut line = format!(
            "  {:>12} {:>6} {:>8} {:>9} {:>8} {:>8} |",
            format!("[{l:+.0}, {h:+.0})"),
            m.len(),
            count(&m, &|m| m.by_pass(1)),
            count(&m, &|m| m.by_pass(2)),
            count(&m, &|m| m.alone()),
            count(&m, &|m| m.ceiling()),
        );
        for c in &causes {
            let k = Outcome::MISSES.iter().position(|o| o == c).unwrap_or(0);
            line.push_str(&format!(" {:>7}", miss[k]));
        }
        println!("{line}");
    }
    Ok(())
}

/// A column heading for a miss cause.
fn short(o: Outcome) -> &'static str {
    match o {
        Outcome::NoPeak => "nopeak",
        Outcome::Dropped => "dropped",
        Outcome::NotConverged => "notcnv",
        Outcome::Crc => "crc",
        Outcome::AllZero => "zero",
        _ => "other",
    }
}
