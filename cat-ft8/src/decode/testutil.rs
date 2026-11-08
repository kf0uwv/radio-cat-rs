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

//! Test helpers shared by the decoder's stage tests: seeded Gaussian
//! noise (f64 and libm, tests only) and synthetic slots.

extern crate std;

use std::vec;
use std::vec::Vec;

use super::frontend::{Band, FrontEnd, Spectrogram};
use crate::message::NoHashes;
use crate::waveform::{synthesize, Placement};
use crate::{SLOT_SAMPLES, SYMBOL_COUNT};

/// Noise RMS used for SNR-defined slots (−30.5 dBFS).
pub(crate) const NOISE_RMS: f64 = 0.03;

/// splitmix64 → Gaussian by Box–Muller.
pub(crate) struct Noise(pub(crate) u64);

impl Noise {
    pub(crate) fn uniform(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64 + 1e-300
    }
    pub(crate) fn gauss(&mut self) -> f64 {
        let (u, v) = (self.uniform(), self.uniform());
        libm::sqrt(-2.0 * libm::log(u)) * libm::cos(2.0 * core::f64::consts::PI * v)
    }
}

/// Peak amplitude of a signal at `snr_db` (2500 Hz) over noise of RMS
/// [`NOISE_RMS`].
pub(crate) fn amplitude_for(snr_db: f64) -> f32 {
    let noise_2500 = NOISE_RMS * NOISE_RMS * 2500.0 / 6000.0;
    libm::sqrt(2.0 * noise_2500 * libm::pow(10.0, snr_db / 10.0)) as f32
}

/// Symbols of a message.
pub(crate) fn symbols_of(message: &str) -> [u8; SYMBOL_COUNT] {
    crate::encode(message, &mut NoHashes).unwrap().symbols
}

/// Adds a signal to `slot`.
pub(crate) fn add_signal(
    slot: &mut [f32],
    symbols: &[u8; SYMBOL_COUNT],
    base_hz: f32,
    dt: f32,
    amp: f32,
) {
    let mut p = Placement::slot(base_hz, dt).unwrap();
    p.amplitude = amp;
    synthesize(symbols, &p, slot).unwrap();
}

/// Adds seeded white noise of RMS `rms`.
pub(crate) fn add_noise(slot: &mut [f32], rms: f64, seed: u64) {
    let mut n = Noise(seed);
    for x in slot.iter_mut() {
        *x += (rms * n.gauss()) as f32;
    }
}

/// An empty slot.
pub(crate) fn empty_slot() -> Vec<f32> {
    vec![0.0; SLOT_SAMPLES]
}

/// A spectrogram's buffers, filled from `samples` in one block.
pub(crate) struct Built {
    pub(crate) front: FrontEnd,
    pub(crate) levels: Vec<u8>,
    pub(crate) band: Band,
}

impl Built {
    pub(crate) fn new(band: Band) -> Self {
        Built {
            front: FrontEnd::new(),
            levels: vec![0; band.spectrogram_len()],
            band,
        }
    }

    /// Runs `f` on a spectrogram of `samples[..n]`.
    pub(crate) fn with<R>(&mut self, samples: &[f32], f: impl FnOnce(&Spectrogram<'_>) -> R) -> R {
        let mut s = Spectrogram::new(&mut self.front, &mut self.levels, self.band).unwrap();
        s.feed(samples);
        f(&s)
    }
}
