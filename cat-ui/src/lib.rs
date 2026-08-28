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

//! Renderer-agnostic presentation logic for a radio console.
//!
//! See `docs/adr/0011-cat-ui-base-widgets-radio-specific-layout.md`.
//! Task 18 of `planning/architect/task_plan.md`.
//!
//! # What this crate is for
//!
//! `ts570d/ui/src/layout.rs` and `ft991a/ui/src/layout.rs` were ported from
//! one another and have since diverged across 1142 of 1467 lines. Some of
//! that divergence is real (two different radios); some of it is one app
//! having a fix the other lacks. This crate is where the parts that are
//! genuinely shared go, so that a fix lands once.
//!
//! # The seam, and where it falls
//!
//! **No `egui`, no `ratatui`, no `wgpu`.** Nothing here emits a glyph, a
//! colour or a rectangle.
//!
//! That is sharper than the original extraction list, and deliberately so.
//! `mini_bar` and `smeter_bar` in both apps return a `String` of `█` and
//! `░` — which is a *terminal* rendering, not renderer-agnostic logic. What
//! is shared is the **fraction**: where a reading sits in its radio's
//! range. Turning that fraction into block characters belongs in
//! `cat-ui-ratatui`, and into a filled rectangle in `cat-ui-egui`. Putting
//! the glyphs here would have quietly made the "renderer-agnostic" crate
//! a terminal crate that a GPU renderer had to work around.
//!
//! # Two rates, kept apart
//!
//! Spectrum frames are push, high-rate, ~60 fps. CAT state is
//! request/response and can take hundreds of milliseconds. [`lanes`] keeps
//! them in separate structures so a renderer cannot end up blocking a
//! waterfall behind a menu read — the failure that makes a console feel
//! broken while every individual part of it is working.

pub mod band;
pub mod format;
pub mod lanes;
pub mod meter;
pub mod spectrum;
/// Bin-to-display mapping, shared by every renderer.
pub mod spectrum_map;

pub use band::{Band, BandPlan};
pub use format::{format_hz, format_hz_compact, format_smeter_label};
pub use lanes::{CatLane, ConsoleState, SpectrumLane};
pub use meter::MeterReading;
pub use spectrum::SpectrumHistory;
pub use spectrum_map::{
    bin_for_column, column_bins, intensity, projection_offset, sample_column, Sample,
};
