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

//! [`Installation`]: what this deployment actually has wired.
//!
//! See `docs/adr/0015-model-facts-versus-installation-facts.md`.
//!
//! # Why this is a separate type
//!
//! [`crate::capabilities::RadioCapabilities`] describes a radio *model* and
//! is a `const`. This describes one *bench* and cannot be: whether a dongle
//! is on the IF tap, what its crystal error measured out at, whether a
//! soundcard is wired to ACC2 — none of that is a property of a TS-570D,
//! and two of the same model can disagree about all of it.
//!
//! The rule, for arguments at the boundary: **if two units of the same
//! model can disagree about it, it is installation data.** A fitted filter
//! is therefore installation data, and so is anything else optional.
//!
//! # A station has several sources, not one
//!
//! [`Installation::sources`] is a list, not a field, because a real station
//! runs more than one at once. The station this was written for has an
//! RTL-SDR on a 73.05 MHz IF tap **and** a USB sound device fed from ACC2.
//!
//! And a source is not a device: that one RTL-SDR provides **two** sources
//! by itself — a band panorama from the IQ, and audio demodulated out of
//! the same IQ — so the station runs three in total, two of them audio.
//! Which is why [`Installation::audio_sources`] is plural and why
//! [`AudioOrigin`] exists: the two audio paths are not interchangeable, and
//! a renderer that treated them as such would draw the radio's filter
//! passband over audio that never passed through it.

use crate::capabilities::{EndpointRole, RadioCapabilities, SignalSupport};
use cat_signal::{IfTapConfig, SignalCapability};
use serde::{Deserialize, Serialize};

/// Whether a source is merely present or actually delivering.
///
/// Three states, not two, and the middle one is not a nicety. An audio path
/// can be fully wired and configured while nothing streams from it —
/// exactly the state of a station whose transport design has not been
/// written yet. A `bool` collapses that into "absent", and a console then
/// tells its operator there is no audio hardware when there is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceState {
    /// Wired and configured, but no data is arriving.
    Configured,
    /// Delivering frames now.
    Streaming,
}

/// Where an audio source's signal comes from.
///
/// Not decoration, and not a label: it decides what a renderer may
/// honestly draw on top of the audio.
///
/// An AF FFT of the radio's own output can carry the radio's IF filter
/// passband as an overlay, because that audio genuinely passed through it.
/// The same overlay on audio demodulated from an IF tap would be a
/// fabrication — that signal never went near the radio's filter, AGC or
/// DSP. A console with one audio panel and two possible sources must know
/// which it is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AudioOrigin {
    /// The radio's own receive chain — post-IF-filter, post-AGC, post-DSP.
    /// What the operator is actually hearing.
    RadioOutput,
    /// Demodulated from an IF tap, ahead of everything the radio does to
    /// its audio.
    ///
    /// It follows the dial exactly as the radio's own audio does — the tap
    /// is dial-centred, so demodulating its centre gives the frequency the
    /// operator is tuned to. It is therefore a second *rendering of the
    /// same signal*, not a second receiver, and a console needs only a
    /// selector rather than an independent tuning control.
    ///
    /// What differs is character, and that is the reason to offer it: no
    /// IF filter, so an operator can hear what sits just outside the
    /// passband; and no AGC, so a strong neighbouring signal does not pump
    /// the one being listened to.
    TapDemodulated,
}

/// One signal source present on this station.
///
/// A source is not a device. One RTL-SDR on an IF tap provides **two** of
/// these: a band panorama from the IQ, and audio demodulated out of the
/// same IQ. They have different capabilities, can be in different states,
/// and a console treats them separately.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstalledSource {
    /// The resolved capability, as a consumer sees it.
    pub capability: SignalCapability,
    pub state: SourceState,
    /// Human-readable, for a settings panel: "RTL-SDR #0 on CN4".
    pub label: String,
    /// Set for audio sources; `None` for a panorama.
    pub audio_origin: Option<AudioOrigin>,
}

impl InstalledSource {
    pub fn new(capability: SignalCapability, state: SourceState, label: impl Into<String>) -> Self {
        Self {
            capability,
            state,
            label: label.into(),
            audio_origin: None,
        }
    }

    /// Note where this audio came from.
    pub fn from_origin(mut self, origin: AudioOrigin) -> Self {
        self.audio_origin = Some(origin);
        self
    }

    /// Whether the radio's own filter passband is a truthful overlay on
    /// this source.
    ///
    /// The question a console's AF FFT has to ask before drawing one.
    pub fn reflects_radio_dsp(&self) -> bool {
        self.audio_origin == Some(AudioOrigin::RadioOutput)
    }

    /// Whether this source can legitimately drive a band panorama.
    pub fn is_band_panorama(&self) -> bool {
        self.capability.is_band_panorama()
    }

    pub fn is_streaming(&self) -> bool {
        self.state == SourceState::Streaming
    }
}

/// What one deployment has connected.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Installation {
    /// Endpoint roles actually wired, as against the topology the model
    /// merely supports.
    pub connected: Vec<EndpointRole>,
    /// Every live source. Empty is a normal state, not an error: a bare
    /// TS-570D with nothing on CN4 is a working radio.
    pub sources: Vec<InstalledSource>,
}

impl Installation {
    /// A station with nothing optional attached.
    pub fn bare(connected: Vec<EndpointRole>) -> Self {
        Self {
            connected,
            sources: Vec::new(),
        }
    }

    pub fn with_source(mut self, source: InstalledSource) -> Self {
        self.sources.push(source);
        self
    }

    pub fn is_connected(&self, role: EndpointRole) -> bool {
        self.connected.contains(&role)
    }

    /// The first source that can drive a band panorama, if any.
    ///
    /// This is the question a waterfall asks. It deliberately skips
    /// `AudioDerived`, which is present and useful and *not* a panorama.
    pub fn band_panorama(&self) -> Option<&InstalledSource> {
        self.sources.iter().find(|s| s.is_band_panorama())
    }

    /// Every audio-derived source.
    ///
    /// Plural, because a station can genuinely have more than one and this
    /// one does: a USB codec fed from ACC2, and audio demodulated from the
    /// RTL-SDR already sitting on the IF tap. Returning only the first
    /// would silently pick one for the operator, and the two do not sound
    /// or behave alike — one is post-DSP and one is not.
    pub fn audio_sources(&self) -> impl Iterator<Item = &InstalledSource> {
        self.sources
            .iter()
            .filter(|s| matches!(s.capability, SignalCapability::AudioDerived { .. }))
    }

    /// The audio source that reflects what the operator is hearing, if any.
    ///
    /// A console with a single AF panel and no explicit selection should
    /// default to this: it is the one whose FFT can carry the radio's
    /// filter passband, and the one that matches the speaker.
    pub fn radio_audio(&self) -> Option<&InstalledSource> {
        self.audio_sources().find(|s| s.reflects_radio_dsp())
    }

    /// Whether the operator has a choice to make.
    pub fn has_multiple_audio_sources(&self) -> bool {
        self.audio_sources().count() > 1
    }

    /// Build an `IfTap` source from what the *model* provides and what this
    /// station measured.
    ///
    /// This is the seam in one function. `if_center_hz` and `inverted` come
    /// from the radio's circuitry; `trim_hz` is one dongle's crystal error,
    /// measured once against a known carrier. Neither half is complete
    /// alone, and keeping the measurement out of the `const` is the whole
    /// point of ADR 0015.
    pub fn if_tap_from(
        radio: &RadioCapabilities,
        trim_hz: i32,
        state: SourceState,
        label: impl Into<String>,
    ) -> Option<InstalledSource> {
        let SignalSupport::IfTapPoint {
            if_center_hz,
            inverted,
        } = radio.signal
        else {
            return None;
        };
        Some(InstalledSource::new(
            SignalCapability::IfTap(IfTapConfig {
                if_center_hz,
                inverted,
                trim_hz,
            }),
            state,
            label,
        ))
    }
}

/// A radio model and one station's wiring, resolved.
///
/// What a client is actually told at handshake. A console does not want to
/// know that a TS-570D *could* take an IF tap; it wants to know whether to
/// draw a waterfall.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub radio: &'static RadioCapabilities,
    pub installation: Installation,
}

impl Session {
    pub fn new(radio: &'static RadioCapabilities, installation: Installation) -> Self {
        Self {
            radio,
            installation,
        }
    }

    /// Whether a waterfall should be drawn at all.
    pub fn has_panorama(&self) -> bool {
        self.installation.band_panorama().is_some()
    }

    /// Whether the radio could take a spectrum source that is not attached.
    ///
    /// The state that deserves an invitation rather than an apology: the
    /// console shows where the tap would go and how to configure it, which
    /// is what the chosen design does with its SPECTRUM tab.
    pub fn panorama_possible_but_absent(&self) -> bool {
        self.radio.signal.is_possible() && !self.has_panorama()
    }
}
