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

//! The coarse panorama: a spectrum trace and a half-block waterfall.
//!
//! ADR 0013 §2(a) in one module. The GUI draws 4096 bins at 60 fps; a
//! terminal draws as many bins as it has columns, at whatever rate the
//! console redraws. That is a **fidelity** exception. It is not a licence
//! to draw nothing, and this module exists so that no app is tempted to.

use crate::{BLOCKS, UPPER_HALF};
use cat_signal::SpectrumFrame;
use ratatui::{buffer::Buffer, layout::Rect, style::Color};

/// Colour ramp for waterfall intensity, cold to hot.
///
/// Deliberately built from a small ordered set rather than a smooth
/// gradient: a terminal has a fixed palette, and picking from it directly
/// is both honest about the medium and cheap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaterfallPalette {
    stops: &'static [Color],
}

impl Default for WaterfallPalette {
    fn default() -> Self {
        Self::TURBO
    }
}

impl WaterfallPalette {
    /// Blue through green to red — the convention every panadapter uses,
    /// so an operator's existing instinct transfers.
    pub const TURBO: Self = Self {
        stops: &[
            Color::Indexed(17),
            Color::Indexed(19),
            Color::Indexed(26),
            Color::Indexed(37),
            Color::Indexed(43),
            Color::Indexed(46),
            Color::Indexed(118),
            Color::Indexed(184),
            Color::Indexed(214),
            Color::Indexed(202),
            Color::Indexed(196),
        ],
    };

    /// For terminals that only have the 16 ANSI colours.
    pub const ANSI16: Self = Self {
        stops: &[
            Color::Black,
            Color::Blue,
            Color::Cyan,
            Color::Green,
            Color::Yellow,
            Color::Red,
        ],
    };

    /// Pick a colour for a 0.0–1.0 intensity, clamped.
    pub fn at(&self, t: f32) -> Color {
        if self.stops.is_empty() {
            return Color::Reset;
        }
        let t = t.clamp(0.0, 1.0);
        let i = (t * (self.stops.len() - 1) as f32).round() as usize;
        self.stops[i.min(self.stops.len() - 1)]
    }
}

/// Where a bin from `frame` lands, in columns, relative to `reference`.
///
/// This is the re-projection that keeps a waterfall's frequency axis true
/// for every row and not merely its newest. It matters because an IF tap is
/// **dial-centred**: the SDR is parked on the IF and the radio's LO tracks
/// the dial, so every retune changes a frame's `center_hz` and a row
/// captured a moment ago describes a different slice of spectrum.
///
/// For a source whose centre never moves — a `DirectSdr` with a fixed
/// tuner — every row shares a centre and this is a no-op. That is why it is
/// safe to do unconditionally here rather than making it a per-radio policy.
fn column_offset(frame: &SpectrumFrame, reference: &SpectrumFrame, width: u16) -> i32 {
    if reference.span_hz == 0 || width == 0 {
        return 0;
    }
    let delta = frame.center_hz as i64 - reference.center_hz as i64;
    ((delta as f64 / f64::from(reference.span_hz)) * f64::from(width)).round() as i32
}

/// Normalize a bin's power to 0.0–1.0 against a floor and a reference.
fn intensity(dbm: f32, floor_dbm: f32, ref_dbm: f32) -> f32 {
    if (ref_dbm - floor_dbm).abs() < f32::EPSILON {
        return 0.0;
    }
    ((dbm - floor_dbm) / (ref_dbm - floor_dbm)).clamp(0.0, 1.0)
}

/// Draw one frame as a column-per-cell trace, eight sub-levels per row.
///
/// Bin 0 is the lowest frequency and is drawn leftmost, which is only
/// correct because `SpectrumFrame` guarantees low-frequency-first. A
/// renderer that reversed this would put a signal above the dial on the
/// wrong side — the failure `cat-signal`'s whole invariant exists to
/// prevent, and the one a fake source cannot catch for you.
pub fn spectrum_trace(
    frame: &SpectrumFrame,
    area: Rect,
    buf: &mut Buffer,
    floor_dbm: f32,
    color: Color,
) {
    if area.width == 0 || area.height == 0 || frame.bins.is_empty() {
        return;
    }
    let levels = u32::from(area.height) * 8;
    for col in 0..area.width {
        let bin = bin_for_column(frame.bins.len(), col, area.width);
        let t = intensity(frame.bins[bin], floor_dbm, frame.ref_level_dbm);
        let filled = (t * levels as f32).round() as u32;
        for row in 0..area.height {
            // Rows are drawn bottom-up: the lowest row holds the first 8
            // levels, so a weak signal is a stub at the bottom rather than
            // a full column of nothing.
            let from_bottom = area.height - 1 - row;
            let base = u32::from(from_bottom) * 8;
            let cell_level = filled.saturating_sub(base).min(8) as usize;
            if cell_level > 0 {
                buf.get_mut(area.x + col, area.y + row)
                    .set_char(BLOCKS[cell_level])
                    .set_fg(color);
            }
        }
    }
}

/// Which bin a column samples. Nearest-bin, not averaged: a terminal has
/// far fewer columns than bins, and averaging would bury a narrow carrier
/// that is exactly what an operator is looking for.
fn bin_for_column(bins: usize, col: u16, width: u16) -> usize {
    if width <= 1 {
        return 0;
    }
    let f = f64::from(col) / f64::from(width - 1);
    ((f * (bins - 1) as f64).round() as usize).min(bins - 1)
}

/// Draw a waterfall, newest row at the top, two history rows per text row.
///
/// `frames` is newest-first. Rows are re-projected onto the newest frame's
/// axis, so a stationary signal holds a vertical column. Columns a row
/// never captured are left as `absent` rather than wrapped or filled with
/// noise — showing data in the wrong place would be worse than showing
/// none, and the staircase those gaps make is itself the tuning history.
pub fn waterfall(
    frames: &[SpectrumFrame],
    area: Rect,
    buf: &mut Buffer,
    palette: WaterfallPalette,
    floor_dbm: f32,
    absent: Color,
) {
    if area.width == 0 || area.height == 0 || frames.is_empty() {
        return;
    }
    let reference = &frames[0];

    for row in 0..area.height {
        // Two history frames per text row: foreground is the upper half.
        let top = frames.get(usize::from(row) * 2);
        let bottom = frames.get(usize::from(row) * 2 + 1);
        for col in 0..area.width {
            let cell = buf.get_mut(area.x + col, area.y + row);
            cell.set_char(UPPER_HALF);
            cell.set_fg(sample(
                top, reference, col, area.width, floor_dbm, palette, absent,
            ));
            cell.set_bg(sample(
                bottom, reference, col, area.width, floor_dbm, palette, absent,
            ));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn sample(
    frame: Option<&SpectrumFrame>,
    reference: &SpectrumFrame,
    col: u16,
    width: u16,
    floor_dbm: f32,
    palette: WaterfallPalette,
    absent: Color,
) -> Color {
    let Some(frame) = frame else { return absent };
    if frame.bins.is_empty() {
        return absent;
    }
    let offset = column_offset(frame, reference, width);
    let src = i32::from(col) - offset;
    if src < 0 || src >= i32::from(width) {
        // This row never saw this frequency.
        return absent;
    }
    let bin = bin_for_column(frame.bins.len(), src as u16, width);
    palette.at(intensity(frame.bins[bin], floor_dbm, frame.ref_level_dbm))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn frame(center_hz: u64, peak_bin: usize, bins: usize) -> SpectrumFrame {
        let mut v = vec![-110.0f32; bins];
        v[peak_bin] = -20.0;
        SpectrumFrame {
            center_hz,
            span_hz: 96_000,
            ref_level_dbm: -20.0,
            bins: v,
            sequence: 1,
        }
    }

    fn buffer(w: u16, h: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, w, h))
    }

    fn area(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    /// The column with any drawn glyph, for a single-peak trace.
    fn drawn_columns(buf: &Buffer, a: Rect) -> Vec<u16> {
        (0..a.width)
            .filter(|c| (0..a.height).any(|r| buf.get(a.x + c, a.y + r).symbol() != " "))
            .collect()
    }

    // -----------------------------------------------------------------
    // The invariant. A terminal renderer can break orientation just as
    // easily as a source can, and this is where it would show.
    // -----------------------------------------------------------------

    #[test]
    fn a_signal_above_the_dial_draws_to_the_right() {
        // Bin 0 is the lowest frequency, so a peak in the upper half of the
        // bins must appear in the right half of the columns. A renderer
        // that iterated bins backwards would pass every other test here.
        let a = area(64, 4);
        let mut buf = buffer(64, 4);
        spectrum_trace(
            &frame(14_074_000, 200, 256),
            a,
            &mut buf,
            -120.0,
            Color::Green,
        );

        let cols = drawn_columns(&buf, a);
        let peak = *cols.last().unwrap();
        assert!(
            peak > a.width / 2,
            "peak drew at column {peak} of {}, expected the right half",
            a.width
        );
    }

    #[test]
    fn a_signal_below_the_dial_draws_to_the_left() {
        let a = area(64, 4);
        let mut buf = buffer(64, 4);
        spectrum_trace(
            &frame(14_074_000, 20, 256),
            a,
            &mut buf,
            -120.0,
            Color::Green,
        );
        assert!(drawn_columns(&buf, a)[0] < a.width / 2);
    }

    #[test]
    fn a_stronger_signal_draws_a_taller_column() {
        let a = area(8, 4);
        let mut weak = buffer(8, 4);
        let mut strong = buffer(8, 4);

        let mut f = frame(14_074_000, 4, 8);
        f.bins[4] = -90.0;
        spectrum_trace(&f, a, &mut weak, -120.0, Color::Green);
        f.bins[4] = -20.0;
        spectrum_trace(&f, a, &mut strong, -120.0, Color::Green);

        let height = |b: &Buffer| (0..4).filter(|r| b.get(4, *r).symbol() != " ").count();
        assert!(height(&strong) > height(&weak));
    }

    #[test]
    fn a_weak_signal_grows_from_the_bottom_not_the_top() {
        // A stub at the top of the panel would read as a strong signal
        // clipped, which is the opposite of the truth.
        let a = area(4, 4);
        let mut buf = buffer(4, 4);
        let mut f = frame(14_074_000, 2, 4);
        f.bins[2] = -105.0;
        spectrum_trace(&f, a, &mut buf, -120.0, Color::Green);
        assert_eq!(buf.get(2, 0).symbol(), " ", "top row should be empty");
        assert_ne!(buf.get(2, 3).symbol(), " ", "bottom row should be drawn");
    }

    #[test]
    fn an_empty_frame_draws_nothing_rather_than_panicking() {
        let a = area(8, 2);
        let mut buf = buffer(8, 2);
        let empty = SpectrumFrame {
            center_hz: 14_074_000,
            span_hz: 96_000,
            ref_level_dbm: -20.0,
            bins: Vec::new(),
            sequence: 1,
        };
        spectrum_trace(&empty, a, &mut buf, -120.0, Color::Green);
        assert!(drawn_columns(&buf, a).is_empty());
    }

    #[test]
    fn a_zero_sized_area_is_a_no_op() {
        let mut buf = buffer(8, 2);
        spectrum_trace(
            &frame(14_074_000, 4, 8),
            Rect::new(0, 0, 0, 0),
            &mut buf,
            -120.0,
            Color::Green,
        );
        waterfall(
            &[frame(14_074_000, 4, 8)],
            Rect::new(0, 0, 8, 0),
            &mut buf,
            WaterfallPalette::TURBO,
            -120.0,
            Color::Black,
        );
    }

    // -----------------------------------------------------------------
    // Re-projection. An IF tap is dial-centred, so every retune changes a
    // frame's centre and a row captured a moment ago describes a different
    // slice of spectrum.
    // -----------------------------------------------------------------

    #[test]
    fn rows_sharing_a_centre_are_not_shifted() {
        // The DirectSdr case: a fixed tuner, every row on one axis. The
        // re-projection must be a no-op, or it would introduce a bug on
        // sources that never had this problem.
        let rows = [frame(14_074_000, 128, 256), frame(14_074_000, 128, 256)];
        assert_eq!(column_offset(&rows[1], &rows[0], 64), 0);
    }

    #[test]
    fn a_row_captured_higher_up_the_band_shifts_right() {
        // Its content sat at higher frequencies, so on the current axis it
        // belongs further right.
        let reference = frame(14_074_000, 128, 256);
        let older = frame(14_074_000 + 24_000, 128, 256);
        // A quarter of a 96 kHz span, across 64 columns, is 16 columns.
        assert_eq!(column_offset(&older, &reference, 64), 16);
    }

    #[test]
    fn a_row_captured_lower_down_the_band_shifts_left() {
        let reference = frame(14_074_000, 128, 256);
        let older = frame(14_074_000 - 24_000, 128, 256);
        assert_eq!(column_offset(&older, &reference, 64), -16);
    }

    #[test]
    fn columns_a_row_never_captured_are_drawn_as_absence() {
        // Half a span away: half the row's columns have no data. Filling
        // them with noise floor, or wrapping them, would put real-looking
        // signal at a frequency that row never saw.
        let a = area(64, 1);
        let mut buf = buffer(64, 1);
        let absent = Color::Magenta;
        let rows = [
            frame(14_074_000, 128, 256),
            frame(14_074_000 + 48_000, 128, 256),
        ];
        waterfall(&rows, a, &mut buf, WaterfallPalette::TURBO, -120.0, absent);

        // Row 1 is the background of text row 0. Its left half has no data.
        assert_eq!(buf.get(0, 0).bg, absent);
        assert_ne!(buf.get(63, 0).bg, absent);
    }

    #[test]
    fn a_waterfall_row_with_no_frame_is_absence_not_black() {
        // Fewer frames than rows: the unfilled rows must be distinguishable
        // from a genuinely silent band.
        let a = area(4, 4);
        let mut buf = buffer(4, 4);
        let absent = Color::Magenta;
        waterfall(
            &[frame(14_074_000, 2, 4)],
            a,
            &mut buf,
            WaterfallPalette::TURBO,
            -120.0,
            absent,
        );
        assert_ne!(buf.get(0, 0).fg, absent, "row 0 has a frame");
        assert_eq!(buf.get(0, 0).bg, absent, "row 1 does not");
        assert_eq!(buf.get(0, 3).fg, absent, "row 6 does not");
    }

    #[test]
    fn two_history_rows_share_one_text_row() {
        let a = area(4, 3);
        let mut buf = buffer(4, 3);
        let rows: Vec<SpectrumFrame> = (0..6).map(|_| frame(14_074_000, 2, 4)).collect();
        waterfall(
            &rows,
            a,
            &mut buf,
            WaterfallPalette::TURBO,
            -120.0,
            Color::Black,
        );
        for y in 0..3 {
            assert_eq!(buf.get(0, y).symbol(), "▀");
        }
    }

    // -----------------------------------------------------------------
    // Palette.
    // -----------------------------------------------------------------

    #[test]
    fn intensity_maps_the_ends_of_the_ramp() {
        let p = WaterfallPalette::TURBO;
        assert_eq!(p.at(0.0), Color::Indexed(17));
        assert_eq!(p.at(1.0), Color::Indexed(196));
        // Out of range clamps rather than panicking on an index.
        assert_eq!(p.at(-5.0), Color::Indexed(17));
        assert_eq!(p.at(99.0), Color::Indexed(196));
    }

    #[test]
    fn the_sixteen_colour_palette_is_a_real_fallback() {
        // Some terminals have no 256-colour support. A waterfall that
        // rendered as a single flat colour there would be an absence
        // wearing a presence's clothes.
        let p = WaterfallPalette::ANSI16;
        assert_ne!(p.at(0.0), p.at(0.5));
        assert_ne!(p.at(0.5), p.at(1.0));
    }

    #[test]
    fn a_flat_reference_does_not_divide_by_zero() {
        assert_eq!(intensity(-50.0, -20.0, -20.0), 0.0);
    }
}
