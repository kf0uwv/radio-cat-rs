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
    // Eight sub-levels per cell, so a 20-cell bar resolves 160 steps —
    // finer than either radio's meter actually reports.
    let total = u32::from(area.width) * 8;
    let filled = (reading.fraction() * total as f32).round() as u32;
    for col in 0..area.width {
        let base = u32::from(col) * 8;
        let level = filled.saturating_sub(base).min(8) as usize;
        let cell = buf.get_mut(area.x + col, area.y);
        if level == 8 {
            cell.set_char('█').set_fg(fill);
        } else if level > 0 {
            // A partial cell is drawn with a LEFT-anchored block so the bar
            // grows smoothly; `BLOCKS` is vertical, so use the horizontal
            // equivalent for the fractional cell.
            cell.set_char(horizontal_block(level)).set_fg(fill);
        } else {
            cell.set_char('░').set_fg(empty);
        }
    }
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
/// The S-unit label comes from [`cat_ui::format_smeter_label`], which scales
/// against the radio's own range. Showing the raw value beside it is
/// deliberate — it is what makes a miscalibrated meter diagnosable rather
/// than merely wrong.
pub fn smeter_line(reading: Option<MeterReading>, label_style: Style, dim: Style) -> Line<'static> {
    match reading {
        None => Line::from(vec![
            Span::styled("—", dim),
            Span::styled("  no reading yet", dim),
        ]),
        Some(r) => Line::from(vec![
            Span::styled(cat_ui::format_smeter_label(r.raw, r.range), label_style),
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
