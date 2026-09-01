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

//! An RTL-SDR [`SpectrumSource`](cat_signal::SpectrumSource), for a dongle
//! parked on a radio's IF tap.
//!
//! See `docs/adr/0014-rtlsdr-spectrum-source.md`. Task 15 of
//! `planning/architect/task_plan.md`.
//!
//! # The one thing to understand about this crate
//!
//! **The dongle never retunes.** It sits on the radio's first IF — 73.05
//! MHz for the TS-570D — and stays there.
//! [`retune`](cat_signal::SpectrumSource::retune) changes only the centre
//! frequency the source *reports*; it issues no USB control transfer.
//!
//! That is not an optimization, it is what makes the calibration tractable:
//! a parked receiver's crystal error is a fixed Hz offset rather than one
//! that scales with frequency, so the whole correction is a single
//! [`IfTapConfig::trim_hz`](cat_signal::IfTapConfig::trim_hz) a user
//! measures once against WWV. Anyone who "fixes" this by calling
//! `set_center_freq` silently invalidates that number.
//!
//! # Layout
//!
//! - [`dsp`] — IQ to corrected frame. Platform-independent, hardware-free,
//!   and where all the correctness lives.
//! - `device` — the librtlsdr worker thread, behind the `device` feature.
//!
//! The split exists so the corrections that are easy to get silently wrong
//! (FFT shift, IF inversion, trim) are testable without a radio on the
//! bench. See [`RtlSdrSource`] for the assembled source, and
//! [`IqSource`] for the seam that lets a test drive it.

pub mod dsp;
pub mod rtl_tcp;

#[cfg(feature = "device")]
pub mod device;

use cat_signal::{
    Access, IfTapConfig, SettingDescriptor, SettingGroup, SettingValue, SignalCapability,
    SpectrumFrame, SpectrumSettings, SpectrumSource, Unit,
};
use dsp::SpectrumPipeline;
use rustfft::num_complex::Complex32;

/// Anything that can hand over a block of IQ samples.
///
/// The seam between the DSP and the hardware. A real implementation reads
/// from a worker thread fed by librtlsdr; a test implementation replays a
/// synthetic tone. Both go through exactly the same corrections, which is
/// the point — a fixture that bypassed the pipeline would test nothing.
pub use rtl_tcp::{RtlTcpError, RtlTcpSource};

pub trait IqSource {
    type Error;

    /// Block until at least `wanted` samples are available, or fail.
    fn read(&mut self, wanted: usize) -> Result<Vec<Complex32>, Self::Error>;

    /// Frames the source produced but could not deliver because the
    /// consumer was behind.
    ///
    /// Per ADR 0014 §3 the newest frame wins and the rest are dropped; this
    /// is what turns "the waterfall feels laggy" into a number.
    fn frames_dropped(&self) -> u64 {
        0
    }
}

/// Errors this source can report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtlSdrError<E> {
    /// The IQ source failed.
    Device(E),
    UnknownSetting(String),
    ReadOnly(&'static str),
    WrongKind(&'static str),
    OutOfRange(&'static str),
}

impl<E: std::fmt::Display> std::fmt::Display for RtlSdrError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RtlSdrError::Device(e) => write!(f, "SDR device error: {e}"),
            RtlSdrError::UnknownSetting(k) => write!(f, "unknown setting: {k}"),
            RtlSdrError::ReadOnly(k) => write!(f, "setting is read-only: {k}"),
            RtlSdrError::WrongKind(k) => write!(f, "wrong value kind for setting: {k}"),
            RtlSdrError::OutOfRange(k) => write!(f, "value out of range: {k}"),
        }
    }
}

impl<E: std::fmt::Debug + std::fmt::Display> std::error::Error for RtlSdrError<E> {}

/// The assembled source: an [`IqSource`] plus the correction pipeline.
pub struct RtlSdrSource<S: IqSource> {
    iq: S,
    pipeline: SpectrumPipeline,
    gain_db: f64,
}

impl<S: IqSource> RtlSdrSource<S> {
    pub fn new(iq: S, sample_rate_hz: u32, fft_size: usize, config: IfTapConfig) -> Self {
        Self {
            iq,
            pipeline: SpectrumPipeline::new(fft_size, sample_rate_hz, config),
            gain_db: 0.0,
        }
    }

    pub fn pipeline(&self) -> &SpectrumPipeline {
        &self.pipeline
    }
}

#[async_trait::async_trait(?Send)]
impl<S: IqSource> SpectrumSource for RtlSdrSource<S> {
    type Error = RtlSdrError<S::Error>;

    async fn next_frame(&mut self) -> Result<SpectrumFrame, Self::Error> {
        loop {
            let wanted = self.pipeline.fft_size();
            let iq = self.iq.read(wanted).map_err(RtlSdrError::Device)?;
            if let Some(frame) = self.pipeline.process(&iq) {
                return Ok(frame);
            }
            // A short read is not an error; ask again.
        }
    }

    fn capability(&self) -> SignalCapability {
        SignalCapability::IfTap(self.pipeline.config())
    }

    fn settings(&self) -> SpectrumSettings {
        let config = self.pipeline.config();
        SpectrumSettings::new(vec![
            SettingDescriptor {
                key: "if_center_hz",
                label: "IF centre",
                group: SettingGroup::Source,
                access: Access::ReadOnly,
                value: SettingValue::Int {
                    value: config.if_center_hz as i64,
                    min: 0,
                    max: i64::MAX,
                    step: 1,
                    unit: Unit::Hz,
                },
            },
            SettingDescriptor {
                key: "inverted",
                label: "Inverted IF",
                group: SettingGroup::Source,
                access: Access::ReadOnly,
                value: SettingValue::Bool(config.inverted),
            },
            // The whole reason ADR 0010 §4 exists. A real per-station
            // calibration, in a generic list, with no bespoke UI treatment.
            SettingDescriptor {
                key: "trim_hz",
                label: "Frequency trim",
                group: SettingGroup::Calibration,
                access: Access::ReadWrite,
                value: SettingValue::Int {
                    value: i64::from(config.trim_hz),
                    min: -50_000,
                    max: 50_000,
                    step: 1,
                    unit: Unit::Hz,
                },
            },
            SettingDescriptor {
                key: "sample_rate_hz",
                label: "Sample rate",
                group: SettingGroup::Source,
                access: Access::ReadOnly,
                value: SettingValue::Int {
                    value: i64::from(self.pipeline.sample_rate_hz()),
                    min: 225_001,
                    max: 3_200_000,
                    step: 1,
                    unit: Unit::Sps,
                },
            },
            SettingDescriptor {
                key: "gain_db",
                label: "Gain",
                group: SettingGroup::Source,
                access: Access::ReadWrite,
                value: SettingValue::Float {
                    value: self.gain_db,
                    min: 0.0,
                    max: 49.6,
                    unit: Unit::Db,
                },
            },
            SettingDescriptor {
                key: "fft_size",
                label: "FFT size",
                group: SettingGroup::Display,
                access: Access::ReadWrite,
                value: SettingValue::Enum {
                    value: fft_size_index(self.pipeline.fft_size()),
                    options: FFT_SIZES,
                },
            },
            SettingDescriptor {
                key: "frames_dropped",
                label: "Frames dropped",
                group: SettingGroup::Display,
                access: Access::ReadOnly,
                value: SettingValue::Int {
                    value: self.iq.frames_dropped() as i64,
                    min: 0,
                    max: i64::MAX,
                    step: 1,
                    unit: Unit::None,
                },
            },
        ])
    }

    fn apply(&mut self, key: &str, value: SettingValue) -> Result<(), Self::Error> {
        let descriptor = self
            .settings()
            .find(key)
            .cloned()
            .ok_or_else(|| RtlSdrError::UnknownSetting(key.to_string()))?;

        if descriptor.access == Access::ReadOnly {
            return Err(RtlSdrError::ReadOnly(descriptor.key));
        }
        if !descriptor.value.same_kind_as(&value) {
            return Err(RtlSdrError::WrongKind(descriptor.key));
        }
        if !value.is_valid() {
            return Err(RtlSdrError::OutOfRange(descriptor.key));
        }

        match (descriptor.key, &value) {
            ("trim_hz", SettingValue::Int { value: v, .. }) => {
                self.pipeline.set_trim_hz(*v as i32);
            }
            ("gain_db", SettingValue::Float { value: v, .. }) => self.gain_db = *v,
            ("fft_size", SettingValue::Enum { value: v, .. }) => {
                self.pipeline
                    .set_fft_size(FFT_SIZES[*v as usize].parse().expect("static table"));
            }
            _ => return Err(RtlSdrError::UnknownSetting(key.to_string())),
        }
        Ok(())
    }

    fn retune(&mut self, dial_hz: u64) {
        // Note what is NOT here: any call into the device. See this
        // module's header, and ADR 0014 §5.
        self.pipeline.set_dial_hz(dial_hz);
    }
}

const FFT_SIZES: &[&str] = &["256", "512", "1024", "2048", "4096"];

fn fft_size_index(size: usize) -> u16 {
    FFT_SIZES
        .iter()
        .position(|s| s.parse::<usize>() == Ok(size))
        .unwrap_or(2) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;

    /// An IQ source that replays a fixed complex tone, so the assembled
    /// source can be exercised end to end without a dongle.
    struct ToneIq {
        offset_hz: f64,
        sample_rate_hz: u32,
        phase: usize,
        fail: bool,
        dropped: u64,
    }

    impl ToneIq {
        fn new(offset_hz: f64, sample_rate_hz: u32) -> Self {
            Self {
                offset_hz,
                sample_rate_hz,
                phase: 0,
                fail: false,
                dropped: 0,
            }
        }
    }

    impl IqSource for ToneIq {
        type Error = &'static str;

        fn read(&mut self, wanted: usize) -> Result<Vec<Complex32>, Self::Error> {
            if self.fail {
                return Err("device gone");
            }
            let out = (0..wanted)
                .map(|i| {
                    let n = self.phase + i;
                    let p = std::f64::consts::TAU * self.offset_hz * n as f64
                        / f64::from(self.sample_rate_hz);
                    Complex32::new(p.cos() as f32, p.sin() as f32)
                })
                .collect();
            self.phase += wanted;
            Ok(out)
        }

        fn frames_dropped(&self) -> u64 {
            self.dropped
        }
    }

    const TS570D: IfTapConfig = IfTapConfig {
        if_center_hz: 73_050_000,
        inverted: true,
        trim_hz: 0,
    };

    fn source(offset_hz: f64) -> RtlSdrSource<ToneIq> {
        let mut s = RtlSdrSource::new(ToneIq::new(offset_hz, 240_000), 240_000, 1024, TS570D);
        s.retune(14_074_000);
        s
    }

    #[test]
    fn the_assembled_source_reports_an_if_tap() {
        let s = source(0.0);
        assert!(s.capability().is_band_panorama());
        let SignalCapability::IfTap(config) = s.capability() else {
            panic!("expected IfTap")
        };
        assert_eq!(config.if_center_hz, 73_050_000);
        assert!(config.inverted);
    }

    #[test]
    fn a_signal_above_the_dial_reaches_the_consumer_on_the_right() {
        // Same invariant as dsp's test, asserted through the full public
        // path a consumer actually uses.
        let mut s = source(-60_000.0);
        let frame = block_on(s.next_frame()).unwrap();
        let peak = frame
            .bins
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        assert!(peak > frame.bins.len() / 2);
        assert_eq!(frame.center_hz, 14_074_000);
    }

    #[test]
    fn trim_is_one_row_in_a_generic_list_not_a_special_case() {
        // ADR 0010 §4's point, asserted: `trim_hz` is discoverable and
        // writable through exactly the same generic path as every other
        // setting, so a UI needs no TS-570D knowledge to offer it.
        let mut s = source(0.0);
        let settings = s.settings();
        let trim = settings.find("trim_hz").unwrap();
        assert_eq!(trim.group, SettingGroup::Calibration);
        assert_eq!(trim.access, Access::ReadWrite);

        s.apply(
            "trim_hz",
            SettingValue::Int {
                value: -1_240,
                min: -50_000,
                max: 50_000,
                step: 1,
                unit: Unit::Hz,
            },
        )
        .unwrap();

        assert_eq!(
            block_on(s.next_frame()).unwrap().center_hz,
            14_074_000 - 1_240
        );
    }

    #[test]
    fn the_immutable_facts_about_the_tap_are_read_only() {
        // if_center_hz and inverted are properties of the radio's design.
        // A user who could edit them would break the correction silently.
        let mut s = source(0.0);
        assert!(!s.settings().is_writable("if_center_hz"));
        assert!(!s.settings().is_writable("inverted"));
        assert_eq!(
            s.apply("inverted", SettingValue::Bool(false)),
            Err(RtlSdrError::ReadOnly("inverted"))
        );
    }

    #[test]
    fn a_device_failure_surfaces_rather_than_hanging() {
        let mut s = source(0.0);
        s.iq.fail = true;
        assert_eq!(
            block_on(s.next_frame()),
            Err(RtlSdrError::Device("device gone"))
        );
    }

    #[test]
    fn dropped_frames_are_visible_to_a_user() {
        let mut s = source(0.0);
        s.iq.dropped = 42;
        let settings = s.settings();
        let dropped = settings.find("frames_dropped").unwrap();
        assert_eq!(dropped.access, Access::ReadOnly);
        assert!(matches!(dropped.value, SettingValue::Int { value: 42, .. }));
    }

    #[test]
    fn changing_fft_size_changes_the_frame_width() {
        let mut s = source(0.0);
        assert_eq!(block_on(s.next_frame()).unwrap().bins.len(), 1024);
        s.apply(
            "fft_size",
            SettingValue::Enum {
                value: 3,
                options: FFT_SIZES,
            },
        )
        .unwrap();
        assert_eq!(block_on(s.next_frame()).unwrap().bins.len(), 2048);
    }

    #[test]
    fn out_of_range_and_wrong_kind_writes_are_rejected() {
        let mut s = source(0.0);
        assert_eq!(
            s.apply(
                "trim_hz",
                SettingValue::Int {
                    value: 999_999,
                    min: -50_000,
                    max: 50_000,
                    step: 1,
                    unit: Unit::Hz
                }
            ),
            Err(RtlSdrError::OutOfRange("trim_hz"))
        );
        assert_eq!(
            s.apply("trim_hz", SettingValue::Bool(true)),
            Err(RtlSdrError::WrongKind("trim_hz"))
        );
        assert!(matches!(
            s.apply("nope", SettingValue::Bool(true)),
            Err(RtlSdrError::UnknownSetting(_))
        ));
    }
}
