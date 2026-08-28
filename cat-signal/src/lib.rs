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

//! Normalized spectrum data: one frame type, one source trait.
//!
//! See `docs/adr/0010-capability-model-and-normalized-signal-source.md`
//! (sections 3-4). Task 12 of `planning/architect/task_plan.md`.
//!
//! # The invariant this crate exists to enforce
//!
//! **[`SpectrumFrame::bins`] is always low-frequency-first.** Every
//! correction — IF inversion, local-oscillator tracking, crystal trim —
//! happens inside a [`SpectrumSource`], never in a consumer.
//!
//! That one rule is what makes a TS-570D IF tap and a radio with a native
//! bandscope interchangeable to the code that draws them. The TS-570D's
//! first IF is 73.05 MHz with high-side LO injection, so its tapped
//! spectrum arrives mirrored; a consumer that had to know this would have
//! to know it about every radio, and would get it wrong for the next one.
//!
//! # Why a source describes its own settings
//!
//! A source's *type* determines which knobs exist: an IF tap has a
//! calibration trim, a native scope has a sweep speed, an audio-derived
//! source has an input device. Rather than a UI switching on
//! [`SignalCapability`] and hand-writing a panel per variant, each source
//! publishes [`SettingDescriptor`]s and the UI renders them generically
//! (see section 4 of the ADR, and
//! `docs/adr/0011-cat-ui-base-widgets-radio-specific-layout.md`).
//!
//! The TS-570D's `trim_hz` is the sharpest case. It is a real per-station
//! calibration a user must be able to set, it exists for no other source
//! type, and it must never reach a UI as a hand-written special case.

use async_trait::async_trait;

/// One spectrum update, already corrected.
///
/// No consumer of this type knows about IF inversion, LO tracking, or
/// crystal trim.
#[derive(Debug, Clone, PartialEq)]
pub struct SpectrumFrame {
    /// Centre of the span in real RF terms — the dial frequency an
    /// operator would read, not an intermediate frequency.
    pub center_hz: u64,
    pub span_hz: u32,
    /// Power that the top of the display should represent, in dBm.
    pub ref_level_dbm: f32,
    /// Bin powers in dBm, **always low frequency first**.
    pub bins: Vec<f32>,
    /// Monotonically increasing per source, so a consumer can detect
    /// dropped frames rather than silently rendering a stale one.
    pub sequence: u64,
}

impl SpectrumFrame {
    /// Hz per bin, or 0 for an empty frame.
    pub fn bin_width_hz(&self) -> f64 {
        if self.bins.is_empty() {
            return 0.0;
        }
        f64::from(self.span_hz) / self.bins.len() as f64
    }

    /// Frequency at the centre of bin `index`.
    ///
    /// Relies on the low-frequency-first invariant: bin 0 is the lowest
    /// frequency in the span, on every source, always.
    pub fn bin_frequency_hz(&self, index: usize) -> Option<f64> {
        if index >= self.bins.len() {
            return None;
        }
        let start = self.center_hz as f64 - f64::from(self.span_hz) / 2.0;
        Some(start + (index as f64 + 0.5) * self.bin_width_hz())
    }

    /// Inclusive frequency bounds of this frame, low first.
    pub fn range_hz(&self) -> (f64, f64) {
        let half = f64::from(self.span_hz) / 2.0;
        (self.center_hz as f64 - half, self.center_hz as f64 + half)
    }
}

/// The three TS-570D corrections, as data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IfTapConfig {
    /// The radio's first IF. 73.05 MHz on the TS-570D.
    pub if_center_hz: u64,
    /// Whether the tapped spectrum is mirrored. `true` on the TS-570D,
    /// which uses high-side LO1 injection.
    pub inverted: bool,
    /// Calibrated once per station against a known carrier (WWV).
    ///
    /// A constant, not a ppm figure: the SDR is parked on the IF and never
    /// retunes, so its crystal error is a fixed Hz offset rather than one
    /// that scales with frequency. That is the whole reason this is a
    /// single number a user can set from a settings panel.
    pub trim_hz: i32,
}

/// Where a radio's spectrum can come from, if anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignalCapability {
    /// No spectrum at all. A first-class state, not an error: a TS-570D
    /// with nothing wired to CN4 reports this, and a console must stay
    /// recognisably itself when it does.
    None,
    /// Band panorama from the radio's own scope, over CAT.
    ///
    /// Defined but **not implemented**: no radio currently in this fleet
    /// exports a bandscope over CAT (ADR 0010, Context). It exists so that
    /// adding such a radio is a new source, not a new abstraction.
    NativeScope { max_span_hz: u32, bins: u16 },
    /// Band panorama from an SDR on a fixed IF tap.
    IfTap(IfTapConfig),
    /// Band panorama from an SDR tuned independently of the radio.
    DirectSdr { tunable_range_hz: (u64, u64) },
    /// Audio-bandwidth only. Drives an AF FFT or scope, **never** a band
    /// waterfall.
    ///
    /// Carries `max_bandwidth_hz` specifically so a consumer can refuse to
    /// render it as a panorama. A capability that lies by omission is
    /// worse than one that is absent.
    AudioDerived { max_bandwidth_hz: u32 },
}

impl SignalCapability {
    /// Whether this source can legitimately drive a band panorama.
    ///
    /// The question every waterfall consumer must ask, answered once here
    /// so that `AudioDerived` cannot be mistaken for a band source by a
    /// consumer that simply forgot the variant existed.
    pub fn is_band_panorama(&self) -> bool {
        matches!(
            self,
            SignalCapability::NativeScope { .. }
                | SignalCapability::IfTap(_)
                | SignalCapability::DirectSdr { .. }
        )
    }

    /// Whether any spectrum data is available at all.
    pub fn is_present(&self) -> bool {
        !matches!(self, SignalCapability::None)
    }
}

/// Physical unit a setting is expressed in, so a UI can label it without
/// parsing the key name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unit {
    None,
    Hz,
    Db,
    Dbm,
    /// Samples per second.
    Sps,
    Percent,
    Seconds,
}

impl Unit {
    /// Suffix to render after a value. Empty for [`Unit::None`].
    pub fn suffix(&self) -> &'static str {
        match self {
            Unit::None => "",
            Unit::Hz => "Hz",
            Unit::Db => "dB",
            Unit::Dbm => "dBm",
            Unit::Sps => "S/s",
            Unit::Percent => "%",
            Unit::Seconds => "s",
        }
    }
}

/// Whether a consumer may write a setting back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    ReadOnly,
    ReadWrite,
}

/// Coarse grouping, so a generic panel can lay settings out sensibly
/// without knowing what any of them mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SettingGroup {
    /// Where the signal comes from: device, sample rate, gain.
    Source,
    /// How it is presented: FFT size, averaging, reference level.
    Display,
    /// Per-station corrections that are measured once and left alone.
    Calibration,
}

/// A setting's current value, its bounds, and enough type information for
/// a UI to pick a control.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Int {
        value: i64,
        min: i64,
        max: i64,
        step: i64,
        unit: Unit,
    },
    Float {
        value: f64,
        min: f64,
        max: f64,
        unit: Unit,
    },
    Bool(bool),
    Enum {
        value: u16,
        options: &'static [&'static str],
    },
}

impl SettingValue {
    /// Whether `self` is within its own declared bounds.
    ///
    /// A source validates a written value against this before applying it,
    /// so every source rejects out-of-range input the same way instead of
    /// each re-implementing the comparison.
    pub fn is_valid(&self) -> bool {
        match self {
            SettingValue::Int {
                value, min, max, ..
            } => value >= min && value <= max,
            SettingValue::Float {
                value, min, max, ..
            } => value >= min && value <= max,
            SettingValue::Bool(_) => true,
            SettingValue::Enum { value, options } => (*value as usize) < options.len(),
        }
    }

    /// Whether `other` is the same *kind* of value as `self`.
    ///
    /// Guards the [`SpectrumSource::apply`] path: a caller that sends a
    /// `Bool` for an `Int` setting is making a category error, and the
    /// source should say so rather than coerce.
    pub fn same_kind_as(&self, other: &SettingValue) -> bool {
        matches!(
            (self, other),
            (SettingValue::Int { .. }, SettingValue::Int { .. })
                | (SettingValue::Float { .. }, SettingValue::Float { .. })
                | (SettingValue::Bool(_), SettingValue::Bool(_))
                | (SettingValue::Enum { .. }, SettingValue::Enum { .. })
        )
    }
}

/// One knob a source exposes.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingDescriptor {
    /// Stable identifier, used by [`SpectrumSource::apply`]. Never shown
    /// to a user.
    pub key: &'static str,
    /// Human-readable label. This is what a UI displays.
    pub label: &'static str,
    pub group: SettingGroup,
    pub access: Access,
    pub value: SettingValue,
}

/// Everything a source lets a consumer see or change.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpectrumSettings {
    pub descriptors: Vec<SettingDescriptor>,
}

impl SpectrumSettings {
    pub fn new(descriptors: Vec<SettingDescriptor>) -> Self {
        Self { descriptors }
    }

    pub fn find(&self, key: &str) -> Option<&SettingDescriptor> {
        self.descriptors.iter().find(|d| d.key == key)
    }

    /// Descriptors in one group, in declaration order.
    pub fn group(&self, group: SettingGroup) -> impl Iterator<Item = &SettingDescriptor> {
        self.descriptors.iter().filter(move |d| d.group == group)
    }

    /// Whether `key` exists and may be written.
    pub fn is_writable(&self, key: &str) -> bool {
        self.find(key)
            .is_some_and(|d| d.access == Access::ReadWrite)
    }
}

/// A source of normalized spectrum frames.
///
/// `#[async_trait(?Send)]` matches the house binding from
/// `docs/adr/0002-async-runtime-binding-for-transport-crates.md`.
#[async_trait(?Send)]
pub trait SpectrumSource {
    type Error;

    /// Wait for and return the next frame, already corrected.
    async fn next_frame(&mut self) -> Result<SpectrumFrame, Self::Error>;

    /// What kind of source this is, and what it can therefore be used for.
    fn capability(&self) -> SignalCapability;

    /// The knobs this source exposes, with current values.
    fn settings(&self) -> SpectrumSettings;

    /// Write one setting by key.
    ///
    /// Implementations reject an unknown key, a read-only key, a value of
    /// the wrong kind, and a value outside its declared bounds.
    fn apply(&mut self, key: &str, value: SettingValue) -> Result<(), Self::Error>;

    /// Tell the source the radio's dial has moved.
    ///
    /// An IF-tap source uses this to retrack: the SDR itself never
    /// retunes — it stays parked on the IF — so this only changes the
    /// `center_hz` the source reports.
    fn retune(&mut self, dial_hz: u64);
}

pub mod fake;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeSourceError, FakeSpectrumSource};
    use futures::executor::block_on;

    // -----------------------------------------------------------------
    // Invariant 1: bins are always low-frequency-first.
    //
    // This is the load-bearing rule of the whole crate, so it is asserted
    // rather than documented. Everything downstream -- the waterfall
    // texture upload, the native protocol's binary frames, the click-to-
    // tune mapping -- reads bin 0 as the lowest frequency.
    // -----------------------------------------------------------------

    #[test]
    fn bin_frequencies_increase_with_index() {
        let frame = block_on(FakeSpectrumSource::new().next_frame()).unwrap();
        let first = frame.bin_frequency_hz(0).unwrap();
        let last = frame.bin_frequency_hz(frame.bins.len() - 1).unwrap();
        assert!(
            first < last,
            "bin 0 ({first}) must be lower in frequency than the last bin ({last})"
        );

        for i in 1..frame.bins.len() {
            let prev = frame.bin_frequency_hz(i - 1).unwrap();
            let cur = frame.bin_frequency_hz(i).unwrap();
            assert!(cur > prev, "bin {i} went backwards in frequency");
        }
    }

    #[test]
    fn a_signal_above_centre_lands_in_the_upper_half() {
        // The end-to-end orientation check, in miniature. The fixture puts
        // its peak at 75% of the span; if anything mirrors the bins, the
        // peak lands at 25% and this fails.
        let source = FakeSpectrumSource::new();
        let expected = source.peak_bin();
        let mut source = source;
        let frame = block_on(source.next_frame()).unwrap();

        let actual = frame
            .bins
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap();

        assert_eq!(actual, expected);
        assert!(actual > frame.bins.len() / 2, "peak should be above centre");
        assert!(frame.bin_frequency_hz(actual).unwrap() > frame.center_hz as f64);
    }

    #[test]
    fn frame_geometry_is_self_consistent() {
        let frame = block_on(FakeSpectrumSource::new().next_frame()).unwrap();
        let (low, high) = frame.range_hz();

        assert!(frame.bin_frequency_hz(0).unwrap() > low);
        assert!(frame.bin_frequency_hz(frame.bins.len() - 1).unwrap() < high);
        assert!((high - low - f64::from(frame.span_hz)).abs() < 1e-6);
        assert!(frame.bin_frequency_hz(frame.bins.len()).is_none());
    }

    #[test]
    fn an_empty_frame_does_not_divide_by_zero() {
        let frame = SpectrumFrame {
            center_hz: 14_074_000,
            span_hz: 48_000,
            ref_level_dbm: -20.0,
            bins: Vec::new(),
            sequence: 1,
        };
        assert_eq!(frame.bin_width_hz(), 0.0);
        assert!(frame.bin_frequency_hz(0).is_none());
    }

    // -----------------------------------------------------------------
    // Invariant 2: AudioDerived cannot be mistaken for a band source.
    // -----------------------------------------------------------------

    #[test]
    fn audio_derived_is_not_a_band_panorama() {
        let audio = SignalCapability::AudioDerived {
            max_bandwidth_hz: 4_000,
        };
        assert!(!audio.is_band_panorama());
        // ...but it IS present. "Cannot draw a panorama" and "has no
        // signal at all" are different states, and a UI that conflates
        // them hides a working AF scope.
        assert!(audio.is_present());

        let SignalCapability::AudioDerived { max_bandwidth_hz } = audio else {
            unreachable!()
        };
        assert_eq!(max_bandwidth_hz, 4_000);
    }

    #[test]
    fn every_band_source_reports_itself_as_one() {
        assert!(SignalCapability::IfTap(IfTapConfig {
            if_center_hz: 73_050_000,
            inverted: true,
            trim_hz: 0,
        })
        .is_band_panorama());
        assert!(SignalCapability::DirectSdr {
            tunable_range_hz: (24_000_000, 1_766_000_000)
        }
        .is_band_panorama());
        assert!(SignalCapability::NativeScope {
            max_span_hz: 1_000_000,
            bins: 475
        }
        .is_band_panorama());
    }

    #[test]
    fn absent_is_absent_and_draws_nothing() {
        assert!(!SignalCapability::None.is_present());
        assert!(!SignalCapability::None.is_band_panorama());
    }

    // -----------------------------------------------------------------
    // Retune: the dial moves, the SDR does not.
    // -----------------------------------------------------------------

    #[test]
    fn retune_moves_the_reported_centre() {
        let mut source = FakeSpectrumSource::new();
        source.retune(7_100_000);
        let frame = block_on(source.next_frame()).unwrap();
        assert_eq!(frame.center_hz, 7_100_000);
        // Orientation survives a retune -- the correction is not applied
        // per-frame from a stale dial value.
        assert!(frame.bin_frequency_hz(0).unwrap() < 7_100_000.0);
    }

    #[test]
    fn sequence_numbers_advance_so_dropped_frames_are_detectable() {
        let mut source = FakeSpectrumSource::new();
        let a = block_on(source.next_frame()).unwrap();
        let b = block_on(source.next_frame()).unwrap();
        assert_eq!(a.sequence + 1, b.sequence);
    }

    #[test]
    fn a_source_can_fail_a_frame_without_hardware() {
        let mut source = FakeSpectrumSource::new();
        source.fail_next_frame();
        assert_eq!(
            block_on(source.next_frame()),
            Err(FakeSourceError::Injected)
        );
        // and recovers
        assert!(block_on(source.next_frame()).is_ok());
    }

    // -----------------------------------------------------------------
    // Delegated settings.
    // -----------------------------------------------------------------

    #[test]
    fn settings_are_found_by_key_and_grouped_for_layout() {
        let source = FakeSpectrumSource::new();
        let settings = source.settings();

        assert!(settings.find("span_hz").is_some());
        assert!(settings.find("nonexistent").is_none());
        assert_eq!(settings.group(SettingGroup::Display).count(), 2);
        assert_eq!(settings.group(SettingGroup::Source).count(), 1);
        assert_eq!(settings.group(SettingGroup::Calibration).count(), 0);
    }

    #[test]
    fn a_read_only_setting_is_reported_as_such_and_rejected_on_write() {
        let mut source = FakeSpectrumSource::new();
        assert!(source.settings().is_writable("span_hz"));
        assert!(!source.settings().is_writable("center_hz"));

        let err = source.apply(
            "center_hz",
            SettingValue::Int {
                value: 1,
                min: 0,
                max: i64::MAX,
                step: 1,
                unit: Unit::Hz,
            },
        );
        assert_eq!(err, Err(FakeSourceError::ReadOnly("center_hz")));
    }

    #[test]
    fn apply_rejects_unknown_keys_wrong_kinds_and_out_of_range_values() {
        let mut source = FakeSpectrumSource::new();

        assert!(matches!(
            source.apply("nope", SettingValue::Bool(true)),
            Err(FakeSourceError::UnknownKey(_))
        ));

        assert_eq!(
            source.apply("span_hz", SettingValue::Bool(true)),
            Err(FakeSourceError::WrongKind("span_hz"))
        );

        assert_eq!(
            source.apply(
                "span_hz",
                SettingValue::Int {
                    value: 999_999_999,
                    min: 1_000,
                    max: 2_400_000,
                    step: 1_000,
                    unit: Unit::Hz,
                },
            ),
            Err(FakeSourceError::OutOfRange("span_hz"))
        );
    }

    #[test]
    fn a_valid_write_takes_effect_in_the_next_frame() {
        let mut source = FakeSpectrumSource::new();
        source
            .apply(
                "span_hz",
                SettingValue::Int {
                    value: 96_000,
                    min: 1_000,
                    max: 2_400_000,
                    step: 1_000,
                    unit: Unit::Hz,
                },
            )
            .unwrap();
        assert_eq!(block_on(source.next_frame()).unwrap().span_hz, 96_000);
    }

    #[test]
    fn enum_settings_are_bounds_checked_against_their_option_list() {
        let options: &[&str] = &["128", "256", "512"];
        assert!(SettingValue::Enum { value: 2, options }.is_valid());
        assert!(!SettingValue::Enum { value: 3, options }.is_valid());
    }

    #[test]
    fn if_tap_config_carries_the_three_corrections_as_data() {
        // The TS-570D's actual numbers. `trim_hz` is a constant rather
        // than a ppm figure because the dongle never retunes -- see the
        // field's own doc comment.
        let ts570d = IfTapConfig {
            if_center_hz: 73_050_000,
            inverted: true,
            trim_hz: -1_240,
        };
        assert!(ts570d.inverted);
        assert_eq!(ts570d.if_center_hz, 73_050_000);
        assert!(SignalCapability::IfTap(ts570d).is_band_panorama());
    }

    #[test]
    fn units_render_a_suffix_without_the_consumer_parsing_key_names() {
        assert_eq!(Unit::Hz.suffix(), "Hz");
        assert_eq!(Unit::Dbm.suffix(), "dBm");
        assert_eq!(Unit::None.suffix(), "");
    }
}
