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
//! cargo run --release -p cat-ft8 --features profile --example frontend_bench -- [SLOTS_PER_SNR]
//! ```
//!
//! 1. `feed` cost per 80 ms hop (whole slot and 480-sample blocks), for
//!    8-bit and 16-bit levels.
//! 2. Level clipping: the share of 8-bit levels at the floor or the top for
//!    white noise at input levels from −110 to −10 dBFS.
//! 3. Level width: with a signal at the nominal start on the bin grid and
//!    white noise at a given SNR (2500 Hz), the share of the 58 data
//!    symbols whose transmitted tone has the highest level among the
//!    eight (ties count as wrong and are also counted separately), from
//!    8-bit levels and 16-bit levels (the 16-bit form is a measurement
//!    baseline only; it gave the same decisions as unquantised power).
//!
//! The last line is one `{"profile": {...}}` JSON object.

use std::hint::black_box;
use std::time::Instant;

use cat_ft8::decode::{Band, FrontEnd, Level, Spectrogram, ROWS};
use cat_ft8::message::NoHashes;
use cat_ft8::symbols::data_index;
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

fn spectrogram<L: Level>(
    samples: &[f32],
    band: Band,
    block: usize,
    levels: &mut [L],
    front: &mut FrontEnd,
) {
    let mut s = Spectrogram::new(front, levels, band).unwrap();
    for chunk in samples.chunks(block) {
        s.feed(chunk);
    }
}

/// Mean seconds per hop of feeding a whole slot `reps` times.
fn feed_time<L: Level>(samples: &[f32], block: usize, reps: u32) -> f64 {
    let band = Band::DEFAULT;
    let mut front = Box::new(FrontEnd::new());
    let mut levels = vec![L::default(); band.spectrogram_len()];
    let t = Instant::now();
    for _ in 0..reps {
        spectrogram(black_box(samples), band, block, &mut levels, &mut front);
    }
    t.elapsed().as_secs_f64() / f64::from(reps) / ROWS as f64
}

/// Hard-decision accuracy over the data symbols: (correct, ties, total,
/// tie-split correct). Tie-split credits a k-way tie that includes the
/// transmitted tone with 1/k (the expected accuracy of breaking ties at
/// random).
fn accuracy(
    symbols: &[u8; SYMBOL_COUNT],
    base_bin: usize,
    level: impl Fn(usize, usize) -> f64,
) -> (u32, u32, u32, f64) {
    let (mut ok, mut ties, mut total, mut split) = (0, 0, 0, 0.0);
    for (k, &tone) in symbols.iter().enumerate() {
        if data_index(k).is_none() {
            continue;
        }
        let row = 5 + 2 * k;
        let vals: Vec<f64> = (0..8).map(|t| level(row, base_bin + 2 * t)).collect();
        let truth = vals[usize::from(tone)];
        let others_max = (0..8)
            .filter(|&t| t != usize::from(tone))
            .map(|t| vals[t])
            .fold(f64::MIN, f64::max);
        if truth > others_max {
            ok += 1;
            split += 1.0;
        } else if truth == others_max {
            ties += 1;
            split += 1.0 / vals.iter().filter(|&&v| v == truth).count() as f64;
        }
        total += 1;
    }
    (ok, ties, total, split)
}

fn main() {
    let per_snr: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(40);
    let band = Band::DEFAULT;

    // 1. feed cost per hop.
    let symbols0 = encode("CQ K1ABC FN42", &mut NoHashes).unwrap().symbols;
    let slot0 = make_slot(&symbols0, 1_500.0, -10.0, NOISE_RMS, 1);
    let t8 = feed_time::<u8>(&slot0, SLOT_SAMPLES, 20);
    let t8b = feed_time::<u8>(&slot0, 480, 20);
    let t16 = feed_time::<u16>(&slot0, SLOT_SAMPLES, 20);
    println!("feed per 80 ms hop (100–3100 Hz): 8-bit {:.1} µs (whole slot), {:.1} µs (480-sample blocks); 16-bit {:.1} µs",
        t8 * 1e6, t8b * 1e6, t16 * 1e6);
    println!("  = {:.3}% of real time (8-bit)", t8 / 0.08 * 100.0);

    // 2. clipping of 8-bit levels against input level.
    println!("8-bit levels at the floor / top, white noise only:");
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

    // 3. level width: hard-symbol accuracy.
    let vectors = include_str!("../tests/data/encoder-vectors.jsonl");
    let messages: Vec<String> = vectors
        .lines()
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["message"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .filter(|m| encode(m, &mut NoHashes).is_ok())
        .collect();
    let mut lv16 = vec![0u16; band.spectrogram_len()];
    let width = band.stored_bins();
    println!("data-symbol hard decisions correct (ties), {per_snr} slots per SNR:");
    println!("  SNR dB    8-bit (ties) tie-split            16-bit (ties)");
    let mut diffs = Vec::new();
    for snr in [-26.0f64, -24.0, -22.0, -20.0, -18.0, -16.0] {
        let mut acc = [(0u32, 0u32, 0u32, 0.0f64); 2];
        for i in 0..per_snr {
            let msg = &messages[(i as usize * 7 + snr as usize) % messages.len()];
            let symbols = encode(msg, &mut NoHashes).unwrap().symbols;
            // A base bin on the grid inside the band, varied per slot.
            let base_bin = 40 + ((i as usize * 97) % 900);
            let slot = make_slot(&symbols, base_bin as f32 * 3.125, snr, NOISE_RMS, 1_000 + i);
            spectrogram(&slot, band, SLOT_SAMPLES, &mut lv8, &mut front);
            spectrogram(&slot, band, SLOT_SAMPLES, &mut lv16, &mut front);
            let rel = base_bin - band.first_bin();
            let a8 = accuracy(&symbols, rel, |r, b| f64::from(lv8[r * width + b]));
            let a16 = accuracy(&symbols, rel, |r, b| f64::from(lv16[r * width + b]));
            for (slot_acc, a) in acc.iter_mut().zip([a8, a16]) {
                slot_acc.0 += a.0;
                slot_acc.1 += a.1;
                slot_acc.2 += a.2;
                slot_acc.3 += a.3;
            }
        }
        type Acc = (u32, u32, u32, f64);
        let pct = |a: Acc| 100.0 * f64::from(a.0) / f64::from(a.2);
        let tie = |a: Acc| 100.0 * f64::from(a.1) / f64::from(a.2);
        let split = |a: Acc| 100.0 * a.3 / f64::from(a.2);
        println!(
            "  {snr:>6}   {:6.2}% ({:4.2}%) split {:6.2}%   {:6.2}% ({:4.2}%)",
            pct(acc[0]),
            tie(acc[0]),
            split(acc[0]),
            pct(acc[1]),
            tie(acc[1])
        );
        diffs.push((
            pct(acc[1]) - pct(acc[0]),
            split(acc[1]) - split(acc[0]),
            f64::from(acc[0].2),
        ));
    }
    let n = diffs.len() as f64;
    let strict = diffs.iter().map(|d| d.0).sum::<f64>() / n;
    let split = diffs.iter().map(|d| d.1).sum::<f64>() / n;
    // Binomial standard error of one accuracy figure near 60%, per SNR.
    let se = (0.6 * 0.4 / diffs[0].2).sqrt() * 100.0;
    println!(
        "mean gain of 16-bit over 8-bit: {strict:+.3} points counting ties as wrong, \
         {split:+.3} points with ties split (one figure's standard error ≈ {se:.2} points)"
    );

    println!(
        "{{\"profile\": {{\"feed_per_hop_u8\": {t8:.3e}, \"feed_per_hop_u8_blocks\": {t8b:.3e}, \"feed_per_hop_u16\": {t16:.3e}, \
         \"front_end_bytes\": {}, \"spectrogram_u8_bytes\": {}, \"spectrogram_u16_bytes\": {}, \"spectrogram_small_u8_bytes\": {}}}}}",
        std::mem::size_of::<FrontEnd>(),
        band.spectrogram_len(),
        band.spectrogram_len() * 2,
        Band::SMALL.spectrogram_len()
    );
}
