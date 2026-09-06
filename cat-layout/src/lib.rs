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

//! How a radio's console is arranged, said by the radio's own server.
//!
//! Pure data and one solver: no rendering, no I/O, and no dependency on
//! either renderer. A layout is published by a server and read by a
//! console, so it sits below both.
//!
//! # Why a description and not a design
//!
//! One console derived from a capability set gets a long way — the right
//! tabs appear, the meters a radio has are the meters it shows — and it
//! stops short of the thing that matters. A TS-570D with an SDR on its IF
//! tap wants a waterfall dominating the screen. An FT-991A has no spectrum
//! at all and 151 menu items, and wants that space given to what it *does*
//! have. Both are the same components; neither is the same arrangement.
//!
//! Deriving that from capabilities means encoding one designer's taste in
//! a renderer and calling it inference. Naming it means the person who
//! knows the rig says what the rig's console looks like.
//!
//! # Who says it
//!
//! The **server**, in the capability document, because the server is what
//! knows the radio. A graphical console is a network client that has never
//! heard of a TS-570D (`ts570d` ADR 0008 §3), so a layout compiled into it
//! could only ever be generic. Sent over the wire, a layout is as specific
//! as the radio it came from.
//!
//! # What is shared and what is not
//!
//! Shared: these components, the units they are measured in, and the
//! solver below. Per radio: which of them appear, where, and how big.
//! That split is the whole point — a new radio composes a console out of
//! parts that already work, and does not get to invent a new kind of
//! meter.

pub mod theme;

pub use theme::{Rgb, Theme};

use serde::{Deserialize, Serialize};

/// A component a console can place.
///
/// The named variants are the shared vocabulary, and most of a console is
/// built from them: a radio composing its own arrangement out of parts
/// that already work is the reason three consoles do not end up with
/// three subtly different S-meters.
///
/// [`PanelKind::Custom`] is the way out, for the thing a rig has that no
/// other rig does. A radio's crate builds the widget from the exposed
/// primitives and registers a painter for it under a name; a renderer that
/// has one draws it, and a renderer that does not says so rather than
/// leaving a hole an operator has to guess about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PanelKind {
    /// The dial, mode, and the fields beside them.
    Readout,
    /// Every meter this radio declares, stacked.
    MeterRail,
    /// The band buttons.
    BandBar,
    /// The mode buttons.
    ModeBar,
    /// The eight-cell settings ribbon.
    QuickBar,
    /// Levels this radio exposes — AF, RF, squelch, power.
    LevelsRail,
    /// The capability-derived workspace body: whichever tab is selected.
    Workspace,
    /// The band panorama. A radio with no spectrum source should not
    /// place this; if it does, the panel says so rather than drawing an
    /// empty axis that looks like a dead receiver.
    Spectrum,
    /// The receive audio as a waveform.
    AfScope,
    /// The receive audio's spectrum.
    AfFft,
    /// The one-line message strip.
    Status,
    /// The `:` command line.
    CommandLine,
    /// Nothing. For a gap a layout wants to keep.
    Blank,
    /// A widget this radio brought with it.
    ///
    /// For a feature that is genuinely one rig's: a clarifier that works
    /// unlike anyone else's, a tuner display, a memory-group grid. The
    /// name is the radio's own — prefix it with the rig, as
    /// `"ft991a.clarifier"`, because two radios that both called something
    /// `"tuner"` would collide in a renderer that had loaded both.
    ///
    /// A renderer resolves it through the painter registry its application
    /// supplied. **A pure network console has not linked the radio's
    /// crate and will not have one** — it draws a named placeholder, which
    /// is the honest outcome and visibly different from a panel that
    /// failed to draw.
    Custom(String),
}

impl PanelKind {
    /// Whether this panel is one of the console's own furniture rather
    /// than a radio's content.
    ///
    /// Furniture is what an operator relies on being in the same place
    /// whatever else changes. A layout may move it; it may not put a tab
    /// body where the command line was and expect typing to work.
    pub fn is_furniture(&self) -> bool {
        matches!(self, PanelKind::Status | PanelKind::CommandLine)
    }

    /// The name a custom panel goes by, if this is one.
    pub fn custom_name(&self) -> Option<&str> {
        match self {
            PanelKind::Custom(name) => Some(name),
            _ => None,
        }
    }

    /// A label to draw when a renderer has no painter for this panel.
    ///
    /// Named rather than blank: an operator seeing "ft991a.clarifier"
    /// knows the layout asked for something this console cannot draw,
    /// which is a different problem from a panel that drew nothing.
    pub fn placeholder_label(&self) -> String {
        match self {
            PanelKind::Custom(name) => format!("no widget for {name}"),
            other => format!("no widget for {other:?}"),
        }
    }
}

/// Which way a split runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Rows,
    Columns,
}

/// How much of a split one child takes.
///
/// The units are the caller's: a terminal passes cells, a GPU console
/// passes points. What is fixed is fixed in whatever the renderer counts
/// in, which is why a rail 22 wide is 22 cells in one and 22 points-worth
/// in the other, and why the two look alike without either knowing about
/// the other's units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Size {
    /// Exactly this much, if there is room.
    ///
    /// For a panel whose size is a property of what it draws rather than
    /// of the window: an AF FFT is 20 cells because that is 150 Hz per
    /// cell, and a percentage would make its resolution depend on how big
    /// somebody's terminal is.
    Fixed(u16),
    /// At least this much, then a share of what is left.
    Min(u16),
    /// A share of what is left, weighted against the other `Fill`s.
    Fill(u16),
}

/// A layout: a tree of splits with components at the leaves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Node {
    /// A component.
    Panel(PanelKind),
    /// A row or column split.
    Split {
        direction: Direction,
        children: Vec<Child>,
    },
}

/// One child of a split, and how much room it asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Child {
    pub size: Size,
    pub node: Node,
}

impl Child {
    pub fn new(size: Size, node: Node) -> Self {
        Self { size, node }
    }

    /// A leaf child, which is most of them.
    pub fn panel(size: Size, kind: PanelKind) -> Self {
        Self::new(size, Node::Panel(kind))
    }
}

impl Node {
    /// A split, spelled the way a layout reads.
    pub fn rows(children: Vec<Child>) -> Self {
        Node::Split {
            direction: Direction::Rows,
            children,
        }
    }

    pub fn columns(children: Vec<Child>) -> Self {
        Node::Split {
            direction: Direction::Columns,
            children,
        }
    }

    /// Every panel this layout places, in tree order.
    pub fn panels(&self) -> Vec<PanelKind> {
        match self {
            Node::Panel(kind) => vec![kind.clone()],
            Node::Split { children, .. } => children.iter().flat_map(|c| c.node.panels()).collect(),
        }
    }

    pub fn places(&self, kind: &PanelKind) -> bool {
        self.panels().contains(kind)
    }
}

/// A rectangle, in whatever units the caller counts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Area {
    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Where one panel ended up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub kind: PanelKind,
    pub area: Area,
}

/// A whole console layout, as a server publishes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutSpec {
    pub root: Node,
}

impl LayoutSpec {
    pub fn new(root: Node) -> Self {
        Self { root }
    }

    /// Resolve the tree against an area.
    ///
    /// Returns every panel and where it goes, in tree order. A panel that
    /// resolved to nothing is dropped rather than returned empty: a
    /// renderer should not have to check, and a zero-height panel is not a
    /// panel an operator can see.
    pub fn resolve(&self, area: Area) -> Vec<Placement> {
        let mut out = Vec::new();
        place(&self.root, area, &mut out);
        out
    }

    /// Where one panel is, if the layout places it at all.
    ///
    /// `None` is an ordinary answer: a radio with no spectrum source does
    /// not place a spectrum panel, and a renderer asking for one should
    /// get "not here" rather than a rectangle it then has to guess about.
    pub fn find(&self, area: Area, kind: &PanelKind) -> Option<Area> {
        self.resolve(area)
            .into_iter()
            .find(|p| &p.kind == kind)
            .map(|p| p.area)
    }
}

fn place(node: &Node, area: Area, out: &mut Vec<Placement>) {
    if area.is_empty() {
        return;
    }
    match node {
        Node::Panel(kind) => out.push(Placement {
            kind: kind.clone(),
            area,
        }),
        Node::Split {
            direction,
            children,
        } => {
            let total = match direction {
                Direction::Rows => area.height,
                Direction::Columns => area.width,
            };
            let spans = solve(children.iter().map(|c| c.size), total);
            let mut offset = 0u16;
            for (child, span) in children.iter().zip(spans) {
                let child_area = match direction {
                    Direction::Rows => Area::new(area.x, area.y + offset, area.width, span),
                    Direction::Columns => Area::new(area.x + offset, area.y, span, area.height),
                };
                place(&child.node, child_area, out);
                offset += span;
            }
        }
    }
}

/// Hand out `total` among the requested sizes.
///
/// Fixed and the minimums of `Min` are satisfied first, in order, out of
/// what there is; whatever remains is shared between `Fill` and `Min` by
/// weight.
///
/// Running out is not an error — a console in a small terminal should lose
/// its rightmost panel, not refuse to draw — and a child that cannot have
/// its stated minimum gets **nothing** rather than part of it. A fixed
/// size is fixed because the content needs it, so a fraction of one is not
/// a smaller panel but a wrong one.
fn solve(sizes: impl Iterator<Item = Size> + Clone, total: u16) -> Vec<u16> {
    let sizes: Vec<Size> = sizes.collect();
    let mut given: Vec<u16> = Vec::with_capacity(sizes.len());
    let mut left = total;

    for size in &sizes {
        let want = match size {
            Size::Fixed(n) | Size::Min(n) => *n,
            Size::Fill(_) => 0,
        };
        // All or nothing. A panel is `Fixed(20)` because twenty is what
        // its content needs — an AF FFT at 150 Hz per cell, a rail wide
        // enough for a label — so twenty-minus-some is not a smaller
        // version of it, it is a wrong one. Better to lose the panel and
        // have the operator see it is gone.
        let got = if want <= left { want } else { 0 };
        given.push(got);
        left -= got;
    }

    let weight: u32 = sizes
        .iter()
        .map(|s| match s {
            Size::Fill(w) => u32::from(*w),
            // A `Min` takes a share of the remainder too, weighted 1.
            // Otherwise a layout of one `Min` and nothing else would leave
            // the window mostly empty, which is never what was meant.
            Size::Min(_) => 1,
            Size::Fixed(_) => 0,
        })
        .sum();

    if weight > 0 && left > 0 {
        let remainder = u32::from(left);
        let mut handed = 0u32;
        let mut last_flexible = None;
        for (i, size) in sizes.iter().enumerate() {
            let w = match size {
                Size::Fill(w) => u32::from(*w),
                Size::Min(_) => 1,
                Size::Fixed(_) => 0,
            };
            if w == 0 {
                continue;
            }
            last_flexible = Some(i);
            let share = remainder * w / weight;
            given[i] += share as u16;
            handed += share;
        }
        // Integer division loses up to one unit per flexible child. The
        // last one takes the difference, so the layout fills its area
        // exactly rather than leaving a seam that moves as the window
        // resizes.
        if let Some(i) = last_flexible {
            given[i] += (remainder - handed) as u16;
        }
    }

    given
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Area {
        Area::new(0, 0, 120, 40)
    }

    /// The arrangement `ts570d` ships, as a layout rather than as code.
    fn ts570d_like() -> LayoutSpec {
        LayoutSpec::new(Node::rows(vec![
            Child::new(
                Size::Min(1),
                Node::columns(vec![
                    Child::panel(Size::Fixed(22), PanelKind::MeterRail),
                    Child::panel(Size::Min(1), PanelKind::Workspace),
                    Child::panel(Size::Fixed(26), PanelKind::LevelsRail),
                ]),
            ),
            Child::panel(Size::Fixed(1), PanelKind::Status),
            Child::panel(Size::Fixed(1), PanelKind::CommandLine),
        ]))
    }

    #[test]
    fn a_layout_fills_its_area_exactly() {
        // A seam is not cosmetic: an unclaimed column reads as a gap
        // between panels that moves when the window resizes, and the
        // integer division that produces it is easy to leave in.
        let placed = ts570d_like().resolve(area());
        let row = placed
            .iter()
            .filter(|p| p.area.y == 0)
            .map(|p| p.area.width)
            .sum::<u16>();
        assert_eq!(row, 120, "the top row left a seam");

        let bottom = placed
            .iter()
            .find(|p| p.kind == PanelKind::CommandLine)
            .unwrap();
        assert_eq!(bottom.area.y + bottom.area.height, 40, "rows left a seam");
    }

    #[test]
    fn fixed_panels_keep_their_size_and_the_rest_flexes() {
        // The AF FFT is 20 cells because that is 150 Hz per cell. A
        // percentage would make its resolution depend on the terminal.
        for width in [90u16, 120, 200] {
            let placed = ts570d_like().resolve(Area::new(0, 0, width, 40));
            let rail = placed
                .iter()
                .find(|p| p.kind == PanelKind::MeterRail)
                .unwrap();
            assert_eq!(rail.area.width, 22, "at width {width}");
        }
    }

    #[test]
    fn a_terminal_too_small_loses_panels_rather_than_refusing() {
        // A console that would not draw below some width is a console that
        // fails exactly when somebody is squeezed for space. The rightmost
        // panels go; what is left still draws.
        let placed = ts570d_like().resolve(Area::new(0, 0, 24, 40));
        assert!(placed.iter().any(|p| p.kind == PanelKind::MeterRail));
        assert!(
            !placed.iter().any(|p| p.kind == PanelKind::LevelsRail),
            "a 24-column terminal cannot have both rails, and should not pretend to"
        );
    }

    #[test]
    fn fill_weights_divide_what_is_left() {
        let spec = LayoutSpec::new(Node::columns(vec![
            Child::panel(Size::Fill(1), PanelKind::Spectrum),
            Child::panel(Size::Fill(3), PanelKind::Workspace),
        ]));
        let placed = spec.resolve(Area::new(0, 0, 100, 10));
        let widths: Vec<u16> = placed.iter().map(|p| p.area.width).collect();
        assert_eq!(widths, vec![25, 75]);
    }

    #[test]
    fn a_panel_squeezed_to_nothing_is_dropped_not_returned_empty() {
        // A renderer should not have to check. A zero-height panel is not
        // a panel an operator can see, and drawing into one is how a
        // border ends up on top of the row below it.
        let spec = LayoutSpec::new(Node::rows(vec![
            Child::panel(Size::Fixed(3), PanelKind::Readout),
            Child::panel(Size::Fixed(3), PanelKind::Status),
        ]));
        let placed = spec.resolve(Area::new(0, 0, 40, 3));
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].kind, PanelKind::Readout);
    }

    #[test]
    fn two_radios_can_arrange_the_same_parts_differently() {
        // The point of the whole module. Same components, different
        // consoles — and neither is derived from the other.
        let with_spectrum = LayoutSpec::new(Node::rows(vec![
            Child::panel(Size::Fill(3), PanelKind::Spectrum),
            Child::panel(Size::Fill(1), PanelKind::Workspace),
        ]));
        let without = LayoutSpec::new(Node::rows(vec![
            Child::panel(Size::Fill(1), PanelKind::Workspace),
            Child::panel(Size::Fixed(6), PanelKind::QuickBar),
        ]));

        assert!(with_spectrum.root.places(&PanelKind::Spectrum));
        assert!(!without.root.places(&PanelKind::Spectrum));
        assert_ne!(
            with_spectrum.find(area(), &PanelKind::Workspace),
            without.find(area(), &PanelKind::Workspace),
        );
    }

    #[test]
    fn a_layout_survives_the_wire() {
        // It is published by a server and read by a console that has never
        // heard of the radio, so it has to round-trip.
        let spec = ts570d_like();
        let json = serde_json::to_string(&spec).unwrap();
        let back: LayoutSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, spec);
    }

    #[test]
    fn a_missing_panel_answers_none_rather_than_a_guess() {
        let spec = ts570d_like();
        assert!(spec.find(area(), &PanelKind::Workspace).is_some());
        assert_eq!(spec.find(area(), &PanelKind::Spectrum), None);
    }

    #[test]
    fn a_radio_can_bring_a_widget_no_other_radio_has() {
        // The escape hatch. A closed vocabulary keeps three consoles from
        // growing three S-meters; it must not stop a rig showing the one
        // thing that is genuinely its own.
        let spec = LayoutSpec::new(Node::rows(vec![
            Child::panel(Size::Fill(1), PanelKind::Workspace),
            Child::panel(
                Size::Fixed(4),
                PanelKind::Custom("ft991a.clarifier".to_string()),
            ),
        ]));
        let placed = spec.resolve(Area::new(0, 0, 80, 20));
        let custom = placed
            .iter()
            .find(|p| p.kind.custom_name() == Some("ft991a.clarifier"))
            .expect("the custom panel was placed");
        assert_eq!(custom.area.height, 4);
    }

    #[test]
    fn a_renderer_without_the_widget_says_which_one() {
        // A blank rectangle is indistinguishable from a panel that failed.
        // The name tells an operator this console did not ship the widget,
        // which is a different problem with a different fix.
        let kind = PanelKind::Custom("ic7100.civ-monitor".to_string());
        assert!(kind.placeholder_label().contains("ic7100.civ-monitor"));
    }

    #[test]
    fn a_custom_panel_survives_the_wire() {
        let spec = LayoutSpec::new(Node::Panel(PanelKind::Custom("x.y".to_string())));
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(serde_json::from_str::<LayoutSpec>(&json).unwrap(), spec);
    }

    #[test]
    fn furniture_is_named_so_a_layout_cannot_quietly_lose_it() {
        // A console with no command line cannot be typed into, and a
        // layout that dropped one would look fine until somebody pressed
        // `:`. Naming the class is what lets a server check its own work.
        assert!(PanelKind::CommandLine.is_furniture());
        assert!(PanelKind::Status.is_furniture());
        assert!(!PanelKind::Workspace.is_furniture());

        let spec = ts570d_like();
        for f in [PanelKind::Status, PanelKind::CommandLine] {
            assert!(spec.root.places(&f), "{f:?} missing");
        }
    }
}
