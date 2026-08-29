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

//! Terminal widgets for a radio console.
//!
//! See `docs/adr/0011-cat-ui-base-widgets-radio-specific-layout.md` (rev 4)
//! and `docs/adr/0013-renderer-parity-tui-and-gui.md`. Task 20 of
//! `planning/architect/task_plan.md`.
//!
//! # What lives here, and what does not
//!
//! Widgets take normalized data and draw it. **A widget never decides
//! whether it should be on screen, or where** — that is layout, and layout
//! is radio-specific (ADR 0011). This crate has no opinion about which
//! panels a TS-570D console shows.
//!
//! The seam against [`cat_ui`] is equally firm: anything that computes a
//! number belongs there, and anything that turns a number into cells
//! belongs here. `cat-ui` owns the *fraction* a meter reading represents;
//! this crate owns the block characters that draw it. That split is why
//! [`cat_ui::MeterReading`] exists rather than a `smeter_bar(u16)`
//! function — the two apps' copies of that function each hardcoded their
//! own radio's scale, which is the bug the fraction removes.
//!
//! # Fidelity, never absence
//!
//! ADR 0013 §2(a): where a terminal cannot carry data at the GUI's
//! fidelity, it carries it *coarsely*. It does not omit it and it does not
//! say "use the GUI". [`waterfall`] and [`spectrum_trace`] are the whole
//! point of this crate, not an afterthought — a console whose terminal
//! renderer had no panorama would be exactly the drift ADR 0013 forbids.

pub mod meter;
pub mod panel;
pub mod session;
pub mod settings;
pub mod spectrum;
pub mod vfo;

pub use meter::{bar_spans, meter_bar, meter_rail, meter_spans, smeter_line, MeterStyles};
pub use panel::{grid, GridItem, GridStyles};
pub use session::{error_panel, header, link_panel, menu_column, ErrorPanelStyles, LinkState};
pub use settings::settings_rows;
pub use spectrum::{spectrum_trace, waterfall, WaterfallPalette};
pub use vfo::vfo_readout;

/// Block characters for a vertical bar, from empty to full.
///
/// Eight sub-levels per cell, so a column of `n` cells resolves `8n`
/// levels. This is the terminal's whole vertical resolution budget and
/// every widget here spends it the same way.
pub(crate) const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Upper half block. Two history rows per text row, which doubles a
/// waterfall's vertical resolution for free by colouring foreground and
/// background independently.
pub(crate) const UPPER_HALF: char = '▀';
