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

//! Opening an IF source, whatever it turns out to be.
//!
//! # An application should not have to know how a dongle works
//!
//! Everything an RTL-SDR needs decided — which sample rates the silicon can
//! be set to, how a local device is named, how many bins to take, how to
//! assemble a device or a socket into a corrected [`SpectrumSource`] — is a
//! fact about the dongle, and none of it is a fact about any radio. A
//! consuming application was carrying all of it, which meant every
//! application would have carried its own copy and they would have drifted;
//! it also meant an application choosing a sample rate its hardware cannot
//! produce, which is exactly what happened (see ADR 0014's 2026-09-02
//! amendment).
//!
//! So the whole of it lives here, and an application supplies the one thing
//! only it knows: **which intermediate frequency its radio's IF output sits
//! on, and what that IF needs corrected**. That is [`IfTapConfig`], and it
//! is a radio fact through and through — a TS-570D's 73.05 MHz high-side
//! LO1 means nothing to this crate beyond a number to pin the tuner to.
//!
//! ```ignore
//! // The whole of an application's involvement.
//! let source = cat_signal_rtlsdr::open(
//!     "rtl:0",                       // or "192.168.1.20:1234"
//!     IfTapConfig { if_center_hz: 73_050_000, inverted: true, trim_hz: 0 },
//!     IfSourceConfig::default(),
//! )?;
//! ```

use cat_signal::{
    IfTapConfig, SettingValue, SignalCapability, SpectrumFrame, SpectrumSettings, SpectrumSource,
};

use crate::{is_valid_sample_rate, rtl_tcp::RtlTcpSource, RtlSdrSource, SAMPLE_RATE_BANDS};

/// How a local dongle is named: `rtl:<index>`.
///
/// An RTL-SDR has no filesystem name on any platform — libusb claims it, so
/// neither the tty layer nor the sound layer ever sees it — and librtlsdr
/// addresses it by index. The same string is therefore correct on Linux and
/// Windows alike, which is not true of any other device this workspace
/// touches.
pub const DEVICE_SPEC_PREFIX: &str = "rtl:";

/// Where an IF source lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfEndpoint<'a> {
    /// A dongle on this machine, `rtl:<index>`.
    Device(&'a str),
    /// An `rtl_tcp` server, `host:port`.
    Network(&'a str),
}

impl<'a> IfEndpoint<'a> {
    /// Classify a spec. Never fails, and never resolves DNS.
    ///
    /// The scheme is checked first because `rtl:0` is a host called `rtl`
    /// on port 0 by shape and is not one.
    pub fn parse(spec: &'a str) -> Self {
        match spec.strip_prefix(DEVICE_SPEC_PREFIX) {
            Some(_) => IfEndpoint::Device(spec),
            None => IfEndpoint::Network(spec),
        }
    }

    pub fn is_device(&self) -> bool {
        matches!(self, IfEndpoint::Device(_))
    }
}

/// The spec that names dongle `index`.
pub fn device_spec(index: u32) -> String {
    format!("{DEVICE_SPEC_PREFIX}{index}")
}

/// How much spectrum to take, and how finely.
///
/// Defaults chosen here rather than by a caller, because both are dongle
/// facts: the rate has to be one the silicon can be set to, and the bin
/// count only means anything relative to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IfSourceConfig {
    pub sample_rate_hz: u32,
    pub fft_size: usize,
}

impl IfSourceConfig {
    /// The rate [`IfSourceConfig::default`] uses, as a `const`.
    ///
    /// Exposed separately so a *fixture* can serve the same rate a real
    /// dongle would run at without constructing anything. `ts570d`'s
    /// emulator uses it for exactly that: the tap and the hardware it
    /// impersonates now cannot disagree, which is the whole lesson of ADR
    /// 0014's 2026-09-02 amendment.
    pub const DEFAULT_SAMPLE_RATE_HZ: u32 = 240_000;
    /// The bin count [`IfSourceConfig::default`] uses.
    pub const DEFAULT_FFT_SIZE: usize = 2048;
}

impl Default for IfSourceConfig {
    /// 240 kHz and 2048 bins — about 117 Hz per bin.
    ///
    /// 240 kHz is the bottom of the RTL2832U's lower settable band. It is
    /// the narrowest window the hardware can actually give, which is what
    /// an IF tap wants: the interesting signal is around the dial, and
    /// every extra hertz of span is resolution spent on band a console will
    /// decimate away anyway.
    ///
    /// It is emphatically **not** the narrowest window one might *wish*
    /// for. This was 96 kHz in a consuming application for months, and no
    /// RTL2832U can be set to that — [`open`] now refuses it here rather
    /// than letting a caller discover it from a real dongle.
    fn default() -> Self {
        Self {
            sample_rate_hz: Self::DEFAULT_SAMPLE_RATE_HZ,
            fft_size: Self::DEFAULT_FFT_SIZE,
        }
    }
}

/// Why an IF source could not be opened.
#[derive(Debug)]
pub enum OpenError {
    /// The requested rate is not one the hardware can be set to.
    SampleRate(u32),
    /// `rtl:<index>` where the index was not a number.
    BadDeviceSpec(String),
    /// This build has no librtlsdr in it.
    NoDeviceSupport(String),
    /// Opening the local dongle failed.
    Device(String),
    /// Reaching the `rtl_tcp` server failed.
    Network(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::SampleRate(hz) => write!(
                f,
                "sample rate {hz} Hz is not one an RTL2832U can be set to; it accepts \
                 {}-{} Hz and {}-{} Hz and nothing between or below",
                SAMPLE_RATE_BANDS[0].0,
                SAMPLE_RATE_BANDS[0].1,
                SAMPLE_RATE_BANDS[1].0,
                SAMPLE_RATE_BANDS[1].1
            ),
            OpenError::BadDeviceSpec(spec) => write!(
                f,
                "{spec:?}: an RTL-SDR is addressed by index -- {DEVICE_SPEC_PREFIX}0, \
                 {DEVICE_SPEC_PREFIX}1, ..."
            ),
            OpenError::NoDeviceSupport(spec) => write!(
                f,
                "this build cannot open the local dongle {spec:?}: it was compiled without \
                 the `device` feature (which needs librtlsdr). Point at an rtl_tcp server \
                 instead."
            ),
            OpenError::Device(why) => write!(f, "{why}"),
            OpenError::Network(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for OpenError {}

/// An IF source, however it was reached.
///
/// One type rather than a generic, so a caller holds the same thing whether
/// the dongle is on this machine or across the shack. Both arms are the
/// same pipeline over a different [`crate::IqSource`]; the correction, the
/// FFT and the low-frequency-first guarantee are identical because they are
/// literally the same code.
pub enum IfSource {
    Network(RtlSdrSource<RtlTcpSource>),
    #[cfg(feature = "device")]
    Device(RtlSdrSource<crate::device::RtlSdrDevice>),
}

/// Do the same thing to whichever arm this is.
macro_rules! either {
    ($self:expr, $source:ident => $body:expr) => {
        match $self {
            IfSource::Network($source) => $body,
            #[cfg(feature = "device")]
            IfSource::Device($source) => $body,
        }
    };
}

#[async_trait::async_trait(?Send)]
impl SpectrumSource for IfSource {
    /// Flattened to a string.
    ///
    /// The two arms fail for different reasons — a socket and a USB device
    /// have nothing in common to report — and a caller that had to name
    /// both would be back to knowing which one it had. What a console does
    /// with either is show it to an operator.
    type Error = String;

    async fn next_frame(&mut self) -> Result<SpectrumFrame, Self::Error> {
        either!(self, s => s.next_frame().await.map_err(|e| e.to_string()))
    }

    fn capability(&self) -> SignalCapability {
        either!(self, s => s.capability())
    }

    fn settings(&self) -> SpectrumSettings {
        either!(self, s => s.settings())
    }

    fn apply(&mut self, key: &str, value: SettingValue) -> Result<(), Self::Error> {
        either!(self, s => s.apply(key, value).map_err(|e| e.to_string()))
    }

    fn retune(&mut self, dial_hz: u64) {
        either!(self, s => s.retune(dial_hz))
    }
}

/// Open the IF source named by `spec`, pinned to `tap`'s intermediate
/// frequency.
///
/// `tap` is the caller's whole contribution: the IF its radio's output sits
/// on, whether that IF arrives mirrored, and the station's crystal trim.
/// Everything else — the rate, the bin count, how a device is named, how
/// the pieces assemble — belongs to the dongle and is decided here.
///
/// **The tuner is pinned to `tap.if_center_hz` and never moved.** An IF tap
/// is dial-centred by the *radio's* local oscillator, not by retuning the
/// SDR; `retune` moves the window the frames report and leaves the hardware
/// where it is. See ADR 0014 §6.
pub fn open(spec: &str, tap: IfTapConfig, config: IfSourceConfig) -> Result<IfSource, OpenError> {
    if !is_valid_sample_rate(config.sample_rate_hz) {
        return Err(OpenError::SampleRate(config.sample_rate_hz));
    }

    match IfEndpoint::parse(spec) {
        IfEndpoint::Network(addr) => {
            let iq = RtlTcpSource::connect(addr)
                .map_err(|e| OpenError::Network(format!("no rtl_tcp server at {addr}: {e}")))?;
            Ok(IfSource::Network(RtlSdrSource::new(
                iq,
                config.sample_rate_hz,
                config.fft_size,
                tap,
            )))
        }
        IfEndpoint::Device(spec) => open_device(spec, tap, config),
    }
}

#[cfg(feature = "device")]
fn open_device(
    spec: &str,
    tap: IfTapConfig,
    config: IfSourceConfig,
) -> Result<IfSource, OpenError> {
    let index: u32 = spec
        .trim_start_matches(DEVICE_SPEC_PREFIX)
        .parse()
        .map_err(|_| OpenError::BadDeviceSpec(spec.to_string()))?;

    // Pinned to the IF, once, and never moved again.
    let device = crate::device::RtlSdrDevice::open(index, tap.if_center_hz, config.sample_rate_hz)
        .map_err(|e| OpenError::Device(format!("could not open {spec}: {e}")))?;

    Ok(IfSource::Device(RtlSdrSource::new(
        device,
        config.sample_rate_hz,
        config.fft_size,
        tap,
    )))
}

#[cfg(not(feature = "device"))]
fn open_device(
    spec: &str,
    _tap: IfTapConfig,
    _config: IfSourceConfig,
) -> Result<IfSource, OpenError> {
    Err(OpenError::NoDeviceSupport(spec.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Result::unwrap_err` wants `T: Debug`, and `IfSource` deliberately
    /// is not: one arm holds a live device handle whose `Debug` would be a
    /// pointer nobody can use.
    fn expect_err(result: Result<IfSource, OpenError>) -> OpenError {
        match result {
            Ok(_) => panic!("expected an error, and something opened"),
            Err(e) => e,
        }
    }

    const TAP: IfTapConfig = IfTapConfig {
        if_center_hz: 73_050_000,
        inverted: true,
        trim_hz: 0,
    };

    #[test]
    fn a_dongle_is_named_by_index_and_anything_else_is_an_address() {
        assert_eq!(IfEndpoint::parse("rtl:0"), IfEndpoint::Device("rtl:0"));
        assert_eq!(IfEndpoint::parse("rtl:12"), IfEndpoint::Device("rtl:12"));
        assert!(IfEndpoint::parse("rtl:0").is_device());

        assert_eq!(
            IfEndpoint::parse("127.0.0.1:1234"),
            IfEndpoint::Network("127.0.0.1:1234")
        );
        assert!(!IfEndpoint::parse("radio.local:1234").is_device());
    }

    #[test]
    fn device_spec_is_the_inverse_of_parse() {
        for index in [0u32, 1, 12] {
            let spec = device_spec(index);
            assert!(IfEndpoint::parse(&spec).is_device(), "{spec}");
            assert_eq!(
                spec.trim_start_matches(DEVICE_SPEC_PREFIX)
                    .parse::<u32>()
                    .unwrap(),
                index
            );
        }
    }

    #[test]
    fn the_default_rate_is_one_the_hardware_can_be_set_to() {
        // The bug this whole module was written to make impossible for a
        // caller to reintroduce.
        assert!(is_valid_sample_rate(
            IfSourceConfig::default().sample_rate_hz
        ));
    }

    #[test]
    fn an_impossible_rate_is_refused_before_anything_is_opened() {
        // Refused here, with the bands named, rather than by a dongle with
        // `errno -22` after a caller has shipped.
        let config = IfSourceConfig {
            sample_rate_hz: 96_000,
            ..IfSourceConfig::default()
        };
        let err = expect_err(open("127.0.0.1:1", TAP, config));
        assert!(matches!(err, OpenError::SampleRate(96_000)));
        let text = err.to_string();
        assert!(text.contains("96000") && text.contains("225001"), "{text}");
    }

    #[test]
    fn the_rate_is_checked_before_the_endpoint_is_even_looked_at() {
        // `127.0.0.1:1` has nothing listening. A rate error must not depend
        // on whether the network happens to answer first.
        let config = IfSourceConfig {
            sample_rate_hz: 1,
            ..IfSourceConfig::default()
        };
        assert!(matches!(
            expect_err(open("127.0.0.1:1", TAP, config)),
            OpenError::SampleRate(1)
        ));
    }

    #[test]
    fn a_dongle_index_that_is_not_a_number_says_so() {
        let err = expect_err(open("rtl:frobnicate", TAP, IfSourceConfig::default()));
        // Without the feature this is refused earlier, for a different and
        // equally true reason; either message names the spec.
        let text = err.to_string();
        assert!(text.contains("frobnicate"), "{text}");
    }

    #[cfg(not(feature = "device"))]
    #[test]
    fn a_build_without_the_driver_says_what_to_do_instead() {
        let err = expect_err(open("rtl:0", TAP, IfSourceConfig::default()));
        assert!(matches!(err, OpenError::NoDeviceSupport(_)));
        assert!(err.to_string().contains("rtl_tcp"), "{err}");
    }

    #[test]
    fn an_unreachable_server_is_a_network_error_and_not_a_panic() {
        let err = expect_err(open("127.0.0.1:1", TAP, IfSourceConfig::default()));
        assert!(matches!(err, OpenError::Network(_)), "{err}");
    }
}
