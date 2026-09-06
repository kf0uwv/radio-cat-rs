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

//! What is plugged into this machine, in terms a console can offer.
//!
//! # Why this is a type and not a `Vec<String>`
//!
//! Sound-card names are per-host, differ between machines, and are not
//! guessable — nobody types `plughw:CARD=Codec,DEV=0` from memory. So a
//! console has to *offer* them, which means it needs more than the string:
//! something to show the operator, something to say which one the host
//! considers default, and somewhere for "the enumeration itself failed" to
//! go that is not an empty list.
//!
//! An empty list and a broken driver look identical otherwise, and they
//! call for opposite responses: one means "plug something in", the other
//! means "this build cannot see your hardware".
//!
//! # The spec is the command line
//!
//! [`DeviceInfo::spec`] is **exactly the string the endpoint argument
//! takes** — `rtl:0`, `hw:1,0`, `127.0.0.1:4002`. That is deliberate: a
//! picker that produced some internal handle would be a second way to name
//! a device, and an operator who picked one from a list could not then
//! write down what they picked. Picking is a shortcut for typing, not a
//! separate mechanism.
//!
//! # Where enumeration lives
//!
//! Not here. This crate is dependency-light on purpose, and enumerating
//! sound cards needs a sound library while enumerating SDRs needs a USB
//! one. Each source crate enumerates its own kind and returns these types;
//! `cat-signal` owns only the vocabulary, so a renderer can draw a list of
//! devices without linking a driver for any of them.

/// What kind of thing a device is, for grouping and for the label a picker
/// puts above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum DeviceKind {
    /// A sound-card input — the ACC2 receive-audio pair's other end.
    AudioInput,
    /// A software-defined radio — the IF output's other end.
    Sdr,
}

impl DeviceKind {
    /// The heading a picker shows for this group.
    pub fn heading(self) -> &'static str {
        match self {
            DeviceKind::AudioInput => "AUDIO INPUT",
            DeviceKind::Sdr => "SDR",
        }
    }

    /// The flag that takes one of these.
    ///
    /// Carried so a picker can tell the operator what they would otherwise
    /// have typed, rather than every renderer hard-coding the pairing.
    pub fn flag(self) -> &'static str {
        match self {
            DeviceKind::AudioInput => "--acc2-audio",
            DeviceKind::Sdr => "--if-out",
        }
    }
}

/// One device an operator could choose.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeviceInfo {
    pub kind: DeviceKind,
    /// **The endpoint string.** What the command line would take, and what
    /// picking this device is a shortcut for typing.
    pub spec: String,
    /// What the operator reads. The driver's own name for the thing.
    pub label: String,
    /// One line of detail — channel counts, sample rates, tuner type —
    /// or `None` when the driver offers nothing worth showing.
    pub detail: Option<String>,
    /// Whether the host considers this its default of that kind.
    pub is_default: bool,
}

/// The result of asking "what is plugged in?".
///
/// Three outcomes, not two. `devices` empty with `error: None` means the
/// enumeration worked and found nothing — plug something in. `error:
/// Some(_)` means the question could not be asked at all, which is a
/// different problem with a different fix, and a console that rendered both
/// as an empty list would send its operator looking in the wrong place.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeviceList {
    pub kind: DeviceKind,
    pub devices: Vec<DeviceInfo>,
    /// Why the enumeration could not be performed.
    pub error: Option<String>,
}

impl DeviceList {
    /// A successful enumeration, however many it found.
    pub fn found(kind: DeviceKind, devices: Vec<DeviceInfo>) -> Self {
        Self {
            kind,
            devices,
            error: None,
        }
    }

    /// The question could not be asked.
    pub fn unavailable(kind: DeviceKind, why: impl Into<String>) -> Self {
        Self {
            kind,
            devices: Vec::new(),
            error: Some(why.into()),
        }
    }

    /// Whether this build can see hardware of this kind at all.
    pub fn is_available(&self) -> bool {
        self.error.is_none()
    }

    /// The host's default, if it named one.
    pub fn default_device(&self) -> Option<&DeviceInfo> {
        self.devices.iter().find(|d| d.is_default)
    }
}

/// What a machine can see, and how to attach one of it.
///
/// The seam that lets a server answer "what signal hardware do you have?"
/// without the protocol layer learning what a sound card is. `cat-native`
/// carries the answer; `cat-signal-audio` and `cat-signal-rtlsdr` produce
/// it; neither needs the other.
///
/// # Why the same trait does both
///
/// A list whose entries cannot be picked is a catalogue, not a picker, and
/// the two halves have to agree about what a `spec` string means. Keeping
/// them in one trait makes that agreement structural rather than a
/// convention two crates have to remember separately.
pub trait DeviceDirectory: Send + Sync + 'static {
    /// Enumerate, now.
    ///
    /// Called per request rather than cached: a dongle plugged in after
    /// the server started should appear without restarting it, and an
    /// operator who plugs one in and sees nothing has no way to tell that
    /// from a broken dongle.
    fn list(&self) -> Vec<DeviceList>;

    /// Attach the device named by `spec`, which came from this directory's
    /// own [`DeviceInfo`].
    ///
    /// The error is shown to an operator verbatim, so it should be the
    /// system's own words: "device or resource busy" names the other
    /// program holding the dongle, where a tidied "attach failed" would
    /// send them to check their cabling instead.
    fn attach(&self, kind: DeviceKind, spec: &str) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(spec: &str, is_default: bool) -> DeviceInfo {
        DeviceInfo {
            kind: DeviceKind::AudioInput,
            spec: spec.to_string(),
            label: spec.to_string(),
            detail: None,
            is_default,
        }
    }

    #[test]
    fn nothing_found_and_cannot_look_are_different_answers() {
        // The distinction this type exists for. One means "plug something
        // in"; the other means "this build cannot see your hardware", and
        // they send an operator to different places.
        let empty = DeviceList::found(DeviceKind::AudioInput, Vec::new());
        let broken = DeviceList::unavailable(DeviceKind::AudioInput, "no backend compiled in");

        assert!(empty.devices.is_empty() && broken.devices.is_empty());
        assert!(empty.is_available());
        assert!(!broken.is_available());
    }

    #[test]
    fn the_default_is_findable_and_optional() {
        let list = DeviceList::found(
            DeviceKind::AudioInput,
            vec![info("hw:0,0", false), info("hw:1,0", true)],
        );
        assert_eq!(
            list.default_device().map(|d| d.spec.as_str()),
            Some("hw:1,0")
        );

        let none = DeviceList::found(DeviceKind::AudioInput, vec![info("hw:0,0", false)]);
        assert_eq!(none.default_device(), None);
    }

    #[test]
    fn each_kind_names_the_flag_that_takes_it() {
        // So a picker can say what the operator would otherwise have typed
        // without every renderer hard-coding the pairing.
        assert_eq!(DeviceKind::AudioInput.flag(), "--acc2-audio");
        assert_eq!(DeviceKind::Sdr.flag(), "--if-out");
    }
}
