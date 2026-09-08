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

//! The console, laid out as the accepted design (option 3, "Workspace").
//!
//! `planning/designer/task_plan.md` iterations 2 and 3 are the
//! specification; this module is its terminal half. The GUI got this
//! structure first and the TUI did not, which
//! `docs/renderer-parity.md` recorded as ground (c) — three rows of it.
//! This closes them.
//!
//! # Three categories, and the layout is what says which is which
//!
//! The design's central claim is that a console has three kinds of thing in
//! it, and that putting them in the same place is what makes consoles
//! confusing:
//!
//! | category | where | test |
//! |---|---|---|
//! | monitoring | left rail | always true, never entered — meters, pending, AF |
//! | quick settings | control layer | changed mid-QSO without looking away |
//! | working surfaces | tabs | enter, do a thing, leave |
//! | reference | right rail | read-only session facts, never touched |
//!
//! So AF lives in the rail rather than in a fifth tab: an AF scope is
//! looked at *while* doing something else, and a tab you must leave to keep
//! operating cannot serve that. And a SPECTRUM tab sitting next to an AUDIO
//! tab would invite reading them as two views of one thing, which they are
//! emphatically not.
//!
//! # The tab bar is a statement about the radio
//!
//! It is derived, not fixed: [`cat_ui::workspace::tabs`] builds it from the
//! capability document, so a radio with no memory has no MEMORY tab and the
//! numbers in the labels are the radio's own. The digits that select tabs
//! follow the same list, so `1` is whatever is actually first. The GUI
//! calls the same function — that shared call is what makes ADR 0013's
//! parity rule cheap to keep rather than a thing to police.
//!
//! # Geometry
//!
//! Built for 120x40, degrading rather than breaking below it:
//!
//! ```text
//! +----------------------+--------------------------------+------------+
//! | left rail        22c | content pane               72c | right  26c |
//! |                      | tab bar                        |            |
//! | meters               | big readout (pending grammar)  | session    |
//! | CAT pending          | quick settings: BAND / MODE    | facts,     |
//! | AF SCOPE  (braille)  |                 ribbon x8      | read-only  |
//! | AF FFT    (20 bins)  | tab content                    |            |
//! +----------------------+--------------------------------+------------+
//! | status strip                                                        |
//! | : command line                                                      |
//! +---------------------------------------------------------------------+
//! ```

use crate::af::{self, AudioState, Passband};
use crate::devices::{self, PickerStyles, Selection};
use crate::meter::{meter_rail, MeterStyles};
use crate::vfo::vfo_readout;
use cat_framework::capabilities::MeterKind;
use cat_signal::DeviceList;
use cat_signal::{AudioScopeFrame, AudioSpectrumFrame, SpectrumFrame};
use cat_ui::meter::MeterReading;
use cat_ui::workspace::{Tab, TabEntry};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use cat_ui::display::RadioDisplay;

/// The monitoring rail's width. 22 rather than 18 because the AF FFT needs
/// 20 cells to be worth drawing at 150 Hz each, plus a column either side.
pub const RAIL_W: u16 = 22;
/// The reference rail's width.
pub const REF_W: u16 = 26;

/// Ink for the AF domain. The RF domain is in colour; AF is not, and amber
/// means "a radio setting" in both.
const AF_INK: Color = Color::White;
const SETTING: Color = Color::Yellow;
const DIM: Color = Color::DarkGray;
const RF_TRACE: Color = Color::Green;

/// What the console itself owns, as against what the radio reports.
///
/// Kept apart from [`RadioDisplay`] on purpose: everything here is the
/// operator's position in the console — which tab they are on, what they
/// have typed — and none of it should be overwritten when a poll cycle
/// lands. A single struct carrying both would make a radio update capable
/// of moving the cursor.
#[derive(Debug, Clone)]
pub struct ConsoleView {
    /// The tabs this radio has, derived once from its capabilities.
    pub tabs: Vec<TabEntry>,
    /// Which one is open.
    pub tab: Tab,
    /// `Some` while the `:` line is open, holding what has been typed.
    pub command: Option<String>,
    /// A frequency asked for but not yet confirmed. The confirmed value
    /// stays put and this follows it — see [`vfo_readout`].
    pub pending_vfo_hz: Option<u64>,
    /// How many CAT requests are in flight, for the rail's pending row.
    pub cat_pending: usize,
    /// Newest first. Empty until a spectrum source is attached.
    pub spectrum: Vec<SpectrumFrame>,
    pub audio: AudioState,
    pub af_scope: Option<AudioScopeFrame>,
    pub af_spectrum: Option<AudioSpectrumFrame>,
    /// The receive passband, for the AF FFT's amber edges.
    pub passband: Option<Passband>,
    /// The last thing the console did or failed to do.
    pub message: Option<String>,
    /// What this machine can see, for the SOURCE tab's picker.
    pub devices: Vec<DeviceList>,
    /// Where the picker's cursor is.
    pub device_selection: Selection,
}

impl Default for ConsoleView {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            tab: Tab::Spectrum,
            command: None,
            pending_vfo_hz: None,
            cat_pending: 0,
            spectrum: Vec::new(),
            // The design's default, and the honest one: this station has an
            // audio path wired and no client transport attached to it yet.
            audio: AudioState::Configured,
            af_scope: None,
            af_spectrum: None,
            passband: None,
            message: None,
            devices: Vec::new(),
            device_selection: Selection::new(),
        }
    }
}

impl ConsoleView {
    /// The console for a radio that says this about itself.
    pub fn for_capabilities(caps: &cat_native::CapabilitiesWire) -> Self {
        let tabs = cat_ui::workspace::tabs(caps);
        let tab = tabs.first().map(|t| t.tab).unwrap_or(Tab::Source);
        Self {
            tabs,
            tab,
            ..Self::default()
        }
    }
}

/// Split the screen into the design's regions.
///
/// Returned rather than drawn so the geometry can be asserted on without a
/// terminal, and so every panel is positioned in one place instead of each
/// one measuring for itself.
pub struct Regions {
    pub rail: Rect,
    pub content: Rect,
    pub reference: Rect,
    pub status: Rect,
    pub command: Rect,
}

pub fn split(area: Rect) -> Regions {
    // Two full-width rows are taken off the bottom first: the status strip
    // and the command line are the console's, not any panel's, and a layout
    // that let a tab claim the bottom row would put the command line
    // somewhere different on each tab.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    // Rails are fixed-width and the content pane takes what is left, rather
    // than percentages: the AF FFT is 20 cells because 150 Hz per cell is
    // the resolution it was designed at, and a percentage would make that
    // depend on the terminal.
    let rail_w = RAIL_W.min(rows[0].width);
    let ref_w = REF_W.min(rows[0].width.saturating_sub(rail_w));
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(rail_w),
            Constraint::Min(1),
            Constraint::Length(ref_w),
        ])
        .split(rows[0]);

    Regions {
        rail: cols[0],
        content: cols[1],
        reference: cols[2],
        status: rows[1],
        command: rows[2],
    }
}

/// Draw the whole console, and return the region a tab's body occupies.
///
/// The caller gets that rectangle back because the accepted design does not
/// place two things this console has: the TS-570D's own feature menus
/// (`[F]` frequency, `[M]` mode/DSP, `[C]` CW …) and the safety-gated
/// screens behind `[D]` and `[P]`. The design was drawn for the console —
/// dial, mode, spectrum, memory, menu, source — and those are the whole of
/// its tab bar.
///
/// Rather than drop what it does not mention, or invent a fifth tab it
/// rejected, those screens **overlay the tab body**: the console is the
/// resting state, and a menu is something you are temporarily in. Nothing
/// that was reachable before stops being reachable, which is the bar a
/// layout change has to clear before it is an improvement.
pub fn draw(
    f: &mut Frame,
    area: Rect,
    radio: &RadioDisplay,
    view: &ConsoleView,
    caps: &cat_native::CapabilitiesWire,
) -> Rect {
    // Keyed state first, and across the full width. It used to be one
    // styled word in a row of other words -- reported from the bench as
    // not obvious enough, and that is the right complaint: a transmitting
    // radio is not a field on a form, it is the single fact that changes
    // what every other control on the screen will do. A whole row costs
    // one line of a panel that has plenty and cannot be confused with
    // anything else on the screen.
    let area = match tx_banner(f, area, radio) {
        Some(rest) => rest,
        None => area,
    };
    match &caps.layout {
        Some(spec) => draw_layout(f, area, radio, view, caps, spec),
        // No layout published. An older server has not declined one, it
        // has never been asked, so this falls back to the arrangement the
        // console has always had rather than to an empty screen.
        None => {
            let r = split(area);
            draw_rail(f, r.rail, radio, view, caps);
            let body = draw_content(f, r.content, radio, view, caps);
            draw_reference(f, r.reference, radio);
            draw_status(f, r.status, radio, view);
            draw_command(f, r.command, view);
            body
        }
    }
}

/// A full-width bar while the radio is keyed, and nothing when it is not.
///
/// Returns the area left for everything else, or `None` when there is no
/// bar to draw. Deliberately takes a row rather than overlaying: an
/// overlay hides whatever is under it, and the thing under it during a
/// transmission is usually the meters an operator is transmitting in
/// order to watch.
fn tx_banner(f: &mut Frame, area: Rect, radio: &RadioDisplay) -> Option<Rect> {
    if !radio.tx || area.height < 3 {
        return None;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);
    let width = rows[0].width as usize;
    let label = "  ◆ ◆ ◆   T R A N S M I T T I N G   ◆ ◆ ◆  ";
    // Centred by padding rather than by an alignment, so the red runs the
    // whole width instead of only under the text.
    let pad = width.saturating_sub(label.chars().count()) / 2;
    let banner = format!(
        "{:pad$}{label}{:>rest$}",
        "",
        "",
        pad = pad,
        rest = width.saturating_sub(pad + label.chars().count())
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            banner,
            Style::default()
                .bg(Color::Red)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ))),
        rows[0],
    );
    Some(rows[1])
}

/// Draw the arrangement the radio's own server asked for.
///
/// The panels are this crate's and the arrangement is the radio's. That
/// split is the point: a new radio composes a console out of parts that
/// already work, and a renderer never has to learn what a TS-570D is.
///
/// Returns the workspace's area, which is what the app overlays its own
/// screens into. A layout with no workspace gets the whole area back —
/// somewhere is better than nowhere for a modal that has to go up.
fn draw_layout(
    f: &mut Frame,
    area: Rect,
    radio: &RadioDisplay,
    view: &ConsoleView,
    caps: &cat_native::CapabilitiesWire,
    spec: &cat_layout::LayoutSpec,
) -> Rect {
    use cat_layout::PanelKind;

    // What the layout draws elsewhere, the quick bar must not repeat.
    let quick_rows = QuickRows {
        band: !spec.root.places(&PanelKind::BandBar),
        mode: !spec.root.places(&PanelKind::ModeBar),
    };

    let placements = spec.resolve(to_area(area));

    // A layout that places a Spectrum panel AND a Workspace draws the
    // spectrum twice while the SPECTRUM tab is selected, because that
    // tab's workspace content *is* the spectrum. Two identical waterfalls,
    // each half the height they could be.
    //
    // Rather than blank the workspace -- which would leave a hole where
    // the operator is looking -- the spectrum takes both. Same principle
    // as `quick_rows` above: what the layout draws elsewhere must not be
    // repeated, and here the panel and the tab are the same picture.
    let spectrum_rect = placements
        .iter()
        .find(|p| p.kind == PanelKind::Spectrum)
        .map(|p| to_rect(p.area));
    let workspace_rect = placements
        .iter()
        .find(|p| p.kind == PanelKind::Workspace)
        .map(|p| to_rect(p.area));
    let merged = match (view.tab == Tab::Spectrum, spectrum_rect, workspace_rect) {
        (true, Some(sp), Some(ws)) => Some(union(sp, ws)),
        _ => None,
    };

    let mut body = area;
    for placement in placements {
        let r = to_rect(placement.area);
        match placement.kind {
            PanelKind::Readout => {
                // The tab bar rides with the readout: they are the two
                // rows an operator reads as one, and a layout that
                // separated them would be describing a console nobody
                // asked for.
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(1)])
                    .split(r);
                draw_tab_bar(f, rows[0], view);
                draw_readout(f, rows[1], radio, view);
            }
            PanelKind::QuickBar => draw_quick_settings(f, r, radio, caps, quick_rows),
            PanelKind::MeterRail => draw_meters(f, r, radio, caps),
            PanelKind::LevelsRail => draw_reference(f, r, radio),
            // When merged, the spectrum is drawn once over both rects.
            PanelKind::Spectrum => draw_spectrum(f, merged.unwrap_or(r), view),
            PanelKind::AfScope => draw_af_scope(f, r, view),
            PanelKind::AfFft => draw_af_fft(f, r, view),
            PanelKind::Workspace => {
                // Already covered by the merged spectrum above.
                if merged.is_none() {
                    draw_tab_content(f, r, radio, view);
                }
                body = r;
            }
            PanelKind::Status => draw_status(f, r, radio, view),
            PanelKind::CommandLine => draw_command(f, r, view),
            // Skipped rather than approximated: drawing something else
            // where a panel was asked for is worse than a gap, because a
            // gap is visibly a gap.
            // Placed by a layout this build does not know how to draw, or
            // deliberately left empty. `PanelKind` is `#[non_exhaustive]`,
            // so a panel added upstream reaches an older console as a gap
            // rather than as a compile error or a wrong drawing.
            PanelKind::BandBar => draw_band_bar(f, r, radio, caps),
            PanelKind::ModeBar => draw_mode_bar(f, r, radio, caps),
            PanelKind::Blank => {}
            _ => {}
        }
    }
    body
}

/// The smallest rect containing both.
///
/// Used only for adjacent panels a layout stacked, so this is a merge
/// rather than an approximation.
fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let right = (a.x + a.width).max(b.x + b.width);
    let bottom = (a.y + a.height).max(b.y + b.height);
    Rect::new(x, y, right - x, bottom - y)
}

fn to_area(r: Rect) -> cat_layout::Area {
    cat_layout::Area::new(r.x, r.y, r.width, r.height)
}

fn to_rect(a: cat_layout::Area) -> Rect {
    Rect::new(a.x, a.y, a.width, a.height)
}

// ── the monitoring rail ─────────────────────────────────────────────────

fn draw_rail(
    f: &mut Frame,
    area: Rect,
    radio: &RadioDisplay,
    view: &ConsoleView,
    caps: &cat_native::CapabilitiesWire,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    // The rail keeps a one-column margin of its own rather than the layout
    // inserting a gutter: 22 / 72 / 26 is the design's split and it uses
    // the full width, so a gutter would have to come out of the content
    // pane. A full-scale meter bar running into the tab bar is the thing
    // being avoided, and the rail is where the fix belongs.
    let area = Rect {
        width: area.width.saturating_sub(1),
        ..area
    };
    if area.width == 0 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // meters
            Constraint::Length(1), // CAT pending
            Constraint::Length(1), // AF SCOPE header
            Constraint::Length(5), // AF SCOPE
            Constraint::Length(1), // AF FFT header
            Constraint::Length(5), // AF FFT
            Constraint::Min(0),
        ])
        .split(area);

    // A TX meter during receive keeps its row, dimmed. Reflowing the rail
    // on every transmit would make the panel jump.
    // The reading picks its range and its S-unit table up from the radio's
    // own declaration rather than from literals here, so the bar and the
    // `SM` command cannot end up describing different meters.
    // The scale comes from the radio in front of this console, not from
    // one radio named in the source: a TS-570D reports 0-30 against an
    // S-unit table, an FT-991A reports 0-255 with no calibration the
    // manual gives, and a bar drawn to the wrong one is wrong in a way
    // that looks entirely plausible.
    let s = MeterReading::from_wire(&caps.meters, MeterKind::S, radio.smeter);
    let meters: Vec<(&str, Option<MeterReading>, bool)> = vec![
        ("S", s, !radio.tx),
        ("PO", None, radio.tx),
        ("SWR", None, radio.tx),
        ("ALC", None, radio.tx),
    ];
    meter_rail(
        &meters,
        rows[0],
        f.buffer_mut(),
        4,
        MeterStyles {
            active: Style::default().fg(Color::White),
            inactive: Style::default().fg(DIM),
            fill: Color::Green,
            empty: DIM,
        },
    );

    let pending = if view.cat_pending == 0 {
        Line::from(Span::styled("CAT idle", Style::default().fg(DIM)))
    } else {
        Line::from(Span::styled(
            format!("CAT BUSY {}", view.cat_pending),
            Style::default().fg(SETTING).add_modifier(Modifier::BOLD),
        ))
    };
    f.render_widget(Paragraph::new(pending), rows[1]);

    f.render_widget(
        Paragraph::new(af::af_header("AF SCOPE", view.audio, AF_INK, DIM)),
        rows[2],
    );
    af::af_scope(
        view.af_scope.as_ref(),
        view.audio,
        rows[3],
        f.buffer_mut(),
        AF_INK,
    );
    f.render_widget(
        Paragraph::new(af::af_header("AF FFT", view.audio, AF_INK, DIM)),
        rows[4],
    );
    af::af_fft(
        view.af_spectrum.as_ref(),
        view.passband,
        view.audio,
        rows[5],
        f.buffer_mut(),
        AF_INK,
        SETTING,
    );
}

// ── the content pane ────────────────────────────────────────────────────

/// The meters, alone.
///
/// `draw_rail` draws these plus the AF panels plus the CAT-busy line,
/// because that is the one arrangement this console used to have. A
/// server-authored layout can put them anywhere, so each is also a panel
/// in its own right.
/// The meter bars themselves.
///
/// Split out of `draw_rail` so a server-authored layout can place the
/// meters somewhere the original arrangement never put them.
fn draw_meter_bars(
    f: &mut Frame,
    area: Rect,
    radio: &RadioDisplay,
    caps: &cat_native::CapabilitiesWire,
) {
    // A TX meter during receive keeps its row, dimmed. Reflowing the rail
    // on every transmit would make the panel jump.
    // The reading picks its range and its S-unit table up from the radio's
    // own declaration rather than from literals here, so the bar and the
    // `SM` command cannot end up describing different meters.
    // The scale comes from the radio in front of this console, not from
    // one radio named in the source: a TS-570D reports 0-30 against an
    // S-unit table, an FT-991A reports 0-255 with no calibration the
    // manual gives, and a bar drawn to the wrong one is wrong in a way
    // that looks entirely plausible.
    let s = MeterReading::from_wire(&caps.meters, MeterKind::S, radio.smeter);
    let meters: Vec<(&str, Option<MeterReading>, bool)> = vec![
        ("S", s, !radio.tx),
        ("PO", None, radio.tx),
        ("SWR", None, radio.tx),
        ("ALC", None, radio.tx),
    ];
    meter_rail(
        &meters,
        area,
        f.buffer_mut(),
        4,
        MeterStyles {
            active: Style::default().fg(Color::White),
            inactive: Style::default().fg(DIM),
            fill: Color::Green,
            empty: DIM,
        },
    );
}

/// The bars this console draws: one row per meter.
///
/// Fixed, not `Min`. `Min(1)` let the bars absorb every spare row in the
/// column -- fifteen of them on the TS-570D's layout at 120x40 -- so four
/// rows of meters were drawn at the top, ten rows of nothing followed,
/// and the `CAT idle` line sat alone at the bottom, fifteen rows from the
/// meters it describes.
///
/// Deliberately *not* derived from `cat_layout::METER_RAIL_ROWS`, which
/// is sized for the GPU console's roomier rail. This console is the
/// tighter of the two and simply leaves the surplus blank beneath its
/// content, rather than spreading four meters over ten rows.
const METER_ROWS: u16 = 4;

// The two halves have to agree and are written in different crates: the
// layout sizes this panel, this file fills it. Checked at compile time
// rather than in a test, because a layout that cannot hold its renderer
// is not a failing case to report -- it is a build that should not
// happen. When they last disagreed the rail took fifteen rows to draw
// five, and nothing anywhere said so.
const _: () = assert!(
    METER_ROWS < cat_layout::METER_RAIL_ROWS,
    "the meter bars plus the link line must fit the rows the layout allots"
);

fn draw_meters(
    f: &mut Frame,
    area: Rect,
    radio: &RadioDisplay,
    caps: &cat_native::CapabilitiesWire,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    // The same one-column margin `draw_rail` keeps, and for the same
    // reason: 22 / 72 / 26 is the design's split and it uses the full
    // width, so a gutter cannot come out of the content pane. Without it
    // a full-scale meter bar runs straight into whatever the layout put
    // in the next column -- on this radio the S-meter's own "S8" ended up
    // touching the spectrum panel's first character, reading as `S8NO
    // SPECTRUM SOURCE`.
    let area = Rect {
        width: area.width.saturating_sub(1),
        ..area
    };
    if area.width == 0 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        // The link line sits directly beneath the bars, and any surplus
        // falls below both. It used to be `Min(1)` then `Length(1)`,
        // which pinned the link line to the *bottom* of whatever the
        // layout allotted -- ten rows adrift from the meters it is
        // reporting on.
        .constraints([
            Constraint::Length(METER_ROWS),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(area);
    draw_meter_bars(f, rows[0], radio, caps);
    let pending = if radio.connected {
        Line::from(Span::styled("CAT idle", Style::default().fg(DIM)))
    } else {
        Line::from(Span::styled("LINK LOST", Style::default().fg(Color::Red)))
    };
    f.render_widget(Paragraph::new(pending), rows[1]);
}

/// The AF scope, with its header.
fn draw_af_scope(f: &mut Frame, area: Rect, view: &ConsoleView) {
    if area.height < 2 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);
    f.render_widget(
        Paragraph::new(af::af_header("AF SCOPE", view.audio, AF_INK, DIM)),
        rows[0],
    );
    af::af_scope(
        view.af_scope.as_ref(),
        view.audio,
        rows[1],
        f.buffer_mut(),
        AF_INK,
    );
}

/// The AF FFT, with its header and the filter edges.
fn draw_af_fft(f: &mut Frame, area: Rect, view: &ConsoleView) {
    if area.height < 2 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);
    f.render_widget(
        Paragraph::new(af::af_header("AF FFT", view.audio, AF_INK, DIM)),
        rows[0],
    );
    af::af_fft(
        view.af_spectrum.as_ref(),
        view.passband,
        view.audio,
        rows[1],
        f.buffer_mut(),
        AF_INK,
        SETTING,
    );
}

fn draw_content(
    f: &mut Frame,
    area: Rect,
    radio: &RadioDisplay,
    view: &ConsoleView,
    caps: &cat_native::CapabilitiesWire,
) -> Rect {
    if area.width == 0 || area.height == 0 {
        return area;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // tab bar
            Constraint::Length(4), // readout: three-row dial + status
            Constraint::Length(6), // quick settings: BAND, MODE, 2x ribbon
            Constraint::Min(0),    // tab content
        ])
        .split(area);

    draw_tab_bar(f, rows[0], view);
    draw_readout(f, rows[1], radio, view);
    draw_quick_settings(f, rows[2], radio, caps, QuickRows::default());
    draw_tab_content(f, rows[3], radio, view);
    rows[3]
}

fn draw_tab_bar(f: &mut Frame, area: Rect, view: &ConsoleView) {
    let mut spans = Vec::new();
    for (i, entry) in view.tabs.iter().enumerate() {
        let selected = entry.tab == view.tab;
        let style = if selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(DIM)
        };
        // The digit is part of the label, because it is how the tab is
        // reached and a terminal has no tab to click.
        spans.push(Span::styled(format!(" {} {} ", i + 1, entry.label), style));
        spans.push(Span::raw(" "));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_readout(f: &mut Frame, area: Rect, radio: &RadioDisplay, view: &ConsoleView) {
    // Transmitting is painted across the whole readout, not tucked into a
    // two-character label between MODE and VFO. An operator has to be able
    // to tell at a glance, from across the room, whether the radio is on
    // the air -- it is the one piece of state with consequences outside
    // the room.
    if radio.tx {
        let banner = Style::default()
            .bg(Color::Red)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD);
        for y in area.y..area.y + area.height {
            f.buffer_mut()
                .set_string(area.x, y, " ".repeat(area.width as usize), banner);
        }
    }

    let big = area.height >= (crate::bigdigits::HEIGHT as u16 + 1);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if big {
            [
                Constraint::Length(crate::bigdigits::HEIGHT as u16),
                Constraint::Length(1),
            ]
        } else {
            [Constraint::Length(1), Constraint::Length(1)]
        })
        .split(area);

    let hz = if radio.connected {
        Some(radio.vfo_a_hz)
    } else {
        // Not zero, and not the last value pretending to be current.
        None
    };

    let confirmed = if radio.tx {
        Style::default()
            .bg(Color::Red)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    };

    if let (true, Some(hz), None) = (big, hz, view.pending_vfo_hz) {
        // Three rows, so the dial is the largest thing on the panel --
        // which is where every physical radio puts it.
        let text = cat_ui::format::format_hz(hz);
        let numeric: String = text
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let glyphs = crate::bigdigits::render(&numeric);
        for (i, row) in glyphs.iter().enumerate() {
            let y = rows[0].y + i as u16;
            if y < rows[0].y + rows[0].height {
                f.buffer_mut().set_string(rows[0].x, y, row, confirmed);
            }
        }
        // The unit stays small: it never changes, so it does not need the
        // space, and giving it the space would crowd the digits.
        let unit_x = rows[0].x + crate::bigdigits::width(&numeric) as u16 + 1;
        if unit_x < rows[0].x + rows[0].width {
            f.buffer_mut().set_string(
                unit_x,
                rows[0].y + crate::bigdigits::HEIGHT as u16 - 1,
                "MHz",
                if radio.tx {
                    confirmed
                } else {
                    Style::default().fg(DIM)
                },
            );
        }
    } else {
        f.render_widget(
            Paragraph::new(vfo_readout(
                hz,
                view.pending_vfo_hz,
                confirmed,
                Style::default().fg(Color::Cyan),
                Style::default().fg(DIM),
            )),
            rows[0],
        );
    }

    let vfo = if radio.split { "SPLIT" } else { "VFO A" };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(radio.mode.clone(), Style::default().fg(SETTING)),
            Span::raw("   "),
            Span::styled(vfo, Style::default().fg(Color::White)),
            Span::raw("   "),
            Span::styled(
                if radio.tx {
                    "◆ TRANSMITTING ◆"
                } else {
                    "RX"
                },
                if radio.tx {
                    Style::default()
                        .bg(Color::Red)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD | Modifier::RAPID_BLINK)
                } else {
                    Style::default().fg(Color::Green)
                },
            ),
        ])),
        rows[1],
    );
}

/// The quick-settings control layer.
///
/// The admission test the design applies is: *would you change it mid-QSO
/// without wanting to look away from the waterfall?* BAND earns a full row
/// because it is a twelve-way pick and stepping through it one click at a
/// time is the wrong affordance; MODE earns one because it is the most
/// changed control on the radio.
///
/// What is deliberately **not** here: MEMORY, MENU and SOURCE are
/// destinations and are tabs; AGC, NB and NR are menu items 09/10/11 with
/// no capability field to derive them from, and putting them here would
/// mean inventing a capability. A ribbon that grows to hold everything is
/// just the reference rail again.
/// The band buttons, as a panel a layout can place on its own.
fn draw_band_bar(
    f: &mut Frame,
    area: Rect,
    radio: &RadioDisplay,
    caps: &cat_native::CapabilitiesWire,
) {
    if area.height == 0 {
        return;
    }
    f.render_widget(Paragraph::new(band_row(radio.vfo_a_hz, caps)), area);
}

/// The mode buttons, likewise.
///
/// Wrapped, because a radio with fourteen modes does not fit on one row of
/// a rail -- and a layout that puts this in a rail is asking for exactly
/// that.
fn draw_mode_bar(
    f: &mut Frame,
    area: Rect,
    radio: &RadioDisplay,
    caps: &cat_native::CapabilitiesWire,
) {
    if area.height == 0 {
        return;
    }
    f.render_widget(
        Paragraph::new(mode_row(&radio.mode, caps)).wrap(ratatui::widgets::Wrap { trim: false }),
        area,
    );
}

/// Which rows the quick bar should draw itself.
///
/// A layout that places `BandBar` or `ModeBar` as panels of their own has
/// already drawn them; the quick bar drawing them again puts two identical
/// band rows on screen, which reads as a rendering fault rather than as a
/// choice.
#[derive(Debug, Clone, Copy)]
struct QuickRows {
    band: bool,
    mode: bool,
}

impl Default for QuickRows {
    /// Both, which is what the arrangement this console shipped with
    /// expects.
    fn default() -> Self {
        Self {
            band: true,
            mode: true,
        }
    }
}

fn draw_quick_settings(
    f: &mut Frame,
    area: Rect,
    radio: &RadioDisplay,
    caps: &cat_native::CapabilitiesWire,
    rows_wanted: QuickRows,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1); 6])
        .split(area);

    if rows_wanted.band {
        f.render_widget(Paragraph::new(band_row(radio.vfo_a_hz, caps)), rows[0]);
    }
    if rows_wanted.mode {
        f.render_widget(Paragraph::new(mode_row(&radio.mode, caps)), rows[1]);
    }

    // Two rows of four cells, each cell label-above-value, so one glance
    // reads the labels and a second reads the values. Eight cells and no
    // more: a ribbon that grows to hold everything is just the reference
    // rail again.
    let cells = ribbon_cells(radio);
    for (i, chunk) in cells.chunks(4).enumerate() {
        f.render_widget(Paragraph::new(ribbon_line(chunk, true)), rows[2 + i * 2]);
        f.render_widget(Paragraph::new(ribbon_line(chunk, false)), rows[3 + i * 2]);
    }
}

/// The HF bands this radio covers, as a pick.
/// The bands this radio reaches, from what it published.
///
/// Derived, not a fixed list. The nine HF bands written out here before
/// were a TS-570D's: an IC-7100 also works 6 m, 2 m and 70 cm, and an
/// operator offered only HF on a radio that reaches 430 MHz would think
/// their console was broken. The GUI had always derived this; the terminal
/// console had not, which is how one renderer ends up quietly wrong.
fn bands_for(caps: &cat_native::CapabilitiesWire) -> Vec<&'static cat_ui::band::Band> {
    cat_ui::band::BANDS
        .iter()
        .filter(|b| caps.rx_range.contains(b.range.min_hz))
        .collect()
}

fn band_row(hz: u64, caps: &cat_native::CapabilitiesWire) -> Line<'static> {
    let mut spans = vec![Span::styled("BAND ", Style::default().fg(DIM))];
    for band in bands_for(caps) {
        let (name, lo, hi) = (band.label, band.range.min_hz, band.range.max_hz);
        let here = hz >= lo && hz <= hi;
        spans.push(Span::styled(
            format!(" {name} "),
            if here {
                Style::default()
                    .fg(Color::Black)
                    .bg(SETTING)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            },
        ));
    }
    Line::from(spans)
}

/// The modes this radio has, in its own words.
///
/// Also derived. The six written out here before were a TS-570D's; an
/// FT-991A has fourteen and an IC-7100 has ten including DV, and a console
/// that offered six would be hiding most of the radio.
fn mode_row(current: &str, caps: &cat_native::CapabilitiesWire) -> Line<'static> {
    let mut spans = vec![Span::styled("MODE ", Style::default().fg(DIM))];
    for descriptor in &caps.modes {
        let m = descriptor.label.as_str();
        let here = m == current;
        spans.push(Span::styled(
            format!(" {m} "),
            if here {
                Style::default()
                    .fg(Color::Black)
                    .bg(SETTING)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            },
        ));
    }
    Line::from(spans)
}

/// The ribbon: label above current value.
///
/// RIT and XIT are present showing `n/a` on a radio that lacks them, rather
/// than absent. The standing discipline is that an unavailable capability
/// shows *where it would be*, dimmed — the same rule as the absent spectrum
/// and the absent audio stream. Two dead cells is what a layout costs when
/// it refuses to reshape itself between radios.
fn ribbon_cells(radio: &RadioDisplay) -> Vec<(&'static str, String, bool)> {
    vec![
        (
            "VFO",
            if radio.rx_vfo == 0 { "A" } else { "B" }.into(),
            true,
        ),
        ("SPLIT", on_off(radio.split), true),
        (
            "STEP",
            if radio.fine_step { "fine" } else { "10 Hz" }.into(),
            true,
        ),
        known_or_dash("FILTER", radio.filter_width_hz.map(|hz| format!("{hz} Hz"))),
        known_or_dash("SHIFT", radio.if_shift_hz.map(|hz| format!("{hz:+} Hz"))),
        known_or_dash("NOTCH", radio.notch.map(on_off)),
        (
            "RIT",
            offset_or_off(radio.rit, radio.rit_xit_offset_hz),
            true,
        ),
        (
            "XIT",
            offset_or_off(radio.xit, radio.rit_xit_offset_hz),
            true,
        ),
    ]
}

/// A cell whose value the radio has told us, or a dimmed placeholder.
///
/// The placeholder is a real cell in a real position, not a gap. An
/// unavailable value shows *where it would be* — the same rule as the
/// absent spectrum and the absent audio stream.
fn known_or_dash(label: &'static str, value: Option<String>) -> (&'static str, String, bool) {
    match value {
        Some(v) => (label, v, true),
        None => (label, "—".to_string(), false),
    }
}

fn on_off(v: bool) -> String {
    if v { "on" } else { "off" }.to_string()
}

fn offset_or_off(active: bool, hz: i32) -> String {
    if active {
        format!("{hz:+}")
    } else {
        "off".to_string()
    }
}

const CELL_W: usize = 9;

fn ribbon_line(cells: &[(&'static str, String, bool)], labels: bool) -> Line<'static> {
    let mut spans = Vec::new();
    for (label, value, known) in cells {
        let text = if labels {
            (*label).to_string()
        } else {
            value.clone()
        };
        let style = if labels {
            Style::default().fg(DIM)
        } else if *known {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(DIM)
        };
        spans.push(Span::styled(format!("{text:<CELL_W$}"), style));
    }
    Line::from(spans)
}

fn draw_tab_content(f: &mut Frame, area: Rect, radio: &RadioDisplay, view: &ConsoleView) {
    match view.tab {
        Tab::Spectrum => draw_spectrum(f, area, view),
        Tab::Memory => draw_memory(f, area, radio, view),
        Tab::Menu => draw_menu(f, area, view),
        Tab::Source => draw_source(f, area, view),
    }
}

/// The MEMORY workspace.
///
/// A tab is a working surface — enter it, do a thing, leave — so this shows
/// the channel the radio is on and how to move, and does not try to be a
/// hundred-row table the console cannot fill. **The radio reports one
/// channel number and nothing else about the others**: `MC` recalls a
/// channel and `MR` reads one, but this console polls neither per channel,
/// so a grid of a hundred rows would be a hundred blanks. What is shown is
/// what is known.
fn draw_memory(f: &mut Frame, area: Rect, radio: &RadioDisplay, view: &ConsoleView) {
    let range = view
        .tabs
        .iter()
        .find(|t| t.tab == Tab::Memory)
        .map(|t| t.label.clone())
        .unwrap_or_else(|| "MEMORY".to_string());

    let mut lines = vec![
        Line::from(vec![
            Span::styled("channel  ", Style::default().fg(DIM)),
            Span::styled(
                format!("{:02}", radio.memory_channel),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(
                if radio.memory_mode { "MEM" } else { "VFO" },
                Style::default().fg(if radio.memory_mode { SETTING } else { DIM }),
            ),
        ]),
        Line::from(Span::styled(range, Style::default().fg(DIM))),
        Line::raw(""),
    ];
    lines.extend(crate::session::menu_column(
        &[
            (":mem <n>", "recall a channel"),
            ("[N]", "the memory menu: read, write, clear"),
        ],
        Style::default().fg(SETTING),
        Style::default().fg(Color::White),
    ));
    f.render_widget(Paragraph::new(lines), area);
}

/// The MENU workspace.
///
/// The TS-570D's fifty-two extension menu items. This console does not poll
/// them — `EX` is one round trip per item and polling fifty-two of them
/// every cycle would swamp a 9600-baud link — so the tab says how many
/// there are and how to reach one, rather than drawing fifty-two rows of
/// em-dashes. `[S]` still opens the menu screens that read and write them.
fn draw_menu(f: &mut Frame, area: Rect, view: &ConsoleView) {
    let count = view
        .tabs
        .iter()
        .find(|t| t.tab == Tab::Menu)
        .map(|t| t.label.clone())
        .unwrap_or_else(|| "MENU".to_string());

    let mut lines = vec![
        Line::from(Span::styled(count, Style::default().fg(Color::White))),
        Line::from(Span::styled(
            "not polled: EX is one round trip per item",
            Style::default().fg(DIM),
        )),
        Line::raw(""),
    ];
    lines.extend(crate::session::menu_column(
        &[("[S]", "system menu: read and write an item")],
        Style::default().fg(SETTING),
        Style::default().fg(Color::White),
    ));
    f.render_widget(Paragraph::new(lines), area);
}

/// The SOURCE workspace: what is actually plugged into this radio.
///
/// The one tab that is about the **installation** rather than the model,
/// which is why it is always present even on a radio with nothing attached
/// — a station with nothing wired still needs somewhere for that to be
/// said. Each attached source renders through the one generic
/// `SettingDescriptor` list, so a new source type needs no new panel.
fn draw_source(f: &mut Frame, area: Rect, view: &ConsoleView) {
    let mut lines = Vec::new();

    lines.push(Line::from(Span::styled(
        "ATTACHED",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));

    lines.push(match view.spectrum.first() {
        Some(frame) => Line::from(vec![
            Span::styled("  IF TAP    ", Style::default().fg(DIM)),
            Span::styled(
                format!(
                    "{:.3} MHz span {} kHz, {} bins",
                    frame.center_hz as f64 / 1e6,
                    frame.span_hz / 1000,
                    frame.bins.len()
                ),
                Style::default().fg(Color::Green),
            ),
        ]),
        None => Line::from(vec![
            Span::styled("  IF TAP    ", Style::default().fg(DIM)),
            Span::styled("nothing attached", Style::default().fg(DIM)),
        ]),
    });

    let audio = match view.audio {
        AudioState::Streaming => Span::styled(
            view.af_scope
                .as_ref()
                .map(|s| format!("{} Hz, {:.0} ms window", s.sample_rate_hz, s.window_ms()))
                .unwrap_or_else(|| "streaming".to_string()),
            Style::default().fg(Color::Green),
        ),
        // The distinction the design insists on: an audio path that exists
        // and is not streaming is not an absent one.
        AudioState::Configured => Span::styled("configured, no stream", Style::default().fg(DIM)),
        AudioState::Absent => Span::styled("nothing wired", Style::default().fg(DIM)),
    };
    lines.push(Line::from(vec![
        Span::styled("  AUDIO     ", Style::default().fg(DIM)),
        audio,
    ]));

    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "AVAILABLE   up/down to choose, enter to attach",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));

    lines.extend(devices::picker_lines(
        &view.devices,
        &view.device_selection,
        PickerStyles {
            heading: Style::default().fg(SETTING),
            label: Style::default().fg(Color::White),
            spec: Style::default().fg(Color::Cyan),
            detail: Style::default().fg(DIM),
            selected: Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
            absent: Style::default().fg(DIM),
        },
    ));

    // What the pick under the cursor is a shortcut for, so an operator can
    // write it down and type it next time.
    if let Some(choice) = view.device_selection.current(&view.devices) {
        let device = &view.devices[choice.list].devices[choice.device];
        lines.push(devices::picker_hint(
            device.kind,
            &device.spec,
            Style::default().fg(DIM),
        ));
    }

    f.render_widget(Paragraph::new(lines), area);
}

/// The band panorama, coarse.
///
/// ADR 0008 is explicit that the terminal gets *a coarse, low-rate
/// rendering of the same frames, not an absence* — so when there is no
/// source this says which, rather than drawing nothing and leaving an
/// operator to wonder whether the panel is broken.
fn draw_spectrum(f: &mut Frame, area: Rect, view: &ConsoleView) {
    if area.height < 3 {
        return;
    }
    let Some(newest) = view.spectrum.first() else {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "NO SPECTRUM SOURCE — attach one with --if-out",
                Style::default().fg(DIM),
            ))),
            area,
        );
        return;
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(area.height / 3), Constraint::Min(1)])
        .split(area);

    crate::spectrum::spectrum_trace(
        newest,
        rows[0],
        f.buffer_mut(),
        newest.ref_level_dbm - 60.0,
        RF_TRACE,
    );
    crate::spectrum::waterfall(
        &view.spectrum,
        rows[1],
        f.buffer_mut(),
        crate::spectrum::WaterfallPalette::default(),
        newest.ref_level_dbm - 60.0,
        Color::Black,
    );
}

// ── the reference rail ──────────────────────────────────────────────────

/// The reference rail's rows, as text.
///
/// A dash where nothing was read. See `RadioDisplay::levels_known`: over
/// the console protocol none of these fields arrive, and drawing the
/// struct's defaults told the operator `AF 200` at a radio reading
/// `AG034`, and `PRE off` at a radio with its preamp on -- confidently,
/// and indistinguishably from a real reading.
pub(crate) fn reference_facts(radio: &RadioDisplay) -> Vec<(&'static str, String)> {
    let known = radio.levels_known;
    let val = |s: String| if known { s } else { "—".to_string() };
    let flag = |b: bool| {
        if known {
            on_off(b).to_string()
        } else {
            "—".to_string()
        }
    };
    vec![
        ("ANT", val(format!("{}", radio.antenna))),
        ("AF", val(format!("{}", radio.af_gain))),
        ("RF", val(format!("{}", radio.rf_gain))),
        ("SQL", val(format!("{}", radio.squelch))),
        ("MIC", val(format!("{}", radio.mic_gain))),
        ("PWR", val(format!("{}W", radio.power_pct))),
        ("AGC", val(format!("{}", radio.agc))),
        ("NB", flag(radio.noise_blanker)),
        ("NR", val(format!("{}", radio.noise_reduction))),
        ("PRE", flag(radio.preamp)),
        ("ATT", flag(radio.attenuator)),
        ("PROC", flag(radio.speech_processor)),
        ("VOX", flag(radio.vox)),
        ("LOCK", flag(radio.freq_lock)),
    ]
}

fn draw_reference(f: &mut Frame, area: Rect, radio: &RadioDisplay) {
    if area.width == 0 {
        return;
    }
    let facts = reference_facts(radio);
    let lines: Vec<Line> = facts
        .iter()
        .map(|(k, v)| {
            Line::from(vec![
                Span::styled(format!("{k:<5}"), Style::default().fg(DIM)),
                Span::styled(v.clone(), Style::default().fg(Color::White)),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), area);
}

// ── status strip and command line ───────────────────────────────────────

fn draw_status(f: &mut Frame, area: Rect, radio: &RadioDisplay, view: &ConsoleView) {
    let link = if radio.initializing {
        Span::styled("connecting", Style::default().fg(SETTING))
    } else if radio.connected {
        Span::styled("linked", Style::default().fg(Color::Green))
    } else {
        Span::styled(
            "LINK LOST",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )
    };
    let mut spans = vec![link, Span::raw("  ")];
    if let Some(msg) = &view.message {
        spans.push(Span::styled(msg.clone(), Style::default().fg(Color::White)));
    } else if let Some(err) = radio.poll_errors.first() {
        spans.push(Span::styled(err.clone(), Style::default().fg(Color::Red)));
        // The old console had a whole panel for these and this strip is one
        // line, so the count goes on the end rather than the rest being
        // silently dropped: "one thing went wrong" and "eleven things went
        // wrong" are different situations and an operator should be able to
        // tell which one they are in.
        if radio.poll_errors.len() > 1 {
            spans.push(Span::styled(
                format!("  (+{} more)", radio.poll_errors.len() - 1),
                Style::default().fg(SETTING),
            ));
        }
    } else {
        spans.push(Span::styled(
            "1-4 tabs   : command   f n m r t c o s d menus   q quit",
            Style::default().fg(DIM),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_command(f: &mut Frame, area: Rect, view: &ConsoleView) {
    let line = match &view.command {
        Some(buf) => Line::from(vec![
            Span::styled(":", Style::default().fg(Color::White)),
            Span::styled(buf.clone(), Style::default().fg(Color::White)),
            Span::styled("_", Style::default().fg(SETTING)),
        ]),
        None => Line::from(Span::styled("", Style::default())),
    };
    f.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_rail_with_nothing_read_shows_dashes_not_defaults() {
        // Reported from the bench: the network console displayed `AF 200`
        // at a radio reading AG034, and `PRE off` at a radio with its
        // preamp on. Both were `RadioDisplay::default()` -- the console
        // protocol carries none of these fields -- rendered
        // indistinguishably from a real reading.
        let radio = RadioDisplay::default();
        assert!(!radio.levels_known, "a fresh display has read nothing");
        for (k, v) in super::reference_facts(&radio) {
            assert_eq!(v, "—", "{k} must not be drawn from a default");
        }
    }

    #[test]
    fn a_rail_that_was_read_shows_its_values() {
        let radio = RadioDisplay {
            levels_known: true,
            af_gain: 34,
            preamp: true,
            ..Default::default()
        };
        let facts = super::reference_facts(&radio);
        let get = |k: &str| {
            facts
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(get("AF"), "34");
        assert_eq!(get("PRE"), "on");
        assert!(!facts.iter().any(|(_, v)| v == "—"), "nothing unknown here");
    }

    #[test]
    fn two_stacked_panels_merge_into_one_rect() {
        // The spectrum was being drawn twice on its own tab: once as the
        // layout's Spectrum panel and once as the SPECTRUM tab's workspace
        // content. Two identical waterfalls, each half the height it could
        // have had. The fix gives the spectrum both rects rather than
        // blanking the workspace, which would have left a hole exactly
        // where the operator is looking.
        let top = Rect::new(0, 5, 100, 12);
        let bottom = Rect::new(0, 17, 100, 8);
        let m = super::union(top, bottom);
        assert_eq!(m, Rect::new(0, 5, 100, 20));
        assert_eq!(
            m.height,
            top.height + bottom.height,
            "no rows lost or invented between adjacent panels"
        );
    }

    #[test]
    fn union_is_order_independent() {
        let a = Rect::new(2, 3, 10, 4);
        let b = Rect::new(2, 7, 10, 6);
        assert_eq!(super::union(a, b), super::union(b, a));
    }

    use super::*;

    fn caps() -> cat_native::CapabilitiesWire {
        cat_ui::demo::full()
    }

    #[test]
    fn the_rails_are_fixed_and_the_content_pane_takes_what_is_left() {
        // The AF FFT is 20 cells because 150 Hz per cell is the resolution
        // it was designed at. A percentage split would make that depend on
        // the terminal size.
        let r = split(Rect::new(0, 0, 120, 40));
        assert_eq!(r.rail.width, RAIL_W);
        assert_eq!(r.reference.width, REF_W);
        assert_eq!(r.content.width, 120 - RAIL_W - REF_W);
        assert_eq!(r.content.width, 72, "the design's content pane");
    }

    #[test]
    fn the_status_strip_and_command_line_belong_to_the_console() {
        // Not to a tab: a command line that moved between tabs would be a
        // different control on each one.
        let r = split(Rect::new(0, 0, 120, 40));
        assert_eq!(r.status.height, 1);
        assert_eq!(r.command.height, 1);
        assert_eq!(r.status.y, 38);
        assert_eq!(r.command.y, 39);
        assert_eq!(r.status.width, 120, "full width, under both rails");
    }

    #[test]
    fn a_narrow_terminal_degrades_rather_than_panicking() {
        // ratatui will happily hand out zero-width rects; every draw
        // function has to survive one.
        for w in [0u16, 1, 10, 22, 40, 119] {
            let r = split(Rect::new(0, 0, w, 40));
            assert!(r.rail.width + r.content.width + r.reference.width <= w.max(1));
        }
    }

    #[test]
    fn the_tabs_come_from_the_radio_and_the_first_one_is_open() {
        let view = ConsoleView::for_capabilities(&caps());
        assert_eq!(view.tab, Tab::Spectrum);
        let labels: Vec<&str> = view.tabs.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, vec!["SPECTRUM", "MEMORY 0–99", "MENU 52", "SOURCE"]);
    }

    #[test]
    fn a_radio_with_nothing_opens_on_the_tab_that_says_so() {
        let view = ConsoleView::for_capabilities(&cat_ui::demo::bare());
        assert_eq!(view.tab, Tab::Source);
    }

    #[test]
    fn the_audio_panels_default_to_pending_not_absent() {
        // This station has the ACC2 pair wired; the client transport is a
        // separate question. Showing NONE would be a claim about the
        // hardware that is not true.
        assert_eq!(ConsoleView::default().audio, AudioState::Configured);
    }

    #[test]
    fn the_band_row_marks_the_band_the_dial_is_in() {
        let caps = test_caps();
        let text: String = band_row(14_074_000, &caps)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains(" 20m "));
        // Every band this radio reaches is offered, because it is a pick
        // and not a step.
        for band in bands_for(&caps) {
            assert!(
                text.contains(band.label),
                "band {} must be reachable",
                band.label
            );
        }
    }

    #[test]
    fn the_bands_offered_are_the_ones_this_radio_reaches() {
        // Derived, not a fixed list. A radio that stops at 30 MHz must not
        // be offered 2 m, and one that reaches 430 MHz must not be denied
        // it -- which is what a hardcoded HF row did to every radio but
        // the one it was written for.
        let hf_only = {
            let mut c = test_caps();
            c.rx_range = cat_native::FrequencyRange::new(500_000, 30_000_000);
            c
        };
        let wide = {
            let mut c = test_caps();
            c.rx_range = cat_native::FrequencyRange::new(30_000, 470_000_000);
            c
        };
        let hf: Vec<&str> = bands_for(&hf_only).iter().map(|b| b.label).collect();
        let all: Vec<&str> = bands_for(&wide).iter().map(|b| b.label).collect();
        assert!(!hf.contains(&"2m"), "{hf:?}");
        assert!(all.contains(&"2m"), "{all:?}");
        assert!(all.len() > hf.len());
    }

    #[test]
    fn the_modes_offered_are_the_ones_this_radio_declares() {
        // Same rule. A radio with a digital-voice mode should show it, and
        // its own label for it -- "DV" on an Icom, "C4FM" on a Yaesu.
        let caps = test_caps();
        let text: String = mode_row("USB", &caps)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        for descriptor in &caps.modes {
            assert!(
                text.contains(descriptor.label.as_str()),
                "{} missing",
                descriptor.label
            );
        }
    }

    #[test]
    fn a_dial_outside_every_ham_band_highlights_none_of_them() {
        // 5 MHz is inside this radio's receive range and inside no band
        // row. Highlighting the nearest would be a lie about where it is.
        let line = band_row(5_000_000, &test_caps());
        let highlighted = line
            .spans
            .iter()
            .filter(|s| s.style.bg == Some(SETTING))
            .count();
        assert_eq!(highlighted, 0);
    }

    #[test]
    fn an_unknown_ribbon_value_is_dimmed_rather_than_guessed() {
        // FILTER/SHIFT/NOTCH have no field in RadioDisplay yet. They show
        // where they would be, dimmed -- the same rule as the absent
        // spectrum and the absent audio stream.
        let radio = RadioDisplay::default();
        let cells = ribbon_cells(&radio);
        let filter = cells.iter().find(|c| c.0 == "FILTER").unwrap();
        assert_eq!(filter.1, "—");
        assert!(!filter.2, "and it is marked as not known");
    }

    #[test]
    fn rit_and_xit_are_present_even_when_off() {
        let cells = ribbon_cells(&RadioDisplay::default());
        assert!(cells.iter().any(|c| c.0 == "RIT"));
        assert!(cells.iter().any(|c| c.0 == "XIT"));
    }

    #[test]
    fn a_disconnected_radio_shows_no_frequency_rather_than_a_stale_one() {
        // "Not asked yet" and "the answer is zero" are different states.
        let radio = RadioDisplay {
            connected: false,
            ..RadioDisplay::default()
        };
        let view = ConsoleView::default();

        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        term.draw(|f| {
            draw(f, f.size(), &radio, &view, &test_caps());
        })
        .unwrap();
        let buf = term.backend().buffer();
        let row: String = (0..120)
            .map(|x| buf.get(x, 1).symbol().to_string())
            .collect();
        assert!(row.contains('—'), "a lost link shows em-dashes: {row:?}");
    }

    #[test]
    fn the_whole_console_draws_at_the_design_size_without_panicking() {
        let radio = RadioDisplay::default();
        let view = ConsoleView::for_capabilities(&caps());
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        term.draw(|f| {
            draw(f, f.size(), &radio, &view, &test_caps());
        })
        .unwrap();
    }

    /// The whole screen as text, for a test that cares where things are.
    fn screen(radio: &RadioDisplay, w: u16, h: u16) -> Vec<String> {
        let view = ConsoleView::for_capabilities(&caps());
        let backend = ratatui::backend::TestBackend::new(w, h);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        term.draw(|f| {
            draw(f, f.size(), radio, &view, &test_caps());
        })
        .unwrap();
        let buf = term.backend().buffer();
        (0..h)
            .map(|y| (0..w).map(|x| buf.get(x, y).symbol().to_string()).collect())
            .collect()
    }

    #[test]
    fn the_meter_rail_keeps_a_gutter_so_a_full_bar_does_not_touch_the_next_panel() {
        // A full-scale S-meter used to run its "S8" straight into the
        // panel beside it, reading as `S8NO SPECTRUM SOURCE`.
        let radio = RadioDisplay {
            connected: true,
            smeter: u16::MAX,
            ..RadioDisplay::default()
        };
        let width = 22u16;
        let backend = ratatui::backend::TestBackend::new(width, 6);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        term.draw(|f| {
            draw_meters(f, f.size(), &radio, &test_caps());
        })
        .unwrap();
        let buf = term.backend().buffer();
        for y in 0..6u16 {
            assert_eq!(
                buf.get(width - 1, y).symbol(),
                " ",
                "the last column is the gutter, row {y}"
            );
        }
    }

    #[test]
    fn a_keyed_radio_says_so_across_the_whole_width() {
        // Reported from the bench: the old indicator was one styled word
        // in a row of other words, and an operator did not see it. A
        // transmitting radio is not a field on a form.
        let radio = RadioDisplay {
            tx: true,
            connected: true,
            ..RadioDisplay::default()
        };
        let rows = screen(&radio, 120, 40);
        assert!(
            rows[0].contains("T R A N S M I T T I N G"),
            "the first row is the banner: {:?}",
            rows[0]
        );
    }

    #[test]
    fn a_receiving_radio_does_not_give_up_a_row_to_the_banner() {
        let radio = RadioDisplay {
            tx: false,
            connected: true,
            ..RadioDisplay::default()
        };
        let rows = screen(&radio, 120, 40);
        assert!(
            !rows.iter().any(|r| r.contains("T R A N S M I T T I N G")),
            "nothing is keyed, so nothing should say it is"
        );
    }

    #[test]
    fn the_banner_is_painted_the_whole_way_across() {
        // Centred by padding rather than alignment: a bar of colour that
        // stopped at the text would read as a label, not an alarm.
        let radio = RadioDisplay {
            tx: true,
            connected: true,
            ..RadioDisplay::default()
        };
        let view = ConsoleView::for_capabilities(&caps());
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        term.draw(|f| {
            draw(f, f.size(), &radio, &view, &test_caps());
        })
        .unwrap();
        let buf = term.backend().buffer();
        for x in [0u16, 1, 59, 118, 119] {
            assert_eq!(
                buf.get(x, 0).style().bg,
                Some(Color::Red),
                "column {x} of the banner must be painted"
            );
        }
    }

    #[test]
    fn a_terminal_too_short_for_a_banner_keeps_its_console() {
        // Losing a row out of five to a banner would cost more than the
        // banner is worth.
        let radio = RadioDisplay {
            tx: true,
            connected: true,
            ..RadioDisplay::default()
        };
        let rows = screen(&radio, 40, 2);
        assert!(!rows.iter().any(|r| r.contains("TRANSMITTING")));
    }

    #[test]
    fn it_also_draws_at_sizes_nobody_designed_for() {
        let view = ConsoleView::for_capabilities(&caps());
        for (w, h) in [(80u16, 24u16), (40, 12), (200, 60), (20, 5)] {
            for tx in [false, true] {
                let radio = RadioDisplay {
                    tx,
                    ..RadioDisplay::default()
                };
                let backend = ratatui::backend::TestBackend::new(w, h);
                let mut term = ratatui::Terminal::new(backend).unwrap();
                term.draw(|f| {
                    draw(f, f.size(), &radio, &view, &test_caps());
                })
                .unwrap();
            }
        }
        let radio = RadioDisplay::default();
        for (w, h) in [(80u16, 24u16), (40, 12), (200, 60), (20, 5)] {
            let backend = ratatui::backend::TestBackend::new(w, h);
            let mut term = ratatui::Terminal::new(backend).unwrap();
            term.draw(|f| {
                draw(f, f.size(), &radio, &view, &test_caps());
            })
            .unwrap();
        }
    }
}

// ── keys ────────────────────────────────────────────────────────────────

/// What a key did to the console.
pub enum ConsoleKey {
    /// The console handled it.
    Consumed,
    /// Not the console's; the radio's own menus get it next.
    Passthrough,
    /// A command line was submitted and parsed to this.
    Action(cat_ui::command::Action),
    /// A command line was submitted and did not parse. Already reported on
    /// the status strip; returned so a caller can log it.
    Rejected(String),
    /// The operator picked a device on the SOURCE tab and pressed enter.
    /// Opening it is the wiring layer's job, not the console's.
    Attach(cat_signal::DeviceInfo),
    /// The operator asked for the device list to be taken again.
    ///
    /// Enumerating is not free -- it talks to the sound server, and over a
    /// network link it is a round trip -- so it happens when asked for,
    /// not on every redraw. Somebody who has just plugged a dongle in
    /// should not have to restart the console to see it.
    RefreshDevices,
}

/// Handle a key against the console's own navigation.
///
/// Deliberately consumes as little as possible. The digits and `:` are the
/// design's, and everything else falls through to the radio's feature
/// menus — which is what keeps `[F]`, `[M]`, `[D]` and the rest reachable
/// from the resting screen exactly as they were before.
pub fn handle_key(
    key: crossterm::event::KeyEvent,
    view: &mut ConsoleView,
    caps: &cat_native::CapabilitiesWire,
) -> ConsoleKey {
    use crossterm::event::KeyCode;

    // While the command line is open it takes every key. A console where
    // typing `q` in a text field quit would be a console nobody types in.
    if let Some(buf) = view.command.as_mut() {
        match key.code {
            KeyCode::Esc => {
                view.command = None;
                view.message = None;
                return ConsoleKey::Consumed;
            }
            KeyCode::Backspace => {
                buf.pop();
                return ConsoleKey::Consumed;
            }
            KeyCode::Char(c) => {
                buf.push(c);
                return ConsoleKey::Consumed;
            }
            KeyCode::Enter => {
                let line = std::mem::take(buf);
                view.command = None;
                return match cat_ui::command::parse(&line, caps) {
                    Ok(action) => {
                        view.message = None;
                        if let cat_ui::command::Action::SelectTab(n) = action {
                            select_tab(view, n);
                            return ConsoleKey::Consumed;
                        }
                        ConsoleKey::Action(action)
                    }
                    Err(e) => {
                        let msg = format!("{e:?}");
                        view.message = Some(msg.clone());
                        ConsoleKey::Rejected(msg)
                    }
                };
            }
            _ => return ConsoleKey::Consumed,
        }
    }

    // The SOURCE tab is a working surface, so while it is open the arrows
    // and enter belong to it. Every other key still falls through -- the
    // digits must keep switching tabs from here, or an operator could get
    // into the picker and not back out.
    if view.tab == Tab::Source {
        match key.code {
            KeyCode::Up => {
                view.device_selection.move_by(-1, &view.devices);
                return ConsoleKey::Consumed;
            }
            KeyCode::Down => {
                view.device_selection.move_by(1, &view.devices);
                return ConsoleKey::Consumed;
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                return ConsoleKey::RefreshDevices;
            }
            KeyCode::Enter => {
                return match view.device_selection.current(&view.devices) {
                    Some(choice) => {
                        ConsoleKey::Attach(view.devices[choice.list].devices[choice.device].clone())
                    }
                    None => {
                        view.message = Some("nothing to attach".to_string());
                        ConsoleKey::Consumed
                    }
                };
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Char(':') => {
            view.command = Some(String::new());
            view.message = None;
            ConsoleKey::Consumed
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            let n = c.to_digit(10).unwrap_or(0) as usize;
            if select_tab(view, n) {
                ConsoleKey::Consumed
            } else {
                // A digit that names no tab is not swallowed: on this radio
                // the feature menus use digits too, and eating one here
                // would make a submenu silently ignore a keypress.
                ConsoleKey::Passthrough
            }
        }
        _ => ConsoleKey::Passthrough,
    }
}

/// Open the 1-based `n`th tab, if there is one. Returns whether there was.
fn select_tab(view: &mut ConsoleView, n: usize) -> bool {
    match cat_ui::workspace::tab_for_digit(&view.tabs, n) {
        Some(tab) => {
            view.tab = tab;
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent};

    fn caps() -> cat_native::CapabilitiesWire {
        cat_ui::demo::full()
    }

    fn press(view: &mut ConsoleView, c: char) -> ConsoleKey {
        handle_key(KeyEvent::from(KeyCode::Char(c)), view, &caps())
    }

    fn key(view: &mut ConsoleView, code: KeyCode) -> ConsoleKey {
        handle_key(KeyEvent::from(code), view, &caps())
    }

    #[test]
    fn digits_select_the_tabs_the_status_strip_advertises() {
        let mut view = ConsoleView::for_capabilities(&caps());
        assert!(matches!(press(&mut view, '3'), ConsoleKey::Consumed));
        assert_eq!(view.tab, Tab::Menu);
    }

    #[test]
    fn a_digit_naming_no_tab_falls_through_to_the_radios_menus() {
        // The feature menus use digits too. Swallowing one here would make
        // a submenu silently ignore a keypress.
        let mut view = ConsoleView::for_capabilities(&caps());
        assert!(matches!(press(&mut view, '9'), ConsoleKey::Passthrough));
    }

    #[test]
    fn the_command_line_opens_and_takes_every_key_while_open() {
        let mut view = ConsoleView::for_capabilities(&caps());
        press(&mut view, ':');
        assert_eq!(view.command.as_deref(), Some(""));

        // `q` must type a `q`, not quit. A console where a text field can
        // exit the program is a console nobody types in.
        for c in "quit".chars() {
            assert!(matches!(press(&mut view, c), ConsoleKey::Consumed));
        }
        assert_eq!(view.command.as_deref(), Some("quit"));
    }

    #[test]
    fn escape_abandons_the_line_without_running_it() {
        let mut view = ConsoleView::for_capabilities(&caps());
        press(&mut view, ':');
        press(&mut view, 't');
        assert!(matches!(key(&mut view, KeyCode::Esc), ConsoleKey::Consumed));
        assert_eq!(view.command, None);
    }

    #[test]
    fn backspace_edits_rather_than_closing() {
        let mut view = ConsoleView::for_capabilities(&caps());
        press(&mut view, ':');
        press(&mut view, 'a');
        press(&mut view, 'b');
        key(&mut view, KeyCode::Backspace);
        assert_eq!(view.command.as_deref(), Some("a"));
    }

    #[test]
    fn a_parsed_command_comes_back_as_an_action() {
        let mut view = ConsoleView::for_capabilities(&caps());
        press(&mut view, ':');
        for c in "t 14074000".chars() {
            press(&mut view, c);
        }
        match key(&mut view, KeyCode::Enter) {
            ConsoleKey::Action(cat_ui::command::Action::Radio(cmd)) => {
                assert!(matches!(
                    cmd,
                    cat_native::Command::Retune { hz: 14_074_000 }
                ));
            }
            other => panic!("expected a retune action, got {}", describe(&other)),
        }
        assert_eq!(view.command, None, "running a command closes the line");
    }

    #[test]
    fn a_line_that_does_not_parse_is_reported_and_not_run() {
        let mut view = ConsoleView::for_capabilities(&caps());
        press(&mut view, ':');
        for c in "wat".chars() {
            press(&mut view, c);
        }
        assert!(matches!(
            key(&mut view, KeyCode::Enter),
            ConsoleKey::Rejected(_)
        ));
        assert!(view.message.is_some(), "and the operator is told why");
    }

    #[test]
    fn a_tab_command_moves_the_tab_here_rather_than_going_to_the_radio() {
        let mut view = ConsoleView::for_capabilities(&caps());
        press(&mut view, ':');
        // A bare digit is the verb -- `:2` and the `2` key are the same
        // action, which is why `Action::SelectTab` exists at all.
        press(&mut view, '2');
        let outcome = key(&mut view, KeyCode::Enter);
        assert!(matches!(outcome, ConsoleKey::Consumed));
        assert_eq!(view.tab, Tab::Memory);
    }

    #[test]
    fn letters_the_console_does_not_use_reach_the_radios_menus() {
        let mut view = ConsoleView::for_capabilities(&caps());
        for c in ['f', 'm', 'd', 'p', 'q'] {
            assert!(
                matches!(press(&mut view, c), ConsoleKey::Passthrough),
                "{c} must still open what it always opened"
            );
        }
    }

    fn describe(k: &ConsoleKey) -> &'static str {
        match k {
            ConsoleKey::Consumed => "Consumed",
            ConsoleKey::Passthrough => "Passthrough",
            ConsoleKey::Action(_) => "Action",
            ConsoleKey::Rejected(_) => "Rejected",
            ConsoleKey::Attach(_) => "Attach",
            ConsoleKey::RefreshDevices => "RefreshDevices",
        }
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;

    fn strip(radio: &RadioDisplay, view: &ConsoleView) -> String {
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        term.draw(|f| {
            draw(f, f.size(), radio, view, &test_caps());
        })
        .unwrap();
        let buf = term.backend().buffer();
        (0..120)
            .map(|x| buf.get(x, 38).symbol().to_string())
            .collect()
    }

    #[test]
    fn several_poll_errors_are_counted_rather_than_hidden() {
        // The old console had a whole panel for these; this strip is one
        // line. Showing the first and saying nothing about the rest would
        // make eleven failures look like one.
        let radio = RadioDisplay {
            initializing: false,
            poll_errors: vec![
                "FA timeout".into(),
                "MD timeout".into(),
                "SM timeout".into(),
            ],
            ..RadioDisplay::default()
        };
        let line = strip(&radio, &ConsoleView::default());
        assert!(line.contains("FA timeout"), "{line:?}");
        assert!(line.contains("+2 more"), "{line:?}");
    }

    #[test]
    fn a_single_error_is_not_decorated_with_a_count() {
        let radio = RadioDisplay {
            initializing: false,
            poll_errors: vec!["FA timeout".into()],
            ..RadioDisplay::default()
        };
        let line = strip(&radio, &ConsoleView::default());
        assert!(line.contains("FA timeout"));
        assert!(!line.contains("more"), "{line:?}");
    }

    #[test]
    fn a_lost_link_says_so_before_it_says_anything_else() {
        let radio = RadioDisplay {
            initializing: false,
            connected: false,
            ..RadioDisplay::default()
        };
        let line = strip(&radio, &ConsoleView::default());
        assert!(line.starts_with("LINK LOST"), "{line:?}");
    }
}

#[cfg(test)]
mod meter_tests {
    use super::*;

    /// A TS-570D's S meter, as a server publishes it.
    ///
    /// A fixture rather than the radio's own declaration, because this
    /// crate is shared and must not name one radio. The values are that
    /// radio's, so the exhaustive test below still checks the thing it was
    /// written to check.
    fn ts570d_meters() -> Vec<cat_native::MeterDescriptorWire> {
        vec![cat_native::MeterDescriptorWire {
            kind: MeterKind::S,
            raw_range: cat_native::RawRange::new(0, 30),
            active_on_transmit: false,
            s_units: Some(cat_native::SUnitScale::TS570D),
        }]
    }

    fn reading(raw: u16) -> MeterReading {
        // The console's own path, not a reimplementation of it: this is
        // the exact call `draw_rail` makes.
        MeterReading::from_wire(&ts570d_meters(), MeterKind::S, raw)
            .expect("this fixture has an S meter")
    }

    /// The table this console shipped with, before any of it moved into a
    /// shared crate. Written out in full rather than referenced, so that a
    /// change to `SUnitScale::TS570D` upstream shows up here as a failure
    /// rather than as agreement.
    fn as_shipped(smeter: u16) -> &'static str {
        match smeter {
            0..=2 => "S0",
            3..=4 => "S1",
            5..=6 => "S2",
            7..=8 => "S3",
            9..=10 => "S4",
            11..=12 => "S5",
            13..=14 => "S6",
            15..=16 => "S7",
            17..=18 => "S8",
            19..=20 => "S9",
            21..=24 => "S9+10",
            25..=28 => "S9+20",
            _ => "S9+30",
        }
    }

    #[test]
    fn every_value_the_meter_can_report_still_reads_the_way_it_always_has() {
        // The acceptance bar for moving onto shared widgets (radio-cat-rs
        // ADR 0011 rev 4) is that the operator sees no change, and the
        // layout rebuild does not lower it. For the S-unit readout that is
        // checkable exhaustively, so it is: the meter reports 0-30 and this
        // walks all 31.
        //
        // It exercises the whole path -- capabilities to `MeterReading` to
        // label -- so it fails if the radio stops publishing its table, not
        // only if the table changes.
        for raw in 0..=30u16 {
            assert_eq!(
                reading(raw).s_unit(),
                as_shipped(raw),
                "raw {raw} changed meaning"
            );
        }
    }

    #[test]
    fn the_reading_carries_this_radios_range_and_not_some_other_ones() {
        // 15 is mid-scale here and under 6% on an FT-991A. Getting the
        // range from capabilities rather than a literal is what keeps the
        // bar honest.
        let r = reading(15);
        assert_eq!(r.range.max, 30);
        assert_eq!(r.fraction(), 0.5);
    }

    #[test]
    fn the_readout_formats_a_frequency_the_way_it_always_has() {
        let line = vfo_readout(
            Some(14_000_000),
            None,
            Style::default(),
            Style::default(),
            Style::default(),
        );
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "14.000.000 MHz");

        let line = vfo_readout(
            Some(7_250_000),
            None,
            Style::default(),
            Style::default(),
            Style::default(),
        );
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "7.250.000 MHz");
    }

    #[test]
    fn a_requested_frequency_follows_the_confirmed_one_rather_than_replacing_it() {
        // The pending grammar the design uses in four places. A readout
        // that showed the requested value alone would claim the radio had
        // done something it has not acknowledged.
        let line = vfo_readout(
            Some(14_074_000),
            Some(14_195_000),
            Style::default(),
            Style::default(),
            Style::default(),
        );
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("14.074.000 MHz"), "{text:?}");
        assert!(text.contains("14.195.000 MHz"), "{text:?}");
    }
}

// ── the passband the AF FFT marks ───────────────────────────────────────

// `mode_from_name` and `passband_for` moved to `cat_ui::af`, keyed on
// `ModeId` rather than on a display label.
//
// Parsing a label back into a mode worked while one radio had a console
// and breaks as soon as a second does: this radio spells a mode "CW",
// the next spells it "CW-U", and a third has "DATA-U" with no Kenwood
// counterpart at all. The shared version reads the width the radio
// itself published, which is both correct for every radio and one fewer
// table to keep in step.

#[cfg(test)]
mod tab_body_tests {
    use super::*;

    fn screen(view: &ConsoleView) -> String {
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        let radio = RadioDisplay::default();
        term.draw(|f| {
            draw(f, f.size(), &radio, view, &test_caps());
        })
        .unwrap();
        let buf = term.backend().buffer();
        (0..40)
            .map(|y| {
                (0..120)
                    .map(|x| buf.get(x, y).symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn selecting_a_tab_draws_that_tabs_body() {
        // The whole point of the tab bar. Guards the wiring from
        // `ConsoleView::tab` through to what is actually on screen.
        let mut view = ConsoleView::for_capabilities(&cat_ui::demo::full());

        view.tab = Tab::Memory;
        assert!(screen(&view).contains("channel"), "MEMORY tab body");

        view.tab = Tab::Spectrum;
        assert!(
            screen(&view).contains("NO SPECTRUM SOURCE"),
            "SPECTRUM tab body with no source attached"
        );
    }

    #[test]
    fn every_tab_body_says_something_specific() {
        // A tab that draws only its own name is a tab that was never
        // finished. Each of these asserts on content the *body* produces,
        // not on the label the bar already shows.
        let mut view = ConsoleView::for_capabilities(&cat_ui::demo::full());

        view.tab = Tab::Memory;
        let s = screen(&view);
        assert!(s.contains("channel"), "MEMORY names the channel");
        assert!(s.contains(":mem <n>"), "and how to change it");

        view.tab = Tab::Menu;
        let s = screen(&view);
        assert!(s.contains("not polled"), "MENU is honest about not polling");
        assert!(s.contains("[S]"), "and says what does read an item");

        view.tab = Tab::Source;
        let s = screen(&view);
        assert!(s.contains("ATTACHED"), "SOURCE says what is attached");
        assert!(
            s.contains("IF TAP") && s.contains("AUDIO"),
            "and names both sources"
        );
        assert!(
            s.contains("AVAILABLE"),
            "and offers what could be attached instead"
        );
    }

    #[test]
    fn source_distinguishes_a_configured_audio_path_from_an_absent_one() {
        // The distinction the whole three-state design rests on. "Nothing
        // wired" and "wired, not streaming" are different facts about the
        // station and must not render the same.
        let mut view = ConsoleView::for_capabilities(&cat_ui::demo::full());
        view.tab = Tab::Source;

        view.audio = AudioState::Configured;
        assert!(screen(&view).contains("configured, no stream"));

        view.audio = AudioState::Absent;
        let s = screen(&view);
        assert!(s.contains("nothing wired"));
        assert!(!s.contains("configured, no stream"));
    }

    #[test]
    fn the_open_tab_is_the_one_highlighted_in_the_bar() {
        let mut view = ConsoleView::for_capabilities(&cat_ui::demo::full());
        view.tab = Tab::Menu;
        let s = screen(&view);
        // Every tab is still listed; only one is open.
        assert!(s.contains("MENU 52") && s.contains("SPECTRUM"));
    }
}

#[cfg(test)]
mod picker_key_tests {
    use super::*;
    use cat_signal::{DeviceInfo, DeviceKind, DeviceList};
    use crossterm::event::{KeyCode, KeyEvent};

    fn caps() -> cat_native::CapabilitiesWire {
        cat_ui::demo::full()
    }

    fn view_on_source() -> ConsoleView {
        let mut view = ConsoleView::for_capabilities(&caps());
        view.tab = Tab::Source;
        view.devices = vec![
            DeviceList::found(
                DeviceKind::AudioInput,
                vec![DeviceInfo {
                    kind: DeviceKind::AudioInput,
                    spec: "hw:1,0".into(),
                    label: "USB Audio".into(),
                    detail: None,
                    is_default: true,
                }],
            ),
            DeviceList::found(
                DeviceKind::Sdr,
                vec![DeviceInfo {
                    kind: DeviceKind::Sdr,
                    spec: "rtl:0".into(),
                    label: "Generic RTL2832U".into(),
                    detail: None,
                    is_default: false,
                }],
            ),
        ];
        view
    }

    fn key(view: &mut ConsoleView, code: KeyCode) -> ConsoleKey {
        handle_key(KeyEvent::from(code), view, &caps())
    }

    #[test]
    fn enter_on_the_source_tab_attaches_what_is_under_the_cursor() {
        let mut view = view_on_source();
        match key(&mut view, KeyCode::Down) {
            ConsoleKey::Consumed => {}
            _ => panic!("down should move the cursor"),
        }
        match key(&mut view, KeyCode::Enter) {
            ConsoleKey::Attach(device) => assert_eq!(device.spec, "rtl:0"),
            _ => panic!("expected an attach"),
        }
    }

    #[test]
    fn the_digits_still_switch_tabs_from_inside_the_picker() {
        // Otherwise an operator could get into the SOURCE tab and not out.
        let mut view = view_on_source();
        assert!(matches!(
            handle_key(KeyEvent::from(KeyCode::Char('1')), &mut view, &caps()),
            ConsoleKey::Consumed
        ));
        assert_eq!(view.tab, Tab::Spectrum);
    }

    #[test]
    fn the_arrows_do_nothing_on_a_tab_that_is_not_source() {
        // They belong to the picker, and the picker is not on screen.
        let mut view = ConsoleView::for_capabilities(&caps());
        view.tab = Tab::Spectrum;
        assert!(matches!(
            key(&mut view, KeyCode::Down),
            ConsoleKey::Passthrough
        ));
    }

    #[test]
    fn enter_with_nothing_to_attach_says_so_rather_than_nothing() {
        let mut view = ConsoleView::for_capabilities(&caps());
        view.tab = Tab::Source;
        assert!(matches!(
            key(&mut view, KeyCode::Enter),
            ConsoleKey::Consumed
        ));
        assert_eq!(view.message.as_deref(), Some("nothing to attach"));
    }

    #[test]
    fn the_source_tab_shows_the_spec_the_flag_would_take() {
        let view = view_on_source();
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        let radio = RadioDisplay::default();
        term.draw(|f| {
            draw(f, f.size(), &radio, &view, &test_caps());
        })
        .unwrap();
        let buf = term.backend().buffer();
        let screen: String = (0..40)
            .map(|y| {
                (0..120)
                    .map(|x| buf.get(x, y).symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("hw:1,0"), "{screen}");
        assert!(screen.contains("rtl:0"));
        assert!(screen.contains("same as:"), "and what typing it would be");
    }
}

/// A capability document for the layout tests to draw against.
///
/// Any radio's would do — these tests are about layout, not about one
/// radio — so this is a stub rather than a transcription of a real one,
/// which would invite reading conclusions about that radio out of a test
/// that is not making any.
#[cfg(test)]
fn test_caps() -> cat_native::CapabilitiesWire {
    cat_native::testing::stub_capabilities()
}
