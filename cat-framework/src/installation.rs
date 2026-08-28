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
//! RTL-SDR on a 73.05 MHz IF tap **and** a USB sound device fed from ACC2:
//! a band panorama and an audio-derived source, live together. A single
//! enum could not say so, and the console design hit exactly that wall.

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

/// One signal source present on this station.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstalledSource {
    /// The resolved capability, as a consumer sees it.
    pub capability: SignalCapability,
    pub state: SourceState,
    /// Human-readable, for a settings panel: "RTL-SDR #0 on CN4".
    pub label: String,
}

impl InstalledSource {
    pub fn new(capability: SignalCapability, state: SourceState, label: impl Into<String>) -> Self {
        Self {
            capability,
            state,
            label: label.into(),
        }
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

    /// The first audio-derived source, if any.
    pub fn audio(&self) -> Option<&InstalledSource> {
        self.sources
            .iter()
            .find(|s| matches!(s.capability, SignalCapability::AudioDerived { .. }))
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
