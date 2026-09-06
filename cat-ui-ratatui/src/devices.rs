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

//! Choosing a device from what the machine can see.
//!
//! Sound-card names are per-host and unguessable — nobody types
//! `plughw:CARD=Codec,DEV=0` from memory — so a console has to offer them.
//! This is the terminal half of that; the selection logic underneath is
//! [`Selection`], which has no ratatui in it so the GUI can share it.
//!
//! # Three outcomes per group, and they must look different
//!
//! [`cat_signal::DeviceList`] distinguishes "found some", "found none" and
//! "could not ask", and this renders all three differently on purpose.
//! Collapsing the last two into an empty list is the failure mode worth
//! avoiding: "plug something in" and "this build cannot see your hardware"
//! send an operator to opposite ends of the shack.
//!
//! # Picking is a shortcut for typing
//!
//! Every row shows the **spec** — the exact string the flag takes — beside
//! the driver's own label. An operator who picks a device can then write it
//! down and use it on the command line next time, which a picker that
//! produced an opaque handle could not offer.

use cat_signal::{DeviceKind, DeviceList};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// Where the cursor is, across several device groups shown as one list.
///
/// Flattened rather than per-group, because arrow keys move down a screen
/// and not down a category — an operator should not have to know the list
/// is three lists.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    index: usize,
}

/// One selectable row: which list it came from, and which device in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub list: usize,
    pub device: usize,
}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every pickable row, in display order.
    ///
    /// Only real devices are pickable: a heading is not, and neither is the
    /// line explaining that a group could not be enumerated.
    pub fn choices(lists: &[DeviceList]) -> Vec<Choice> {
        lists
            .iter()
            .enumerate()
            .flat_map(|(l, list)| {
                (0..list.devices.len()).map(move |d| Choice { list: l, device: d })
            })
            .collect()
    }

    /// What is currently selected, if anything is selectable at all.
    pub fn current(&self, lists: &[DeviceList]) -> Option<Choice> {
        Self::choices(lists).get(self.index).copied()
    }

    /// Move the cursor, clamping at both ends.
    ///
    /// Clamped rather than wrapping: a list that jumps from the bottom back
    /// to the top makes an operator lose their place, and these lists are
    /// short enough that wrapping saves nothing.
    pub fn move_by(&mut self, delta: isize, lists: &[DeviceList]) {
        let count = Self::choices(lists).len();
        if count == 0 {
            self.index = 0;
            return;
        }
        let next = self.index as isize + delta;
        self.index = next.clamp(0, count as isize - 1) as usize;
    }

    /// Put the cursor on the host's default, if any group named one.
    ///
    /// Called when a picker opens, so the most likely answer is already
    /// under the cursor.
    pub fn select_default(&mut self, lists: &[DeviceList]) {
        if let Some(i) = Self::choices(lists)
            .iter()
            .position(|c| lists[c.list].devices[c.device].is_default)
        {
            self.index = i;
        }
    }
}

/// Styles a picker draws with.
#[derive(Debug, Clone, Copy)]
pub struct PickerStyles {
    pub heading: Style,
    pub label: Style,
    pub spec: Style,
    pub detail: Style,
    /// The row under the cursor.
    pub selected: Style,
    /// "nothing plugged in", and the reason a group could not be asked.
    pub absent: Style,
}

/// Render the picker as lines.
///
/// Lines rather than a widget so a caller can put it in whatever panel it
/// already has — a tab body, a modal — without this module owning layout.
pub fn picker_lines(
    lists: &[DeviceList],
    selection: &Selection,
    styles: PickerStyles,
) -> Vec<Line<'static>> {
    let current = selection.current(lists);
    let mut out = Vec::new();

    for (l, list) in lists.iter().enumerate() {
        out.push(Line::from(Span::styled(
            list.kind.heading().to_string(),
            styles.heading,
        )));

        match (&list.error, list.devices.is_empty()) {
            // Could not ask. Says what to do, because "no devices" would
            // send the operator to look at their cabling instead.
            (Some(why), _) => out.push(Line::from(Span::styled(
                format!("  unavailable: {why}"),
                styles.absent,
            ))),
            // Asked, and there is nothing there.
            (None, true) => out.push(Line::from(Span::styled(
                "  nothing attached".to_string(),
                styles.absent,
            ))),
            (None, false) => {
                for (d, device) in list.devices.iter().enumerate() {
                    let here = current == Some(Choice { list: l, device: d });
                    let mark = if here { "> " } else { "  " };
                    let label_style = if here { styles.selected } else { styles.label };
                    let mut spans = vec![
                        Span::styled(mark.to_string(), label_style),
                        Span::styled(device.label.clone(), label_style),
                    ];
                    if device.is_default {
                        spans.push(Span::styled("  (default)".to_string(), styles.detail));
                    }
                    out.push(Line::from(spans));
                    // The spec on its own line, so a long ALSA name and a
                    // long label do not fight for the same row.
                    out.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(device.spec.clone(), styles.spec),
                    ]));
                    if let Some(detail) = &device.detail {
                        out.push(Line::from(vec![
                            Span::raw("    "),
                            Span::styled(detail.clone(), styles.detail),
                        ]));
                    }
                }
            }
        }
        out.push(Line::from(Span::raw("")));
    }

    out
}

/// The hint line telling an operator what a pick is equivalent to.
pub fn picker_hint(kind: DeviceKind, spec: &str, style: Style) -> Line<'static> {
    Line::from(Span::styled(
        format!("same as: {} {}", kind.flag(), spec),
        style,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_signal::{DeviceInfo, DeviceKind};
    use ratatui::style::Modifier;

    fn dev(kind: DeviceKind, spec: &str, is_default: bool) -> DeviceInfo {
        DeviceInfo {
            kind,
            spec: spec.to_string(),
            label: format!("{spec} label"),
            detail: None,
            is_default,
        }
    }

    fn lists() -> Vec<DeviceList> {
        vec![
            DeviceList::found(
                DeviceKind::AudioInput,
                vec![
                    dev(DeviceKind::AudioInput, "hw:0,0", false),
                    dev(DeviceKind::AudioInput, "hw:1,0", true),
                ],
            ),
            DeviceList::found(DeviceKind::Sdr, vec![dev(DeviceKind::Sdr, "rtl:0", false)]),
        ]
    }

    fn text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn styles() -> PickerStyles {
        let s = Style::default();
        PickerStyles {
            heading: s.add_modifier(Modifier::BOLD),
            label: s,
            spec: s,
            detail: s,
            selected: s.add_modifier(Modifier::REVERSED),
            absent: s,
        }
    }

    #[test]
    fn the_cursor_moves_across_groups_and_not_within_one() {
        // An operator moves down a screen, not down a category, and should
        // not have to know the list is more than one list.
        let lists = lists();
        let mut sel = Selection::new();
        assert_eq!(sel.current(&lists), Some(Choice { list: 0, device: 0 }));
        sel.move_by(1, &lists);
        assert_eq!(sel.current(&lists), Some(Choice { list: 0, device: 1 }));
        sel.move_by(1, &lists);
        assert_eq!(
            sel.current(&lists),
            Some(Choice { list: 1, device: 0 }),
            "the third row down is the first SDR"
        );
    }

    #[test]
    fn the_cursor_clamps_rather_than_wrapping() {
        let lists = lists();
        let mut sel = Selection::new();
        sel.move_by(-5, &lists);
        assert_eq!(sel.current(&lists), Some(Choice { list: 0, device: 0 }));
        sel.move_by(99, &lists);
        assert_eq!(sel.current(&lists), Some(Choice { list: 1, device: 0 }));
    }

    #[test]
    fn opening_the_picker_lands_on_the_hosts_default() {
        let lists = lists();
        let mut sel = Selection::new();
        sel.select_default(&lists);
        assert_eq!(sel.current(&lists), Some(Choice { list: 0, device: 1 }));
    }

    #[test]
    fn a_list_with_nothing_in_it_has_nothing_to_select() {
        let empty = vec![DeviceList::found(DeviceKind::Sdr, Vec::new())];
        let mut sel = Selection::new();
        sel.move_by(3, &empty);
        assert_eq!(sel.current(&empty), None, "and moving does not panic");
    }

    #[test]
    fn cannot_ask_reads_differently_from_nothing_attached() {
        // The distinction the whole type exists for, at the point an
        // operator actually sees it.
        let broken = vec![DeviceList::unavailable(
            DeviceKind::AudioInput,
            "no sound backend in this build",
        )];
        let empty = vec![DeviceList::found(DeviceKind::AudioInput, Vec::new())];

        let broken = text(&picker_lines(&broken, &Selection::new(), styles()));
        let empty = text(&picker_lines(&empty, &Selection::new(), styles()));

        assert!(broken.contains("unavailable"), "{broken}");
        assert!(broken.contains("no sound backend in this build"));
        assert!(empty.contains("nothing attached"), "{empty}");
        assert!(!empty.contains("unavailable"));
    }

    #[test]
    fn every_row_shows_the_string_the_command_line_would_take() {
        // Picking is a shortcut for typing. A row that showed only a
        // friendly name would leave an operator unable to write down what
        // they picked.
        let rendered = text(&picker_lines(&lists(), &Selection::new(), styles()));
        for spec in ["hw:0,0", "hw:1,0", "rtl:0"] {
            assert!(rendered.contains(spec), "{spec} missing from:\n{rendered}");
        }
    }

    #[test]
    fn the_host_default_is_marked_so_a_picker_can_be_trusted() {
        let rendered = text(&picker_lines(&lists(), &Selection::new(), styles()));
        assert!(rendered.contains("(default)"));
    }

    #[test]
    fn the_hint_names_the_flag_that_takes_the_pick() {
        let hint = picker_hint(DeviceKind::Sdr, "rtl:0", Style::default());
        let text: String = hint.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "same as: --if-out rtl:0");
    }

    #[test]
    fn both_groups_are_headed_even_when_one_is_empty() {
        // A group that vanished when empty would make an operator wonder
        // whether the console supports that kind of device at all.
        let lists = vec![
            DeviceList::found(DeviceKind::AudioInput, Vec::new()),
            DeviceList::found(DeviceKind::Sdr, vec![dev(DeviceKind::Sdr, "rtl:0", false)]),
        ];
        let rendered = text(&picker_lines(&lists, &Selection::new(), styles()));
        assert!(rendered.contains("AUDIO INPUT"));
        assert!(rendered.contains("SDR"));
    }
}
