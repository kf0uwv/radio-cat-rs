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

/// Scratch for one search: three rows of scores ([`scratch_width`] bins).
pub const fn scratch_len(base_bins: usize) -> usize {
    3 * scratch_width(base_bins)
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
    if SCALAR_FORMS {
        return search_paired(rows, min_score, scratch, out);
    }
    let cols = rows.band().base_bins();
    let first_row = NOMINAL_START_ROW - MAX_START_OFFSET;
    select(
        START_ROWS,
        cols,
        |q, out| costas_scores(rows, first_row + q as i32, out),
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

/// Rows a start row's score reads: every symbol of every Costas array.
const COSTAS_ROWS: usize = COSTAS_POSITIONS.len() * COSTAS.len();

/// [`costas_score`] at every base bin `0..out.len()` of one start row, into
/// `out` as cells ([`NONE`] where there is none): the same integer sums,
/// added in another order (exact), the rows' levels summed down each column
/// first, vectorised ([`costas_scores_by_columns`]). The search uses it
/// where the target has a vector unit; without one it scores start rows in
/// pairs ([`search_paired`]).
fn costas_scores(spec: &Rows<'_>, start_row: i32, out: &mut [ScoreCell]) {
    costas_scores_by_columns(spec, start_row, out);
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
/// in `i32`. The rows are read in this function's own loop, their sent
/// tones' offsets kept as `usize`: through a shared visitor with `u8`
/// offsets the sync stage took 1.1-2.0% more time on the Cortex-A76
/// (findings O35).
#[inline(always)]
fn costas_scores_by_columns(spec: &Rows<'_>, start_row: i32, out: &mut [ScoreCell]) {
    let cols = out.len();
    let rows = spec.len() as i32;
    let mut read: [&[u8]; COSTAS_ROWS] = [&[]; COSTAS_ROWS];
    let mut sent_at = [0usize; COSTAS_ROWS];
    let mut n = 0;
    let mut present = 0i32;
    for &pos in COSTAS_POSITIONS.iter() {
        let first = start_row + 2 * pos as i32;
        let last = first + 2 * (COSTAS.len() as i32 - 1);
        if first < 0 || last >= rows {
            continue;
        }
        for (j, &tone) in COSTAS.iter().enumerate() {
            let Some(row) = spec.row((first + 2 * j as i32) as usize) else {
                // Unreachable (rows below `rows_filled` are filled); as
                // `costas_score`, no score.
                out.fill(cell_of(NONE));
                return;
            };
            // Tone 7's level at the last bin is the row's last one used.
            read[n] = &row[..cols + 14];
            sent_at[n] = 2 * usize::from(tone);
            n += 1;
        }
        present += 1;
    }
    if present < 2 {
        out.fill(cell_of(NONE));
        return;
    }
    // 6 / n with n = 2 or 3: × 3 or × 2, exact.
    let scale = 6 / present;
    let (read, sent_at) = (&read[..n], &sent_at[..n]);
    // A column or sent sum adds at most `COSTAS_ROWS` levels of a `u8`.
    const _: () = assert!(COSTAS_ROWS as u32 * u8::MAX as u32 <= u16::MAX as u32);
    for (c, chunk) in out.chunks_mut(COLUMN_CHUNK).enumerate() {
        let first = c * COLUMN_CHUNK;
        let len = chunk.len();
        let mut column = [0u16; COLUMN_CHUNK + 14];
        let mut sent = [0u16; COLUMN_CHUNK];
        for (row, &at) in read.iter().zip(sent_at) {
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

/// Places a local maximum, `entry`, in `out`, whose first `count` entries
/// are ordered: after every entry `before` holds of (a prefix of them). If
/// the list is full, the entry or the list's last is dropped. Counts it in
/// `report` as found, and as dropped if so. For [`select`] and
/// [`select_paired`], as the last statement of their loop over a row's
/// bins, which a dropped entry `continue`s.
// A macro rather than a function: as a function (inlined always or as the
// compiler chose, with an early return or without, its entry built at once
// or by a closure) the insertion cost the Cortex-M33's sync stage 1.17%
// (about 0.10% of the small slot) and grew the hosts' `search_rows`
// (x86-64: 464 or 465 instructions, against 349); so did this macro with
// an `else` or a labelled block for its `continue` (441, 447). The
// statements as both selections wrote them keep it (findings O33b). The
// list, its count and the report are names, so each is a place, read and
// written as such.
macro_rules! insert_ranked {
    ($out:ident, $count:ident, $report:ident, $before:expr, $entry:expr $(,)?) => {{
        $report.found += 1;
        let pos = $out[..$count].partition_point($before);
        if pos >= $out.len() {
            $report.dropped += 1;
            continue;
        }
        if $count == $out.len() {
            $report.dropped += 1;
        } else {
            $count += 1;
        }
        $out.copy_within(pos..$count - 1, pos + 1);
        $out[pos] = $entry;
    }};
}

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
            // After every entry scoring ≥ s (found in (q, b) order, so
            // earlier points first).
            insert_ranked!(out, count, report, |e| e.score() >= s, make(q, b, s));
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

// Start rows q and q + 2 read the same rows but one of each array: q's
// symbol j is q + 2's symbol j − 1 (rows two apart, a symbol apart). Where
// the target has no vector unit ([`SCALAR_FORMS`]) the search scores the
// two together ([`costas_scores_paired`]), so the rows they share run their
// running sums once for both, and keeps the scores of up to five start
// rows ([`PAIRED_ROWS`]): [`search_paired`]. Its local maxima read each
// neighbour from three rows' slices, with no row index taken mod 5 a
// neighbour. On the Cortex-M33 (rustc 1.93.0, opt-level "s") the sync
// stage took 32.9% fewer instructions and the small slot 4.0% fewer
// (s0086; findings O33), for about 1.2 KB of code.

/// Score rows the paired search keeps: rows q − 1 to q + 1 for the local
/// maxima of row q, and the q + 2 and q + 3 scored with q and q + 1.
const PAIRED_ROWS: usize = 5;

/// The bins of a row of [`scratch_len`]: `base_bins`, or where the
/// target has no vector unit ([`SCALAR_FORMS`]) [`paired_width`].
const fn scratch_width(base_bins: usize) -> usize {
    if SCALAR_FORMS {
        paired_width(base_bins)
    } else {
        base_bins
    }
}

/// The bins of a row of [`select_paired`]'s scratch, which takes three of
/// them: `base_bins`, and at least [`PAIRED_ROWS`], so that its five rows
/// hold spans of at least one bin and its two neighbours. The same for
/// every band `Band::from_hz` gives (nine bins or more).
const fn paired_width(base_bins: usize) -> usize {
    if base_bins < PAIRED_ROWS {
        PAIRED_ROWS
    } else {
        base_bins
    }
}

/// [`search_rows`] where the target has no vector unit: the same
/// candidates, in the same order, and the same report, from start rows
/// scored in pairs ([`costas_scores_paired`]) and the local maxima of
/// [`select_paired`]. `scratch` needs three rows of [`paired_width`]
/// bins ([`scratch_len`] where the target has no vector unit).
pub(crate) fn search_paired(
    rows: &Rows<'_>,
    min_score: i32,
    scratch: &mut [ScoreCell],
    out: &mut [Candidate],
) -> Result<SyncReport, Error> {
    let cols = rows.band().base_bins();
    let first_row = NOMINAL_START_ROW - MAX_START_OFFSET;
    select_paired(
        START_ROWS,
        cols,
        |q, lo, a, b| costas_scores_paired(rows, first_row + q as i32, lo, a, b),
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

/// [`costas_scores`] of start rows `start_row` into `a` and `start_row` + 2
/// into `b` (if given), at base bins `lo..lo + a.len()`: the same integer
/// sums ([`costas_score`]), added in another order (exact). An array both
/// rows have shares six of its rows: each such row's running sums
/// ([`add_row_paired`]) serve both, and each start row adds its own one
/// ([`add_row_running`]). An array only one of them has is added to that
/// one alone. Where fewer than two arrays are present, [`NONE`].
// Every row read is below `spec.len()`, so `Rows::row` is `Some`; were one
// `None`, neither start row would have a score (`NONE` in both), as
// `costas_scores` leaves none. `spec` by reference, as the search before pairs
// took it: by value (a `Copy` view of the rows) the Cortex-M33's running
// sums kept their loop (codegen-check `sumloop6`) and the sync stage its
// count, but the small slot rose 0.38% (s0086, in the feed): findings
// O25b. Not tried by value since.
fn costas_scores_paired(
    spec: &Rows<'_>,
    start_row: i32,
    lo: usize,
    a: &mut [ScoreCell],
    mut b: Option<&mut [ScoreCell]>,
) {
    let rows = spec.len() as i32;
    let tone = |j: usize| 2 * usize::from(COSTAS[j]);
    let last = 2 * (COSTAS.len() as i32 - 1);
    a.fill(cell_of(0));
    if let Some(b) = b.as_deref_mut() {
        b.fill(cell_of(0));
    }
    let (mut in_a, mut in_b) = (0i32, 0i32);
    for &pos in COSTAS_POSITIONS.iter() {
        let first = start_row + 2 * pos as i32;
        // Row j of the eight: symbol j of `start_row`, j − 1 of the other.
        let row =
            |j: usize| -> Option<&[u8]> { Some(&spec.row((first + 2 * j as i32) as usize)?[lo..]) };
        let has_a = first >= 0 && first + last < rows;
        let has_b = b.is_some() && first + 2 >= 0 && first + 2 + last < rows;
        for j in 0..=COSTAS.len() {
            let (to_a, to_b) = (has_a && j < COSTAS.len(), has_b && j > 0);
            if !to_a && !to_b {
                continue;
            }
            let Some(levels) = row(j) else {
                a.fill(cell_of(NONE));
                if let Some(b) = b {
                    b.fill(cell_of(NONE));
                }
                return;
            };
            // A row for neither was skipped, so one not for `b` is for `a`;
            // `to_b` holds only if `b` is given.
            if !to_b {
                add_row_alone(levels, tone(j), a);
            } else if let Some(b) = b.as_deref_mut() {
                if to_a {
                    add_row_pair(levels, tone(j), tone(j - 1), a, b);
                } else {
                    add_row_alone(levels, tone(j - 1), b);
                }
            }
        }
        in_a += i32::from(has_a);
        in_b += i32::from(has_b);
    }
    let finish = |out: &mut [ScoreCell], present: i32| {
        if present >= 2 {
            // 6 / n with n = 2 or 3: × 3 or × 2, exact.
            let scale = if present == 3 { 2 } else { 3 };
            for s in out.iter_mut() {
                *s = cell_of(score_of(*s) * scale);
            }
        } else {
            out.fill(cell_of(NONE));
        }
    };
    finish(a, in_a);
    if let Some(b) = b {
        finish(b, in_b);
    }
}

/// One Costas symbol's row of one start row, for [`costas_scores_paired`]
/// (through [`add_row_alone`]): adds `8 · level(b + sent_at) − Σₜ level(b +
/// 2t)` (t = 0..8, the eight tones two bins apart; `sent_at` = 2 · the tone
/// sent) to `out[b]` at every base bin b < `out.len()`, from the row's
/// levels `row`. The eight tones' sum at bin b is tones 0 to 6's sum there
/// plus tone 7's level, and less tone 0's level it is tones 0 to 6's sum
/// at b + 2: running sums along the even and the odd bins, so a bin reads
/// three levels rather than nine. The two sums run side by side, a pair of
/// bins a step, then an odd count's last bin. Integers: the same values as
/// adding the eight afresh.
///
/// Carrying tones 0 to 6's sum rather than the eight's, a step reads tone
/// 7's level at b + 14 and none past the last bin's, so the loop has no
/// last step to leave out and no bounds checks: on the Cortex-M33 (rustc
/// 1.93.0, opt-level "s") 13 instructions a bin, one parity at a time,
/// against 15 with the eight's sum. Running the two parities side by side
/// takes that to 25 a pair, 12.5 a bin, saving about 0.34% more of the
/// small slot in the search before start rows were paired (findings O32).
/// [`add_row_paired`], whose step serves two start rows, runs one parity
/// at a time instead: 18 instructions a bin (findings O33).
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

/// [`add_row_running`] out of line: one copy for the paired search's rows of
/// one start row.
#[inline(never)]
fn add_row_alone(row: &[u8], sent_at: usize, out: &mut [ScoreCell]) {
    add_row_running(row, sent_at, out);
}

/// [`add_row_paired`] out of line.
#[inline(never)]
fn add_row_pair(
    row: &[u8],
    sent_a: usize,
    sent_b: usize,
    a: &mut [ScoreCell],
    b: &mut [ScoreCell],
) {
    add_row_paired(row, sent_a, sent_b, a, b);
}

/// [`add_row_running`] into two score rows at once: adds
/// `8 · level(x + sent_a) − Σₜ level(x + 2t)` to `a[x]` and the same with
/// `sent_b` to `b[x]`, at every base bin x < `a.len()`. The eight tones'
/// running sums are the row's, whichever tone was sent, so both rows take
/// them from one pass. `b` is as long as `a`.
#[inline(always)]
fn add_row_paired(
    row: &[u8],
    sent_a: usize,
    sent_b: usize,
    a: &mut [ScoreCell],
    b: &mut [ScoreCell],
) {
    let cols = a.len();
    let b = &mut b[..cols];
    // Tone 7's level at the last bin is the row's last one used.
    let row = &row[..cols + 14];
    let (sa, sb) = (&row[sent_a..sent_a + cols], &row[sent_b..sent_b + cols]);
    let level = |k: usize| i32::from(row[k]);
    let add = |cell: &mut ScoreCell, sent: u8, all: i32| {
        *cell = cell_of(score_of(*cell) + 8 * i32::from(sent) - all);
    };
    for parity in 0..2 {
        // Tones 0 to 6's sum at bin `parity`.
        let mut sum = (0..7).fold(0, |s, t| s + level(parity + 2 * t));
        let mut x = parity;
        while x < cols {
            let all = sum + level(x + 14);
            add(&mut a[x], sa[x], all);
            add(&mut b[x], sb[x], all);
            sum = all - level(x);
            x += 2;
        }
    }
}

/// An entry of [`select_paired`]'s list: its score and its grid point
/// (q, b), by which equal scores are ordered.
pub(crate) trait GridPoint: HasScore {
    fn point(&self) -> (usize, usize);
}

impl GridPoint for Candidate {
    fn point(&self) -> (usize, usize) {
        let q = i32::from(self.start_row) - (NOMINAL_START_ROW - MAX_START_OFFSET);
        (q as usize, usize::from(self.base))
    }
}

/// [`select`] over a grid whose rows are scored in pairs: `score(q, lo,
/// a, b)` fills row q into `a` and row q + 2 into `b` (`None` past the
/// last row), base bins `lo..lo + a.len()`, for q = 0, 1, 4, 5, 8, …. The
/// same points are kept, ordered by score descending, then (q, b)
/// ascending, and the same report.
///
/// Five rows of scores ([`PAIRED_ROWS`]) are live at most: before row q's
/// maxima, rows q − 1 to q + 1 are needed, and a pair is scored when its
/// first row is (q + 1 ≡ 0 or 1 mod 4), so row q + 2 may be held too.
/// Every live row lies within five consecutive rows, so row q keeps slot
/// q mod 5. Where five rows of the band do not fit the scratch, the bins
/// are taken in spans, each scored one bin beyond on either side for its
/// neighbours, and the entries are placed by their score and grid point
/// rather than by the order they are found in (which no longer follows
/// (q, b)), so the list is the same.
pub(crate) fn select_paired<T, S, M>(
    rows: usize,
    cols: usize,
    score: S,
    min_score: i32,
    scratch: &mut [ScoreCell],
    out: &mut [T],
    make: M,
) -> Result<SyncReport, Error>
where
    S: Fn(usize, usize, &mut [ScoreCell], Option<&mut [ScoreCell]>),
    M: Fn(usize, usize, i32) -> T,
    T: Copy + GridPoint,
{
    if scratch.len() < 3 * paired_width(cols) {
        return Err(Error::BufferTooSmall);
    }
    let mut report = SyncReport::default();
    if rows == 0 {
        return Ok(report);
    }
    let mut count = 0usize;
    // Bins a span selects: all of them if five rows of the band fit, else
    // as many as leave room for a neighbour on either side.
    let fit = scratch.len() / PAIRED_ROWS;
    let span = if cols <= fit { cols } else { fit - 2 };
    let mut c0 = 0;
    while c0 < cols {
        let c1 = (c0 + span).min(cols);
        // The bins scored: the span and a neighbour either side.
        let lo = c0.saturating_sub(1);
        let width = (c1 + 1).min(cols) - lo;
        // Rows q and q + 2 into slots `x` (q mod 5) and `y` ((q + 2) mod 5).
        let fill = |scratch: &mut [ScoreCell], q: usize, x: usize, y: usize| {
            let (first, second) = scratch.split_at_mut(x.max(y) * width);
            let (a, b) = if x < y {
                (&mut first[x * width..(x + 1) * width], &mut second[..width])
            } else {
                (&mut second[..width], &mut first[y * width..(y + 1) * width])
            };
            score(q, lo, a, (q + 2 < rows).then_some(b));
        };
        // Row q's slot, q mod 5, kept as q steps (no division a row).
        let after = |slot: usize| if slot + 1 == PAIRED_ROWS { 0 } else { slot + 1 };
        // The first pair, rows 0 and 2; the rest as their first row is
        // needed.
        fill(scratch, 0, 0, 2);
        let mut slot = 0;
        for q in 0..rows {
            if (q + 1) % 4 < 2 && q + 1 < rows {
                let next = after(slot);
                fill(scratch, q + 1, next, after(after(next)));
            }
            // Rows q − 1, q and q + 1 at the bins scored; none past the
            // grid's rows. A bin's neighbours past the band's bins are
            // past the slice. A neighbour with no score (`NONE`, the least
            // `i32`) beats no score.
            let row = |slot: usize| &scratch[slot * width..][..width];
            let before = if slot == 0 { PAIRED_ROWS - 1 } else { slot - 1 };
            let (above, here) = ((q > 0).then(|| row(before)), row(slot));
            let below = (q + 1 < rows).then(|| row(after(slot)));
            slot = after(slot);
            for i in c0 - lo..c1 - lo {
                let s = score_of(here[i]);
                if s == NONE || s < min_score {
                    continue;
                }
                // Earlier neighbours beat s if they score as much, later
                // ones if they score more.
                let (left, right) = (i > 0, i + 1 < width);
                let earlier = |r: &[ScoreCell], i: usize| s <= score_of(r[i]);
                let later = |r: &[ScoreCell], i: usize| s < score_of(r[i]);
                let beaten = (left && earlier(here, i - 1))
                    || (right && later(here, i + 1))
                    || above.is_some_and(|r| {
                        earlier(r, i) || (left && earlier(r, i - 1)) || (right && earlier(r, i + 1))
                    })
                    || below.is_some_and(|r| {
                        later(r, i) || (left && later(r, i - 1)) || (right && later(r, i + 1))
                    });
                if beaten {
                    continue;
                }
                let b = lo + i;
                // After every entry scoring more, or as much at an earlier
                // point.
                insert_ranked!(
                    out,
                    count,
                    report,
                    |e| e.score() > s || (e.score() == s && e.point() < (q, b)),
                    make(q, b, s),
                );
            }
        }
        c0 = c1;
    }
    report.kept = count;
    Ok(report)
}

#[cfg(test)]
mod tests;
