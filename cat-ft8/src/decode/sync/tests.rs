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
    let mut scratch = vec![0.0f32; 3 * cols];
    let mut out = vec![P::default(); k];
    let fill = |q: usize, row: &mut [f32]| {
        for (b, s) in row.iter_mut().enumerate() {
            *s = cell_of(grid[q][b].unwrap_or(NONE));
        }
    };
    let r = select(rows, cols, fill, 0, &mut scratch, &mut out, P).unwrap();
    out.truncate(r.kept);
    (out, r)
}

#[test]
fn a_score_survives_a_cell_bit_for_bit() {
    // Scores that are subnormal, normal and NaN as `f32`, among them the
    // one that means no score (negative zero).
    let mut scores = vec![0, 1, -1, 2, 1_000, -1_000, 85_680, -85_680, NONE, i32::MAX];
    scores.extend((0..32).map(|shift| 1i32 << shift));
    scores.extend((0..32).map(|shift| -(1i32 << shift).max(1)));
    // Signalling NaNs as `f32`, at both ends of their range and either sign:
    // a load or store that quietened one would set bit 22.
    scores.extend([0x7f80_0001u32, 0x7fbf_ffff, 0xff80_0001, 0xffbf_ffff].map(|b| b as i32));
    // Through memory, as the scratch is.
    let cells: Vec<f32> = scores.iter().map(|&s| cell_of(s)).collect();
    let cells = std::hint::black_box(cells);
    let back: Vec<i32> = cells.iter().map(|&c| score_of(c)).collect();
    assert_eq!(back, scores);
    assert_eq!(cell_of(0).to_bits(), 0);
}

/// The local maxima of `grid` at or above `min_score`, from the rules in
/// the module's docs, without any rolling buffer.
fn maxima_by_definition(grid: &[Vec<Option<i32>>], min_score: i32) -> Vec<P> {
    let (rows, cols) = (grid.len() as isize, grid[0].len() as isize);
    let at = |q: isize, b: isize| {
        if q < 0 || q >= rows || b < 0 || b >= cols {
            return None;
        }
        grid[q as usize][b as usize]
    };
    let mut kept = Vec::new();
    for q in 0..rows {
        for b in 0..cols {
            let Some(s) = at(q, b).filter(|&s| s >= min_score) else {
                continue;
            };
            let holds = (-1..=1isize).all(|dq| {
                (-1..=1isize).all(|db| match at(q + dq, b + db) {
                    Some(n) if dq != 0 || db != 0 => {
                        if dq < 0 || (dq == 0 && db < 0) {
                            s > n
                        } else {
                            s >= n
                        }
                    }
                    _ => true,
                })
            });
            if holds {
                kept.push(P(q as usize, b as usize, s));
            }
        }
    }
    kept.sort_by_key(|p| (-i64::from(p.2), p.0, p.1));
    kept
}

#[test]
fn select_takes_any_scores_and_ignores_what_its_scratch_held() {
    // Negative scores are NaNs as cells and small positive ones subnormals.
    let mut noise = crate::decode::testutil::Noise(5);
    let grid: Vec<Vec<Option<i32>>> = (0..12)
        .map(|_| {
            (0..9)
                .map(|_| {
                    let u = noise.uniform();
                    (u > 0.1).then_some(((u - 0.5) * 2_000.0) as i32)
                })
                .collect()
        })
        .collect();
    let fill = |q: usize, row: &mut [f32]| {
        for (b, s) in row.iter_mut().enumerate() {
            *s = cell_of(grid[q][b].unwrap_or(NONE));
        }
    };
    for min_score in [i32::MIN + 1, -300, 0, 300] {
        let want = maxima_by_definition(&grid, min_score);
        assert!(!want.is_empty());
        for stale in [0.0, f32::NAN, cell_of(i32::MAX), cell_of(NONE)] {
            let mut scratch = vec![stale; 3 * 9];
            let mut out = vec![P::default(); 200];
            let r = select(12, 9, fill, min_score, &mut scratch, &mut out, P).unwrap();
            assert_eq!(&out[..r.kept], &want[..], "min {min_score}, held {stale}");
        }
    }
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
    let mut built = Built::new(Band::DEFAULT);
    built.with(samples, |spec| {
        let mut scratch = vec![0.0f32; scratch_len(spec.band().base_bins())];
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
    let mut built = Built::new(Band::DEFAULT);
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
        let mut built = Built::new(Band::DEFAULT);
        let (got, want) = built.with(&slot, |spec| {
            let mut scratch = vec![0.0f32; scratch_len(spec.band().base_bins())];
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

#[test]
fn row_scores_equal_the_single_scores() {
    // Signals and noise, in a full slot and in slots cut where one, two
    // and all three Costas arrays of some start rows are filled; every
    // start row the search visits and a few beyond, every base bin; the
    // levels summed down the columns first, against `costas_score`, on band
    // widths from one bin, odd (all that `Band::from_hz` gives are 8k + 1)
    // and even. The running sums along each row, at the same widths:
    // `paired_scores_equal_the_single_scores`.
    let mut slot = empty_slot();
    add_noise(&mut slot, NOISE_RMS, 11);
    for (k, msg) in ["CQ K1ABC FN42", "K1ABC W9XYZ -11", "W9XYZ K1ABC R-07"]
        .iter()
        .enumerate()
    {
        let amp = amplitude_for(-8.0 + 6.0 * k as f64);
        add_signal(
            &mut slot,
            &symbols_of(msg),
            hz(100 + 300 * k),
            -1.5 + 1.4 * k as f32,
            amp,
        );
    }
    for band in [
        Band::DEFAULT,
        Band::SMALL,
        Band::from_hz(1_000, 1_200).unwrap(),
        // The narrowest that `Band::from_hz` gives: nine base bins.
        Band::from_hz(1_000, 1_025).unwrap(),
        // Widths only a test builds, over the second signal (tone 0 at
        // bin 128): one bin, and even counts.
        Band::of_bins(124, 1),
        Band::of_bins(124, 2),
        Band::of_bins(124, 8),
        Band::of_bins(124, 64),
    ] {
        for samples in [SLOT_SAMPLES, 50_000, 90_000, 130_000, 3_000] {
            let mut built = Built::new(band);
            built.with(&slot[..samples], |spec| {
                let cols = band.base_bins();
                let mut row = vec![0.0f32; cols];
                let first = NOMINAL_START_ROW - MAX_START_OFFSET - 3;
                for start in first..first + START_ROWS as i32 + 6 {
                    costas_scores(spec.rows(), start, &mut row);
                    for (b, &got) in row.iter().enumerate() {
                        let want = costas_score(spec, start, b).unwrap_or(NONE);
                        let got = score_of(got);
                        assert_eq!(got, want, "samples {samples} start {start} base {b}");
                    }
                }
            });
        }
    }
}

impl GridPoint for P {
    fn point(&self) -> (usize, usize) {
        (self.0, self.1)
    }
}

/// Runs `select_paired` over a grid (`None` = no score) with `cells` of
/// scratch, `k` entries and `min_score`, its rows' scratch first filled
/// with `stale`.
fn run_paired(
    grid: &[Vec<Option<i32>>],
    cells: usize,
    k: usize,
    min_score: i32,
    stale: f32,
) -> (Vec<P>, SyncReport) {
    let (rows, cols) = (grid.len(), grid[0].len());
    let mut scratch = vec![stale; cells];
    let mut out = vec![P::default(); k];
    let row = |q: usize, lo: usize, out: &mut [f32]| {
        for (i, s) in out.iter_mut().enumerate() {
            *s = cell_of(grid[q][lo + i].unwrap_or(NONE));
        }
    };
    let fill = |q: usize, lo: usize, a: &mut [f32], b: Option<&mut [f32]>| {
        // Pairs start at q ≡ 0 or 1 mod 4, with q + 2 given while a row.
        assert!(q % 4 < 2 && b.is_some() == (q + 2 < rows), "q {q}");
        row(q, lo, a);
        if let Some(b) = b {
            assert_eq!(b.len(), a.len());
            row(q + 2, lo, b);
        }
    };
    let r = select_paired(rows, cols, fill, min_score, &mut scratch, &mut out, P).unwrap();
    out.truncate(r.kept);
    (out, r)
}

/// The scratch sizes `select_paired` is run with on a grid `cols` wide:
/// the least it takes (three rows, of at least five bins: spans of
/// `3 · cols / 5 − 2` bins, or of one where `cols` is below five), five
/// rows less a cell (spans of `cols` − 3) and five rows and more (one
/// span).
fn paired_cells(cols: usize) -> Vec<usize> {
    let least = 3 * paired_width(cols);
    let mut cells: Vec<usize> = [least, 5 * cols - 1, 5 * cols, 5 * cols + 7]
        .into_iter()
        .filter(|&c| c >= least)
        .collect();
    cells.dedup();
    cells
}

#[test]
fn paired_selection_keeps_the_maxima_by_definition() {
    // Random grids of odd and even sizes with no-score points, scores
    // from a few values (so plateaus and equal scores across the spans are
    // common), every scratch size from spans of one bin to one span, lists
    // short enough to drop entries, and scratch that held anything.
    let mut noise = crate::decode::testutil::Noise(17);
    for (rows, cols) in [
        (1, 1),
        (2, 1),
        (3, 2),
        (4, 3),
        (5, 4),
        (7, 5),
        (12, 9),
        (13, 10),
        (63, 11),
        (9, 40),
    ] {
        let grid: Vec<Vec<Option<i32>>> = (0..rows)
            .map(|_| {
                (0..cols)
                    .map(|_| {
                        let u = noise.uniform();
                        (u > 0.1).then_some(((u * 6.0) as i32 - 3) * 500)
                    })
                    .collect()
            })
            .collect();
        for min_score in [i32::MIN + 1, -600, 0, 1_000] {
            let want = maxima_by_definition(&grid, min_score);
            for cells in paired_cells(cols) {
                for k in [0, 1, 3, want.len(), want.len() + 5] {
                    for stale in [0.0, f32::NAN, cell_of(i32::MAX), cell_of(NONE)] {
                        let (got, r) = run_paired(&grid, cells, k, min_score, stale);
                        let at =
                            std::format!("{rows} × {cols}, min {min_score}, cells {cells}, k {k}");
                        assert_eq!(got, &want[..k.min(want.len())], "{at}");
                        assert_eq!(r.found, want.len(), "{at}");
                        assert_eq!(
                            (r.kept, r.dropped),
                            (got.len(), want.len() - got.len()),
                            "{at}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn paired_selection_equals_select_on_plateaus_and_ties() {
    // The fixed grids of the tests above, every scratch size: the same
    // lists and reports as `select`.
    let mut ties = vec![vec![Some(0); 9]; 3];
    for (b, s) in [(0, 5), (2, 9), (4, 9), (6, 7), (8, 9)] {
        ties[1][b] = Some(s);
    }
    let mut grids = vec![
        ties,
        plateau(&[(2, 2), (2, 3)]),
        plateau(&[(2, 2), (3, 2)]),
        plateau(&[(2, 2), (2, 3), (3, 2), (3, 3)]),
        plateau(&[(2, 3), (3, 2), (3, 3)]),
        plateau(&[(3, 1), (3, 2), (2, 2)]),
        plateau(&[(0, 0), (0, 1), (1, 0)]),
        plateau(&[(5, 6), (5, 5)]),
    ];
    let mut g = plateau(&[(1, 1), (4, 5)]);
    g[1][2] = None;
    grids.push(g);
    // A whole grid of one score: only its first point is kept.
    grids.push(vec![vec![Some(4); 8]; 7]);
    for grid in &grids {
        for k in [1, 2, 3, 64] {
            let (want, wr) = run(grid, k);
            for cells in paired_cells(grid[0].len()) {
                let (got, r) = run_paired(grid, cells, k, 0, 0.0);
                assert_eq!((&got, r), (&want, wr), "{grid:?} cells {cells} k {k}");
            }
        }
    }
}

#[test]
fn paired_selection_needs_its_scratch() {
    // Three rows of the band, and of at least five bins; where the target
    // has no vector unit the decoder's scratch (`scratch_len`) is that.
    let fill = |_: usize, _: usize, _: &mut [f32], _: Option<&mut [f32]>| {};
    for cols in [1, 4, 5, 6, 100] {
        let least = 3 * cols.max(PAIRED_ROWS);
        assert_eq!(3 * paired_width(cols), least);
        if SCALAR_FORMS {
            assert_eq!(scratch_len(cols), least);
        }
        let mut out = [P::default(); 4];
        let mut scratch = vec![0.0f32; least - 1];
        let r = select_paired(3, cols, fill, 0, &mut scratch, &mut out, P);
        assert_eq!(r, Err(Error::BufferTooSmall), "cols {cols}");
        let mut scratch = vec![0.0f32; least];
        let r = select_paired(3, cols, fill, 0, &mut scratch, &mut out, P);
        assert!(r.is_ok(), "cols {cols}");
    }
}

/// Slots of signals and noise for the paired search's tests: a full slot
/// and slots cut where one, two and all three Costas arrays of some start
/// rows are filled.
fn paired_slot() -> Vec<f32> {
    let mut slot = empty_slot();
    add_noise(&mut slot, NOISE_RMS, 11);
    for (k, msg) in ["CQ K1ABC FN42", "K1ABC W9XYZ -11", "W9XYZ K1ABC R-07"]
        .iter()
        .enumerate()
    {
        let amp = amplitude_for(-8.0 + 6.0 * k as f64);
        add_signal(
            &mut slot,
            &symbols_of(msg),
            hz(100 + 300 * k),
            -1.5 + 1.4 * k as f32,
            amp,
        );
    }
    slot
}

/// Bands of every width the tests build: the presets, `Band::from_hz`'s
/// widths of 8k + 1 and the narrowest of them, and widths only a test
/// builds, odd and even, from one bin.
fn paired_bands() -> Vec<Band> {
    let mut bands = vec![
        Band::DEFAULT,
        Band::SMALL,
        Band::from_hz(1_000, 1_200).unwrap(),
        Band::from_hz(1_000, 1_025).unwrap(),
    ];
    bands.extend([1, 2, 3, 4, 5, 6, 8, 64].map(|w| Band::of_bins(124, w)));
    bands
}

/// `costas_scores_paired` of start rows q and q + 2 (and of q alone),
/// over spans at either end of the band and, with `within`, within it,
/// odd and even, against `costas_score` at every bin: every start row the
/// search visits and a few beyond, in slots cut at `cuts` (one, two and
/// three arrays, or none).
fn check_paired_scores(within: bool, cuts: &[usize]) {
    let slot = paired_slot();
    for band in paired_bands() {
        let cols = band.base_bins();
        let mut spans = vec![(0, cols), (0, 1), (cols - 1, 1)];
        if cols > 2 && within {
            spans.extend([(1, cols - 2), (0, cols - 1), (1, cols - 1)]);
        }
        if cols > 9 {
            spans.extend([(3, 6), (cols / 3, 5)]);
            if within {
                spans.push((cols / 2, cols - cols / 2));
            }
        }
        for &samples in cuts {
            let mut built = Built::new(band);
            built.with(&slot[..samples], |spec| {
                let first = NOMINAL_START_ROW - MAX_START_OFFSET - 3;
                for &(lo, width) in &spans {
                    for start in first..first + START_ROWS as i32 + 6 {
                        for paired in [true, false] {
                            let (mut a, mut b) = (vec![f32::NAN; width], vec![f32::NAN; width]);
                            let b_out = paired.then_some(&mut b[..]);
                            costas_scores_paired(spec.rows(), start, lo, &mut a, b_out);
                            for i in 0..width {
                                let at = std::format!(
                                    "samples {samples} start {start} lo {lo} width {width} bin {}",
                                    lo + i
                                );
                                let want = costas_score(spec, start, lo + i).unwrap_or(NONE);
                                assert_eq!(score_of(a[i]), want, "{at}");
                                if paired {
                                    let want =
                                        costas_score(spec, start + 2, lo + i).unwrap_or(NONE);
                                    assert_eq!(score_of(b[i]), want, "{at}, + 2");
                                }
                            }
                        }
                    }
                }
            });
        }
    }
}

#[test]
fn paired_scores_equal_the_single_scores() {
    check_paired_scores(false, &[SLOT_SAMPLES, 50_000, 130_000]);
}

#[test]
#[ignore = "slow in debug builds; CI runs it in release"]
fn paired_scores_equal_the_single_scores_widely() {
    check_paired_scores(true, &[SLOT_SAMPLES, 50_000, 90_000, 130_000, 3_000]);
}

/// `search_paired` against `select` over `costas_scores` (the hosts'
/// form), on every band width the tests build, slots with one, two and
/// three arrays, scratch from the least to one span, lists of `ks`
/// entries (that drop entries) and floors `mins`.
fn check_paired_search(mins: &[i32], ks: &[usize]) {
    let slot = paired_slot();
    for band in paired_bands() {
        let cols = band.base_bins();
        for samples in [SLOT_SAMPLES, 90_000, 130_000] {
            let mut built = Built::new(band);
            built.with(&slot[..samples], |spec| {
                let first_row = NOMINAL_START_ROW - MAX_START_OFFSET;
                for &min_score in mins {
                    for &k in ks {
                        let mut want = vec![Candidate::default(); k];
                        let mut scratch = vec![0.0f32; 3 * cols];
                        let wr = select(
                            START_ROWS,
                            cols,
                            |q, out| costas_scores(spec.rows(), first_row + q as i32, out),
                            min_score,
                            &mut scratch,
                            &mut want,
                            |q, b, score| Candidate {
                                score,
                                start_row: (first_row + q as i32) as i16,
                                base: b as u16,
                            },
                        )
                        .unwrap();
                        want.truncate(wr.kept);
                        for cells in paired_cells(cols) {
                            let mut got = vec![Candidate::default(); k];
                            let mut scratch = vec![f32::NAN; cells];
                            let r = search_paired(spec.rows(), min_score, &mut scratch, &mut got)
                                .unwrap();
                            got.truncate(r.kept);
                            let at = std::format!(
                                "cols {cols} samples {samples} min {min_score} k {k} cells {cells}"
                            );
                            assert_eq!((&got, r), (&want, wr), "{at}");
                        }
                    }
                }
            });
        }
    }
}

#[test]
fn paired_search_equals_the_search_by_rows() {
    check_paired_search(&[i32::MIN + 1, 2_000], &[7, 400]);
}

#[test]
#[ignore = "slow in debug builds; CI runs it in release"]
fn paired_search_equals_the_search_by_rows_widely() {
    check_paired_search(&[i32::MIN + 1, 0, 2_000], &[1, 7, 60, 400]);
}
