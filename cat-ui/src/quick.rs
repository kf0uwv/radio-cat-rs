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

//! The quick-settings bar: mode, filter, shift, split — always visible.
//!
//! # Why this exists
//!
//! The accepted design (option 3) reaches every capability through tabs
//! and a command line, which is what makes it keyboard-complete and what
//! makes the TUI able to hold parity with it. The review found the cost:
//! *"it is unclear how to change quick settings like mode, filters, etc
//! like you can in 2"*. Correct. A command line is complete and
//! undiscoverable, and the settings an operator changes constantly should
//! not require knowing a verb.
//!
//! So the controls that get touched every few minutes live on a permanent
//! bar, and the command line stays as the complete path rather than the
//! only one. Both drive the same commands.
//!
//! # Absent is not off
//!
//! Which controls appear is derived from the radio, and a control the
//! radio lacks is **not shown at all** rather than shown greyed. The
//! TS-570D is the case that makes this concrete: it has IF shift but
//! publishes no selectable filter widths over CAT and has no notch. A bar
//! offering a width selector that can never do anything is a bar that
//! wastes an operator's time exactly once, and then teaches them to
//! distrust the rest of it.

use cat_native::CapabilitiesWire;

/// One control on the quick bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    /// Cycle through the radio's modes.
    Mode,
    /// Choose from the radio's published filter widths.
    FilterWidth {
        widths_hz: Vec<u32>,
    },
    /// Nudge IF shift, within the radio's limit.
    IfShift {
        limit_hz: i32,
    },
    Notch,
    Split,
}

impl Control {
    /// The label the bar shows for this control.
    pub fn label(&self) -> &'static str {
        match self {
            Control::Mode => "MODE",
            Control::FilterWidth { .. } => "FILTER",
            Control::IfShift { .. } => "SHIFT",
            Control::Notch => "NOTCH",
            Control::Split => "SPLIT",
        }
    }
}

/// The quick controls this radio actually has, in bar order.
pub fn controls(caps: &CapabilitiesWire) -> Vec<Control> {
    let mut out = Vec::new();
    if !caps.modes.is_empty() {
        out.push(Control::Mode);
    }
    if let Some(widths) = caps.filters.widths_hz.as_ref() {
        if !widths.is_empty() {
            out.push(Control::FilterWidth {
                widths_hz: widths.clone(),
            });
        }
    }
    if let Some(limit_hz) = caps.filters.if_shift_hz {
        out.push(Control::IfShift { limit_hz });
    }
    if caps.filters.notch {
        out.push(Control::Notch);
    }
    if caps.vfos.split {
        out.push(Control::Split);
    }
    out
}

/// The mode after this one, wrapping.
///
/// Cycling rather than a dropdown because mode is the control that gets
/// changed most and a dropdown costs two clicks. The order is the radio's
/// own, so it matches the order everything else lists modes in.
pub fn next_mode(
    caps: &CapabilitiesWire,
    current: Option<cat_native::ModeId>,
) -> Option<cat_native::ModeId> {
    if caps.modes.is_empty() {
        return None;
    }
    let index = current
        .and_then(|c| caps.modes.iter().position(|m| m.id == c))
        .map(|i| (i + 1) % caps.modes.len())
        .unwrap_or(0);
    Some(caps.modes[index].id)
}

/// Clamp an IF-shift request to what the radio will accept.
///
/// Symmetric, because the capability model carries one magnitude — which
/// is what both radios described so far actually have.
pub fn clamp_shift(hz: i32, limit_hz: i32) -> i32 {
    hz.clamp(-limit_hz.abs(), limit_hz.abs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::{bare as caps_bare, full as caps_ts570d};

    #[test]
    fn the_bar_offers_what_this_radio_can_actually_do() {
        // The TS-570D has IF shift but publishes no selectable widths over
        // CAT, and has no notch.
        let controls = controls(&caps_ts570d());
        let labels: Vec<&str> = controls.iter().map(|c| c.label()).collect();
        assert_eq!(labels, vec!["MODE", "SHIFT", "SPLIT"]);
    }

    #[test]
    fn a_control_the_radio_lacks_is_absent_and_not_merely_greyed() {
        // A width selector that can never do anything wastes an operator's
        // time exactly once, and then teaches them to distrust the bar.
        let controls = controls(&caps_ts570d());
        assert!(!controls
            .iter()
            .any(|c| matches!(c, Control::FilterWidth { .. })));
        assert!(!controls.contains(&Control::Notch));
    }

    #[test]
    fn a_radio_with_widths_gets_a_width_control_carrying_them() {
        let mut caps = caps_ts570d();
        caps.filters.widths_hz = Some(vec![500, 2400]);
        caps.filters.notch = true;
        let controls = controls(&caps);
        assert!(controls.contains(&Control::Notch));
        match controls
            .iter()
            .find(|c| matches!(c, Control::FilterWidth { .. }))
            .expect("has a width control")
        {
            Control::FilterWidth { widths_hz } => assert_eq!(widths_hz, &[500, 2400]),
            other => panic!("expected a width control, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_width_list_is_the_same_as_no_width_list() {
        // A radio that published `Some(vec![])` would otherwise get a
        // control with nothing in it.
        let mut caps = caps_ts570d();
        caps.filters.widths_hz = Some(vec![]);
        assert!(!controls(&caps)
            .iter()
            .any(|c| matches!(c, Control::FilterWidth { .. })));
    }

    #[test]
    fn a_radio_that_can_do_nothing_quick_gets_an_empty_bar() {
        assert!(controls(&caps_bare()).is_empty());
    }

    #[test]
    fn mode_cycles_in_the_radios_own_order_and_wraps() {
        let caps = caps_ts570d();
        let first = caps.modes[0].id;
        let last = caps.modes[caps.modes.len() - 1].id;
        assert_eq!(next_mode(&caps, Some(last)), Some(first));
        assert_eq!(next_mode(&caps, Some(first)), Some(caps.modes[1].id));
    }

    #[test]
    fn cycling_from_an_unknown_mode_starts_at_the_beginning() {
        // The radio can be in a mode this console has not heard from yet
        // -- on startup, before the first poll lands.
        let caps = caps_ts570d();
        assert_eq!(next_mode(&caps, None), Some(caps.modes[0].id));
    }

    #[test]
    fn cycling_a_mode_the_radio_does_not_have_does_not_wedge() {
        // A stale readout naming a mode that is not in the set must not
        // leave the control dead.
        let caps = caps_ts570d();
        assert_eq!(
            next_mode(&caps, Some(cat_native::ModeId::C4fm)),
            Some(caps.modes[0].id)
        );
    }

    #[test]
    fn shift_is_clamped_to_the_radios_own_limit_in_both_directions() {
        assert_eq!(clamp_shift(5_000, 1_000), 1_000);
        assert_eq!(clamp_shift(-5_000, 1_000), -1_000);
        assert_eq!(clamp_shift(250, 1_000), 250);
        // A radio publishing a negative magnitude must not invert the
        // clamp into an empty range and panic.
        assert_eq!(clamp_shift(5_000, -1_000), 1_000);
    }
}
