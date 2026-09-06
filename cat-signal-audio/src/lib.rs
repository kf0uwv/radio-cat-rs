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

//! A radio's receive audio, in as AF scope and spectrum frames — off a
//! socket or off a sound card — plus the transmit half of the pair.
//!
//! See `docs/adr/0017-acc2-audio-source.md` and its 2026-09-02 amendment.
//!
//! # Two sources, one pipeline
//!
//! | Source | Opened with | Spec |
//! |--------|-------------|------|
//! | A radio's audio endpoint over the network | [`AudioStream::connect`] | `127.0.0.1:4533` |
//! | A local sound card | `AudioCapture::open` (needs the `device` feature) | `audio:<name>` |
//!
//! Both produce the same [`AudioFrame`]s through the same DSP, the same
//! newest-wins frame slot and the same terminal-close rule; a sound card
//! reaches that pipeline through [`ring::PcmRing`] and the
//! [`AudioStream::from_reader`] seam that already existed. Code that is
//! generic over [`AudioSource`] needs no branch.
//!
//! [`input_devices`] lists the sound cards this machine has, and answers
//! "this build cannot look" rather than "nothing is plugged in" when the
//! `device` feature is off. See the [`device`] module for the spec grammar.
//!
//! # The wire (the network source)
//!
//! The first server is `ts570d`'s emulator (`--acc2-audio <addr>`, that
//! repo's ADR 0009), which carries a TS-570D's ACC2 audio pins over the
//! socket; nothing in this crate is specific to that radio or connector.
//!
//! One TCP connection, both directions live at once, 48 kHz mono signed
//! 16-bit little-endian PCM, paced to real time by the server.
//!
//! - **Server to client** is the radio's receive audio (ACC2 pin 3, ANO).
//! - **Client to server** is transmit audio into the radio (ACC2 pin 11,
//!   PKD).
//!
//! The stream is continuous in both directions. A receiving radio always
//! sends *something* — there is a noise floor — and a transmitting radio
//! sends digital silence. So silence is a normal frame and never an error,
//! and a gap in the stream means the link is broken rather than the band
//! being quiet.
//!
//! # What the transmit path does not do
//!
//! [`AudioTransmitter::send`] puts audio on the radio's PKD pin. It does
//! **not** key the radio: no VOX, no PTT, no CAT command, nothing. On the
//! station this was built for, keying is DTR through an opto-isolator
//! ([`cat-transport-serial`]'s `ModemControlLines`, or
//! `cat-transport-rfc2217`'s), and it stays that way. Audio arriving at a
//! receiving radio's PKD pin does nothing at all, which is the correct and
//! safe outcome.
//!
//! [`cat-transport-serial`]: https://github.com/kf0uwv/radio-cat-rs
//!
//! # Backpressure: the newest frame wins
//!
//! The reader thread and the console share **one slot**. If the console has
//! not collected the previous frame, the worker overwrites it and counts a
//! drop (`frames_dropped`, a read-only setting).
//!
//! This matters more for audio than it did for
//! [`cat-signal-rtlsdr`](https://github.com/kf0uwv/radio-cat-rs)'s
//! waterfall. A queue between a real-time stream and a slow consumer does
//! not merely add latency once; the latency grows without bound, because
//! the producer never slows down. A console showing audio from 400 ms ago,
//! then 800 ms, then two seconds, is worse than useless on a transmit
//! monitor. Dropping is the only policy that keeps the display at the live
//! edge.
//!
//! # Threading, and the monoio trap this crate avoids
//!
//! A reader thread does the blocking socket I/O and the DSP; the console
//! collects finished frames through a `Mutex` + `Condvar`. **No task is
//! ever woken from another OS thread**, because no waker is ever
//! registered — [`AudioStream::next_frame`] blocks the calling thread on
//! the condvar.
//!
//! That is deliberate, not incidental. `docs/adr/0016-rfc2217-transport.md`
//! records what the other shape costs: a cross-thread wake into a monoio
//! task panics (*"waker can only be sent across threads when `sync`
//! feature enabled"*) unless every caller remembers a feature flag this
//! crate cannot enforce and no test here can check. This crate needs no
//! such flag from anybody.
//!
//! The price is that `next_frame` blocks — on a monoio executor, that is
//! the executor thread, for up to one block (21 ms at the defaults). **A
//! console driven by a render loop should call
//! [`AudioStream::try_next_frame`] instead**, which never blocks at all.
//!
//! # Example
//!
//! ```no_run
//! use cat_signal_audio::{AudioPipelineConfig, AudioStream};
//!
//! let (mut audio, transmit) =
//!     AudioStream::connect("127.0.0.1:4533", AudioPipelineConfig::default())?;
//!
//! // Receive: poll from a render loop; never blocks.
//! if let Some(frame) = audio.try_next_frame()? {
//!     draw_scope(&frame.scope);
//!     draw_af_spectrum(&frame.spectrum);
//! }
//!
//! // Transmit: audio into the radio. This keys nothing.
//! let clipped = transmit.send(&[0.0; 960])?;
//! # fn draw_scope(_: &cat_signal::AudioScopeFrame) {}
//! # fn draw_af_spectrum(_: &cat_signal::AudioSpectrumFrame) {}
//! # let _ = clipped;
//! # Ok::<(), cat_signal_audio::AudioError>(())
//! ```
//!
//! The same console, reading a sound card instead — note that only the
//! opening differs, and that the list is worth showing even in a build
//! that cannot capture, because it says so:
//!
//! ```no_run
//! # #[cfg(feature = "device")]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use cat_signal_audio::{AudioCapture, CaptureConfig};
//!
//! let list = cat_signal_audio::input_devices();
//! if let Some(why) = &list.error {
//!     eprintln!("cannot look for sound cards: {why}");
//! }
//! let picked = list.default_device().or_else(|| list.devices.first());
//!
//! if let Some(device) = picked {
//!     // `device.spec` is exactly what `--acc2-audio` takes.
//!     let mut audio = AudioCapture::open(&device.spec, CaptureConfig::default())?;
//!     eprintln!("{} at {} Hz", audio.label(), audio.format().sample_rate_hz);
//!     if let Some(frame) = audio.try_next_frame()? {
//!         let _ = frame.scope.samples.len();
//!     }
//! }
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "device"))]
//! # fn main() {}
//! ```

#[cfg(feature = "device")]
pub mod capture;
pub mod device;
pub mod dsp;
pub mod pcm;
pub mod ring;
pub mod stream;

use async_trait::async_trait;
use cat_signal::{SettingValue, SignalCapability, SpectrumSettings};

#[cfg(feature = "device")]
pub use capture::{AudioCapture, CaptureConfig, CaptureError, CaptureFormat};
pub use device::{device_spec, input_devices, AudioEndpoint, DEVICE_SPEC_PREFIX};
pub use dsp::{AudioPipeline, AudioPipelineConfig};
pub use ring::{PcmRing, RingReader};

// `AudioFrame` lives in `cat-signal` now, beside the two halves it pairs.
// Re-exported because it is this crate's own output type and callers
// should not have to know which crate declares it -- and because
// `cat-native` must be able to name it without depending on a crate that
// can link cpal.
pub use cat_signal::AudioFrame;

/// Anything audio frames can be pulled from, without blocking.
///
/// Two things implement it and they are the same shape by design: a
/// network stream and a local sound card both hand over finished
/// [`AudioFrame`]s and both promise never to block. A consumer does not
/// care which it has — a radio's receive audio is its receive audio
/// however it reached the machine — so this is the whole of what it needs
/// to know.
///
/// It lives here rather than in a console crate because it is now asked
/// for on both sides of a socket: a server capturing audio to publish
/// needs exactly the same abstraction as a console drawing it, and a
/// server that had to depend on a terminal UI crate to name the trait
/// would be an odd shape indeed.
pub trait AudioTap {
    /// The next finished frame, or `None` if none is ready yet.
    ///
    /// Never blocks. A caller polls at its own rate and takes what has
    /// arrived; see `AudioStream` for what happens to frames produced
    /// faster than they are taken.
    fn try_next_frame(&mut self) -> Result<Option<AudioFrame>, AudioError>;
}

impl AudioTap for AudioStream {
    fn try_next_frame(&mut self) -> Result<Option<AudioFrame>, AudioError> {
        AudioStream::try_next_frame(self)
    }
}

#[cfg(feature = "device")]
impl AudioTap for crate::AudioCapture {
    fn try_next_frame(&mut self) -> Result<Option<AudioFrame>, AudioError> {
        crate::AudioCapture::try_next_frame(self)
    }
}
pub use stream::{AudioStream, AudioTransmitter};

/// What can go wrong on an audio link.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("could not connect to the audio endpoint: {0}")]
    Connect(#[source] std::io::Error),
    /// The stream ended and will not resume.
    ///
    /// **Terminal.** Once a stream reports this, every later call reports
    /// it again with the same reason; there is no reconnect inside the
    /// source. See ADR 0017 §6 for why reconnect policy belongs to the
    /// application.
    #[error("audio stream ended: {reason}")]
    Closed { reason: String },
    #[error("unknown setting: {0}")]
    UnknownSetting(String),
    #[error("setting is read-only: {0}")]
    ReadOnly(&'static str),
    #[error("wrong value kind for setting: {0}")]
    WrongKind(&'static str),
    #[error("value out of range: {0}")]
    OutOfRange(&'static str),
}

/// A source of audio-domain frames.
///
/// The audio-side counterpart of [`cat_signal::SpectrumSource`], and
/// deliberately **not** that trait: an audio stream has no `center_hz` and
/// no `retune`, because audio does not move when the dial does. A trait
/// that offered `retune` on an audio source would be inviting a console to
/// position speech on a band axis, which is the confusion
/// `cat_signal::audio` exists to make impossible.
///
/// `#[async_trait(?Send)]` matches the house binding from
/// `docs/adr/0002-async-runtime-binding-for-transport-crates.md`.
#[async_trait(?Send)]
pub trait AudioSource {
    type Error;

    /// Wait for and return the next frame.
    ///
    /// Blocks the calling thread until a frame is available — see the crate
    /// header. A render loop wants [`AudioStream::try_next_frame`].
    async fn next_frame(&mut self) -> Result<AudioFrame, Self::Error>;

    /// What kind of source this is, and what it can therefore be used for.
    ///
    /// Always a [`SignalCapability::AudioDerived`], whose `max_bandwidth_hz`
    /// is what lets a console refuse to render this as a panorama.
    fn capability(&self) -> SignalCapability;

    /// The knobs this source exposes, with current values.
    fn settings(&self) -> SpectrumSettings;

    /// Write one setting by key.
    fn apply(&mut self, key: &str, value: SettingValue) -> Result<(), Self::Error>;
}
