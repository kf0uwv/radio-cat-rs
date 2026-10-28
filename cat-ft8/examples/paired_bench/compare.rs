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

//! `compare`: two saved runs paired slot by slot and message by message,
//! and the statistics it (and `ceiling`) use.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::common::Rng;
use crate::record::{miss_counts, misses_text, SlotResult};
use crate::sets::below;

// ----------------------------------------------------------- statistics

/// The SNR at which the decoded fraction first reaches ½, by linear
/// interpolation between points sorted by SNR (`None` if it never does).
fn half_point(points: &[(f64, f64)]) -> Option<f64> {
    for w in points.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if y0 < 0.5 && y1 >= 0.5 {
            return Some(x0 + (0.5 - y0) * (x1 - x0) / (y1 - y0));
        }
    }
    None
}

/// The `p` quantile of already sorted values (`None` if empty).
pub fn quantile(sorted: &[f64], p: f64) -> Option<f64> {
    (!sorted.is_empty()).then(|| sorted[((sorted.len() - 1) as f64 * p).round() as usize])
}

/// Sorts and returns the 2.5% and 97.5% quantiles.
fn interval(v: &mut [f64]) -> (f64, f64) {
    v.sort_by(f64::total_cmp);
    (
        quantile(v, 0.025).unwrap_or(f64::NAN),
        quantile(v, 0.975).unwrap_or(f64::NAN),
    )
}

/// Per-slot sweep outcomes by SNR: the map is keyed by the SNR as a
/// sortable integer (tenths of a dB), so its order is the SNR order the
/// interpolation needs.
pub type Sweep<T> = BTreeMap<i64, Vec<T>>;

/// The SNR key of a sweep point label.
pub fn tenths(point: &str) -> i64 {
    (point.parse::<f64>().unwrap_or(0.0) * 10.0).round() as i64
}

/// The sweep's 50% point, counting an entry as decoded when `hit` says
/// so, over all of each point's entries or the resampled `idx`.
fn curve_by<T>(
    sweep: &Sweep<T>,
    hit: impl Fn(&T) -> bool,
    idx: Option<&[Vec<usize>]>,
) -> Option<f64> {
    let pts: Vec<(f64, f64)> = sweep
        .iter()
        .enumerate()
        .map(|(j, (tenths, v))| {
            let (hits, n) = match idx {
                Some(ix) => (ix[j].iter().filter(|&&i| hit(&v[i])).count(), ix[j].len()),
                None => (v.iter().filter(|x| hit(x)).count(), v.len()),
            };
            (*tenths as f64 / 10.0, hits as f64 / n as f64)
        })
        .collect();
    half_point(&pts)
}

/// The 50% point of one run's sweep outcomes.
pub fn half_point_of(sweep: &Sweep<bool>) -> Option<f64> {
    curve_by(sweep, |&ok| ok, None)
}

/// A paired bootstrap of the sweep's 50% point: the same resampled slots
/// for both runs, 1 000 times.
struct Bootstrap {
    old: Vec<f64>,
    new: Vec<f64>,
    shift: Vec<f64>,
    /// Resamples where either run has no 50% point.
    dropped: usize,
}

fn bootstrap(sweep: &Sweep<(bool, bool)>) -> Bootstrap {
    let mut r = Rng(12_345);
    let mut b = Bootstrap {
        old: Vec::new(),
        new: Vec::new(),
        shift: Vec::new(),
        dropped: 0,
    };
    for _ in 0..1_000 {
        let ix: Vec<Vec<usize>> = sweep
            .values()
            .map(|v| (0..v.len()).map(|_| below(&mut r, v.len())).collect())
            .collect();
        match (
            curve_by(sweep, |p| p.0, Some(&ix)),
            curve_by(sweep, |p| p.1, Some(&ix)),
        ) {
            (Some(o), Some(n)) => {
                b.old.push(o);
                b.new.push(n);
                b.shift.push(n - o);
            }
            _ => b.dropped += 1,
        }
    }
    b
}

/// The sweep's 50% points and their shift; prints them and returns the
/// shift gate (false if a 50% point is missing).
fn sweep_shift(sweep: &Sweep<(bool, bool)>) -> bool {
    let (h_old, h_new) = (
        curve_by(sweep, |p| p.0, None),
        curve_by(sweep, |p| p.1, None),
    );
    let mut b = bootstrap(sweep);
    match (h_old, h_new) {
        (Some(o), Some(n)) => {
            let shift_ok = n - o <= 0.05;
            let ((ol, oh), (nl, nh), (dl, dh)) = (
                interval(&mut b.old),
                interval(&mut b.new),
                interval(&mut b.shift),
            );
            println!(
                "  50% point: old {o:.3} dB [{ol:.3}, {oh:.3}], new {n:.3} dB [{nl:.3}, {nh:.3}], shift {:+.3} dB [{dl:+.3}, {dh:+.3}] ({} of 1 000 resamples without a 50% point): {}",
                n - o,
                b.dropped,
                if shift_ok { "ok" } else { "SHIFT" }
            );
            shift_ok
        }
        _ => {
            println!("  50% point: MISSING (old {h_old:?}, new {h_new:?})");
            false
        }
    }
}

// ----------------------------------------------------------------- load

/// A saved run's slots by (set, block).
type Run = BTreeMap<(String, String), Vec<SlotResult>>;

fn load(dir: &Path) -> Result<Run, String> {
    let mut out = BTreeMap::new();
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for e in entries {
        let p = e.map_err(|e| e.to_string())?.path();
        if p.extension().is_some_and(|x| x == "jsonl") {
            let stem = p
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let (set, block) = stem
                .split_once('-')
                .ok_or_else(|| format!("unexpected file {}", p.display()))?;
            let text = fs::read_to_string(&p).map_err(|e| e.to_string())?;
            let mut slots = Vec::new();
            for (n, l) in text.lines().enumerate() {
                let v: Value = serde_json::from_str(l)
                    .map_err(|e| format!("{}:{}: {e}", p.display(), n + 1))?;
                slots.push(
                    SlotResult::from_json(&v)
                        .ok_or_else(|| format!("{}:{}: not a slot record", p.display(), n + 1))?,
                );
            }
            out.insert((set.to_string(), block.to_string()), slots);
        }
    }
    Ok(out)
}

// -------------------------------------------------------------- pairing

/// Per-point paired counts.
#[derive(Default, Clone, Copy)]
struct Pair {
    messages: u64,
    both: u64,
    old_only: u64,
    new_only: u64,
    false_old: u64,
    false_new: u64,
}

impl Pair {
    /// The per-point gates: (no loss beyond 3·√(b + c), false decodes —
    /// none on `empty`, elsewhere no more than the old run's).
    fn verdict(&self, set: &str) -> (bool, bool) {
        let (bb, cc) = (self.old_only as f64, self.new_only as f64);
        let loss_ok = bb - cc <= 3.0 * (bb + cc).sqrt();
        let false_ok = if set == "empty" {
            self.false_new == 0
        } else {
            self.false_new <= self.false_old
        };
        (loss_ok, false_ok)
    }
}

/// One side's work: candidates through BP, BP rounds, decodes truncated,
/// candidates dropped; and summed stage seconds.
#[derive(Default)]
struct Work {
    counts: [u64; 4],
    seconds: BTreeMap<String, f64>,
}

impl Work {
    fn add(&mut self, r: &SlotResult) {
        self.counts[0] += r.counts.kept;
        self.counts[1] += r.bp_rounds;
        self.counts[2] += r.counts.truncated;
        self.counts[3] += r.counts.dropped;
        for (k, t) in &r.seconds {
            *self.seconds.entry(k.clone()).or_default() += t;
        }
    }
}

/// A set's paired tallies.
#[derive(Default)]
struct Tally {
    /// Per point, in the order points first appear.
    points: BTreeMap<String, Pair>,
    order: Vec<String>,
    sweep: Sweep<(bool, bool)>,
    work: [Work; 2],
    /// Slot pairs whose seed, samples or messages differ.
    input_differs: usize,
}

fn tally(set: &str, sa: &[SlotResult], sb: &[SlotResult]) -> Tally {
    let mut t = Tally::default();
    for (x, y) in sa.iter().zip(sb) {
        if x.index != y.index
            || x.seed != y.seed
            || x.sample_hash != y.sample_hash
            || x.sent != y.sent
        {
            t.input_differs += 1;
            continue;
        }
        if !t.points.contains_key(&x.point) {
            t.order.push(x.point.clone());
        }
        let p = t.points.entry(x.point.clone()).or_default();
        let da: Vec<u128> = x.decodes.iter().map(|d| d.payload).collect();
        let db: Vec<u128> = y.decodes.iter().map(|d| d.payload).collect();
        let sent: Vec<u128> = x.sent.iter().map(|s| s.payload).collect();
        for m in &sent {
            let (ia, ib) = (da.contains(m), db.contains(m));
            p.messages += 1;
            p.both += u64::from(ia && ib);
            p.old_only += u64::from(ia && !ib);
            p.new_only += u64::from(!ia && ib);
            if set == "sweep" {
                t.sweep.entry(tenths(&x.point)).or_default().push((ia, ib));
            }
        }
        p.false_old += da.iter().filter(|d| !sent.contains(d)).count() as u64;
        p.false_new += db.iter().filter(|d| !sent.contains(d)).count() as u64;
        t.work[0].add(x);
        t.work[1].add(y);
    }
    t
}

/// Prints the per-point table and returns (b, c, every point passes).
fn print_points(set: &str, t: &Tally) -> (u64, u64, bool) {
    println!(
        "  {:>10} {:>8} {:>8} {:>8} {:>6} {:>6} {:>8} {:>8}  verdict",
        "point", "messages", "dec_old", "dec_new", "b", "c", "false_o", "false_n"
    );
    let (mut tb, mut tc, mut pass) = (0u64, 0u64, true);
    for pt in &t.order {
        let p = t.points[pt];
        let (loss_ok, false_ok) = p.verdict(set);
        pass &= loss_ok && false_ok;
        tb += p.old_only;
        tc += p.new_only;
        println!(
            "  {:>10} {:>8} {:>8} {:>8} {:>6} {:>6} {:>8} {:>8}  {}{}",
            pt,
            p.messages,
            p.both + p.old_only,
            p.both + p.new_only,
            p.old_only,
            p.new_only,
            p.false_old,
            p.false_new,
            if loss_ok { "" } else { "LOSS " },
            if false_ok { "ok" } else { "FALSE" }
        );
    }
    (tb, tc, pass)
}

/// One side's CRC failures, the false decodes they imply, and its misses
/// by cause.
fn print_misses(side: &str, run: &[SlotResult]) {
    let crc: u64 = run.iter().map(|r| r.counts.crc_failed).sum();
    let per_slot = crc as f64 / run.len().max(1) as f64;
    let most = run.iter().map(|r| r.counts.crc_failed).max().unwrap_or(0);
    let recorded = run
        .iter()
        .all(|r| r.outcomes.as_ref().is_some_and(|o| o.len() == r.sent.len()));
    let misses = if recorded {
        let m = miss_counts(run.iter().flat_map(|r| r.outcomes.iter().flatten()));
        format!("missed: {}", misses_text(&m))
    } else {
        "missed by cause: not recorded".to_string()
    };
    println!(
        "  {side}: CRC failures {crc} ({per_slot:.4} per slot, at most {most}); expected false decodes {:.2e} per slot; {misses}",
        per_slot / 16_384.0
    );
}

/// Compares one (set, block) of the two runs; prints it and returns its
/// verdict.
fn compare_set(set: &str, block: &str, sa: &[SlotResult], sb: &[SlotResult]) -> bool {
    if sa.len() != sb.len() {
        println!(
            "\n== {set} ({block}): different slot counts {} and {}",
            sa.len(),
            sb.len()
        );
        return false;
    }
    let t = tally(set, sa, sb);
    let mut pass = true;
    println!("\n== {set} ({block}), {} slots", sa.len());
    if t.input_differs > 0 {
        println!(
            "  INPUT DIFFERS in {} slots (seed, sample hash or messages): not comparable",
            t.input_differs
        );
        pass = false;
    }
    let (tb, tc, points_ok) = print_points(set, &t);
    pass &= points_ok;
    let (bb, cc) = (tb as f64, tc as f64);
    let set_ok = bb - cc <= 2.0 * (bb + cc).sqrt();
    pass &= set_ok;
    println!(
        "  set: b {tb}, c {tc}, b − c = {} (limit {:.1}): {}",
        tb as i64 - tc as i64,
        2.0 * (bb + cc).sqrt(),
        if set_ok { "ok" } else { "LOSS" }
    );
    if set == "sweep" {
        pass &= sweep_shift(&t.sweep);
    }
    let (ca, cb) = (&t.work[0].counts, &t.work[1].counts);
    println!(
        "  cost: candidates through BP {} -> {}; rounds per candidate {:.2} -> {:.2}; decodes truncated {} -> {}; candidates dropped {} -> {}",
        ca[0],
        cb[0],
        ca[1] as f64 / ca[0].max(1) as f64,
        cb[1] as f64 / cb[0].max(1) as f64,
        ca[2],
        cb[2],
        ca[3],
        cb[3]
    );
    print_misses("old", sa);
    print_misses("new", sb);
    let mut line = String::from("  summed stage seconds (all threads):");
    for (k, va) in &t.work[0].seconds {
        let vb = t.work[1].seconds.get(k).copied().unwrap_or(0.0);
        let _ = write!(line, " {k} {va:.1} -> {vb:.1};");
    }
    println!("{line}");
    println!("  verdict: {}", if pass { "PASS" } else { "FAIL" });
    pass
}

pub fn compare(old: &Path, new: &Path, partial: bool) -> Result<bool, String> {
    let (a, b) = (load(old)?, load(new)?);
    let first = |d: &Path| {
        fs::read_to_string(d.join("config.txt"))
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or("")
            .to_string()
    };
    println!("old: {}\nnew: {}", first(old), first(new));
    let mut all_pass = true;
    let keys: BTreeSet<&(String, String)> = a.keys().chain(b.keys()).collect();
    let mut compared = 0;
    for key in &keys {
        let (Some(sa), Some(sb)) = (a.get(*key), b.get(*key)) else {
            println!(
                "\n== {} ({}): MISSING from the {} run",
                key.0,
                key.1,
                if a.contains_key(*key) { "new" } else { "old" }
            );
            all_pass = false;
            continue;
        };
        compared += 1;
        all_pass &= compare_set(&key.0, &key.1, sa, sb);
    }
    if compared == 0 {
        println!("\nnothing to compare");
        all_pass = false;
    }
    if !partial {
        for set in ["sweep", "crowded", "edges", "empty"] {
            let key = (set.to_string(), "main".to_string());
            if !a.contains_key(&key) && !b.contains_key(&key) {
                println!(
                    "\n== {set} (main): MISSING from both runs (use --partial to compare a subset)"
                );
                all_pass = false;
            }
        }
    }
    println!("\noverall: {}", if all_pass { "PASS" } else { "FAIL" });
    Ok(all_pass)
}
