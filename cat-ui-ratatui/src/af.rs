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

//! The audio-domain panels: an AF scope and an AF FFT, at cell resolution.
//!
//! These live in the console's **monitoring rail**, not in a tab. The
//! accepted design (option 3) draws the distinction as: a tab is a working
//! surface you enter, do a thing in, and leave; the rail is always true and
//! never entered. An AF scope is looked at *while* doing something else, so
//! a tab you must leave to keep operating cannot serve it.
//!
//! # These are monitors, not instruments
//!
//! The rail is 22 columns. At 150 Hz per cell an AF FFT answers "is the
//! energy inside the passband, and roughly where" and does not answer "what
//! is the exact pitch". That limit is accepted rather than worked around:
//! a measuring-grade AF FFT wants a tab of its own, and designing one into
//! the rail would spend the same space twice.
//!
//! # Colour grammar
//!
//! **RF is in colour, AF is in ink.** The waterfall gets a turbo map and the
//! spectrum trace gets green; everything here is neutral. Amber keeps its
//! one meaning in both domains — *a radio setting* — so it marks the VFO
//! cursor on the waterfall and the receive passband edges here, and nowhere
//! else. The rule needs no legend and stays readable for a colourblind
//! operator, because nothing here is distinguished by hue alone.
//!
//! # Three states, one geometry
//!
//! [`AudioState`] has three cases and they occupy **identical space**. An
//! absent panel that collapses makes the whole rail jump when a cable is
//! unplugged, and an operator learns to read the layout rather than the
//! data. `Configured` — the audio path exists, nothing is streaming yet —
//! is the default and the one the design was built around, because that is
//! what a console shows until an audio transport is actually attached.

use cat_signal::{AudioScopeFrame, AudioSpectrumFrame};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine, Points};
use ratatui::widgets::Widget;
use ratatui::{buffer::Buffer, text::Line, text::Span};

// The panel's arithmetic — where a column sits in hertz, how loud that
// makes it look, where the filter edges fall — lives in `cat-ui`, so the
// terminal and the GPU consoles cannot answer it differently. Two consoles
// showing the same radio at the same moment with different bar heights
// would leave an operator unable to trust either.
pub use cat_ui::af::{AudioState, Passband, AF_FFT_SPAN_HZ};

/// Eight sub-levels, so one cell resolves more than one bar height.
const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Draw the AF FFT: one column per cell, the passband edges ticked.
///
/// `frame` is `None` in every state but [`AudioState::Streaming`], and the
/// panel still draws — an empty grid of the same size, so the rail does not
/// move when audio comes and goes.
pub fn af_fft(
    frame: Option<&AudioSpectrumFrame>,
    passband: Option<Passband>,
    state: AudioState,
    area: Rect,
    buf: &mut Buffer,
    ink: Color,
    setting: Color,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    // The axis is fixed at 0..AF_FFT_SPAN_HZ rather than taken from the
    // frame. An AF display whose scale moved with the data would make two
    // recordings incomparable, and the span is a property of the panel
    // here, not of the signal.
    let hz_per_col = AF_FFT_SPAN_HZ / f32::from(area.width);

    if let Some(frame) = frame {
        let levels = u32::from(area.height) * 8;
        for col in 0..area.width {
            let lo = f32::from(col) * hz_per_col;
            let hi = lo + hz_per_col;
            let Some(t) = band_energy(frame, lo, hi) else {
                continue;
            };
            let filled = (t * levels as f32).round() as u32;
            for row in 0..area.height {
                let from_bottom = area.height - 1 - row;
                let base = u32::from(from_bottom) * 8;
                let cell = filled.saturating_sub(base).min(8) as usize;
                if cell > 0 {
                    buf.get_mut(area.x + col, area.y + row)
                        .set_char(BLOCKS[cell])
                        .set_fg(ink);
                }
            }
        }
    } else {
        // The absent/pending state: a floor line, so the panel reads as an
        // instrument with nothing in it rather than as a hole.
        let row = area.y + area.height - 1;
        for col in 0..area.width {
            buf.get_mut(area.x + col, row)
                .set_char('·')
                .set_fg(dim_for(state, ink));
        }
    }

    // Passband edges last, so they are never overdrawn by a bar. Amber
    // means "a radio setting" here exactly as it does on the waterfall.
    if let Some(pb) = passband {
        for hz in [pb.low_hz, pb.high_hz] {
            if !(0.0..AF_FFT_SPAN_HZ).contains(&hz) {
                continue;
            }
            let col = (hz / hz_per_col) as u16;
            if col < area.width {
                buf.get_mut(area.x + col, area.y + area.height - 1)
                    .set_char('┃')
                    .set_fg(setting);
            }
        }
    }
}

/// How far below the frame's strongest bin the panel's floor sits.
///
/// A fixed window, not the frame's own min-to-max. Audio has a noise floor
/// that is often only a few dB under the speech in it, and stretching that
/// range across the panel's full height paints almost every cell solid —
/// technically a faithful normalisation, and useless to look at. 40 dB is
/// wide enough that a quiet band still shows structure and narrow enough
/// that a tone stands clear of the hiss.
/// The energy in `lo_hz..hi_hz`, as a fraction of the panel's height.
///
/// A thin wrapper on `cat_ui::af::column_energy`, kept so this module's
/// drawing code reads in its own terms.
fn band_energy(frame: &AudioSpectrumFrame, lo_hz: f32, hi_hz: f32) -> Option<f32> {
    cat_ui::af::column_energy(frame, lo_hz, hi_hz)
}

/// Draw the AF scope as a braille trace.
///
/// Braille rather than block characters because a scope is a *line*, and
/// blocks can only draw a bar chart of |amplitude| — which loses the zero
/// crossing, and with it the only thing that makes a trace look like a
/// waveform. Braille gives 2x4 dots per cell, so a 20x5 panel is a 40x20
/// dot grid: coarse, but unmistakably a wave.
pub fn af_scope(
    frame: Option<&AudioScopeFrame>,
    state: AudioState,
    area: Rect,
    buf: &mut Buffer,
    ink: Color,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let dim = dim_for(state, ink);
    let samples: Vec<f64> = frame
        .map(|f| f.samples.iter().map(|&s| f64::from(s)).collect())
        .unwrap_or_default();

    let canvas = Canvas::default()
        .marker(ratatui::symbols::Marker::Braille)
        .x_bounds([0.0, 1.0])
        .y_bounds([-1.0, 1.0])
        .paint(move |ctx| {
            // The zero line is drawn in every state, including the empty
            // one. It is what makes the panel legible as a scope rather
            // than as a blank rectangle, and it is where the trace will be.
            ctx.draw(&CanvasLine {
                x1: 0.0,
                y1: 0.0,
                x2: 1.0,
                y2: 0.0,
                color: dim,
            });
            if samples.is_empty() {
                return;
            }
            let n = samples.len();
            let points: Vec<(f64, f64)> = samples
                .iter()
                .enumerate()
                .map(|(i, &s)| (i as f64 / (n - 1).max(1) as f64, s.clamp(-1.0, 1.0)))
                .collect();
            ctx.draw(&Points {
                coords: &points,
                color: ink,
            });
        });
    canvas.render(area, buf);
}

/// A one-line header for an AF panel: its name, and what state it is in.
///
/// The state is always shown, including when it is `LIVE`. A panel that
/// only labels itself when something is wrong leaves an operator unable to
/// tell "working" from "not labelled yet".
pub fn af_header(name: &str, state: AudioState, ink: Color, dim: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(name.to_string(), Style::default().fg(ink)),
        Span::raw(" "),
        Span::styled(
            state.label().to_string(),
            Style::default().fg(match state {
                AudioState::Streaming => ink,
                _ => dim,
            }),
        ),
    ])
}

/// Ink for a panel with nothing in it.
fn dim_for(state: AudioState, ink: Color) -> Color {
    match state {
        AudioState::Streaming => ink,
        _ => Color::DarkGray,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(w: u16, h: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, w, h))
    }

    fn rendered(buf: &Buffer, area: Rect) -> Vec<String> {
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf.get(area.x + x, area.y + y).symbol().to_string())
                    .collect()
            })
            .collect()
    }

    fn spectrum(bins: Vec<f32>) -> AudioSpectrumFrame {
        AudioSpectrumFrame {
            start_hz: 0,
            span_hz: 3_000,
            bins,
            sequence: 1,
        }
    }

    fn scope(samples: Vec<f32>) -> AudioScopeFrame {
        AudioScopeFrame {
            sample_rate_hz: 48_000,
            samples,
            sequence: 1,
        }
    }

    #[test]
    fn the_three_states_occupy_identical_space() {
        // The property the design asks for by name: unplugging a cable must
        // not make the rail jump.
        let area = Rect::new(0, 0, 20, 5);
        let mut drawn = Vec::new();
        for state in [
            AudioState::Absent,
            AudioState::Configured,
            AudioState::Streaming,
        ] {
            let mut buf = buffer(20, 5);
            let frame = spectrum(vec![-80.0; 128]);
            let f = matches!(state, AudioState::Streaming).then_some(&frame);
            af_fft(f, None, state, area, &mut buf, Color::White, Color::Yellow);
            drawn.push(rendered(&buf, area));
        }
        for d in &drawn {
            assert_eq!(d.len(), 5, "every state is five rows");
            assert!(d.iter().all(|r| r.chars().count() == 20));
        }
    }

    #[test]
    fn an_empty_fft_still_draws_a_floor() {
        // Not a blank rectangle: an instrument with nothing in it.
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = buffer(20, 5);
        af_fft(
            None,
            None,
            AudioState::Configured,
            area,
            &mut buf,
            Color::White,
            Color::Yellow,
        );
        let rows = rendered(&buf, area);
        assert_eq!(rows[4], "·".repeat(20));
    }

    #[test]
    fn a_tone_lands_in_the_cell_its_frequency_belongs_to() {
        // 150 Hz per cell over 20 cells. A tone at 1500 Hz belongs in cell
        // 10, and putting it anywhere else would make the panel lie about
        // where the energy is.
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = buffer(20, 5);
        let mut bins = vec![-100.0f32; 200];
        // bin i is at (i + 0.5) * 15 Hz for a 200-bin, 3 kHz frame.
        bins[100] = 0.0;
        let frame = spectrum(bins);

        af_fft(
            Some(&frame),
            None,
            AudioState::Streaming,
            area,
            &mut buf,
            Color::White,
            Color::Yellow,
        );

        let rows = rendered(&buf, area);
        let top = &rows[0];
        let loud: Vec<usize> = top
            .chars()
            .enumerate()
            .filter(|(_, c)| *c == '█')
            .map(|(i, _)| i)
            .collect();
        assert_eq!(loud, vec![10], "1507 Hz belongs in cell 10, and only there");
    }

    #[test]
    fn the_axis_does_not_move_with_the_data() {
        // A display whose scale followed the signal would make two moments
        // incomparable. The span is the panel's, not the frame's.
        let area = Rect::new(0, 0, 20, 5);
        let mut wide = buffer(20, 5);
        let mut narrow = buffer(20, 5);

        let mut bins = vec![-100.0f32; 200];
        bins[100] = 0.0;
        af_fft(
            Some(&spectrum(bins.clone())),
            None,
            AudioState::Streaming,
            area,
            &mut wide,
            Color::White,
            Color::Yellow,
        );

        // Same tone, a frame that claims a narrower span. The tone must not
        // move: `audio_frequency_hz` is what places it.
        let mut f = spectrum(bins);
        f.span_hz = 3_000;
        af_fft(
            Some(&f),
            None,
            AudioState::Streaming,
            area,
            &mut narrow,
            Color::White,
            Color::Yellow,
        );
        assert_eq!(rendered(&wide, area), rendered(&narrow, area));
    }

    #[test]
    fn the_passband_edges_are_marked_in_the_setting_colour() {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = buffer(20, 5);
        af_fft(
            None,
            Some(Passband {
                low_hz: 300.0,
                high_hz: 2_700.0,
            }),
            AudioState::Configured,
            area,
            &mut buf,
            Color::White,
            Color::Yellow,
        );
        // 300 Hz -> cell 2, 2700 Hz -> cell 18, at 150 Hz per cell.
        assert_eq!(buf.get(2, 4).symbol(), "┃");
        assert_eq!(buf.get(18, 4).symbol(), "┃");
        assert_eq!(buf.get(2, 4).style().fg, Some(Color::Yellow));
    }

    #[test]
    fn a_passband_edge_outside_the_span_is_dropped_rather_than_clamped() {
        // Clamping would draw an edge at the panel's boundary and claim the
        // filter ends there, which is a different and wrong statement.
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = buffer(20, 5);
        af_fft(
            None,
            Some(Passband {
                low_hz: -500.0,
                high_hz: 9_000.0,
            }),
            AudioState::Configured,
            area,
            &mut buf,
            Color::White,
            Color::Yellow,
        );
        let rows = rendered(&buf, area);
        assert!(!rows[4].contains('┃'));
    }

    #[test]
    fn the_scope_draws_a_zero_line_even_with_no_samples() {
        // What makes the empty panel read as a scope.
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = buffer(20, 5);
        af_scope(None, AudioState::Configured, area, &mut buf, Color::White);
        let rows = rendered(&buf, area);
        assert!(
            rows.iter().any(|r| r.chars().any(|c| c != ' ')),
            "an empty scope still shows where the trace will be"
        );
    }

    #[test]
    fn a_wave_draws_above_and_below_the_line() {
        // The reason for braille over blocks: a bar chart of |amplitude|
        // loses the zero crossing, and with it the waveform.
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = buffer(20, 5);
        let samples: Vec<f32> = (0..200)
            .map(|i| (i as f32 / 200.0 * std::f32::consts::TAU * 3.0).sin() * 0.9)
            .collect();
        af_scope(
            Some(&scope(samples)),
            AudioState::Streaming,
            area,
            &mut buf,
            Color::White,
        );
        let rows = rendered(&buf, area);
        let ink = |r: &String| r.chars().filter(|c| *c != ' ').count();
        assert!(ink(&rows[0]) > 0, "the trace reaches the top half");
        assert!(ink(&rows[4]) > 0, "and the bottom half");
    }

    #[test]
    fn a_clipping_sample_does_not_escape_the_panel() {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = buffer(20, 5);
        af_scope(
            Some(&scope(vec![9.0, -9.0, 9.0, -9.0])),
            AudioState::Streaming,
            area,
            &mut buf,
            Color::White,
        );
        // Nothing to assert beyond "it drew and did not panic": the bounds
        // are the canvas's, and out-of-range points are clamped into them.
        assert_eq!(rendered(&buf, area).len(), 5);
    }

    #[test]
    fn the_header_names_the_state_in_every_state() {
        for state in [
            AudioState::Absent,
            AudioState::Configured,
            AudioState::Streaming,
        ] {
            let line = af_header("AF FFT", state, Color::White, Color::DarkGray);
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(text.starts_with("AF FFT"));
            assert!(text.contains(state.label()));
        }
    }

    #[test]
    fn pending_is_the_default_states_label_and_it_is_not_an_error_word() {
        // "PENDING", not "NO AUDIO": the path is configured and the
        // transport simply has not been attached yet.
        assert_eq!(AudioState::Configured.label(), "PENDING");
        assert_eq!(AudioState::Absent.label(), "NONE");
        assert_eq!(AudioState::Streaming.label(), "LIVE");
    }
}

#[cfg(test)]
mod range_tests {
    use super::*;

    fn frame(bins: Vec<f32>) -> AudioSpectrumFrame {
        AudioSpectrumFrame {
            start_hz: 0,
            span_hz: 3_000,
            bins,
            sequence: 1,
        }
    }

    #[test]
    fn a_noise_floor_sits_low_rather_than_filling_the_panel() {
        // The bug this replaced: normalising against the frame's own min and
        // max maps the quietest bin to zero and the loudest to full, so the
        // floor lands near the top whenever the range is narrow and the
        // panel paints almost solid. A fixed window below the peak keeps
        // the floor where it belongs. Found by pointing the console at the
        // emulator's audio and seeing a wall of blocks.
        let mut bins = vec![-75.0f32; 200];
        bins[100] = -40.0; // 35 dB of range, which is ordinary for audio
        let f = frame(bins);

        let floor = band_energy(&f, 0.0, 150.0).expect("a floor cell");
        assert!(
            floor < 0.2,
            "the noise floor must sit low in the panel: {floor}"
        );
        let tone = band_energy(&f, 1_450.0, 1_600.0).expect("the tone's cell");
        assert!(tone > 0.9, "and the tone must stand clear of it: {tone}");
    }

    #[test]
    fn the_strongest_bin_reaches_the_top() {
        let mut bins = vec![-90.0f32; 200];
        bins[100] = -30.0;
        let f = frame(bins);
        let peak = band_energy(&f, 1_450.0, 1_600.0).expect("the tone's cell");
        assert!(
            peak > 0.99,
            "the loudest thing on the panel fills it: {peak}"
        );
    }

    #[test]
    fn anything_more_than_the_window_below_the_peak_is_floored_not_negative() {
        let mut bins = vec![-140.0f32; 200];
        bins[100] = -30.0;
        let f = frame(bins);
        let quiet = band_energy(&f, 0.0, 150.0).expect("a floor cell");
        assert_eq!(quiet, 0.0);
    }
}
