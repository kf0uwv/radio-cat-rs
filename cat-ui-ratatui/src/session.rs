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

//! Session panels: connection state, errors, header, menu columns.
//!
//! These come out of the TUI divergence audit ADR 0011 rev 4 asked for.
//! Diffing `ts570d/ui/src/layout.rs` against `ft991a/ui/src/layout.rs`
//! function by function found **no silent bugfix drift** — the worry that
//! made the audit a prerequisite. What it did find was four functions
//! duplicated for reasons that are not radio differences at all:
//!
//! - `draw_disconnected` — **byte-identical**, 47 lines, in both apps.
//! - `draw_errors` — identical but for the concrete display type it took.
//! - `draw_header` — identical but for the title string.
//! - `build_menu_column` — identical but for the key type (`&str` vs `char`).
//!
//! A second pass, made while actually migrating `ts570d` onto these
//! widgets, found that the extraction had been slightly too eager in three
//! places — it dropped `draw_disconnected`'s `[Q] Quit` footer, fixed the
//! header's alignment to left where one app centres, and made an empty
//! error list draw nothing where one app's fixed-height slot needs it to
//! say so. All three are restored as parameters. Reading the two apps side
//! by side is not the same as running the result; both are worth doing.
//!
//! Everything else that differs, differs legitimately: `split_areas`
//! allocates 7 status rows against 4 because the two radios have different
//! amounts to show, `draw_control_panel` is 5.4× richer on the FT-991A
//! because its menu topology is, and `draw_diag_warning_panel` differs by
//! one line of copy about a keyer memory the TS-570D does not have. Those
//! stay in each app, which is exactly where ADR 0011 puts layout.

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

/// Whether the console has a working link to the radio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// First contact has not completed yet.
    Connecting,
    /// Contact was established and then lost.
    Lost,
}

/// The panel shown when there is no usable link.
///
/// Distinguishes *connecting* from *lost*, which matters more than it
/// looks: on first start a red "CONNECTION LOST" is alarming and wrong,
/// and during an outage a patient "connecting…" hides that something
/// broke. Both apps already drew this distinction identically, which is
/// why it is here rather than in either of them.
///
/// `footer` is set off by a blank line and closes the panel. Both apps end
/// it with `[Q] Quit`, and this panel replaces the control panel outright
/// — so it is the only place the one key that still works is written down.
/// The first extraction dropped that line; migrating `ts570d` is what
/// surfaced it.
pub fn link_panel(
    state: LinkState,
    errors: &[String],
    title: &str,
    footer: Option<Span<'static>>,
    area: Rect,
    buf: &mut Buffer,
) {
    let lines: Vec<Line> = match state {
        LinkState::Connecting => vec![
            Line::from(Span::styled(
                "Connecting to radio...",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(""),
            Line::from("Waiting for response. This may take a few seconds."),
        ],
        LinkState::Lost => {
            let mut v = vec![
                Line::from(Span::styled(
                    "CONNECTION LOST",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from("The radio is not responding."),
                Line::from("Reconnect the cable or restart the radio."),
                Line::from("The UI will recover automatically when contact is restored."),
                Line::from(""),
            ];
            // Bounded: a radio failing in a loop can produce errors far
            // faster than anyone reads them, and an unbounded list would
            // push the recovery instructions off the top of the panel.
            for e in errors.iter().take(8) {
                v.push(Line::from(Span::styled(
                    e.as_str(),
                    Style::default().fg(Color::Yellow),
                )));
            }
            v
        }
    };

    let mut lines = lines;
    if let Some(footer) = footer {
        lines.push(Line::from(""));
        lines.push(Line::from(footer));
    }

    Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {title} ")),
        )
        .wrap(Wrap { trim: false })
        .render(area, buf);
}

/// How an error panel presents itself.
///
/// The empty case is the interesting one, and it has two right answers
/// depending on the layout around it — see [`ErrorPanelStyles::quiet`].
#[derive(Debug, Clone, Copy)]
pub struct ErrorPanelStyles {
    /// Style for each error line.
    pub error: Style,
    /// What to draw when there are no errors.
    ///
    /// `None` draws nothing at all — correct for a panel whose space is
    /// reclaimed when it is empty. `Some` draws the text instead, which is
    /// what a panel in a **fixed** slot needs: `ts570d` reserves three rows
    /// for errors unconditionally, and drawing nothing there leaves a
    /// bordered empty box that reads as a broken panel rather than a quiet
    /// one.
    pub quiet: Option<(&'static str, Style)>,
}

/// A bounded list of recent errors, most recent first.
///
/// Bounded at three because the panel is a fixed-height slot in both
/// consoles: an unbounded list would overflow it, and the *oldest* three
/// are the least useful ones to keep — which is why this reverses first.
pub fn error_panel(
    errors: &[String],
    title: &str,
    styles: ErrorPanelStyles,
    area: Rect,
    buf: &mut Buffer,
) {
    let lines: Vec<Line> = if errors.is_empty() {
        match styles.quiet {
            None => return,
            Some((text, style)) => vec![Line::from(Span::styled(text, style))],
        }
    } else {
        errors
            .iter()
            .rev()
            .take(3)
            .map(|e| Line::from(Span::styled(e.as_str(), styles.error)))
            .collect()
    };
    Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {title} ")),
        )
        .wrap(Wrap { trim: false })
        .render(area, buf);
}

/// The title bar.
///
/// Takes the title and its alignment rather than hardcoding either. The
/// title was the only thing that differed between the two apps' copies;
/// alignment is here because the first extraction fixed it to left and
/// `ts570d` centres.
pub fn header(title: &str, alignment: Alignment, area: Rect, buf: &mut Buffer, style: Style) {
    Paragraph::new(Line::from(Span::styled(title.to_string(), style)))
        .alignment(alignment)
        .block(Block::default().borders(Borders::ALL))
        .render(area, buf);
}

/// A two-column key/description list, as used by menu and help panels.
///
/// Generic over both cell types, which is the whole reason the two apps
/// could not share it: `ts570d` keys menus by `&'static str` and `ft991a`
/// by `char`, and `ts570d` needs a second copy for menus whose labels are
/// built at runtime and so are `String`. Neither is wrong, and none of
/// them needed changing — three concrete copies collapse into one
/// signature.
pub fn menu_column<K: std::fmt::Display, T: std::fmt::Display>(
    items: &[(K, T)],
    key_style: Style,
    text_style: Style,
) -> Vec<Line<'static>> {
    items
        .iter()
        .map(|(key, text)| {
            Line::from(vec![
                Span::styled(format!("[{key}] "), key_style),
                Span::styled(text.to_string(), text_style),
            ])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render<F: FnOnce(Rect, &mut Buffer)>(w: u16, h: u16, f: F) -> String {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        f(area, &mut buf);
        (0..h)
            .map(|y| (0..w).map(|x| buf.get(x, y).symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn connecting_does_not_look_like_a_failure() {
        // On first start, a red CONNECTION LOST is alarming and wrong.
        let out = render(60, 6, |a, b| {
            link_panel(LinkState::Connecting, &[], "Radio Status", None, a, b)
        });
        assert!(out.contains("Connecting"));
        assert!(!out.contains("LOST"));
    }

    #[test]
    fn a_lost_link_says_so_and_says_what_to_do() {
        let out = render(70, 10, |a, b| {
            link_panel(LinkState::Lost, &[], "Radio Status", None, a, b)
        });
        assert!(out.contains("CONNECTION LOST"));
        assert!(out.contains("Reconnect"));
        // ...and that it will recover on its own, so nobody restarts the
        // console unnecessarily.
        assert!(out.contains("recover"));
    }

    #[test]
    fn a_flood_of_errors_cannot_push_the_instructions_off_screen() {
        // A radio failing in a loop produces errors faster than anyone
        // reads them. The recovery text is the part that must survive.
        let errors: Vec<String> = (0..200).map(|i| format!("error {i}")).collect();
        let out = render(70, 20, |a, b| {
            link_panel(LinkState::Lost, &errors, "Radio Status", None, a, b)
        });
        assert!(out.contains("Reconnect"));
        assert!(out.contains("error 0"));
        assert!(!out.contains("error 9"), "the list must be bounded");
    }

    const QUIET: ErrorPanelStyles = ErrorPanelStyles {
        error: Style::new(),
        quiet: None,
    };

    #[test]
    fn the_disconnected_panel_still_names_the_one_key_that_works() {
        // This panel replaces the control panel outright, so if it does not
        // say how to quit, nothing on screen does. The first extraction
        // dropped the line that both apps had.
        let quit = Span::raw("[Q] Quit");
        for state in [LinkState::Connecting, LinkState::Lost] {
            let out = render(70, 14, |a, b| {
                link_panel(state, &[], "Radio Status", Some(quit.clone()), a, b)
            });
            assert!(out.contains("[Q] Quit"), "{state:?} lost the footer");
        }
    }

    #[test]
    fn a_flood_of_errors_cannot_push_the_footer_off_screen_either() {
        let errors: Vec<String> = (0..200).map(|i| format!("error {i}")).collect();
        let out = render(70, 20, |a, b| {
            link_panel(
                LinkState::Lost,
                &errors,
                "Radio Status",
                Some(Span::raw("[Q] Quit")),
                a,
                b,
            )
        });
        assert!(out.contains("[Q] Quit"));
    }

    #[test]
    fn a_centred_header_is_actually_centred() {
        let out = render(41, 3, |a, b| {
            header("TITLE", Alignment::Center, a, b, Style::default())
        });
        let row = out.lines().nth(1).unwrap();
        let left = row.find("TITLE").unwrap();
        let right = row.len() - (left + "TITLE".len());
        assert!(
            left.abs_diff(right) <= 1,
            "not centred: {left} left, {right} right"
        );
    }

    #[test]
    fn no_errors_means_no_panel_at_all() {
        // Not an empty bordered box announcing that nothing is wrong.
        let out = render(40, 5, |a, b| error_panel(&[], "Errors", QUIET, a, b));
        assert!(out.trim().is_empty());
    }

    #[test]
    fn a_panel_in_a_fixed_slot_can_say_it_is_quiet_instead() {
        // ts570d reserves the rows whether or not anything went wrong, so
        // drawing nothing there leaves a bordered void that reads as a
        // broken panel rather than a healthy one.
        let styles = ErrorPanelStyles {
            error: Style::new(),
            quiet: Some(("No errors", Style::new())),
        };
        let out = render(40, 5, |a, b| error_panel(&[], "Errors", styles, a, b));
        assert!(out.contains("No errors"), "got {out:?}");
    }

    #[test]
    fn the_error_panel_shows_the_most_recent_first() {
        let errors: Vec<String> = (0..10).map(|i| format!("e{i}")).collect();
        let out = render(40, 5, |a, b| error_panel(&errors, "Errors", QUIET, a, b));
        assert!(out.contains("e9"), "newest must be visible");
        assert!(!out.contains("e0"), "oldest is dropped, not the newest");
    }

    #[test]
    fn the_header_takes_its_title_rather_than_hardcoding_one() {
        // The only thing that differed between the two apps' copies.
        let out = render(40, 3, |a, b| {
            header(
                "TS-570D RADIO CONTROL",
                Alignment::Left,
                a,
                b,
                Style::default(),
            )
        });
        assert!(out.contains("TS-570D"));
    }

    #[test]
    fn menu_columns_work_with_either_apps_key_type() {
        // ts570d keys by &str, ft991a by char. Neither is wrong, and this
        // is the only reason the two could not share the function.
        let by_str = menu_column(
            &[("F1", "band"), ("F2", "mode")],
            Style::default(),
            Style::default(),
        );
        // ...and with labels built at runtime, which is a third concrete
        // copy `ts570d` was carrying for no reason but the text type.
        let owned = menu_column(
            &[(1u8, format!("memory {}", 14))],
            Style::default(),
            Style::default(),
        );
        let text: String = owned[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "[1] memory 14");
        let by_char = menu_column(
            &[('a', "band"), ('b', "mode")],
            Style::default(),
            Style::default(),
        );
        assert_eq!(by_str.len(), 2);
        assert_eq!(by_char.len(), 2);

        let text: String = by_char[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(text, "[a] band");
    }
}
