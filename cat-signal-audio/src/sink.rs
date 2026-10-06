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

//! The parts of playback that need no sound card: settings, the format a
//! device ended up in, errors, the queue between an application and the
//! output callback, and the rule for choosing an output format.
//!
//! **Compiled in every build and tested there**, exactly as
//! [`PcmRing`](crate::ring::PcmRing) is for capture: the invariants live
//! here, and the `device`-gated [`playback`](crate::playback) module is
//! only `cpal` calls around them.
//!
//! # The callback never waits and never stalls
//!
//! An output callback runs on the driver's real-time thread and must
//! return a full buffer *now*. [`PlaybackQueue::render`] takes what is
//! queued and fills the rest with **silence**; it never blocks and never
//! leaves the device without samples. The writer side,
//! [`PlaybackQueue::write`], never blocks either: it accepts what fits and
//! says how much that was, and the application decides whether to retry.
//! One mutex is the only thing the two sides share; nothing registers a
//! waker or wakes a task from the callback thread (ADR 0017 §5).
//!
//! # What an underrun is
//!
//! A callback that *was playing* — the previous callback filled its whole
//! buffer from the queue — and ran out partway. Silence because nothing
//! was written is idle and is not counted, nor is the silence after
//! [`PlaybackQueue::clear`], nor the callbacks after the first short one.
//!
//! The queue cannot tell starvation from the end of what was written, so
//! the natural end of a piece of audio counts as one underrun unless it
//! happens to land on a callback boundary. An application that wants to
//! detect starvation reads the counter while it still has audio to feed:
//! an increase *then* is a gap the listener heard.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// How a playback is set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackConfig {
    /// The rate asked for. If the device cannot do it, its own default
    /// rate is used and [`PlaybackFormat::rate_substituted`] says so —
    /// there is no resampler, so the caller must render at
    /// [`PlaybackFormat::sample_rate_hz`].
    pub sample_rate_hz: u32,
    /// Queue capacity, in milliseconds of audio at the negotiated rate.
    pub buffer_ms: u32,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        PlaybackConfig {
            sample_rate_hz: 48_000,
            buffer_ms: 2_000,
        }
    }
}

/// What the output device is actually running at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackFormat {
    /// The rate to render at.
    pub sample_rate_hz: u32,
    /// The device's channel count; mono is copied to every channel.
    pub channels: u16,
    /// The device's sample format, as the driver names it.
    pub sample_format: String,
    /// Whether the requested rate had to be given up for the device's own.
    pub rate_substituted: bool,
}

/// What can go wrong opening or feeding a sound-card output.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlaybackError {
    /// The spec was not `audio:<name>`.
    #[error("{0:?} is not a sound-card spec: a device spec looks like `audio:<name>`")]
    NotADeviceSpec(String),
    /// Nothing on this machine has that name (or there is no default).
    #[error("no audio output device named {name:?}{alternatives}")]
    NotFound {
        name: String,
        /// The names that do exist, so the message is actionable.
        alternatives: String,
    },
    /// The device will not say what it can play.
    #[error("audio device {name:?} would not report a usable output format: {why}")]
    Unsupported { name: String, why: String },
    /// The stream could not be created or started.
    #[error("could not start playback on {name:?}: {why}")]
    Start { name: String, why: String },
    /// The device stopped (unplugged, or the sound service went away).
    /// Terminal, and reported on the next call.
    #[error("audio output closed: {0}")]
    Closed(String),
}

struct QueueState {
    samples: VecDeque<f32>,
    /// The previous `render` took queued audio.
    playing: bool,
    closed: Option<String>,
}

/// The bounded mono queue between an application and an output callback.
pub struct PlaybackQueue {
    state: Mutex<QueueState>,
    capacity: usize,
    underruns: AtomicU64,
}

fn lock(m: &Mutex<QueueState>) -> MutexGuard<'_, QueueState> {
    // As `ring::lock`: a panic on one side must not poison the other, and
    // nothing behind this mutex has an invariant a panic could break.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl PlaybackQueue {
    /// A queue holding at most `capacity_samples` mono samples (at least 1).
    pub fn new(capacity_samples: usize) -> Self {
        let capacity = capacity_samples.max(1);
        PlaybackQueue {
            state: Mutex::new(QueueState {
                samples: VecDeque::with_capacity(capacity),
                playing: false,
                closed: None,
            }),
            capacity,
            underruns: AtomicU64::new(0),
        }
    }

    /// A queue of `buffer_ms` at `rate_hz`.
    pub fn for_duration(rate_hz: u32, buffer_ms: u32) -> Self {
        let n = (rate_hz as usize).saturating_mul(buffer_ms as usize) / 1000;
        PlaybackQueue::new(n)
    }

    /// Capacity in samples.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Queues as much of `mono` as fits, clamped to ±1.0 (and NaN to 0);
    /// returns how many samples were taken. **Never blocks.** After the
    /// device has stopped, returns [`PlaybackError::Closed`].
    pub fn write(&self, mono: &[f32]) -> Result<usize, PlaybackError> {
        let mut st = lock(&self.state);
        if let Some(why) = &st.closed {
            return Err(PlaybackError::Closed(why.clone()));
        }
        let room = self.capacity - st.samples.len();
        let n = room.min(mono.len());
        st.samples.extend(mono[..n].iter().map(
            |&x| {
                if x.is_nan() {
                    0.0
                } else {
                    x.clamp(-1.0, 1.0)
                }
            },
        ));
        Ok(n)
    }

    /// Samples accepted but not yet handed to the device.
    pub fn queued(&self) -> usize {
        lock(&self.state).samples.len()
    }

    /// Discards everything queued: silence from the next callback. Not
    /// counted as an underrun.
    pub fn clear(&self) {
        let mut st = lock(&self.state);
        st.samples.clear();
        st.playing = false;
    }

    /// Callbacks that ran out of audio while playing (see the module doc).
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    /// Marks the device as stopped, with why. Terminal.
    pub fn close(&self, why: impl Into<String>) {
        let mut st = lock(&self.state);
        if st.closed.is_none() {
            st.closed = Some(why.into());
        }
    }

    /// Why the device stopped, if it has.
    pub fn closed(&self) -> Option<String> {
        lock(&self.state).closed.clone()
    }

    /// **The callback side.** Fills an interleaved buffer of `channels`
    /// channels: each queued mono sample goes to every channel of one
    /// frame, and frames with nothing queued are silence. Never blocks.
    pub fn render(&self, out: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        let frames = out.len() / channels;
        let mut st = lock(&self.state);
        let available = st.samples.len();
        let take = available.min(frames);
        for (frame, x) in out.chunks_exact_mut(channels).zip(st.samples.drain(..take)) {
            frame.fill(x);
        }
        out[take * channels..].fill(0.0);
        if take < frames && st.playing {
            self.underruns.fetch_add(1, Ordering::Relaxed);
        }
        // Ran dry or not, a callback that did not fill up leaves the queue
        // empty, so the next silent callback is idle rather than another
        // underrun.
        st.playing = take > 0 && take == frames;
    }
}

/// One output configuration a device offers, reduced to what the choice
/// depends on. The `device` build maps `cpal`'s ranges into these, so the
/// rule itself is testable without a sound card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputOffer {
    /// Channels.
    pub channels: u16,
    /// Lowest supported rate.
    pub min_rate_hz: u32,
    /// Highest supported rate.
    pub max_rate_hz: u32,
    /// Conversion cost of the sample format; lower is better
    /// (f32 0, i16 1, i32 2, …).
    pub format_rank: u8,
}

/// The output configuration to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputChoice {
    /// Offer number `index`, at the wanted rate.
    Offer {
        /// Index into the offers given.
        index: usize,
    },
    /// No offer covers the wanted rate: use the device's default config.
    DeviceDefault,
}

/// Chooses an output configuration for `wanted_hz`.
///
/// Prefers the device's own default channel count (so the driver does no
/// up- or down-mixing of its own), then the fewest channels, then the
/// least conversion. If nothing covers the rate, the device default is
/// used and the caller reports the rate as substituted — there is no
/// resampler.
pub fn choose_output(
    offers: &[OutputOffer],
    default_channels: u16,
    wanted_hz: u32,
) -> OutputChoice {
    offers
        .iter()
        .enumerate()
        .filter(|(_, o)| o.min_rate_hz <= wanted_hz && o.max_rate_hz >= wanted_hz && o.channels > 0)
        .min_by_key(|(i, o)| {
            (
                o.channels != default_channels,
                o.channels,
                o.format_rank,
                *i,
            )
        })
        .map_or(OutputChoice::DeviceDefault, |(index, _)| {
            OutputChoice::Offer { index }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_never_blocks_and_reports_what_fit() {
        let q = PlaybackQueue::new(5);
        assert_eq!(q.write(&[0.1; 3]).unwrap(), 3);
        assert_eq!(q.write(&[0.1; 3]).unwrap(), 2);
        assert_eq!(q.write(&[0.1; 3]).unwrap(), 0);
        assert_eq!(q.queued(), 5);
    }

    #[test]
    fn samples_are_clamped_and_nan_is_silence() {
        let q = PlaybackQueue::new(4);
        q.write(&[2.0, -3.0, f32::NAN, 0.5]).unwrap();
        let mut out = [9.0f32; 4];
        q.render(&mut out, 1);
        assert_eq!(out, [1.0, -1.0, 0.0, 0.5]);
    }

    #[test]
    fn mono_goes_to_every_channel() {
        let q = PlaybackQueue::new(8);
        q.write(&[0.25, -0.5]).unwrap();
        let mut out = [9.0f32; 6];
        q.render(&mut out, 3);
        assert_eq!(out, [0.25, 0.25, 0.25, -0.5, -0.5, -0.5]);
    }

    #[test]
    fn idle_silence_is_not_an_underrun() {
        let q = PlaybackQueue::new(8);
        let mut out = [9.0f32; 4];
        for _ in 0..10 {
            q.render(&mut out, 2);
            assert_eq!(out, [0.0; 4]);
        }
        assert_eq!(q.underruns(), 0);
    }

    #[test]
    fn running_dry_while_playing_is_one_underrun_and_plays_silence() {
        let q = PlaybackQueue::new(16);
        q.write(&[0.5; 6]).unwrap();
        let mut out = [9.0f32; 4];
        q.render(&mut out, 1); // full
        assert_eq!(out, [0.5; 4]);
        q.render(&mut out, 1); // 2 left: short
        assert_eq!(out, [0.5, 0.5, 0.0, 0.0]);
        assert_eq!(q.underruns(), 1);
        q.render(&mut out, 1); // idle now
        q.render(&mut out, 1);
        assert_eq!(q.underruns(), 1);
        // Fed again, then starved again: a second underrun.
        q.write(&[0.5; 5]).unwrap();
        q.render(&mut out, 1);
        q.render(&mut out, 1);
        assert_eq!(q.underruns(), 2);
    }

    #[test]
    fn clear_discards_without_counting() {
        let q = PlaybackQueue::new(16);
        q.write(&[0.5; 12]).unwrap();
        let mut out = [9.0f32; 4];
        q.render(&mut out, 1);
        q.clear();
        assert_eq!(q.queued(), 0);
        q.render(&mut out, 1);
        assert_eq!(out, [0.0; 4]);
        assert_eq!(q.underruns(), 0);
    }

    #[test]
    fn close_is_terminal_and_carries_why() {
        let q = PlaybackQueue::new(4);
        q.close("unplugged");
        q.close("second reason is ignored");
        assert_eq!(
            q.write(&[0.1]),
            Err(PlaybackError::Closed("unplugged".into()))
        );
        assert_eq!(q.closed().as_deref(), Some("unplugged"));
    }

    #[test]
    fn capacity_from_duration() {
        assert_eq!(
            PlaybackQueue::for_duration(48_000, 2_000).capacity(),
            96_000
        );
        assert_eq!(PlaybackQueue::for_duration(48_000, 0).capacity(), 1);
    }

    fn offer(channels: u16, min: u32, max: u32, rank: u8) -> OutputOffer {
        OutputOffer {
            channels,
            min_rate_hz: min,
            max_rate_hz: max,
            format_rank: rank,
        }
    }

    #[test]
    fn output_choice_prefers_default_channels_then_fewest_then_format() {
        let offers = [
            offer(1, 8_000, 96_000, 0),
            offer(2, 8_000, 96_000, 1),
            offer(2, 8_000, 96_000, 0),
        ];
        assert_eq!(
            choose_output(&offers, 2, 48_000),
            OutputChoice::Offer { index: 2 }
        );
        assert_eq!(
            choose_output(&offers, 6, 48_000),
            OutputChoice::Offer { index: 0 }
        );
    }

    #[test]
    fn output_choice_falls_back_to_the_device_default() {
        let offers = [offer(2, 44_100, 44_100, 0)];
        assert_eq!(
            choose_output(&offers, 2, 48_000),
            OutputChoice::DeviceDefault
        );
        assert_eq!(choose_output(&[], 2, 48_000), OutputChoice::DeviceDefault);
    }
}
