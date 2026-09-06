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

//! What `--acc2-audio` accepts, and what sound cards this machine has.
//!
//! **This module is compiled in every build.** Only the body of
//! [`input_devices`] is behind the `device` feature, because a console's
//! device picker is compiled once and has to be able to say *why* a list
//! is empty. `cat-signal-rtlsdr` gates its whole `device` module, which is
//! why `cat_signal_rtlsdr::device::devices` does not exist at all without
//! the feature; that is the one place this crate deliberately departs from
//! that precedent.
//!
//! # The grammar, and why it needs a scheme
//!
//! `--acc2-audio` already took a network endpoint. It now takes two kinds
//! of thing, and `hw:1,0`, `plughw:0`, `default` and `127.0.0.1:4533`
//! cannot be told apart by inspection without a heuristic that would have
//! to do DNS at parse time to be sure. So the grammar is explicit:
//!
//! | Spec | Means |
//! |------|-------|
//! | `audio:<name>` | the local input device the host calls `<name>` |
//! | `audio:` | whatever the host considers its default input |
//! | anything else | a network endpoint, exactly as before |
//!
//! `cat-signal-rtlsdr` names an SDR `rtl:<index>` for the same reason, so a
//! console parses all three with one rule.
//!
//! [`AudioEndpoint::parse`] lives here rather than in each console so that
//! there is one grammar rather than one per application, and it is
//! available **without** the `device` feature: being able to read a spec
//! is not the same as being able to open one, and a build that cannot
//! capture should still be able to say "that is a sound card, and this
//! build has no sound-card support" rather than trying to resolve it as a
//! hostname.
//!
//! # The name in a spec is the driver's name, not an ALSA id
//!
//! `cpal` identifies a device by the string `DeviceTrait::name` returns and
//! offers no other handle: its ALSA backend keeps the openable PCM id
//! (`plughw:0`) private and reports the card's name ("HDA Intel"). So the
//! spec carries that name and opening means enumerating and matching it.
//! The same code is therefore correct on WASAPI and CoreAudio, where
//! ALSA-style ids do not exist at all.

use cat_signal::DeviceList;

/// The scheme that marks a spec as a local sound card.
///
/// Exported so a console's argument parser and its help text cannot
/// disagree with this crate about the spelling.
pub const DEVICE_SPEC_PREFIX: &str = "audio:";

/// What a `--acc2-audio` spec turned out to name.
///
/// Total: every string is one or the other, and an unresolvable network
/// endpoint fails where it is opened rather than where it is parsed, which
/// keeps parsing free of DNS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioEndpoint<'a> {
    /// A local sound-card input. The name is the driver's own; empty means
    /// "whatever the host considers its default input".
    Device(&'a str),
    /// A radio's audio endpoint over the network, as ADR 0017 describes.
    Network(&'a str),
}

impl<'a> AudioEndpoint<'a> {
    /// Read a `--acc2-audio` spec.
    ///
    /// Never fails: a spec is a sound card if and only if it starts with
    /// [`DEVICE_SPEC_PREFIX`], and everything else is a network endpoint.
    /// Surrounding whitespace is trimmed because a name pasted out of a
    /// picker tends to bring some with it.
    pub fn parse(spec: &'a str) -> Self {
        let spec = spec.trim();
        match spec.strip_prefix(DEVICE_SPEC_PREFIX) {
            Some(name) => AudioEndpoint::Device(name.trim()),
            None => AudioEndpoint::Network(spec),
        }
    }

    /// Whether this names a local sound card.
    pub fn is_device(&self) -> bool {
        matches!(self, AudioEndpoint::Device(_))
    }
}

/// The spec for a device the driver calls `name`.
///
/// The inverse of [`AudioEndpoint::parse`], and the only thing that should
/// ever build one of these strings.
pub fn device_spec(name: &str) -> String {
    format!("{DEVICE_SPEC_PREFIX}{name}")
}

/// The sound-card inputs this machine has.
///
/// Returns [`cat_signal::DeviceList`] so a console can render sound cards
/// and SDRs through one picker without linking a driver for either — see
/// that type's module doc.
///
/// Three outcomes, and they are kept apart deliberately:
///
/// - a list, possibly empty, when the host answered — an empty one means
///   "nothing is plugged in";
/// - [`DeviceList::unavailable`] when the host *could not be asked*,
///   because the sound service is not running or the driver failed;
/// - [`DeviceList::unavailable`] naming the feature to rebuild with, when
///   this build has no sound-card support compiled in at all.
///
/// Every [`spec`](cat_signal::DeviceInfo::spec) is exactly what
/// `--acc2-audio` takes, so picking from this list is a shortcut for
/// typing and never a second naming mechanism.
///
/// # Duplicate names
///
/// Two devices the driver gives the same name are listed **once**. A spec
/// names a device by that string and nothing else, so listing both would
/// offer an operator a choice that cannot be honoured: picking the second
/// would open the first. One entry is the honest count of how many
/// distinct things this machine can be asked for.
#[cfg(feature = "device")]
pub fn input_devices() -> DeviceList {
    crate::capture::enumerate()
}

/// The sound-card inputs this machine has.
///
/// This build has no sound-card support compiled in, so the question
/// cannot be asked and the answer says what to rebuild with. It is
/// deliberately **not** an empty list: an empty list means "nothing is
/// plugged in", which would send an operator to look at their cabling
/// instead of at their build.
#[cfg(not(feature = "device"))]
pub fn input_devices() -> DeviceList {
    DeviceList::unavailable(
        cat_signal::DeviceKind::AudioInput,
        "this build cannot see sound cards: rebuild cat-signal-audio with \
         --features device (needs the platform's sound headers, ALSA on Linux)",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scheme_tells_a_sound_card_from_a_network_endpoint() {
        // The whole reason the prefix exists: both kinds contain colons,
        // and one flag takes both.
        assert_eq!(
            AudioEndpoint::parse("audio:HDA Intel"),
            AudioEndpoint::Device("HDA Intel")
        );
        assert_eq!(
            AudioEndpoint::parse("127.0.0.1:4533"),
            AudioEndpoint::Network("127.0.0.1:4533")
        );
        assert_eq!(
            AudioEndpoint::parse("radio.local:4533"),
            AudioEndpoint::Network("radio.local:4533")
        );
        // A bare ALSA-looking string is a network endpoint, because
        // without the scheme that is what the flag has always meant.
        assert_eq!(
            AudioEndpoint::parse("hw:1,0"),
            AudioEndpoint::Network("hw:1,0")
        );
    }

    #[test]
    fn an_empty_device_name_means_the_host_default() {
        assert_eq!(AudioEndpoint::parse("audio:"), AudioEndpoint::Device(""));
        assert_eq!(AudioEndpoint::parse("audio:  "), AudioEndpoint::Device(""));
    }

    #[test]
    fn a_spec_round_trips_through_the_parser() {
        // Picking is a shortcut for typing, so what a picker writes down
        // must be what the parser reads back.
        for name in ["default", "HDA Intel", "USB Audio CODEC", "hw:1,0"] {
            assert_eq!(
                AudioEndpoint::parse(&device_spec(name)),
                AudioEndpoint::Device(name)
            );
        }
    }

    #[test]
    fn pasted_whitespace_does_not_change_what_a_spec_means() {
        assert_eq!(
            AudioEndpoint::parse("  audio: HDA Intel  "),
            AudioEndpoint::Device("HDA Intel")
        );
    }

    #[cfg(not(feature = "device"))]
    #[test]
    fn without_the_feature_the_answer_is_cannot_ask_and_names_the_rebuild() {
        use cat_signal::DeviceKind;
        // Not an empty list. An empty list means "plug something in" and
        // would send an operator to look at their cabling rather than at
        // their build.
        let list = input_devices();
        assert_eq!(list.kind, DeviceKind::AudioInput);
        assert!(!list.is_available());
        assert!(list.devices.is_empty());
        let why = list.error.expect("an unavailable list carries a reason");
        assert!(
            why.contains("--features device"),
            "the reason must name what to rebuild with: {why}"
        );
    }
}
