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

//! Meter readings, scaled against the radio's own range.

use cat_framework::capabilities::{MeterDescriptor, MeterKind, MeterSet, RawRange};

/// A raw meter reading paired with the range it means something in.
///
/// The pairing is the point. A bare `u16` is meaningless — 15 is mid-scale
/// on a TS-570D S-meter and under 6% on an FT-991A's — and every bug this
/// type prevents is one where a raw value travelled somewhere without its
/// scale. Both existing apps hardcoded their own radio's range into an
/// otherwise identical `smeter_bar`, which is exactly that bug, twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeterReading {
    pub kind: MeterKind,
    pub raw: u16,
    pub range: RawRange,
}

impl MeterReading {
    pub fn new(kind: MeterKind, raw: u16, range: RawRange) -> Self {
        Self { kind, raw, range }
    }

    /// Build a reading from a radio's own meter set, or `None` if the
    /// radio has no such meter.
    ///
    /// A renderer that goes through this cannot draw a meter the radio
    /// does not have.
    pub fn from_meters(meters: &MeterSet, kind: MeterKind, raw: u16) -> Option<Self> {
        meters
            .find(kind)
            .map(|descriptor| Self::new(kind, raw, descriptor.raw_range))
    }

    pub fn from_descriptor(descriptor: &MeterDescriptor, raw: u16) -> Self {
        Self::new(descriptor.kind, raw, descriptor.raw_range)
    }

    /// Where this reading sits in its range, 0.0-1.0, clamped.
    ///
    /// The one number a renderer needs. `cat-ui-ratatui` turns it into
    /// block characters; `cat-ui-egui` turns it into a filled rectangle;
    /// neither needs to know which radio produced it.
    pub fn fraction(&self) -> f32 {
        self.range.fraction(self.raw)
    }

    /// The reading as a percentage, for a numeric readout.
    pub fn percent(&self) -> u8 {
        (self.fraction() * 100.0).round() as u8
    }

    /// `true` when the reading is at the very top of its scale.
    ///
    /// Worth distinguishing: a pegged SWR meter and a high one call for
    /// different treatment, and a renderer comparing floats for equality
    /// would get it subtly wrong.
    pub fn is_pegged(&self) -> bool {
        self.raw >= self.range.max
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_framework::capabilities::MeterSet;

    const METERS: &[MeterDescriptor] = &[
        MeterDescriptor {
            kind: MeterKind::S,
            raw_range: RawRange::new(0, 30),
            active_on_transmit: false,
        },
        MeterDescriptor {
            kind: MeterKind::Swr,
            raw_range: RawRange::new(0, 255),
            active_on_transmit: true,
        },
    ];

    #[test]
    fn a_reading_carries_its_scale_so_the_renderer_does_not_have_to() {
        let ts = MeterReading::new(MeterKind::S, 15, RawRange::new(0, 30));
        let ft = MeterReading::new(MeterKind::S, 15, RawRange::new(0, 255));
        assert_eq!(ts.fraction(), 0.5);
        assert!(ft.fraction() < 0.06);
        // Same kind, same raw value, different meaning -- and a renderer
        // drawing both gets each right without knowing either radio.
        assert_eq!(ts.kind, ft.kind);
        assert_eq!(ts.raw, ft.raw);
    }

    #[test]
    fn a_meter_the_radio_lacks_cannot_be_constructed_from_its_set() {
        let meters = MeterSet::new(METERS);
        assert!(MeterReading::from_meters(&meters, MeterKind::S, 10).is_some());
        assert!(MeterReading::from_meters(&meters, MeterKind::Comp, 10).is_none());
    }

    #[test]
    fn a_reading_built_from_a_set_picks_up_that_meters_range() {
        let meters = MeterSet::new(METERS);
        let s = MeterReading::from_meters(&meters, MeterKind::S, 30).unwrap();
        let swr = MeterReading::from_meters(&meters, MeterKind::Swr, 30).unwrap();
        assert_eq!(s.fraction(), 1.0);
        assert!(swr.fraction() < 0.13);
    }

    #[test]
    fn percent_is_a_rounded_whole_number() {
        let r = MeterReading::new(MeterKind::Po, 15, RawRange::new(0, 30));
        assert_eq!(r.percent(), 50);
        assert_eq!(
            MeterReading::new(MeterKind::Po, 0, RawRange::new(0, 30)).percent(),
            0
        );
        assert_eq!(
            MeterReading::new(MeterKind::Po, 30, RawRange::new(0, 30)).percent(),
            100
        );
    }

    #[test]
    fn pegged_is_distinguishable_from_merely_high() {
        let range = RawRange::new(0, 30);
        assert!(!MeterReading::new(MeterKind::Swr, 29, range).is_pegged());
        assert!(MeterReading::new(MeterKind::Swr, 30, range).is_pegged());
        // An over-range reading is still pegged, not wrapped.
        assert!(MeterReading::new(MeterKind::Swr, 60, range).is_pegged());
    }

    #[test]
    fn an_over_range_reading_clamps_to_full_scale() {
        let r = MeterReading::new(MeterKind::S, u16::MAX, RawRange::new(0, 30));
        assert_eq!(r.fraction(), 1.0);
        assert_eq!(r.percent(), 100);
    }
}
