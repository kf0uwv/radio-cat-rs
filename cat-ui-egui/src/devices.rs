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

//! Choosing a source, on a machine this console is not sitting at.
//!
//! # The bug this exists to prevent
//!
//! A GUI is a network client (ADR 0008 §3). Enumerating *its own* sound
//! cards and offering them as the radio's would let an operator on a
//! laptop select their laptop microphone as the radio's receive audio.
//! That looks entirely correct on screen and is wrong, and nothing later
//! in the signal chain can detect it. So this console never looks at its
//! own hardware: it asks the server, and shows whatever the server says.
//!
//! # Four states, not two
//!
//! "Haven't asked", "the server declines the question", "the server
//! looked and found nothing" and "here they are" are four different
//! situations with four different next actions for an operator. Folding
//! the middle two into an empty list is the mistake worth naming: "plug
//! something in" and "this server cannot help you from here" send someone
//! to opposite ends of the shack.

use cat_signal::{DeviceKind, DeviceList};

/// What the server has said about its hardware.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Offer {
    /// Nothing asked yet, or asked and still waiting.
    #[default]
    Unasked,
    /// The server does not offer device selection -- either it declines,
    /// or it predates the command and could not parse it.
    NotOffered,
    /// The server's own answer, one list per kind.
    Listed(Vec<DeviceList>),
}

impl Offer {
    /// The lists to draw, if the server gave any.
    pub fn lists(&self) -> &[DeviceList] {
        match self {
            Offer::Listed(lists) => lists,
            _ => &[],
        }
    }
}

/// The heading a group gets, and the line under it when it has no rows.
///
/// Separated from the drawing so it can be asserted without an egui
/// context: what these lines *say* is the whole point of the type, and a
/// test that had to spin up a renderer to check it would not get written.
pub fn empty_line(list: &DeviceList) -> Option<String> {
    match &list.error {
        Some(why) => Some(format!("unavailable: {why}")),
        None if list.devices.is_empty() => Some("nothing attached".to_string()),
        None => None,
    }
}

/// What to show instead of a list, when there is no list.
pub fn absent_line(offer: &Offer) -> Option<&'static str> {
    match offer {
        Offer::Unasked => Some("asking the radio's host what it has…"),
        Offer::NotOffered => {
            Some("this server does not offer device selection — set its sources with --if-out")
        }
        Offer::Listed(_) => None,
    }
}

/// The flag an operator would type to get the same result.
///
/// Picking is a shortcut for typing, and a row that showed only a friendly
/// name would leave someone unable to write down what they chose.
pub fn flag_for(kind: DeviceKind) -> &'static str {
    kind.flag()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_signal::DeviceInfo;

    fn dev(spec: &str) -> DeviceInfo {
        DeviceInfo {
            kind: DeviceKind::Sdr,
            spec: spec.to_string(),
            label: "a dongle".to_string(),
            detail: None,
            is_default: false,
        }
    }

    #[test]
    fn not_offered_and_found_nothing_read_differently() {
        // The distinction the type exists for. A console that showed the
        // same line for both would send an operator to check their cabling
        // when the real answer is that this server cannot help from here.
        let declined = absent_line(&Offer::NotOffered).unwrap();
        let empty = Offer::Listed(vec![DeviceList::found(DeviceKind::Sdr, Vec::new())]);
        assert!(
            absent_line(&empty).is_none(),
            "an answer is still an answer"
        );
        let empty_row = empty_line(&empty.lists()[0]).unwrap();

        assert_ne!(declined, empty_row);
        assert!(declined.contains("does not offer"), "{declined}");
        assert!(empty_row.contains("nothing attached"), "{empty_row}");
    }

    #[test]
    fn a_group_that_could_not_be_asked_says_why() {
        let list = DeviceList::unavailable(DeviceKind::AudioInput, "no sound backend");
        assert_eq!(
            empty_line(&list).unwrap(),
            "unavailable: no sound backend",
            "the server's reason must survive to the screen, not be replaced by ours"
        );
    }

    #[test]
    fn a_group_with_devices_has_no_stand_in_line() {
        let list = DeviceList::found(DeviceKind::Sdr, vec![dev("rtl:0")]);
        assert_eq!(empty_line(&list), None);
    }

    #[test]
    fn waiting_is_not_reported_as_nothing() {
        // A console that showed "nothing attached" between connecting and
        // the first reply would be making a claim it has not yet heard.
        let line = absent_line(&Offer::Unasked).unwrap();
        assert!(line.contains("asking"), "{line}");
    }

    #[test]
    fn every_kind_names_the_flag_that_would_do_the_same() {
        assert_eq!(flag_for(DeviceKind::Sdr), "--if-out");
        assert_eq!(flag_for(DeviceKind::AudioInput), "--acc2-audio");
    }
}
