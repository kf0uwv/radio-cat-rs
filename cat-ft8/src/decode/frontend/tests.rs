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

extern crate std;

use std::vec;
use std::vec::Vec;

use super::*;
use crate::message::NoHashes;
use crate::symbols::COSTAS;
use crate::waveform::{synthesize, Placement};

/// splitmix64 → Gaussian pairs by Box–Muller (tests only: f64 and libm).
struct Noise(u64);

impl Noise {
    fn uniform(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64 + 1e-300
    }
    fn gauss(&mut self) -> f32 {
        let (u, v) = (self.uniform(), self.uniform());
        (libm::sqrt(-2.0 * libm::log(u)) * libm::cos(2.0 * core::f64::consts::PI * v)) as f32
    }
}

/// A slot with `message` at `base_hz`, `dt`, amplitude `amp`, plus white
/// noise of RMS `noise_rms` (seeded).
fn slot(message: &str, base_hz: f32, dt: f32, amp: f32, noise_rms: f32, seed: u64) -> Vec<f32> {
    let symbols = crate::encode(message, &mut NoHashes).unwrap().symbols;
    let mut out = vec![0.0f32; SLOT_SAMPLES];
    let mut p = Placement::slot(base_hz, dt).unwrap();
    p.amplitude = amp;
    synthesize(&symbols, &p, &mut out).unwrap();
    let mut n = Noise(seed);
    for x in out.iter_mut() {
        *x += noise_rms * n.gauss();
    }
    out
}

fn spectrogram_of(samples: &[f32], band: Band, block: usize) -> (Vec<u8>, usize) {
    let mut front = FrontEnd::new();
    let mut levels = vec![0u8; band.spectrogram_len()];
    let rows;
    {
        let mut s = Spectrogram::new(&mut front, &mut levels, band).unwrap();
        for chunk in samples.chunks(block.max(1)) {
            s.feed(chunk);
        }
        rows = s.rows_filled();
    }
    (levels, rows)
}

#[test]
fn geometry_constants() {
    assert_eq!(ROWS, 184);
    assert_eq!(Band::DEFAULT.base_bins(), 961);
    assert_eq!(Band::DEFAULT.stored_bins(), 976);
    assert_eq!(Band::DEFAULT.spectrogram_len(), 184 * 976);
    assert_eq!(Band::SMALL.stored_bins(), 880);
    assert_eq!(Band::from_hz(100, 3100), Ok(Band::DEFAULT));
    assert_eq!(Band::from_hz(200, 2900), Ok(Band::SMALL));
    assert_eq!(Band::DEFAULT.range_hz(), (100.0, 3100.0));
    assert_eq!(Band::from_hz(101, 3100), Err(Error::InvalidBand));
    assert_eq!(Band::from_hz(0, 3100), Err(Error::InvalidBand));
    assert_eq!(Band::from_hz(3100, 100), Err(Error::InvalidBand));
    assert_eq!(Band::from_hz(100, 5975), Err(Error::InvalidBand)); // reaches Nyquist
    assert_eq!(core::mem::size_of::<FrontEnd>(), 30_720);
}

#[test]
fn level_scales() {
    assert_eq!(level_of(0.0), 255);
    // −10 dB = log2 power −3.3219: 20 steps below the top for u8.
    let l2 = -10.0 / 3.010_3;
    assert_eq!(level_of(l2), 235);
    assert_eq!(level_of(-100.0), 0); // the floor
    assert_eq!(level_of(10.0), 255); // clamped
    let mut last = 0;
    for k in 0..2_000 {
        let v = u32::from(level_of(-45.0 + k as f32 * 0.02));
        assert!(v >= last);
        last = v;
    }
}

#[test]
fn a_tone_lands_in_its_bin() {
    // 1 503.125 Hz = bin 481, full scale; noise-free.
    let tone: Vec<f32> = (0..SLOT_SAMPLES)
        .map(|n| libm::cos(2.0 * core::f64::consts::PI * 1_503.125 * n as f64 / 12_000.0) as f32)
        .collect();
    let (levels, rows) = spectrogram_of(&tone, Band::DEFAULT, SLOT_SAMPLES);
    assert_eq!(rows, ROWS);
    let width = Band::DEFAULT.stored_bins();
    for r in [0, 50, 183] {
        let row = &levels[r * width..(r + 1) * width];
        let (best, &top) = row
            .iter()
            .enumerate()
            .max_by_key(|&(i, &v)| (v, core::cmp::Reverse(i)))
            .unwrap();
        assert_eq!(best + Band::DEFAULT.first_bin(), 481, "row {r}");
        assert!(top >= 254, "row {r}: {top}");
    }
}

#[test]
fn row_origin_puts_nominal_symbol_centres_on_rows_5_7_9() {
    // A signal at the nominal start: the Costas symbol j is strongest on
    // its own tone at row 5 + 2j, and stronger there than half a symbol
    // either side (where the neighbouring symbols' tones share the window).
    let base = 1_500.0; // bin 480
    let samples = slot("CQ K1ABC FN42", base, 0.0, 0.5, 0.0, 1);
    let (levels, _) = spectrogram_of(&samples, Band::DEFAULT, 4_096);
    let width = Band::DEFAULT.stored_bins();
    let at =
        |row: usize, tone: u8| u32::from(levels[row * width + (480 - 32) + 2 * usize::from(tone)]);
    for (j, &c) in COSTAS.iter().enumerate() {
        let row = 5 + 2 * j;
        let best = (0..8u8).max_by_key(|&t| at(row, t)).unwrap();
        assert_eq!(best, c, "symbol {j}");
        if j > 0 && j < 6 {
            assert!(
                at(row, c) > at(row - 1, c) && at(row, c) > at(row + 1, c),
                "symbol {j}"
            );
        }
    }
}

#[test]
fn rows_do_not_depend_on_block_sizes() {
    let samples = slot("K1ABC W9XYZ -11", 812.5, 0.3, 0.1, 0.03, 7);
    let (whole, _) = spectrogram_of(&samples, Band::DEFAULT, SLOT_SAMPLES);
    for block in [1, 7, 960, 1_921, 4_096] {
        let (got, rows) = spectrogram_of(&samples, Band::DEFAULT, block);
        assert_eq!(rows, ROWS);
        assert!(got == whole, "block {block}");
    }
    let (small, _) = spectrogram_of(&samples, Band::SMALL, SLOT_SAMPLES);
    let (small_333, _) = spectrogram_of(&samples, Band::SMALL, 333);
    assert!(small_333 == small);
}

#[test]
fn skip_and_non_finite_samples_are_zeros_and_counted() {
    let mut samples = slot("CQ K1ABC FN42", 1_000.0, 0.0, 0.3, 0.03, 3);
    let mut zeroed = samples.clone();
    for x in &mut zeroed[50_000..52_000] {
        *x = 0.0;
    }
    let (want, _) = spectrogram_of(&zeroed, Band::DEFAULT, SLOT_SAMPLES);

    // Skipping the same stretch.
    let mut front = FrontEnd::new();
    let mut levels = vec![0u8; Band::DEFAULT.spectrogram_len()];
    {
        let mut s = Spectrogram::new(&mut front, &mut levels, Band::DEFAULT).unwrap();
        s.feed(&samples[..50_000]);
        assert_eq!(s.skip(2_000), 2_000);
        s.feed(&samples[52_000..]);
        assert_eq!(s.skipped_samples(), 2_000);
        assert_eq!(s.sanitised_samples(), 0);
        assert_eq!(s.rows_filled(), ROWS);
    }
    assert!(levels == want);

    // Non-finite samples in the same stretch.
    samples[50_000] = f32::NAN;
    for x in &mut samples[50_001..52_000] {
        *x = if x.is_sign_negative() {
            f32::NEG_INFINITY
        } else {
            f32::INFINITY
        };
    }
    let mut front = FrontEnd::new();
    let mut levels = vec![0u8; Band::DEFAULT.spectrogram_len()];
    {
        let mut s = Spectrogram::new(&mut front, &mut levels, Band::DEFAULT).unwrap();
        s.feed(&samples);
        assert_eq!(s.sanitised_samples(), 2_000);
    }
    assert!(levels == want);
}

#[test]
fn i16_input_equals_the_scaled_f32_input() {
    let samples = slot("CQ K1ABC FN42", 1_500.0, 0.0, 0.3, 0.03, 9);
    let ints: Vec<i16> = samples
        .iter()
        .map(|&x| (x * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16)
        .collect();
    let floats: Vec<f32> = ints
        .iter()
        .map(|&v| f32::from(v) * (1.0 / 32_768.0))
        .collect();
    let (want, _) = spectrogram_of(&floats, Band::DEFAULT, SLOT_SAMPLES);
    let mut front = FrontEnd::new();
    let mut levels = vec![0u8; Band::DEFAULT.spectrogram_len()];
    {
        let mut s = Spectrogram::new(&mut front, &mut levels, Band::DEFAULT).unwrap();
        for chunk in ints.chunks(1_000) {
            s.feed_i16(chunk);
        }
    }
    assert!(levels == want);
}

#[test]
fn slot_end_partial_slots_and_start_slot() {
    let samples = slot("CQ K1ABC FN42", 1_500.0, 0.0, 0.3, 0.03, 5);
    let mut front = FrontEnd::new();
    let mut levels = vec![0u8; Band::DEFAULT.spectrogram_len()];
    let mut s = Spectrogram::new(&mut front, &mut levels, Band::DEFAULT).unwrap();
    // Partial: rows complete as their windows fill.
    assert_eq!(s.feed(&samples[..100_000]), 100_000);
    assert_eq!(s.rows_filled(), (100_000 - ROW_ORIGIN - 3_840) / HOP + 1);
    // Past the end: only the slot's samples are taken.
    let mut extra = samples.clone();
    extra.extend_from_slice(&samples[..5_000]);
    assert_eq!(s.feed(&extra[100_000..]), SLOT_SAMPLES - 100_000);
    assert_eq!(s.samples_consumed(), SLOT_SAMPLES);
    assert_eq!(s.rows_filled(), ROWS);
    assert_eq!(s.feed(&samples[..10]), 0);
    assert!(s.row(ROWS).is_none());
    let row_17: Vec<u8> = s.row(17).unwrap().to_vec();
    // A new slot starts from nothing and reproduces the same rows.
    s.start_slot();
    assert_eq!((s.rows_filled(), s.samples_consumed()), (0, 0));
    s.feed(&samples);
    assert_eq!(s.row(17), Some(&row_17[..]));
}

#[test]
fn rows_are_read_through_a_shared_reference() {
    let samples = slot("CQ K1ABC FN42", 1_500.0, 0.0, 0.3, 0.03, 5);
    let mut front = FrontEnd::new();
    let mut levels = vec![0u8; Band::DEFAULT.spectrogram_len()];
    let mut s = Spectrogram::new(&mut front, &mut levels, Band::DEFAULT).unwrap();
    s.feed(&samples[..20_000]);
    let shared: &Spectrogram<'_> = &s;
    let filled = shared.rows_filled();
    assert!(filled > 0 && shared.row(filled - 1).is_some() && shared.row(filled).is_none());
}

#[test]
fn early_samples_are_counted_and_huge_ones_clamped() {
    // Non-finite and huge samples before row 0 are counted too.
    let mut samples = slot("CQ K1ABC FN42", 1_500.0, 0.0, 0.3, 0.03, 11);
    samples[10] = f32::NAN;
    samples[20] = 1.0e30;
    samples[100_000] = -1.0e9;
    samples[100_001] = 70_000.0;
    let mut clipped = samples.clone();
    clipped[10] = 0.0;
    clipped[20] = INPUT_LIMIT; // before row 0: no effect on the rows anyway
    clipped[100_000] = -INPUT_LIMIT;
    clipped[100_001] = INPUT_LIMIT;
    let (want, _) = spectrogram_of(&clipped, Band::DEFAULT, SLOT_SAMPLES);
    let mut front = FrontEnd::new();
    let mut levels = vec![0u8; Band::DEFAULT.spectrogram_len()];
    {
        let mut s = Spectrogram::new(&mut front, &mut levels, Band::DEFAULT).unwrap();
        for chunk in samples.chunks(77) {
            s.feed(chunk);
        }
        assert_eq!(s.sanitised_samples(), 1);
        assert_eq!(s.clamped_samples(), 3);
    }
    assert!(levels == want);
}

#[test]
fn floor_and_top_levels_are_counted() {
    let mut front = FrontEnd::new();
    let mut levels = vec![0u8; Band::DEFAULT.spectrogram_len()];
    let mut s = Spectrogram::new(&mut front, &mut levels, Band::DEFAULT).unwrap();
    // Silence: every level is at the floor.
    s.skip(SLOT_SAMPLES);
    assert_eq!(s.floored_levels(), Band::DEFAULT.spectrogram_len());
    assert_eq!(s.top_levels(), 0);
    // A tone 6 dB over full scale, centred in bin 481: its bin is at the
    // top in every row.
    s.start_slot();
    let tone: Vec<f32> = (0..SLOT_SAMPLES)
        .map(|n| {
            2.0 * libm::cos(2.0 * core::f64::consts::PI * 1_503.125 * n as f64 / 12_000.0) as f32
        })
        .collect();
    s.feed(&tone);
    assert!(s.top_levels() >= ROWS, "{}", s.top_levels());
    assert_eq!(s.row(0).unwrap()[481 - 32], 255);
}

#[test]
fn buffers_are_checked() {
    let mut front = FrontEnd::new();
    let mut levels = vec![0u8; Band::DEFAULT.spectrogram_len() - 1];
    assert!(matches!(
        Spectrogram::new(&mut front, &mut levels, Band::DEFAULT),
        Err(Error::BufferTooSmall)
    ));
}
