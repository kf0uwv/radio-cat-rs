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
