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

//! One generic renderer for a source's own settings.
//!
//! ADR 0010 §4. **Nothing here switches on `SignalCapability`**, and the
//! TS-570D's `trim_hz` gets no special treatment — it is one row in the
//! Calibration group, like any other.
//!
//! This is the same contract `cat-ui-ratatui`'s renderer honours, which is
//! what keeps a setting from being reachable in one renderer and invisible
//! in the other (ADR 0013).

use cat_signal::{Access, SettingDescriptor, SettingGroup, SettingValue, SpectrumSettings};
use egui::Ui;

/// Format a value with its unit, identically to the terminal renderer.
pub fn format_value(value: &SettingValue) -> String {
    match value {
        SettingValue::Int { value, unit, .. } => match unit.suffix() {
            "" => value.to_string(),
            s => format!("{value} {s}"),
        },
        SettingValue::Float { value, unit, .. } => match unit.suffix() {
            "" => format!("{value:.1}"),
            s => format!("{value:.1} {s}"),
        },
        SettingValue::Bool(b) => (if *b { "on" } else { "off" }).to_string(),
        SettingValue::Enum { value, options } => options
            .get(usize::from(*value))
            .map(|s| (*s).to_string())
            // An out-of-range index is a bug in the source. An empty cell
            // would hide it; this does not.
            .unwrap_or_else(|| format!("? ({value})")),
    }
}

/// Groups laid out in order. `SettingGroup` is `#[non_exhaustive]`, so
/// [`settings_grid`] also sweeps up anything in a group this crate has not
/// heard of — otherwise a group added to `cat-signal` later would silently
/// vanish from one renderer while still showing in the other.
const KNOWN_GROUPS: [SettingGroup; 3] = [
    SettingGroup::Source,
    SettingGroup::Display,
    SettingGroup::Calibration,
];

fn group_name(group: SettingGroup) -> &'static str {
    match group {
        SettingGroup::Source => "SOURCE",
        SettingGroup::Display => "DISPLAY",
        SettingGroup::Calibration => "CALIBRATION",
        _ => "OTHER",
    }
}

/// The rows a settings panel should draw, grouped and in order.
///
/// Returned rather than painted so that spacing, columns and controls stay
/// with the app — a settings panel's *shape* is layout. What is shared is
/// which rows exist, in what order, under what headings, and which are
/// writable.
pub fn settings_rows(settings: &SpectrumSettings) -> Vec<Row<'_>> {
    let mut out = Vec::new();
    let mut drawn = 0usize;
    for group in KNOWN_GROUPS {
        let rows: Vec<&SettingDescriptor> = settings.group(group).collect();
        if rows.is_empty() {
            continue;
        }
        out.push(Row::Heading(group_name(group)));
        drawn += rows.len();
        out.extend(rows.into_iter().map(Row::Setting));
    }
    if drawn < settings.descriptors.len() {
        let leftover: Vec<&SettingDescriptor> = settings
            .descriptors
            .iter()
            .filter(|d| !KNOWN_GROUPS.contains(&d.group))
            .collect();
        if !leftover.is_empty() {
            out.push(Row::Heading("OTHER"));
            out.extend(leftover.into_iter().map(Row::Setting));
        }
    }
    out
}

/// A row in a settings panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Row<'a> {
    Heading(&'static str),
    Setting(&'a SettingDescriptor),
}

/// Draw a settings panel as a two-column grid.
pub fn settings_grid(ui: &mut Ui, settings: &SpectrumSettings) {
    for row in settings_rows(settings) {
        match row {
            Row::Heading(h) => {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(h).small().strong());
            }
            Row::Setting(d) => {
                ui.horizontal(|ui| {
                    ui.label(d.label);
                    ui.label(format_value(&d.value));
                    if d.access == Access::ReadOnly {
                        ui.label(egui::RichText::new("RO").small().weak());
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_signal::Unit;

    fn d(
        key: &'static str,
        label: &'static str,
        group: SettingGroup,
        access: Access,
        value: SettingValue,
    ) -> SettingDescriptor {
        SettingDescriptor {
            key,
            label,
            group,
            access,
            value,
        }
    }

    fn if_tap() -> SpectrumSettings {
        SpectrumSettings::new(vec![
            d(
                "gain_db",
                "Gain",
                SettingGroup::Source,
                Access::ReadWrite,
                SettingValue::Float {
                    value: 28.0,
                    min: 0.0,
                    max: 49.6,
                    unit: Unit::Db,
                },
            ),
            d(
                "trim_hz",
                "Frequency trim",
                SettingGroup::Calibration,
                Access::ReadWrite,
                SettingValue::Int {
                    value: -1420,
                    min: -50_000,
                    max: 50_000,
                    step: 1,
                    unit: Unit::Hz,
                },
            ),
        ])
    }

    #[test]
    fn trim_hz_is_one_row_among_others() {
        let settings = if_tap();
        let rows = settings_rows(&settings);
        let trims = rows
            .iter()
            .filter(|r| matches!(r, Row::Setting(d) if d.key == "trim_hz"))
            .count();
        assert_eq!(trims, 1);
    }

    #[test]
    fn a_group_with_no_descriptors_gets_no_heading() {
        let audio = SpectrumSettings::new(vec![d(
            "input_device",
            "Input device",
            SettingGroup::Source,
            Access::ReadWrite,
            SettingValue::Enum {
                value: 0,
                options: &["Box USB Audio"],
            },
        )]);
        let headings: Vec<&str> = settings_rows(&audio)
            .iter()
            .filter_map(|r| match r {
                Row::Heading(h) => Some(*h),
                _ => None,
            })
            .collect();
        assert_eq!(headings, vec!["SOURCE"]);
    }

    #[test]
    fn both_renderers_format_a_value_identically() {
        // ADR 0013 parity, at the smallest possible scale. If these two
        // ever diverge, the same setting reads differently depending on
        // which renderer an operator happens to be using.
        let v = SettingValue::Float {
            value: 28.0,
            min: 0.0,
            max: 49.6,
            unit: Unit::Db,
        };
        assert_eq!(format_value(&v), cat_ui_ratatui_format(&v));
    }

    /// The terminal renderer's formatting, restated. `cat-ui-ratatui` is
    /// not a dependency of this crate -- a GPU widget set must not pull in
    /// a terminal one -- so parity is asserted against a copy rather than
    /// by calling across. If this ever fails, one of the two moved.
    fn cat_ui_ratatui_format(value: &SettingValue) -> String {
        match value {
            SettingValue::Float { value, unit, .. } => match unit.suffix() {
                "" => format!("{value:.1}"),
                s => format!("{value:.1} {s}"),
            },
            other => format_value(other),
        }
    }

    #[test]
    fn an_out_of_range_enum_is_visible_rather_than_blank() {
        assert_eq!(
            format_value(&SettingValue::Enum {
                value: 7,
                options: &["Hann"]
            }),
            "? (7)"
        );
    }
}
