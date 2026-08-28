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

//! The waterfall: a scrolling texture fed by `SpectrumFrame`s.

use cat_signal::SpectrumFrame;
use cat_ui::{sample_column, Sample};

/// Maps 0.0-1.0 intensity to colour.
///
/// Kept as a small ordered stop list with linear interpolation between
/// stops rather than a closed-form colormap: it is trivially auditable, it
/// matches what [`cat_ui_ratatui`](https://docs.rs) picks from a terminal
/// palette, and it can be handed to a shader as a lookup table unchanged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    stops: &'static [[u8; 3]],
}

impl Default for Palette {
    fn default() -> Self {
        Self::TURBO
    }
}

impl Palette {
    /// Blue through green to red, the panadapter convention.
    pub const TURBO: Self = Self {
        stops: &[
            [12, 12, 60],
            [16, 40, 140],
            [0, 130, 190],
            [0, 190, 170],
            [40, 220, 80],
            [170, 235, 40],
            [245, 220, 40],
            [250, 150, 30],
            [235, 70, 30],
            [220, 30, 30],
        ],
    };

    /// Monochrome, for a console that wants the waterfall to sit quietly
    /// behind other information.
    pub const MONO: Self = Self {
        stops: &[[8, 10, 12], [70, 78, 86], [140, 152, 162], [235, 240, 245]],
    };

    /// Colour for a 0.0-1.0 intensity, clamped, interpolated between stops.
    pub fn at(&self, t: f32) -> [u8; 3] {
        if self.stops.is_empty() {
            return [0, 0, 0];
        }
        if self.stops.len() == 1 {
            return self.stops[0];
        }
        let t = t.clamp(0.0, 1.0);
        let scaled = t * (self.stops.len() - 1) as f32;
        let i = scaled.floor() as usize;
        // At t == 1.0 the floor lands on the last stop; there is no stop
        // after it to interpolate toward.
        if i >= self.stops.len() - 1 {
            return self.stops[self.stops.len() - 1];
        }
        let f = scaled - i as f32;
        let (a, b) = (self.stops[i], self.stops[i + 1]);
        [
            lerp(a[0], b[0], f),
            lerp(a[1], b[1], f),
            lerp(a[2], b[2], f),
        ]
    }
}

fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8
}

/// A scrolling RGBA image built from spectrum frames.
///
/// Rows are stored in a ring, so pushing a frame costs one row of writes
/// and an index bump rather than moving the whole image. [`rgba`] presents
/// it newest-row-first, which is the order a waterfall is drawn and the
/// order a texture upload wants.
pub struct WaterfallImage {
    width: u32,
    height: u32,
    /// RGBA, `width * height * 4`, in ring order.
    pixels: Vec<u8>,
    /// Row index that currently holds the newest frame.
    head: usize,
    rows_filled: u32,
    palette: Palette,
    floor_dbm: f32,
    /// Colour for a column no row ever captured. Deliberately not the
    /// palette's cold end: measured silence and no measurement at all are
    /// different facts, and a waterfall that conflates them invents signal
    /// history that never existed.
    no_data: [u8; 3],
    /// The frame every other row is re-projected onto.
    reference: Option<SpectrumFrame>,
}

impl WaterfallImage {
    pub fn new(width: u32, height: u32, palette: Palette, floor_dbm: f32) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        Self {
            width,
            height,
            pixels: vec![0; (width * height * 4) as usize],
            head: 0,
            rows_filled: 0,
            palette,
            floor_dbm,
            no_data: [26, 30, 34],
            reference: None,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// How many rows hold real frames. Below `height`, the rest is unwritten.
    pub fn rows_filled(&self) -> u32 {
        self.rows_filled
    }

    pub fn set_no_data_color(&mut self, rgb: [u8; 3]) {
        self.no_data = rgb;
    }

    /// The frame the axis currently belongs to, if any.
    pub fn reference(&self) -> Option<&SpectrumFrame> {
        self.reference.as_ref()
    }

    /// Add a frame as the newest row.
    ///
    /// The new frame becomes the reference, so the axis always describes
    /// what is on screen now. Older rows are **not** rewritten — they keep
    /// the pixels they were given. That is a deliberate trade: re-projecting
    /// the whole image on every frame would cost the scroll its cheapness,
    /// and a row's error only grows once the dial moves, which is exactly
    /// when the staircase of `no_data` at its edges becomes the visible
    /// record of that move.
    pub fn push(&mut self, frame: &SpectrumFrame) {
        let reference = frame.clone();
        self.head = if self.rows_filled == 0 {
            0
        } else {
            (self.head + self.height as usize - 1) % self.height as usize
        };
        let row = self.head;
        for col in 0..self.width {
            let rgb = match sample_column(frame, &reference, col, self.width, self.floor_dbm) {
                Sample::Signal(t) => self.palette.at(t),
                Sample::NoData => self.no_data,
            };
            let i = ((row as u32 * self.width + col) * 4) as usize;
            self.pixels[i] = rgb[0];
            self.pixels[i + 1] = rgb[1];
            self.pixels[i + 2] = rgb[2];
            self.pixels[i + 3] = 255;
        }
        self.rows_filled = (self.rows_filled + 1).min(self.height);
        self.reference = Some(reference);
    }

    /// Rebuild the whole image from a newest-first history, re-projecting
    /// every row onto the newest frame's axis.
    ///
    /// This is the expensive, exact path: after a retune it is what makes a
    /// stationary signal hold a vertical column all the way down the
    /// scrollback instead of only above the seam. A console can call it on
    /// dial change and use [`push`](Self::push) the rest of the time.
    pub fn rebuild(&mut self, frames: &[SpectrumFrame]) {
        self.pixels.fill(0);
        self.rows_filled = 0;
        self.head = 0;
        let Some(reference) = frames.first().cloned() else {
            self.reference = None;
            return;
        };
        let rows = (frames.len() as u32).min(self.height);
        for (row, frame) in frames.iter().take(rows as usize).enumerate() {
            for col in 0..self.width {
                let rgb = match sample_column(frame, &reference, col, self.width, self.floor_dbm) {
                    Sample::Signal(t) => self.palette.at(t),
                    Sample::NoData => self.no_data,
                };
                let i = ((row as u32 * self.width + col) * 4) as usize;
                self.pixels[i] = rgb[0];
                self.pixels[i + 1] = rgb[1];
                self.pixels[i + 2] = rgb[2];
                self.pixels[i + 3] = 255;
            }
        }
        self.rows_filled = rows;
        self.reference = Some(reference);
    }

    /// RGBA bytes, newest row first, ready for a texture upload.
    ///
    /// Allocates, because the ring's newest row is rarely row zero. A
    /// caller uploading every frame should prefer [`row_rgba`](Self::row_rgba)
    /// plus a partial texture write, which is what makes the scroll free.
    pub fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len());
        for r in 0..self.height {
            let src = ((self.head as u32 + r) % self.height) as usize;
            let start = src * (self.width * 4) as usize;
            out.extend_from_slice(&self.pixels[start..start + (self.width * 4) as usize]);
        }
        out
    }

    /// One row's RGBA, `0` being the newest.
    pub fn row_rgba(&self, row: u32) -> &[u8] {
        let src = ((self.head as u32 + row.min(self.height - 1)) % self.height) as usize;
        let start = src * (self.width * 4) as usize;
        &self.pixels[start..start + (self.width * 4) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(center_hz: u64, peak_bin: usize, bins: usize) -> SpectrumFrame {
        let mut v = vec![-110.0f32; bins];
        v[peak_bin] = -20.0;
        SpectrumFrame {
            center_hz,
            span_hz: 96_000,
            ref_level_dbm: -20.0,
            bins: v,
            sequence: 1,
        }
    }

    fn pixel(img: &WaterfallImage, row: u32, col: u32) -> [u8; 3] {
        let r = img.row_rgba(row);
        let i = (col * 4) as usize;
        [r[i], r[i + 1], r[i + 2]]
    }

    /// Brightest column of a row, by luminance.
    fn peak_column(img: &WaterfallImage, row: u32) -> u32 {
        (0..img.width())
            .max_by_key(|c| {
                let p = pixel(img, row, *c);
                u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])
            })
            .unwrap()
    }

    // -----------------------------------------------------------------
    // Orientation. A renderer can break it as easily as a source can.
    // -----------------------------------------------------------------

    #[test]
    fn a_signal_above_the_dial_lands_in_the_right_half() {
        // Bins are low-frequency-first, so a peak in the upper bins must
        // be drawn to the right. Reversing the loop would pass every other
        // test in this file.
        let mut img = WaterfallImage::new(64, 8, Palette::TURBO, -120.0);
        img.push(&frame(14_074_000, 200, 256));
        assert!(peak_column(&img, 0) > img.width() / 2);
    }

    #[test]
    fn a_signal_below_the_dial_lands_in_the_left_half() {
        let mut img = WaterfallImage::new(64, 8, Palette::TURBO, -120.0);
        img.push(&frame(14_074_000, 20, 256));
        assert!(peak_column(&img, 0) < img.width() / 2);
    }

    // -----------------------------------------------------------------
    // Scrolling.
    // -----------------------------------------------------------------

    #[test]
    fn the_newest_frame_is_row_zero() {
        let mut img = WaterfallImage::new(32, 4, Palette::TURBO, -120.0);
        img.push(&frame(14_074_000, 20, 256));
        let older = peak_column(&img, 0);
        img.push(&frame(14_074_000, 200, 256));
        let newest = peak_column(&img, 0);
        assert_ne!(newest, older, "row 0 must hold the frame just pushed");
        assert_eq!(
            peak_column(&img, 1),
            older,
            "the previous row scrolled down"
        );
    }

    #[test]
    fn rows_wrap_rather_than_growing() {
        // The ring is what makes a scroll an index bump. If this ever
        // starts allocating per frame, a 60 fps waterfall notices.
        let mut img = WaterfallImage::new(16, 4, Palette::TURBO, -120.0);
        let before = img.rgba().len();
        for i in 0..100 {
            img.push(&frame(14_074_000, i % 256, 256));
        }
        assert_eq!(img.rgba().len(), before);
        assert_eq!(img.rows_filled(), 4);
    }

    #[test]
    fn rows_filled_reports_how_much_is_real() {
        // A console needs to know the difference between a quiet band and
        // a waterfall that has only just started.
        let mut img = WaterfallImage::new(8, 6, Palette::TURBO, -120.0);
        assert_eq!(img.rows_filled(), 0);
        img.push(&frame(14_074_000, 4, 8));
        assert_eq!(img.rows_filled(), 1);
        for _ in 0..20 {
            img.push(&frame(14_074_000, 4, 8));
        }
        assert_eq!(img.rows_filled(), 6);
    }

    // -----------------------------------------------------------------
    // Re-projection, on the exact path.
    // -----------------------------------------------------------------

    #[test]
    fn rebuild_holds_a_stationary_signal_in_one_column() {
        // The point of re-projection. Three frames captured at three dial
        // positions, all containing the same absolute RF carrier; after a
        // rebuild they must line up vertically instead of stepping across
        // the image as the dial moved.
        //
        // 64 bins over a 96 kHz span is 1500 Hz per bin, and 64 columns
        // makes one bin exactly one column -- so a 1500 Hz dial move is a
        // clean one-column shift with no rounding to argue about.
        let width = 64;
        let mut img = WaterfallImage::new(width, 3, Palette::TURBO, -120.0);
        let frames = vec![
            frame(14_074_000, 32, 64),
            frame(14_074_000 - 1_500, 33, 64),
            frame(14_074_000 - 3_000, 34, 64),
        ];
        img.rebuild(&frames);
        let cols: Vec<u32> = (0..3).map(|r| peak_column(&img, r)).collect();
        assert_eq!(
            cols,
            vec![cols[0]; 3],
            "a stationary carrier drifted as the dial moved: {cols:?}"
        );
    }

    #[test]
    fn without_re_projection_the_same_carrier_would_drift() {
        // The control for the test above: the raw bin index does move, so
        // the alignment there is the re-projection working rather than the
        // frames happening to agree.
        let frames = [frame(14_074_000, 32, 64), frame(14_074_000 - 1_500, 33, 64)];
        let peak = |f: &SpectrumFrame| {
            f.bins
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .unwrap()
                .0
        };
        assert_ne!(peak(&frames[0]), peak(&frames[1]));
    }

    #[test]
    fn a_sub_column_dial_move_rounds_rather_than_smearing() {
        // A raster cannot shift by a fraction of a column. A quarter-column
        // dial move therefore rounds to no shift at all, which is honest --
        // the alternative would be interpolating history rows and inventing
        // intensities nothing measured.
        let a = frame(14_074_000, 32, 64);
        let b = frame(14_074_000 - 375, 32, 64);
        assert_eq!(cat_ui::projection_offset(&b, &a, 64), 0);
    }

    #[test]
    fn columns_a_row_never_captured_are_marked_no_data() {
        // Half a span away: half the row has no measurement. Filling it
        // with the palette's cold end would invent signal history.
        let mut img = WaterfallImage::new(64, 2, Palette::TURBO, -120.0);
        img.set_no_data_color([1, 2, 3]);
        img.rebuild(&[
            frame(14_074_000, 128, 256),
            frame(14_074_000 + 48_000, 128, 256),
        ]);
        assert_eq!(pixel(&img, 1, 0), [1, 2, 3]);
        assert_ne!(pixel(&img, 1, 63), [1, 2, 3]);
    }

    #[test]
    fn no_data_is_distinguishable_from_measured_silence() {
        // The distinction the whole design rests on. A silent band is a
        // measurement; a moved dial is the absence of one.
        let img = WaterfallImage::new(4, 1, Palette::TURBO, -120.0);
        let silence = img.palette.at(0.0);
        assert_ne!(silence, img.no_data);
    }

    #[test]
    fn a_band_sized_jump_leaves_the_history_honestly_empty() {
        // No row overlaps, so there is nothing true to draw. Showing stale
        // rows in the wrong place would be worse than showing none.
        let mut img = WaterfallImage::new(32, 2, Palette::TURBO, -120.0);
        img.set_no_data_color([1, 2, 3]);
        img.rebuild(&[frame(14_074_000, 128, 256), frame(7_100_000, 128, 256)]);
        assert!((0..32).all(|c| pixel(&img, 1, c) == [1, 2, 3]));
    }

    #[test]
    fn rebuilding_with_no_frames_clears_rather_than_keeping_a_stale_axis() {
        let mut img = WaterfallImage::new(8, 2, Palette::TURBO, -120.0);
        img.push(&frame(14_074_000, 4, 8));
        assert!(img.reference().is_some());
        img.rebuild(&[]);
        assert!(img.reference().is_none());
        assert_eq!(img.rows_filled(), 0);
    }

    #[test]
    fn more_frames_than_rows_keeps_the_newest() {
        let mut img = WaterfallImage::new(16, 2, Palette::TURBO, -120.0);
        let frames: Vec<SpectrumFrame> = (0..10).map(|i| frame(14_074_000, 20 + i, 256)).collect();
        img.rebuild(&frames);
        assert_eq!(img.rows_filled(), 2);
    }

    // -----------------------------------------------------------------
    // Palette.
    // -----------------------------------------------------------------

    #[test]
    fn the_palette_spans_its_stops_and_clamps_outside_them() {
        let p = Palette::TURBO;
        assert_eq!(p.at(0.0), [12, 12, 60]);
        assert_eq!(p.at(1.0), [220, 30, 30]);
        assert_eq!(p.at(-1.0), [12, 12, 60]);
        assert_eq!(p.at(2.0), [220, 30, 30]);
    }

    #[test]
    fn the_palette_interpolates_between_stops() {
        // Banding at ten stops would be visible on a real waterfall.
        let p = Palette::TURBO;
        let a = p.at(0.0);
        let mid = p.at(0.055);
        assert_ne!(a, mid);
    }

    #[test]
    fn a_monochrome_palette_still_has_range() {
        let p = Palette::MONO;
        assert_ne!(p.at(0.0), p.at(1.0));
    }

    #[test]
    fn a_zero_sized_image_is_promoted_rather_than_panicking() {
        let img = WaterfallImage::new(0, 0, Palette::TURBO, -120.0);
        assert_eq!(img.width(), 1);
        assert_eq!(img.height(), 1);
        assert_eq!(img.row_rgba(99).len(), 4);
    }
}
