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
//! ADR 0010 §4: a spectrum source describes its settings as typed
//! descriptors and a UI renders the list. **Nothing here switches on
//! `SignalCapability`**, and there is no special case for the TS-570D's
//! `trim_hz` — it is one row, in the Calibration group, like any other.

use cat_signal::{Access, SettingDescriptor, SettingGroup, SettingValue, SpectrumSettings};
use ratatui::{
    style::Style,
    text::{Line, Span},
};

/// Format a value for display, with its unit.
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
            // An out-of-range enum is a source bug, not a rendering one.
            // Say so rather than drawing an empty cell.
            .unwrap_or_else(|| format!("? ({value})")),
    }
}

fn group_name(group: SettingGroup) -> &'static str {
    match group {
        SettingGroup::Source => "SOURCE",
        SettingGroup::Display => "DISPLAY",
        SettingGroup::Calibration => "CALIBRATION",
        // `SettingGroup` is `#[non_exhaustive]`, so a group added upstream
        // reaches this crate as an unknown. Render it rather than refusing
        // to compile or silently dropping its rows: a new group means new
        // settings, and settings a user cannot see are settings they cannot
        // fix. The heading is deliberately ugly so it gets noticed.
        _ => "OTHER",
    }
}

/// Groups this renderer lays out, in order.
///
/// Also `#[non_exhaustive]`-aware: descriptors in a group not listed here
/// would otherwise never be drawn, so [`settings_rows`] sweeps up whatever
/// is left over at the end.
const KNOWN_GROUPS: [SettingGroup; 3] = [
    SettingGroup::Source,
    SettingGroup::Display,
    SettingGroup::Calibration,
];

/// Render a settings list as lines, grouped, with read-only rows marked.
///
/// A group with no descriptors does not appear at all — an `AudioDerived`
/// source has no Calibration settings, and an empty heading would imply it
/// was missing something.
pub fn settings_rows(
    settings: &SpectrumSettings,
    label_width: usize,
    heading: Style,
    label: Style,
    value: Style,
    readonly: Style,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut drawn = 0usize;
    for group in KNOWN_GROUPS {
        let rows: Vec<&SettingDescriptor> = settings.group(group).collect();
        if rows.is_empty() {
            continue;
        }
        lines.push(Line::from(Span::styled(group_name(group), heading)));
        drawn += rows.len();
        for d in rows {
            let ro = d.access == Access::ReadOnly;
            lines.push(Line::from(vec![
                Span::styled(format!("{:<label_width$}", d.label), label),
                Span::styled(format_value(&d.value), if ro { readonly } else { value }),
                Span::styled(if ro { "  RO" } else { "" }, readonly),
            ]));
        }
    }

    // Anything in a group this crate does not know about. Without this, a
    // group added to `cat-signal` after this crate was written would have
    // its settings silently vanish from the terminal while still appearing
    // in the GUI -- a parity break introduced by an upstream addition,
    // which is exactly the kind nobody would think to look for.
    if drawn < settings.descriptors.len() {
        let leftover: Vec<&SettingDescriptor> = settings
            .descriptors
            .iter()
            .filter(|d| !KNOWN_GROUPS.contains(&d.group))
            .collect();
        if !leftover.is_empty() {
            lines.push(Line::from(Span::styled("OTHER", heading)));
            for d in leftover {
                let ro = d.access == Access::ReadOnly;
                lines.push(Line::from(vec![
                    Span::styled(format!("{:<label_width$}", d.label), label),
                    Span::styled(format_value(&d.value), if ro { readonly } else { value }),
                    Span::styled(if ro { "  RO" } else { "" }, readonly),
                ]));
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_signal::{SettingGroup, Unit};
    use ratatui::style::Style;

    fn descriptor(
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
            descriptor(
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
            descriptor(
                "if_center_hz",
                "IF centre",
                SettingGroup::Calibration,
                Access::ReadOnly,
                SettingValue::Int {
                    value: 73_050_000,
                    min: 0,
                    max: i64::MAX,
                    step: 1,
                    unit: Unit::Hz,
                },
            ),
            descriptor(
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

    fn audio() -> SpectrumSettings {
        SpectrumSettings::new(vec![descriptor(
            "input_device",
            "Input device",
            SettingGroup::Source,
            Access::ReadWrite,
            SettingValue::Enum {
                value: 0,
                options: &["Box USB Audio", "System default"],
            },
        )])
    }

    fn render(s: &SpectrumSettings) -> Vec<String> {
        let st = Style::default();
        settings_rows(s, 18, st, st, st, st)
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn trim_hz_is_one_row_with_no_special_treatment() {
        // ADR 0010 §4's sharpest example. A per-station calibration that a
        // UI hand-wrote a panel for would be exactly the coupling the
        // delegated-settings design exists to remove.
        let lines = render(&if_tap());
        let trim: Vec<&String> = lines
            .iter()
            .filter(|l| l.contains("Frequency trim"))
            .collect();
        assert_eq!(trim.len(), 1);
        assert!(trim[0].contains("-1420 Hz"));
        // It sits under the same heading as any other calibration row.
        let cal = lines.iter().position(|l| l == "CALIBRATION").unwrap();
        let row = lines
            .iter()
            .position(|l| l.contains("Frequency trim"))
            .unwrap();
        assert!(row > cal);
    }

    #[test]
    fn a_group_with_no_descriptors_does_not_appear() {
        // An AudioDerived source has no Calibration settings. An empty
        // heading would imply something was missing.
        let lines = render(&audio());
        assert!(lines.iter().any(|l| l == "SOURCE"));
        assert!(!lines.iter().any(|l| l == "CALIBRATION"));
        assert!(!lines.iter().any(|l| l == "DISPLAY"));
    }

    #[test]
    fn the_renderer_never_switches_on_the_source_type() {
        // The same function, two different source types, no branch between
        // them. If this ever needed a `match` on SignalCapability, the
        // delegated-settings design would have failed.
        assert!(!render(&if_tap()).is_empty());
        assert!(!render(&audio()).is_empty());
    }

    #[test]
    fn read_only_rows_are_marked_as_such() {
        let lines = render(&if_tap());
        let ro = lines.iter().find(|l| l.contains("IF centre")).unwrap();
        assert!(ro.ends_with("RO"), "got {ro:?}");
        let rw = lines.iter().find(|l| l.contains("Frequency trim")).unwrap();
        assert!(!rw.ends_with("RO"));
    }

    #[test]
    fn values_render_with_their_unit() {
        assert_eq!(
            format_value(&SettingValue::Int {
                value: 48_000,
                min: 0,
                max: 1,
                step: 1,
                unit: Unit::Sps
            }),
            "48000 S/s"
        );
        assert_eq!(
            format_value(&SettingValue::Float {
                value: 28.0,
                min: 0.0,
                max: 1.0,
                unit: Unit::Db
            }),
            "28.0 dB"
        );
        assert_eq!(format_value(&SettingValue::Bool(true)), "on");
    }

    #[test]
    fn a_unitless_value_gets_no_stray_suffix() {
        assert_eq!(
            format_value(&SettingValue::Int {
                value: 4,
                min: 0,
                max: 8,
                step: 1,
                unit: Unit::None
            }),
            "4"
        );
    }

    #[test]
    fn an_enum_renders_its_selected_option() {
        assert_eq!(
            format_value(&SettingValue::Enum {
                value: 1,
                options: &["Hann", "Hamming", "Blackman"]
            }),
            "Hamming"
        );
    }

    #[test]
    fn an_out_of_range_enum_says_so_rather_than_drawing_blank() {
        // That is a bug in the source, and a blank cell would hide it.
        assert_eq!(
            format_value(&SettingValue::Enum {
                value: 9,
                options: &["Hann"]
            }),
            "? (9)"
        );
    }
}
