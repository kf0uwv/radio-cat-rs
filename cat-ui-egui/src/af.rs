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

//! The AF scope and AF FFT, drawn with a GPU behind them.
//!
//! Every decision about *what* these show — the fixed 3 kHz axis, the
//! 40 dB window below the peak, where the filter edges fall — belongs to
//! `cat_ui::af` and is shared with the terminal console. What is here is
//! only how to put it on screen.
//!
//! # Where this differs from the terminal, and why that is allowed
//!
//! The terminal draws the scope in braille, at 2x4 dots per cell, because
//! that is the finest a character grid goes. Here it is a polyline at
//! whatever the panel's pixel width is. That is a *fidelity* difference in
//! the sense ADR 0013 allows: the same trace, drawn as finely as each
//! renderer can. Neither shows a different waveform.
//!
//! # Both panels draw in every state
//!
//! An empty panel keeps its size and its zero line. A rail whose panels
//! appeared and vanished as audio came and went would jump under the
//! operator's eyes at exactly the moment they were watching it.

use cat_signal::{AudioScopeFrame, AudioSpectrumFrame};
use egui::{Color32, Rect, Rounding, Stroke, Ui};

use cat_ui::af::{self, AudioState, Passband};

/// Colours an AF panel draws with.
#[derive(Debug, Clone, Copy)]
pub struct AfStyle {
    /// The trace and the bars.
    pub ink: Color32,
    /// The zero line, and everything in a state that is not streaming.
    pub dim: Color32,
    /// Behind the panel.
    pub trough: Color32,
    /// The filter edge marks.
    pub setting: Color32,
}

/// Draw the AF scope: the receive audio as a waveform.
///
/// A line and not a bar chart. Bars can only show |amplitude|, which loses
/// the zero crossing, and the zero crossing is the only thing that makes a
/// trace look like a wave rather than like a histogram of loudness.
pub fn af_scope(ui: &Ui, rect: Rect, frame: Option<&AudioScopeFrame>, state: AudioState) {
    af_scope_styled(ui, rect, frame, state, default_style())
}

/// [`af_scope`] with the caller's own palette.
pub fn af_scope_styled(
    ui: &Ui,
    rect: Rect,
    frame: Option<&AudioScopeFrame>,
    state: AudioState,
    style: AfStyle,
) {
    let painter = ui.painter();
    painter.rect_filled(rect, Rounding::ZERO, style.trough);

    // Drawn in every state, including the empty one. It is what makes the
    // panel legible as a scope rather than as a blank rectangle, and it is
    // where the trace will be.
    let mid = rect.center().y;
    painter.line_segment(
        [egui::pos2(rect.left(), mid), egui::pos2(rect.right(), mid)],
        Stroke::new(1.0, style.dim),
    );

    let Some(frame) = frame.filter(|f| state.is_streaming() && !f.samples.is_empty()) else {
        return;
    };

    let half = rect.height() / 2.0 - 1.0;
    let n = frame.samples.len();
    let points: Vec<egui::Pos2> = frame
        .samples
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let x = rect.left() + rect.width() * (i as f32 / (n - 1).max(1) as f32);
            // Clamped, not scaled to the loudest sample present: a scope
            // that renormalized every frame would show a whisper and a
            // shout as the same height, which is the one thing an
            // operator watches it to tell apart.
            egui::pos2(x, mid - s.clamp(-1.0, 1.0) * half)
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(1.0, style.ink)));
}

/// Draw the AF FFT: the receive audio's spectrum, with the filter edges.
pub fn af_fft(
    ui: &Ui,
    rect: Rect,
    frame: Option<&AudioSpectrumFrame>,
    passband: Option<Passband>,
    state: AudioState,
) {
    af_fft_styled(ui, rect, frame, passband, state, default_style())
}

/// [`af_fft`] with the caller's own palette.
pub fn af_fft_styled(
    ui: &Ui,
    rect: Rect,
    frame: Option<&AudioSpectrumFrame>,
    passband: Option<Passband>,
    state: AudioState,
    style: AfStyle,
) {
    let painter = ui.painter();
    painter.rect_filled(rect, Rounding::ZERO, style.trough);

    let live = frame.filter(|_| state.is_streaming());

    // One bar per two pixels, but never finer than the frame can answer:
    // a panel with more columns than bins leaves a gap in each column no
    // bin lands in, and the comb reads as structure in the signal rather
    // than as the display outrunning its data.
    let drawable = ((rect.width() / 2.0) as usize).max(1);
    let bars = match live {
        Some(f) => drawable.min(cat_ui::af::resolvable_columns(f)),
        None => drawable,
    };
    let bar_w = rect.width() / bars as f32;

    if let Some(frame) = live {
        for (i, height) in af::columns(frame, bars).into_iter().enumerate() {
            // `None` is not zero: the frame has nothing to say about this
            // column, so nothing is drawn there.
            let Some(height) = height else { continue };
            let h = rect.height() * height;
            if h <= 0.0 {
                continue;
            }
            let x = rect.left() + i as f32 * bar_w;
            painter.rect_filled(
                Rect::from_min_max(
                    egui::pos2(x, rect.bottom() - h),
                    egui::pos2(x + bar_w, rect.bottom()),
                ),
                Rounding::ZERO,
                style.ink,
            );
        }
    } else {
        // The absent state: a floor line, so the panel reads as an
        // instrument with nothing in it rather than as a hole.
        painter.line_segment(
            [
                egui::pos2(rect.left(), rect.bottom() - 0.5),
                egui::pos2(rect.right(), rect.bottom() - 0.5),
            ],
            Stroke::new(1.0, style.dim),
        );
    }

    // The filter edges last, so they are never buried under a bar. Drawn
    // whatever the state: where the radio's filter sits is true even when
    // no audio is arriving.
    if let Some(passband) = passband {
        let (low, high) = passband.edge_columns(bars);
        for edge in [low, high].into_iter().flatten() {
            let x = rect.left() + edge as f32 * bar_w;
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                Stroke::new(1.0, style.setting),
            );
        }
    }
}

/// A palette that reads on a dark console without the caller choosing one.
fn default_style() -> AfStyle {
    AfStyle {
        ink: Color32::from_rgb(0x4e, 0xc9, 0x7e),
        dim: Color32::from_rgb(0x46, 0x53, 0x5d),
        trough: Color32::from_rgb(0x0a, 0x0d, 0x10),
        setting: Color32::from_rgb(0xe6, 0xab, 0x44),
    }
}
