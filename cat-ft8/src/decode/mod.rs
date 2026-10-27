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

//! The decoder (single pass), being built step by step (ADR 0024,
//! amendment "single-pass decoder"): a block-fed spectrogram, the Costas
//! sync search, soft bits, belief propagation and the CRC, behind the
//! [`Decoder`] and [`DecodeList`] API (see the [`decoder`] module).
//!
//! The stages (`sync`, `soft`, `bp`) are crate-private; the `profile`
//! feature makes them public for measurement benches only.

#[cfg(feature = "profile")]
#[doc(hidden)]
pub mod bp;
#[cfg(not(feature = "profile"))]
pub(crate) mod bp;
pub mod decoder;
pub(crate) mod fft;
pub mod frontend;
pub(crate) mod math;
pub(crate) mod snr;
#[cfg(feature = "profile")]
#[doc(hidden)]
pub mod soft;
#[cfg(not(feature = "profile"))]
pub(crate) mod soft;
#[cfg(feature = "profile")]
#[doc(hidden)]
pub mod sync;
#[cfg(not(feature = "profile"))]
pub(crate) mod sync;
pub(crate) mod tables;
#[cfg(test)]
pub(crate) mod testutil;

pub use bp::{BpFailure, CheckRule};
pub use decoder::{
    CandidateResult, CandidateScratch, Decode, DecodeConfig, DecodeList, DecodeReport, Decoder,
    CANDIDATES, HOST_DECODES, SMALL_DECODES,
};
pub use frontend::{Band, FrontEnd, Spectrogram, BIN_HZ, HOP, ROWS, ROW_ORIGIN};
pub use snr::SNR_FLOOR_DB;
pub use sync::{Candidate, SyncReport};

/// Bytes of the decoder's static tables (FFT twiddles, real-split
/// twiddles, half Hann window, digit-reversal table, belief
/// propagation's check list and degrees, and the level-to-power table),
/// in flash.
pub const TABLE_BYTES: usize = fft::TABLE_BYTES
    + core::mem::size_of::<[[u8; 7]; 83]>()
    + core::mem::size_of::<[u8; 83]>()
    + core::mem::size_of::<[f32; 256]>();
