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

//! `cost`: two configurations timed interleaved on crowded slots.

use std::collections::BTreeMap;

use crate::compare::quantile;
use crate::parse_config;
use crate::record::decode_slot;
use crate::sets::{build, Slot};

pub fn cost(spec_a: &str, spec_b: &str, slots: usize, runs: usize) -> Result<(), String> {
    let (a, b) = (parse_config(spec_a)?, parse_config(spec_b)?);
    // Evenly over the three sizes: indices 0.., 200.., 400.. in turn.
    let indices: Vec<usize> = (0..slots).map(|k| (k % 3) * 200 + k / 3).collect();
    let input: Vec<Slot> = indices
        .iter()
        .map(|&i| build("crowded", "main", i))
        .collect();
    let runs = runs.max(5);
    let mut per: [BTreeMap<String, Vec<f64>>; 2] = Default::default();
    for r in 0..runs {
        let order = if r % 2 == 0 { [0, 1] } else { [1, 0] };
        for which in order {
            let setup = if which == 0 { &a } else { &b };
            let mut tot: BTreeMap<String, f64> = BTreeMap::new();
            for (s, &i) in input.iter().zip(&indices) {
                for (k, t) in decode_slot("crowded", "main", i, s, setup).seconds {
                    *tot.entry(k).or_default() += t;
                }
            }
            let total: f64 = tot.values().sum();
            tot.insert("total".into(), total);
            for (k, t) in tot {
                per[which].entry(k).or_default().push(t / slots as f64);
            }
        }
    }
    println!(
        "median of {runs} interleaved runs over {slots} crowded slots ({} each of 10, 20 and 40 messages), one thread (ms per slot):",
        slots / 3
    );
    for (k, va) in &per[0] {
        let (mut sa, mut sb) = (va.clone(), per[1][k].clone());
        sa.sort_by(f64::total_cmp);
        sb.sort_by(f64::total_cmp);
        let (ma, mb) = (
            quantile(&sa, 0.5).unwrap_or(f64::NAN),
            quantile(&sb, 0.5).unwrap_or(f64::NAN),
        );
        println!(
            "  {k:<20} A {:8.3}  B {:8.3}  {:+6.1}%",
            ma * 1e3,
            mb * 1e3,
            (mb - ma) / ma * 100.0
        );
    }
    Ok(())
}

/// The multi-pass cost statistic: every 40-message slot of the crowded
/// set's main block (all 200, or the first SLOTS), decoded with A and B
/// interleaved (the order alternating by run), one thread, RUNS times (at
/// least 5). Per slot the median of its runs' total decode time; reported:
/// the largest and the mean of those medians, the slot that is largest,
/// and per step (summed over passes, and per pass) the mean of the
/// per-slot medians. The deterministic work per pass is printed beside it:
/// candidates through belief propagation, rounds, subtractions and new
/// decodes (list entries the pass added); then each pass's cost (its
/// steps' means summed), new decodes and cost per new decode.
pub fn slot_cost(spec_a: &str, spec_b: &str, slots: usize, runs: usize) -> Result<(), String> {
    use crate::record::decode_input;
    let setups = [parse_config(spec_a)?, parse_config(spec_b)?];
    let slots = slots.clamp(1, 200);
    let input: Vec<Slot> = (0..slots)
        .map(|k| build("crowded", "main", 400 + k))
        .collect();
    let runs = runs.max(5);
    // [config][slot][run] -> step seconds.
    let mut times: Vec<Vec<Vec<BTreeMap<String, f64>>>> = vec![vec![Vec::new(); slots]; 2];
    let mut work: [BTreeMap<String, u64>; 2] = Default::default();
    for r in 0..runs {
        let order = if r % 2 == 0 { [0, 1] } else { [1, 0] };
        for which in order {
            for (k, s) in input.iter().enumerate() {
                let d = decode_input(&s.samples, s.skip, &s.sent, &setups[which]);
                let mut m: BTreeMap<String, f64> = d.seconds.into_iter().collect();
                let total: f64 = m
                    .iter()
                    .filter(|(k, _)| !k.starts_with("pass"))
                    .map(|(_, v)| v)
                    .sum();
                m.insert("total".into(), total);
                times[which][k].push(m);
                if r == 0 {
                    let w = &mut work[which];
                    match &d.passes {
                        Some(ps) => {
                            for p in ps {
                                *w.entry(format!("pass{} candidates", p.pass)).or_default() +=
                                    p.kept;
                                *w.entry(format!("pass{} BP rounds", p.pass)).or_default() +=
                                    p.bp_rounds;
                                *w.entry(format!("pass{} subtractions", p.pass)).or_default() +=
                                    p.subtracted;
                                *w.entry(format!("pass{} slots", p.pass)).or_default() += 1;
                                *w.entry(format!("pass{} new decodes", p.pass)).or_default() +=
                                    p.decoded - p.truncated;
                            }
                        }
                        None => {
                            *w.entry("pass1 candidates".into()).or_default() += d.counts.kept;
                            *w.entry("pass1 BP rounds".into()).or_default() += d.bp_rounds;
                            *w.entry("pass1 slots".into()).or_default() += 1;
                            *w.entry("pass1 new decodes".into()).or_default() +=
                                d.counts.decoded - d.counts.truncated;
                        }
                    }
                }
            }
        }
    }
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        quantile(&v, 0.5).unwrap_or(f64::NAN)
    };
    println!(
        "{slots} crowded 40-message slots (main), {runs} interleaved runs, one thread; per slot the median of its runs (ms):"
    );
    for (which, spec) in [spec_a, spec_b].iter().enumerate() {
        let per: Vec<f64> = (0..slots)
            .map(|k| median(times[which][k].iter().map(|m| m["total"]).collect()))
            .collect();
        let (worst_k, worst) = per
            .iter()
            .copied()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((0, f64::NAN));
        let mean = per.iter().sum::<f64>() / slots as f64;
        println!(
            "  {} {spec}: worst {:.1} ms (slot {}), mean {:.1} ms",
            if which == 0 { "A" } else { "B" },
            worst * 1e3,
            400 + worst_k,
            mean * 1e3
        );
        let keys: std::collections::BTreeSet<String> = times[which]
            .iter()
            .flat_map(|runs| runs.iter().flat_map(|m| m.keys().cloned()))
            .collect();
        let mut line = String::from("    mean of per-slot medians:");
        let mut means: BTreeMap<&str, f64> = BTreeMap::new();
        for key in keys.iter().filter(|k| *k != "total") {
            let m: f64 = (0..slots)
                .map(|k| {
                    median(
                        times[which][k]
                            .iter()
                            .map(|m| m.get(key).copied().unwrap_or(0.0))
                            .collect(),
                    )
                })
                .sum::<f64>()
                / slots as f64;
            line.push_str(&format!(" {key} {:.2};", m * 1e3));
            means.insert(key, m);
        }
        println!("{line}");
        let mut line = String::from("    work per slot (mean):");
        for (k, v) in &work[which] {
            line.push_str(&format!(" {k} {:.1};", *v as f64 / slots as f64));
        }
        println!("{line}");
        // Each pass's cost: its own steps (`passN_*`) where several passes
        // ran, every step where one did.
        let mut line = String::from("    per pass (mean):");
        let passes = work[which].keys().filter(|k| k.ends_with(" slots")).count();
        for p in 1..=passes {
            let prefix = format!("pass{p}_");
            let ms = 1e3
                * means
                    .iter()
                    .filter(|(k, _)| {
                        if passes > 1 {
                            k.starts_with(&prefix)
                        } else {
                            !k.starts_with("pass")
                        }
                    })
                    .map(|(_, m)| m)
                    .sum::<f64>();
            let new = work[which]
                .get(&format!("pass{p} new decodes"))
                .copied()
                .unwrap_or(0) as f64
                / slots as f64;
            line.push_str(&format!(
                " pass{p} {ms:.2} ms, {new:.1} new decodes, {:.2} ms per new decode;",
                ms / new
            ));
        }
        println!("{line}");
    }
    Ok(())
}
