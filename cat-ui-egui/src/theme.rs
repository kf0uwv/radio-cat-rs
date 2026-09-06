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

//! This console's visual identity.
//!
//! ADR 0011 leaves visual identity in the app: `cat-ui-egui` draws the
//! structure and each console chooses the colours. These are the values
//! from the accepted mockup (`planning/designer/mockups/console/`,
//! option 3), kept in one place so a change is a change to the design
//! rather than to nine call sites.

use egui::Color32;

pub const BG: Color32 = Color32::from_rgb(0x06, 0x08, 0x0b);
pub const PANEL: Color32 = Color32::from_rgb(0x0d, 0x11, 0x15);
pub const PANEL_ALT: Color32 = Color32::from_rgb(0x0a, 0x0d, 0x10);
pub const LINE: Color32 = Color32::from_rgb(0x1c, 0x24, 0x2a);
pub const LINE_BRIGHT: Color32 = Color32::from_rgb(0x29, 0x33, 0x3b);

pub const TEXT: Color32 = Color32::from_rgb(0xc6, 0xd3, 0xdc);
pub const DIM: Color32 = Color32::from_rgb(0x6e, 0x7f, 0x8c);
pub const DIMMER: Color32 = Color32::from_rgb(0x46, 0x53, 0x5d);

pub const AMBER: Color32 = Color32::from_rgb(0xe6, 0xab, 0x44);
pub const GREEN: Color32 = Color32::from_rgb(0x4e, 0xc9, 0x7e);
pub const RED: Color32 = Color32::from_rgb(0xe2, 0x54, 0x3c);
pub const BLUE: Color32 = Color32::from_rgb(0x5a, 0xa6, 0xd8);
pub const VIOLET: Color32 = Color32::from_rgb(0x9b, 0x7f, 0xd4);

pub const SELECT: Color32 = Color32::from_rgb(0x1c, 0x4d, 0x6b);
pub const SELECT_LINE: Color32 = Color32::from_rgb(0x4d, 0x94, 0xc0);

/// The colour a capability that is absent is drawn in.
///
/// Absent is not the same as off, and the console says so in one colour
/// consistently: an absent capability is `DIMMER` and unreachable, an
/// inactive one keeps its normal weight. Collapsing the two is how an
/// operator ends up hunting for a control that was never going to exist.
pub const ABSENT: Color32 = DIMMER;

/// Type sizes, named for what they are rather than measured at each use.
pub const SIZE_KEY: f32 = 9.5;
pub const SIZE_VALUE: f32 = 13.0;
pub const SIZE_DIAL: f32 = 30.0;
pub const SIZE_BODY: f32 = 12.0;

/// Apply the console's identity to an egui context.
pub fn install(ctx: &egui::Context) {
    install_with(ctx, Palette::console_default())
}

/// The same, in a radio's own palette.
///
/// Split out because egui's own visuals — window fill, scrollbars, the
/// colour a disabled widget goes — are set on the context rather than read
/// per frame, so a themed console has to re-install them when the radio
/// changes rather than only setting the ambient.
pub fn install_with(ctx: &egui::Context, palette: Palette) {
    set_active(palette);
    // Monospace throughout. This console is an instrument: columns of
    // numbers have to line up, and a proportional face makes a frequency
    // readout shuffle sideways as its digits change. The mockup is set in
    // mono for the same reason, and the first build of this crate ignored
    // that and looked like a form.
    let mut styles = egui::style::default_text_styles();
    for (_, font) in styles.iter_mut() {
        font.family = egui::FontFamily::Monospace;
    }
    styles.insert(egui::TextStyle::Body, egui::FontId::monospace(SIZE_BODY));
    styles.insert(egui::TextStyle::Button, egui::FontId::monospace(SIZE_BODY));
    styles.insert(egui::TextStyle::Small, egui::FontId::monospace(SIZE_KEY));

    let mut style = (*ctx.style()).clone();
    style.text_styles = styles;
    // Square, tight, and flat. Rounded corners and generous padding read
    // as a settings dialog; this is a panel.
    style.visuals.widgets.noninteractive.rounding = egui::Rounding::ZERO;
    style.visuals.widgets.inactive.rounding = egui::Rounding::ZERO;
    style.visuals.widgets.hovered.rounding = egui::Rounding::ZERO;
    style.visuals.widgets.active.rounding = egui::Rounding::ZERO;
    style.visuals.window_rounding = egui::Rounding::ZERO;
    style.visuals.menu_rounding = egui::Rounding::ZERO;
    style.spacing.item_spacing = egui::vec2(8.0, 3.0);
    style.spacing.window_margin = egui::Margin::same(0.0);
    style.spacing.button_padding = egui::vec2(6.0, 2.0);
    style.spacing.interact_size = egui::vec2(0.0, 18.0);
    ctx.set_style(style);

    install_visuals(ctx);
}

fn install_visuals(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = PANEL_ALT;
    visuals.override_text_color = Some(TEXT);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, LINE);
    visuals.widgets.inactive.bg_fill = PANEL;
    visuals.widgets.hovered.bg_fill = SELECT;
    visuals.widgets.active.bg_fill = SELECT;
    visuals.selection.bg_fill = SELECT;
    visuals.selection.stroke = egui::Stroke::new(1.0, SELECT_LINE);
    ctx.set_visuals(visuals);
}

// ---------------------------------------------------------------------------
// The radio's own palette
// ---------------------------------------------------------------------------

/// The constants above, resolved against whatever palette a radio asked
/// for.
///
/// The consts stay: they are this console's default and what a server that
/// publishes no theme gets. This is the same set of roles, sourced from
/// the radio in front of the operator instead — an amber LCD on charcoal
/// for one rig, a blue TFT for another, the same instrument either way.
///
/// Structure, type scale and spacing are deliberately **not** here. A
/// radio picks its palette; it does not get to restyle a component into
/// something another radio's operator would not recognise.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub panel_alt: Color32,
    pub line: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub absent: Color32,
    /// A value that has been set, a filter edge: what this rig highlights.
    pub accent: Color32,
    /// A meter fill, a spectrum trace.
    pub signal: Color32,
    /// Transmit, and anything that should stop somebody.
    pub warning: Color32,
}

fn rgb(c: cat_layout::Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

impl Palette {
    /// This console's own colours, for a server that published none.
    pub const fn console_default() -> Self {
        Self {
            bg: BG,
            panel: PANEL,
            panel_alt: PANEL_ALT,
            line: LINE,
            text: TEXT,
            dim: DIM,
            absent: ABSENT,
            accent: AMBER,
            signal: GREEN,
            warning: RED,
        }
    }

    /// A radio's published palette.
    ///
    /// The derived shades come from `cat_layout::Theme`'s own mixing
    /// rather than from a second set of rules here, so the terminal and
    /// the GPU consoles dim a label by the same amount.
    pub fn from_theme(theme: &cat_layout::Theme) -> Self {
        Self {
            bg: rgb(theme.background),
            panel: rgb(theme.panel),
            panel_alt: rgb(theme.trough()),
            line: rgb(theme.line()),
            text: rgb(theme.ink),
            dim: rgb(theme.dim()),
            absent: rgb(theme.absent()),
            accent: rgb(theme.accent),
            signal: rgb(theme.signal),
            warning: rgb(theme.warning),
        }
    }

    /// The palette for a radio, falling back to the console's own.
    pub fn for_radio(theme: Option<&cat_layout::Theme>) -> Self {
        match theme {
            Some(t) => Self::from_theme(t),
            None => Self::console_default(),
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::console_default()
    }
}

#[cfg(test)]
mod palette_tests {
    use super::*;

    #[test]
    fn a_server_that_publishes_nothing_gets_the_console_it_had() {
        // The whole compatibility story. An operator whose server predates
        // themes should see no change at all.
        let p = Palette::for_radio(None);
        assert_eq!(p.bg, BG);
        assert_eq!(p.text, TEXT);
        assert_eq!(p.accent, AMBER);
    }

    #[test]
    fn a_radios_palette_reaches_every_role() {
        // Not just the background: a theme that only repainted the ground
        // and left amber accents on a blue rig would look like a bug.
        let theme = cat_layout::Theme {
            background: cat_layout::Rgb::hex(0x001428),
            panel: cat_layout::Rgb::hex(0x002448),
            ink: cat_layout::Rgb::hex(0xdcebff),
            accent: cat_layout::Rgb::hex(0x66ccff),
            signal: cat_layout::Rgb::hex(0x33ff99),
            warning: cat_layout::Rgb::hex(0xff4444),
        };
        let p = Palette::from_theme(&theme);
        let d = Palette::console_default();
        for (a, b) in [
            (p.bg, d.bg),
            (p.panel, d.panel),
            (p.text, d.text),
            (p.accent, d.accent),
            (p.signal, d.signal),
        ] {
            assert_ne!(a, b, "a role kept the console's default colour");
        }
    }

    #[test]
    fn the_derived_shades_come_from_the_shared_rules() {
        // Both renderers dim a label by the same amount, because both ask
        // `cat_layout::Theme` rather than each having its own idea.
        let theme = cat_layout::Theme::console_default();
        assert_eq!(Palette::from_theme(&theme).dim, rgb(theme.dim()));
        assert_eq!(Palette::from_theme(&theme).absent, rgb(theme.absent()));
    }
}

// ---------------------------------------------------------------------------
// The palette in force, for the frame being drawn
// ---------------------------------------------------------------------------

thread_local! {
    static ACTIVE: std::cell::Cell<Palette> =
        const { std::cell::Cell::new(Palette::console_default()) };
}

/// Set the palette for the frame about to be drawn.
///
/// Called once at the top of a frame, from the radio's published theme.
///
/// # Why an ambient and not a parameter
///
/// This is immediate-mode drawing: the palette is constant for a frame and
/// read by nearly every function in it, including small helpers that build
/// a `RichText` and nothing else. Threading it through forty signatures
/// would obscure what each of them is actually about, and a wrong colour
/// is a visible bug rather than a silent one — which is the case where an
/// ambient is the right trade rather than a shortcut.
///
/// Thread-local because egui draws on one thread and a second console in
/// the same process must not repaint the first one's window.
pub fn set_active(palette: Palette) {
    ACTIVE.with(|p| p.set(palette));
}

/// The palette in force.
pub fn active() -> Palette {
    ACTIVE.with(|p| p.get())
}

/// Shorthand, because this is read constantly while drawing.
pub fn pal() -> Palette {
    active()
}
