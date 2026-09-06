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

//! What the console currently believes about the radio.
//!
//! # Everything here is an `Option`, and that is not caution
//!
//! **The native protocol has no read side yet.** `Command::ReadMeter`
//! validates that the meter exists and answers `Ack`; it does not return a
//! reading, and there is no command at all that reports the dial, the mode
//! or the split state. A console on this protocol today can *send* and
//! cannot *see*.
//!
//! So a value is `Unknown` until something tells us otherwise, and the
//! console draws "—". That is the honest rendering, and it is also the
//! rendering that has to exist regardless: on any protocol, there is a
//! window between connecting and the first state arriving, and a console
//! that shows `0.000.000 MHz` during it has told the operator something
//! false.
//!
//! # Pending is a third state, not a flicker
//!
//! When the operator asks for a change, the console knows what it asked
//! for and does not yet know whether it happened. Showing the requested
//! value immediately is a lie that is usually true; showing the old value
//! makes the console feel broken. So a request is held as `Pending` and
//! drawn distinctly, and it resolves when the radio confirms — which,
//! today, it cannot. That is deliberate: when the read side lands, this
//! resolves properly with no change to the drawing code.

/// One value the console displays.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Field<T> {
    /// Nothing has told us yet.
    #[default]
    Unknown,
    /// Asked for, not yet confirmed.
    Pending(T),
    /// The radio said so.
    Known(T),
}

impl<T: Copy> Field<T> {
    /// The value to draw, whether or not it is confirmed.
    pub fn value(&self) -> Option<T> {
        match self {
            Field::Unknown => None,
            Field::Pending(v) | Field::Known(v) => Some(*v),
        }
    }

    pub fn is_pending(&self) -> bool {
        matches!(self, Field::Pending(_))
    }

    /// Record that we asked for `value`.
    pub fn request(&mut self, value: T) {
        *self = Field::Pending(value);
    }

    /// Record what the radio actually reports.
    ///
    /// Confirmation always wins, including when it disagrees with what was
    /// asked for. A radio that rounded a frequency, refused a mode or was
    /// turned by hand at the front panel is the authority, and a console
    /// that kept showing its own request would be quietly wrong for as
    /// long as nobody looked at the radio.
    pub fn confirm(&mut self, value: T) {
        *self = Field::Known(value);
    }
}

/// Everything the status strip and quick bar draw.
#[derive(Debug, Clone, Default)]
pub struct Readout {
    pub vfo_a_hz: Field<u64>,
    pub mode: Field<cat_native::ModeId>,
    pub split: Field<bool>,
    pub if_shift_hz: Field<i32>,
    pub filter_width_hz: Field<u32>,
    pub memory_channel: Field<u16>,
    /// Raw S-meter, paired with its range by the drawing code so the
    /// number never travels without its scale.
    pub smeter_raw: Field<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_value_draws_as_unknown_and_not_as_zero() {
        // A console showing 0.000.000 MHz before the first state arrives
        // has told the operator something false.
        let f: Field<u64> = Field::Unknown;
        assert_eq!(f.value(), None);
    }

    #[test]
    fn a_request_is_visible_immediately_but_marked_as_not_yet_true() {
        // Showing the old value makes the console feel broken; showing the
        // new one as settled is a lie that is usually true. Pending is
        // neither.
        let mut f = Field::Unknown;
        f.request(14_074_000u64);
        assert_eq!(f.value(), Some(14_074_000));
        assert!(f.is_pending());
    }

    #[test]
    fn the_radio_wins_even_when_it_disagrees_with_what_we_asked_for() {
        // It rounded the frequency, or somebody turned the dial by hand.
        // A console that kept showing its own request would be quietly
        // wrong for as long as nobody looked at the radio.
        let mut f = Field::Unknown;
        f.request(14_074_003u64);
        f.confirm(14_074_000);
        assert_eq!(f.value(), Some(14_074_000));
        assert!(!f.is_pending());
    }

    #[test]
    fn a_confirmation_that_matches_still_clears_pending() {
        let mut f = Field::Unknown;
        f.request(7u32);
        f.confirm(7);
        assert_eq!(f, Field::Known(7));
    }

    #[test]
    fn a_fresh_readout_knows_nothing() {
        let r = Readout::default();
        assert_eq!(r.vfo_a_hz.value(), None);
        assert_eq!(r.smeter_raw.value(), None);
        assert_eq!(r.split.value(), None);
    }
}
