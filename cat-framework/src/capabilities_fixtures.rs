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
use crate::installation::{Installation, InstalledSource, Session, SourceState};
use cat_signal::SignalCapability;

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
    // A MODEL fact: every TS-570D has a CN4 header on its TX-RX unit, at a
    // 73.05 MHz first IF, mirrored because LO1 is high-side injection
    // (73.05-103.05 MHz). Whether a dongle hangs off it is not a fact about
    // the model, and lives in an `Installation` instead -- ADR 0015, which
    // this fixture's previous shape is what prompted.
    signal: SignalSupport::IfTapPoint {
        if_center_hz: 73_050_000,
        inverted: true,
    },
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
    // No bandscope over CAT and no IF tap point. Verified against the
    // FT-991A CAT manual: it has menu items for its own scope display, but
    // no command that returns scope DATA (ADR 0010, Context).
    signal: SignalSupport::None,
};

// ---------------------------------------------------------------------------
// The same radio, two benches
// ---------------------------------------------------------------------------

/// A TS-570D with nothing optional attached: one serial cable, no dongle,
/// no soundcard.
///
/// The model description above is byte-for-byte the same one the fitted
/// station uses. That is the whole point of ADR 0015 -- a radio's
/// description does not change when someone unplugs something.
pub fn ts570d_bare() -> Installation {
    Installation::bare(vec![EndpointRole::Cat, EndpointRole::Keying])
}

/// A TS-570D behind the interface box: CAT serial with DTR keying through
/// ACC2, an RTL-SDR on the CN4 tap, and RF audio through ACC2 into a USB
/// sound device.
///
/// `trim_hz` is a measurement, not a constant. It belongs to one dongle's
/// crystal and is calibrated against a known carrier; a different dongle on
/// the same radio has a different number. That it can live here at all,
/// rather than as a placeholder zero inside a `const`, is what ADR 0015
/// bought.
pub fn ts570d_with_box(trim_hz: i32) -> Installation {
    let mut install = Installation::bare(vec![
        EndpointRole::Cat,
        EndpointRole::Keying,
        EndpointRole::Audio,
    ]);
    if let Some(tap) = Installation::if_tap_from(
        &TS570D,
        trim_hz,
        SourceState::Streaming,
        "RTL-SDR #0 on CN4",
    ) {
        install.sources.push(tap);
    }
    install.sources.push(InstalledSource::new(
        SignalCapability::AudioDerived {
            max_bandwidth_hz: 3_000,
        },
        // Configured, not Streaming: the audio path is wired and the
        // transport design does not exist yet. This is the state a `bool`
        // could not express.
        SourceState::Configured,
        "Box USB Audio from ACC2",
    ));
    install
}

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
    fn the_model_says_a_tap_is_possible_not_that_one_is_fitted() {
        // The distinction ADR 0015 exists for. Every TS-570D has a CN4
        // header; only some benches have a dongle on it.
        assert!(TS570D.signal.is_possible());
        assert!(!FT991A.signal.is_possible());

        let SignalSupport::IfTapPoint {
            if_center_hz,
            inverted,
        } = TS570D.signal
        else {
            panic!("the TS-570D has an IF tap point")
        };
        assert_eq!(if_center_hz, 73_050_000);
        assert!(inverted, "LO1 is high-side, so the IF is mirrored");
    }

    #[test]
    fn the_same_model_description_serves_both_benches() {
        // The property that makes RadioCapabilities a description of a
        // MODEL: unplugging a dongle must not change it.
        let bare = Session::new(&TS570D, ts570d_bare());
        let fitted = Session::new(&TS570D, ts570d_with_box(-1_420));
        assert_eq!(bare.radio.model, fitted.radio.model);
        assert_eq!(bare.radio.signal, fitted.radio.signal);
        assert_ne!(bare.installation, fitted.installation);
    }

    #[test]
    fn only_the_fitted_bench_can_draw_a_waterfall() {
        assert!(!Session::new(&TS570D, ts570d_bare()).has_panorama());
        assert!(Session::new(&TS570D, ts570d_with_box(-1_420)).has_panorama());
    }

    #[test]
    fn a_bare_ts570d_is_an_invitation_not_an_apology() {
        // The radio COULD take a tap and does not have one. That is the
        // state the console turns into "configure a source here", rather
        // than hiding the panel or greying it out.
        let bare = Session::new(&TS570D, ts570d_bare());
        assert!(bare.panorama_possible_but_absent());

        // An FT-991A is a different state entirely: no tap is possible, so
        // there is nothing to invite.
        let ft = Session::new(&FT991A, Installation::bare(vec![EndpointRole::Cat]));
        assert!(!ft.panorama_possible_but_absent());
    }

    #[test]
    fn a_station_can_run_two_sources_at_once() {
        // The finding that blocked the audio panels: one enum field could
        // not say this, and `ConsoleState` derived exactly one lane from
        // it, so there was no lane for audio at all.
        let fitted = ts570d_with_box(-1_420);
        assert_eq!(fitted.sources.len(), 2);
        assert!(fitted.band_panorama().is_some());
        assert!(fitted.audio().is_some());
    }

    #[test]
    fn audio_is_present_without_being_a_panorama() {
        let fitted = ts570d_with_box(-1_420);
        let audio = fitted.audio().unwrap();
        assert!(!audio.is_band_panorama());
        // ...and it is not the source a waterfall would pick up.
        assert_ne!(fitted.band_panorama().unwrap().capability, audio.capability);
    }

    #[test]
    fn configured_and_streaming_are_different_states() {
        // The middle state a bool could not hold. The tap is delivering;
        // the audio path is wired and silent because its transport design
        // does not exist yet. A console must tell those apart, or it
        // reports missing hardware that is sitting right there.
        let fitted = ts570d_with_box(-1_420);
        assert!(fitted.band_panorama().unwrap().is_streaming());
        assert!(!fitted.audio().unwrap().is_streaming());
        assert_eq!(fitted.audio().unwrap().state, SourceState::Configured);
    }

    #[test]
    fn the_trim_is_a_measurement_and_lives_with_the_bench() {
        // It used to be a placeholder zero inside a const, with a comment
        // apologising for it. Two dongles on the same radio now differ.
        let a = ts570d_with_box(-1_420);
        let b = ts570d_with_box(880);
        assert_ne!(a.band_panorama(), b.band_panorama());

        let SignalCapability::IfTap(cfg) = a.band_panorama().unwrap().capability else {
            panic!("expected an IF tap")
        };
        assert_eq!(cfg.trim_hz, -1_420);
        // ...while the two facts that come from the radio are identical.
        assert_eq!(cfg.if_center_hz, 73_050_000);
        assert!(cfg.inverted);
    }

    #[test]
    fn the_box_adds_an_audio_endpoint_the_bare_radio_does_not_have() {
        assert!(!ts570d_bare().is_connected(EndpointRole::Audio));
        assert!(ts570d_with_box(0).is_connected(EndpointRole::Audio));
        // Both still share one handle for CAT and keying: that is the
        // radio's RS-232C port, and a fact about the model.
        for install in [ts570d_bare(), ts570d_with_box(0)] {
            assert!(install.is_connected(EndpointRole::Cat));
            assert!(install.is_connected(EndpointRole::Keying));
        }
    }

    #[test]
    fn a_radio_with_no_tap_point_cannot_be_given_one() {
        // if_tap_from reads the MODEL. An installation cannot invent a tap
        // on a radio that has no header for it, which is what stops
        // installation data from quietly becoming a second source of truth
        // about the radio.
        assert!(Installation::if_tap_from(&FT991A, -1_420, SourceState::Streaming, "x").is_none());
        assert!(Installation::if_tap_from(&TS570D, -1_420, SourceState::Streaming, "x").is_some());
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

            // Whether a waterfall is even possible.
            let _ = anonymous.signal.is_possible();

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
