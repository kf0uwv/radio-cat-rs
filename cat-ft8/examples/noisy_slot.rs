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

//! A test slot with white Gaussian noise:
//!
//! ```text
//! cargo run --release -p cat-ft8 --example noisy_slot -- "<message>" --snr -15 [--seed 1] [--freq 1500] [--dt 0.0] --out slot.wav
//! ```
//!
//! The signal-to-noise ratio is measured in a 2500 Hz bandwidth (the usual
//! FT8 definition). Noise RMS is 0.03 of full scale; the signal amplitude
//! follows from the SNR. The noise is seeded and reproducible. A test
//! harness, not part of the library: it uses `f64` and the platform's
//! `ln`/`sin`/`cos`.

use std::fs::File;
use std::io::BufWriter;
use std::process::ExitCode;

use cat_ft8::message::NoHashes;
use cat_ft8::waveform::{synthesize, Placement};
use cat_ft8::{encode, wav, SAMPLE_RATE, SLOT_SAMPLES};

const USAGE: &str =
    "usage: noisy_slot \"<message>\" --snr DB [--seed N] [--freq HZ] [--dt S] --out FILE.wav";
const NOISE_RMS: f64 = 0.03;

/// xoshiro256** seeded through splitmix64, with Box–Muller normals.
struct Noise {
    s: [u64; 4],
    spare: Option<f64>,
}

impl Noise {
    fn new(seed: u64) -> Self {
        let mut z = seed;
        let mut next = || {
            z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            x ^ (x >> 31)
        };
        Noise {
            s: [next(), next(), next(), next()],
            spare: None,
        }
    }

    fn next_u64(&mut self) -> u64 {
        let r = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        r
    }

    fn uniform(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    fn normal(&mut self) -> f64 {
        if let Some(v) = self.spare.take() {
            return v;
        }
        let (u1, u2) = (self.uniform(), self.uniform());
        let r = (-2.0 * u1.ln()).sqrt();
        let th = 2.0 * std::f64::consts::PI * u2;
        self.spare = Some(r * th.sin());
        r * th.cos()
    }
}

fn run() -> Result<(), String> {
    let mut it = std::env::args().skip(1);
    let (mut message, mut out) = (String::new(), String::new());
    let (mut freq, mut dt, mut snr, mut seed) = (1500.0f32, 0.0f32, None::<f64>, 1u64);
    while let Some(arg) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--freq" => freq = val("--freq")?.parse().map_err(|e| format!("--freq: {e}"))?,
            "--dt" => dt = val("--dt")?.parse().map_err(|e| format!("--dt: {e}"))?,
            "--snr" => snr = Some(val("--snr")?.parse().map_err(|e| format!("--snr: {e}"))?),
            "--seed" => seed = val("--seed")?.parse().map_err(|e| format!("--seed: {e}"))?,
            "--out" => out = val("--out")?,
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n{USAGE}")),
            s if message.is_empty() => message = s.to_string(),
            s => return Err(format!("unexpected argument {s}\n{USAGE}")),
        }
    }
    let (Some(snr), false, false) = (snr, message.is_empty(), out.is_empty()) else {
        return Err(USAGE.into());
    };
    let enc = encode(&message, &mut NoHashes).map_err(|e| format!("{message}: {e}"))?;
    // Noise power in 2500 Hz of the 6000 Hz band; a sine of amplitude A
    // has power A²/2.
    let noise_2500 = NOISE_RMS * NOISE_RMS * 2500.0 / (f64::from(SAMPLE_RATE) / 2.0);
    let amplitude = (2.0 * noise_2500 * 10f64.powf(snr / 10.0)).sqrt() as f32;
    let mut placement = Placement::slot(freq, dt).map_err(|e| e.to_string())?;
    placement.amplitude = amplitude;
    let mut slot = vec![0.0f32; SLOT_SAMPLES];
    synthesize(&enc.symbols, &placement, &mut slot).map_err(|e| e.to_string())?;
    let mut noise = Noise::new(seed);
    for x in slot.iter_mut() {
        *x += (NOISE_RMS * noise.normal()) as f32;
    }
    let file = File::create(&out).map_err(|e| format!("{out}: {e}"))?;
    let clipped = wav::write_wav(BufWriter::new(file), &slot, SAMPLE_RATE)
        .map_err(|e| format!("{out}: {e}"))?;
    if clipped > 0 {
        eprintln!("warning: {clipped} samples clipped");
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
