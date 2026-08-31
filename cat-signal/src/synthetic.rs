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

//! A band with signals in it, for testing a console against a dummy radio.
//!
//! [`FakeSpectrumSource`](crate::fake::FakeSpectrumSource) emits one peak
//! at a known offset, which is exactly right for checking *orientation*
//! and useless for anything else. This is the other thing: a populated
//! band, so a waterfall has something to look like and click-to-tune has
//! somewhere to tune to.
//!
//! # Signals live at absolute frequencies
//!
//! That is the property that makes this worth building rather than
//! sprinkling noise into a buffer. Each emitter has a real frequency, and
//! [`Band::render`] shows whatever part of the band the current window
//! covers. So retuning **moves the window over a fixed landscape**, the
//! way a radio does — a signal at 14.074 MHz is at 14.074 MHz whatever the
//! dial says, and tuning to it brings it to the centre.
//!
//! A generator that placed signals relative to the window would look
//! identical in a screenshot and behave wrongly the moment anybody tuned,
//! which is the thing a console most needs to be tested for.
//!
//! # Deterministic, and still varied
//!
//! Seeded. The same seed gives the same band every run, so a test can
//! assert where a carrier is, while an operator watching it still sees a
//! band that looks populated and changes over time. Randomness that cannot
//! be reproduced is not a feature in a test fixture.

use crate::SpectrumFrame;

/// What kind of thing is transmitting.
///
/// The shapes differ in ways a panadapter makes obvious, which is the
/// point: a console that renders them all identically has a bug worth
/// seeing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emission {
    /// An unmodulated carrier. One or two bins wide, and the case that
    /// catches a renderer averaging its columns instead of peak-holding.
    Cw,
    /// Voice. A few kHz of asymmetric, restless energy on one sideband.
    Ssb,
    /// A digital mode in fixed-length transmit cycles — FT8-shaped. Hard
    /// edges, constant amplitude, and it appears and disappears on a
    /// schedule rather than fading.
    Digital,
    /// Carrier plus two symmetric sidebands. The one whose *shape* tells
    /// you the mode without reading a label.
    Am,
    /// Broadband noise: an electric fence, a switching supply. Wide, flat
    /// and unmodulated, and it should not look like a signal.
    Noise,
}

impl Emission {
    /// Nominal occupied bandwidth.
    pub fn bandwidth_hz(&self) -> f64 {
        match self {
            Emission::Cw => 100.0,
            Emission::Ssb => 2_600.0,
            Emission::Digital => 50.0,
            Emission::Am => 6_000.0,
            Emission::Noise => 20_000.0,
        }
    }
}

/// One thing transmitting, at a fixed place in the band.
#[derive(Debug, Clone, Copy)]
pub struct Emitter {
    pub frequency_hz: u64,
    pub emission: Emission,
    /// Peak level in dBm when fully on.
    pub level_dbm: f32,
    /// Seed for this emitter's own variation, so one emitter's behaviour
    /// does not depend on how many others exist.
    seed: u64,
}

impl Emitter {
    pub fn new(frequency_hz: u64, emission: Emission, level_dbm: f32) -> Self {
        Self {
            frequency_hz,
            emission,
            level_dbm,
            seed: frequency_hz.wrapping_mul(0x9E37_79B9_7F4A_7C15),
        }
    }

    /// How strongly this emitter is transmitting at time `t`, 0.0-1.0.
    ///
    /// Time is passed in rather than read from a clock so a test can step
    /// it, and so two renders of the same instant agree.
    fn envelope(&self, t: f64) -> f32 {
        match self.emission {
            // Always on. A carrier is a carrier.
            Emission::Cw => {
                // Keyed at a plausible sending speed, so it blinks rather
                // than sits — an operator watching a static bar learns
                // nothing about whether the display is live.
                let period = 0.4 + (self.seed % 7) as f64 * 0.05;
                if (t / period) as u64 % 3 == 0 {
                    0.35
                } else {
                    1.0
                }
            }
            Emission::Ssb => {
                // Speech is restless and never quite silent.
                let a = (t * 5.3 + self.seed as f64 * 0.001).sin();
                let b = (t * 11.7 + self.seed as f64 * 0.003).sin();
                (0.55 + 0.45 * (a * 0.6 + b * 0.4)) as f32
            }
            Emission::Digital => {
                // FT8-shaped: 15-second slots, transmitting in some of
                // them. Hard edges and constant amplitude while on.
                let slot = (t / 15.0) as u64;
                let mine = mix(self.seed ^ slot) % 3 != 0;
                let within = t % 15.0;
                if mine && within < 12.6 {
                    1.0
                } else {
                    0.0
                }
            }
            Emission::Am => 0.9,
            Emission::Noise => 0.7,
        }
    }

    /// This emitter's contribution, in dB above the floor, at `hz`.
    ///
    /// `bin_width_hz` is not decoration. A real FFT bin **integrates**
    /// energy across its width, so a carrier narrower than a bin appears
    /// at full height in the bin it lands in. Sampling the shape at the
    /// bin's centre instead makes a 50 Hz digital signal fall between
    /// 187 Hz bins and all but vanish — which is a fixture inventing a
    /// physical effect that does not exist, and would have had somebody
    /// debugging a console that was rendering it correctly.
    fn power_at(&self, hz: f64, t: f64, bin_width_hz: f64) -> f32 {
        let envelope = self.envelope(t);
        if envelope <= 0.0 {
            return f32::NEG_INFINITY;
        }
        let offset = hz - self.frequency_hz as f64;
        // Anything narrower than a bin is a bin wide, for the reason
        // above. Widening never makes a signal wider than it is on
        // screen: at these spans the bin is the resolution limit.
        let bw = self.emission.bandwidth_hz().max(bin_width_hz);

        let shape = match self.emission {
            // Symmetric and steep.
            Emission::Cw | Emission::Digital => gaussian(offset, bw * 0.5),
            // One sideband only, so it leans. A renderer that mirrors the
            // spectrum makes this lean the wrong way, which is visible at
            // a glance and is the reason SSB is in this list.
            Emission::Ssb => {
                if offset >= 0.0 && offset <= bw {
                    // Gentle roll-off across the passband rather than a
                    // brick wall; real voice does not have square edges.
                    (1.0 - (offset / bw).powi(2) * 0.55) as f32
                } else {
                    gaussian(offset.min(0.0).abs().max(offset - bw), bw * 0.08)
                }
            }
            // Carrier plus two sidebands, and the carrier is the tallest
            // part.
            Emission::Am => {
                let carrier = gaussian(offset, 60.0);
                let upper = gaussian(offset - bw * 0.35, bw * 0.22) * 0.5;
                let lower = gaussian(offset + bw * 0.35, bw * 0.22) * 0.5;
                carrier.max(upper).max(lower)
            }
            // Flat-topped and wide.
            Emission::Noise => {
                if offset.abs() < bw * 0.5 {
                    0.85
                } else {
                    gaussian(offset.abs() - bw * 0.5, bw * 0.15) * 0.85
                }
            }
        };

        if shape <= 0.0005 {
            return f32::NEG_INFINITY;
        }
        self.level_dbm + 20.0 * (shape * envelope).log10()
    }
}

/// A band full of emitters.
#[derive(Debug, Clone)]
pub struct Band {
    emitters: Vec<Emitter>,
    /// Mean noise floor, dBm.
    pub floor_dbm: f32,
    seed: u64,
}

impl Band {
    /// An empty band with a floor.
    pub fn empty(floor_dbm: f32, seed: u64) -> Self {
        Self {
            emitters: Vec::new(),
            floor_dbm,
            // Offset rather than `| 1`. Forcing the low bit was meant to
            // avoid a zero state and instead mapped 42 and 43 onto the
            // same band -- every even seed collided with its successor,
            // silently, in a fixture whose whole job is to be varied.
            seed: seed.wrapping_add(GOLDEN),
        }
    }

    pub fn with(mut self, emitter: Emitter) -> Self {
        self.emitters.push(emitter);
        self
    }

    pub fn emitters(&self) -> &[Emitter] {
        &self.emitters
    }

    /// Populate `count` emitters at random across `low_hz..high_hz`.
    ///
    /// The mix is deliberately uneven — mostly CW and digital, which is
    /// what a real HF band sounds like, and enough SSB and AM that the
    /// distinct shapes appear. One noise source, because a console should
    /// be tested against something that is *not* a signal.
    pub fn populated(low_hz: u64, high_hz: u64, count: usize, floor_dbm: f32, seed: u64) -> Self {
        let mut band = Band::empty(floor_dbm, seed);
        let mut state = seed.wrapping_add(GOLDEN);
        let span = high_hz.saturating_sub(low_hz).max(1);
        for i in 0..count {
            state = mix(state ^ (i as u64).wrapping_mul(0x2545_F491_4F6C_DD1D));
            let frequency_hz = low_hz + state % span;
            let emission = match mix(state ^ 0xA5A5) % 10 {
                0..=3 => Emission::Cw,
                4..=6 => Emission::Digital,
                7 | 8 => Emission::Ssb,
                _ => Emission::Am,
            };
            // Strong enough to see, varied enough that the display has a
            // range to show. Nothing is at the very top: a band where
            // everything is full-scale tells a renderer nothing.
            let level_dbm = floor_dbm + 12.0 + (mix(state ^ 0x1234) % 45) as f32;
            band = band.with(Emitter::new(frequency_hz, emission, level_dbm));
        }
        // Exactly one wideband noise source, placed away from the middle
        // so it does not swamp whatever the dial starts on.
        let noise_hz = low_hz + span / 8;
        band.with(Emitter::new(noise_hz, Emission::Noise, floor_dbm + 14.0))
    }

    /// Render the window `center_hz ± span_hz/2` into `bins` powers.
    ///
    /// Low-frequency-first, per `cat-signal`'s invariant. Any mirroring a
    /// real tap needs happens in the source that reads that tap, and this
    /// source has no tap to mirror.
    pub fn render(&self, center_hz: u64, span_hz: u32, bins: usize, t: f64) -> Vec<f32> {
        let bins = bins.max(1);
        let low = center_hz as f64 - f64::from(span_hz) / 2.0;
        let bin_width = f64::from(span_hz) / bins as f64;

        (0..bins)
            .map(|i| {
                let hz = low + (i as f64 + 0.5) * bin_width;
                // A floor that moves a little. A perfectly flat floor
                // makes a waterfall look like a still image and hides
                // whether it is updating at all.
                let wobble = noise_at(self.seed, i as u64, (t * 12.0) as u64);
                let mut power = self.floor_dbm + wobble * 2.5;
                for emitter in &self.emitters {
                    let contribution = emitter.power_at(hz, t, bin_width);
                    if contribution > power {
                        power = contribution;
                    }
                }
                power
            })
            .collect()
    }

    /// A whole frame, ready to send.
    pub fn frame(
        &self,
        center_hz: u64,
        span_hz: u32,
        bins: usize,
        t: f64,
        sequence: u64,
    ) -> SpectrumFrame {
        SpectrumFrame {
            center_hz,
            span_hz,
            ref_level_dbm: self.floor_dbm + 70.0,
            sequence,
            bins: self.render(center_hz, span_hz, bins, t),
        }
    }
}

/// An odd, well-mixed offset, so that adjacent seeds do not produce
/// adjacent — or identical — bands.
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// A bell curve, peaking at 1.0 when `offset` is zero.
fn gaussian(offset: f64, width_hz: f64) -> f32 {
    if width_hz <= 0.0 {
        return 0.0;
    }
    let x = offset / width_hz;
    (-(x * x) / 2.0).exp() as f32
}

/// A cheap deterministic hash. Not cryptographic and not trying to be —
/// it exists so the same seed gives the same band, without a dependency.
fn mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^ (x >> 33)
}

/// Floor variation for one bin at one moment, roughly -1.0..1.0.
fn noise_at(seed: u64, bin: u64, tick: u64) -> f32 {
    let h = mix(seed ^ bin.wrapping_mul(0x9E37_79B9) ^ tick.wrapping_mul(0x85EB_CA6B));
    ((h % 2001) as f32 / 1000.0) - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOOR: f32 = -110.0;

    /// The bin a frequency lands in, for a window.
    fn bin_of(center_hz: u64, span_hz: u32, bins: usize, hz: u64) -> usize {
        let low = center_hz as f64 - f64::from(span_hz) / 2.0;
        (((hz as f64 - low) / f64::from(span_hz)) * bins as f64) as usize
    }

    #[test]
    fn a_carrier_appears_at_its_own_frequency() {
        let band = Band::empty(FLOOR, 1).with(Emitter::new(14_074_000, Emission::Cw, -60.0));
        let bins = band.render(14_074_000, 48_000, 256, 0.05);
        let expected = bin_of(14_074_000, 48_000, 256, 14_074_000);
        let peak = bins
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert!(
            peak.abs_diff(expected) <= 1,
            "carrier at bin {peak}, expected {expected}"
        );
    }

    #[test]
    fn tuning_moves_the_window_over_a_fixed_landscape() {
        // The property that makes this worth building. A generator placing
        // signals relative to the window would pass a screenshot test and
        // behave wrongly the instant anybody tuned -- which is exactly
        // what a console needs testing for.
        let band = Band::empty(FLOOR, 1).with(Emitter::new(14_074_000, Emission::Cw, -60.0));

        // Dial 12 kHz low: the carrier should sit right of centre.
        let bins = band.render(14_062_000, 48_000, 256, 0.05);
        let peak = bins
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert!(
            peak > 128,
            "carrier should be right of centre, got bin {peak}"
        );

        // Tune to it, and it comes to the middle.
        let bins = band.render(14_074_000, 48_000, 256, 0.05);
        let peak = bins
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert!(
            peak.abs_diff(128) <= 1,
            "tuning did not centre it: bin {peak}"
        );
    }

    #[test]
    fn a_signal_outside_the_window_does_not_appear_in_it() {
        let band = Band::empty(FLOOR, 1).with(Emitter::new(21_000_000, Emission::Cw, -40.0));
        let bins = band.render(14_074_000, 48_000, 256, 0.05);
        let loudest = bins.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!(
            loudest < FLOOR + 8.0,
            "a signal 7 MHz away leaked in at {loudest} dBm"
        );
    }

    #[test]
    fn an_empty_band_is_a_noise_floor_and_not_silence() {
        // A flat floor makes a waterfall look like a still image, and an
        // operator cannot tell a live display from a frozen one.
        let band = Band::empty(FLOOR, 7);
        let bins = band.render(14_074_000, 48_000, 256, 0.05);
        let min = bins.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = bins.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!(min > FLOOR - 6.0 && max < FLOOR + 6.0, "floor {min}..{max}");
        assert!(max - min > 0.5, "the floor is perfectly flat");
    }

    #[test]
    fn the_same_seed_and_time_render_identically() {
        // Randomness that cannot be reproduced is not a feature in a test
        // fixture.
        let a = Band::populated(14_000_000, 14_350_000, 20, FLOOR, 42);
        let b = Band::populated(14_000_000, 14_350_000, 20, FLOOR, 42);
        assert_eq!(
            a.render(14_074_000, 48_000, 128, 3.5),
            b.render(14_074_000, 48_000, 128, 3.5)
        );
    }

    #[test]
    fn different_seeds_give_different_bands() {
        let a = Band::populated(14_000_000, 14_350_000, 20, FLOOR, 42);
        let b = Band::populated(14_000_000, 14_350_000, 20, FLOOR, 43);
        let fa: Vec<u64> = a.emitters().iter().map(|e| e.frequency_hz).collect();
        let fb: Vec<u64> = b.emitters().iter().map(|e| e.frequency_hz).collect();
        assert_ne!(fa, fb);
    }

    #[test]
    fn a_populated_band_puts_every_emitter_inside_the_band() {
        let band = Band::populated(14_000_000, 14_350_000, 40, FLOOR, 9);
        for e in band.emitters() {
            assert!(
                (14_000_000..=14_350_000).contains(&e.frequency_hz),
                "{} is outside the band",
                e.frequency_hz
            );
        }
    }

    #[test]
    fn a_populated_band_has_more_than_one_kind_of_signal_in_it() {
        // A console that renders every emission identically has a bug, and
        // a fixture with only carriers in it would never show that.
        let band = Band::populated(14_000_000, 14_350_000, 40, FLOOR, 9);
        let mut kinds: Vec<Emission> = band.emitters().iter().map(|e| e.emission).collect();
        kinds.sort_by_key(|k| format!("{k:?}"));
        kinds.dedup();
        assert!(kinds.len() >= 3, "only {kinds:?} in a 40-signal band");
        assert!(
            kinds.contains(&Emission::Noise),
            "nothing that is not a signal"
        );
    }

    #[test]
    fn cw_is_narrower_than_ssb_which_is_narrower_than_am() {
        // The shapes have to actually differ, or the fixture is one signal
        // wearing five labels.
        fn width(emission: Emission) -> usize {
            let band = Band::empty(FLOOR, 1).with(Emitter::new(14_074_000, emission, -50.0));
            let bins = band.render(14_074_000, 48_000, 512, 0.05);
            bins.iter().filter(|p| **p > FLOOR + 20.0).count()
        }
        let cw = width(Emission::Cw);
        let ssb = width(Emission::Ssb);
        let am = width(Emission::Am);
        assert!(cw >= 1, "a carrier vanished entirely");
        assert!(cw < ssb, "CW ({cw}) is not narrower than SSB ({ssb})");
        assert!(ssb < am, "SSB ({ssb}) is not narrower than AM ({am})");
    }

    #[test]
    fn ssb_leans_to_one_side_of_its_frequency() {
        // Upper sideband: energy above the suppressed carrier, not below.
        // A renderer that mirrors the spectrum makes this lean the wrong
        // way, which is visible at a glance -- which is why it is here.
        let band = Band::empty(FLOOR, 1).with(Emitter::new(14_074_000, Emission::Ssb, -50.0));
        let bins = band.render(14_074_000, 48_000, 512, 0.05);
        let mid = bins.len() / 2;
        let above: f32 = bins[mid..].iter().sum();
        let below: f32 = bins[..mid].iter().sum();
        assert!(above > below, "USB leaned the wrong way");
    }

    #[test]
    fn a_digital_signal_is_present_in_some_slots_and_absent_in_others() {
        // Fifteen-second cycles. A fixture that transmitted continuously
        // would never exercise a console's "the signal went away" path.
        let band = Band::empty(FLOOR, 1).with(Emitter::new(14_074_000, Emission::Digital, -50.0));
        let peak_at = |t: f64| {
            band.render(14_074_000, 48_000, 256, t)
                .into_iter()
                .fold(f32::NEG_INFINITY, f32::max)
        };
        let samples: Vec<f32> = (0..8).map(|i| peak_at(i as f64 * 15.0 + 1.0)).collect();
        let on = samples.iter().filter(|p| **p > FLOOR + 20.0).count();
        assert!(on > 0, "the digital signal never transmitted");
        assert!(on < samples.len(), "the digital signal never stopped");
        // And within a slot it stops before the end, the way a real cycle
        // leaves room for decoding.
        assert!(peak_at(14.0) < FLOOR + 20.0, "transmitted past the slot");
    }

    #[test]
    fn a_frame_carries_the_window_it_was_rendered_for() {
        let band = Band::populated(14_000_000, 14_350_000, 10, FLOOR, 3);
        let frame = band.frame(14_074_000, 48_000, 128, 1.0, 7);
        assert_eq!(frame.center_hz, 14_074_000);
        assert_eq!(frame.span_hz, 48_000);
        assert_eq!(frame.bins.len(), 128);
        assert_eq!(frame.sequence, 7);
        // `bin_frequency_hz` relies on low-frequency-first, so this also
        // asserts the invariant holds for this source.
        let first = frame.bin_frequency_hz(0).unwrap();
        let last = frame.bin_frequency_hz(127).unwrap();
        assert!(first < last);
    }

    #[test]
    fn rendering_survives_degenerate_windows() {
        let band = Band::populated(14_000_000, 14_350_000, 5, FLOOR, 2);
        assert_eq!(band.render(14_074_000, 48_000, 0, 0.0).len(), 1);
        assert_eq!(band.render(14_074_000, 0, 16, 0.0).len(), 16);
        assert_eq!(band.render(0, 48_000, 16, 0.0).len(), 16);
        for p in band.render(14_074_000, 48_000, 64, 0.0) {
            assert!(p.is_finite(), "a bin rendered as {p}");
        }
    }
}
