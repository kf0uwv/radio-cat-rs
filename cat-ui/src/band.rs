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

//! Amateur band plans.
//!
//! Shared because the bands are a property of the spectrum, not of a
//! radio: 20 m is 14.000-14.350 MHz whether a TS-570D or an FT-991A is
//! listening to it. What differs per radio is which of them it can reach,
//! and that is answered by `RadioCapabilities::rx_range`, not here.

use cat_framework::capabilities::{FrequencyRange, RadioCapabilities};

/// One amateur band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    /// Conventional name, as an operator says it: "20m", "2m", "70cm".
    pub label: &'static str,
    pub range: FrequencyRange,
}

impl Band {
    pub fn contains(&self, hz: u64) -> bool {
        self.range.contains(hz)
    }

    /// A sensible frequency to land on when selecting this band: the
    /// bottom of the band plus a nudge, rather than the exact edge.
    pub fn default_hz(&self) -> u64 {
        let span = self.range.max_hz - self.range.min_hz;
        self.range.min_hz + span / 4
    }
}

/// IARU Region 2 amateur allocations, HF through UHF.
///
/// Region 2 because that is where this workspace's radios are. A
/// region-aware plan is a later concern and would be a different
/// `BandPlan`, not a field on `Band`.
pub const BANDS: &[Band] = &[
    band("160m", 1_800_000, 2_000_000),
    band("80m", 3_500_000, 4_000_000),
    band("60m", 5_330_500, 5_406_400),
    band("40m", 7_000_000, 7_300_000),
    band("30m", 10_100_000, 10_150_000),
    band("20m", 14_000_000, 14_350_000),
    band("17m", 18_068_000, 18_168_000),
    band("15m", 21_000_000, 21_450_000),
    band("12m", 24_890_000, 24_990_000),
    band("10m", 28_000_000, 29_700_000),
    band("6m", 50_000_000, 54_000_000),
    band("2m", 144_000_000, 148_000_000),
    band("70cm", 420_000_000, 450_000_000),
];

const fn band(label: &'static str, min_hz: u64, max_hz: u64) -> Band {
    Band {
        label,
        range: FrequencyRange { min_hz, max_hz },
    }
}

/// The bands a particular radio can actually reach.
#[derive(Debug, Clone, Copy)]
pub struct BandPlan {
    coverage: FrequencyRange,
}

impl BandPlan {
    /// Build a plan limited to what `capabilities` says the radio covers.
    ///
    /// This is what stops a TS-570D console offering a 2 m button. The
    /// grid is drawn from data, not from a per-radio hardcoded list, which
    /// is the difference between a shared widget and a radio-specific one.
    pub fn for_radio(capabilities: &RadioCapabilities) -> Self {
        Self {
            coverage: capabilities.rx_range,
        }
    }

    pub fn from_coverage(coverage: FrequencyRange) -> Self {
        Self { coverage }
    }

    /// Bands this radio can reach, in ascending frequency order.
    ///
    /// A band counts as reachable if any part of it is within coverage —
    /// a radio that reaches 14.000-14.100 MHz can still work 20 m.
    pub fn bands(&self) -> impl Iterator<Item = &'static Band> + '_ {
        BANDS.iter().filter(|b| {
            b.range.min_hz <= self.coverage.max_hz && b.range.max_hz >= self.coverage.min_hz
        })
    }

    /// The band containing `hz`, if any. Out-of-band frequencies are a
    /// normal state (general coverage receive), not an error.
    pub fn band_for(&self, hz: u64) -> Option<&'static Band> {
        BANDS.iter().find(|b| b.contains(hz))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coverage(min_hz: u64, max_hz: u64) -> BandPlan {
        BandPlan::from_coverage(FrequencyRange::new(min_hz, max_hz))
    }

    #[test]
    fn an_hf_radio_is_not_offered_vhf_bands() {
        // The TS-570D's coverage. Offering it a 2m button would be a
        // button that cannot work.
        let plan = coverage(500_000, 60_000_000);
        let labels: Vec<_> = plan.bands().map(|b| b.label).collect();
        assert!(labels.contains(&"20m"));
        assert!(labels.contains(&"6m"));
        assert!(!labels.contains(&"2m"));
        assert!(!labels.contains(&"70cm"));
    }

    #[test]
    fn a_vhf_uhf_radio_is_offered_them() {
        // The FT-991A's coverage.
        let plan = coverage(30_000, 470_000_000);
        let labels: Vec<_> = plan.bands().map(|b| b.label).collect();
        assert!(labels.contains(&"160m"));
        assert!(labels.contains(&"2m"));
        assert!(labels.contains(&"70cm"));
    }

    #[test]
    fn partial_coverage_of_a_band_still_offers_it() {
        // A radio reaching only the bottom of 20m can still work 20m.
        let plan = coverage(14_000_000, 14_100_000);
        let labels: Vec<_> = plan.bands().map(|b| b.label).collect();
        assert_eq!(labels, vec!["20m"]);
    }

    #[test]
    fn a_frequency_maps_back_to_its_band() {
        let plan = coverage(500_000, 60_000_000);
        assert_eq!(plan.band_for(14_074_000).unwrap().label, "20m");
        assert_eq!(plan.band_for(7_100_000).unwrap().label, "40m");
    }

    #[test]
    fn out_of_band_frequencies_are_a_normal_state_not_an_error() {
        // General-coverage receive: WWV at 10 MHz is between 30m and 20m
        // and belongs to neither. It is also exactly where the trim
        // calibration is measured, so this path is used in anger.
        let plan = coverage(500_000, 60_000_000);
        assert!(plan.band_for(10_000_000).is_none());
        assert!(plan.band_for(0).is_none());
    }

    #[test]
    fn bands_are_offered_in_ascending_frequency_order() {
        let plan = coverage(30_000, 470_000_000);
        let mins: Vec<u64> = plan.bands().map(|b| b.range.min_hz).collect();
        let mut sorted = mins.clone();
        sorted.sort_unstable();
        assert_eq!(mins, sorted);
    }

    #[test]
    fn a_bands_default_frequency_is_inside_it_and_off_the_edge() {
        for band in BANDS {
            let hz = band.default_hz();
            assert!(band.contains(hz), "{} default outside itself", band.label);
            assert!(
                hz > band.range.min_hz,
                "{} default sits on the edge",
                band.label
            );
        }
    }
}
