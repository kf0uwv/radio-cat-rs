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

//! What an AF panel shows, before anything decides how to draw it.
//!
//! The arithmetic here — where a column sits in audio hertz, how loud that
//! makes it look, where the filter edges fall — is the same question in
//! every renderer, and the answer has to be the same too. Two consoles
//! showing the same radio at the same moment with different bar heights
//! would leave an operator unable to trust either.
//!
//! So this holds the decisions and the drawing lives in `cat-ui-ratatui`
//! and `cat-ui-egui`, the same split that `workspace`, `quick` and
//! `command` already use.

use cat_native::{CapabilitiesWire, ModeId, ModeKind};
use cat_signal::AudioSpectrumFrame;

/// The AF FFT's full span.
///
/// Fixed rather than taken from the frame. A display whose scale moved
/// with its data would make two moments incomparable, and the span is a
/// property of the panel here, not of the signal.
pub const AF_FFT_SPAN_HZ: f32 = 3_000.0;

/// How far below the loudest bin the panel floor sits.
///
/// A fixed window, not min-to-max normalization. Normalizing to the range
/// present would paint a silent panel solid the moment the noise floor
/// wobbled, because the quietest bin would always be at the bottom and the
/// loudest always at the top however little separated them.
pub const AF_RANGE_DB: f32 = 40.0;

/// What a console knows about the audio path, and it is three things.
///
/// The middle one is why this is not a `bool`: "configured but nothing has
/// arrived" and "nothing is wired at all" send an operator to opposite
/// ends of the shack, and a panel that showed both as empty would tell
/// them nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioState {
    /// No audio path at all — this installation has nothing wired.
    Absent,
    /// An audio path is configured, but no samples have arrived. **The
    /// default**: the pair exists on this station, and whether the
    /// transport is up is a separate question from the panels.
    #[default]
    Configured,
    /// Samples are arriving.
    Streaming,
}

impl AudioState {
    /// The one-word label a panel header carries.
    ///
    /// Always shown, including when it is `LIVE`. A panel that only labels
    /// itself when something is wrong leaves an operator unable to tell
    /// "working" from "not labelled yet".
    pub fn label(self) -> &'static str {
        match self {
            AudioState::Absent => "NONE",
            AudioState::Configured => "PENDING",
            AudioState::Streaming => "LIVE",
        }
    }

    /// Whether a renderer should be drawing live data.
    pub fn is_streaming(self) -> bool {
        self == AudioState::Streaming
    }
}

/// The receive passband, in audio hertz, for the filter edge marks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Passband {
    pub low_hz: f32,
    pub high_hz: f32,
}

impl Passband {
    /// Where an edge falls across a panel `columns` wide, if it falls
    /// inside the panel at all.
    ///
    /// `None` for an edge off the end of the span rather than a clamped
    /// value at the last column: a mark drawn at the edge of the panel
    /// would claim the filter ends there, which is a different and wrong
    /// statement about the radio.
    pub fn edge_columns(&self, columns: usize) -> (Option<usize>, Option<usize>) {
        (
            column_for_hz(self.low_hz, columns),
            column_for_hz(self.high_hz, columns),
        )
    }
}

/// Which column an audio frequency lands in, or `None` if off-panel.
pub fn column_for_hz(hz: f32, columns: usize) -> Option<usize> {
    if columns == 0 || !(0.0..AF_FFT_SPAN_HZ).contains(&hz) {
        return None;
    }
    let col = (hz / AF_FFT_SPAN_HZ * columns as f32) as usize;
    Some(col.min(columns - 1))
}

/// The height of one column, 0.0 to 1.0, or `None` if nothing is there.
///
/// Peak rather than mean across the column: at 150 Hz per column a mean
/// buries a single tone under the silence either side of it, and a tone is
/// the thing an operator is looking for.
pub fn column_energy(frame: &AudioSpectrumFrame, lo_hz: f32, hi_hz: f32) -> Option<f32> {
    if frame.bins.is_empty() {
        return None;
    }
    let mut top = f32::NEG_INFINITY;
    for &b in &frame.bins {
        top = top.max(b);
    }
    if !top.is_finite() {
        return None;
    }

    let mut peak = f32::NEG_INFINITY;
    for (i, &b) in frame.bins.iter().enumerate() {
        let hz = frame.audio_frequency_hz(i)? as f32;
        if hz >= lo_hz && hz < hi_hz {
            peak = peak.max(b);
        }
    }
    if !peak.is_finite() {
        return None;
    }
    Some(((peak - (top - AF_RANGE_DB)) / AF_RANGE_DB).clamp(0.0, 1.0))
}

/// How many columns this frame can actually answer for.
///
/// A renderer with more pixels than the frame has bins will leave gaps —
/// every column that no bin falls into reports `None`, and the panel
/// combs. The gaps are honest, but they read as structure in the signal
/// rather than as the display outrunning its data, which is the worse of
/// the two mistakes.
///
/// So a renderer asks for the smaller of what it can draw and what the
/// frame can say.
pub fn resolvable_columns(frame: &AudioSpectrumFrame) -> usize {
    (0..frame.bins.len())
        .filter_map(|i| frame.audio_frequency_hz(i))
        .filter(|&hz| (hz as f32) < AF_FFT_SPAN_HZ)
        .count()
        .max(1)
}

/// Every column's height for a panel `columns` wide.
///
/// The whole panel in one call, so a renderer loops over answers rather
/// than re-deriving the axis. `None` in a slot means the frame has nothing
/// to say about that column, which is not the same as zero.
pub fn columns(frame: &AudioSpectrumFrame, columns: usize) -> Vec<Option<f32>> {
    if columns == 0 {
        return Vec::new();
    }
    let hz_per_col = AF_FFT_SPAN_HZ / columns as f32;
    (0..columns)
        .map(|c| {
            let lo = c as f32 * hz_per_col;
            column_energy(frame, lo, lo + hz_per_col)
        })
        .collect()
}

/// The receive passband to mark, for a mode this radio declares.
///
/// Derived from the capability set rather than from a table of one
/// radio's filters, so a console drawing a mode's passband is drawing the
/// bandwidth that radio actually published.
///
/// `None` for CW. The audio a CW receiver produces sits at the operator's
/// sidetone pitch, which is a menu setting and not a property of the mode,
/// and a mark drawn at a guessed pitch would be worse than no mark: it
/// would look like a measurement.
///
/// The axis starts at 0 because it plots *audio* frequency, which has no
/// sideband — a lower-sideband passband occupies the same audio hertz as
/// its upper-sideband mirror, so both come back identical.
pub fn passband_for(caps: &CapabilitiesWire, mode: ModeId) -> Option<Passband> {
    let descriptor = caps.modes.iter().find(|m| m.id == mode)?;
    if descriptor.kind == ModeKind::Cw {
        return None;
    }
    let width = descriptor.default_bandwidth_hz as f32;
    if width <= 0.0 {
        return None;
    }
    match descriptor.kind {
        // SSB and the data modes sit above a suppressed carrier, so the
        // audio runs from the low cut to the low cut plus the bandwidth.
        // 300 Hz is where a communications receiver's response starts,
        // and it is what makes the low mark land somewhere meaningful
        // rather than at zero.
        ModeKind::Ssb | ModeKind::Data => Some(Passband {
            low_hz: SSB_LOW_CUT_HZ,
            high_hz: SSB_LOW_CUT_HZ + width,
        }),
        // AM and FM are carrier-centred: the audio is the modulation, and
        // it runs from DC up to half the RF bandwidth.
        _ => Some(Passband {
            low_hz: 0.0,
            high_hz: width / 2.0,
        }),
    }
}

/// Where a communications receiver's SSB response starts.
///
/// Not a per-radio number — it is the shoulder of the standard voice
/// passband, and every radio in this fleet is within a few tens of hertz
/// of it. A radio that published its own low cut would be worth reading
/// instead; none does.
pub const SSB_LOW_CUT_HZ: f32 = 300.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(bins: Vec<f32>, span_hz: u32) -> AudioSpectrumFrame {
        AudioSpectrumFrame {
            start_hz: 0,
            span_hz,
            bins,
            sequence: 1,
        }
    }

    #[test]
    fn a_small_spread_stays_a_small_spread() {
        // The bug a fixed window exists to prevent, stated as what it
        // actually guarantees. Min-to-max normalization would stretch a
        // 3 dB difference across the whole panel height, so a silent
        // radio would show a jagged skyline that moved with the noise.
        // Against a fixed 40 dB window the same 3 dB is 3/40 of the
        // panel, and the trace stays visibly flat.
        //
        // Note what this does *not* claim: the window is anchored to the
        // peak, so a quiet panel sits near the top rather than near the
        // bottom. An AF FFT is a shape, not a power meter -- see
        // `the_floor_sits_a_fixed_distance_below_the_peak`.
        let quiet = frame(vec![-100.0, -98.0, -99.0, -97.0], 4_000);
        let heights: Vec<f32> = columns(&quiet, 20).into_iter().flatten().collect();
        let lo = heights.iter().cloned().fold(f32::INFINITY, f32::min);
        let hi = heights.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!(
            hi - lo < 0.1,
            "a 3 dB spread became {:.0}% of the panel: {heights:?}",
            (hi - lo) * 100.0
        );
    }

    #[test]
    fn a_real_signal_reaches_the_top() {
        let loud = frame(vec![-100.0, -100.0, -55.0, -100.0], 4_000);
        let heights: Vec<f32> = columns(&loud, 20).into_iter().flatten().collect();
        assert!(
            heights.iter().any(|&h| h > 0.9),
            "the loudest bin should reach the top: {heights:?}"
        );
    }

    #[test]
    fn the_floor_sits_a_fixed_distance_below_the_peak() {
        // Anything more than AF_RANGE_DB down is at the floor, whatever
        // the absolute levels are. The same signal 30 dB louder must look
        // identical -- an AF panel is a shape, not a power meter.
        let quiet = frame(vec![-100.0, -50.0], 4_000);
        let loud = frame(vec![-70.0, -20.0], 4_000);
        assert_eq!(columns(&quiet, 8), columns(&loud, 8));
    }

    #[test]
    fn the_axis_does_not_move_with_the_data() {
        // A panel whose scale followed its frame would make two moments
        // incomparable. 1500 Hz is the middle of the span regardless of
        // what the source's own span happens to be.
        assert_eq!(column_for_hz(1_500.0, 20), Some(10));
        assert_eq!(column_for_hz(0.0, 20), Some(0));
        assert_eq!(column_for_hz(2_999.0, 20), Some(19));
    }

    #[test]
    fn an_edge_off_the_panel_is_not_drawn_at_its_rim() {
        // Clamping would claim the filter ends where the panel does, which
        // is a different statement about the radio and a wrong one.
        assert_eq!(column_for_hz(3_500.0, 20), None);
        assert_eq!(column_for_hz(-10.0, 20), None);

        let wide = Passband {
            low_hz: 100.0,
            high_hz: 4_000.0,
        };
        let (low, high) = wide.edge_columns(20);
        assert_eq!(low, Some(0));
        assert_eq!(high, None, "an edge beyond the span must not be marked");
    }

    #[test]
    fn an_ssb_passband_marks_both_edges_inside_the_panel() {
        let ssb = Passband {
            low_hz: 300.0,
            high_hz: 2_700.0,
        };
        let (low, high) = ssb.edge_columns(20);
        assert_eq!((low, high), (Some(2), Some(18)));
    }

    #[test]
    fn an_empty_frame_says_nothing_rather_than_zero() {
        // Zero is a height. "No data" is not, and a renderer has to be
        // able to tell them apart to leave the column blank.
        assert!(columns(&frame(Vec::new(), 4_000), 4)
            .iter()
            .all(|c| c.is_none()));
    }

    #[test]
    fn a_panel_finer_than_its_data_is_capped_rather_than_combed() {
        // Asking for more columns than the frame has bins leaves a gap in
        // every column no bin lands in, and a comb reads as structure in
        // the signal rather than as the display outrunning its data.
        let f = frame(vec![-90.0; 40], 4_000);
        let useful = resolvable_columns(&f);
        assert!(useful <= 40, "{useful}");

        let capped = columns(&f, useful);
        assert!(
            capped.iter().all(|c| c.is_some()),
            "even at its own resolution the panel had holes: {capped:?}"
        );

        let overdrawn = columns(&f, useful * 4);
        assert!(
            overdrawn.iter().any(|c| c.is_none()),
            "the comb this exists to avoid did not appear, so the test proves nothing"
        );
    }

    #[test]
    fn a_zero_width_panel_asks_for_nothing() {
        assert!(columns(&frame(vec![-50.0], 4_000), 0).is_empty());
        assert_eq!(column_for_hz(1_000.0, 0), None);
    }

    #[test]
    fn the_three_states_are_labelled_distinctly() {
        let labels = [
            AudioState::Absent.label(),
            AudioState::Configured.label(),
            AudioState::Streaming.label(),
        ];
        let unique: std::collections::HashSet<_> = labels.iter().collect();
        assert_eq!(unique.len(), 3, "{labels:?}");
        assert!(AudioState::Streaming.is_streaming());
        assert!(!AudioState::Configured.is_streaming());
    }
}

#[cfg(test)]
mod passband_tests {
    use super::*;

    fn caps() -> CapabilitiesWire {
        cat_native::testing::stub_capabilities()
    }

    #[test]
    fn both_sidebands_mark_the_same_audio_hertz() {
        // The AF axis plots audio frequency, which has no sideband. A
        // console that marked LSB and USB differently would be claiming a
        // difference the panel cannot show.
        let usb = passband_for(&caps(), ModeId::Usb).unwrap();
        let lsb = passband_for(&caps(), ModeId::Lsb).unwrap();
        assert_eq!(usb, lsb);
        assert!(usb.low_hz > 0.0 && usb.high_hz > usb.low_hz);
    }

    #[test]
    fn the_width_comes_from_what_the_radio_published() {
        // Not a table of one radio's filters. A radio declaring 2400 Hz
        // gets a 2400 Hz mark.
        let caps = caps();
        let declared = caps
            .modes
            .iter()
            .find(|m| m.id == ModeId::Usb)
            .unwrap()
            .default_bandwidth_hz as f32;
        let pb = passband_for(&caps, ModeId::Usb).unwrap();
        assert_eq!(pb.high_hz - pb.low_hz, declared);
    }

    #[test]
    fn a_mode_this_radio_does_not_have_marks_nothing() {
        assert_eq!(passband_for(&caps(), ModeId::C4fm), None);
    }

    #[test]
    fn the_marks_land_inside_the_panel() {
        // A passband whose edges fell off the span would be marked
        // nowhere, and the operator would think the console had no filter
        // information at all.
        let pb = passband_for(&caps(), ModeId::Usb).unwrap();
        let (low, high) = pb.edge_columns(40);
        assert!(low.is_some() && high.is_some(), "{pb:?}");
    }
}
