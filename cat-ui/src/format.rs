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

//! Frequency and signal-strength formatting.
//!
//! [`format_hz`] is the first thing this crate absorbed, and the clearest
//! case for its existence: it was **byte-identical** in
//! `ts570d/ui/src/layout.rs:29` and `ft991a/ui/src/layout.rs:69`. Two
//! copies, maintained twice, with nothing radio-specific in either.

use cat_framework::capabilities::RawRange;

/// A frequency as an operator reads it: `14.074.000 MHz`.
///
/// Byte-for-byte the output both apps already produce, so migrating a TUI
/// onto this crate changes nothing an operator sees — which ADR 0011 sets
/// as the acceptance bar for that migration.
pub fn format_hz(hz: u64) -> String {
    let mhz = hz / 1_000_000;
    let khz = (hz % 1_000_000) / 1_000;
    let hz_rem = hz % 1_000;
    format!("{}.{:03}.{:03} MHz", mhz, khz, hz_rem)
}

/// The same frequency without the unit, for places that are already
/// labelled: `14.074.000`.
pub fn format_hz_compact(hz: u64) -> String {
    let mhz = hz / 1_000_000;
    let khz = (hz % 1_000_000) / 1_000;
    let hz_rem = hz % 1_000;
    format!("{}.{:03}.{:03}", mhz, khz, hz_rem)
}

/// Where each S-unit boundary falls on a radio's raw meter scale.
///
/// **S-meter law is radio data, not a display preference.** Where S9 sits
/// and how the units below it space out is a property of the meter circuit,
/// and it is not linear on real radios. The TS-570D's own table gives S0
/// three raw counts and every other unit two, which no clean formula
/// reproduces — substituting one changes the reading at 7 of its 31 raw
/// values.
///
/// So a radio that knows its own law supplies it, exactly as it already
/// supplies [`RawRange`] rather than letting a widget assume 0-255.
///
/// `thresholds` is the **inclusive upper bound** of each label in
/// `LABELS`, ascending. A reading above the last threshold takes the last
/// label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SUnitScale {
    thresholds: &'static [u16],
}

/// The labels [`SUnitScale`] assigns, in order.
pub const S_UNIT_LABELS: [&str; 13] = [
    "S0", "S1", "S2", "S3", "S4", "S5", "S6", "S7", "S8", "S9", "S9+10", "S9+20", "S9+30",
];

impl SUnitScale {
    /// A scale from explicit upper bounds, one per label in
    /// [`S_UNIT_LABELS`].
    pub const fn new(thresholds: &'static [u16]) -> Self {
        Self { thresholds }
    }

    /// The Kenwood TS-570D's table, as its TUI has always drawn it.
    ///
    /// Preserved exactly rather than approximated: this is what its
    /// operators have been reading, and ADR 0011 rev 4 sets "the operator
    /// sees no change" as the bar for migrating onto shared widgets.
    pub const TS570D: Self = Self::new(&[2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 24, 28, u16::MAX]);

    /// The label for a raw reading.
    pub fn label(&self, raw: u16) -> &'static str {
        for (i, bound) in self.thresholds.iter().enumerate() {
            if raw <= *bound {
                return S_UNIT_LABELS[i.min(S_UNIT_LABELS.len() - 1)];
            }
        }
        S_UNIT_LABELS[S_UNIT_LABELS.len() - 1]
    }
}

/// An S-unit label for a raw meter reading, scaled against the radio's own
/// range.
///
/// This is the function that could not be shared before. `ts570d`'s
/// version hardcoded a 0-30 scale; `ft991a` had **no S-unit label at
/// all**, so its operators read a bar with no number against it. Taking
/// the range as a parameter is what lets one implementation serve both —
/// and the range is radio *data* from `MeterDescriptor::raw_range`, not a
/// per-radio preference, which is the distinction ADR 0011 draws between a
/// legitimate shared-widget parameter and one that should not exist.
///
/// S9 is placed at two thirds of full scale, which is where both radios'
/// meters put it.
pub fn format_smeter_label(raw: u16, range: RawRange) -> &'static str {
    let fraction = range.fraction(raw);
    // S0-S9 over the lower two thirds, then +10/+20/+30 dB over the rest.
    if fraction >= 1.0 {
        return "S9+30";
    }
    if fraction > 2.0 / 3.0 {
        let over = (fraction - 2.0 / 3.0) / (1.0 / 3.0);
        return match (over * 3.0) as u8 {
            0 => "S9+10",
            1 => "S9+20",
            _ => "S9+30",
        };
    }
    let s_unit = ((fraction / (2.0 / 3.0)) * 9.0).round() as u8;
    match s_unit {
        0 => "S0",
        1 => "S1",
        2 => "S2",
        3 => "S3",
        4 => "S4",
        5 => "S5",
        6 => "S6",
        7 => "S7",
        8 => "S8",
        _ => "S9",
    }
}

/// The fallback used when a radio has not supplied its own table.
///
/// Places S9 at two thirds of full scale and spaces the units below it
/// evenly. Reasonable, and **not** a substitute for a radio's real law: a
/// radio that knows its own should pass an [`SUnitScale`]. This exists so
/// that a radio which has never shown S-units at all — the FT-991A's TUI
/// shows a bar with no number — gets something correct-ish rather than
/// nothing.
pub fn format_smeter_label_default(raw: u16, range: RawRange) -> &'static str {
    format_smeter_label(raw, range)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequencies_render_the_way_both_apps_already_render_them() {
        // The exact strings the existing TUIs produce. If these change, a
        // migrated TUI shows something different to its operator, which
        // ADR 0011 forbids.
        assert_eq!(format_hz(14_074_000), "14.074.000 MHz");
        assert_eq!(format_hz(7_100_000), "7.100.000 MHz");
        assert_eq!(format_hz(500_000), "0.500.000 MHz");
        assert_eq!(format_hz(0), "0.000.000 MHz");
    }

    #[test]
    fn sub_khz_digits_are_not_dropped() {
        // A radio tuned 1 Hz off must not look identical to one on
        // frequency -- this is exactly what the trim calibration adjusts.
        assert_eq!(format_hz(14_074_001), "14.074.001 MHz");
        assert_ne!(format_hz(14_074_000), format_hz(14_074_001));
    }

    #[test]
    fn leading_zeroes_are_preserved_in_each_group() {
        assert_eq!(format_hz(1_000_005), "1.000.005 MHz");
        assert_eq!(format_hz(1_005_000), "1.005.000 MHz");
    }

    #[test]
    fn the_compact_form_differs_only_in_the_unit() {
        assert_eq!(format_hz_compact(14_074_000), "14.074.000");
        assert!(format_hz(14_074_000).starts_with(&format_hz_compact(14_074_000)));
    }

    #[test]
    fn vhf_and_uhf_frequencies_format_without_special_casing() {
        // The FT-991A reaches 470 MHz; the TS-570D stops at 60. One
        // formatter must serve both.
        assert_eq!(format_hz(144_200_000), "144.200.000 MHz");
        assert_eq!(format_hz(432_100_000), "432.100.000 MHz");
    }

    #[test]
    fn the_same_s_unit_comes_from_different_raw_values_per_radio() {
        // The whole reason this takes a range. Raw 20 on a 0-30 meter and
        // raw 170 on a 0-255 meter are the same signal, and both must read
        // S9.
        let ts570d = RawRange::new(0, 30);
        let ft991a = RawRange::new(0, 255);
        assert_eq!(format_smeter_label(20, ts570d), "S9");
        assert_eq!(format_smeter_label(170, ft991a), "S9");
    }

    #[test]
    fn s_units_span_the_lower_two_thirds_of_the_scale() {
        let range = RawRange::new(0, 30);
        assert_eq!(format_smeter_label(0, range), "S0");
        assert_eq!(format_smeter_label(10, range), "S5");
        assert_eq!(format_smeter_label(20, range), "S9");
    }

    #[test]
    fn readings_above_s9_report_decibels_over() {
        let range = RawRange::new(0, 30);
        assert_eq!(format_smeter_label(30, range), "S9+30");
        assert!(format_smeter_label(24, range).starts_with("S9+"));
        assert!(format_smeter_label(27, range).starts_with("S9+"));
    }

    #[test]
    fn an_out_of_range_reading_clamps_rather_than_panicking() {
        // Radios do report outside their documented range. A meter label
        // is not the place to discover that.
        let range = RawRange::new(0, 30);
        assert_eq!(format_smeter_label(u16::MAX, range), "S9+30");
        let offset = RawRange::new(10, 20);
        assert_eq!(format_smeter_label(0, offset), "S0");
    }

    #[test]
    fn a_degenerate_range_does_not_panic() {
        assert_eq!(format_smeter_label(5, RawRange::new(5, 5)), "S0");
    }
}

#[cfg(test)]
mod s_unit_scale_tests {
    use super::*;

    /// The TS-570D TUI's table, transcribed from `ui/src/layout.rs` as it
    /// stood before the migration. This is the reference an operator has
    /// actually been reading.
    fn ts570d_as_shipped(raw: u16) -> &'static str {
        match raw {
            0..=2 => "S0",
            3..=4 => "S1",
            5..=6 => "S2",
            7..=8 => "S3",
            9..=10 => "S4",
            11..=12 => "S5",
            13..=14 => "S6",
            15..=16 => "S7",
            17..=18 => "S8",
            19..=20 => "S9",
            21..=24 => "S9+10",
            25..=28 => "S9+20",
            _ => "S9+30",
        }
    }

    #[test]
    fn the_ts570d_scale_reproduces_its_shipped_table_exactly() {
        // ADR 0011 rev 4's acceptance bar for migrating an app onto shared
        // widgets is that the operator sees no change. This is that bar,
        // as a test, across every raw value the meter can report.
        for raw in 0..=40u16 {
            assert_eq!(
                SUnitScale::TS570D.label(raw),
                ts570d_as_shipped(raw),
                "raw {raw} would have changed on screen"
            );
        }
    }

    #[test]
    fn the_generic_formula_does_not_reproduce_it_and_that_is_the_point() {
        // EIGHT of thirty-one raw values differ, and the count itself has
        // a lesson in it. A scratch model of this comparison written in
        // Python reported seven -- it missed raw 10, because Python's
        // round() breaks ties to even and Rust's f32::round breaks them
        // away from zero, and 4.5 is exactly such a tie.
        //
        // So the model of the code disagreed with the code about how many
        // ways the code disagreed with the radio. That is why this test
        // compares the real implementations rather than a description of
        // them, and why SUnitScale exists at all: an S-meter's law is a
        // property of its circuit, and a formula that fits one radio is
        // not evidence about another.
        let range = RawRange::new(0, 30);
        let differing: Vec<u16> = (0..=30u16)
            .filter(|r| format_smeter_label(*r, range) != ts570d_as_shipped(*r))
            .collect();
        assert_eq!(differing, vec![2, 4, 6, 8, 10, 24, 27, 28]);
    }

    #[test]
    fn a_reading_past_the_last_threshold_pegs_rather_than_wrapping() {
        assert_eq!(SUnitScale::TS570D.label(u16::MAX), "S9+30");
    }

    #[test]
    fn a_scale_shorter_than_the_label_list_still_terminates() {
        // A radio need not describe every unit. Running off the end must
        // peg, not index out of bounds.
        let coarse = SUnitScale::new(&[10, 20]);
        assert_eq!(coarse.label(5), "S0");
        assert_eq!(coarse.label(15), "S1");
        assert_eq!(coarse.label(999), "S9+30");
    }

    #[test]
    fn the_default_still_serves_a_radio_with_no_table() {
        // The FT-991A's TUI shows an S-meter bar with no S-unit at all.
        // Something correct-ish beats nothing, and it needs no per-radio
        // calibration to adopt.
        let range = RawRange::new(0, 255);
        assert_eq!(format_smeter_label_default(0, range), "S0");
        assert_eq!(format_smeter_label_default(170, range), "S9");
        assert_eq!(format_smeter_label_default(255, range), "S9+30");
    }
}
