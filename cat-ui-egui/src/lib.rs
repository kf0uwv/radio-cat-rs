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

//! GPU-rendered widgets for a radio console.
//!
//! See `docs/adr/0011-cat-ui-base-widgets-radio-specific-layout.md` (rev 4)
//! and `docs/adr/0013-renderer-parity-tui-and-gui.md`. Task 19 of
//! `planning/architect/task_plan.md`.
//!
//! # The same data, a different medium
//!
//! Every question with one right answer per input is answered in
//! [`cat_ui`], not here: which bin a column samples, how bright it is, and
//! where a history row's bins land once the dial has moved. This crate
//! turns those answers into pixels; [`cat_ui_ratatui`] turns the identical
//! answers into cells. That is the seam ADR 0011 draws, and it is why the
//! two renderers cannot disagree about what the spectrum *is* — only about
//! how finely they can show it.
//!
//! # Where the GPU actually helps
//!
//! A waterfall is the one widget in a radio console with a real per-frame
//! cost: 2048 bins at 60 fps, scrolled, is 120 000 values a second landing
//! in a texture. [`WaterfallImage`] keeps that as a ring of rows and hands
//! out an RGBA buffer ready to upload, so the expensive part is a texture
//! write and the scroll is an index rather than a memmove.
//!
//! Everything else here — meters, the VFO readout, grids, the settings
//! list — is immediate-mode drawing that costs nothing worth optimizing.
//! Reaching for a shader there would be effort spent where no time goes.
//!
//! # Layout is not here
//!
//! A widget takes normalized data and draws it. It never decides whether it
//! should be on screen, or where, or what the console does when a
//! capability is absent. Those are layout, they are radio-specific, and
//! they live in each app's own crate.
//!
//! In particular **cursor policy is not here**. Whether a dial marker is
//! pinned to the centre of the span (which is what an `IfTap` physically
//! requires — the SDR is parked on the IF while the radio's LO tracks the
//! dial) or moves freely within a fixed window (a `DirectSdr`) is a
//! property of the source, and a shared widget that assumed either would be
//! silently wrong on the other.

pub mod meter;
pub mod settings;
pub mod waterfall;

pub use meter::{meter_bar, smeter_text};
pub use settings::settings_grid;
pub use waterfall::{Palette, WaterfallImage};
