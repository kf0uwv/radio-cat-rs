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

use super::frontend::{Rows, Spectrogram};
use super::math::SCALAR_FORMS;
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

/// The scratch's element: a score's bits ([`cell_of`]).
pub(crate) type ScoreCell = f32;

/// A score as a scratch cell. The scratch is `f32` because the decoder
/// keeps it in the FFT's work area ([`Decoder`](super::Decoder)). A cell
/// holds the score's bits, and a cell is only ever loaded, stored and
/// turned back with [`score_of`], none of which changes bits. No
/// floating-point arithmetic touches it, so it does not matter that small
/// scores are subnormals as `f32` and negative ones NaNs.
#[inline(always)]
fn cell_of(score: i32) -> ScoreCell {
    f32::from_bits(score as u32)
}

/// The score in a scratch cell ([`cell_of`]).
#[inline(always)]
fn score_of(cell: ScoreCell) -> i32 {
    cell.to_bits() as i32
}

/// Searches `spec` (its filled rows) and fills `out` with the best
/// candidates scoring at least `min_score`, in order. `scratch` needs
/// [`scratch_len`] entries.
#[cfg(any(test, feature = "profile"))]
pub fn find_candidates(
    spec: &Spectrogram<'_>,
    min_score: i32,
    scratch: &mut [ScoreCell],
    out: &mut [Candidate],
) -> Result<SyncReport, Error> {
    search_rows(spec.rows(), min_score, scratch, out)
}

/// [`find_candidates`] over the filled rows.
pub(crate) fn search_rows(
    rows: &Rows<'_>,
    min_score: i32,
    scratch: &mut [ScoreCell],
    out: &mut [Candidate],
) -> Result<SyncReport, Error> {
    let cols = rows.band().base_bins();
    let first_row = NOMINAL_START_ROW - MAX_START_OFFSET;
    select(
        START_ROWS,
        cols,
        |q, out| costas_scores::<{ !SCALAR_FORMS }>(rows, first_row + q as i32, out),
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

/// One Costas symbol's row for [`costas_scores`] without a vector unit: adds
/// `8 · level(b + sent_at) − Σₜ level(b + 2t)` (t = 0..8, the eight tones
/// two bins apart; `sent_at` = 2 · the tone sent) to `out[b]` at every base
/// bin b < `out.len()`, from the row's levels `row`. The eight tones' sum at
/// bin b is tones 0 to 6's sum there plus tone 7's level, and less tone
/// 0's level it is tones 0 to 6's sum at b + 2: running sums along the even
/// and the odd bins, so a bin reads three levels rather than nine. The two
/// sums run side by side, a pair of bins a step, then an odd count's last
/// bin. Integers: the same values as adding the eight afresh.
///
/// Carrying tones 0 to 6's sum rather than the eight's, a step reads tone
/// 7's level at b + 14 and none past the last bin's, so the loop has no
/// last step to leave out and no bounds checks: on the Cortex-M33 (rustc
/// 1.93.0, opt-level "s") 13 instructions a bin, one parity at a time,
/// against 15 with the eight's sum. Running the two parities side by side
/// takes that to 25 a pair, 12.5 a bin, saving about 0.34% more of the
/// small slot (findings O32).
#[inline(always)]
fn add_row_running(row: &[u8], sent_at: usize, out: &mut [ScoreCell]) {
    let cols = out.len();
    // Tone 7's level at the last bin is the row's last one used.
    let row = &row[..cols + 14];
    let sent = &row[sent_at..sent_at + cols];
    let level = |k: usize| i32::from(row[k]);
    // Tones 0 to 6's sums at bins 0 and 1.
    let mut even = (0..7).fold(0, |s, t| s + level(2 * t));
    let mut odd = (0..7).fold(0, |s, t| s + level(1 + 2 * t));
    let mut b = 0;
    while b + 1 < cols {
        let all = even + level(b + 14);
        out[b] = cell_of(score_of(out[b]) + 8 * i32::from(sent[b]) - all);
        even = all - level(b);
        let all = odd + level(b + 15);
        out[b + 1] = cell_of(score_of(out[b + 1]) + 8 * i32::from(sent[b + 1]) - all);
        odd = all - level(b + 1);
        b += 2;
    }
    if b < cols {
        let all = even + level(b + 14);
        out[b] = cell_of(score_of(out[b]) + 8 * i32::from(sent[b]) - all);
    }
}

/// Rows a start row's score reads: every symbol of every Costas array.
const COSTAS_ROWS: usize = COSTAS_POSITIONS.len() * COSTAS.len();

/// The rows one start row's scores read, for both forms of
/// [`costas_scores`]: calls `f` with the levels of each symbol of every
/// Costas array of `start_row` that lies in `spec`'s filled rows, in order,
/// and its sent tone as a bin offset (2 · tone). Returns the scale, 6 / the
/// arrays present (2 or 3: × 3 or × 2, exact), or `None` if fewer than two
/// arrays are present (no score, as [`costas_score`]).
// A visitor rather than an array of the rows: the M33's running sums take
// each row as they read it, as before; an array of the rows grew its
// images by 196 B of code (findings O29b).
#[inline(always)]
fn gather<'a>(
    spec: &'a Rows<'_>,
    start_row: i32,
    mut f: impl FnMut(&'a [u8], usize),
) -> Option<i32> {
    let rows = spec.len() as i32;
    let mut present = 0i32;
    for &pos in COSTAS_POSITIONS.iter() {
        let first = start_row + 2 * pos as i32;
        let last = first + 2 * (COSTAS.len() as i32 - 1);
        if first < 0 || last >= rows {
            continue;
        }
        for (j, &tone) in COSTAS.iter().enumerate() {
            // `None` is unreachable (rows below `rows_filled` are filled);
            // as `costas_score`, no score.
            f(
                spec.row((first + 2 * j as i32) as usize)?,
                2 * usize::from(tone),
            );
        }
        present += 1;
    }
    (present >= 2).then(|| 6 / present)
}

/// [`costas_score`] at every base bin `0..out.len()` of one start row, into
/// `out` as cells ([`NONE`] where there is none): the same integer sums,
/// added in another order (exact), from the rows [`gather`] reads. With
/// `VECTOR_FORM` (where the target has a vector unit) the rows' levels are
/// summed down each column first ([`costas_scores_by_columns`], which
/// vectorises); otherwise by running sums along each row
/// ([`costas_scores_running`]).
// `spec` by reference: by value (a `Copy` view of the rows) the M33's
// running sums keep their loop (codegen-check `sumloop6`) and the sync stage
// its count, but the small slot rose 0.38% (s0086, in the feed): findings
// O25b.
fn costas_scores<const VECTOR_FORM: bool>(spec: &Rows<'_>, start_row: i32, out: &mut [ScoreCell]) {
    let scored = if VECTOR_FORM {
        costas_scores_by_columns(spec, start_row, out)
    } else {
        costas_scores_running(spec, start_row, out)
    };
    if scored.is_none() {
        out.fill(cell_of(NONE));
    }
}

/// [`costas_scores`] without a vector unit: each row's running sums
/// ([`add_row_running`]) added along the bins, then scaled; `None` as
/// [`gather`].
#[inline(always)]
fn costas_scores_running(spec: &Rows<'_>, start_row: i32, out: &mut [ScoreCell]) -> Option<()> {
    out.fill(cell_of(0));
    let scale = gather(spec, start_row, |row, at| add_row_running(row, at, out))?;
    for s in out.iter_mut() {
        *s = cell_of(score_of(*s) * scale);
    }
    Some(())
}

/// Base bins [`costas_scores_by_columns`] takes at a time. Not tuned: 64
/// was the one size timed (findings O29).
const COLUMN_CHUNK: usize = 64;

/// [`costas_scores`] where the target has a vector unit. A row's eight
/// tones' sum does not depend on the tone sent, so the levels of all the
/// rows read (up to [`COSTAS_ROWS`]) are first added down each column, and
/// each bin then takes eight of those column sums rather than eight levels
/// a row: the score is 8 · Σ_rows level(b + 2·tone) − Σₜ column(b + 2t). A
/// bin reads about 2 · 21 + 8 sums and levels instead of 21 · 9 levels
/// (findings O29). The bins go [`COLUMN_CHUNK`] at a time, so the column
/// and sent sums need only small arrays on the stack. They are `u16`: each
/// is a sum of at most [`COSTAS_ROWS`] `u8` levels (checked at compile time
/// beside the arrays); a bin's eight column sums and its score are added
/// in `i32`. `None` as [`gather`].
#[inline(always)]
fn costas_scores_by_columns(spec: &Rows<'_>, start_row: i32, out: &mut [ScoreCell]) -> Option<()> {
    let cols = out.len();
    let mut read: [&[u8]; COSTAS_ROWS] = [&[]; COSTAS_ROWS];
    let mut sent_at = [0u8; COSTAS_ROWS];
    let mut n = 0;
    let scale = gather(spec, start_row, |row, at| {
        // Tone 7's level at the last bin is the row's last one used.
        read[n] = &row[..cols + 14];
        // At most 2 · 7.
        debug_assert!(at <= usize::from(u8::MAX));
        sent_at[n] = at as u8;
        n += 1;
    })?;
    let (read, sent_at) = (&read[..n], &sent_at[..n]);
    // A column or sent sum adds at most `COSTAS_ROWS` levels of a `u8`.
    const _: () = assert!(COSTAS_ROWS as u32 * u8::MAX as u32 <= u16::MAX as u32);
    for (c, chunk) in out.chunks_mut(COLUMN_CHUNK).enumerate() {
        let first = c * COLUMN_CHUNK;
        let len = chunk.len();
        let mut column = [0u16; COLUMN_CHUNK + 14];
        let mut sent = [0u16; COLUMN_CHUNK];
        for (row, &at) in read.iter().zip(sent_at) {
            let at = usize::from(at);
            for (x, &v) in column[..len + 14]
                .iter_mut()
                .zip(&row[first..first + len + 14])
            {
                *x += u16::from(v);
            }
            for (x, &v) in sent[..len]
                .iter_mut()
                .zip(&row[first + at..first + at + len])
            {
                *x += u16::from(v);
            }
        }
        for (b, cell) in chunk.iter_mut().enumerate() {
            let all: i32 = (0..8).map(|t| i32::from(column[b + 2 * t])).sum();
            *cell = cell_of((8 * i32::from(sent[b]) - all) * scale);
        }
    }
    Some(())
}

/// The sync score of a candidate at `start_row`, base bin `base`, or
/// `None` if fewer than two Costas arrays lie in the filled rows.
pub(crate) fn costas_score(spec: &Spectrogram<'_>, start_row: i32, base: usize) -> Option<i32> {
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
                all += i32::from(row[base + 2 * t]);
            }
            sum += 8 * i32::from(row[base + 2 * usize::from(tone)]) - all;
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
pub fn refine(spec: &Spectrogram<'_>, c: &Candidate) -> (f32, f32) {
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

/// No score at this point of the grid.
const NONE: i32 = i32::MIN;

/// Local-maximum suppression and top-K selection over a `rows × cols` score
/// grid whose row q `score(q, row)` fills with cells ([`NONE`] for no score),
/// visited row by row with a rolling three-row buffer. Every row is filled
/// before it is read, so the scratch's earlier contents do not matter and it
/// holds nothing afterwards. Keeps points scoring at least `min_score`,
/// ordered by score descending, then (q, b) ascending; `make` builds entries.
pub(crate) fn select<T, S, M>(
    rows: usize,
    cols: usize,
    score: S,
    min_score: i32,
    scratch: &mut [ScoreCell],
    out: &mut [T],
    make: M,
) -> Result<SyncReport, Error>
where
    S: Fn(usize, &mut [ScoreCell]),
    M: Fn(usize, usize, i32) -> T,
    T: Copy + HasScore,
{
    if scratch.len() < 3 * cols {
        return Err(Error::BufferTooSmall);
    }
    let mut report = SyncReport::default();
    let mut count = 0usize;
    let fill = |scratch: &mut [ScoreCell], q: usize| {
        score(q, &mut scratch[(q % 3) * cols..(q % 3 + 1) * cols]);
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
            score_of(scratch[(qq as usize % 3) * cols + b as usize])
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
