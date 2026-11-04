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
fn sync_scores_stay_in_the_front_end_up_to_the_fft_cells() {
    use crate::decode::Decoder;
    let bins = |base_bins: u16| Band {
        lo_bin: 32,
        hi_bin: 32 + base_bins - 1,
    };
    assert_eq!(FftBuffer::CELLS, 3 * 1_280);
    assert_eq!(Decoder::sync_scratch_len(bins(1_280)), 0);
    assert_eq!(Decoder::sync_scratch_len(bins(1_281)), 3 * 1_281);
    // In whole multiples of 25 Hz, the widths `Band::from_hz` takes.
    let wide = |hz: u32| Band::from_hz(100, 100 + hz).unwrap();
    assert_eq!(Decoder::sync_scratch_len(wide(3_975)), 0);
    assert_eq!(Decoder::sync_scratch_len(wide(4_000)), 3 * 1_281);
    assert_eq!(Decoder::sync_scratch_len(Band::DEFAULT), 0);
    assert_eq!(Decoder::sync_scratch_len(Band::SMALL), 0);
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
fn level_guess_constants_follow_from_db_per_log2() {
    // 2 · DB_PER_LOG2 levels a unit of log2 (the f32 constant, exactly in
    // f64); 2²³ units of the bits an octave; both scaled by 2⁴⁰.
    let levels = 2.0 * f64::from(DB_PER_LOG2);
    let scale = libm::round(levels * 2f64.powi(40 - 23)) as i64;
    let offset = libm::round((255.5 - 127.0 * levels - 0.02) * 2f64.powi(40)) as i64;
    assert_eq!(scale, LEVEL_GUESS_SCALE);
    assert_eq!(offset, LEVEL_GUESS_OFFSET);
}

/// The level of `bits` both ways: by the logarithm and by the steps.
fn levels_both_ways(bits: u32) -> (u8, u8) {
    let p = f32::from_bits(bits);
    (level_of(log2_power(p)), level_by_steps(p))
}

#[test]
fn level_steps_give_the_level_near_every_step_and_across_the_range() {
    // Each step's own bits and those beside it, every 65 537th bit
    // pattern (both signs, NaNs and infinities among them), and the ends.
    let near = LEVEL_STEPS[..255]
        .iter()
        .flat_map(|&s| s - 3..=s + 3)
        .chain((0..=u32::MAX).step_by(65_537))
        .chain([
            0,
            1,
            POWER_FLOOR.to_bits(),
            f32::MAX.to_bits(),
            0x7f80_0000,
            0xff80_0000,
            u32::MAX,
        ]);
    for bits in near {
        let (want, got) = levels_both_ways(bits);
        assert_eq!(got, want, "bits {bits:#010x}");
    }
}

#[test]
#[ignore = "exhaustive: every f32 (about 4.3 · 10⁹), in threads; run with --release"]
fn level_steps_give_every_level_everywhere() {
    let threads = 16u64;
    let span = (1u64 << 32) / threads;
    let bad: u64 = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                scope.spawn(move || {
                    (t * span..(t + 1) * span)
                        .filter(|&b| {
                            let (want, got) = levels_both_ways(b as u32);
                            want != got
                        })
                        .count() as u64
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).sum()
    });
    assert_eq!(bad, 0);
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
fn rows_are_the_levels_of_each_window() {
    // Each row is the levels of its own window of the slot, as the
    // transform of the joined window gives them: so the window's slide (or,
    // where the transform takes one lane and in the tests built with
    // `cat_ft8_test_group_one`, its ring of quarters) holds every row's
    // samples in order. Blocks of 1 921 samples end within a quarter.
    let samples = slot("K1ABC W9XYZ -11", 812.5, 0.3, 0.1, 0.03, 7);
    for band in [Band::DEFAULT, Band::SMALL] {
        let (levels, rows) = spectrogram_of(&samples, band, 1_921);
        assert_eq!(rows, ROWS);
        let width = band.stored_bins();
        let mut buf = crate::decode::fft::FftBuffer::new();
        let mut p = vec![0.0f32; width];
        for r in 0..rows {
            let window: &[f32; WINDOW] = samples[ROW_ORIGIN + r * HOP..][..WINDOW]
                .try_into()
                .unwrap();
            crate::decode::fft::power_spectrum(window, &mut buf, band.first_bin(), &mut p);
            let want: Vec<u8> = p.iter().map(|&x| level_of(log2_power(x))).collect();
            assert!(
                levels[r * width..(r + 1) * width] == want[..],
                "{band:?} row {r}"
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
        assert_eq!(s.clamped_samples(), 0);
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
        assert_eq!((s.sanitised_samples(), s.clamped_samples()), (0, 0));
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
fn feed_equals_feeding_the_classified_samples() {
    // Samples at and just beyond ±INPUT_LIMIT, the largest finite values,
    // infinities, NaNs, signed zeros and subnormals, scattered so that
    // they fall before the row origin, inside blocks and on block edges:
    // the levels and counters equal those of feeding the samples that
    // `classify` returns, one at a time, which `feed` copies unchanged.
    // The reference goes through `feed` itself (every one of its samples
    // passes the block check, so it takes the copy). What is independent
    // here is `classify`, called directly for the values and the counts.
    // The levels of in-range input are anchored separately by the
    // transform's and powers' reference tests in `fft`, and the zeros by
    // `skip_and_non_finite_samples_are_zeros_and_counted`.
    let mut samples = slot("CQ K1ABC FN42", 1_500.0, 0.0, 0.3, 0.03, 13);
    let above = f32::from_bits(INPUT_LIMIT.to_bits() + 1);
    let specials = [
        INPUT_LIMIT,
        -INPUT_LIMIT,
        above,
        -above,
        f32::MAX,
        f32::MIN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        -f32::NAN,
        f32::from_bits(0x7f80_0001),
        -0.0,
        f32::from_bits(1),
    ];
    for (k, at) in [
        0, 7, 239, 240, 241, 3_839, 4_079, 4_080, 50_000, 99_999, 179_999,
    ]
    .into_iter()
    .enumerate()
    {
        samples[at] = specials[k % specials.len()];
    }
    for (k, x) in samples[120_000..120_000 + 4 * specials.len()]
        .iter_mut()
        .enumerate()
    {
        *x = specials[k / 4];
    }
    let (mut sanitised, mut clamped) = (0, 0);
    let classified: Vec<f32> = samples
        .iter()
        .map(|&x| classify(x, &mut sanitised, &mut clamped))
        .collect();
    assert!(sanitised > 0 && clamped > 0);
    let (want, _) = spectrogram_of(&classified, Band::DEFAULT, 1);
    for block in [1, 77, 240, 960, 3_840, SLOT_SAMPLES] {
        let mut front = FrontEnd::new();
        let mut levels = vec![0u8; Band::DEFAULT.spectrogram_len()];
        {
            let mut s = Spectrogram::new(&mut front, &mut levels, Band::DEFAULT).unwrap();
            for chunk in samples.chunks(block) {
                s.feed(chunk);
            }
            assert_eq!(
                (s.sanitised_samples(), s.clamped_samples()),
                (sanitised, clamped)
            );
        }
        assert!(levels == want, "block {block}");
    }
}

/// The block check ([`within_limit`]) holds exactly where [`classify`]
/// returns the sample unchanged and counts nothing: for the values the
/// front end treats specially and their neighbours, every exponent with
/// both signs, and one sample out of the limit at each place of blocks of
/// several lengths, where [`checked_copy`] counts it and writes what
/// `classify` returns. A block with samples of 2 and more beside small
/// ones, whose bitwise or is past the limit's, is taken whole too.
#[test]
fn the_block_check_is_where_classify_is_the_identity() {
    let above = f32::from_bits(INPUT_LIMIT.to_bits() + 1);
    let mut values = vec![
        0.0,
        -0.0,
        f32::from_bits(1),
        f32::MIN_POSITIVE,
        INPUT_LIMIT,
        -INPUT_LIMIT,
        f32::from_bits(INPUT_LIMIT.to_bits() - 1),
        above,
        -above,
        f32::MAX,
        f32::MIN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        -f32::NAN,
        f32::from_bits(0x7f80_0001),
        f32::from_bits(0x7fff_ffff),
    ];
    for e in 0..256u32 {
        for m in [0u32, 1, 0x40_0000, 0x7f_ffff] {
            let x = f32::from_bits(e << 23 | m);
            values.push(x);
            values.push(-x);
        }
    }
    let identity = |x: f32| {
        let (mut sanitised, mut clamped) = (0, 0);
        let y = classify(x, &mut sanitised, &mut clamped);
        y.to_bits() == x.to_bits() && sanitised + clamped == 0
    };
    let mut out = 0;
    for &x in &values {
        assert_eq!(within_limit(&[x]), identity(x), "{:#x}", x.to_bits());
        out += usize::from(!identity(x));
    }
    assert!(out > 100 && out < values.len() - 100, "{out}");
    assert!(within_limit(&[]));
    for len in [1usize, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 33, 240, 961] {
        let ok: Vec<f32> = (0..len).map(|i| (i as f32 - 3.0) * 0.25).collect();
        assert!(within_limit(&ok), "{len}");
        let mut wide = ok.clone();
        wide[len / 2] = -INPUT_LIMIT;
        assert!(within_limit(&wide), "{len}");
        for at in 0..len {
            for bad in [f32::NAN, f32::INFINITY, -INPUT_LIMIT * 1.5, above] {
                let mut v = ok.clone();
                v[at] = bad;
                assert!(!within_limit(&v), "{len} {at} {bad}");
                let mut copied = vec![1.0f32; len];
                let (sanitised, clamped) = checked_copy(&v, &mut copied);
                assert_eq!(sanitised + clamped, 1, "{len} {at} {bad}");
                let (mut s, mut c) = (0, 0);
                let want: Vec<u32> = v
                    .iter()
                    .map(|&x| classify(x, &mut s, &mut c).to_bits())
                    .collect();
                let got: Vec<u32> = copied.iter().map(|x| x.to_bits()).collect();
                assert_eq!(got, want, "{len} {at} {bad}");
            }
        }
    }
}

/// [`level_counts`] counts the levels at 0 and at 255 as filtering does,
/// over rows of several widths up to [`BINS`], with no
/// level, every level and random mixes at either end.
#[test]
fn level_counts_equal_the_filtered_counts() {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for width in [0usize, 1, 15, 16, 17, 31, 32, 33, 255, 256, 880, 976, BINS] {
        for kind in 0..5 {
            let row: Vec<u8> = (0..width)
                .map(|_| {
                    let r = next();
                    match kind {
                        0 => 0,
                        1 => 255,
                        2 => [0, 127, 255][(r % 3) as usize],
                        3 => (r >> 8) as u8,
                        _ => 1 + (r % 254) as u8,
                    }
                })
                .collect();
            let want = (
                row.iter().filter(|&&l| l == 0).count(),
                row.iter().filter(|&&l| l == u8::MAX).count(),
            );
            assert_eq!(level_counts(&row), want, "width {width} kind {kind}");
        }
    }
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

#[test]
fn feed_i16_equals_feed_of_the_scaled_samples() {
    // A noisy slot quantised to 16 bits, including both extremes.
    let f = slot("CQ K1ABC FN42", 1_000.0, 0.0, 0.2, 0.05, 11);
    let mut ints: Vec<i16> = f
        .iter()
        .map(|&x| (x * 32_768.0).clamp(-32_768.0, 32_767.0) as i16)
        .collect();
    ints[1_000] = i16::MIN;
    ints[2_000] = i16::MAX;
    let scaled: Vec<f32> = ints.iter().map(|&x| f32::from(x) / 32_768.0).collect();
    let (want, _) = spectrogram_of(&scaled, Band::DEFAULT, 1_013);
    let mut front = FrontEnd::new();
    let mut levels = vec![0u8; Band::DEFAULT.spectrogram_len()];
    {
        let mut s = Spectrogram::new(&mut front, &mut levels, Band::DEFAULT).unwrap();
        for chunk in ints.chunks(777) {
            s.feed_i16(chunk);
        }
        assert_eq!(s.rows_filled(), ROWS);
    }
    assert!(levels == want);
}
