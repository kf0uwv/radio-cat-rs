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

//! The VFO readout.

use cat_ui::format_hz;
use ratatui::{
    style::Style,
    text::{Line, Span},
};

/// A frequency readout, or an explicit unknown.
///
/// `None` renders as em-dashes, never as `0.000.000 MHz`. "Not asked yet"
/// and "the answer is zero" are different states, and a console that
/// conflates them tells its operator the radio is at DC.
///
/// `requested` is the pending value: the confirmed frequency stays put and
/// the requested one follows it after a marker, so the readout never claims
/// a value the radio has not acknowledged.
pub fn vfo_readout(
    hz: Option<u64>,
    requested: Option<u64>,
    confirmed_style: Style,
    pending_style: Style,
    unknown_style: Style,
) -> Line<'static> {
    let mut spans = match hz {
        Some(hz) => vec![Span::styled(format_hz(hz), confirmed_style)],
        None => vec![Span::styled("—.———.——— MHz", unknown_style)],
    };
    if let Some(req) = requested {
        spans.push(Span::styled(
            format!("  ▸ {}", format_hz(req)),
            pending_style,
        ));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn an_unread_vfo_shows_dashes_not_zero() {
        // 0.000.000 MHz would tell an operator the radio is at DC. It is
        // the single most misleading thing a cold console can display.
        let s = Style::default();
        let out = text(vfo_readout(None, None, s, s, s));
        assert!(out.contains('—'));
        assert!(!out.contains("0.000.000"));
    }

    #[test]
    fn a_read_vfo_matches_the_apps_existing_format() {
        let s = Style::default();
        assert_eq!(
            text(vfo_readout(Some(14_074_000), None, s, s, s)),
            "14.074.000 MHz"
        );
    }

    #[test]
    fn a_requested_frequency_follows_the_confirmed_one() {
        // The confirmed value never moves to something unacknowledged.
        let s = Style::default();
        let out = text(vfo_readout(Some(14_074_000), Some(14_195_000), s, s, s));
        assert!(out.starts_with("14.074.000 MHz"));
        assert!(out.contains("▸ 14.195.000 MHz"));
    }

    #[test]
    fn a_request_on_a_cold_vfo_still_renders_both_states() {
        let s = Style::default();
        let out = text(vfo_readout(None, Some(14_195_000), s, s, s));
        assert!(out.contains('—'));
        assert!(out.contains("14.195.000"));
    }
}
