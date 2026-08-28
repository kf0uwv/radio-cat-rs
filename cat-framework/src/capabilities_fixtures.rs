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

//! Task 13, the acceptance gate on [`crate::capabilities`].
//!
//! ADR 0010 concedes the capability model "is a guess until three radios
//! are actually described by it." This module describes two, and asserts
//! that describing them needed **no escape hatch** — no
//! `Option<serde_json::Value>`, no `extra: HashMap`, and no `model` string
//! that downstream code matches on.
//!
//! # Why this is `#[cfg(test)]`
//!
//! This crate's own contract (see `lib.rs`) is that it "contains **no
//! radio-specific** command definitions, modes, frequencies, or state."
//! These fixtures are radio-specific by definition, so they exist only
//! when compiling tests. Shipping them would break the very property that
//! makes `cat-framework` generic.
//!
//! # Where the values come from
//!
//! Every number here is read out of the two app repos rather than
//! remembered, and cited in a comment at the point of use. `ts570d` and
//! `ft991a` are read-only to this task.
//!
//! # The verdict
//!
//! Recorded in `planning/architect/findings.md` and in the tests at the
//! bottom of this file. Summary: the model fits, with **one strain** worth
//! naming — [`MenuCapability`] is a count and a writability flag, and the
//! FT-991A's menu is 152 heterogeneous typed entries. See
//! `menu_capability_is_the_thinnest_part_of_the_model`.

use crate::capabilities::*;
use cat_signal::{IfTapConfig, SignalCapability};

// ---------------------------------------------------------------------------
// Kenwood TS-570D
// ---------------------------------------------------------------------------

/// The TS-570D's single RS-232C port carries CAT **and** keying at once.
///
/// This is the case that motivated `shareable_with`: one handle, two
/// roles, and no way to express that with a flat list of endpoints.
const TS570D_ENDPOINTS: &[EndpointDescriptor] = &[EndpointDescriptor {
    role: EndpointRole::Cat,
    required: true,
    shareable_with: &[EndpointRole::Keying],
}];

/// Eight modes. Discriminants 1-7 and 9 on the wire; 8 is unused on this
/// radio (`ts570d/radio/src/radio_trait.rs`'s `Mode`).
const TS570D_MODES: &[ModeDescriptor] = &[
    ModeDescriptor {
        id: ModeId::Lsb,
        label: "LSB",
        kind: ModeKind::Ssb,
        sideband: Some(Sideband::Lower),
        default_bandwidth_hz: 2400,
    },
    ModeDescriptor {
        id: ModeId::Usb,
        label: "USB",
        kind: ModeKind::Ssb,
        sideband: Some(Sideband::Upper),
        default_bandwidth_hz: 2400,
    },
    ModeDescriptor {
        id: ModeId::CwUpper,
        label: "CW",
        kind: ModeKind::Cw,
        sideband: Some(Sideband::Upper),
        default_bandwidth_hz: 500,
    },
    ModeDescriptor {
        id: ModeId::Fm,
        label: "FM",
        kind: ModeKind::Fm,
        sideband: None,
        default_bandwidth_hz: 12000,
    },
    ModeDescriptor {
        id: ModeId::Am,
        label: "AM",
        kind: ModeKind::Am,
        sideband: None,
        default_bandwidth_hz: 6000,
    },
    ModeDescriptor {
        id: ModeId::RttyLsb,
        label: "FSK",
        kind: ModeKind::Data,
        sideband: Some(Sideband::Lower),
        default_bandwidth_hz: 500,
    },
    ModeDescriptor {
        id: ModeId::CwLower,
        label: "CW-R",
        kind: ModeKind::Cw,
        sideband: Some(Sideband::Lower),
        default_bandwidth_hz: 500,
    },
    ModeDescriptor {
        id: ModeId::RttyUsb,
        label: "FSK-R",
        kind: ModeKind::Data,
        sideband: Some(Sideband::Upper),
        default_bandwidth_hz: 500,
    },
];

/// S-meter reported over **0-30**.
///
/// From `ts570d/ui/src/layout.rs`'s `smeter_bar`, which scales
/// `(smeter.min(30) * 20 / 30)`. Contrast the FT-991A below: same meter,
/// different range, and that difference is a property of the radio.
const TS570D_METERS: &[MeterDescriptor] = &[
    MeterDescriptor {
        kind: MeterKind::S,
        raw_range: RawRange::new(0, 30),
        active_on_transmit: false,
    },
    MeterDescriptor {
        kind: MeterKind::Po,
        raw_range: RawRange::new(0, 30),
        active_on_transmit: true,
    },
    MeterDescriptor {
        kind: MeterKind::Swr,
        raw_range: RawRange::new(0, 30),
        active_on_transmit: true,
    },
    MeterDescriptor {
        kind: MeterKind::Alc,
        raw_range: RawRange::new(0, 30),
        active_on_transmit: true,
    },
];

/// Kenwood TS-570D, with the CN4 IF tap fitted.
pub const TS570D: RadioCapabilities = RadioCapabilities {
    model: "Kenwood TS-570D",
    endpoints: EndpointSet::new(TS570D_ENDPOINTS),
    vfos: VfoCapability {
        count: 2,
        split: true,
        // RIT/XIT offset -9999..+9999 Hz (`radio_trait.rs`'s IF response
        // layout, byte 15).
        rit_hz: Some(9999),
        xit_hz: Some(9999),
    },
    modes: TS570D_MODES,
    tuning_steps_hz: &[10, 100, 1_000, 5_000, 9_000, 10_000],
    // `Frequency::MIN_HZ`/`MAX_HZ` in `ts570d/radio/src/radio_trait.rs`.
    rx_range: FrequencyRange::new(500_000, 60_000_000),
    filters: FilterCapability {
        // `get_if_shift` returns a direction character and an offset; the
        // radio has IF shift but exposes no selectable width list over CAT.
        if_shift_hz: Some(1_000),
        widths_hz: None,
        notch: false,
    },
    meters: MeterSet::new(TS570D_METERS),
    memory: Some(MemoryCapability {
        // "memory channel (00-99)" — `radio_trait.rs`'s IF layout, byte 24.
        channels: RawRange::new(0, 99),
        named: false,
        stores_mode: true,
        scan: true,
    }),
    menu: Some(MenuCapability {
        // `Ts570dState::menu_values: [u16; 52]`.
        item_count: 52,
        writable: true,
    }),
    // NOTE (2026-08-28): this field is where ADR 0015 says this fixture is
    // wrong. Asserting `IfTap` here claims every TS-570D *has* a tap fitted,
    // when the truth is that every TS-570D has a CN4 header and whether a
    // dongle hangs off it is a fact about one bench. `trim_hz: 0` below is
    // the same mistake in miniature — a per-station measurement standing in
    // a per-model constant. Left as-is until ADR 0015 is settled, because
    // changing it piecemeal would be worse than one deliberate split.
    //
    // No bandscope. The spectrum comes from an SDR on the CN4 IF tap:
    // 73.05 MHz first IF, inverted because LO1 is high-side injection
    // (73.05-103.05 MHz), and one calibrated trim. `trim_hz` is zero here
    // because it is a per-station measurement, not a property of the model.
    signal: SignalCapability::IfTap(IfTapConfig {
        if_center_hz: 73_050_000,
        inverted: true,
        trim_hz: 0,
    }),
};

// ---------------------------------------------------------------------------
// Yaesu FT-991A — the stressing case
// ---------------------------------------------------------------------------

/// **Three** endpoints, none shareable.
///
/// This is why two radios is the minimum for this gate. The FT-991A's USB
/// bridge enumerates two CP210x virtual COM ports — "Enhanced" for CAT and
/// "Standard" for keying — plus a USB audio codec (`ft991a` ADR 0002).
/// Nothing here may be shared, which is the exact inverse of the TS-570D
/// and the reason `shareable_with` is per-endpoint rather than a global
/// flag on the set.
const FT991A_ENDPOINTS: &[EndpointDescriptor] = &[
    EndpointDescriptor {
        role: EndpointRole::Cat,
        required: true,
        shareable_with: &[],
    },
    EndpointDescriptor {
        role: EndpointRole::Keying,
        required: false,
        shareable_with: &[],
    },
    EndpointDescriptor {
        role: EndpointRole::Audio,
        required: false,
        shareable_with: &[],
    },
];

/// Fifteen modes, including two the TS-570D has no concept of: the DATA
/// family and Yaesu's C4FM digital voice.
const FT991A_MODES: &[ModeDescriptor] = &[
    ModeDescriptor {
        id: ModeId::Lsb,
        label: "LSB",
        kind: ModeKind::Ssb,
        sideband: Some(Sideband::Lower),
        default_bandwidth_hz: 2400,
    },
    ModeDescriptor {
        id: ModeId::Usb,
        label: "USB",
        kind: ModeKind::Ssb,
        sideband: Some(Sideband::Upper),
        default_bandwidth_hz: 2400,
    },
    ModeDescriptor {
        id: ModeId::CwUpper,
        label: "CW-U",
        kind: ModeKind::Cw,
        sideband: Some(Sideband::Upper),
        default_bandwidth_hz: 500,
    },
    ModeDescriptor {
        id: ModeId::Fm,
        label: "FM",
        kind: ModeKind::Fm,
        sideband: None,
        default_bandwidth_hz: 16000,
    },
    ModeDescriptor {
        id: ModeId::Am,
        label: "AM",
        kind: ModeKind::Am,
        sideband: None,
        default_bandwidth_hz: 6000,
    },
    ModeDescriptor {
        id: ModeId::RttyLsb,
        label: "RTTY-L",
        kind: ModeKind::Data,
        sideband: Some(Sideband::Lower),
        default_bandwidth_hz: 500,
    },
    ModeDescriptor {
        id: ModeId::CwLower,
        label: "CW-L",
        kind: ModeKind::Cw,
        sideband: Some(Sideband::Lower),
        default_bandwidth_hz: 500,
    },
    ModeDescriptor {
        id: ModeId::DataLsb,
        label: "DATA-L",
        kind: ModeKind::Data,
        sideband: Some(Sideband::Lower),
        default_bandwidth_hz: 3000,
    },
    ModeDescriptor {
        id: ModeId::RttyUsb,
        label: "RTTY-U",
        kind: ModeKind::Data,
        sideband: Some(Sideband::Upper),
        default_bandwidth_hz: 500,
    },
    ModeDescriptor {
        id: ModeId::DataFm,
        label: "DATA-FM",
        kind: ModeKind::Data,
        sideband: None,
        default_bandwidth_hz: 16000,
    },
    ModeDescriptor {
        id: ModeId::FmNarrow,
        label: "FM-N",
        kind: ModeKind::Fm,
        sideband: None,
        default_bandwidth_hz: 9000,
    },
    ModeDescriptor {
        id: ModeId::DataUsb,
        label: "DATA-U",
        kind: ModeKind::Data,
        sideband: Some(Sideband::Upper),
        default_bandwidth_hz: 3000,
    },
    ModeDescriptor {
        id: ModeId::AmNarrow,
        label: "AM-N",
        kind: ModeKind::Am,
        sideband: None,
        default_bandwidth_hz: 3000,
    },
    ModeDescriptor {
        id: ModeId::C4fm,
        label: "C4FM",
        kind: ModeKind::DigitalVoice,
        sideband: None,
        default_bandwidth_hz: 12500,
    },
];

/// S-meter reported over **0-255**.
///
/// From `ft991a/ui/src/layout.rs`'s `smeter_bar`, which scales
/// `smeter as usize * width / 255`. Eight and a half times the TS-570D's
/// range for the same physical quantity.
const FT991A_METERS: &[MeterDescriptor] = &[
    MeterDescriptor {
        kind: MeterKind::S,
        raw_range: RawRange::new(0, 255),
        active_on_transmit: false,
    },
    MeterDescriptor {
        kind: MeterKind::Po,
        raw_range: RawRange::new(0, 255),
        active_on_transmit: true,
    },
    MeterDescriptor {
        kind: MeterKind::Swr,
        raw_range: RawRange::new(0, 255),
        active_on_transmit: true,
    },
    MeterDescriptor {
        kind: MeterKind::Alc,
        raw_range: RawRange::new(0, 255),
        active_on_transmit: true,
    },
    MeterDescriptor {
        kind: MeterKind::Id,
        raw_range: RawRange::new(0, 255),
        active_on_transmit: true,
    },
    MeterDescriptor {
        kind: MeterKind::Vdd,
        raw_range: RawRange::new(0, 255),
        active_on_transmit: true,
    },
    MeterDescriptor {
        kind: MeterKind::Comp,
        raw_range: RawRange::new(0, 255),
        active_on_transmit: true,
    },
];

/// Yaesu FT-991A.
pub const FT991A: RadioCapabilities = RadioCapabilities {
    model: "Yaesu FT-991A",
    endpoints: EndpointSet::new(FT991A_ENDPOINTS),
    vfos: VfoCapability {
        count: 2,
        split: true,
        rit_hz: Some(9999),
        xit_hz: Some(9999),
    },
    modes: FT991A_MODES,
    tuning_steps_hz: &[10, 100, 1_000, 5_000, 6_250, 10_000, 12_500, 25_000],
    // `Frequency::MIN_HZ`/`MAX_HZ` in `ft991a/radio/src/radio_trait.rs`.
    // HF through UHF, versus the TS-570D's HF-only coverage.
    rx_range: FrequencyRange::new(30_000, 470_000_000),
    filters: FilterCapability {
        if_shift_hz: Some(1_000),
        // `SH_BANDWIDTH_TABLE`/`filter_bandwidth_hz` in
        // `ft991a/radio/src/ft991a_radio.rs`.
        widths_hz: Some(&[
            200, 400, 500, 800, 1_200, 1_500, 1_800, 2_400, 2_900, 3_000, 3_200,
        ]),
        notch: true,
    },
    meters: MeterSet::new(FT991A_METERS),
    memory: Some(MemoryCapability {
        // "Invalid memory channel: valid 1-117" (`radio_trait.rs`).
        channels: RawRange::new(1, 117),
        named: true,
        stores_mode: true,
        scan: true,
    }),
    menu: Some(MenuCapability {
        // `EX_MENU_TABLE` has 152 entries.
        item_count: 152,
        writable: true,
    }),
    // No bandscope over CAT. Verified against the FT-991A CAT manual: it
    // has menu items for its own scope display, but no command that
    // returns scope DATA (ADR 0010, Context).
    signal: SignalCapability::None,
};

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------
    // The gate itself.
    //
    // ADR 0010's pass condition is that both radios read naturally with
    // no escape hatch. Two of those three forbidden hatches -- a
    // `serde_json::Value` field and an `extra: HashMap` -- cannot exist
    // here, because `RadioCapabilities` is `Copy` and `const`: neither
    // type is either, so adding one would fail to compile. That is a
    // structural guarantee, not a promise.
    //
    // The third hatch, matching on `model`, is not structurally
    // preventable, so the tests below answer every question from the
    // data instead, and would keep passing if the model strings were
    // swapped.
    // -----------------------------------------------------------------

    #[test]
    fn both_radios_are_compile_time_constants() {
        // If describing either radio had needed a Vec, a HashMap, or an
        // owned String, this function could not exist.
        const RADIOS: [RadioCapabilities; 2] = [TS570D, FT991A];
        assert_eq!(RADIOS.len(), 2);
        assert_ne!(RADIOS[0].model, RADIOS[1].model);
    }

    #[test]
    fn one_shared_handle_and_three_separate_ones_are_both_expressible() {
        // The TS-570D: a single RS-232C port doing CAT and keying at once.
        assert_eq!(TS570D.endpoints.handle_count(), 1);
        assert!(TS570D.endpoints.serves(EndpointRole::Cat));
        assert!(TS570D.endpoints.serves(EndpointRole::Keying));
        assert!(TS570D.endpoints.find(EndpointRole::Keying).is_none());
        assert!(!TS570D.endpoints.serves(EndpointRole::Audio));

        // The FT-991A: three handles, nothing shared.
        assert_eq!(FT991A.endpoints.handle_count(), 3);
        for role in [EndpointRole::Cat, EndpointRole::Keying, EndpointRole::Audio] {
            assert!(FT991A.endpoints.serves(role));
            assert!(FT991A.endpoints.find(role).is_some());
        }

        // The distinction that matters to a caller opening ports: both
        // radios "serve" keying, but only one needs a second handle for it.
        assert_eq!(
            TS570D
                .endpoints
                .find(EndpointRole::Cat)
                .unwrap()
                .shareable_with,
            &[EndpointRole::Keying]
        );
        assert!(FT991A
            .endpoints
            .find(EndpointRole::Cat)
            .unwrap()
            .shareable_with
            .is_empty());
    }

    #[test]
    fn only_the_cat_endpoint_is_required_on_either_radio() {
        for radio in [TS570D, FT991A] {
            let required: Vec<_> = radio
                .endpoints
                .endpoints
                .iter()
                .filter(|e| e.required)
                .map(|e| e.role)
                .collect();
            assert_eq!(required, vec![EndpointRole::Cat], "{}", radio.model);
        }
    }

    #[test]
    fn the_same_smeter_reading_means_different_things_on_each_radio() {
        // The single most useful thing this model does, and the concrete
        // fix for the duplication ADR 0011 found: `smeter_bar` existed
        // twice, each copy with its own radio's scale hardcoded.
        let ts = TS570D.meters.find(MeterKind::S).unwrap().raw_range;
        let ft = FT991A.meters.find(MeterKind::S).unwrap().raw_range;
        assert_eq!(ts, RawRange::new(0, 30));
        assert_eq!(ft, RawRange::new(0, 255));

        // Raw 15: mid-scale on one radio, nearly nothing on the other.
        assert_eq!(ts.fraction(15), 0.5);
        assert!(ft.fraction(15) < 0.06);
    }

    #[test]
    fn meter_sets_differ_in_size_without_either_distorting_the_other() {
        // The FT-991A reports three meters the TS-570D has no concept of.
        assert_eq!(TS570D.meters.meters.len(), 4);
        assert_eq!(FT991A.meters.meters.len(), 7);

        for kind in [MeterKind::Id, MeterKind::Vdd, MeterKind::Comp] {
            assert!(FT991A.meters.has(kind));
            assert!(!TS570D.meters.has(kind));
        }

        // Absent is absent -- not zero, not a sentinel.
        assert!(TS570D.meters.find(MeterKind::Comp).is_none());
    }

    #[test]
    fn mode_sets_overlap_without_being_forced_into_one_list() {
        assert_eq!(TS570D.modes.len(), 8);
        assert_eq!(FT991A.modes.len(), 14);

        // Shared vocabulary where the radios genuinely agree...
        for id in [ModeId::Lsb, ModeId::Usb, ModeId::Am, ModeId::Fm] {
            assert!(TS570D.supports_mode(id));
            assert!(FT991A.supports_mode(id));
        }

        // ...and divergence where they do not.
        assert!(FT991A.supports_mode(ModeId::C4fm));
        assert!(!TS570D.supports_mode(ModeId::C4fm));
        assert!(FT991A.supports_mode(ModeId::DataUsb));
        assert!(!TS570D.supports_mode(ModeId::DataUsb));
    }

    #[test]
    fn the_same_mode_can_carry_a_different_label_per_radio() {
        // Both radios have lower-sideband CW. Kenwood calls it "CW-R",
        // Yaesu "CW-L". The identity is shared; the label is not, and a
        // UI must show each radio's own word for it.
        assert_eq!(TS570D.mode(ModeId::CwLower).unwrap().label, "CW-R");
        assert_eq!(FT991A.mode(ModeId::CwLower).unwrap().label, "CW-L");
        assert_eq!(
            TS570D.mode(ModeId::CwLower).unwrap().kind,
            FT991A.mode(ModeId::CwLower).unwrap().kind
        );
    }

    #[test]
    fn wire_encodings_are_absent_and_would_have_collided_if_present() {
        // The concrete reason ModeDescriptor carries no wire code. Both
        // radios put 6 on the wire for their sixth mode -- FSK on the
        // TS-570D, RTTY-LSB on the FT-991A. A shared code table would have
        // been a table of coincidences.
        //
        // The model expresses this as two descriptors with the same
        // ModeId and different labels, and no byte anywhere.
        assert_eq!(TS570D.modes[5].id, ModeId::RttyLsb);
        assert_eq!(FT991A.modes[5].id, ModeId::RttyLsb);
        assert_eq!(TS570D.modes[5].label, "FSK");
        assert_eq!(FT991A.modes[5].label, "RTTY-L");
    }

    #[test]
    fn frequency_coverage_distinguishes_an_hf_radio_from_a_vhf_uhf_one() {
        assert!(TS570D.rx_range.contains(14_074_000));
        assert!(!TS570D.rx_range.contains(144_200_000));
        assert!(FT991A.rx_range.contains(14_074_000));
        assert!(FT991A.rx_range.contains(144_200_000));
        assert!(FT991A.rx_range.contains(432_100_000));
    }

    #[test]
    fn filter_differences_are_expressible_without_a_dead_control() {
        // The TS-570D exposes IF shift but no selectable width list over
        // CAT; the FT-991A exposes both. A UI reading these draws a width
        // selector for one radio and not the other -- which is the point
        // of the fields being independently optional.
        assert!(TS570D.filters.if_shift_hz.is_some());
        assert!(TS570D.filters.widths_hz.is_none());
        assert!(FT991A.filters.if_shift_hz.is_some());
        assert_eq!(FT991A.filters.widths_hz.unwrap().len(), 11);
    }

    #[test]
    fn memory_numbering_differs_at_both_ends() {
        // 0-99 versus 1-117. A consumer that assumed either a zero base or
        // a hundred-channel limit would be wrong on one of these radios,
        // which is why the range is data rather than a count.
        let ts = TS570D.memory.unwrap();
        let ft = FT991A.memory.unwrap();
        assert_eq!(ts.channels, RawRange::new(0, 99));
        assert_eq!(ft.channels, RawRange::new(1, 117));
        assert!(!ts.named);
        assert!(ft.named);
    }

    #[test]
    fn signal_capability_separates_a_tapped_radio_from_a_bare_one() {
        // The TS-570D has no bandscope but does have a CN4 IF tap, so it
        // can drive a panorama. The FT-991A has neither, and reports so.
        assert!(TS570D.signal.is_present());
        assert!(TS570D.signal.is_band_panorama());
        assert!(!FT991A.signal.is_present());
        assert!(!FT991A.signal.is_band_panorama());

        let SignalCapability::IfTap(config) = TS570D.signal else {
            panic!("TS-570D should report an IF tap");
        };
        assert_eq!(config.if_center_hz, 73_050_000);
        assert!(config.inverted, "LO1 is high-side, so the IF is mirrored");
    }

    #[test]
    fn every_question_is_answered_from_data_not_from_the_model_string() {
        // The gate's third forbidden hatch. This function asks the same
        // questions a consumer would, having deliberately thrown the model
        // names away -- if any answer required knowing which radio it was,
        // this could not be written as a loop.
        for radio in [TS570D, FT991A] {
            let anonymous = RadioCapabilities { model: "", ..radio };

            // Which handles to open.
            assert!(anonymous.endpoints.serves(EndpointRole::Cat));
            let extra_handles = anonymous.endpoints.handle_count() - 1;
            assert!(extra_handles <= 2);

            // Whether to draw a waterfall.
            let _ = anonymous.signal.is_band_panorama();

            // How to scale the S-meter.
            let s = anonymous.meters.find(MeterKind::S).unwrap();
            assert_eq!(s.raw_range.fraction(s.raw_range.max), 1.0);

            // Which mode buttons to render, and what to write on them.
            assert!(!anonymous.modes.is_empty());
            for mode in anonymous.modes {
                assert!(!mode.label.is_empty());
            }

            // Whether a width selector is meaningful.
            let _ = anonymous.filters.widths_hz.is_some();
        }
    }

    #[test]
    fn menu_capability_is_the_thinnest_part_of_the_model() {
        // RECORDED STRAIN, not a failure.
        //
        // `MenuCapability` is a count and a writability flag. That is
        // enough to answer "is there a menu, and how big", which is all
        // the protocol handshake and a menu-count display need.
        //
        // It is NOT enough to RENDER a menu. The FT-991A's EX_MENU_TABLE
        // has 152 entries, each with its own value type, range and label;
        // the TS-570D's 52 are a different shape again. Neither is
        // expressible here, and deliberately so: menu topology is
        // radio-specific and ADR 0011 leaves it in each app's own crate.
        //
        // The reason to write this down rather than quietly widen the
        // type: the temptation, the first time a GUI wants a menu screen,
        // will be to add a `&'static [MenuItemDescriptor]` here. That
        // would pull 152 FT-991A-shaped entries into the generic library
        // and re-create exactly the coupling this model exists to remove.
        // If it is ever needed, the right shape is the same one
        // `cat-signal` already uses for spectrum settings -- a list of
        // typed `SettingDescriptor`s the radio publishes -- not a new
        // bespoke menu type.
        assert_eq!(TS570D.menu.unwrap().item_count, 52);
        assert_eq!(FT991A.menu.unwrap().item_count, 152);
        assert!(FT991A.menu.unwrap().item_count > TS570D.menu.unwrap().item_count * 2);
    }
}
