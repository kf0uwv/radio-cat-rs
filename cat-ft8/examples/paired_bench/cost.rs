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
