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

extern crate std;

use std::vec;
use std::vec::Vec;

use super::*;
use crate::decode::frontend::Band;
use crate::decode::testutil::{
    add_noise, add_signal, amplitude_for, empty_slot, symbols_of, Built, NOISE_RMS,
};
use crate::SLOT_SAMPLES;

/// A test entry: (q, b, score).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct P(usize, usize, i32);

impl HasScore for P {
    fn score(&self) -> i32 {
        self.2
    }
}

/// Runs `select` over a grid given as rows of scores (`None` = no score).
fn run(grid: &[Vec<Option<i32>>], k: usize) -> (Vec<P>, SyncReport) {
    let (rows, cols) = (grid.len(), grid[0].len());
    let mut scratch = vec![0i32; 3 * cols];
    let mut out = vec![P::default(); k];
    let r = select(rows, cols, |q, b| grid[q][b], 0, &mut scratch, &mut out, P).unwrap();
    out.truncate(r.kept);
    (out, r)
}

/// A 6 × 7 grid of score 1 with the given points set to 10.
fn plateau(points: &[(usize, usize)]) -> Vec<Vec<Option<i32>>> {
    let mut g = vec![vec![Some(1); 7]; 6];
    for &(q, b) in points {
        g[q][b] = Some(10);
    }
    g
}

fn maxima_at_10(grid: &[Vec<Option<i32>>]) -> Vec<(usize, usize)> {
    run(grid, 64)
        .0
        .into_iter()
        .filter(|p| p.2 == 10)
        .map(|p| (p.0, p.1))
        .collect()
}

#[test]
fn plateaus_keep_exactly_their_first_point() {
    assert_eq!(maxima_at_10(&plateau(&[(2, 2), (2, 3)])), [(2, 2)]); // 1 × 2
    assert_eq!(maxima_at_10(&plateau(&[(2, 2), (3, 2)])), [(2, 2)]); // 2 × 1
    assert_eq!(
        maxima_at_10(&plateau(&[(2, 2), (2, 3), (3, 2), (3, 3)])),
        [(2, 2)]
    ); // 2 × 2
    assert_eq!(maxima_at_10(&plateau(&[(2, 3), (3, 2), (3, 3)])), [(2, 3)]); // L
    assert_eq!(maxima_at_10(&plateau(&[(3, 1), (3, 2), (2, 2)])), [(2, 2)]); // L, other way
    assert_eq!(maxima_at_10(&plateau(&[(0, 0), (0, 1), (1, 0)])), [(0, 0)]); // at the edge
    assert_eq!(maxima_at_10(&plateau(&[(5, 6), (5, 5)])), [(5, 5)]); // far edge
                                                                     // Two separate peaks both survive; points without a score impose nothing.
    let mut g = plateau(&[(1, 1), (4, 5)]);
    g[1][2] = None;
    assert_eq!(maxima_at_10(&g), [(1, 1), (4, 5)]);
}

#[test]
fn order_ties_and_truncation() {
    // Five isolated peaks: scores 5, 9, 9, 7, 9 in visiting order.
    let mut g = vec![vec![Some(0); 9]; 3];
    for (b, s) in [(0, 5), (2, 9), (4, 9), (6, 7), (8, 9)] {
        g[1][b] = Some(s);
    }
    let (all, r) = run(&g, 10);
    assert_eq!(r.found, 5);
    let order: Vec<(usize, i32)> = all.iter().map(|p| (p.1, p.2)).collect();
    assert_eq!(order, [(2, 9), (4, 9), (8, 9), (6, 7), (0, 5)]);
    let (top, r) = run(&g, 3);
    assert_eq!((r.found, r.kept, r.dropped), (5, 3, 2));
    assert_eq!(top.iter().map(|p| p.1).collect::<Vec<_>>(), [2, 4, 8]);
}

/// Base frequency of base bin `b` in the default band.
fn hz(b: usize) -> f32 {
    (32 + b) as f32 * 3.125
}

/// Start row → dt.
fn dt(start_row: i32) -> f32 {
    (start_row - NOMINAL_START_ROW) as f32 * ROW_SECONDS
}

fn candidates(samples: &[f32], k: usize) -> (Vec<Candidate>, SyncReport) {
    let mut built = Built::<u8>::new(Band::DEFAULT);
    built.with(samples, |spec| {
        let mut scratch = vec![0i32; scratch_len(spec.band().base_bins())];
        let mut out = vec![Candidate::default(); k];
        let r = find_candidates(spec, 0, &mut scratch, &mut out).unwrap();
        out.truncate(r.kept);
        (out, r)
    })
}

#[test]
fn encoder_signals_are_found_at_their_grid_point() {
    let symbols = symbols_of("CQ K1ABC FN42");
    for (row, b) in [(5, 448), (0, 17), (19, 900), (-12, 300), (31, 620)] {
        let mut slot = empty_slot();
        add_signal(&mut slot, &symbols, hz(b), dt(row), 0.3);
        add_noise(&mut slot, 0.003, 1);
        let (c, _) = candidates(&slot, 8);
        assert_eq!(
            (i32::from(c[0].start_row), usize::from(c[0].base)),
            (row, b),
            "{c:?}"
        );
    }
}

#[test]
fn both_ends_of_the_time_range_use_partial_sync() {
    let symbols = symbols_of("K1ABC W9XYZ -11");
    for row in [
        NOMINAL_START_ROW - MAX_START_OFFSET,
        NOMINAL_START_ROW + MAX_START_OFFSET,
    ] {
        let mut slot = empty_slot();
        add_signal(&mut slot, &symbols, hz(500), dt(row), 0.3);
        add_noise(&mut slot, 0.003, 2);
        let (c, _) = candidates(&slot, 8);
        assert_eq!(
            (i32::from(c[0].start_row), c[0].base),
            (row, 500),
            "row {row}: {c:?}"
        );
    }
}

#[test]
fn weak_signals_beside_a_strong_one_are_still_candidates() {
    let mut slot = empty_slot();
    add_signal(
        &mut slot,
        &symbols_of("CQ K1ABC FN42"),
        hz(400),
        0.0,
        amplitude_for(0.0),
    );
    add_signal(
        &mut slot,
        &symbols_of("K1ABC W9XYZ -11"),
        hz(380),
        0.16,
        amplitude_for(-15.0),
    );
    add_signal(
        &mut slot,
        &symbols_of("W9XYZ K1ABC R-09"),
        hz(420),
        -0.16,
        amplitude_for(-15.0),
    );
    add_noise(&mut slot, NOISE_RMS, 3);
    let (c, _) = candidates(&slot, 200);
    let at = |row: i32, b: u16| {
        c.iter()
            .position(|x| i32::from(x.start_row) == row && x.base == b)
    };
    assert_eq!(at(5, 400), Some(0));
    assert!(at(7, 380).is_some(), "weak below");
    assert!(at(3, 420).is_some(), "weak above");
}

#[test]
fn a_partial_slot_scores_only_the_arrays_it_has() {
    let symbols = symbols_of("CQ K1ABC FN42");
    let mut slot = empty_slot();
    add_signal(&mut slot, &symbols, hz(448), 0.0, 0.3);
    // Rows up to the middle Costas array only: arrays 0 and 1 present.
    let rows_needed = 5 + 2 * 42 + 1;
    let samples = 240 + 960 * (rows_needed - 1) + 3_840;
    let mut built = Built::<u8>::new(Band::DEFAULT);
    let (full, partial, none) = built.with(&slot[..samples.min(SLOT_SAMPLES)], |spec| {
        (
            costas_score(spec, 5, 448).is_some(),
            spec.rows_filled(),
            costas_score(spec, 30, 448),
        )
    });
    assert!(full);
    assert_eq!(partial, rows_needed);
    assert_eq!(none, None); // start row 30: only array 0 is filled
}

#[test]
fn refinement_moves_towards_sub_grid_offsets() {
    // A noise-free signal placed off the grid by known fractions. Along
    // each axis the refined offset moves from the grid point towards the
    // true position (it may stop short: a parabola through three sync
    // scores is a rough model), and stays near 0 when the signal is on the
    // grid along that axis.
    for &(fb, fr) in &[
        (0.0f32, 0.0f32),
        (0.25, 0.0),
        (0.0, 0.25),
        (-0.3, 0.4),
        (0.45, -0.45),
    ] {
        let mut slot = empty_slot();
        let bin = 480.0 + fb;
        let start = dt(5) + fr * ROW_SECONDS;
        add_signal(
            &mut slot,
            &symbols_of("CQ K1ABC FN42"),
            bin * 3.125,
            start,
            0.3,
        );
        let mut built = Built::<u8>::new(Band::DEFAULT);
        let (got, want) = built.with(&slot, |spec| {
            let mut scratch = vec![0i32; scratch_len(spec.band().base_bins())];
            let mut out = vec![Candidate::default(); 4];
            find_candidates(spec, 0, &mut scratch, &mut out).unwrap();
            let c = out[0];
            let true_row = 5.0 + fr - f32::from(c.start_row);
            let true_bin = bin - (spec.band().first_bin() + usize::from(c.base)) as f32;
            let (r, b) = refine(spec, &c);
            ([r, b], [true_row, true_bin])
        });
        for axis in 0..2 {
            let (g, w) = (got[axis], want[axis]);
            if w.abs() < 0.05 {
                assert!(
                    g.abs() <= 0.15,
                    "offset ({fb}, {fr}) axis {axis}: {g} for {w}"
                );
            } else {
                assert!(
                    g * w > 0.0 && (g - w).abs() < w.abs(),
                    "offset ({fb}, {fr}) axis {axis}: {g} for {w}"
                );
            }
        }
    }
}
