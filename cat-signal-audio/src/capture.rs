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

//! Capture from a local sound card. Behind the `device` feature.
//!
//! The second [`AudioSource`] ADR 0017 §1 predicted, and the reason the
//! trait was left in this crate rather than promoted into `cat-signal`.
//! Where [`AudioStream::connect`] takes the receive audio off a socket,
//! [`AudioCapture::open`] takes it off the sound card an ACC2 lead is
//! plugged into.
//!
//! # It is the same pipeline, reached through the seam that already existed
//!
//! `cpal` hands audio to a callback on a real-time thread; the reader
//! thread wants a [`Read`](std::io::Read). [`PcmRing`](crate::ring::PcmRing)
//! is the whole of the adaptation, so the DSP, the newest-wins frame slot,
//! the terminal-close rule and the no-cross-thread-wake property are the
//! *same code* the socket path uses and the same code the wire tests
//! already cover. Nothing in `stream.rs` changed to make this work.
//!
//! # What happens to a device that is not 48 kHz mono
//!
//! Real sound cards are not. The rules, in full:
//!
//! - **Sample rate.** 48 kHz is requested; if the device does not offer it,
//!   the device's own default rate is used and **the pipeline is told**.
//!   There is no resampler (ADR 0017 scopes resampling out and this does
//!   not change that). Nothing is misleading because every frame reports
//!   the rate it was actually sampled at, and `window_ms`,
//!   `bin_width_hz` and `max_bandwidth_hz` all derive from that number
//!   rather than from an assumed 48 000.
//! - **Channels.** The device's own channel count is requested — the
//!   driver is never asked to convert — and **one channel is taken**,
//!   channel 0 unless [`CaptureConfig::channel`] says otherwise. Not an
//!   average: averaging a rig feed wired to one input halves the level by
//!   6 dB and mixes the other channel's noise into the spectrum, both
//!   invisibly. Taking the wrong channel gives silence, which an operator
//!   can see and fix. `channels` and `channel` are read-only settings so a
//!   console can show "2 ch, using ch 0".
//! - **Sample format.** Any format `cpal` offers is accepted and converted
//!   to normalized `f32`, then quantized to 16 bits by the ring. On a
//!   24-bit or f32 card that is a −96 dBFS quantization floor, which is
//!   some 40 dB below any receiver's audio noise floor, and it buys one
//!   pipeline and one set of numbers for both kinds of source.
//!
//! Refusing a 44.1 kHz card was the alternative, and it is worse: the card
//! works perfectly well, and a console that reports the rate it is
//! actually running at tells the operator no lies.

use std::sync::Arc;

use async_trait::async_trait;
use cat_signal::{
    Access, DeviceInfo, DeviceKind, DeviceList, SettingDescriptor, SettingGroup, SettingValue,
    SignalCapability, SpectrumSettings, Unit,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SampleRate, SizedSample, SupportedStreamConfig};

use crate::device::{device_spec, AudioEndpoint};
use crate::dsp::AudioPipelineConfig;
use crate::ring::PcmRing;
use crate::stream::AudioStream;
use crate::{AudioError, AudioFrame, AudioSource};

/// What can go wrong opening a sound card.
///
/// Separate from [`AudioError`] because these all happen *before* there is
/// a stream: once capture is running, every failure is an
/// [`AudioError::Closed`] like any other source's, and a console's
/// running-state handling does not need a second shape.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// The spec was a network endpoint, not `audio:<name>`.
    #[error("{0:?} is not a sound-card spec: a device spec looks like `audio:<name>`")]
    NotADeviceSpec(String),
    /// The host could not be asked what it has.
    #[error("could not enumerate audio input devices: {0}")]
    Enumerate(String),
    /// Enumeration worked; nothing on this machine has that name.
    #[error("no audio input device named {name:?}{alternatives}")]
    NotFound {
        name: String,
        /// The names that *do* exist, so the message is actionable rather
        /// than merely correct.
        alternatives: String,
    },
    /// The device is there but will not say what it can do.
    #[error("audio device {name:?} would not report a usable input format: {why}")]
    Unsupported { name: String, why: String },
    /// A channel was asked for that the device does not have.
    #[error("audio device {name:?} has {channels} channel(s); channel {channel} was asked for")]
    NoSuchChannel {
        name: String,
        channels: u16,
        channel: usize,
    },
    /// The stream could not be created or started.
    #[error("could not start capture from {name:?}: {why}")]
    Start { name: String, why: String },
}

/// How a capture is set up.
///
/// `Default` is 48 kHz-preferring, channel 0, 200 ms of ring — the sensible
/// thing for a rig feed on a USB codec, and every field is there because
/// some station will need to change it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureConfig {
    /// The DSP configuration. Its `sample_rate_hz` is the rate that will be
    /// *asked for*; the negotiated rate is written back into the pipeline
    /// and reported by [`AudioCapture::format`].
    pub pipeline: AudioPipelineConfig,
    /// Which channel of a multi-channel device carries the radio.
    pub channel: usize,
    /// How much audio the ring holds between the capture callback and the
    /// reader thread.
    ///
    /// This is not display latency — the frame slot bounds that to one
    /// block — it is only headroom against a scheduling hiccup. Bigger
    /// hides more jitter; smaller means an overrun is counted sooner. The
    /// ring is never allowed to be smaller than four FFT blocks, since a
    /// ring that cannot hold one block would overrun on every push.
    pub buffer_ms: u32,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            pipeline: AudioPipelineConfig::default(),
            channel: 0,
            buffer_ms: 200,
        }
    }
}

/// What the device actually gave us, as opposed to what was asked for.
///
/// A console should show this: it is the difference between "the radio is
/// quiet" and "you are listening to the other channel of a 44.1 kHz card".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureFormat {
    /// The rate the device is running at, which is also the rate the
    /// pipeline and every frame report.
    pub sample_rate_hz: u32,
    /// How many channels the device delivers.
    pub channels: u16,
    /// Which of them is being used.
    pub channel: usize,
    /// The device's sample format, as the driver names it.
    pub sample_format: String,
    /// Whether the requested rate had to be given up on.
    ///
    /// True means "this is the device's rate, not yours" — the one thing
    /// about a capture that most deserves a line in a status bar.
    pub rate_substituted: bool,
}

/// A sound-card input, as a source of [`AudioFrame`]s.
///
/// The same frames, the same backpressure and the same terminal-close rule
/// as [`AudioStream`]; only where the samples come from is different. It
/// implements [`AudioSource`], so code that is generic over a source needs
/// no branch.
pub struct AudioCapture {
    /// Kept only to hold the device open: `cpal` runs the callback from
    /// its own thread and this handle is what keeps that thread alive.
    /// Declared **first**, so it is dropped first and the producer stops
    /// before the consumer it feeds.
    _device_stream: cpal::Stream,
    stream: AudioStream,
    ring: Arc<PcmRing>,
    spec: String,
    label: String,
    format: CaptureFormat,
}

impl AudioCapture {
    /// Open the device a `--acc2-audio` spec names.
    ///
    /// `spec` must be `audio:<name>` (or `audio:` for the host default) —
    /// exactly the string [`input_devices`](crate::input_devices) puts in
    /// [`DeviceInfo::spec`]. A network endpoint is rejected here rather
    /// than quietly treated as a device name, because a console that
    /// mixed the two up would report "no such device: 127.0.0.1:4533".
    pub fn open(spec: &str, config: CaptureConfig) -> Result<Self, CaptureError> {
        let name = match AudioEndpoint::parse(spec) {
            AudioEndpoint::Device(name) => name,
            AudioEndpoint::Network(_) => {
                return Err(CaptureError::NotADeviceSpec(spec.to_string()))
            }
        };
        Self::open_named(name, config)
    }

    /// Open whatever the host considers its default input.
    ///
    /// The same as `open("audio:", config)`.
    pub fn open_default(config: CaptureConfig) -> Result<Self, CaptureError> {
        Self::open_named("", config)
    }

    fn open_named(name: &str, config: CaptureConfig) -> Result<Self, CaptureError> {
        let host = cpal::default_host();

        let device = if name.is_empty() {
            host.default_input_device().ok_or_else(|| {
                // Not `NotFound`: nothing was named, so there is no name to
                // report back and no list of alternatives that would help.
                CaptureError::NotFound {
                    name: String::new(),
                    alternatives: ": this host has no default input device".to_string(),
                }
            })?
        } else {
            find_by_name(&host, name)?
        };

        let label = device.name().unwrap_or_else(|_| name.to_string());
        let chosen = negotiate(&device, config.pipeline.sample_rate_hz).map_err(|why| {
            CaptureError::Unsupported {
                name: label.clone(),
                why,
            }
        })?;

        let channels = chosen.config.channels();
        if config.channel >= usize::from(channels) {
            return Err(CaptureError::NoSuchChannel {
                name: label,
                channels,
                channel: config.channel,
            });
        }

        let rate = chosen.config.sample_rate().0;
        let mut pipeline = config.pipeline;
        // The negotiated rate, not the requested one. Every frame's
        // `sample_rate_hz`, `window_ms`, `bin_width_hz` and the source's
        // `max_bandwidth_hz` come from here, so a 44.1 kHz card is
        // reported as a 44.1 kHz card rather than drawn as though it were
        // 48 kHz -- which would put every frequency 8.8% out.
        pipeline.sample_rate_hz = rate;
        // A span wider than this device's Nyquist would ask the pipeline
        // for bins that do not exist; it clamps, but clamping silently is
        // worse than starting from something true.
        pipeline.span_hz = pipeline.span_hz.min(rate / 2);

        let capacity = ring_capacity(rate, config.buffer_ms, pipeline.fft_size);
        let ring = PcmRing::new(capacity);
        let stream = AudioStream::from_reader(ring.reader(), pipeline);

        let device_stream =
            build_stream(&device, &chosen.config, config.channel, &ring).map_err(|why| {
                CaptureError::Start {
                    name: label.clone(),
                    why,
                }
            })?;
        device_stream.play().map_err(|e| CaptureError::Start {
            name: label.clone(),
            why: e.to_string(),
        })?;

        Ok(Self {
            _device_stream: device_stream,
            stream,
            ring,
            spec: device_spec(&label),
            label,
            format: CaptureFormat {
                sample_rate_hz: rate,
                channels,
                channel: config.channel,
                sample_format: chosen.config.sample_format().to_string(),
                rate_substituted: chosen.rate_substituted,
            },
        })
    }

    /// What the device is actually delivering.
    pub fn format(&self) -> &CaptureFormat {
        &self.format
    }

    /// The driver's name for this device — what an operator reads.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The spec that would reopen this device, which is also what
    /// `--acc2-audio` takes.
    pub fn spec(&self) -> &str {
        &self.spec
    }

    /// Take the newest frame if one is waiting, without blocking.
    ///
    /// **What a console's render loop should call**, for exactly the
    /// reason [`AudioStream::try_next_frame`] gives.
    pub fn try_next_frame(&mut self) -> Result<Option<AudioFrame>, AudioError> {
        self.stream.try_next_frame()
    }

    /// Block until a frame arrives or the capture ends.
    pub fn blocking_next_frame(&mut self) -> Result<AudioFrame, AudioError> {
        self.stream.blocking_next_frame()
    }

    /// Frames produced but never collected, because the console was behind.
    pub fn frames_dropped(&self) -> u64 {
        self.stream.frames_dropped()
    }

    /// Samples the *capture* side threw away because the ring filled.
    ///
    /// A different fault from [`frames_dropped`](Self::frames_dropped) with
    /// a different fix: frames dropped means the console is slow and the
    /// display is merely at a lower rate; samples dropped means audio was
    /// lost between the sound card and the DSP, which puts a click in the
    /// trace and a smear in the spectrum. It should be zero.
    pub fn samples_dropped(&self) -> u64 {
        self.ring.overruns()
    }

    /// Whether the device is still delivering audio.
    ///
    /// Goes false and stays false when the card is unplugged or the sound
    /// service stops — terminal, exactly as a lost socket is (ADR 0017 §6),
    /// and for the same reason: reconnect policy belongs to the
    /// application, and a source that silently reopened would hide a
    /// station fault behind a gap.
    pub fn is_connected(&self) -> bool {
        self.stream.is_connected()
    }

    /// The pipeline configuration in force, with the *negotiated* rate.
    pub fn config(&self) -> AudioPipelineConfig {
        self.stream.config()
    }

    /// The underlying stream, for code that already speaks [`AudioStream`].
    pub fn stream(&mut self) -> &mut AudioStream {
        &mut self.stream
    }
}

impl std::fmt::Debug for AudioCapture {
    /// Hand-written because `cpal::Stream` is not `Debug`, and because what
    /// is worth printing about a capture is which device it is and what
    /// that device is actually doing.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioCapture")
            .field("spec", &self.spec)
            .field("label", &self.label)
            .field("format", &self.format)
            .field("connected", &self.is_connected())
            .finish()
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        // This runs before any field is dropped, and it is what stops the
        // reader thread: that thread is blocked in `read_exact` on the
        // ring, and nothing else would ever wake it. `cat-transport-rfc2217`
        // learned the same lesson in ADR 0016 -- a worker holding its own
        // `Arc` means dropping the handle frees nothing.
        self.ring.close("capture stopped by the consumer");
    }
}

#[async_trait(?Send)]
impl AudioSource for AudioCapture {
    type Error = AudioError;

    async fn next_frame(&mut self) -> Result<AudioFrame, Self::Error> {
        // Blocks the calling thread on a condvar, registers no waker and
        // wakes no task from another thread -- the property that keeps ADR
        // 0016's monoio `sync`-feature trap out of this crate. `cpal`'s
        // callback thread only ever touches the ring's mutex.
        self.stream.blocking_next_frame()
    }

    fn capability(&self) -> SignalCapability {
        self.stream.capability()
    }

    fn settings(&self) -> SpectrumSettings {
        let mut settings = self.stream.settings();
        // Appended rather than replacing: a console's settings panel shows
        // the same audio knobs for both kinds of source, plus what is only
        // true of a card.
        settings.descriptors.push(SettingDescriptor {
            key: "channels",
            label: "Device channels",
            group: SettingGroup::Source,
            access: Access::ReadOnly,
            value: SettingValue::Int {
                value: i64::from(self.format.channels),
                min: 0,
                max: i64::from(u16::MAX),
                step: 1,
                unit: Unit::None,
            },
        });
        settings.descriptors.push(SettingDescriptor {
            key: "channel",
            label: "Channel in use",
            group: SettingGroup::Source,
            access: Access::ReadOnly,
            value: SettingValue::Int {
                value: self.format.channel as i64,
                min: 0,
                max: i64::from(u16::MAX),
                step: 1,
                unit: Unit::None,
            },
        });
        // Read-only and separate from `frames_dropped` because the two
        // faults have different fixes -- see `samples_dropped`.
        settings.descriptors.push(SettingDescriptor {
            key: "samples_dropped",
            label: "Samples dropped",
            group: SettingGroup::Source,
            access: Access::ReadOnly,
            value: SettingValue::Int {
                value: self.samples_dropped() as i64,
                min: 0,
                max: i64::MAX,
                step: 1,
                unit: Unit::None,
            },
        });
        settings
    }

    fn apply(&mut self, key: &str, value: SettingValue) -> Result<(), Self::Error> {
        // The device-only settings above are all read-only, so anything
        // writable is the pipeline's and the stream already validates it.
        match key {
            "channels" => Err(AudioError::ReadOnly("channels")),
            "channel" => Err(AudioError::ReadOnly("channel")),
            "samples_dropped" => Err(AudioError::ReadOnly("samples_dropped")),
            _ => self.stream.apply(key, value),
        }
    }
}

// ---------------------------------------------------------------------------
// Enumeration
// ---------------------------------------------------------------------------

/// The body of [`crate::input_devices`], when the feature is on.
pub(crate) fn enumerate() -> DeviceList {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|device| device.name().ok());

    let found = match host.input_devices() {
        Ok(devices) => devices,
        // "Cannot ask" -- the sound service is not running, or the driver
        // failed. Deliberately not an empty list: that means "plug
        // something in", and these two send an operator to different
        // places.
        Err(e) => {
            return DeviceList::unavailable(
                DeviceKind::AudioInput,
                format!("could not enumerate audio input devices: {e}"),
            )
        }
    };

    let mut devices: Vec<DeviceInfo> = Vec::new();
    for device in found {
        // A device whose name cannot be read cannot be addressed by a
        // spec either, so listing it would offer a choice that could not
        // be honoured.
        let Ok(name) = device.name() else { continue };
        // See the doc on `input_devices`: a duplicate name is a spec that
        // could not be honoured.
        if devices.iter().any(|d| d.label == name) {
            continue;
        }
        devices.push(DeviceInfo {
            kind: DeviceKind::AudioInput,
            spec: device_spec(&name),
            is_default: default_name.as_deref() == Some(name.as_str()),
            detail: describe(&device),
            label: name,
        });
    }

    DeviceList::found(DeviceKind::AudioInput, devices)
}

/// One line saying what this device will actually give a console.
///
/// Written before the operator picks, because "44.1 kHz" after the fact is
/// a support question and "44.1 kHz" in the picker is a decision.
fn describe(device: &cpal::Device) -> Option<String> {
    let default = device.default_input_config().ok()?;
    let supports_48k = device
        .supported_input_configs()
        .map(|mut ranges| {
            ranges.any(|r| r.min_sample_rate().0 <= 48_000 && r.max_sample_rate().0 >= 48_000)
        })
        .unwrap_or(false);
    let rate = if supports_48k {
        "48 kHz available".to_string()
    } else {
        format!("{} Hz only", default.sample_rate().0)
    };
    Some(format!(
        "{} ch, {} Hz default, {}, {rate}",
        default.channels(),
        default.sample_rate().0,
        default.sample_format(),
    ))
}

fn find_by_name<H: HostTrait>(host: &H, name: &str) -> Result<H::Device, CaptureError> {
    let devices = host
        .input_devices()
        .map_err(|e| CaptureError::Enumerate(e.to_string()))?;

    let mut seen: Vec<String> = Vec::new();
    for device in devices {
        match device.name() {
            Ok(found) if found == name => return Ok(device),
            Ok(found) => seen.push(found),
            Err(_) => {}
        }
    }
    Err(CaptureError::NotFound {
        name: name.to_string(),
        alternatives: if seen.is_empty() {
            String::new()
        } else {
            format!(". This machine has: {}", seen.join(", "))
        },
    })
}

// ---------------------------------------------------------------------------
// Format negotiation
// ---------------------------------------------------------------------------

/// The outcome of asking a device for a rate.
struct Chosen {
    config: SupportedStreamConfig,
    /// True when the device could not do the requested rate and its own is
    /// being used instead.
    rate_substituted: bool,
}

/// Ask `device` for `wanted_hz`, falling back to its own default rate.
///
/// The device's **own** channel count is kept rather than asking the driver
/// to convert: an ALSA plug layer reducing two channels to one may take
/// channel 0 or may mix, and which it does is not something to leave to a
/// layer nobody can see. Extracting a channel here is explicit and
/// reportable.
fn negotiate<D: DeviceTrait>(device: &D, wanted_hz: u32) -> Result<Chosen, String> {
    let default = device
        .default_input_config()
        .map_err(|e| format!("no default input config: {e}"))?;

    let ranges = match device.supported_input_configs() {
        Ok(ranges) => ranges.collect::<Vec<_>>(),
        // A device that will not enumerate its ranges but does have a
        // default config is still usable at that default.
        Err(_) => Vec::new(),
    };

    match pick_range(&ranges, default.channels(), wanted_hz) {
        Some(range) => Ok(Chosen {
            config: range.with_sample_rate(SampleRate(wanted_hz)),
            rate_substituted: false,
        }),
        None => Ok(Chosen {
            rate_substituted: default.sample_rate().0 != wanted_hz,
            config: default,
        }),
    }
}

/// Rank of a sample format, lower being less conversion work.
///
/// Only a tie-break: every format is accepted and converted, so this
/// chooses between otherwise identical offers rather than deciding
/// anything a user could observe.
fn format_rank(format: SampleFormat) -> u8 {
    match format {
        SampleFormat::F32 => 0,
        SampleFormat::I16 => 1,
        SampleFormat::I32 => 2,
        SampleFormat::U16 => 3,
        SampleFormat::I8 => 4,
        SampleFormat::U8 => 5,
        SampleFormat::F64 => 6,
        _ => 9,
    }
}

/// The best offer covering `wanted_hz`, or `None` if there is none.
///
/// Pure, so the rule is testable without a sound card: prefer the device's
/// own channel count, then the fewest channels, then the least conversion.
fn pick_range(
    ranges: &[cpal::SupportedStreamConfigRange],
    preferred_channels: u16,
    wanted_hz: u32,
) -> Option<cpal::SupportedStreamConfigRange> {
    ranges
        .iter()
        .filter(|r| r.min_sample_rate().0 <= wanted_hz && r.max_sample_rate().0 >= wanted_hz)
        .min_by_key(|r| {
            (
                r.channels() != preferred_channels,
                r.channels(),
                format_rank(r.sample_format()),
            )
        })
        .cloned()
}

/// How many samples of headroom the ring gets.
///
/// Never less than four FFT blocks: a ring that cannot hold one block would
/// count an overrun on every push and lose audio on a machine that was
/// keeping up perfectly well.
fn ring_capacity(rate_hz: u32, buffer_ms: u32, fft_size: usize) -> usize {
    let from_time = (rate_hz as usize).saturating_mul(buffer_ms as usize) / 1000;
    from_time.max(fft_size.saturating_mul(4)).max(1)
}

// ---------------------------------------------------------------------------
// The capture callback
// ---------------------------------------------------------------------------

fn build_stream<D: DeviceTrait>(
    device: &D,
    config: &SupportedStreamConfig,
    channel: usize,
    ring: &Arc<PcmRing>,
) -> Result<D::Stream, String> {
    let channels = usize::from(config.channels());
    // `#[non_exhaustive]`, so the catch-all is required rather than
    // optional -- and a format cpal adds later should say so plainly
    // instead of failing to build.
    match config.sample_format() {
        SampleFormat::I8 => typed::<D, i8>(device, config, channels, channel, ring),
        SampleFormat::I16 => typed::<D, i16>(device, config, channels, channel, ring),
        SampleFormat::I32 => typed::<D, i32>(device, config, channels, channel, ring),
        SampleFormat::I64 => typed::<D, i64>(device, config, channels, channel, ring),
        SampleFormat::U8 => typed::<D, u8>(device, config, channels, channel, ring),
        SampleFormat::U16 => typed::<D, u16>(device, config, channels, channel, ring),
        SampleFormat::U32 => typed::<D, u32>(device, config, channels, channel, ring),
        SampleFormat::U64 => typed::<D, u64>(device, config, channels, channel, ring),
        SampleFormat::F32 => typed::<D, f32>(device, config, channels, channel, ring),
        SampleFormat::F64 => typed::<D, f64>(device, config, channels, channel, ring),
        other => Err(format!("unsupported sample format {other}")),
    }
}

fn typed<D: DeviceTrait, T>(
    device: &D,
    config: &SupportedStreamConfig,
    channels: usize,
    channel: usize,
    ring: &Arc<PcmRing>,
) -> Result<D::Stream, String>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let producer = Arc::clone(ring);
    let on_error = Arc::clone(ring);
    // Owned by the callback and reused, so the real-time thread allocates
    // once rather than on every block.
    let mut mono: Vec<f32> = Vec::new();

    device
        .build_input_stream::<T, _, _>(
            &config.config(),
            move |data: &[T], _| {
                mono.clear();
                mono.extend(
                    data.iter()
                        .skip(channel)
                        .step_by(channels)
                        .map(|s| s.to_sample::<f32>()),
                );
                // Never blocks: the ring drops its oldest audio rather
                // than stall a real-time thread that owns the device.
                producer.push(&mono);
            },
            move |e| {
                // The device went away, or the service did. Terminal, and
                // carrying the driver's own words -- "the peer closed the
                // connection" would be a lie about a USB codec.
                on_error.close(format!("audio device stopped: {e}"));
            },
            None,
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cpal::{SupportedBufferSize, SupportedStreamConfigRange};

    fn range(
        channels: u16,
        min: u32,
        max: u32,
        format: SampleFormat,
    ) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(
            channels,
            SampleRate(min),
            SampleRate(max),
            SupportedBufferSize::Unknown,
            format,
        )
    }

    #[test]
    fn forty_eight_kilohertz_is_taken_when_the_device_offers_it() {
        let ranges = [
            range(2, 8_000, 96_000, SampleFormat::I16),
            range(1, 8_000, 96_000, SampleFormat::F32),
        ];
        let picked = pick_range(&ranges, 2, 48_000).expect("48 kHz is in range");
        let config = picked.with_sample_rate(SampleRate(48_000));
        assert_eq!(config.sample_rate().0, 48_000);
        // The device's own channel count, not the driver's conversion.
        assert_eq!(config.channels(), 2);
    }

    #[test]
    fn a_device_that_cannot_do_the_wanted_rate_offers_nothing() {
        // Which is what makes `negotiate` fall back to the device's own
        // default rate rather than asking for one it does not have.
        let ranges = [range(2, 44_100, 44_100, SampleFormat::F32)];
        assert!(pick_range(&ranges, 2, 48_000).is_none());
        assert!(pick_range(&ranges, 2, 44_100).is_some());
    }

    #[test]
    fn the_devices_own_channel_count_beats_a_lower_one() {
        // Asking a driver for fewer channels than the card has hands the
        // 2->1 decision to a plug layer that may take channel 0 or may
        // mix, invisibly either way. We take the channel ourselves.
        let ranges = [
            range(1, 8_000, 96_000, SampleFormat::F32),
            range(2, 8_000, 96_000, SampleFormat::F32),
            range(6, 8_000, 96_000, SampleFormat::F32),
        ];
        assert_eq!(pick_range(&ranges, 2, 48_000).unwrap().channels(), 2);
        assert_eq!(pick_range(&ranges, 6, 48_000).unwrap().channels(), 6);
        // When the preferred count is not on offer, fewest wins.
        assert_eq!(pick_range(&ranges, 4, 48_000).unwrap().channels(), 1);
    }

    #[test]
    fn format_is_only_a_tie_break() {
        let ranges = [
            range(2, 8_000, 96_000, SampleFormat::U8),
            range(2, 8_000, 96_000, SampleFormat::F32),
            range(2, 8_000, 96_000, SampleFormat::I16),
        ];
        assert_eq!(
            pick_range(&ranges, 2, 48_000).unwrap().sample_format(),
            SampleFormat::F32
        );
        // ...and never at the cost of a channel count.
        let mixed = [
            range(1, 8_000, 96_000, SampleFormat::F32),
            range(2, 8_000, 96_000, SampleFormat::U8),
        ];
        assert_eq!(pick_range(&mixed, 2, 48_000).unwrap().channels(), 2);
    }

    #[test]
    fn the_ring_always_holds_at_least_four_blocks() {
        // A ring smaller than one FFT block would count an overrun on
        // every push and lose audio on a machine that was keeping up.
        assert_eq!(ring_capacity(48_000, 200, 1024), 9_600);
        assert_eq!(ring_capacity(48_000, 1, 1024), 4_096);
        assert_eq!(ring_capacity(8_000, 0, 4_096), 16_384);
    }

    #[test]
    fn a_network_endpoint_is_refused_rather_than_looked_up_as_a_device_name() {
        // A console that mixed the two up would tell its operator "no such
        // device: 127.0.0.1:4533", which sends them to the wrong place.
        let error = AudioCapture::open("127.0.0.1:4533", CaptureConfig::default())
            .expect_err("a network endpoint is not a device");
        assert!(matches!(error, CaptureError::NotADeviceSpec(_)));
        assert!(error.to_string().contains("audio:"));
    }

    #[test]
    fn a_name_that_is_not_here_says_what_is() {
        // Actionable, not merely correct: the names that do exist are the
        // whole content of the fix.
        let error = AudioCapture::open("audio:no such card", CaptureConfig::default())
            .expect_err("this machine has no card by that name");
        assert!(
            matches!(error, CaptureError::NotFound { .. }),
            "unexpected error: {error}"
        );
    }
}
