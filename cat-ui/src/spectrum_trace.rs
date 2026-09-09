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

//! The live trace that belongs above a waterfall.
//!
//! A waterfall shows history and hides the present: the newest row is one
//! pixel high, so the thing an operator is actually tuning is the hardest
//! part of the picture to read. Every real spectrum analyser answers this
//! the same way, with a trace above the fall, and this is that trace.
//!
//! It is built on [`sample_column`](crate::spectrum_map::sample_column),
//! the same projection and the same intensity scale the waterfall itself
//! uses, so column *n* of the trace is column *n* of the fall. Anything
//! else would put a peak in the trace above a different frequency in the
//! fall, which is worse than having no trace: it would look right.
//!
//! Peak hold decays on a clock rather than per frame. Frame rate here is
//! whatever the SDR and the network are managing at the time -- it was
//! measured between 10 and 29 a second on this bench -- and a per-frame
//! decay would make the hold fade at a speed that varied with the link.

use std::time::{Duration, Instant};

use cat_signal::SpectrumFrame;

use crate::spectrum_map::{sample_column, Sample};

/// How long a peak takes to fall from the top of the display to the
/// bottom, with no signal under it.
///
/// Long enough to catch a signal an operator glanced away from, short
/// enough that the display is not still showing a station that left the
/// band. Two seconds is the convention on bench analysers and reads right
/// here too.
pub const PEAK_FALL: Duration = Duration::from_secs(2);

/// A live trace and its peak hold, in display columns.
///
/// Both are 0.0 at the noise floor and 1.0 at the reference level, which
/// is the same scale the waterfall's palette is indexed by.
pub struct Trace {
    live: Vec<Option<f32>>,
    peak: Vec<Option<f32>>,
    updated: Option<Instant>,
}

impl Trace {
    pub fn new() -> Self {
        Self {
            live: Vec::new(),
            peak: Vec::new(),
            updated: None,
        }
    }

    /// The newest frame, projected onto `width` columns.
    ///
    /// `None` in a column means this frame never saw that frequency --
    /// the dial moved. It is left as a gap rather than drawn at the noise
    /// floor, for the reason [`Sample::NoData`] gives: data in the wrong
    /// place is worse than no data.
    pub fn live(&self) -> &[Option<f32>] {
        &self.live
    }

    /// The peak hold, on the same columns and the same scale.
    pub fn peak(&self) -> &[Option<f32>] {
        &self.peak
    }

    /// Take a frame at `now`.
    ///
    /// Explicit `now` so the decay is testable without sleeping.
    pub fn push_at(&mut self, frame: &SpectrumFrame, width: u32, floor_dbm: f32, now: Instant) {
        let width = width.max(1) as usize;
        if self.live.len() != width {
            // A resized window is not a reason to show a peak hold from a
            // different set of columns.
            self.live = vec![None; width];
            self.peak = vec![None; width];
            self.updated = None;
        }

        let fall = match self.updated {
            Some(then) => {
                now.saturating_duration_since(then).as_secs_f32()
                    / PEAK_FALL.as_secs_f32().max(f32::EPSILON)
            }
            // First frame: the peak starts at the signal, not above it.
            None => 1.0,
        };
        self.updated = Some(now);

        for column in 0..width {
            let sample = sample_column(frame, frame, column as u32, width as u32, floor_dbm);
            let value = match sample {
                Sample::Signal(v) => Some(v),
                Sample::NoData => None,
            };
            self.live[column] = value;
            self.peak[column] = match (self.peak[column], value) {
                // A new peak is taken instantly; that is the point of it.
                (Some(held), Some(v)) if v >= held => Some(v),
                (Some(held), Some(v)) => Some((held - fall).max(v)),
                // Nothing under it any more: keep falling, to zero.
                (Some(held), None) => {
                    let faded = held - fall;
                    (faded > 0.0).then_some(faded)
                }
                (None, v) => v,
            };
        }
    }

    /// The same, at the current instant.
    pub fn push(&mut self, frame: &SpectrumFrame, width: u32, floor_dbm: f32) {
        self.push_at(frame, width, floor_dbm, Instant::now());
    }

    /// Forget everything, for a retune.
    ///
    /// After the dial moves, a held peak is a peak from a frequency that
    /// is no longer on screen.
    pub fn clear(&mut self) {
        self.live.iter_mut().for_each(|v| *v = None);
        self.peak.iter_mut().for_each(|v| *v = None);
        self.updated = None;
    }
}

impl Default for Trace {
    fn default() -> Self {
        Self::new()
    }
}

/// The dBm a 0.0-1.0 trace value stands for.
///
/// The inverse of `intensity`, so a grid line drawn at some fraction of
/// the height can be labelled with the power it actually represents
/// instead of a bare percentage.
pub fn dbm_at(value: f32, floor_dbm: f32, ref_level_dbm: f32) -> f32 {
    floor_dbm + value * (ref_level_dbm - floor_dbm)
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
    fn a_trace_has_one_value_per_display_column() {
        let mut trace = Trace::new();
        trace.push(&frame(14_074_000, vec![-100.0; 8]), 64, -120.0);
        assert_eq!(trace.live().len(), 64);
        assert_eq!(trace.peak().len(), 64);
    }

    #[test]
    fn the_trace_uses_the_same_columns_as_the_waterfall() {
        // The whole reason this module exists rather than a private
        // helper in a widget crate: a peak in the trace must sit above
        // the same frequency in the fall. A trace that disagreed would
        // still look right.
        let f = frame(14_074_000, vec![-120.0, -60.0, -120.0, -120.0]);
        let mut trace = Trace::new();
        trace.push(&f, 64, -120.0);
        for column in 0..64u32 {
            let expected = match sample_column(&f, &f, column, 64, -120.0) {
                Sample::Signal(v) => Some(v),
                Sample::NoData => None,
            };
            assert_eq!(trace.live()[column as usize], expected, "column {column}");
        }
    }

    #[test]
    fn a_peak_is_taken_immediately_and_falls_slowly() {
        let start = Instant::now();
        let mut trace = Trace::new();
        let loud = frame(14_074_000, vec![-20.0; 4]);
        let quiet = frame(14_074_000, vec![-120.0; 4]);

        trace.push_at(&loud, 8, -120.0, start);
        assert_eq!(trace.peak()[0], Some(1.0), "a peak is taken at once");

        trace.push_at(&quiet, 8, -120.0, start + Duration::from_millis(10));
        assert_eq!(trace.live()[0], Some(0.0), "the live trace drops at once");
        let held = trace.peak()[0].expect("the hold survives");
        assert!(held > 0.9, "and falls slowly: {held}");
    }

    #[test]
    fn a_peak_reaches_the_floor_after_the_fall_time() {
        let start = Instant::now();
        let mut trace = Trace::new();
        trace.push_at(&frame(14_074_000, vec![-20.0; 4]), 8, -120.0, start);
        trace.push_at(
            &frame(14_074_000, vec![-120.0; 4]),
            8,
            -120.0,
            start + PEAK_FALL,
        );
        assert_eq!(
            trace.peak()[0],
            Some(0.0),
            "a full fall time takes it all the way down"
        );
    }

    #[test]
    fn a_louder_signal_raises_the_hold_rather_than_waiting_for_it_to_fall() {
        let start = Instant::now();
        let mut trace = Trace::new();
        trace.push_at(&frame(14_074_000, vec![-70.0; 4]), 8, -120.0, start);
        let first = trace.peak()[0].unwrap();
        trace.push_at(
            &frame(14_074_000, vec![-20.0; 4]),
            8,
            -120.0,
            start + Duration::from_millis(30),
        );
        assert!(trace.peak()[0].unwrap() > first);
        assert_eq!(trace.peak()[0], Some(1.0));
    }

    #[test]
    fn a_resize_starts_the_hold_over_rather_than_reusing_other_columns() {
        let mut trace = Trace::new();
        trace.push(&frame(14_074_000, vec![-20.0; 4]), 64, -120.0);
        trace.push(&frame(14_074_000, vec![-120.0; 4]), 128, -120.0);
        assert_eq!(trace.live().len(), 128);
        assert_eq!(
            trace.peak()[0],
            Some(0.0),
            "a hold from a different column count is not this column's history"
        );
    }

    #[test]
    fn clearing_drops_a_hold_from_a_frequency_that_has_left_the_screen() {
        let mut trace = Trace::new();
        trace.push(&frame(14_074_000, vec![-20.0; 4]), 8, -120.0);
        trace.clear();
        assert!(trace.peak().iter().all(|v| v.is_none()));
        assert!(trace.live().iter().all(|v| v.is_none()));
    }

    #[test]
    fn an_empty_frame_leaves_gaps_rather_than_a_floor() {
        // `NoData` must not be drawn as the noise floor: the dial had
        // moved, and inventing a floor there is data in the wrong place.
        let mut trace = Trace::new();
        trace.push(&frame(14_074_000, Vec::new()), 8, -120.0);
        assert!(trace.live().iter().all(|v| v.is_none()));
    }

    #[test]
    fn a_zero_width_display_does_not_panic() {
        let mut trace = Trace::new();
        trace.push(&frame(14_074_000, vec![-100.0; 4]), 0, -120.0);
        assert_eq!(trace.live().len(), 1);
    }

    #[test]
    fn a_grid_line_can_be_labelled_with_the_power_it_stands_for() {
        assert_eq!(dbm_at(0.0, -120.0, -20.0), -120.0);
        assert_eq!(dbm_at(1.0, -120.0, -20.0), -20.0);
        assert_eq!(dbm_at(0.5, -120.0, -20.0), -70.0);
    }
}
