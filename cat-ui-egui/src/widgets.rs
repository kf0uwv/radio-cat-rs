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

//! Widgets a radio brought with it.
//!
//! Most of a console is the shared vocabulary — meters, waterfall, AF
//! panels — and that is the point: three radios do not grow three subtly
//! different S-meters. But a rig sometimes has a feature no other rig has,
//! and telling its operator to do without because the vocabulary is closed
//! would be the wrong trade.
//!
//! So a radio's crate builds the widget out of the exposed primitives —
//! [`crate::meter_bar`], [`crate::af`], [`crate::waterfall`], and egui
//! itself — registers a painter for it under a name, and places
//! `PanelKind::Custom(name)` in its layout.
//!
//! # What a renderer does when it has never heard of the panel
//!
//! It says so. A **pure network console has not linked the radio's crate**
//! and cannot have the painter: `ts570d-gui` talks to any server that
//! speaks the protocol, including one for a radio it was built before.
//! Drawing a blank rectangle there would be indistinguishable from a
//! widget that failed; drawing the panel's name is a different message
//! with a different fix.

use std::collections::HashMap;

use egui::{Rect, Ui};

use crate::theme::Palette;

/// What a custom painter is given.
///
/// Deliberately narrow. A radio's widget gets somewhere to draw, the
/// palette in force, and the console's current view of the radio — and
/// not the console itself, so a widget cannot reach in and change the
/// arrangement it was placed into.
pub struct PanelContext<'a> {
    pub rect: Rect,
    pub palette: &'a Palette,
    pub radio: &'a cat_ui::display::RadioDisplay,
    pub capabilities: &'a cat_native::CapabilitiesWire,
}

/// Draws one radio-supplied panel.
pub type Painter = Box<dyn Fn(&Ui, &PanelContext<'_>)>;

/// The custom widgets an application has supplied.
///
/// Empty by default, which is the state a network console stays in.
#[derive(Default)]
pub struct Widgets {
    painters: HashMap<String, Painter>,
}

impl Widgets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a painter under the name a layout will use for it.
    ///
    /// Prefix the name with the rig — `"ft991a.clarifier"` — because two
    /// radios that both called something `"tuner"` would collide in an
    /// application that had loaded both.
    pub fn register(
        &mut self,
        name: impl Into<String>,
        painter: impl Fn(&Ui, &PanelContext<'_>) + 'static,
    ) -> &mut Self {
        self.painters.insert(name.into(), Box::new(painter));
        self
    }

    pub fn get(&self, name: &str) -> Option<&Painter> {
        self.painters.get(name)
    }

    pub fn has(&self, name: &str) -> bool {
        self.painters.contains_key(name)
    }

    pub fn is_empty(&self) -> bool {
        self.painters.is_empty()
    }

    /// Every name registered, for a diagnostic that has to explain why a
    /// panel came up as a placeholder.
    pub fn names(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self.painters.keys().map(String::as_str).collect();
        out.sort_unstable();
        out
    }
}

impl std::fmt::Debug for Widgets {
    /// Painters are closures and have nothing useful to print; what a
    /// reader wants to know is which names are covered.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Widgets")
            .field("registered", &self.names())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_console_with_no_radio_crate_has_no_widgets() {
        // The state a pure network console stays in, and it must be a
        // normal one rather than something to work around.
        let w = Widgets::new();
        assert!(w.is_empty());
        assert!(!w.has("ft991a.clarifier"));
        assert!(w.get("ft991a.clarifier").is_none());
    }

    #[test]
    fn a_radio_can_register_its_own() {
        let mut w = Widgets::new();
        w.register("ft991a.clarifier", |_, _| {});
        assert!(w.has("ft991a.clarifier"));
        assert_eq!(w.names(), vec!["ft991a.clarifier"]);
    }

    #[test]
    fn names_are_prefixed_so_two_radios_do_not_collide() {
        // Not enforced -- a name is a string and this cannot police it --
        // but the registry keeps them apart, which is what matters when an
        // application has loaded two radios' widgets.
        let mut w = Widgets::new();
        w.register("ft991a.tuner", |_, _| {});
        w.register("ts570d.tuner", |_, _| {});
        assert_eq!(w.names(), vec!["ft991a.tuner", "ts570d.tuner"]);
    }
}
