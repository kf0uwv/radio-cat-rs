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

//! Meter bars and the meter rail.

use cat_ui::MeterReading;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
};

/// The four styles a meter rail picks between.
///
/// Bundled rather than passed loose because the set travels together, and a
/// caller that got two of them the wrong way round would produce something
/// that still rendered. Visual identity stays radio-specific (ADR 0011) --
/// this crate draws the structure, each app chooses the colours.
#[derive(Debug, Clone, Copy)]
pub struct MeterStyles {
    /// A meter that is reading now.
    pub active: Style,
    /// A meter the radio has, that is inert in this state -- a TX meter
    /// during receive. It keeps its row.
    pub inactive: Style,
    pub fill: Color,
    pub empty: Color,
}

/// Draw one meter as a horizontal bar of full and empty blocks.
///
/// Takes a [`MeterReading`], not a raw value, so the scale travels with the
/// number. `ts570d` and `ft991a` each had a `smeter_bar` with their own
/// radio's range hardcoded — this signature is what makes one
/// implementation correct for both.
pub fn meter_bar(reading: MeterReading, area: Rect, buf: &mut Buffer, fill: Color, empty: Color) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let (drawn, trough) = bar_halves(reading.fraction(), area.width);
    for (col, ch) in drawn.chars().chain(trough.chars()).enumerate() {
        let colour = if col < drawn.chars().count() {
            fill
        } else {
            empty
        };
        buf.get_mut(area.x + col as u16, area.y)
            .set_char(ch)
            .set_fg(colour);
    }
}

/// The two halves of a bar: the drawn part, then the trough.
///
/// One computation, two renderers. `meter_bar` writes these into a buffer
/// cell by cell; [`bar_spans`] hands them to a caller composing a `Line`.
/// Splitting the *characters* out from the *drawing* is what stops those
/// two from drifting apart, which is the whole failure mode ADR 0011
/// exists to prevent — and the reason a second bar helper is a shared
/// function rather than a copy in the app that needed it.
///
/// Every drawn cell precedes every trough cell: the fill level falls
/// monotonically across columns, so the drawn part is always a prefix.
/// That is what lets the result be two spans rather than `width` of them.
///
/// Eight sub-levels per cell, so a 20-cell bar resolves 160 steps — finer
/// than either radio's meter actually reports.
fn bar_halves(fraction: f32, width: u16) -> (String, String) {
    let total = u32::from(width) * 8;
    let filled = (fraction.clamp(0.0, 1.0) * total as f32).round() as u32;
    let mut drawn = String::new();
    let mut trough = String::new();
    for col in 0..width {
        let base = u32::from(col) * 8;
        let level = filled.saturating_sub(base).min(8) as usize;
        if level == 0 {
            trough.push('░');
        } else {
            // A partial cell is drawn with a LEFT-anchored block so the bar
            // grows smoothly; `BLOCKS` is vertical, so use the horizontal
            // equivalent for the fractional cell.
            drawn.push(horizontal_block(level));
        }
    }
    (drawn, trough)
}

/// A bar as two styled spans, for a caller composing a `Line`.
///
/// [`meter_bar`] draws into a buffer, which is the right shape for a
/// widget that owns a `Rect` and the wrong shape for a console that builds
/// its status rows as spans — `ts570d` writes `AF:[████░░░░░░]` inline
/// beside four other readouts, and had been round-tripping a `String`
/// through a character filter to recover the two halves it needed. That is
/// an impedance mismatch, not a missing feature: the same bar, wanted in a
/// different composition model.
///
/// Takes a bare fraction because not every bar is a meter — a gain
/// setting is 0.0-1.0 with no `RawRange` behind it. Use [`meter_spans`]
/// when there is a reading, so the scale travels with the number.
pub fn bar_spans(fraction: f32, width: u16, fill: Style, empty: Style) -> Vec<Span<'static>> {
    let (drawn, trough) = bar_halves(fraction, width);
    vec![Span::styled(drawn, fill), Span::styled(trough, empty)]
}

/// [`bar_spans`] for a meter, so the reading's own range does the scaling.
pub fn meter_spans(
    reading: MeterReading,
    width: u16,
    fill: Style,
    empty: Style,
) -> Vec<Span<'static>> {
    bar_spans(reading.fraction(), width, fill, empty)
}

fn horizontal_block(level: usize) -> char {
    match level {
        1 => '▏',
        2 => '▎',
        3 => '▍',
        4 => '▌',
        5 => '▋',
        6 => '▊',
        7 => '▉',
        _ => '█',
    }
}

/// A one-line S-meter readout: `S8   17/30`.
///
/// The S-unit comes from [`MeterReading::s_unit`], so the radio's own
/// table is used whenever it published one — this widget is never told a
/// scale and so can never be told the wrong one.
///
/// Showing the raw value beside the label is deliberate — it is what makes
/// a miscalibrated meter diagnosable rather than merely wrong.
pub fn smeter_line(reading: Option<MeterReading>, label_style: Style, dim: Style) -> Line<'static> {
    match reading {
        None => Line::from(vec![
            Span::styled("—", dim),
            Span::styled("  no reading yet", dim),
        ]),
        Some(r) => Line::from(vec![
            Span::styled(r.s_unit(), label_style),
            Span::styled(format!("  {}/{}", r.raw, r.range.max), dim),
        ]),
    }
}

/// Draw a stack of meters, one per row, labelled.
///
/// A meter the radio does not have is not passed in, so it cannot be drawn.
/// A meter that is present but inactive — a TX meter during receive — is
/// passed with `active: false` and keeps its row, dimmed. Reflowing the
/// rail on every transmit would make the whole panel jump.
pub fn meter_rail(
    meters: &[(&str, Option<MeterReading>, bool)],
    area: Rect,
    buf: &mut Buffer,
    label_width: u16,
    styles: MeterStyles,
) {
    let MeterStyles {
        active,
        inactive,
        fill,
        empty,
    } = styles;
    for (row, (name, reading, is_active)) in meters.iter().enumerate() {
        let y = area.y + row as u16;
        if y >= area.y + area.height {
            break;
        }
        let style = if *is_active { active } else { inactive };
        buf.set_string(area.x, y, *name, style);
        let bar_x = area.x + label_width;
        if bar_x >= area.x + area.width {
            continue;
        }
        let bar_area = Rect {
            x: bar_x,
            y,
            width: area.width - label_width,
            height: 1,
        };
        match (reading, is_active) {
            (Some(r), true) => meter_bar(*r, bar_area, buf, fill, empty),
            // Present but inert: an empty trough, so the row still reads as
            // a meter rather than as blank space.
            _ => meter_bar(
                MeterReading::new(
                    cat_framework::capabilities::MeterKind::S,
                    0,
                    cat_framework::capabilities::RawRange::new(0, 1),
                ),
                bar_area,
                buf,
                empty,
                empty,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_framework::capabilities::{MeterKind, RawRange};
    use ratatui::layout::Rect;

    fn buf(w: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, w, 1))
    }

    fn filled(b: &Buffer, w: u16) -> usize {
        (0..w).filter(|x| b.get(*x, 0).symbol() == "█").count()
    }

    #[test]
    fn the_same_raw_value_fills_differently_on_each_radio() {
        // The whole reason a bar takes a MeterReading. Raw 15 is mid-scale
        // on a TS-570D S-meter and under 6% on an FT-991A's, and the two
        // apps' copies of this function each hardcoded one of them.
        let a = Rect::new(0, 0, 20, 1);
        let mut ts = buf(20);
        let mut ft = buf(20);
        meter_bar(
            MeterReading::new(MeterKind::S, 15, RawRange::new(0, 30)),
            a,
            &mut ts,
            Color::Green,
            Color::DarkGray,
        );
        meter_bar(
            MeterReading::new(MeterKind::S, 15, RawRange::new(0, 255)),
            a,
            &mut ft,
            Color::Green,
            Color::DarkGray,
        );
        assert_eq!(filled(&ts, 20), 10);
        assert!(filled(&ft, 20) <= 1);
    }

    #[test]
    fn a_full_scale_reading_fills_every_cell() {
        let a = Rect::new(0, 0, 12, 1);
        let mut b = buf(12);
        meter_bar(
            MeterReading::new(MeterKind::S, 30, RawRange::new(0, 30)),
            a,
            &mut b,
            Color::Green,
            Color::DarkGray,
        );
        assert_eq!(filled(&b, 12), 12);
    }

    #[test]
    fn an_empty_reading_draws_a_trough_not_blank_space() {
        // A blank row would read as "no meter here"; a trough reads as
        // "a meter, reading zero", which is a different fact.
        let a = Rect::new(0, 0, 6, 1);
        let mut b = buf(6);
        meter_bar(
            MeterReading::new(MeterKind::S, 0, RawRange::new(0, 30)),
            a,
            &mut b,
            Color::Green,
            Color::DarkGray,
        );
        assert_eq!(filled(&b, 6), 0);
        assert_eq!(b.get(0, 0).symbol(), "░");
    }

    #[test]
    fn partial_cells_give_sub_cell_resolution() {
        // 20 cells at 8 sub-levels resolves 160 steps, finer than either
        // radio reports. Without it a 0-30 meter would move in 1/20ths and
        // adjacent S-units would look identical.
        let a = Rect::new(0, 0, 20, 1);
        let mut lo = buf(20);
        let mut hi = buf(20);
        meter_bar(
            MeterReading::new(MeterKind::S, 7, RawRange::new(0, 30)),
            a,
            &mut lo,
            Color::Green,
            Color::DarkGray,
        );
        meter_bar(
            MeterReading::new(MeterKind::S, 8, RawRange::new(0, 30)),
            a,
            &mut hi,
            Color::Green,
            Color::DarkGray,
        );
        let render = |b: &Buffer| {
            (0..20)
                .map(|x| b.get(x, 0).symbol().to_string())
                .collect::<String>()
        };
        assert_ne!(render(&lo), render(&hi), "one raw step must be visible");
    }

    #[test]
    fn an_over_range_reading_clamps_rather_than_overflowing() {
        let a = Rect::new(0, 0, 8, 1);
        let mut b = buf(8);
        meter_bar(
            MeterReading::new(MeterKind::Swr, u16::MAX, RawRange::new(0, 30)),
            a,
            &mut b,
            Color::Green,
            Color::DarkGray,
        );
        assert_eq!(filled(&b, 8), 8);
    }

    fn text_of(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn a_bar_drawn_into_a_buffer_and_one_built_as_spans_are_the_same_bar() {
        // The whole reason `bar_halves` exists. If these two ever disagree,
        // the same meter reads differently in two panels of one console.
        let r = MeterReading::new(MeterKind::S, 17, RawRange::new(0, 30));
        let area = Rect::new(0, 0, 20, 1);
        let mut b = Buffer::empty(area);
        meter_bar(r, area, &mut b, Color::Green, Color::DarkGray);
        let drawn: String = (0..20).map(|x| b.get(x, 0).symbol()).collect();

        let spans = meter_spans(r, 20, Style::new(), Style::new());
        let composed: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(drawn, composed);
    }

    #[test]
    fn a_bar_is_always_two_spans_because_the_fill_is_a_prefix() {
        // A partial cell belongs to the drawn half, so there is never a
        // trough cell before a drawn one -- which is what lets a caller
        // splice a bar into a `Line` without knowing its width.
        for raw in 0..=30u16 {
            let spans = bar_spans(raw as f32 / 30.0, 20, Style::new(), Style::new());
            assert_eq!(spans.len(), 2);
            let (drawn, trough) = (&spans[0].content, &spans[1].content);
            assert!(!drawn.contains('░'), "raw {raw}: trough char in the fill");
            assert!(
                trough.chars().all(|c| c == '░'),
                "raw {raw}: fill char in the trough"
            );
            assert_eq!(
                drawn.chars().count() + trough.chars().count(),
                20,
                "raw {raw}: bar must always be exactly as wide as asked"
            );
        }
    }

    #[test]
    fn a_bar_span_clamps_rather_than_overflowing_its_width() {
        // `bar_spans` takes a bare f32, so unlike `meter_spans` nothing
        // upstream has already clamped it.
        for f in [-1.0, 0.0, 1.0, 2.0, f32::NAN] {
            let spans = bar_spans(f, 10, Style::new(), Style::new());
            let width: usize = spans.iter().map(|s| s.content.chars().count()).sum();
            assert_eq!(width, 10, "fraction {f} produced a {width}-cell bar");
        }
    }

    #[test]
    fn a_radios_own_s_unit_table_beats_the_generic_formula() {
        // Raw 24 is where the TS-570D's table and the interpolated formula
        // part company; a console that passes `None` silently loses its
        // radio's calibration.
        let bare = MeterReading::new(MeterKind::S, 24, RawRange::new(0, 30));
        let with_table = bare.with_s_units(cat_ui::SUnitScale::TS570D);
        assert_eq!(with_table.s_unit(), "S9+10");
        assert_ne!(
            with_table.s_unit(),
            bare.s_unit(),
            "the table must actually be consulted"
        );
        // And the widget picks it up without being told, which is the
        // whole reason the table hangs off the reading.
        assert!(
            text_of(&smeter_line(Some(with_table), Style::new(), Style::new()))
                .starts_with("S9+10")
        );
    }

    #[test]
    fn an_unread_smeter_says_so_rather_than_showing_s0() {
        let dim = Style::default();
        let line = smeter_line(None, dim, dim);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains('—'));
        assert!(!text.contains("S0"), "unknown must not render as a reading");
    }

    #[test]
    fn a_read_smeter_shows_its_unit_and_its_raw_value() {
        let dim = Style::default();
        let line = smeter_line(
            Some(MeterReading::new(MeterKind::S, 20, RawRange::new(0, 30))),
            dim,
            dim,
        );
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("S9"), "got {text:?}");
        // The raw value is what makes a miscalibrated meter diagnosable.
        assert!(text.contains("20/30"), "got {text:?}");
    }

    #[test]
    fn an_inactive_meter_keeps_its_row() {
        // A TX meter during receive must not reflow the rail.
        let a = Rect::new(0, 0, 24, 4);
        let mut b = Buffer::empty(a);
        let s = Style::default();
        meter_rail(
            &[
                (
                    "S",
                    Some(MeterReading::new(MeterKind::S, 15, RawRange::new(0, 30))),
                    true,
                ),
                ("PO", None, false),
                ("SWR", None, false),
            ],
            a,
            &mut b,
            5,
            MeterStyles {
                active: s,
                inactive: s,
                fill: Color::Green,
                empty: Color::DarkGray,
            },
        );
        assert_eq!(b.get(0, 0).symbol(), "S");
        assert_eq!(b.get(0, 1).symbol(), "P");
        assert_eq!(b.get(0, 2).symbol(), "S");
        // The inert rows still draw a trough.
        assert_eq!(b.get(5, 1).symbol(), "░");
    }
}
