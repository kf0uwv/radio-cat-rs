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

//! The two-rate discipline.
//!
//! Spectrum frames are push, high-rate, ~60 fps. CAT state is
//! request/response and can take hundreds of milliseconds. Keeping them in
//! separate structures is what stops a renderer blocking a waterfall behind
//! a menu read — a console that stutters whenever the operator opens a
//! menu feels broken even though every individual part of it works.
//!
//! [`ConsoleState`] holds both lanes and deliberately offers **no** way to
//! wait on one from the other.

use crate::spectrum::SpectrumHistory;
use cat_framework::capabilities::MeterKind;
use cat_framework::installation::{Installation, Session, SourceState};
use cat_signal::SpectrumFrame;

/// Slow lane: whatever the radio last told us, and how stale it is.
///
/// Every field is `Option`, because "not asked yet" and "asked, and the
/// answer is zero" are different states. A VFO readout showing 0.000.000
/// MHz because nothing has been read yet is a lie a console should not
/// tell.
#[derive(Debug, Clone, Default)]
pub struct CatLane {
    pub vfo_a_hz: Option<u64>,
    pub vfo_b_hz: Option<u64>,
    pub mode: Option<cat_framework::capabilities::ModeId>,
    pub split: Option<bool>,
    pub transmitting: Option<bool>,
    /// Raw meter readings, paired with kind. Scaling happens at render
    /// time against the radio's own `MeterSet`.
    pub meters: Vec<(MeterKind, u16)>,
    /// Commands issued but not yet acknowledged.
    ///
    /// The count a renderer needs to show a pending state. ADR 0008's
    /// designer brief puts it bluntly: a control that looks instantaneous
    /// but is not will read as broken, so the pending state has to be
    /// designed before the resting one — which means it has to exist here.
    pub pending: usize,
}

impl CatLane {
    /// Note a command that has been sent and not yet answered.
    pub fn begin_command(&mut self) {
        self.pending += 1;
    }

    /// Note a command that has been answered, one way or the other.
    pub fn end_command(&mut self) {
        self.pending = self.pending.saturating_sub(1);
    }

    pub fn is_busy(&self) -> bool {
        self.pending > 0
    }

    /// The last reading for `kind`, if the radio has reported one.
    pub fn meter(&self, kind: MeterKind) -> Option<u16> {
        self.meters
            .iter()
            .rev()
            .find(|(k, _)| *k == kind)
            .map(|(_, v)| *v)
    }

    /// Record a meter reading, replacing any previous one of that kind.
    pub fn set_meter(&mut self, kind: MeterKind, raw: u16) {
        if let Some(slot) = self.meters.iter_mut().find(|(k, _)| *k == kind) {
            slot.1 = raw;
        } else {
            self.meters.push((kind, raw));
        }
    }

    /// Whether anything at all has been read yet.
    pub fn is_cold(&self) -> bool {
        self.vfo_a_hz.is_none() && self.mode.is_none() && self.meters.is_empty()
    }
}

/// What a source lane can be. Three states, not two.
///
/// The middle one is why this is not a `bool`. A path can be wired and
/// configured while nothing arrives — which is the state of an audio
/// endpoint whose transport design has not been written. Collapsing it into
/// "absent" makes a console report missing hardware that is sitting right
/// there, plugged in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneState {
    /// Nothing is attached, and a console should say so as a first-class
    /// state rather than an error.
    Absent,
    /// Attached and configured; no data arriving yet.
    Configured,
    /// Delivering.
    Streaming,
}

impl LaneState {
    fn from(state: Option<SourceState>) -> Self {
        match state {
            None => LaneState::Absent,
            Some(SourceState::Configured) => LaneState::Configured,
            Some(SourceState::Streaming) => LaneState::Streaming,
        }
    }

    /// Whether there is anything to draw right now.
    pub fn has_data(&self) -> bool {
        *self == LaneState::Streaming
    }

    /// Whether the hardware exists, whether or not it is delivering.
    pub fn is_present(&self) -> bool {
        *self != LaneState::Absent
    }
}

/// Fast lane: spectrum history and nothing that can block.
#[derive(Debug, Clone)]
pub struct SpectrumLane {
    pub history: SpectrumHistory,
    pub state: LaneState,
}

impl SpectrumLane {
    pub fn new(capacity: usize, state: LaneState) -> Self {
        Self {
            history: SpectrumHistory::new(capacity),
            state,
        }
    }

    pub fn push(&mut self, frame: SpectrumFrame) {
        self.history.push(frame);
    }
}

/// Every lane, with no path from one to the other.
#[derive(Debug, Clone)]
pub struct ConsoleState {
    pub cat: CatLane,
    /// The band panorama. Drawn as a waterfall.
    pub spectrum: SpectrumLane,
    /// Audio-derived spectrum. **Never** a band panorama — it is audio
    /// bandwidth only, and a console that fed it to a waterfall would
    /// stretch a few kHz across a whole band and look authoritative doing
    /// it. A separate lane is what makes that mistake require effort.
    pub audio: SpectrumLane,
}

impl ConsoleState {
    /// Build state for one radio on one bench.
    ///
    /// Takes a [`Session`] rather than a `RadioCapabilities`, because
    /// whether a source is attached is a fact about the bench (ADR 0015).
    /// A lane always exists, even when nothing is connected, so a layout
    /// can reserve its space instead of reflowing when a tap is plugged in.
    pub fn for_session(session: &Session, history_capacity: usize) -> Self {
        Self::for_installation(&session.installation, history_capacity)
    }

    pub fn for_installation(installation: &Installation, history_capacity: usize) -> Self {
        Self {
            cat: CatLane::default(),
            spectrum: SpectrumLane::new(
                history_capacity,
                LaneState::from(installation.band_panorama().map(|s| s.state)),
            ),
            audio: SpectrumLane::new(
                history_capacity,
                LaneState::from(installation.audio().map(|s| s.state)),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_framework::capabilities::*;
    use cat_framework::installation::{Installation, InstalledSource, Session, SourceState};

    const MODES: &[ModeDescriptor] = &[ModeDescriptor {
        id: ModeId::Lsb,
        label: "LSB",
        kind: ModeKind::Ssb,
        sideband: Some(Sideband::Lower),
        default_bandwidth_hz: 2400,
    }];
    const METERS: &[MeterDescriptor] = &[MeterDescriptor {
        kind: MeterKind::S,
        raw_range: RawRange::new(0, 30),
        active_on_transmit: false,
    }];
    const ENDPOINTS: &[EndpointDescriptor] = &[EndpointDescriptor {
        role: EndpointRole::Cat,
        required: true,
        shareable_with: &[],
    }];

    fn radio() -> RadioCapabilities {
        RadioCapabilities {
            model: "Test",
            endpoints: EndpointSet::new(ENDPOINTS),
            vfos: VfoCapability {
                count: 2,
                split: true,
                rit_hz: None,
                xit_hz: None,
            },
            modes: MODES,
            tuning_steps_hz: &[10],
            rx_range: FrequencyRange::new(500_000, 60_000_000),
            filters: FilterCapability {
                if_shift_hz: None,
                widths_hz: None,
                notch: false,
            },
            meters: MeterSet::new(METERS),
            memory: None,
            menu: None,
            signal: SignalSupport::IfTapPoint {
                if_center_hz: 73_050_000,
                inverted: true,
            },
        }
    }

    fn tap(state: SourceState) -> InstalledSource {
        InstalledSource::new(
            cat_signal::SignalCapability::IfTap(cat_signal::IfTapConfig {
                if_center_hz: 73_050_000,
                inverted: true,
                trim_hz: -1_420,
            }),
            state,
            "RTL-SDR",
        )
    }

    fn audio_source(state: SourceState) -> InstalledSource {
        InstalledSource::new(
            cat_signal::SignalCapability::AudioDerived {
                max_bandwidth_hz: 3_000,
            },
            state,
            "USB codec",
        )
    }

    fn frame(sequence: u64) -> SpectrumFrame {
        SpectrumFrame {
            center_hz: 14_074_000,
            span_hz: 48_000,
            ref_level_dbm: -20.0,
            bins: vec![-110.0, -40.0],
            sequence,
        }
    }

    #[test]
    fn a_cold_console_reports_unknown_rather_than_zero() {
        // Showing 0.000.000 MHz before anything has been read is a lie.
        let state = ConsoleState::for_installation(&Installation::default(), 8);
        assert!(state.cat.is_cold());
        assert_eq!(state.cat.vfo_a_hz, None);
        assert_eq!(state.cat.mode, None);
    }

    #[test]
    fn a_bench_without_a_tap_still_gets_a_lane() {
        // So a layout can reserve the waterfall's space rather than
        // reflowing when a tap is plugged in. Absence is a design state.
        let state = ConsoleState::for_installation(&Installation::default(), 8);
        assert_eq!(state.spectrum.state, LaneState::Absent);
        assert!(state.spectrum.history.is_empty());
    }

    #[test]
    fn a_fitted_bench_reports_its_panorama_as_streaming() {
        let install = Installation::default().with_source(tap(SourceState::Streaming));
        let state = ConsoleState::for_installation(&install, 8);
        assert_eq!(state.spectrum.state, LaneState::Streaming);
        assert!(state.spectrum.state.has_data());
    }

    #[test]
    fn configured_is_neither_absent_nor_streaming() {
        // The state a bool could not hold, and the one this station's
        // audio path is actually in: wired, and silent because its
        // transport design does not exist.
        let install = Installation::default().with_source(audio_source(SourceState::Configured));
        let state = ConsoleState::for_installation(&install, 8);
        assert_eq!(state.audio.state, LaneState::Configured);
        assert!(state.audio.state.is_present(), "the hardware is there");
        assert!(!state.audio.state.has_data(), "nothing is arriving");
    }

    #[test]
    fn a_station_with_both_sources_gets_both_lanes() {
        // One enum field could not express this, and ConsoleState derived
        // exactly one lane from it -- so audio had nowhere to go at all.
        let install = Installation::default()
            .with_source(tap(SourceState::Streaming))
            .with_source(audio_source(SourceState::Configured));
        let state = ConsoleState::for_installation(&install, 8);
        assert_eq!(state.spectrum.state, LaneState::Streaming);
        assert_eq!(state.audio.state, LaneState::Configured);
    }

    #[test]
    fn an_audio_source_never_lands_in_the_panorama_lane() {
        // AudioDerived is a few kHz. Feeding it to a waterfall would
        // stretch it across a whole band and look authoritative doing it.
        let install = Installation::default().with_source(audio_source(SourceState::Streaming));
        let state = ConsoleState::for_installation(&install, 8);
        assert_eq!(state.spectrum.state, LaneState::Absent);
        assert_eq!(state.audio.state, LaneState::Streaming);
    }

    #[test]
    fn a_session_carries_the_model_and_the_bench_together() {
        let radio: &'static RadioCapabilities = Box::leak(Box::new(radio()));
        let session = Session::new(
            radio,
            Installation::default().with_source(tap(SourceState::Streaming)),
        );
        assert!(session.has_panorama());
        assert!(!session.panorama_possible_but_absent());

        let bare = Session::new(radio, Installation::default());
        assert!(!bare.has_panorama());
        // The radio COULD take one. That is an invitation, not an error.
        assert!(bare.panorama_possible_but_absent());
    }

    #[test]
    fn spectrum_frames_do_not_touch_the_cat_lane() {
        // The two-rate discipline, asserted: 1000 frames arriving must
        // leave slow-lane state exactly as it was.
        let mut state = ConsoleState::for_installation(&Installation::default(), 4);
        state.cat.vfo_a_hz = Some(14_074_000);
        state.cat.begin_command();

        for i in 1..=1000 {
            state.spectrum.push(frame(i));
        }

        assert_eq!(state.cat.vfo_a_hz, Some(14_074_000));
        assert_eq!(state.cat.pending, 1);
        assert_eq!(state.spectrum.history.len(), 4);
    }

    #[test]
    fn pending_commands_are_counted_for_a_renderer_to_show() {
        let mut lane = CatLane::default();
        assert!(!lane.is_busy());
        lane.begin_command();
        lane.begin_command();
        assert_eq!(lane.pending, 2);
        assert!(lane.is_busy());
        lane.end_command();
        lane.end_command();
        assert!(!lane.is_busy());
    }

    #[test]
    fn ending_more_commands_than_were_started_does_not_underflow() {
        // A duplicate response, or a response after a timeout already
        // resolved the command. An underflow here would make the console
        // permanently "busy".
        let mut lane = CatLane::default();
        lane.end_command();
        lane.end_command();
        assert_eq!(lane.pending, 0);
        assert!(!lane.is_busy());
    }

    #[test]
    fn a_meter_reading_replaces_rather_than_accumulates() {
        // Meters update continuously; appending would grow without bound
        // for the lifetime of the session.
        let mut lane = CatLane::default();
        for raw in 0..100 {
            lane.set_meter(MeterKind::S, raw);
        }
        assert_eq!(lane.meters.len(), 1);
        assert_eq!(lane.meter(MeterKind::S), Some(99));
        assert_eq!(lane.meter(MeterKind::Swr), None);
    }
}
