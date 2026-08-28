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

//! IQ samples in, one corrected [`SpectrumFrame`] out.
//!
//! Everything in this module is platform-independent and hardware-free,
//! which is deliberate: the corrections that are easy to get silently
//! wrong — FFT shift, IF inversion, trim — are exactly the ones that must
//! be testable without a radio on the bench.
//!
//! See `docs/adr/0014-rtlsdr-spectrum-source.md`.

use cat_signal::{IfTapConfig, SpectrumFrame};
use rustfft::{num_complex::Complex32, FftPlanner};

/// Turns interleaved IQ into corrected spectrum frames.
pub struct SpectrumPipeline {
    fft_size: usize,
    window: Vec<f32>,
    /// Sum of the window, for amplitude normalization.
    window_gain: f32,
    planner: FftPlanner<f32>,
    config: IfTapConfig,
    sample_rate_hz: u32,
    dial_hz: u64,
    sequence: u64,
}

impl SpectrumPipeline {
    pub fn new(fft_size: usize, sample_rate_hz: u32, config: IfTapConfig) -> Self {
        let window = hann(fft_size);
        let window_gain = window.iter().sum::<f32>().max(f32::MIN_POSITIVE);
        Self {
            fft_size,
            window,
            window_gain,
            planner: FftPlanner::new(),
            config,
            sample_rate_hz,
            dial_hz: 0,
            sequence: 0,
        }
    }

    /// Note the radio's dial position.
    ///
    /// This is the whole of `retune`. **No frequency reaches the SDR** —
    /// the dongle stays parked on the IF, which is why `trim_hz` is a
    /// constant rather than a ppm figure. See ADR 0014 §5.
    pub fn set_dial_hz(&mut self, dial_hz: u64) {
        self.dial_hz = dial_hz;
    }

    pub fn dial_hz(&self) -> u64 {
        self.dial_hz
    }

    pub fn config(&self) -> IfTapConfig {
        self.config
    }

    pub fn set_trim_hz(&mut self, trim_hz: i32) {
        self.config.trim_hz = trim_hz;
    }

    pub fn fft_size(&self) -> usize {
        self.fft_size
    }

    pub fn set_fft_size(&mut self, fft_size: usize) {
        self.fft_size = fft_size;
        self.window = hann(fft_size);
        self.window_gain = self.window.iter().sum::<f32>().max(f32::MIN_POSITIVE);
    }

    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    /// The centre frequency this pipeline reports, in real RF terms.
    ///
    /// `dial + trim`, and nothing else. The IF never appears: a consumer
    /// of a `SpectrumFrame` must never learn that this radio has one.
    pub fn center_hz(&self) -> u64 {
        self.dial_hz
            .saturating_add_signed(i64::from(self.config.trim_hz))
    }

    /// Process one block of IQ into a frame.
    ///
    /// Returns `None` if `iq` is shorter than the FFT size — a short read
    /// is not an error, just not yet a frame.
    pub fn process(&mut self, iq: &[Complex32]) -> Option<SpectrumFrame> {
        if iq.len() < self.fft_size {
            return None;
        }

        let mut buffer: Vec<Complex32> = iq[..self.fft_size]
            .iter()
            .zip(&self.window)
            .map(|(sample, w)| sample * *w)
            .collect();

        self.planner
            .plan_fft_forward(self.fft_size)
            .process(&mut buffer);

        // FFT output is [DC..+Nyquist, -Nyquist..DC). Rotating by half puts
        // the most negative frequency first, which is what
        // low-frequency-first means for a complex baseband spectrum.
        buffer.rotate_left(self.fft_size / 2);

        let mut bins: Vec<f32> = buffer
            .iter()
            .map(|c| {
                let magnitude = c.norm() / self.window_gain;
                // 20*log10 of a voltage ratio, floored so a null bin does
                // not become -inf and poison a renderer's autoscale.
                if magnitude <= 1e-12 {
                    -200.0
                } else {
                    20.0 * magnitude.log10()
                }
            })
            .collect();

        // The TS-570D's LO1 is high-side (73.05-103.05 MHz), so its tapped
        // IF spectrum is mirrored. Correct it HERE -- ADR 0010's invariant
        // is that no consumer ever learns this happened.
        if self.config.inverted {
            bins.reverse();
        }

        self.sequence += 1;

        Some(SpectrumFrame {
            center_hz: self.center_hz(),
            span_hz: self.sample_rate_hz,
            ref_level_dbm: 0.0,
            bins,
            sequence: self.sequence,
        })
    }
}

/// Hann window. Fixed for now; a selectable window is a `SettingDescriptor`
/// away, which is the point of the delegated-settings design.
fn hann(n: usize) -> Vec<f32> {
    if n <= 1 {
        return vec![1.0; n];
    }
    (0..n)
        .map(|i| {
            let x = std::f32::consts::PI * 2.0 * i as f32 / (n - 1) as f32;
            0.5 - 0.5 * x.cos()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TS570D: IfTapConfig = IfTapConfig {
        if_center_hz: 73_050_000,
        inverted: true,
        trim_hz: 0,
    };

    const NON_INVERTED: IfTapConfig = IfTapConfig {
        if_center_hz: 73_050_000,
        inverted: false,
        trim_hz: 0,
    };

    /// A complex tone at `offset_hz` from the sampled centre.
    fn tone(len: usize, sample_rate_hz: u32, offset_hz: f64) -> Vec<Complex32> {
        (0..len)
            .map(|i| {
                let phase =
                    std::f64::consts::TAU * offset_hz * i as f64 / f64::from(sample_rate_hz);
                Complex32::new(phase.cos() as f32, phase.sin() as f32)
            })
            .collect()
    }

    fn peak_bin(frame: &SpectrumFrame) -> usize {
        frame
            .bins
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap()
    }

    #[test]
    fn a_tone_above_the_dial_appears_to_the_right() {
        // THE test for this crate. ADR 0010's whole invariant in one
        // assertion: after inversion correction, a signal above the dial
        // must land in the upper half of the bins.
        let mut pipeline = SpectrumPipeline::new(1024, 240_000, TS570D);
        pipeline.set_dial_hz(14_074_000);

        // On an inverted IF, a signal ABOVE the dial appears BELOW centre
        // in the raw IQ. So the input tone is negative and the output peak
        // must still be on the right.
        let iq = tone(1024, 240_000, -60_000.0);
        let frame = pipeline.process(&iq).unwrap();

        let peak = peak_bin(&frame);
        assert!(
            peak > frame.bins.len() / 2,
            "a signal above the dial must render to the right, got bin {peak} of {}",
            frame.bins.len()
        );
        assert!(frame.bin_frequency_hz(peak).unwrap() > frame.center_hz as f64);
    }

    #[test]
    fn inversion_actually_mirrors_and_is_not_a_no_op() {
        // Guards against the correction being silently dropped: the same
        // input through an inverted and a non-inverted pipeline must land
        // on opposite sides.
        let iq = tone(1024, 240_000, -60_000.0);

        let mut inverted = SpectrumPipeline::new(1024, 240_000, TS570D);
        inverted.set_dial_hz(14_074_000);
        let a = peak_bin(&inverted.process(&iq).unwrap());

        let mut plain = SpectrumPipeline::new(1024, 240_000, NON_INVERTED);
        plain.set_dial_hz(14_074_000);
        let b = peak_bin(&plain.process(&iq).unwrap());

        assert_ne!(a, b);
        assert_eq!(a + b, 1023, "mirroring should reflect about the centre");
        assert!(b < 512, "raw: negative offset is on the left");
        assert!(a > 512, "corrected: the same signal is on the right");
    }

    #[test]
    fn a_tone_lands_in_the_bin_its_frequency_predicts() {
        let mut pipeline = SpectrumPipeline::new(1024, 240_000, NON_INVERTED);
        pipeline.set_dial_hz(14_074_000);
        let frame = pipeline.process(&tone(1024, 240_000, 30_000.0)).unwrap();

        let peak = peak_bin(&frame);
        let observed = frame.bin_frequency_hz(peak).unwrap();
        let expected = 14_074_000.0 + 30_000.0;
        assert!(
            (observed - expected).abs() < frame.bin_width_hz() * 2.0,
            "peak at {observed} Hz, expected near {expected} Hz"
        );
    }

    #[test]
    fn dc_lands_at_the_centre_bin() {
        let mut pipeline = SpectrumPipeline::new(1024, 240_000, NON_INVERTED);
        pipeline.set_dial_hz(14_074_000);
        let frame = pipeline.process(&tone(1024, 240_000, 0.0)).unwrap();
        // The rotate_left is what makes this true; without it DC sits at
        // bin 0 and the whole spectrum is half a span out of place.
        assert!((peak_bin(&frame) as i64 - 512).abs() <= 1);
    }

    #[test]
    fn retune_moves_the_reported_centre_and_nothing_else() {
        let mut pipeline = SpectrumPipeline::new(256, 240_000, TS570D);
        pipeline.set_dial_hz(14_074_000);
        let a = pipeline.process(&tone(256, 240_000, -50_000.0)).unwrap();

        pipeline.set_dial_hz(7_100_000);
        let b = pipeline.process(&tone(256, 240_000, -50_000.0)).unwrap();

        assert_eq!(a.center_hz, 14_074_000);
        assert_eq!(b.center_hz, 7_100_000);
        // Span is a property of the sample rate, not the dial. If retune
        // ever starts touching the device, this is what changes first.
        assert_eq!(a.span_hz, b.span_hz);
        assert_eq!(peak_bin(&a), peak_bin(&b));
    }

    #[test]
    fn trim_offsets_the_centre_by_a_constant_at_any_dial_frequency() {
        // The claim that justifies trim_hz being a single number rather
        // than a ppm figure: the offset does not scale with frequency,
        // because the dongle never retunes.
        let mut pipeline = SpectrumPipeline::new(256, 240_000, TS570D);
        pipeline.set_trim_hz(-1_240);

        pipeline.set_dial_hz(3_500_000);
        assert_eq!(pipeline.center_hz(), 3_500_000 - 1_240);

        pipeline.set_dial_hz(28_000_000);
        assert_eq!(pipeline.center_hz(), 28_000_000 - 1_240);
    }

    #[test]
    fn a_positive_trim_moves_the_centre_up() {
        let mut pipeline = SpectrumPipeline::new(256, 240_000, TS570D);
        pipeline.set_dial_hz(14_074_000);
        pipeline.set_trim_hz(500);
        assert_eq!(pipeline.center_hz(), 14_074_500);
    }

    #[test]
    fn the_if_frequency_never_appears_in_a_frame() {
        // ADR 0010's normalization promise. If 73.05 MHz can be seen from
        // outside, a consumer will eventually depend on it.
        let mut pipeline = SpectrumPipeline::new(256, 240_000, TS570D);
        pipeline.set_dial_hz(14_074_000);
        let frame = pipeline.process(&tone(256, 240_000, 0.0)).unwrap();
        assert_eq!(frame.center_hz, 14_074_000);
        let (low, high) = frame.range_hz();
        assert!(high < 73_050_000.0, "the IF leaked into the frame");
        assert!(low > 0.0);
    }

    #[test]
    fn a_short_read_is_not_a_frame_and_not_an_error() {
        let mut pipeline = SpectrumPipeline::new(1024, 240_000, TS570D);
        assert!(pipeline.process(&tone(100, 240_000, 0.0)).is_none());
        assert!(pipeline.process(&tone(1024, 240_000, 0.0)).is_some());
    }

    #[test]
    fn sequence_numbers_advance_only_on_real_frames() {
        let mut pipeline = SpectrumPipeline::new(256, 240_000, TS570D);
        let a = pipeline.process(&tone(256, 240_000, 0.0)).unwrap().sequence;
        assert!(pipeline.process(&tone(10, 240_000, 0.0)).is_none());
        let b = pipeline.process(&tone(256, 240_000, 0.0)).unwrap().sequence;
        assert_eq!(a + 1, b, "a short read must not consume a sequence number");
    }

    #[test]
    fn a_silent_input_produces_a_floor_not_negative_infinity() {
        let mut pipeline = SpectrumPipeline::new(256, 240_000, TS570D);
        let silence = vec![Complex32::new(0.0, 0.0); 256];
        let frame = pipeline.process(&silence).unwrap();
        assert!(frame.bins.iter().all(|b| b.is_finite()));
        assert!(frame.bins.iter().all(|b| *b <= -100.0));
    }

    #[test]
    fn changing_fft_size_rebuilds_the_window() {
        let mut pipeline = SpectrumPipeline::new(256, 240_000, NON_INVERTED);
        pipeline.set_dial_hz(14_074_000);
        assert_eq!(
            pipeline
                .process(&tone(256, 240_000, 0.0))
                .unwrap()
                .bins
                .len(),
            256
        );

        pipeline.set_fft_size(1024);
        let frame = pipeline.process(&tone(1024, 240_000, 0.0)).unwrap();
        assert_eq!(frame.bins.len(), 1024);
        // A stale window of the old length would have made this wrong.
        assert!((peak_bin(&frame) as i64 - 512).abs() <= 1);
    }
}
