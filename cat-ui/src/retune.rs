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

//! Moving the view when an operator clicks a signal.
//!
//! # What is wrong with repainting
//!
//! Clicking a signal used to swap the waterfall's axis in one frame. The
//! scrollback is still there, but every trace in it jumps sideways at
//! once, and an operator has to re-find the signal they were looking at in
//! a picture that no longer resembles the one they clicked on.
//!
//! The information was never lost — the history knows its own frequencies
//! and can be redrawn from any vantage point. What was lost was the
//! *continuity*: the fact that this picture is the previous picture, seen
//! from somewhere else.
//!
//! # The move
//!
//! So the view travels. Its centre eases from the old dial to the new one,
//! and its span narrows on the way and opens again on arrival — a dolly
//! toward the signal rather than a cut. The whole history is reprojected
//! at every step, so a carrier the operator clicked stays a continuous
//! line that curves into the middle of the screen instead of teleporting
//! there.
//!
//! It ends at the ordinary span. A console that stayed zoomed in would
//! need a way back out, and inventing a control to undo an animation is a
//! sign the animation went too far.

/// A view of the band: what a waterfall's axis currently says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub center_hz: f64,
    pub span_hz: f64,
}

impl View {
    pub fn new(center_hz: f64, span_hz: f64) -> Self {
        Self { center_hz, span_hz }
    }
}

/// How far in the move dips, as a fraction of the span.
///
/// 0.55 is a visible push toward the signal without the band becoming
/// unrecognisable at the midpoint — the operator has to be able to follow
/// what moved where, which is the entire point of animating.
pub const ZOOM_DEPTH: f32 = 0.55;

/// How long the move takes.
///
/// Long enough to read as motion, short enough not to be in the way of
/// somebody chasing a fading signal. Retuning is something an operator
/// does repeatedly, and an animation they have to wait out twice is worse
/// than the jump it replaced.
pub const DURATION_MS: u32 = 320;

/// A move in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Retune {
    from: View,
    to: View,
    elapsed_ms: u32,
    duration_ms: u32,
}

impl Retune {
    /// Start a move from `from` to a view centred on `target_hz`.
    ///
    /// The destination span is the source's: the move ends looking at the
    /// band the way it started, one signal further along.
    pub fn to(from: View, target_hz: f64) -> Self {
        Self {
            from,
            to: View::new(target_hz, from.span_hz),
            elapsed_ms: 0,
            duration_ms: DURATION_MS,
        }
    }

    /// Advance by `dt_ms`. Returns whether the move is still running.
    pub fn advance(&mut self, dt_ms: u32) -> bool {
        self.elapsed_ms = self.elapsed_ms.saturating_add(dt_ms);
        !self.is_done()
    }

    pub fn is_done(&self) -> bool {
        self.elapsed_ms >= self.duration_ms
    }

    /// How far through, 0.0 to 1.0.
    pub fn progress(&self) -> f32 {
        if self.duration_ms == 0 {
            return 1.0;
        }
        (self.elapsed_ms as f32 / self.duration_ms as f32).clamp(0.0, 1.0)
    }

    /// The view to draw right now.
    pub fn view(&self) -> View {
        let t = ease(self.progress());
        let center = self.from.center_hz + (self.to.center_hz - self.from.center_hz) * f64::from(t);

        // The dip: full span at both ends, tightest in the middle. A
        // signal the operator clicked grows as the view closes on it and
        // settles back to the band they know.
        let dip = 1.0 - (1.0 - ZOOM_DEPTH) * arc(self.progress());
        let span = (self.from.span_hz + (self.to.span_hz - self.from.span_hz) * f64::from(t))
            * f64::from(dip);

        View::new(center, span)
    }

    /// Where the move ends, whatever it is doing now.
    pub fn destination(&self) -> View {
        self.to
    }
}

/// Ease in and out. Slow at both ends, quick through the middle.
///
/// A linear pan reads as a machine moving a picture; this reads as
/// something with weight, which is what makes it legible as *the same
/// picture moving* rather than as a series of unrelated frames.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        2.0 * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
    }
}

/// Zero at both ends, one in the middle.
fn arc(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    (std::f32::consts::PI * t).sin()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start() -> Retune {
        Retune::to(View::new(14_100_000.0, 240_000.0), 14_150_000.0)
    }

    #[test]
    fn it_begins_where_the_operator_was_looking() {
        // The first drawn frame must match what was on screen, or the
        // animation opens with the jump it exists to remove.
        let r = start();
        let v = r.view();
        assert!((v.center_hz - 14_100_000.0).abs() < 1.0);
        assert!((v.span_hz - 240_000.0).abs() < 1.0);
    }

    #[test]
    fn it_ends_on_the_signal_at_the_ordinary_span() {
        // Ending zoomed in would need a control to get back out, and
        // inventing one to undo an animation says the animation went too
        // far.
        let mut r = start();
        r.advance(DURATION_MS + 10);
        assert!(r.is_done());
        let v = r.view();
        assert!((v.center_hz - 14_150_000.0).abs() < 1.0, "{v:?}");
        assert!((v.span_hz - 240_000.0).abs() < 1.0, "{v:?}");
    }

    #[test]
    fn it_pushes_in_on_the_way_and_comes_back_out() {
        // The dolly. Halfway through, the view is tighter than at either
        // end -- that is what makes it read as moving toward the signal
        // rather than sliding past it.
        let mut r = start();
        r.advance(DURATION_MS / 2);
        let middle = r.view();
        assert!(middle.span_hz < 240_000.0 * 0.9, "no push-in: {middle:?}");
        assert!(
            middle.span_hz > 240_000.0 * 0.4,
            "pushed so far the band is unrecognisable: {middle:?}"
        );
    }

    #[test]
    fn the_centre_only_ever_moves_toward_the_target() {
        // An ease that overshot would swing past the signal and back,
        // which looks like a mistake being corrected.
        let mut r = start();
        let mut last = r.view().center_hz;
        for _ in 0..40 {
            r.advance(10);
            let now = r.view().center_hz;
            assert!(now >= last - 1.0, "went backwards: {last} then {now}");
            assert!(now <= 14_150_000.0 + 1.0, "overshot: {now}");
            last = now;
        }
    }

    #[test]
    fn a_move_downward_works_the_same_way() {
        let mut r = Retune::to(View::new(14_200_000.0, 240_000.0), 14_050_000.0);
        r.advance(DURATION_MS);
        assert!((r.view().center_hz - 14_050_000.0).abs() < 1.0);
    }

    #[test]
    fn it_finishes_even_if_frames_are_dropped() {
        // A console that stalled -- a slow repaint, a laptop lid -- must
        // not leave the view stranded partway between two signals.
        let mut r = start();
        assert!(r.advance(1), "one step must not finish a 320 ms move");
        r.advance(10_000);
        assert!(r.is_done());
        assert!((r.view().center_hz - 14_150_000.0).abs() < 1.0);
    }

    #[test]
    fn clicking_where_you_already_are_is_not_a_journey() {
        // Retuning to the current dial should look like nothing happening,
        // not like a zoom to nowhere.
        let mut r = Retune::to(View::new(14_100_000.0, 240_000.0), 14_100_000.0);
        r.advance(DURATION_MS / 2);
        assert!((r.view().center_hz - 14_100_000.0).abs() < 1.0);
    }

    #[test]
    fn the_easing_is_symmetric_and_bounded() {
        assert!((ease(0.0) - 0.0).abs() < 1e-6);
        assert!((ease(1.0) - 1.0).abs() < 1e-6);
        assert!((ease(0.5) - 0.5).abs() < 1e-6);
        for i in 0..=100 {
            let t = i as f32 / 100.0;
            assert!((0.0..=1.0).contains(&ease(t)), "{t} -> {}", ease(t));
        }
    }
}
