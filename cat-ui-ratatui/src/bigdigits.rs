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

//! Three-row digits, for the one number an operator reads from across the
//! room.
//!
//! A dial frequency drawn in body text is the same size as the mode, the
//! VFO letter and every label around it, so nothing about the layout says
//! which of them matters. Every physical radio ever built solves this the
//! same way -- the frequency is the biggest thing on the front panel --
//! and a terminal can do the same with three rows instead of one.

/// Columns each glyph occupies, including its trailing gap.
pub const GLYPH_WIDTH: usize = 4;
/// Rows a rendered number occupies.
pub const HEIGHT: usize = 3;

/// The glyphs. Box-drawing rather than solid blocks: at three rows a solid
/// `█` digit loses its counters (the holes in 0, 8 and 9) and 8 becomes
/// indistinguishable from 0.
fn glyph(c: char) -> [&'static str; HEIGHT] {
    match c {
        '0' => ["╭─╮", "│ │", "╰─╯"],
        '1' => ["  ╷", "  │", "  ╵"],
        '2' => ["╭─╮", "╭─╯", "╰─╴"],
        '3' => ["╭─╮", " ─┤", "╰─╯"],
        '4' => ["╷ ╷", "╰─┤", "  ╵"],
        '5' => ["╭─╴", "╰─╮", "╰─╯"],
        '6' => ["╭─╴", "├─╮", "╰─╯"],
        '7' => ["╶─╮", "  │", "  ╵"],
        '8' => ["╭─╮", "├─┤", "╰─╯"],
        '9' => ["╭─╮", "╰─┤", "  ╵"],
        '.' => ["   ", "   ", " ▪ "],
        '-' | '—' => ["   ", "╶─╴", "   "],
        _ => ["   ", "   ", "   "],
    }
}

/// Render `text` as three rows.
///
/// Anything without a glyph becomes blank of the same width, so a caller
/// can pass a formatted frequency straight through and keep its alignment.
pub fn render(text: &str) -> [String; HEIGHT] {
    let mut rows = [String::new(), String::new(), String::new()];
    for c in text.chars() {
        let g = glyph(c);
        for (row, part) in rows.iter_mut().zip(g.iter()) {
            row.push_str(part);
            row.push(' ');
        }
    }
    rows
}

/// How wide `text` will be once rendered.
pub fn width(text: &str) -> usize {
    text.chars().count() * GLYPH_WIDTH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_digit_has_a_glyph_of_the_right_shape() {
        for c in "0123456789.".chars() {
            let g = glyph(c);
            for row in g {
                assert_eq!(
                    row.chars().count(),
                    3,
                    "glyph {c:?} row {row:?} is not three columns"
                );
            }
        }
    }

    #[test]
    fn no_two_digits_look_alike() {
        // The reason for box-drawing over solid blocks. At three rows a
        // filled glyph loses its counters and 8 renders identically to 0,
        // which on a dial readout is a wrong frequency displayed
        // confidently.
        let mut seen = std::collections::HashMap::new();
        for c in "0123456789".chars() {
            let shape = glyph(c).join("|");
            if let Some(other) = seen.insert(shape.clone(), c) {
                panic!("{c} and {other} render identically:\n{shape}");
            }
        }
    }

    #[test]
    fn a_frequency_renders_three_rows_of_equal_width() {
        let rows = render("14.074.055");
        assert_eq!(rows.len(), HEIGHT);
        let w = rows[0].chars().count();
        assert!(rows.iter().all(|r| r.chars().count() == w), "ragged rows");
        assert_eq!(w, width("14.074.055"));
    }

    #[test]
    fn an_unknown_character_keeps_its_column() {
        // A caller passing "— MHz" must not have the layout shift under it.
        assert_eq!(render("?").iter().map(|r| r.chars().count()).max(), Some(4));
    }
}
