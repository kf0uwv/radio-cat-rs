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

//! Which workspaces this radio has, derived from what it says it is.
//!
//! The accepted design (option 3, "Workspace") makes the tab bar a
//! **statement about the radio** rather than a fixed menu: a radio with no
//! memory has no MEMORY tab, and the counts in the labels come from the
//! capability set rather than from a constant somebody has to remember to
//! change. The same derivation drives the TUI, which is what makes ADR
//! 0013's parity rule cheap to keep rather than a thing to police.

use cat_native::CapabilitiesWire;

/// One workspace the console can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Spectrum,
    Memory,
    Menu,
    Source,
}

/// A tab, with the label the radio's own numbers produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabEntry {
    pub tab: Tab,
    pub label: String,
}

/// The tabs this radio has, in display order.
///
/// SOURCE is always present, and is the one tab that is about the
/// installation rather than the model: it is where an operator sees what
/// is actually plugged in, and a station with nothing attached still needs
/// somewhere for that to be said. Every other tab is a model fact and is
/// absent when the radio lacks the capability — not present-but-disabled,
/// which is how an operator ends up hunting for a control that was never
/// going to work.
pub fn tabs(caps: &CapabilitiesWire) -> Vec<TabEntry> {
    let mut out = Vec::new();

    if !matches!(caps.signal, cat_native::SignalSupport::None) {
        out.push(TabEntry {
            tab: Tab::Spectrum,
            label: "SPECTRUM".to_string(),
        });
    }
    if let Some(memory) = caps.memory {
        out.push(TabEntry {
            tab: Tab::Memory,
            // The radio's own numbering, not a count. A TS-570D starts at
            // 0 and an FT-991A at 1, and an operator reading "117" would
            // guess wrong about one of them.
            label: format!("MEMORY {}–{}", memory.channels.min, memory.channels.max),
        });
    }
    if let Some(menu) = caps.menu {
        out.push(TabEntry {
            tab: Tab::Menu,
            label: format!("MENU {}", menu.item_count),
        });
    }
    out.push(TabEntry {
        tab: Tab::Source,
        label: "SOURCE".to_string(),
    });
    out
}

/// The tab a digit key selects, 1-based, as the status line advertises.
pub fn tab_for_digit(entries: &[TabEntry], digit: usize) -> Option<Tab> {
    digit
        .checked_sub(1)
        .and_then(|i| entries.get(i))
        .map(|e| e.tab)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::{bare as caps_bare, full as caps_ts570d};

    #[test]
    fn the_tabs_are_the_radios_own_description_of_itself() {
        let entries = tabs(&caps_ts570d());
        let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, vec!["SPECTRUM", "MEMORY 0–99", "MENU 52", "SOURCE"]);
    }

    #[test]
    fn a_capability_the_radio_lacks_has_no_tab_at_all() {
        // Not a disabled tab. A control that is present but never going to
        // work is worse than one that is absent, because an operator will
        // spend time on it.
        let entries = tabs(&caps_bare());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tab, Tab::Source);
    }

    #[test]
    fn a_radio_with_no_spectrum_source_has_no_spectrum_tab() {
        // The FT-991A's case: a scope display, but no command that returns
        // scope data. The console must not offer a waterfall that will
        // never fill.
        let mut caps = caps_ts570d();
        caps.signal = cat_native::SignalSupport::None;
        assert!(!tabs(&caps).iter().any(|e| e.tab == Tab::Spectrum));
    }

    #[test]
    fn memory_labels_carry_the_radios_numbering_and_not_a_count() {
        // 0-99 and 1-117 are a hundred and a hundred-and-seventeen
        // channels, and both start somewhere a consumer would guess wrong.
        let mut caps = caps_ts570d();
        caps.memory = Some(cat_native::MemoryCapability {
            channels: cat_native::RawRange::new(1, 117),
            named: true,
            stores_mode: true,
            scan: true,
        });
        let entries = tabs(&caps);
        assert!(entries.iter().any(|e| e.label == "MEMORY 1–117"));
    }

    #[test]
    fn source_is_offered_even_when_nothing_is_plugged_in() {
        // It is the tab that says so.
        let entries = tabs(&caps_bare());
        assert!(entries.iter().any(|e| e.tab == Tab::Source));
    }

    #[test]
    fn digit_keys_select_the_tabs_the_status_line_advertises() {
        let entries = tabs(&caps_ts570d());
        assert_eq!(tab_for_digit(&entries, 1), Some(Tab::Spectrum));
        assert_eq!(tab_for_digit(&entries, 4), Some(Tab::Source));
        assert_eq!(tab_for_digit(&entries, 5), None);
        // Guards the 1-based arithmetic rather than the happy path.
        assert_eq!(tab_for_digit(&entries, 0), None);
    }

    #[test]
    fn the_digits_follow_the_radio_rather_than_a_fixed_map() {
        // On a radio with no spectrum, "1" is whatever is actually first.
        // A hardcoded 1=SPECTRUM would key an absent tab.
        let mut caps = caps_ts570d();
        caps.signal = cat_native::SignalSupport::None;
        let entries = tabs(&caps);
        assert_eq!(tab_for_digit(&entries, 1), Some(Tab::Memory));
    }
}
