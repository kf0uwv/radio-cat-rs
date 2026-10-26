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
//! amendment "single-pass decoder"). So far: numeric helpers, the
//! spectrum of one analysis window, and the block-fed front end.

pub(crate) mod fft;
pub mod frontend;
pub(crate) mod math;
mod tables;

/// Bytes of the decoder's static tables (FFT twiddles, real-split
/// twiddles, half Hann window, digit-reversal table), in flash.
pub use fft::TABLE_BYTES;
pub use frontend::{Band, FrontEnd, Level, Spectrogram, HOP, ROWS, ROW_ORIGIN};
