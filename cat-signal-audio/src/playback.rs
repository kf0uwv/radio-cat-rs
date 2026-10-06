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

//! Playback to a local sound card. Behind the `device` feature.
//!
//! The mirror of [`AudioCapture`](crate::AudioCapture): the same `audio:`
//! spec grammar, the same "the device's own rate if it cannot do yours,
//! and say so" rule, the same no-waker threading. What it plays is
//! whatever the application writes — locally generated audio such as a
//! Morse trainer's tone — and it keys nothing.
//!
//! Everything with an invariant lives in [`sink`](crate::sink) and is
//! tested without a sound card; this module is the `cpal` calls around it.
//! The output callback *tries* the queue's one mutex, copies, and returns;
//! it never waits on it.

use std::sync::Arc;

use cat_signal::{DeviceInfo, DeviceKind, DeviceList};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SampleRate, SizedSample, SupportedStreamConfig};

use crate::capture::format_rank;
use crate::device::{device_spec, AudioEndpoint};
use crate::sink::{
    choose_output, OutputChoice, OutputOffer, PlaybackConfig, PlaybackError, PlaybackFormat,
    PlaybackQueue,
};

/// A sound-card output fed from a non-blocking queue.
///
/// Not `Send` on every platform (a `cpal::Stream` is not): keep it on the
/// thread that opened it, as with [`AudioCapture`](crate::AudioCapture).
pub struct AudioPlayback {
    /// Holds the device open; dropping it stops the callback.
    _device_stream: cpal::Stream,
    queue: Arc<PlaybackQueue>,
    label: String,
    format: PlaybackFormat,
}

impl AudioPlayback {
    /// Opens the output a spec names: `audio:<name>`, or `audio:` for the
    /// host default. A network endpoint is refused.
    pub fn open(spec: &str, config: PlaybackConfig) -> Result<Self, PlaybackError> {
        match AudioEndpoint::parse(spec) {
            AudioEndpoint::Device(name) => Self::open_named(name, config),
            AudioEndpoint::Network(_) => Err(PlaybackError::NotADeviceSpec(spec.to_string())),
        }
    }

    /// Opens the host's default output. The same as `open("audio:", ..)`.
    pub fn open_default(config: PlaybackConfig) -> Result<Self, PlaybackError> {
        Self::open_named("", config)
    }

    fn open_named(name: &str, config: PlaybackConfig) -> Result<Self, PlaybackError> {
        let _host_lock = crate::device::host_lock();
        let host = cpal::default_host();
        let device = if name.is_empty() {
            host.default_output_device()
                .ok_or_else(|| PlaybackError::NotFound {
                    name: String::new(),
                    alternatives: ": this host has no default output device".to_string(),
                })?
        } else {
            find_by_name(&host, name)?
        };
        let label = device.name().unwrap_or_else(|_| name.to_string());
        let (chosen, rate_substituted) =
            negotiate(&device, config.sample_rate_hz).map_err(|why| {
                PlaybackError::Unsupported {
                    name: label.clone(),
                    why,
                }
            })?;
        let rate = chosen.sample_rate().0;
        let channels = chosen.channels();
        let queue = Arc::new(PlaybackQueue::for_duration(rate, config.buffer_ms));
        let stream =
            build_stream(&device, &chosen, &queue).map_err(|why| PlaybackError::Start {
                name: label.clone(),
                why,
            })?;
        stream.play().map_err(|e| PlaybackError::Start {
            name: label.clone(),
            why: e.to_string(),
        })?;
        Ok(AudioPlayback {
            _device_stream: stream,
            queue,
            format: PlaybackFormat {
                sample_rate_hz: rate,
                channels,
                sample_format: chosen.sample_format().to_string(),
                rate_substituted,
            },
            label,
        })
    }

    /// What the device is running at; render at its `sample_rate_hz`.
    pub fn format(&self) -> &PlaybackFormat {
        &self.format
    }

    /// The driver's name for the device.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Queues mono audio at [`PlaybackFormat::sample_rate_hz`], copied to
    /// every channel and clamped to ±1. **Never blocks**: returns how many
    /// samples were taken (fewer than given when the queue is full).
    pub fn write(&self, mono: &[f32]) -> Result<usize, PlaybackError> {
        self.queue.write(mono)
    }

    /// Samples accepted but not yet handed to the device.
    pub fn queued(&self) -> usize {
        self.queue.queued()
    }

    /// Discards queued audio; silence from the next callback.
    pub fn clear(&self) {
        self.queue.clear()
    }

    /// Callbacks that ran short while playing (see [`crate::sink`]).
    pub fn underruns(&self) -> u64 {
        self.queue.underruns()
    }

    /// Callbacks that found a write in progress and played silence rather
    /// than wait for it (see [`crate::sink`]).
    pub fn busy_callbacks(&self) -> u64 {
        self.queue.busy_callbacks()
    }
}

impl std::fmt::Debug for AudioPlayback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioPlayback")
            .field("label", &self.label)
            .field("format", &self.format)
            .field("queued", &self.queued())
            .field("underruns", &self.underruns())
            .finish()
    }
}

/// The body of [`crate::output_devices`], when the feature is on.
pub(crate) fn enumerate() -> DeviceList {
    let _host_lock = crate::device::host_lock();
    let host = cpal::default_host();
    let default_name = host.default_output_device().and_then(|d| d.name().ok());
    let found = match host.output_devices() {
        Ok(devices) => devices,
        Err(e) => {
            return DeviceList::unavailable(
                DeviceKind::AudioOutput,
                format!("could not enumerate audio output devices: {e}"),
            )
        }
    };
    let mut devices: Vec<DeviceInfo> = Vec::new();
    for device in found {
        let Ok(name) = device.name() else { continue };
        // A duplicate name is a spec that could not be honoured.
        if devices.iter().any(|d| d.label == name) {
            continue;
        }
        let detail = device.default_output_config().ok().map(|c| {
            format!(
                "{} ch, {} Hz default, {}",
                c.channels(),
                c.sample_rate().0,
                c.sample_format()
            )
        });
        devices.push(DeviceInfo {
            kind: DeviceKind::AudioOutput,
            spec: device_spec(&name),
            is_default: default_name.as_deref() == Some(name.as_str()),
            detail,
            label: name,
        });
    }
    DeviceList::found(DeviceKind::AudioOutput, devices)
}

fn find_by_name<H: HostTrait>(host: &H, name: &str) -> Result<H::Device, PlaybackError> {
    let devices = host.output_devices().map_err(|e| PlaybackError::NotFound {
        name: name.to_string(),
        alternatives: format!(": could not enumerate outputs ({e})"),
    })?;
    let mut seen: Vec<String> = Vec::new();
    for device in devices {
        match device.name() {
            Ok(found) if found == name => return Ok(device),
            Ok(found) => seen.push(found),
            Err(_) => {}
        }
    }
    Err(PlaybackError::NotFound {
        name: name.to_string(),
        alternatives: if seen.is_empty() {
            String::new()
        } else {
            format!(". This machine has: {}", seen.join(", "))
        },
    })
}

/// The config to open, and whether the rate was substituted.
fn negotiate<D: DeviceTrait>(
    device: &D,
    wanted_hz: u32,
) -> Result<(SupportedStreamConfig, bool), String> {
    let default = device
        .default_output_config()
        .map_err(|e| format!("no default output config: {e}"))?;
    let ranges = device
        .supported_output_configs()
        .map(|r| r.collect::<Vec<_>>())
        .unwrap_or_default();
    let offers: Vec<OutputOffer> = ranges
        .iter()
        .map(|r| OutputOffer {
            channels: r.channels(),
            min_rate_hz: r.min_sample_rate().0,
            max_rate_hz: r.max_sample_rate().0,
            format_rank: format_rank(r.sample_format()),
        })
        .collect();
    Ok(
        match choose_output(&offers, default.channels(), wanted_hz) {
            OutputChoice::Offer { index } => {
                (ranges[index].with_sample_rate(SampleRate(wanted_hz)), false)
            }
            OutputChoice::DeviceDefault => {
                let substituted = default.sample_rate().0 != wanted_hz;
                (default, substituted)
            }
        },
    )
}

fn build_stream<D: DeviceTrait>(
    device: &D,
    config: &SupportedStreamConfig,
    queue: &Arc<PlaybackQueue>,
) -> Result<D::Stream, String> {
    match config.sample_format() {
        SampleFormat::I8 => typed::<D, i8>(device, config, queue),
        SampleFormat::I16 => typed::<D, i16>(device, config, queue),
        SampleFormat::I32 => typed::<D, i32>(device, config, queue),
        SampleFormat::I64 => typed::<D, i64>(device, config, queue),
        SampleFormat::U8 => typed::<D, u8>(device, config, queue),
        SampleFormat::U16 => typed::<D, u16>(device, config, queue),
        SampleFormat::U32 => typed::<D, u32>(device, config, queue),
        SampleFormat::U64 => typed::<D, u64>(device, config, queue),
        SampleFormat::F32 => typed::<D, f32>(device, config, queue),
        SampleFormat::F64 => typed::<D, f64>(device, config, queue),
        other => Err(format!("unsupported sample format {other}")),
    }
}

fn typed<D: DeviceTrait, T>(
    device: &D,
    config: &SupportedStreamConfig,
    queue: &Arc<PlaybackQueue>,
) -> Result<D::Stream, String>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let channels = usize::from(config.channels());
    let producer = Arc::clone(queue);
    let on_error = Arc::clone(queue);
    // Reused, so the real-time thread allocates only when a callback asks
    // for a bigger buffer than any before it.
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_output_stream::<T, _, _>(
            &config.config(),
            move |data: &mut [T], _| {
                scratch.resize(data.len(), 0.0);
                producer.render(&mut scratch, channels);
                for (d, &x) in data.iter_mut().zip(&scratch) {
                    *d = T::from_sample(x);
                }
            },
            move |e| on_error.close(format!("audio device stopped: {e}")),
            None,
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_network_endpoint_is_refused() {
        let e = AudioPlayback::open("127.0.0.1:4533", PlaybackConfig::default())
            .expect_err("not a device");
        assert!(matches!(e, PlaybackError::NotADeviceSpec(_)));
    }

    #[test]
    fn a_name_that_is_not_here_is_not_found() {
        let e = AudioPlayback::open("audio:no such card", PlaybackConfig::default())
            .expect_err("no card by that name");
        assert!(matches!(e, PlaybackError::NotFound { .. }), "{e}");
    }

    #[test]
    fn enumeration_answers_with_the_output_kind() {
        let list = enumerate();
        assert_eq!(list.kind, DeviceKind::AudioOutput);
        for d in &list.devices {
            assert!(d.spec.starts_with("audio:"));
        }
    }
}
