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

//! Decoder front-end measurements:
//!
//! ```text
//! cargo run --release -p cat-ft8 --features profile --example frontend_bench
//! ```
//!
//! 1. `feed` cost per 80 ms hop (whole slot and 480-sample blocks).
//! 2. Clipping: the share of levels at the floor or the top for
//!    white noise at input levels from −110 to −10 dBFS.
//!
//! The last line is one `{"profile": {...}}` JSON object.

use std::hint::black_box;
use std::time::Instant;

use cat_ft8::decode::{Band, FrontEnd, Spectrogram, ROWS};
use cat_ft8::message::NoHashes;
use cat_ft8::waveform::{synthesize, Placement};
use cat_ft8::{encode, SLOT_SAMPLES, SYMBOL_COUNT};

const NOISE_RMS: f64 = 0.03;

struct Noise(u64);

impl Noise {
    fn uniform(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64 + 1e-300
    }
    fn gauss(&mut self) -> f64 {
        let (u, v) = (self.uniform(), self.uniform());
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

/// A slot: `symbols` at `base_hz` and the nominal start, white noise of
/// RMS `noise_rms`, SNR `snr_db` in 2500 Hz.
fn make_slot(
    symbols: &[u8; SYMBOL_COUNT],
    base_hz: f32,
    snr_db: f64,
    noise_rms: f64,
    seed: u64,
) -> Vec<f32> {
    let noise_2500 = noise_rms * noise_rms * 2500.0 / 6000.0;
    let amp = (2.0 * noise_2500 * 10f64.powf(snr_db / 10.0)).sqrt() as f32;
    let mut p = Placement::slot(base_hz, 0.0).unwrap();
    p.amplitude = amp;
    let mut out = vec![0.0f32; SLOT_SAMPLES];
    synthesize(symbols, &p, &mut out).unwrap();
    let mut n = Noise(seed);
    for x in out.iter_mut() {
        *x += (noise_rms * n.gauss()) as f32;
    }
    out
}

fn spectrogram(samples: &[f32], band: Band, block: usize, levels: &mut [u8], front: &mut FrontEnd) {
    let mut s = Spectrogram::new(front, levels, band).unwrap();
    for chunk in samples.chunks(block) {
        s.feed(chunk);
    }
}

/// Mean seconds per hop of feeding a whole slot `reps` times.
fn feed_time(samples: &[f32], block: usize, reps: u32) -> f64 {
    let band = Band::DEFAULT;
    let mut front = Box::new(FrontEnd::new());
    let mut levels = vec![0u8; band.spectrogram_len()];
    let t = Instant::now();
    for _ in 0..reps {
        spectrogram(black_box(samples), band, block, &mut levels, &mut front);
    }
    t.elapsed().as_secs_f64() / f64::from(reps) / ROWS as f64
}

fn main() {
    let band = Band::DEFAULT;

    // 1. feed cost per hop.
    let symbols0 = encode("CQ K1ABC FN42", &mut NoHashes).unwrap().symbols;
    let slot0 = make_slot(&symbols0, 1_500.0, -10.0, NOISE_RMS, 1);
    let t8 = feed_time(&slot0, SLOT_SAMPLES, 20);
    let t8b = feed_time(&slot0, 480, 20);
    println!(
        "feed per 80 ms hop (100–3100 Hz): {:.1} µs (whole slot), {:.1} µs (480-sample blocks)",
        t8 * 1e6,
        t8b * 1e6
    );
    println!("  = {:.3}% of real time", t8 / 0.08 * 100.0);

    // 2. clipping of levels against input level.
    println!("levels at the floor / top, white noise only:");
    let mut front = Box::new(FrontEnd::new());
    let mut lv8 = vec![0u8; band.spectrogram_len()];
    for dbfs in [-110.0f64, -100.0, -90.0, -80.0, -60.0, -40.0, -20.0, -10.0] {
        let rms = 10f64.powf(dbfs / 20.0);
        let mut n = Noise(7);
        let noise: Vec<f32> = (0..SLOT_SAMPLES)
            .map(|_| (rms * n.gauss()) as f32)
            .collect();
        spectrogram(&noise, band, SLOT_SAMPLES, &mut lv8, &mut front);
        let floor = lv8.iter().filter(|&&v| v == 0).count() as f64 / lv8.len() as f64;
        let top = lv8.iter().filter(|&&v| v == 255).count() as f64 / lv8.len() as f64;
        let mean = lv8.iter().map(|&v| f64::from(v)).sum::<f64>() / lv8.len() as f64;
        println!("  noise {dbfs:>5} dBFS RMS: floor {:6.3}%  top {:6.3}%  mean level {mean:6.1} ({:.1} dB)",
            floor * 100.0, top * 100.0, mean / 2.0 - 127.5);
    }

    println!(
        "{{\"profile\": {{\"feed_per_hop_u8\": {t8:.3e}, \"feed_per_hop_u8_blocks\": {t8b:.3e}, \
         \"front_end_bytes\": {}, \"spectrogram_u8_bytes\": {}, \"spectrogram_small_u8_bytes\": {}}}}}",
        std::mem::size_of::<FrontEnd>(),
        band.spectrogram_len(),
        Band::SMALL.spectrogram_len()
    );
}
