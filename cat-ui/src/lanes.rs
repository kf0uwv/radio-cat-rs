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

/// Which control a command targets.
///
/// The reason this exists: `pending` used to be a bare `usize`, so a
/// renderer could show a global "CAT busy" chip and nothing else. No
/// control could show *its own* pending state, and every renderer worked
/// around it with local intent memory that no shared type held — which is
/// two renderers inventing the same thing separately, the drift ADR 0013
/// forbids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Control {
    VfoFrequency(u8),
    Mode,
    Split,
    FilterWidth,
    IfShift,
    Notch,
    MemoryChannel,
    Meter(MeterKind),
}

/// How a command ended.
///
/// Three outcomes, not two, and `Clamped` is the one that matters most.
/// A radio frequently accepts something *near* what was asked — snapping to
/// its own tuning grid, or pinning to a band edge — and reports success. A
/// console that treated that as a plain confirmation would leave its cursor
/// somewhere the operator did not click and say nothing, which is worse
/// than being slow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    Confirmed,
    Rejected,
    /// Accepted, but not at the requested value.
    Clamped,
}

/// A command sent and not yet answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pending {
    pub token: u64,
    pub control: Control,
}

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
    /// Selected IF filter width.
    ///
    /// This and the three below close a gap the console design found:
    /// `FilterCapability`, `MemoryCapability` and `MenuCapability` were all
    /// advertised in the handshake with no field to read their current
    /// values back into. A control like that can be *sent* and never
    /// *confirmed* — and combined with a bare pending count, it could never
    /// leave the pending state at all. Three of them are quick-settings
    /// controls, which makes them among the most-touched in the console.
    pub filter_width_hz: Option<u32>,
    pub if_shift_hz: Option<i32>,
    pub notch: Option<bool>,
    pub memory_channel: Option<u16>,
    /// Raw meter readings, paired with kind. Scaling happens at render time
    /// against the radio's own `MeterSet`.
    pub meters: Vec<(MeterKind, u16)>,
    /// Recent readings per meter, oldest first, for a trend trace.
    meter_history: Vec<(MeterKind, Vec<u16>)>,
    history_len: usize,
    in_flight: Vec<Pending>,
    next_token: u64,
    /// The most recent outcome per control, so a renderer can show *why* a
    /// control stopped being pending rather than only that it did.
    last_outcome: Vec<(Control, Outcome)>,
}

impl CatLane {
    /// A lane that keeps `history_len` readings per meter.
    pub fn with_history(history_len: usize) -> Self {
        Self {
            history_len,
            ..Self::default()
        }
    }

    /// Note a command sent, returning a token to resolve it with.
    ///
    /// The token is what makes an outcome attributable. Without it a
    /// response can only decrement a counter, and a renderer cannot tell
    /// which control just settled.
    pub fn begin(&mut self, control: Control) -> u64 {
        self.next_token += 1;
        self.in_flight.push(Pending {
            token: self.next_token,
            control,
        });
        self.next_token
    }

    /// Resolve a command. Returns the control it targeted, if the token was
    /// one we issued.
    ///
    /// An unknown token is ignored rather than decrementing something: a
    /// duplicate response, or one arriving after a timeout already resolved
    /// the command, must not make some *other* control look settled.
    pub fn resolve(&mut self, token: u64, outcome: Outcome) -> Option<Control> {
        let idx = self.in_flight.iter().position(|p| p.token == token)?;
        let control = self.in_flight.remove(idx).control;
        match self.last_outcome.iter_mut().find(|(c, _)| *c == control) {
            Some(slot) => slot.1 = outcome,
            None => self.last_outcome.push((control, outcome)),
        }
        Some(control)
    }

    /// Whether this specific control is waiting on the radio.
    pub fn is_pending(&self, control: Control) -> bool {
        self.in_flight.iter().any(|p| p.control == control)
    }

    /// How this control's last command ended.
    pub fn outcome(&self, control: Control) -> Option<Outcome> {
        self.last_outcome
            .iter()
            .find(|(c, _)| *c == control)
            .map(|(_, o)| *o)
    }

    /// Everything in flight, for a global busy indicator.
    pub fn in_flight(&self) -> &[Pending] {
        &self.in_flight
    }

    pub fn pending_count(&self) -> usize {
        self.in_flight.len()
    }

    pub fn is_busy(&self) -> bool {
        !self.in_flight.is_empty()
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
    ///
    /// Also appends to a bounded history. That history is what lets a
    /// console show signal over time when there is no panorama to show —
    /// the only signal history CAT can give — and it has to live here
    /// rather than in a renderer, or the two renderers cannot hold parity
    /// on it.
    pub fn set_meter(&mut self, kind: MeterKind, raw: u16) {
        match self.meters.iter_mut().find(|(k, _)| *k == kind) {
            Some(slot) => slot.1 = raw,
            None => self.meters.push((kind, raw)),
        }
        if self.history_len == 0 {
            return;
        }
        let entry = match self.meter_history.iter_mut().find(|(k, _)| *k == kind) {
            Some(e) => e,
            None => {
                self.meter_history.push((kind, Vec::new()));
                self.meter_history.last_mut().expect("just pushed")
            }
        };
        entry.1.push(raw);
        if entry.1.len() > self.history_len {
            entry.1.remove(0);
        }
    }

    /// Recent readings for `kind`, oldest first.
    pub fn meter_history(&self, kind: MeterKind) -> &[u16] {
        self.meter_history
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&[])
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
            // The radio's own audio if there is one, otherwise whatever
            // audio there is. A station can have several -- an ACC2 codec
            // and audio demodulated from the IF tap -- and they are not
            // interchangeable, so a console with one AF panel defaults to
            // the one that matches the speaker and lets the operator
            // change it. Which source is selected is layout's business.
            audio: SpectrumLane::new(
                history_capacity,
                LaneState::from(
                    installation
                        .radio_audio()
                        .or_else(|| installation.audio_sources().next())
                        .map(|s| s.state),
                ),
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
        state.cat.begin(Control::Split);

        for i in 1..=1000 {
            state.spectrum.push(frame(i));
        }

        assert_eq!(state.cat.vfo_a_hz, Some(14_074_000));
        assert_eq!(state.cat.pending_count(), 1);
        assert_eq!(state.spectrum.history.len(), 4);
    }

    #[test]
    fn a_control_knows_its_own_pending_state() {
        // The gap a bare count left. A global "CAT busy" chip cannot tell
        // an operator which control they are waiting on.
        let mut lane = CatLane::default();
        let token = lane.begin(Control::Split);
        assert!(lane.is_pending(Control::Split));
        assert!(!lane.is_pending(Control::Mode));
        assert_eq!(lane.pending_count(), 1);

        lane.resolve(token, Outcome::Confirmed);
        assert!(!lane.is_pending(Control::Split));
        assert!(!lane.is_busy());
    }

    #[test]
    fn a_clamped_command_is_distinguishable_from_a_confirmed_one() {
        // The sharpest form of the problem. A radio often accepts something
        // NEAR what was asked -- snapping to its tuning grid, or pinning to
        // a band edge -- and reports success. A cursor that silently lands
        // somewhere the operator did not click is worse than a slow one.
        let mut lane = CatLane::default();
        let a = lane.begin(Control::VfoFrequency(0));
        lane.resolve(a, Outcome::Clamped);
        assert_eq!(
            lane.outcome(Control::VfoFrequency(0)),
            Some(Outcome::Clamped)
        );

        let b = lane.begin(Control::VfoFrequency(0));
        lane.resolve(b, Outcome::Confirmed);
        assert_eq!(
            lane.outcome(Control::VfoFrequency(0)),
            Some(Outcome::Confirmed)
        );
    }

    #[test]
    fn a_rejected_command_is_not_silence() {
        let mut lane = CatLane::default();
        let t = lane.begin(Control::Notch);
        lane.resolve(t, Outcome::Rejected);
        assert_eq!(lane.outcome(Control::Notch), Some(Outcome::Rejected));
        assert!(!lane.is_pending(Control::Notch));
    }

    #[test]
    fn an_unknown_token_settles_nothing() {
        // A duplicate response, or one arriving after a timeout already
        // resolved the command. Decrementing a counter here would make some
        // OTHER control look settled -- which is exactly what a bare count
        // did.
        let mut lane = CatLane::default();
        lane.begin(Control::Mode);
        assert_eq!(lane.resolve(9_999, Outcome::Confirmed), None);
        assert!(
            lane.is_pending(Control::Mode),
            "an unrelated control settled"
        );
        assert_eq!(lane.pending_count(), 1);
    }

    #[test]
    fn two_controls_can_be_in_flight_independently() {
        let mut lane = CatLane::default();
        let freq = lane.begin(Control::VfoFrequency(0));
        lane.begin(Control::Mode);
        assert_eq!(lane.pending_count(), 2);

        lane.resolve(freq, Outcome::Confirmed);
        assert!(!lane.is_pending(Control::VfoFrequency(0)));
        assert!(lane.is_pending(Control::Mode), "mode is still waiting");
    }

    #[test]
    fn the_ribbon_controls_have_somewhere_to_read_back_into() {
        // FilterCapability, MemoryCapability and MenuCapability were all
        // advertised at handshake with no lane field for their values, so
        // those controls could be sent and never confirmed.
        let mut lane = CatLane::default();
        assert_eq!(lane.filter_width_hz, None);
        assert_eq!(lane.if_shift_hz, None);
        assert_eq!(lane.notch, None);
        assert_eq!(lane.memory_channel, None);

        lane.filter_width_hz = Some(1_800);
        lane.if_shift_hz = Some(-150);
        lane.notch = Some(false);
        lane.memory_channel = Some(7);
        // "off" and "not read yet" stay distinguishable.
        assert_eq!(lane.notch, Some(false));
    }

    #[test]
    fn meter_history_is_bounded_and_oldest_first() {
        // The only signal history CAT can give. It has to live in cat-ui,
        // or the two renderers cannot hold parity on it.
        let mut lane = CatLane::with_history(4);
        for raw in 0..10 {
            lane.set_meter(MeterKind::S, raw);
        }
        assert_eq!(lane.meter_history(MeterKind::S), &[6, 7, 8, 9]);
        assert_eq!(lane.meter(MeterKind::S), Some(9));
    }

    #[test]
    fn a_lane_with_no_history_keeps_only_the_current_reading() {
        let mut lane = CatLane::default();
        lane.set_meter(MeterKind::S, 5);
        assert!(lane.meter_history(MeterKind::S).is_empty());
        assert_eq!(lane.meter(MeterKind::S), Some(5));
    }

    #[test]
    fn history_is_kept_per_meter() {
        let mut lane = CatLane::with_history(3);
        lane.set_meter(MeterKind::S, 1);
        lane.set_meter(MeterKind::Swr, 9);
        lane.set_meter(MeterKind::S, 2);
        assert_eq!(lane.meter_history(MeterKind::S), &[1, 2]);
        assert_eq!(lane.meter_history(MeterKind::Swr), &[9]);
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

#[cfg(test)]
mod audio_source_tests {
    use super::*;
    use cat_framework::installation::{AudioOrigin, Installation, InstalledSource, SourceState};

    fn audio(origin: AudioOrigin, state: SourceState, bw: u32) -> InstalledSource {
        InstalledSource::new(
            cat_signal::SignalCapability::AudioDerived {
                max_bandwidth_hz: bw,
            },
            state,
            match origin {
                AudioOrigin::RadioOutput => "ACC2 codec",
                _ => "SDR demodulated",
            },
        )
        .from_origin(origin)
    }

    #[test]
    fn the_audio_lane_defaults_to_the_radios_own_output() {
        // With two paths available, a console with one AF panel should show
        // what the operator is hearing, not the tap-demodulated one -- and
        // it is the only one whose FFT may carry the radio's filter
        // passband as an overlay.
        let install = Installation::default()
            .with_source(audio(
                AudioOrigin::TapDemodulated,
                SourceState::Streaming,
                12_000,
            ))
            .with_source(audio(
                AudioOrigin::RadioOutput,
                SourceState::Configured,
                3_000,
            ));
        let state = ConsoleState::for_installation(&install, 8);
        // The ACC2 path is Configured, the tapped one Streaming. Picking
        // the first source in the list would have reported Streaming.
        assert_eq!(state.audio.state, LaneState::Configured);
    }

    #[test]
    fn a_station_with_only_tapped_audio_still_gets_a_lane() {
        // Falling back matters: a bench with no ACC2 wiring but an SDR on
        // the tap has audio, and a console that showed none would be wrong.
        let install = Installation::default().with_source(audio(
            AudioOrigin::TapDemodulated,
            SourceState::Streaming,
            12_000,
        ));
        let state = ConsoleState::for_installation(&install, 8);
        assert_eq!(state.audio.state, LaneState::Streaming);
    }

    #[test]
    fn no_audio_at_all_is_still_a_lane() {
        let state = ConsoleState::for_installation(&Installation::default(), 8);
        assert_eq!(state.audio.state, LaneState::Absent);
    }
}
