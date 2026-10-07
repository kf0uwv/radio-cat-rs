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

//! CW state on the GPU console (ADR 0023 decision 12): the same
//! [`cat_ui::cw::CwPanel`] the terminal console draws, so the two say the
//! same thing (ADR 0013).

use cat_native::{CwCapabilityWire, CwState};
use cat_ui::cw::{CwPanel, Tone};
use egui::{Color32, RichText};

use crate::theme::{self, Palette};

/// What the status strip says: CW first in every layout. A failed
/// restore replaces the console's own text; an arm or a send rides in
/// front of it.
pub fn status_line(
    own: &str,
    cw: Option<&CwState>,
    cap: Option<&CwCapabilityWire>,
) -> (String, Option<Tone>) {
    match cw.and_then(|state| CwPanel::new(state, cap).status()) {
        Some((alert, Tone::Alarm)) => (alert, Some(Tone::Alarm)),
        Some((text, tone)) if own.is_empty() => (text, Some(tone)),
        Some((text, tone)) => (format!("{text}  ·  {own}"), Some(tone)),
        None => (own.to_string(), None),
    }
}

/// A tone in this console's palette.
pub fn colour(tone: Tone, pal: &Palette) -> Color32 {
    match tone {
        Tone::Quiet => pal.dim,
        Tone::Normal => pal.text,
        Tone::Active => pal.warning,
        Tone::Warning => pal.accent,
        Tone::Alarm => pal.warning,
    }
}

/// The shared CW panel.
pub fn panel(ui: &mut egui::Ui, cw: Option<&CwState>, cap: Option<&CwCapabilityWire>) {
    let pal = theme::pal();
    let Some(state) = cw else {
        ui.label(
            RichText::new("no CW control on this server")
                .color(pal.absent)
                .size(theme::SIZE_BODY),
        );
        return;
    };
    let panel = CwPanel::new(state, cap);
    if let Some(alert) = &panel.alert {
        // As loud as this console gets: the radio is not as it was left.
        egui::Frame::none()
            .fill(pal.warning)
            .inner_margin(egui::Margin::same(4.0))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(alert)
                        .color(pal.bg)
                        .strong()
                        .size(theme::SIZE_BODY),
                );
            });
    }
    if let Some(owed) = &panel.id_owed {
        ui.label(
            RichText::new(owed)
                .color(colour(Tone::Warning, &pal))
                .strong()
                .size(theme::SIZE_BODY),
        );
    }
    for line in panel.lines() {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{:<5}", line.label))
                    .monospace()
                    .color(pal.dim)
                    .size(theme::SIZE_BODY),
            );
            ui.label(
                RichText::new(&line.text)
                    .color(colour(line.tone, &pal))
                    .size(theme::SIZE_BODY),
            );
            if line.label == "SEND" {
                if let Some(fraction) = panel.progress {
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .desired_width(120.0)
                            .fill(colour(line.tone, &pal)),
                    );
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_native::{ArmedState, RestoreStatus, SenderState};

    fn state(restore: bool, armed: bool) -> CwState {
        CwState {
            pitch_hz: Some(600),
            keyer_wpm: Some(20),
            break_in: None,
            armed: armed.then(|| ArmedState {
                callsign: "N0CALL".into(),
                power_raw: 5,
                mine: true,
            }),
            sender: SenderState::Sending {
                job: 1,
                sent_chars: 3,
                total_chars: 6,
            },
            restore: restore.then(|| RestoreStatus {
                attempts: 1,
                last_error: "PC write timed out".into(),
            }),
            id_owed: None,
        }
    }

    #[test]
    fn a_failed_restore_replaces_the_status_line() {
        let s = state(true, true);
        let (text, tone) = status_line("frequency set", Some(&s), None);
        assert_eq!(tone, Some(Tone::Alarm));
        assert!(text.starts_with("CW RESTORE FAILED (1 attempt)"), "{text}");
        assert!(!text.contains("frequency set"));
    }

    #[test]
    fn a_send_rides_in_front_of_the_consoles_own_text() {
        let s = state(false, true);
        let (text, tone) = status_line("frequency set", Some(&s), None);
        assert_eq!(text, "CW sending #1 · 3/6  ·  frequency set");
        assert_eq!(tone, Some(Tone::Active));
    }

    #[test]
    fn without_cw_the_line_is_the_consoles_own() {
        assert_eq!(
            status_line("ready", None, None),
            ("ready".to_string(), None)
        );
    }

    #[test]
    fn the_panel_paints_every_state_without_panicking() {
        let ctx = egui::Context::default();
        for cw in [None, Some(state(false, false)), Some(state(true, true))] {
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| panel(ui, cw.as_ref(), None));
            });
        }
    }

    #[test]
    fn the_terminal_and_the_gpu_console_say_the_same_thing() {
        // ADR 0013: one model, two renderers. The strip text here is the
        // model's, word for word.
        let s = state(false, true);
        let model = CwPanel::new(&s, None).status().unwrap().0;
        assert!(status_line("", Some(&s), None).0 == model);
    }

    #[test]
    fn an_owed_id_reaches_the_status_line() {
        let s = CwState {
            armed: None,
            sender: SenderState::Idle,
            id_owed: Some("N0CALL".into()),
            ..state(false, false)
        };
        let (text, tone) = status_line("ready", Some(&s), None);
        assert_eq!(tone, Some(Tone::Warning));
        assert!(text.contains("ID OWED: N0CALL"), "{text}");
    }
}
