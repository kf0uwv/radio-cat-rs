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

//! Radio-independent Morse code, for training and for decoding.
//!
//! See `docs/adr/0021-cat-morse.md`.
//!
//! | Module | Does |
//! |--------|------|
//! | [`codec`] | text ↔ tokens ↔ dits and dahs, prosigns as `<AR>` |
//! | [`timing`] | PARIS and Farnsworth speed, seeded operator fists, keying |
//! | [`decoder`] | streaming tone decoder: fixed or auto pitch, adaptive speed |
//! | [`scenario`] | seeded QSO scripts: rag-chew, contest, POTA, SOTA, pileup |
//! | [`score`] | align sent vs copied, CER, per-character stats |
//! | [`synth`] | click-free tone, and a band: noise, QSB, QRN, QRM, filter |
//!
//! # What this crate is not
//!
//! It does no device audio I/O — it makes and reads `f32` PCM buffers and
//! the caller owns the sound card — and it knows nothing about any radio:
//! no keying commands, no CAT. It is synchronous and has no async runtime.
//!
//! # Determinism
//!
//! Everything seeded uses the in-crate [`rng::Rng`], never `rand`.
//! [`timing::key`] and [`scenario::generate`] are bit-identical on every
//! platform for a given seed. Rendered audio ([`synth`]) is bit-identical
//! on one platform and equal within float tolerance across platforms,
//! because the tone uses the platform's `sin`/`cos`.
//!
//! # A copy-practice loop
//!
//! ```
//! use std::time::Duration;
//! use cat_morse::{align, decode_all, generate, timing, Callsign, Channel, DecoderConfig,
//!     PitchMode, ScenarioConfig, ScenarioKind, ScoreOptions, Signal};
//! use cat_morse::scenario::{Difficulty, Party};
//!
//! let script = generate(&ScenarioConfig {
//!     kind: ScenarioKind::RagChew,
//!     my_call: Callsign::parse("KF0UWV").unwrap(),
//!     difficulty: Difficulty::beginner(),
//!     seed: 7,
//! }).unwrap();
//! let line = script.transmissions.iter().find(|t| matches!(t.from, Party::Them(_))).unwrap();
//! let sig = script.signal_for(line).unwrap();
//! let keying = timing::key_text(&line.text, sig.speed, &sig.fist, sig.fist_seed).unwrap();
//!
//! let mut band = Channel::new(8000, 1);
//! band.signals.push(Signal::new(keying, 700.0 + sig.pitch_offset_hz)
//!     .starting_at(Duration::from_millis(300)));
//! band.padding = Duration::from_millis(500);
//! let pcm = band.render().unwrap(); // hand this to a sound card
//!
//! // The operator's copy would come from the keyboard; here the decoder copies.
//! let copied = decode_all(DecoderConfig::new(8000, PitchMode::Auto { min_hz: 400.0, max_hz: 1000.0 }), &pcm).unwrap();
//! let score = align(&line.text, &copied, &ScoreOptions::default()).unwrap().score();
//! assert!(score.cer() < 0.1);
//! ```

pub mod codec;
pub mod decoder;
mod dsp;
pub mod rng;
pub mod scenario;
pub mod score;
pub mod synth;
pub mod timing;

pub use codec::{CodecError, DecodeStyle, Element, Prosign, Token};
pub use decoder::{decode_all, Decoded, Decoder, DecoderConfig, DecoderError, PitchMode};
pub use dsp::goertzel_power;
pub use rng::Rng;
pub use scenario::{generate, Callsign, ScenarioConfig, ScenarioError, ScenarioKind, Script};
pub use score::{
    align, align_unordered, Alignment, CharStats, Score, ScoreOptions, UnorderedAlignment,
};
pub use synth::{
    Awgn, Channel, ChannelRenderer, Qrn, Qsb, RxFilter, Signal, SynthError, ToneShape,
};
pub use timing::{Fist, Keying, Segment, Speed, TimingError};
