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
use cat_framework::capabilities::{MeterKind, RadioCapabilities};
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

/// Fast lane: spectrum history and nothing that can block.
#[derive(Debug, Clone)]
pub struct SpectrumLane {
    pub history: SpectrumHistory,
    /// `false` when the radio reports no spectrum source. A first-class
    /// state, not an error: a TS-570D with nothing on CN4 is a working
    /// radio, and the console should stay recognisably itself.
    pub available: bool,
}

impl SpectrumLane {
    pub fn new(capacity: usize, available: bool) -> Self {
        Self {
            history: SpectrumHistory::new(capacity),
            available,
        }
    }

    pub fn push(&mut self, frame: SpectrumFrame) {
        self.history.push(frame);
    }
}

/// Both lanes, with no path from one to the other.
#[derive(Debug, Clone)]
pub struct ConsoleState {
    pub cat: CatLane,
    pub spectrum: SpectrumLane,
}

impl ConsoleState {
    /// Build state sized and configured for a particular radio.
    ///
    /// A radio with no spectrum source gets a lane marked unavailable
    /// rather than no lane at all, so a layout can reserve its space
    /// instead of reflowing when a tap is plugged in.
    pub fn for_radio(capabilities: &RadioCapabilities, history_capacity: usize) -> Self {
        Self {
            cat: CatLane::default(),
            spectrum: SpectrumLane::new(history_capacity, capabilities.signal.is_band_panorama()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_framework::capabilities::*;

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

    fn radio(signal: cat_signal::SignalCapability) -> RadioCapabilities {
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
            signal,
        }
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
        let state = ConsoleState::for_radio(&radio(cat_signal::SignalCapability::None), 8);
        assert!(state.cat.is_cold());
        assert_eq!(state.cat.vfo_a_hz, None);
        assert_eq!(state.cat.mode, None);
    }

    #[test]
    fn a_radio_without_a_tap_still_gets_a_lane_marked_unavailable() {
        // So a layout can reserve the waterfall's space rather than
        // reflowing when a tap is connected. ADR 0008 calls capability
        // absence a first-class design state.
        let state = ConsoleState::for_radio(&radio(cat_signal::SignalCapability::None), 8);
        assert!(!state.spectrum.available);
        assert!(state.spectrum.history.is_empty());
    }

    #[test]
    fn a_tapped_radio_reports_its_spectrum_lane_as_available() {
        let tapped = radio(cat_signal::SignalCapability::IfTap(
            cat_signal::IfTapConfig {
                if_center_hz: 73_050_000,
                inverted: true,
                trim_hz: 0,
            },
        ));
        let state = ConsoleState::for_radio(&tapped, 8);
        assert!(state.spectrum.available);
    }

    #[test]
    fn an_audio_only_source_is_not_offered_as_a_waterfall() {
        // AudioDerived is present but cannot drive a band panorama. A lane
        // that said otherwise would produce a waterfall of AF bandwidth
        // stretched across a band -- confidently wrong.
        let audio = radio(cat_signal::SignalCapability::AudioDerived {
            max_bandwidth_hz: 4_000,
        });
        assert!(!ConsoleState::for_radio(&audio, 8).spectrum.available);
    }

    #[test]
    fn spectrum_frames_do_not_touch_the_cat_lane() {
        // The two-rate discipline, asserted: 1000 frames arriving must
        // leave slow-lane state exactly as it was.
        let mut state = ConsoleState::for_radio(&radio(cat_signal::SignalCapability::None), 4);
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
