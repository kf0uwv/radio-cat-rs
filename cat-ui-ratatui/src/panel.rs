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

//! Selection grids for bands, modes and filter widths.

use ratatui::{buffer::Buffer, layout::Rect, style::Style};

/// One cell of a selection grid.
///
/// `enabled` is not decoration. A capability the radio lacks is drawn in
/// place and dimmed rather than omitted, so the grid keeps its shape and an
/// operator learns the control exists but this radio cannot do it. The
/// TS-570D's RIT and XIT are the live case: `VfoCapability::rit_hz` is
/// `None`, and hiding them would silently make the panel a different size
/// on a different radio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridItem<'a> {
    pub label: &'a str,
    pub selected: bool,
    /// A value requested but not yet confirmed by the radio.
    pub pending: bool,
    pub enabled: bool,
}

impl<'a> GridItem<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            selected: false,
            pending: false,
            enabled: true,
        }
    }

    pub fn selected(mut self, yes: bool) -> Self {
        self.selected = yes;
        self
    }

    pub fn pending(mut self, yes: bool) -> Self {
        self.pending = yes;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// Styles a grid picks between. Passed in rather than defined here:
/// visual identity is radio-specific (ADR 0011), so this crate draws the
/// structure and each app chooses the colours.
#[derive(Debug, Clone, Copy)]
pub struct GridStyles {
    pub normal: Style,
    pub selected: Style,
    /// Requested, not yet acknowledged. Distinct from `selected` because a
    /// control that shows a value the radio has not confirmed is lying.
    pub pending: Style,
    pub disabled: Style,
}

/// Draw `items` in a grid `columns` wide, centring each label in its cell.
///
/// Returns the number of rows used, so a caller can lay out what follows
/// without recomputing the arithmetic.
pub fn grid(
    items: &[GridItem<'_>],
    columns: u16,
    area: Rect,
    buf: &mut Buffer,
    styles: GridStyles,
) -> u16 {
    if columns == 0 || area.width == 0 || area.height == 0 || items.is_empty() {
        return 0;
    }
    let cell_w = area.width / columns;
    if cell_w == 0 {
        return 0;
    }
    let mut rows_used = 0;
    for (i, item) in items.iter().enumerate() {
        let row = i as u16 / columns;
        let col = i as u16 % columns;
        if row >= area.height {
            break;
        }
        rows_used = row + 1;
        let style = if !item.enabled {
            styles.disabled
        } else if item.pending {
            styles.pending
        } else if item.selected {
            styles.selected
        } else {
            styles.normal
        };
        let x = area.x + col * cell_w;
        let text = centre(item.label, cell_w);
        buf.set_string(x, area.y + row, text, style);
    }
    rows_used
}

/// Centre `label` in `width` cells, truncating if it cannot fit.
fn centre(label: &str, width: u16) -> String {
    let w = usize::from(width);
    let len = label.chars().count();
    if len >= w {
        return label.chars().take(w).collect();
    }
    let pad = w - len;
    let left = pad / 2;
    let mut out = String::with_capacity(w);
    out.extend(std::iter::repeat(' ').take(left));
    out.push_str(label);
    out.extend(std::iter::repeat(' ').take(pad - left));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};

    fn styles() -> GridStyles {
        GridStyles {
            normal: Style::default(),
            selected: Style::default().add_modifier(Modifier::REVERSED),
            pending: Style::default().fg(Color::Cyan),
            disabled: Style::default().fg(Color::DarkGray),
        }
    }

    fn render(items: &[GridItem<'_>], cols: u16, w: u16, h: u16) -> (Buffer, u16) {
        let a = Rect::new(0, 0, w, h);
        let mut b = Buffer::empty(a);
        let rows = grid(items, cols, a, &mut b, styles());
        (b, rows)
    }

    #[test]
    fn items_wrap_into_rows_and_report_the_height_used() {
        let items: Vec<GridItem> = ["160m", "80m", "40m", "20m", "15m", "10m"]
            .iter()
            .map(|l| GridItem::new(l))
            .collect();
        let (_, rows) = render(&items, 4, 24, 4);
        assert_eq!(rows, 2, "six items in fours is two rows");
    }

    #[test]
    fn a_capability_the_radio_lacks_is_dimmed_in_place_not_omitted() {
        // The TS-570D's RIT/XIT. Hiding them would silently change the
        // panel's size between radios, and an operator would never learn
        // the control exists.
        let items = [
            GridItem::new("RIT").disabled(),
            GridItem::new("XIT").disabled(),
        ];
        let (b, rows) = render(&items, 2, 12, 1);
        assert_eq!(rows, 1);
        let text: String = (0..12).map(|x| b.get(x, 0).symbol()).collect();
        assert!(
            text.contains("RIT"),
            "disabled items still occupy their cell"
        );
        assert_eq!(b.get(1, 0).style().fg, Some(Color::DarkGray));
    }

    #[test]
    fn a_requested_value_is_distinguishable_from_a_confirmed_one() {
        // Showing a pending change as selected would claim the radio had
        // acknowledged something it has not.
        let items = [
            GridItem::new("USB").selected(true),
            GridItem::new("CW").pending(true),
        ];
        let (b, _) = render(&items, 2, 12, 1);
        assert_ne!(b.get(1, 0).style(), b.get(7, 0).style());
        assert_eq!(b.get(7, 0).style().fg, Some(Color::Cyan));
    }

    #[test]
    fn labels_are_centred_in_their_cell() {
        let items = [GridItem::new("CW")];
        let (b, _) = render(&items, 1, 6, 1);
        let text: String = (0..6).map(|x| b.get(x, 0).symbol()).collect();
        assert_eq!(text, "  CW  ");
    }

    #[test]
    fn a_label_too_wide_for_its_cell_is_truncated_not_overflowed() {
        // Overflow would corrupt the neighbouring cell, which in a grid of
        // controls means mislabelling a different button.
        let items = [GridItem::new("DATA-FM"), GridItem::new("AM")];
        let (b, _) = render(&items, 2, 8, 1);
        let text: String = (0..8).map(|x| b.get(x, 0).symbol()).collect();
        assert_eq!(text.chars().count(), 8);
        assert!(text.starts_with("DATA"));
    }

    #[test]
    fn a_grid_too_narrow_for_its_columns_draws_nothing_rather_than_panicking() {
        let items = [GridItem::new("A"), GridItem::new("B")];
        let (_, rows) = render(&items, 8, 4, 1);
        assert_eq!(rows, 0);
    }

    #[test]
    fn more_items_than_rows_stops_at_the_edge() {
        let items: Vec<GridItem> = (0..20).map(|_| GridItem::new("x")).collect();
        let (_, rows) = render(&items, 4, 16, 2);
        assert_eq!(rows, 2);
    }
}
