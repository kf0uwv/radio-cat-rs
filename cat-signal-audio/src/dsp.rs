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

//! One block of normalized audio in, one [`AudioFrame`] out.
//!
//! Platform-independent, socket-free and thread-free, for the reason
//! `cat-signal-rtlsdr`'s `dsp` module is: the arithmetic that is easy to
//! get silently wrong should be testable without hardware.
//!
//! # One block produces both frames
//!
//! The scope trace and the AF spectrum are computed from the *same*
//! samples and carry the *same* sequence number, so a console cannot show
//! a trace and a spectrum that disagree about what the radio was doing.
//! That is why [`AudioPipeline::process`] returns a pair rather than
//! offering two independent methods.
//!
//! # The scope window is the FFT block
//!
//! Deliberately, and it is the decision most worth knowing about. At 48 kHz
//! a 1024-sample block is 21.3 ms of audio and 46.9 frames per second — a
//! readable scope window and a comfortable console frame rate at the same
//! setting. Raising `fft_size` buys frequency resolution and spends both
//! time resolution and frame rate; that trade is real, it is visible in
//! [`AudioScopeFrame::window_ms`], and it is the same trade
//! `cat-signal-rtlsdr` exposes. A second, independent block size would have
//! meant two rates, two buffers and two things to explain, for a scope
//! window nobody has yet asked to set separately.
//!
//! See `docs/adr/0017-acc2-audio-source.md` §3.

use cat_signal::{AudioScopeFrame, AudioSpectrumFrame};
use rustfft::{num_complex::Complex32, FftPlanner};

use crate::AudioFrame;

/// The tunable shape of the pipeline.
///
/// Shared with the reader thread behind a mutex, so a `apply()` from the
/// console side takes effect on the next block rather than needing the
/// stream to be torn down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioPipelineConfig {
    /// Samples per second on the wire. 48 000 for the ACC2 pair.
    pub sample_rate_hz: u32,
    /// FFT length, and therefore the block size and the scope window.
    pub fft_size: usize,
    /// How much of the spectrum to report, from 0 Hz up.
    ///
    /// Clamped to Nyquist. The default is 4 kHz because that is the useful
    /// part of a communications receiver's audio; the other 20 kHz of a
    /// 48 kHz stream is empty, and drawing it wastes five sixths of the
    /// display on nothing.
    pub span_hz: u32,
    /// Exponential smoothing of the **spectrum only**, 0.0 (off) to 0.95.
    ///
    /// Never applied to the scope trace: a time-domain trace that has been
    /// averaged with earlier traces is not a waveform, it is an artefact,
    /// and it hides exactly the transient a scope is being watched for.
    pub averaging: f32,
}

impl Default for AudioPipelineConfig {
    fn default() -> Self {
        Self {
            sample_rate_hz: 48_000,
            fft_size: 1024,
            span_hz: 4_000,
            averaging: 0.0,
        }
    }
}

/// Normalized audio blocks in, paired scope + spectrum frames out.
pub struct AudioPipeline {
    config: AudioPipelineConfig,
    window: Vec<f32>,
    /// Sum of the window, for amplitude normalization.
    window_gain: f32,
    planner: FftPlanner<f32>,
    scratch: Vec<Complex32>,
    /// Smoothed dB bins across the whole half-spectrum, before truncation.
    averaged: Vec<f32>,
    sequence: u64,
}

impl AudioPipeline {
    pub fn new(config: AudioPipelineConfig) -> Self {
        let window = hann(config.fft_size);
        let window_gain = window.iter().sum::<f32>().max(f32::MIN_POSITIVE);
        Self {
            config,
            window,
            window_gain,
            planner: FftPlanner::new(),
            scratch: Vec::new(),
            averaged: Vec::new(),
            sequence: 0,
        }
    }

    pub fn config(&self) -> AudioPipelineConfig {
        self.config
    }

    /// How many samples [`process`](Self::process) needs for one frame.
    pub fn block_size(&self) -> usize {
        self.config.fft_size
    }

    /// Adopt a new configuration.
    ///
    /// Rebuilds the window and discards the averaging history when the FFT
    /// size changes: smoothing bins of one width into bins of another
    /// produces a plausible-looking spectrum that means nothing.
    pub fn reconfigure(&mut self, config: AudioPipelineConfig) {
        if config.fft_size != self.config.fft_size {
            self.window = hann(config.fft_size);
            self.window_gain = self.window.iter().sum::<f32>().max(f32::MIN_POSITIVE);
            self.averaged.clear();
        }
        self.config = config;
    }

    /// Bins actually reported, given the span and the FFT size.
    pub fn reported_bins(&self) -> usize {
        let half = self.config.fft_size / 2;
        if half == 0 {
            return 0;
        }
        let bin_width = f64::from(self.config.sample_rate_hz) / self.config.fft_size as f64;
        let wanted = (f64::from(self.config.span_hz) / bin_width).ceil() as usize;
        wanted.clamp(1, half)
    }

    /// Turn one block into a scope trace and an AF spectrum.
    ///
    /// Returns `None` if `samples` is shorter than the block size — a short
    /// read is not an error, it is just not yet a frame. The sequence
    /// number is **not** consumed in that case, so a gap in `sequence`
    /// always means a frame was genuinely dropped.
    pub fn process(&mut self, samples: &[f32]) -> Option<AudioFrame> {
        let n = self.config.fft_size;
        if n == 0 || samples.len() < n {
            return None;
        }
        let block = &samples[..n];

        self.scratch.clear();
        self.scratch.extend(
            block
                .iter()
                .zip(&self.window)
                .map(|(s, w)| Complex32::new(s * w, 0.0)),
        );
        self.planner.plan_fft_forward(n).process(&mut self.scratch);

        // Real input, so the FFT is conjugate-symmetric and only the first
        // half carries information. Bins 1..n/2 are doubled to account for
        // the mirrored energy discarded with the upper half, so a
        // full-scale sine reads 0 dB rather than -6.
        let half = n / 2;
        if self.averaged.len() != half {
            self.averaged.clear();
        }
        let alpha = self.config.averaging.clamp(0.0, 0.95);
        let mut bins = Vec::with_capacity(half);
        for (index, c) in self.scratch[..half].iter().enumerate() {
            let scale = if index == 0 { 1.0 } else { 2.0 };
            let magnitude = c.norm() * scale / self.window_gain;
            // Floored so a null bin does not become -inf and poison a
            // renderer's autoscale -- the same floor the RF pipeline uses.
            let db = if magnitude <= 1e-12 {
                -200.0
            } else {
                20.0 * magnitude.log10()
            };
            let smoothed = match self.averaged.get(index) {
                Some(previous) if alpha > 0.0 => alpha * previous + (1.0 - alpha) * db,
                _ => db,
            };
            bins.push(smoothed);
        }
        self.averaged.clone_from(&bins);

        let reported = self.reported_bins();
        bins.truncate(reported);
        let bin_width = f64::from(self.config.sample_rate_hz) / n as f64;

        self.sequence += 1;
        Some(AudioFrame {
            scope: AudioScopeFrame {
                sample_rate_hz: self.config.sample_rate_hz,
                samples: block.to_vec(),
                sequence: self.sequence,
            },
            spectrum: AudioSpectrumFrame {
                // Baseband audio: bin 0 starts at DC. `start_hz` earns its
                // keep the day someone wants a zoomed AF view; reporting
                // anything but 0 here today would be a lie.
                start_hz: 0,
                // The span actually covered by the bins reported, not the
                // span that was asked for -- so `bin_width_hz()` on the
                // frame agrees with the FFT that produced it.
                span_hz: (reported as f64 * bin_width).round() as u32,
                bins,
                sequence: self.sequence,
            },
        })
    }
}

/// Hann window, matching `cat-signal-rtlsdr`'s RF pipeline.
///
/// The same choice for the same reason: -31 dB sidelobes and a 1.5-bin main
/// lobe is the right default for looking at speech and carriers, and a
/// selectable window is a `SettingDescriptor` away if anyone ever wants
/// one.
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

    const RATE: u32 = 48_000;

    fn pipeline(span_hz: u32) -> AudioPipeline {
        AudioPipeline::new(AudioPipelineConfig {
            sample_rate_hz: RATE,
            fft_size: 1024,
            span_hz,
            averaging: 0.0,
        })
    }

    /// A real sine of `amplitude` at `hz`.
    fn tone(len: usize, hz: f64, amplitude: f32) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let p = std::f64::consts::TAU * hz * i as f64 / f64::from(RATE);
                amplitude * p.sin() as f32
            })
            .collect()
    }

    fn peak_bin(frame: &AudioSpectrumFrame) -> usize {
        frame
            .bins
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0
    }

    #[test]
    fn a_tone_lands_at_the_audio_frequency_it_was_generated_at() {
        // The load-bearing assertion for the AF display: 1 kHz of speech
        // must be drawn at 1 kHz.
        let mut p = pipeline(4_000);
        let f = p.process(&tone(1024, 1_000.0, 0.5)).unwrap();
        let hz = f
            .spectrum
            .audio_frequency_hz(peak_bin(&f.spectrum))
            .unwrap();
        assert!(
            (hz - 1_000.0).abs() < f.spectrum.bin_width_hz() * 2.0,
            "peak reported at {hz:.0} Hz, expected 1000"
        );
    }

    #[test]
    fn bins_stay_low_frequency_first() {
        let mut p = pipeline(4_000);
        let f = p.process(&tone(1024, 300.0, 0.5)).unwrap();
        let s = &f.spectrum;
        assert!(s.audio_frequency_hz(0).unwrap() < s.audio_frequency_hz(s.bins.len() - 1).unwrap());
        // ...and a low tone is in the low bins, which is the same claim
        // stated where a mirrored axis would fail it.
        assert!(peak_bin(s) < s.bins.len() / 4);
    }

    #[test]
    fn a_full_scale_sine_reads_about_zero_db() {
        // The one-sided doubling, asserted. Without it every level in the
        // display is 6 dB pessimistic, which is invisible until someone
        // compares it against a real meter.
        let mut p = pipeline(4_000);
        let f = p.process(&tone(1024, 1_000.0, 1.0)).unwrap();
        let level = f.spectrum.bins[peak_bin(&f.spectrum)];
        assert!(level.abs() < 1.5, "full-scale sine read {level} dB");
    }

    #[test]
    fn halving_the_amplitude_costs_six_db() {
        let mut p = pipeline(4_000);
        let loud = p.process(&tone(1024, 1_000.0, 1.0)).unwrap();
        let quiet = p.process(&tone(1024, 1_000.0, 0.5)).unwrap();
        let delta = loud.spectrum.bins[peak_bin(&loud.spectrum)]
            - quiet.spectrum.bins[peak_bin(&quiet.spectrum)];
        assert!((delta - 6.02).abs() < 0.5, "expected ~6 dB, got {delta}");
    }

    #[test]
    fn digital_silence_is_a_floor_not_negative_infinity() {
        // A transmitting radio sends digital silence on ANO. It must
        // produce an ordinary frame, not -inf bins that poison autoscale.
        let mut p = pipeline(4_000);
        let f = p.process(&vec![0.0; 1024]).unwrap();
        assert!(f.spectrum.bins.iter().all(|b| b.is_finite()));
        assert!(f.spectrum.bins.iter().all(|b| *b <= -100.0));
        assert_eq!(f.scope.peak(), 0.0);
        assert!(!f.scope.is_clipping());
    }

    #[test]
    fn the_span_reported_is_the_span_the_bins_actually_cover() {
        // If `span_hz` were echoed back unchanged, `bin_width_hz()` on the
        // frame would disagree with the FFT and every frequency a console
        // computed from it would be slightly wrong.
        let mut p = pipeline(4_000);
        let f = p.process(&tone(1024, 1_000.0, 0.5)).unwrap();
        let bin_width = f64::from(RATE) / 1024.0; // 46.875 Hz
        assert_eq!(f.spectrum.bins.len(), 86); // ceil(4000 / 46.875)
        assert!((f.spectrum.bin_width_hz() - bin_width).abs() < 0.5);
        assert!(f.spectrum.span_hz >= 4_000);
    }

    #[test]
    fn a_span_beyond_nyquist_is_clamped_rather_than_invented() {
        let mut p = AudioPipeline::new(AudioPipelineConfig {
            span_hz: 96_000,
            ..Default::default()
        });
        let f = p.process(&tone(1024, 1_000.0, 0.5)).unwrap();
        assert_eq!(f.spectrum.bins.len(), 512);
        assert!(f.spectrum.span_hz <= RATE / 2);
    }

    #[test]
    fn the_scope_window_is_the_fft_block() {
        // ADR 0017 section 3, asserted: one setting moves both, and
        // `window_ms()` tells the truth about which one you have.
        let mut p = pipeline(4_000);
        let f = p.process(&tone(1024, 1_000.0, 0.5)).unwrap();
        assert_eq!(f.scope.samples.len(), 1024);
        assert!((f.scope.window_ms() - 21.33).abs() < 0.01);

        p.reconfigure(AudioPipelineConfig {
            fft_size: 2048,
            ..p.config()
        });
        let f = p.process(&tone(2048, 1_000.0, 0.5)).unwrap();
        assert_eq!(f.scope.samples.len(), 2048);
        assert!((f.scope.window_ms() - 42.67).abs() < 0.01);
    }

    #[test]
    fn scope_and_spectrum_describe_the_same_instant() {
        let mut p = pipeline(4_000);
        let f = p.process(&tone(1024, 1_000.0, 0.5)).unwrap();
        assert_eq!(f.scope.sequence, f.spectrum.sequence);
        // The trace is the raw block, un-windowed and un-averaged.
        assert_eq!(f.scope.samples[..], tone(1024, 1_000.0, 0.5)[..]);
    }

    #[test]
    fn a_short_block_is_not_a_frame_and_does_not_consume_a_sequence_number() {
        let mut p = pipeline(4_000);
        let a = p.process(&tone(1024, 1_000.0, 0.5)).unwrap().scope.sequence;
        assert!(p.process(&tone(100, 1_000.0, 0.5)).is_none());
        let b = p.process(&tone(1024, 1_000.0, 0.5)).unwrap().scope.sequence;
        assert_eq!(a + 1, b, "a short read must not look like a dropped frame");
    }

    #[test]
    fn averaging_smooths_the_spectrum_and_leaves_the_trace_alone() {
        // A time-domain trace averaged with earlier traces is not a
        // waveform; it hides the transient the scope is being watched for.
        let mut p = AudioPipeline::new(AudioPipelineConfig {
            averaging: 0.9,
            ..Default::default()
        });
        let loud = tone(1024, 1_000.0, 1.0);
        let quiet = vec![0.0f32; 1024];

        let first = p.process(&loud).unwrap();
        let second = p.process(&quiet).unwrap();

        let peak = peak_bin(&first.spectrum);
        // The spectrum still remembers the tone...
        assert!(
            second.spectrum.bins[peak] > first.spectrum.bins[peak] - 20.0,
            "averaging did not hold the level: {} then {}",
            first.spectrum.bins[peak],
            second.spectrum.bins[peak]
        );
        // ...but the trace is exactly the silence that was fed in.
        assert!(second.scope.samples.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn changing_the_fft_size_discards_the_averaging_history() {
        // Smoothing 46.875 Hz bins into 23.4 Hz bins produces a
        // plausible-looking spectrum that means nothing.
        let mut p = AudioPipeline::new(AudioPipelineConfig {
            averaging: 0.9,
            ..Default::default()
        });
        p.process(&tone(1024, 1_000.0, 1.0)).unwrap();
        p.reconfigure(AudioPipelineConfig {
            fft_size: 2048,
            averaging: 0.9,
            ..Default::default()
        });
        let f = p.process(&vec![0.0; 2048]).unwrap();
        assert!(
            f.spectrum.bins.iter().all(|b| *b <= -100.0),
            "stale averaging survived an FFT size change"
        );
    }

    #[test]
    fn a_clipped_capture_is_visible_on_the_trace() {
        let mut p = pipeline(4_000);
        let mut samples = tone(1024, 1_000.0, 0.5);
        samples[10] = 1.0;
        let f = p.process(&samples).unwrap();
        assert!(f.scope.is_clipping());
    }
}
