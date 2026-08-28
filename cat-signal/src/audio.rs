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

//! Audio-domain data: post-detector audio, not a slice of the band.
//!
//! ADR 0015 §6. These types exist so the **compiler** enforces what was
//! previously a method plus a doc comment asking consumers to check.
//!
//! # Why not just reuse `SpectrumFrame`
//!
//! [`crate::SpectrumFrame`] carries a `center_hz` and is retunable: it
//! describes a window onto the radio band, positioned by the dial. Audio
//! has neither property. Pushing an AF spectrum through it would make
//! `bin_frequency_hz()` return audio hertz that are indistinguishable from
//! RF hertz — a 1500 Hz tone reported as 1500 Hz of *band* — and
//! `retune()` would be meaningless on it.
//!
//! That is exactly the confusion
//! [`SignalCapability::AudioDerived`](crate::SignalCapability::AudioDerived)'s
//! `max_bandwidth_hz` was introduced to prevent, and a renderer that forgot
//! to check would produce something confidently wrong: a few kHz of speech
//! stretched across a whole band, drawn with the same authority as a real
//! panorama.
//!
//! With separate types the mistake stops being possible. A waterfall takes
//! a `SpectrumFrame`; an AF display takes one of these; neither signature
//! accepts the other.

/// Audio-domain spectrum — an AF FFT.
///
/// Note what is deliberately **absent**: no `center_hz`, so this cannot be
/// positioned on a band axis, and no retune concept, because audio does not
/// move when the dial does.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AudioSpectrumFrame {
    /// Frequency of the low edge of bin 0. Usually 0 for baseband audio.
    pub start_hz: u32,
    /// Total width covered. For a 3 kHz SSB passband this is ~3000, not the
    /// tens of kHz a panorama spans — which is the whole distinction.
    pub span_hz: u32,
    /// Bin magnitudes in dB, **low-frequency-first**, matching
    /// [`crate::SpectrumFrame`]'s invariant so a renderer's mapping code
    /// behaves the same in both domains.
    pub bins: Vec<f32>,
    pub sequence: u64,
}

impl AudioSpectrumFrame {
    /// Hz per bin, or 0 for an empty frame.
    pub fn bin_width_hz(&self) -> f64 {
        if self.bins.is_empty() {
            return 0.0;
        }
        f64::from(self.span_hz) / self.bins.len() as f64
    }

    /// Audio frequency at the centre of `index`.
    ///
    /// Named `audio_frequency_hz`, not `bin_frequency_hz`, so that a value
    /// read from here cannot be mistaken at the call site for an RF one.
    pub fn audio_frequency_hz(&self, index: usize) -> Option<f64> {
        if index >= self.bins.len() {
            return None;
        }
        Some(f64::from(self.start_hz) + (index as f64 + 0.5) * self.bin_width_hz())
    }
}

/// Time-domain audio — an AF scope trace.
///
/// ADR 0010 §4 gave `AudioDerived` only FFT-flavoured settings
/// (`input_device`, `fft_size`, `window`, `averaging`) because a scope was
/// never modelled at all. This is that shape.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AudioScopeFrame {
    pub sample_rate_hz: u32,
    /// Samples normalized to -1.0..=1.0. A renderer scales to its own
    /// height and needs no knowledge of the codec's bit depth.
    pub samples: Vec<f32>,
    pub sequence: u64,
}

impl AudioScopeFrame {
    /// How much time this trace covers, in milliseconds.
    pub fn window_ms(&self) -> f64 {
        if self.sample_rate_hz == 0 {
            return 0.0;
        }
        self.samples.len() as f64 * 1000.0 / f64::from(self.sample_rate_hz)
    }

    /// The largest absolute sample, for a clipping indicator.
    ///
    /// A scope whose trace is flat-topped and a scope whose gain is merely
    /// high look similar at low resolution; this is what lets a console
    /// tell an operator which one they have.
    pub fn peak(&self) -> f32 {
        self.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    /// Whether any sample reached full scale.
    pub fn is_clipping(&self) -> bool {
        self.peak() >= 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn af_spectrum(bins: Vec<f32>) -> AudioSpectrumFrame {
        AudioSpectrumFrame {
            start_hz: 0,
            span_hz: 3_000,
            bins,
            sequence: 1,
        }
    }

    #[test]
    fn audio_frequencies_are_audio_sized_not_band_sized() {
        // The distinction the type exists to make. An AF FFT's whole span
        // is 3 kHz; a panorama's is tens of kHz to megahertz.
        let f = af_spectrum(vec![-80.0; 512]);
        assert_eq!(f.audio_frequency_hz(0).unwrap().round(), 3.0);
        assert!(f.audio_frequency_hz(511).unwrap() < 3_000.0);
    }

    #[test]
    fn bins_are_low_frequency_first_here_too() {
        // Same invariant as SpectrumFrame, so a renderer's mapping code
        // behaves identically in both domains.
        let f = af_spectrum(vec![-80.0; 64]);
        let lo = f.audio_frequency_hz(0).unwrap();
        let hi = f.audio_frequency_hz(63).unwrap();
        assert!(lo < hi);
    }

    #[test]
    fn an_empty_audio_frame_does_not_divide_by_zero() {
        let f = af_spectrum(Vec::new());
        assert_eq!(f.bin_width_hz(), 0.0);
        assert!(f.audio_frequency_hz(0).is_none());
    }

    #[test]
    fn a_scope_frame_reports_its_own_window() {
        let f = AudioScopeFrame {
            sample_rate_hz: 48_000,
            samples: vec![0.0; 960],
            sequence: 1,
        };
        assert_eq!(f.window_ms(), 20.0);
    }

    #[test]
    fn clipping_is_distinguishable_from_merely_loud() {
        // At low resolution a flat-topped trace and a hot one look alike.
        // A console needs to tell an operator which they have.
        let loud = AudioScopeFrame {
            sample_rate_hz: 48_000,
            samples: vec![0.94, -0.91, 0.88],
            sequence: 1,
        };
        let clipped = AudioScopeFrame {
            sample_rate_hz: 48_000,
            samples: vec![1.0, -1.0, 0.5],
            sequence: 2,
        };
        assert!(!loud.is_clipping());
        assert!(clipped.is_clipping());
        assert!(loud.peak() < clipped.peak());
    }

    #[test]
    fn a_scope_with_no_sample_rate_does_not_divide_by_zero() {
        let f = AudioScopeFrame {
            sample_rate_hz: 0,
            samples: vec![0.0; 10],
            sequence: 1,
        };
        assert_eq!(f.window_ms(), 0.0);
    }

    /// The point of this module, stated as a compile-time fact.
    ///
    /// These functions exist to be *read*, not run: each takes exactly one
    /// domain, and the other will not typecheck. Before ADR 0015 §6 both
    /// were `SpectrumFrame` and the only thing standing between an AF
    /// spectrum and a band waterfall was a consumer remembering to call
    /// `is_band_panorama()`.
    #[test]
    fn the_two_domains_are_not_interchangeable() {
        fn draw_panorama(_: &crate::SpectrumFrame) {}
        fn draw_af_fft(_: &AudioSpectrumFrame) {}

        let rf = crate::SpectrumFrame {
            center_hz: 14_074_000,
            span_hz: 96_000,
            ref_level_dbm: -20.0,
            bins: vec![-100.0; 8],
            sequence: 1,
        };
        let af = af_spectrum(vec![-80.0; 8]);
        draw_panorama(&rf);
        draw_af_fft(&af);
        // draw_panorama(&af) does not compile, and that is the feature.

        // Only the RF frame can answer a question about the band.
        assert!(rf.bin_frequency_hz(0).unwrap() > 14_000_000.0);
        assert!(af.audio_frequency_hz(0).unwrap() < 3_000.0);
    }
}
