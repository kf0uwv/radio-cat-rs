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

//! A server to point a console at, for tests that have no radio.
//!
//! # Why this is here and not in each caller
//!
//! A console crate should be able to test what it does with a reply
//! without depending on a radio crate or on `cat-framework` to build a
//! capability set. `ts570d`'s GUI in particular is forbidden both (its
//! ADR 0008 §3): it is a protocol client and nothing else, and a test
//! double that made it link a radio would quietly undo the boundary the
//! crate exists to hold.
//!
//! So the double lives next to the protocol, where both halves already
//! depend on it.

use std::sync::{Arc, Mutex};

use cat_framework::capabilities::{
    EndpointDescriptor, EndpointRole, EndpointSet, FilterCapability, FrequencyRange,
    MeterDescriptor, MeterKind, MeterSet, ModeDescriptor, ModeId, ModeKind, RadioCapabilities,
    RawRange, SUnitScale, Sideband, SignalSupport, VfoCapability,
};
use cat_signal::DeviceList;

use crate::server::RadioHost;
use crate::{Command, RadioState};

const MODES: &[ModeDescriptor] = &[
    ModeDescriptor {
        id: ModeId::Lsb,
        label: "LSB",
        kind: ModeKind::Ssb,
        sideband: Some(Sideband::Lower),
        default_bandwidth_hz: 2400,
    },
    ModeDescriptor {
        id: ModeId::Usb,
        label: "USB",
        kind: ModeKind::Ssb,
        sideband: Some(Sideband::Upper),
        default_bandwidth_hz: 2400,
    },
];

const METERS: &[MeterDescriptor] = &[MeterDescriptor {
    kind: MeterKind::S,
    raw_range: RawRange::new(0, 30),
    active_on_transmit: false,
    s_units: Some(SUnitScale::TS570D),
}];

const ENDPOINTS: &[EndpointDescriptor] = &[EndpointDescriptor {
    role: EndpointRole::Cat,
    required: true,
    shareable_with: &[],
}];

/// A plausible radio, shaped like something with an IF tap so a console
/// under test grows the workspaces it would grow in the field.
pub static STUB_RADIO: RadioCapabilities = RadioCapabilities {
    model: "Stub Radio",
    endpoints: EndpointSet::new(ENDPOINTS),
    vfos: VfoCapability {
        count: 2,
        split: true,
        rit_hz: Some(9999),
        xit_hz: Some(9999),
    },
    modes: MODES,
    tuning_steps_hz: &[10, 100],
    rx_range: FrequencyRange {
        min_hz: 30_000,
        max_hz: 60_000_000,
    },
    filters: FilterCapability {
        if_shift_hz: Some(1200),
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
};

/// A radio that answers, remembers what it was asked, and can be told to
/// decline device selection.
pub struct StubHost {
    /// `None` declines the device question, the way a server with no
    /// directory does.
    devices: Mutex<Option<Vec<DeviceList>>>,
    /// Every command that reached `apply`, in order.
    applied: Mutex<Vec<Command>>,
}

impl StubHost {
    /// A host that declines device selection.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            devices: Mutex::new(None),
            applied: Mutex::new(Vec::new()),
        })
    }

    /// A host that offers exactly these lists.
    pub fn offering(devices: Vec<DeviceList>) -> Arc<Self> {
        Arc::new(Self {
            devices: Mutex::new(Some(devices)),
            applied: Mutex::new(Vec::new()),
        })
    }

    /// What has been applied so far.
    pub fn applied(&self) -> Vec<Command> {
        self.applied.lock().map(|a| a.clone()).unwrap_or_default()
    }

    /// Change what the host can see, as plugging something in would.
    pub fn set_devices(&self, devices: Option<Vec<DeviceList>>) {
        if let Ok(mut slot) = self.devices.lock() {
            *slot = devices;
        }
    }
}

impl RadioHost for StubHost {
    fn capabilities(&self) -> &'static RadioCapabilities {
        &STUB_RADIO
    }

    fn state(&self) -> RadioState {
        RadioState {
            vfo_a_hz: 14_074_000,
            vfo_b_hz: 7_074_000,
            mode: ModeId::Usb,
            split: false,
            transmitting: false,
            memory_channel: None,
            if_shift_hz: Some(0),
            filter_width_hz: None,
            meters: Vec::new(),
        }
    }

    fn devices(&self) -> Option<Vec<DeviceList>> {
        self.devices.lock().ok().and_then(|d| d.clone())
    }

    fn apply(&self, command: &Command) -> Result<(), String> {
        if let Ok(mut applied) = self.applied.lock() {
            applied.push(command.clone());
        }
        Ok(())
    }
}

/// Serve `host` on an ephemeral port, returning the address to connect to.
///
/// The listener thread runs until the process ends; tests do not stop it.
pub fn serve_stub(host: Arc<StubHost>) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let addr = listener.local_addr().expect("a bound address").to_string();
    std::thread::spawn(move || {
        let _ = crate::server::serve(listener, host);
    });
    addr
}

/// The stub radio's capabilities, as a client would receive them.
///
/// For a test that needs a `CapabilitiesWire` without standing up a
/// socket — anything that reads what the server *said* rather than what it
/// does.
pub fn stub_capabilities() -> crate::CapabilitiesWire {
    crate::CapabilitiesWire::from(&STUB_RADIO)
}
