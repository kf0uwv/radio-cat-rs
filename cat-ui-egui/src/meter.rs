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

//! Meters, drawn as painted rectangles.

use cat_ui::MeterReading;
use egui::{Color32, Rect, Rounding, Ui};

/// Draw a meter as a filled bar within `rect`.
///
/// Takes a [`MeterReading`], never a raw value, so the scale travels with
/// the number. The two apps' terminal versions of this each hardcoded their
/// own radio's range; passing the reading is what lets one implementation
/// be right for a 0-30 meter and a 0-255 one at the same time.
pub fn meter_bar(ui: &Ui, rect: Rect, reading: MeterReading, fill: Color32, trough: Color32) {
    let painter = ui.painter();
    painter.rect_filled(rect, Rounding::ZERO, trough);
    let w = rect.width() * reading.fraction();
    if w > 0.0 {
        let filled = Rect::from_min_size(rect.min, egui::vec2(w, rect.height()));
        painter.rect_filled(filled, Rounding::ZERO, fill);
    }
}

/// The S-meter's text: an S-unit and the raw value behind it.
///
/// Returning a string rather than drawing it keeps the caller in charge of
/// placement and style, which is layout. Showing the raw value beside the
/// unit is what makes a miscalibrated meter diagnosable instead of merely
/// wrong.
pub fn smeter_text(reading: Option<MeterReading>) -> String {
    match reading {
        None => "—".to_string(),
        Some(r) => format!(
            "{}  {}/{}",
            cat_ui::format_smeter_label(r.raw, r.range),
            r.raw,
            r.range.max
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_framework::capabilities::{MeterKind, RawRange};

    #[test]
    fn an_unread_meter_says_so_rather_than_reading_zero() {
        assert_eq!(smeter_text(None), "—");
    }

    #[test]
    fn the_same_raw_value_reads_differently_per_radio() {
        // One implementation, two radios, because the reading carries its
        // own scale. This is the duplication ADR 0011 found, closed.
        let ts = MeterReading::new(MeterKind::S, 20, RawRange::new(0, 30));
        let ft = MeterReading::new(MeterKind::S, 20, RawRange::new(0, 255));
        assert!(smeter_text(Some(ts)).starts_with("S9"));
        assert!(smeter_text(Some(ft)).starts_with("S1 "));
    }

    #[test]
    fn the_raw_value_is_shown_so_a_bad_trim_is_diagnosable() {
        let r = MeterReading::new(MeterKind::S, 17, RawRange::new(0, 30));
        assert!(smeter_text(Some(r)).contains("17/30"));
    }
}
