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

//! Turning spectrum frames into something a renderer can draw.
//!
//! This module is the answer to a question that has exactly one correct
//! answer per input and no renderer opinion at all: **where does this bin
//! go, and how bright is it?** Both widget crates need it, and both would
//! otherwise write it — which is the duplication ADR 0011 exists to stop,
//! and which nearly happened here: `cat-ui-ratatui` had its own copy first.
//!
//! Nothing here knows about cells or pixels. Widths are in *display units*,
//! whatever the renderer's unit happens to be.

use cat_signal::SpectrumFrame;

/// Where `frame`'s bins land relative to `reference`, in display units.
///
/// This is the re-projection that keeps a waterfall's frequency axis true
/// for every row rather than only its newest. It exists because **an IF tap
/// is dial-centred**: the SDR is parked on the intermediate frequency while
/// the radio's local oscillator tracks the dial, so every retune changes a
/// frame's `center_hz`, and a row captured a moment ago describes a
/// different slice of spectrum than the one below it.
///
/// For a source whose centre never moves — a `DirectSdr` with a fixed
/// tuner — every row shares a centre and this returns zero. That is why a
/// renderer can apply it unconditionally instead of branching on
/// `SignalCapability`, which would make the *widget* radio-specific.
pub fn projection_offset(frame: &SpectrumFrame, reference: &SpectrumFrame, width: u32) -> i32 {
    if reference.span_hz == 0 || width == 0 {
        return 0;
    }
    let delta = frame.center_hz as i64 - reference.center_hz as i64;
    ((delta as f64 / f64::from(reference.span_hz)) * f64::from(width)).round() as i32
}

/// Normalize a bin's power to 0.0-1.0 between a noise floor and a
/// reference level, clamped.
pub fn intensity(dbm: f32, floor_dbm: f32, ref_dbm: f32) -> f32 {
    if (ref_dbm - floor_dbm).abs() < f32::EPSILON {
        return 0.0;
    }
    ((dbm - floor_dbm) / (ref_dbm - floor_dbm)).clamp(0.0, 1.0)
}

/// The half-open range of bins a display column covers.
///
/// A display almost always has fewer columns than the frame has bins -- 64
/// columns over 2048 bins is ordinary -- so each column stands for several.
pub fn column_bins(bins: usize, column: u32, width: u32) -> std::ops::Range<usize> {
    if bins == 0 || width == 0 {
        return 0..0;
    }
    let lo = ((u64::from(column) * bins as u64) / u64::from(width)) as usize;
    let hi = ((u64::from(column + 1) * bins as u64) / u64::from(width)) as usize;
    // Never empty: with more columns than bins, several columns share one.
    let lo = lo.min(bins - 1);
    lo..hi.max(lo + 1).min(bins)
}

/// The bin a column reports: the **strongest** one it covers.
///
/// Peak-hold, not averaging and not nearest-bin, and the distinction is not
/// cosmetic on a panadapter.
///
/// *Averaging* buries a narrow carrier: a CW signal one bin wide, averaged
/// across the four bins a column covers, loses most of its height and sinks
/// toward the noise floor.
///
/// *Nearest-bin* is worse, and was this function's first implementation --
/// it samples one bin in four and never looks at the other three, so a
/// carrier that lands between sampled bins **disappears entirely**. That is
/// a console hiding signals, which is worse than showing them dimly, and it
/// was caught only because a test happened to place a peak on an unsampled
/// bin and the peak vanished.
///
/// Peak-hold keeps a one-bin carrier at full height wherever in the column
/// it falls. The cost is a noise floor that reads slightly high, which is
/// the right trade for an instrument whose job is to tell you something is
/// there.
pub fn bin_for_column(bins: &[f32], column: u32, width: u32) -> Option<usize> {
    let range = column_bins(bins.len(), column, width);
    if range.is_empty() {
        return None;
    }
    range.max_by(|a, b| {
        bins[*a]
            .partial_cmp(&bins[*b])
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// What a display column should show for one row of a waterfall.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sample {
    /// This row saw this frequency, at the given 0.0-1.0 intensity.
    Signal(f32),
    /// This row never captured this frequency — the dial had moved.
    ///
    /// A renderer must draw this distinctly and must **not** substitute a
    /// noise floor. Data in the wrong place is worse than no data, and the
    /// staircase these gaps make as the dial moves is itself the tuning
    /// history.
    NoData,
}

/// Sample one column of one waterfall row, re-projected onto `reference`.
pub fn sample_column(
    frame: &SpectrumFrame,
    reference: &SpectrumFrame,
    column: u32,
    width: u32,
    floor_dbm: f32,
) -> Sample {
    if frame.bins.is_empty() {
        return Sample::NoData;
    }
    let offset = projection_offset(frame, reference, width);
    let src = i64::from(column) - i64::from(offset);
    if src < 0 || src >= i64::from(width) {
        return Sample::NoData;
    }
    let Some(bin) = bin_for_column(&frame.bins, src as u32, width) else {
        return Sample::NoData;
    };
    Sample::Signal(intensity(frame.bins[bin], floor_dbm, frame.ref_level_dbm))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(center_hz: u64, bins: Vec<f32>) -> SpectrumFrame {
        SpectrumFrame {
            center_hz,
            span_hz: 96_000,
            ref_level_dbm: -20.0,
            bins,
            sequence: 1,
        }
    }

    #[test]
    fn rows_sharing_a_centre_are_not_shifted() {
        let a = frame(14_074_000, vec![-100.0; 8]);
        let b = frame(14_074_000, vec![-100.0; 8]);
        assert_eq!(projection_offset(&b, &a, 64), 0);
    }

    #[test]
    fn a_row_from_higher_up_the_band_shifts_right() {
        let reference = frame(14_074_000, vec![-100.0; 8]);
        let older = frame(14_098_000, vec![-100.0; 8]);
        // 24 kHz of a 96 kHz span is a quarter; a quarter of 64 is 16.
        assert_eq!(projection_offset(&older, &reference, 64), 16);
    }

    #[test]
    fn a_row_from_lower_down_the_band_shifts_left() {
        let reference = frame(14_074_000, vec![-100.0; 8]);
        let older = frame(14_050_000, vec![-100.0; 8]);
        assert_eq!(projection_offset(&older, &reference, 64), -16);
    }

    #[test]
    fn a_degenerate_span_or_width_does_not_divide_by_zero() {
        let mut a = frame(14_074_000, vec![-100.0; 8]);
        let b = frame(14_098_000, vec![-100.0; 8]);
        a.span_hz = 0;
        assert_eq!(projection_offset(&b, &a, 64), 0);
        assert_eq!(projection_offset(&b, &frame(14_074_000, vec![]), 0), 0);
    }

    #[test]
    fn columns_a_row_never_captured_report_no_data() {
        // Half a span away: half this row's columns have no data at all.
        let reference = frame(14_074_000, vec![-40.0; 64]);
        let shifted = frame(14_074_000 + 48_000, vec![-40.0; 64]);
        assert_eq!(
            sample_column(&shifted, &reference, 0, 64, -120.0),
            Sample::NoData
        );
        assert!(matches!(
            sample_column(&shifted, &reference, 63, 64, -120.0),
            Sample::Signal(_)
        ));
    }

    #[test]
    fn an_empty_frame_is_no_data_rather_than_silence() {
        // Silence is a measurement; an empty frame is the absence of one.
        let reference = frame(14_074_000, vec![-40.0; 8]);
        let empty = frame(14_074_000, vec![]);
        assert_eq!(
            sample_column(&empty, &reference, 0, 8, -120.0),
            Sample::NoData
        );
    }

    #[test]
    fn a_column_covers_a_contiguous_range_of_bins() {
        assert_eq!(column_bins(256, 0, 64), 0..4);
        assert_eq!(column_bins(256, 63, 64), 252..256);
        assert_eq!(column_bins(256, 32, 64), 128..132);
    }

    #[test]
    fn ranges_tile_the_frame_with_no_gaps_and_no_overlap() {
        // A gap hides a slice of spectrum; an overlap draws the same signal
        // twice. Either is a lie about what is on the band.
        let mut next = 0;
        for c in 0..64 {
            let r = column_bins(256, c, 64);
            assert_eq!(r.start, next, "gap or overlap at column {c}");
            next = r.end;
        }
        assert_eq!(next, 256, "the last column must reach the end");
    }

    #[test]
    fn more_columns_than_bins_still_yields_a_bin_each() {
        // A wide window over a small FFT. Every column must resolve to
        // something, rather than some columns silently drawing nothing.
        for c in 0..64 {
            assert!(!column_bins(8, c, 64).is_empty(), "column {c} was empty");
        }
    }

    #[test]
    fn a_column_reports_its_strongest_bin_not_its_first() {
        let bins = vec![-110.0f32, -110.0, -20.0, -110.0];
        assert_eq!(bin_for_column(&bins, 0, 1), Some(2));
    }

    #[test]
    fn a_narrow_carrier_survives_downsampling_wherever_it_falls() {
        // THE test for this choice. 256 bins into 64 columns: each column
        // covers four bins, and the carrier must survive in all four
        // positions. Nearest-bin passed this for one position in four,
        // which is exactly how it hid signals.
        for offset in 0..4 {
            let mut bins = vec![-110.0f32; 256];
            let carrier = 128 + offset;
            bins[carrier] = -20.0;
            let f = frame(14_074_000, bins);
            let hit = (0..64)
                .filter(|c| {
                    matches!(
                        sample_column(&f, &f, *c, 64, -120.0),
                        Sample::Signal(t) if t > 0.99
                    )
                })
                .count();
            assert_eq!(hit, 1, "a carrier at bin {carrier} was lost");
        }
    }

    #[test]
    fn bin_selection_survives_degenerate_inputs() {
        assert_eq!(bin_for_column(&[], 5, 64), None);
        assert_eq!(bin_for_column(&[-100.0], 0, 0), None);
        assert_eq!(bin_for_column(&[-100.0, -20.0], 999, 2), Some(1));
    }

    #[test]
    fn intensity_clamps_at_both_ends() {
        assert_eq!(intensity(-20.0, -120.0, -20.0), 1.0);
        assert_eq!(intensity(-120.0, -120.0, -20.0), 0.0);
        assert_eq!(intensity(0.0, -120.0, -20.0), 1.0);
        assert_eq!(intensity(-999.0, -120.0, -20.0), 0.0);
    }

    #[test]
    fn a_flat_reference_does_not_divide_by_zero() {
        assert_eq!(intensity(-50.0, -20.0, -20.0), 0.0);
    }
}
