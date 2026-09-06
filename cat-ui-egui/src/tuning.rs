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

//! Clicking the waterfall tunes the radio, and the display recentres.
//!
//! # Why there is no inversion handling here
//!
//! The TS-570D's first IF is 73.05 MHz with high-side LO injection, so its
//! tapped spectrum arrives **mirrored**. It is corrected in the
//! `SpectrumSource`, not here: `cat-signal` guarantees
//! `SpectrumFrame::bins` is always low-frequency-first, on every source,
//! always.
//!
//! That guarantee is the whole reason this file is four lines of
//! arithmetic. A console that "helpfully" re-applied `inverted` would
//! mirror the tuning about the dial — click above the carrier, tune below
//! it — and it would look entirely plausible while doing so, because the
//! waterfall would still show a signal moving toward the cursor. There is
//! a test named for that mistake.
//!
//! # Why it recentres
//!
//! An IF tap is dial-centred *by construction*. The SDR sits on the fixed
//! intermediate frequency while the radio's local oscillator tracks the
//! dial, so the centre of the picture is wherever the dial is — not a
//! choice the console gets to make. Tuning somewhere and leaving the
//! picture where it was would require the hardware to do something it
//! cannot.
//!
//! So the cursor is a fixed reticle at the centre and the spectrum slides
//! under it, rather than a cursor that moves across a stationary spectrum.

use cat_signal::SpectrumFrame;

/// The frequency under a click at `fraction` across the waterfall.
///
/// `fraction` is 0.0 at the left edge and 1.0 at the right, and is clamped
/// — a drag that leaves the widget should tune to its edge rather than off
/// the end of the band.
pub fn frequency_at(frame: &SpectrumFrame, fraction: f32) -> u64 {
    let (low, high) = frame.range_hz();
    let f = f64::from(fraction.clamp(0.0, 1.0));
    (low + (high - low) * f).round().max(0.0) as u64
}

/// Round `hz` to the nearest step the radio actually tunes in.
///
/// Sending an unrounded frequency is not harmless: the radio rounds it
/// anyway, and then the console's idea of the dial and the radio's differ
/// by a few Hz until the next poll corrects it — which reads as the
/// display drifting on its own.
///
/// `steps` is the radio's own `tuning_steps_hz`, and the finest is used:
/// a click is a coarse gesture and the operator can refine it, so the
/// least surprising thing is to preserve as much of the click as the
/// hardware can express.
pub fn snap(hz: u64, steps: &[u32]) -> u64 {
    let Some(step) = steps.iter().copied().filter(|s| *s > 0).min() else {
        return hz;
    };
    let step = u64::from(step);
    let rem = hz % step;
    if rem * 2 >= step {
        hz + (step - rem)
    } else {
        hz - rem
    }
}

/// Where a click should tune, given the radio's limits.
///
/// `None` when the target is outside what the radio covers. Refusing is
/// better than clamping: clamping tunes somewhere the operator did not
/// click and gives no sign it happened.
pub fn tune_target(
    frame: &SpectrumFrame,
    fraction: f32,
    steps: &[u32],
    min_hz: u64,
    max_hz: u64,
) -> Option<u64> {
    let hz = snap(frequency_at(frame, fraction), steps);
    (min_hz..=max_hz).contains(&hz).then_some(hz)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(center_hz: u64, span_hz: u32) -> SpectrumFrame {
        SpectrumFrame {
            center_hz,
            span_hz,
            ref_level_dbm: -100.0,
            sequence: 0,
            bins: vec![-100.0; 64],
        }
    }

    const STEPS: &[u32] = &[10, 100, 1_000, 5_000, 9_000, 10_000];
    const MIN: u64 = 500_000;
    const MAX: u64 = 60_000_000;

    #[test]
    fn clicking_the_centre_tunes_to_where_the_dial_already_is() {
        let f = frame(14_074_000, 48_000);
        assert_eq!(frequency_at(&f, 0.5), 14_074_000);
    }

    #[test]
    fn clicking_the_edges_tunes_to_the_edges_of_the_span() {
        let f = frame(14_074_000, 48_000);
        assert_eq!(frequency_at(&f, 0.0), 14_050_000);
        assert_eq!(frequency_at(&f, 1.0), 14_098_000);
    }

    #[test]
    fn clicking_right_of_centre_tunes_up_and_never_down() {
        // The mistake this file exists to prevent. The TS-570D's tap is
        // mirrored in hardware and un-mirrored in the source, so a console
        // that re-applied `inverted` would tune the wrong way -- and would
        // look plausible doing it, because the signal would still slide
        // toward the cursor.
        let f = frame(14_074_000, 48_000);
        let right = frequency_at(&f, 0.75);
        let left = frequency_at(&f, 0.25);
        assert!(right > f.center_hz, "clicking right must tune up");
        assert!(left < f.center_hz, "clicking left must tune down");
    }

    #[test]
    fn the_mapping_is_monotonic_across_the_whole_width() {
        let f = frame(14_074_000, 48_000);
        let mut previous = 0;
        for i in 0..=100 {
            let hz = frequency_at(&f, i as f32 / 100.0);
            assert!(hz >= previous, "went backwards at {i}%");
            previous = hz;
        }
    }

    #[test]
    fn a_click_outside_the_widget_tunes_to_its_edge_rather_than_off_the_band() {
        let f = frame(14_074_000, 48_000);
        assert_eq!(frequency_at(&f, -3.0), frequency_at(&f, 0.0));
        assert_eq!(frequency_at(&f, 7.5), frequency_at(&f, 1.0));
    }

    #[test]
    fn a_tuned_frequency_lands_on_a_step_the_radio_can_actually_reach() {
        // Otherwise the radio rounds it, the console does not, and the
        // display drifts by a few Hz until the next poll -- which looks
        // like the dial moving on its own.
        assert_eq!(snap(14_074_003, STEPS), 14_074_000);
        assert_eq!(snap(14_074_007, STEPS), 14_074_010);
        assert_eq!(snap(14_074_005, STEPS), 14_074_010);
    }

    #[test]
    fn snapping_uses_the_finest_step_the_radio_has() {
        // A click is coarse and the operator refines it, so keep as much
        // of the gesture as the hardware can express.
        assert_eq!(snap(14_074_003, &[10, 10_000]), 14_074_000);
        assert_eq!(snap(14_074_003, &[10_000, 10]), 14_074_000);
    }

    #[test]
    fn a_radio_that_publishes_no_steps_is_tuned_exactly() {
        assert_eq!(snap(14_074_003, &[]), 14_074_003);
        // A zero step would divide by zero; it is ignored, not trusted.
        assert_eq!(snap(14_074_003, &[0]), 14_074_003);
    }

    #[test]
    fn a_click_past_the_edge_of_coverage_is_refused_and_not_clamped() {
        // Clamping would tune somewhere the operator did not click and
        // give no sign it had happened.
        let f = frame(59_999_000, 48_000);
        assert_eq!(tune_target(&f, 0.0, STEPS, MIN, MAX), Some(59_975_000));
        assert_eq!(tune_target(&f, 1.0, STEPS, MIN, MAX), None);
    }

    #[test]
    fn a_click_inside_coverage_is_accepted() {
        let f = frame(14_074_000, 48_000);
        assert_eq!(tune_target(&f, 0.5, STEPS, MIN, MAX), Some(14_074_000));
    }

    #[test]
    fn the_picture_recentres_because_the_hardware_leaves_no_choice() {
        // An IF tap is dial-centred by construction: the SDR is parked on
        // the intermediate frequency and the LO tracks the dial. After a
        // retune the next frame arrives centred on the new dial, so the
        // click lands under the reticle. This is the physics, expressed as
        // the property a renderer can rely on.
        let before = frame(14_074_000, 48_000);
        let target = tune_target(&before, 0.75, STEPS, MIN, MAX).unwrap();
        let after = frame(target, 48_000);
        assert_eq!(frequency_at(&after, 0.5), target);
    }
}
