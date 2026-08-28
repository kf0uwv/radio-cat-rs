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

//! [`FakeSpectrumSource`]: a deterministic [`SpectrumSource`] with no
//! hardware behind it.
//!
//! Mirrors `cat-server::test_fixtures` and
//! `cat_transport_core::test_support`'s role in this workspace: one shared
//! test double, not a per-crate reinvention. It ships in the library
//! rather than behind `#[cfg(test)]` because the consumers that need it
//! most — `cat-server`'s protocol tests (Task 16) and the waterfall widget
//! (Task 19) — live in other crates.
//!
//! It emits a single peak at a known offset from centre, which makes it
//! useful for the one thing a fake normally cannot check: **orientation**.
//! A renderer that mirrors its bins will put the peak on the wrong side,
//! and [`FakeSpectrumSource::peak_bin`] says where it should have been.

use crate::{
    Access, SettingDescriptor, SettingGroup, SettingValue, SignalCapability, SpectrumFrame,
    SpectrumSettings, SpectrumSource, Unit,
};
use async_trait::async_trait;

/// Errors a [`FakeSpectrumSource`] can report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeSourceError {
    UnknownKey(String),
    ReadOnly(&'static str),
    WrongKind(&'static str),
    OutOfRange(&'static str),
    /// The source was told to fail its next frame, via
    /// [`FakeSpectrumSource::fail_next_frame`].
    Injected,
}

impl std::fmt::Display for FakeSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FakeSourceError::UnknownKey(k) => write!(f, "unknown setting key: {k}"),
            FakeSourceError::ReadOnly(k) => write!(f, "setting is read-only: {k}"),
            FakeSourceError::WrongKind(k) => write!(f, "wrong value kind for setting: {k}"),
            FakeSourceError::OutOfRange(k) => write!(f, "value out of declared range: {k}"),
            FakeSourceError::Injected => write!(f, "injected failure"),
        }
    }
}

impl std::error::Error for FakeSourceError {}

/// A deterministic spectrum source for tests.
#[derive(Debug, Clone)]
pub struct FakeSpectrumSource {
    center_hz: u64,
    span_hz: u32,
    bin_count: usize,
    sequence: u64,
    noise_floor_dbm: f32,
    peak_dbm: f32,
    /// Peak position as a fraction of the span, 0.0 = lowest frequency.
    peak_fraction: f64,
    capability: SignalCapability,
    fail_next: bool,
}

impl Default for FakeSpectrumSource {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeSpectrumSource {
    /// A source centred on 14.074 MHz with a 48 kHz span and 512 bins,
    /// carrying one peak above the centre frequency.
    ///
    /// The peak sits **above** centre deliberately: a mirrored renderer
    /// draws it below, so the default fixture catches an inversion bug
    /// without the test having to arrange anything.
    pub fn new() -> Self {
        Self {
            center_hz: 14_074_000,
            span_hz: 48_000,
            bin_count: 512,
            sequence: 0,
            noise_floor_dbm: -110.0,
            peak_dbm: -40.0,
            peak_fraction: 0.75,
            capability: SignalCapability::DirectSdr {
                tunable_range_hz: (24_000_000, 1_766_000_000),
            },
            fail_next: false,
        }
    }

    /// Present as a specific capability — an IF tap, a native scope, or
    /// [`SignalCapability::None`] for testing the absent-source path.
    pub fn with_capability(mut self, capability: SignalCapability) -> Self {
        self.capability = capability;
        self
    }

    pub fn with_center_hz(mut self, center_hz: u64) -> Self {
        self.center_hz = center_hz;
        self
    }

    pub fn with_span_hz(mut self, span_hz: u32) -> Self {
        self.span_hz = span_hz;
        self
    }

    pub fn with_bins(mut self, bin_count: usize) -> Self {
        self.bin_count = bin_count;
        self
    }

    /// Place the peak at `fraction` of the span, 0.0 being the lowest
    /// frequency. Clamped to the span.
    pub fn with_peak_at(mut self, fraction: f64) -> Self {
        self.peak_fraction = fraction.clamp(0.0, 1.0);
        self
    }

    /// Make the next [`next_frame`](SpectrumSource::next_frame) fail, so a
    /// consumer's error path can be exercised without hardware.
    pub fn fail_next_frame(&mut self) {
        self.fail_next = true;
    }

    /// The bin index the peak should land in.
    ///
    /// A renderer or transport that reverses the bins will disagree with
    /// this, which is the point.
    pub fn peak_bin(&self) -> usize {
        let idx = (self.peak_fraction * (self.bin_count.saturating_sub(1)) as f64).round() as usize;
        idx.min(self.bin_count.saturating_sub(1))
    }

    /// How many frames have been emitted.
    pub fn frames_emitted(&self) -> u64 {
        self.sequence
    }
}

#[async_trait(?Send)]
impl SpectrumSource for FakeSpectrumSource {
    type Error = FakeSourceError;

    async fn next_frame(&mut self) -> Result<SpectrumFrame, Self::Error> {
        if self.fail_next {
            self.fail_next = false;
            return Err(FakeSourceError::Injected);
        }

        let peak = self.peak_bin();
        let bins = (0..self.bin_count)
            .map(|i| {
                // A narrow triangular peak over a flat floor. Shape is
                // unimportant; being deterministic and asymmetric about
                // the centre bin is not.
                let distance = i.abs_diff(peak);
                if distance <= 3 {
                    self.peak_dbm - (distance as f32 * 8.0)
                } else {
                    self.noise_floor_dbm
                }
            })
            .collect();

        self.sequence += 1;

        Ok(SpectrumFrame {
            center_hz: self.center_hz,
            span_hz: self.span_hz,
            ref_level_dbm: -20.0,
            bins,
            sequence: self.sequence,
        })
    }

    fn capability(&self) -> SignalCapability {
        self.capability
    }

    fn settings(&self) -> SpectrumSettings {
        SpectrumSettings::new(vec![
            SettingDescriptor {
                key: "span_hz",
                label: "Span",
                group: SettingGroup::Display,
                access: Access::ReadWrite,
                value: SettingValue::Int {
                    value: i64::from(self.span_hz),
                    min: 1_000,
                    max: 2_400_000,
                    step: 1_000,
                    unit: Unit::Hz,
                },
            },
            SettingDescriptor {
                key: "bin_count",
                label: "FFT size",
                group: SettingGroup::Display,
                access: Access::ReadWrite,
                value: SettingValue::Enum {
                    value: 2,
                    options: &["128", "256", "512", "1024", "2048"],
                },
            },
            SettingDescriptor {
                key: "center_hz",
                label: "Centre",
                group: SettingGroup::Source,
                access: Access::ReadOnly,
                value: SettingValue::Int {
                    value: self.center_hz as i64,
                    min: 0,
                    max: i64::MAX,
                    step: 1,
                    unit: Unit::Hz,
                },
            },
        ])
    }

    fn apply(&mut self, key: &str, value: SettingValue) -> Result<(), Self::Error> {
        let descriptor = self
            .settings()
            .find(key)
            .cloned()
            .ok_or_else(|| FakeSourceError::UnknownKey(key.to_string()))?;

        if descriptor.access == Access::ReadOnly {
            return Err(FakeSourceError::ReadOnly(descriptor.key));
        }
        if !descriptor.value.same_kind_as(&value) {
            return Err(FakeSourceError::WrongKind(descriptor.key));
        }
        if !value.is_valid() {
            return Err(FakeSourceError::OutOfRange(descriptor.key));
        }

        match (descriptor.key, &value) {
            ("span_hz", SettingValue::Int { value: v, .. }) => self.span_hz = *v as u32,
            ("bin_count", SettingValue::Enum { value: v, .. }) => {
                self.bin_count = 128usize << *v as usize;
            }
            _ => return Err(FakeSourceError::UnknownKey(key.to_string())),
        }
        Ok(())
    }

    fn retune(&mut self, dial_hz: u64) {
        // Note what a real IF-tap source does NOT do here: touch the SDR.
        // The dongle stays parked on the IF; only the reported centre moves.
        self.center_hz = dial_hz;
    }
}
