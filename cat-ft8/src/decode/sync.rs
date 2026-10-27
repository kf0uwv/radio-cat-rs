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

//! Sync search (ADR 0024, amendment "single-pass decoder"): every base bin
//! of the band and every start row from 2.48 s early to 2.48 s late is
//! scored against the three Costas arrays (Costas, 1984; pattern and
//! positions from QEX §4), local maxima are kept, and the best are listed
//! in a fixed order.
//!
//! - **Grid.** A candidate is a start row (the row of the first symbol's
//!   centre; row 5 is the nominal 0.5 s start) and a base bin. Rows step
//!   half a symbol (80 ms), bins 3.125 Hz.
//! - **Score** (integer, in level steps). Each Costas symbol scores
//!   `8 · L[expected tone] − Σ L[all eight tones]`: how far the expected
//!   tone stands above the eight-tone mean, times eight. An array counts
//!   only if all seven of its rows are filled; a candidate needs at least
//!   two arrays, and their summed score is multiplied by 6 / n (n = 2 or
//!   3) so partial and full candidates share one scale.
//! - **Local maxima.** A point is kept if its score is ≥ every neighbour
//!   in the 3 × 3 neighbourhood that comes later in (row, bin) order and >
//!   every one that comes earlier, so a plateau keeps its first point.
//!   Neighbours outside the grid, or without a score, impose nothing.
//! - **Order.** Score descending, then start row, then base bin. The list
//!   keeps the best `out.len()`; the rest are counted as dropped.

use super::frontend::{Level, Spectrogram};
use crate::symbols::{COSTAS, COSTAS_POSITIONS};
use crate::Error;

/// Start row of a signal that starts at the nominal 0.5 s.
pub const NOMINAL_START_ROW: i32 = 5;
/// Furthest start-row offset searched either side (31 half-symbols:
/// ±2.48 s).
pub const MAX_START_OFFSET: i32 = 31;
/// Start rows searched: −26 ..= 36.
pub const START_ROWS: usize = (2 * MAX_START_OFFSET + 1) as usize;
/// Seconds per row.
pub const ROW_SECONDS: f32 = 0.08;

/// One sync candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Candidate {
    /// Sync score (level steps; see the module docs).
    pub score: i32,
    /// Row of the first symbol's centre (may be negative).
    pub start_row: i16,
    /// Base (tone 0) bin, counted from the band's first bin.
    pub base: u16,
}

impl Candidate {
    /// Time offset from the nominal start, seconds (grid value).
    pub fn dt(&self) -> f32 {
        dt_of(i32::from(self.start_row))
    }
}

/// Time offset from the nominal start of a start row, seconds.
pub fn dt_of(start_row: i32) -> f32 {
    (start_row - NOMINAL_START_ROW) as f32 * ROW_SECONDS
}

/// What a search found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyncReport {
    /// Local maxima at or above the minimum score.
    pub found: usize,
    /// Of those, kept in the list.
    pub kept: usize,
    /// Of those, dropped because the list was full.
    pub dropped: usize,
}

/// Scratch for one search: three rows of scores (`3 × base bins` values).
pub const fn scratch_len(base_bins: usize) -> usize {
    3 * base_bins
}

/// Searches `spec` (its filled rows) and fills `out` with the best
/// candidates scoring at least `min_score`, in order. `scratch` needs
/// [`scratch_len`] entries.
pub fn find_candidates<L: Level>(
    spec: &Spectrogram<'_, L>,
    min_score: i32,
    scratch: &mut [i32],
    out: &mut [Candidate],
) -> Result<SyncReport, Error> {
    let cols = spec.band().base_bins();
    let first_row = NOMINAL_START_ROW - MAX_START_OFFSET;
    select(
        START_ROWS,
        cols,
        |q, b| costas_score(spec, first_row + q as i32, b),
        min_score,
        scratch,
        out,
        |q, b, score| Candidate {
            score,
            start_row: (first_row + q as i32) as i16,
            base: b as u16,
        },
    )
}

/// The sync score of a candidate at `start_row`, base bin `base`, or
/// `None` if fewer than two Costas arrays lie in the filled rows.
pub(crate) fn costas_score<L: Level>(
    spec: &Spectrogram<'_, L>,
    start_row: i32,
    base: usize,
) -> Option<i32> {
    let rows = spec.rows_filled() as i32;
    let (mut sum, mut present) = (0i32, 0i32);
    for &pos in COSTAS_POSITIONS.iter() {
        let first = start_row + 2 * pos as i32;
        let last = first + 2 * (COSTAS.len() as i32 - 1);
        if first < 0 || last >= rows {
            continue;
        }
        for (j, &tone) in COSTAS.iter().enumerate() {
            let row = spec.row((first + 2 * j as i32) as usize)?;
            let mut all = 0i32;
            for t in 0..8 {
                all += row[base + 2 * t].value() as i32;
            }
            sum += 8 * row[base + 2 * usize::from(tone)].value() as i32 - all;
        }
        present += 1;
    }
    // 6 / n with n = 2 or 3: × 3 or × 2, exact.
    (present >= 2).then(|| sum * (6 / present))
}

/// Sub-grid offsets of a candidate, in rows and bins (each −½…½): the
/// vertex of the parabola through the sync scores of the candidate and
/// its two neighbours along each axis (0 along an axis where a neighbour
/// has no score or the scores are not peaked).
pub fn refine<L: Level>(spec: &Spectrogram<'_, L>, c: &Candidate) -> (f32, f32) {
    let row = i32::from(c.start_row);
    let base = usize::from(c.base);
    let cols = spec.band().base_bins();
    let s0 = c.score;
    let vertex = |a: Option<i32>, b: Option<i32>| -> f32 {
        let (Some(a), Some(b)) = (a, b) else {
            return 0.0;
        };
        let den = a - 2 * s0 + b;
        if den >= 0 {
            return 0.0;
        }
        // Finite: integers over a negative denominator.
        ((a - b) as f32 / (2 * den) as f32).clamp(-0.5, 0.5)
    };
    let fr = vertex(
        costas_score(spec, row - 1, base),
        costas_score(spec, row + 1, base),
    );
    let left = base.checked_sub(1).and_then(|b| costas_score(spec, row, b));
    let right = (base + 1 < cols)
        .then(|| costas_score(spec, row, base + 1))
        .flatten();
    (fr, vertex(left, right))
}

/// Local-maximum suppression and top-K selection over a `rows × cols`
/// score grid given by `score(q, b)`, visited row by row with a rolling
/// three-row buffer. Keeps points scoring at least `min_score`, ordered by
/// score descending, then (q, b) ascending; `make` builds the entries.
pub(crate) fn select<T, S, M>(
    rows: usize,
    cols: usize,
    score: S,
    min_score: i32,
    scratch: &mut [i32],
    out: &mut [T],
    make: M,
) -> Result<SyncReport, Error>
where
    S: Fn(usize, usize) -> Option<i32>,
    M: Fn(usize, usize, i32) -> T,
    T: Copy + HasScore,
{
    if scratch.len() < 3 * cols {
        return Err(Error::BufferTooSmall);
    }
    const NONE: i32 = i32::MIN;
    let mut report = SyncReport::default();
    let mut count = 0usize;
    let fill = |scratch: &mut [i32], q: usize| {
        let slot = &mut scratch[(q % 3) * cols..(q % 3 + 1) * cols];
        for (b, s) in slot.iter_mut().enumerate() {
            *s = score(q, b).unwrap_or(NONE);
        }
    };
    if rows == 0 {
        return Ok(report);
    }
    fill(scratch, 0);
    for q in 0..rows {
        if q + 1 < rows {
            fill(scratch, q + 1);
        }
        let at = |dq: isize, b: isize| -> i32 {
            let qq = q as isize + dq;
            if qq < 0 || qq >= rows as isize || b < 0 || b >= cols as isize {
                return NONE;
            }
            scratch[(qq as usize % 3) * cols + b as usize]
        };
        for b in 0..cols {
            let s = at(0, b as isize);
            if s == NONE || s < min_score {
                continue;
            }
            let mut keep = true;
            'n: for dq in -1..=1isize {
                for db in -1..=1isize {
                    if dq == 0 && db == 0 {
                        continue;
                    }
                    let n = at(dq, b as isize + db);
                    if n == NONE {
                        continue;
                    }
                    let earlier = dq < 0 || (dq == 0 && db < 0);
                    if (earlier && s <= n) || (!earlier && s < n) {
                        keep = false;
                        break 'n;
                    }
                }
            }
            if !keep {
                continue;
            }
            report.found += 1;
            // Insert after every entry scoring ≥ s (earlier points first).
            let pos = out[..count].partition_point(|e| e.score() >= s);
            if pos >= out.len() {
                report.dropped += 1;
                continue;
            }
            if count == out.len() {
                report.dropped += 1;
            } else {
                count += 1;
            }
            out.copy_within(pos..count - 1, pos + 1);
            out[pos] = make(q, b, s);
        }
    }
    report.kept = count;
    Ok(report)
}

/// An entry with a sync score (for [`select`]).
pub(crate) trait HasScore {
    fn score(&self) -> i32;
}

impl HasScore for Candidate {
    fn score(&self) -> i32 {
        self.score
    }
}

#[cfg(test)]
mod tests;
