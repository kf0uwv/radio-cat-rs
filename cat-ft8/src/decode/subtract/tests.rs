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

use std::vec::Vec;

use super::*;
use crate::decode::frontend::INPUT_LIMIT;
use crate::decode::testutil::{
    add_noise, add_signal, amplitude_for, empty_slot, symbols_of, Noise,
};
use crate::message::NoHashes;
use crate::{pack, Error, SLOT_SAMPLES};

const MESSAGE: &str = "CQ K1ABC FN42";

/// A decode of [`MESSAGE`] as the decoder might report it: frequency and
/// time a little off the truth.
fn decode_near(freq_hz: f32, dt: f32) -> Decode {
    Decode {
        payload: pack(MESSAGE, &mut NoHashes).unwrap(),
        freq_hz: freq_hz + 0.37,
        dt: dt - 0.011,
        ..Decode::EMPTY
    }
}

/// A slot with [`MESSAGE`] at `freq_hz` and `dt` (whole-sample start),
/// peak amplitude `amp`.
fn slot_with(freq_hz: f32, dt: f32, amp: f32) -> Vec<f32> {
    let mut s = empty_slot();
    add_signal(&mut s, &symbols_of(MESSAGE), freq_hz, dt, amp);
    s
}

/// [`subtract`] with scratch storage whose contents are left from other
/// calls (not cleared): the result must not depend on them.
fn sub(
    audio: &mut [f32],
    decode: &Decode,
    skip: usize,
    config: &SubtractConfig,
) -> Result<Subtraction, Error> {
    std::thread_local! {
        static SCRATCH: core::cell::RefCell<Vec<u8>> =
            core::cell::RefCell::new(std::vec![0; SCRATCH_BYTES]);
    }
    SCRATCH.with(|s| subtract(audio, decode, skip, config, &mut s.borrow_mut()))
}

fn energy(x: &[f32]) -> f64 {
    x.iter().map(|&v| f64::from(v) * f64::from(v)).sum()
}

fn db(a: f64, b: f64) -> f64 {
    10.0 * libm::log10(a / b)
}

#[test]
fn noiseless_signal_is_suppressed_below_the_floor() {
    // Tone 0 as low as a band may start (25 Hz) and below 100 Hz too.
    for (freq, dt) in [
        (1_500.0f32, 0.2f32),
        (212.5, -0.9),
        (2_987.0, 1.3),
        (25.0, 0.0),
        (60.0, -0.4),
    ] {
        let clean = slot_with(freq, dt, 0.1);
        let mut audio = clean.clone();
        let r = sub(
            &mut audio,
            &decode_near(freq, dt),
            0,
            &SubtractConfig::DEFAULT,
        )
        .unwrap();
        let left = db(energy(&audio), energy(&clean));
        assert!(left < -40.0, "{freq} Hz, {dt} s: {left:.1} dB");
        assert_eq!(r.start, Placement::slot(freq, dt).unwrap().start);
        assert!((r.freq_hz - freq).abs() < 0.01, "{r:?}");
        assert!(r.drift_hz_per_s.abs() < 0.01, "{r:?}");
        assert!((r.dt - dt).abs() < 0.5 / 12_000.0, "{r:?}");
        let first = r.start.max(0) as usize;
        let last = (r.start + WAVEFORM_SAMPLES as i64).min(SLOT_SAMPLES as i64) as usize;
        assert_eq!(r.samples, last - first);
        assert!(r.residue_db().unwrap() < -40.0, "{r:?}");
        assert!(r.left_db().unwrap() < -40.0, "{r:?}");
    }
}

/// FNV-1a 64 over the residual's bits and every estimate.
fn hash(audio: &[f32], r: &Subtraction) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for x in audio {
        eat(&x.to_bits().to_le_bytes());
    }
    eat(&r.start.to_le_bytes());
    for v in [
        r.dt,
        r.freq_hz,
        r.drift_hz_per_s,
        r.energy,
        r.residue,
        r.residue_noise,
    ] {
        eat(&v.to_bits().to_le_bytes());
    }
    eat(&(r.samples as u64).to_le_bytes());
    h
}

/// A noisy slot with [`MESSAGE`] at +5 dB and a second signal (−3 dB)
/// overlapping it.
fn noisy_slot() -> Vec<f32> {
    let mut s = slot_with(1_234.5, 0.31, amplitude_for(5.0));
    add_signal(
        &mut s,
        &symbols_of("K1ABC W9XYZ RR73"),
        1_251.0,
        0.05,
        amplitude_for(-3.0),
    );
    add_noise(&mut s, crate::decode::testutil::NOISE_RMS, 77);
    s
}

#[test]
fn noisy_slot_is_suppressed_to_the_noise() {
    let freq = 1_234.5;
    let clean = slot_with(freq, 0.31, amplitude_for(5.0));
    let audio = noisy_slot();
    let mut residual = audio.clone();
    let r = sub(
        &mut residual,
        &decode_near(freq, 0.31),
        0,
        &SubtractConfig::DEFAULT,
    )
    .unwrap();
    // What is left of the signal: the residual less everything else.
    let other: Vec<f32> = audio.iter().zip(&clean).map(|(a, c)| a - c).collect();
    let err: Vec<f32> = residual.iter().zip(&other).map(|(a, o)| a - o).collect();
    let left = db(energy(&err), energy(&clean));
    // The overlapping signal (16.5 Hz up, 8 dB weaker) pulls the
    // estimates a little: measured −18.7 dB, 6 samples early.
    assert!(left < -15.0, "{left:.1} dB {r:?}");
    let start = Placement::slot(freq, 0.31).unwrap().start;
    assert!((r.start - start).abs() <= 8, "{r:?}");
    assert!((r.freq_hz - freq).abs() < 0.05, "{r:?}");
}

#[test]
fn a_slot_cut_short_is_handled() {
    let (freq, dt) = (1_000.0, 0.4);
    let clean = slot_with(freq, dt, 0.1);
    // Cut at 13.5 s: the waveform's last 1.86 s are missing.
    let cut = 162_000;
    let mut audio = clean[..cut].to_vec();
    let r = sub(
        &mut audio,
        &decode_near(freq, dt),
        0,
        &SubtractConfig::DEFAULT,
    )
    .unwrap();
    assert_eq!(r.start, Placement::slot(freq, dt).unwrap().start);
    assert_eq!(r.samples, cut - r.start as usize);
    let left = db(energy(&audio), energy(&clean[..cut]));
    assert!(left < -40.0, "{left:.1} dB {r:?}");
    // A buffer that ends before the waveform starts changes nothing.
    let mut early = clean[..6_000].to_vec();
    let r = sub(
        &mut early,
        &decode_near(freq, dt),
        0,
        &SubtractConfig::DEFAULT,
    )
    .unwrap();
    assert_eq!(r.samples, 0);
    assert_eq!(early, clean[..6_000]);
    // Nor does an empty one.
    let r = sub(&mut [], &decode_near(freq, dt), 0, &SubtractConfig::DEFAULT).unwrap();
    assert_eq!(r.samples, 0);
}

#[test]
fn a_segment_with_too_few_samples_takes_the_previous_estimate() {
    // The buffer ends 100 samples into a two-symbol segment: too few for
    // an estimate of its own, so it takes the previous segment's (with
    // none, the amplitude would fall towards zero across the cut and
    // leave about half the signal there).
    let (freq, dt) = (1_000.0, 0.4);
    let clean = slot_with(freq, dt, 0.1);
    let config = SubtractConfig::DEFAULT.with_segment(2 * T);
    let start = Placement::slot(freq, dt).unwrap().start as usize;
    let cut = start + 30 * 2 * T + 100;
    let mut audio = clean[..cut].to_vec();
    let r = sub(&mut audio, &decode_near(freq, dt), 0, &config).unwrap();
    assert_eq!(r.samples, cut - start);
    let tail = db(energy(&audio[cut - 100..]), energy(&clean[cut - 100..cut]));
    assert!(tail < -30.0, "{tail:.1} dB {r:?}");
    let left = db(energy(&audio), energy(&clean[..cut]));
    assert!(left < -40.0, "{left:.1} dB {r:?}");
}

#[test]
fn a_skipped_region_is_neither_read_nor_written() {
    // A signal starting at 0.1 s (sample 1 200) in a slot whose first
    // second is missing (zero).
    let (freq, dt) = (1_700.0, -0.4);
    let mut audio = slot_with(freq, dt, 0.1);
    let skip = 12_000;
    audio[..skip].fill(0.0);
    let clean = audio.clone();
    let r = sub(
        &mut audio,
        &decode_near(freq, dt),
        skip,
        &SubtractConfig::DEFAULT,
    )
    .unwrap();
    assert!(audio[..skip].iter().all(|x| x.to_bits() == 0));
    // Whatever the skipped region holds, the result is bit for bit the
    // same and the region is left as it was.
    for junk in [f32::NAN, f32::INFINITY, 1.0e30, -3.0e4] {
        let mut other = clean.clone();
        other[..skip].fill(junk);
        let ro = sub(
            &mut other,
            &decode_near(freq, dt),
            skip,
            &SubtractConfig::DEFAULT,
        )
        .unwrap();
        assert_eq!(
            hash(&other[skip..], &ro),
            hash(&audio[skip..], &r),
            "{junk}"
        );
        assert!(other[..skip].iter().all(|x| x.to_bits() == junk.to_bits()));
    }
    let start = Placement::slot(freq, dt).unwrap().start;
    assert_eq!(r.start, start);
    assert_eq!(r.samples, start as usize + WAVEFORM_SAMPLES - skip);
    let left = db(energy(&audio), energy(&clean));
    assert!(left < -40.0, "{left:.1} dB {r:?}");
    // Skipping everything changes nothing.
    let mut all = clean.clone();
    let r = sub(
        &mut all,
        &decode_near(freq, dt),
        SLOT_SAMPLES,
        &SubtractConfig::DEFAULT,
    );
    assert_eq!(r.unwrap().samples, 0);
    assert_eq!(all, clean);
}

#[test]
fn non_finite_and_huge_samples_are_taken_as_the_front_end_takes_them() {
    let (freq, dt) = (1_234.5, 0.31);
    let clean = slot_with(freq, dt, amplitude_for(5.0));
    let start = Placement::slot(freq, dt).unwrap().start as usize;
    // One bad sample in a symbol's middle half and one near a symbol
    // boundary, each also as zero (what the front end makes of them).
    let bad = [start + 10 * 1_920 + 700, start + 40 * 1_920 + 10];
    for junk in [f32::NAN, f32::NEG_INFINITY] {
        let mut a = noisy_slot();
        let mut z = noisy_slot();
        for &i in &bad {
            a[i] = junk;
            z[i] = 0.0;
        }
        let ra = sub(&mut a, &decode_near(freq, dt), 0, &SubtractConfig::DEFAULT).unwrap();
        let rz = sub(&mut z, &decode_near(freq, dt), 0, &SubtractConfig::DEFAULT).unwrap();
        assert_eq!(hash(&a, &ra), hash(&z, &rz), "{junk}");
        assert!(a.iter().all(|x| x.is_finite()));
        assert!(ra.samples > 0 && ra.left_db().unwrap() < -15.0, "{ra:?}");
    }
    // A finite sample beyond ±2¹⁶ is clamped to it, as the front end does.
    let mut a = clean.clone();
    let mut z = clean.clone();
    a[bad[0]] = 1.0e30;
    z[bad[0]] = INPUT_LIMIT;
    let ra = sub(&mut a, &decode_near(freq, dt), 0, &SubtractConfig::DEFAULT).unwrap();
    let rz = sub(&mut z, &decode_near(freq, dt), 0, &SubtractConfig::DEFAULT).unwrap();
    assert_eq!(hash(&a, &ra), hash(&z, &rz));
    assert!(a.iter().all(|x| x.is_finite()) && ra.energy.is_finite());
}

#[test]
fn a_clipped_signal_is_mostly_removed() {
    // A strong signal (peak 0.1) clipped at ±0.06 by the receiver, with
    // noise: the clipping's distortion stays, the fundamental goes.
    let (freq, dt) = (1_500.0, 0.0);
    let mut audio = slot_with(freq, dt, 0.1);
    add_noise(&mut audio, crate::decode::testutil::NOISE_RMS, 5);
    for x in &mut audio {
        *x = x.clamp(-0.06, 0.06);
    }
    let before = energy(&audio);
    let r = sub(
        &mut audio,
        &decode_near(freq, dt),
        0,
        &SubtractConfig::DEFAULT,
    )
    .unwrap();
    assert!(audio.iter().all(|x| x.is_finite()));
    assert_eq!(r.start, Placement::slot(freq, dt).unwrap().start);
    assert!((r.freq_hz - freq).abs() < 0.01, "{r:?}");
    // Most of the energy was the signal's (measured −7.3 dB: the
    // clipping's harmonics and the noise stay).
    let left = db(energy(&audio), before);
    assert!(left < -6.0, "{left:.1} dB {r:?}");
}

#[test]
fn deterministic_and_pinned() {
    let d = decode_near(1_234.5, 0.31);
    let mut pins = Vec::new();
    for config in [
        SubtractConfig::DEFAULT,
        SubtractConfig::DEFAULT.with_segment(960).with_drift(false),
        SubtractConfig::DEFAULT.with_segment(WAVEFORM_SAMPLES),
    ] {
        let mut a = noisy_slot();
        let mut b = noisy_slot();
        let ra = sub(&mut a, &d, 0, &config).unwrap();
        let rb = sub(&mut b, &d, 0, &config).unwrap();
        assert_eq!(hash(&a, &ra), hash(&b, &rb));
        pins.push(hash(&a, &ra));
    }
    // Residual bits and every estimate, pinned (new outputs: these change
    // only with the subtraction itself).
    assert_eq!(
        pins,
        [
            0xaee5_ced2_9b40_3427,
            0x8b52_1897_b57e_67fa,
            0xbaca_d495_bda7_b030
        ],
        "{pins:x?}"
    );
}

/// The report's projection and noise term, pinned bit for bit on sums
/// whose cross term matters (random references, not sinusoids: over a
/// real segment `c·s` sums to almost nothing, so a reordering of the
/// cross term rounds the same there and no subtraction pin sees it).
#[test]
fn projection_pinned() {
    let mut r = 0x2545_f491_4f6c_dd1du64;
    let mut uniform = || {
        r ^= r << 13;
        r ^= r >> 7;
        r ^= r << 17;
        (r >> 40) as f32 / (1u64 << 24) as f32 - 0.5
    };
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |v: f32| h = (h ^ u64::from(v.to_bits())).wrapping_mul(0x0000_0100_0000_01b3);
    let mut found = 0;
    for n in 8..72 {
        let mut sums = Sums::default();
        for _ in 0..n {
            let (c, s) = (uniform(), uniform() + 0.3 * uniform());
            let x = 0.7 * c - 0.4 * s + 0.2 * uniform();
            sums.add(x, c, s);
        }
        if let Some((e, noise)) = sums.projection(1) {
            eat(e);
            eat(noise);
            found += 1;
        }
    }
    assert_eq!(found, 64);
    assert_eq!(h, 0xd1c9_6c7e_8eae_3966, "{h:#x}");
}

/// The fit's two conditions at their boundaries, in `f32` exactly as
/// `Fit::det` rounds them: a determinant equal to `0.1·e·e` (taken as
/// `(0.1·e)·e`, which here is one step above `0.1·(e·e)`) is refused and
/// the next `f32` up is accepted; an energy of exactly an eighth of the
/// segment is enough.
#[test]
fn fit_conditions_at_their_boundaries() {
    let fit = |cc: f32, ss: f32| Fit {
        cc,
        ss,
        ..Fit::default()
    };
    let at = f32::from_bits(0x3e02_10a8); // 0.127 016 66
    let above = f32::from_bits(0x3e02_10a9);
    let e = at + 1.0;
    // With ss = 1 and cs = 0 the determinant is cc.
    assert_eq!(at, 0.1 * e * e);
    assert!(0.1 * (e * e) < 0.1 * e * e);
    assert!(e >= 8.0 * 0.125);
    assert_eq!(fit(at, 1.0).det(8), None);
    assert_eq!(fit(above, 1.0).det(8), Some(above));
    // Energy: cc + ss = 1 = 8 / 8 exactly, well conditioned.
    assert_eq!(fit(0.5, 0.5).det(8), Some(0.25));
    assert_eq!(fit(0.5, 0.5).det(9), None);
}

#[test]
fn settings_and_inputs_are_checked() {
    let d = decode_near(1_000.0, 0.0);
    let mut audio = empty_slot();
    for segment in [
        0,
        SubtractConfig::MIN_SEGMENT - SubtractConfig::SEGMENT_STEP,
        SubtractConfig::MIN_SEGMENT + 1,
        WAVEFORM_SAMPLES + SubtractConfig::SEGMENT_STEP,
    ] {
        let config = SubtractConfig::DEFAULT.with_segment(segment);
        assert_eq!(sub(&mut audio, &d, 0, &config), Err(Error::InvalidConfig));
    }
    let bad_dt = Decode { dt: f32::NAN, ..d };
    assert_eq!(
        sub(&mut audio, &bad_dt, 0, &SubtractConfig::DEFAULT),
        Err(Error::NonFinite)
    );
    let bad_freq = Decode {
        freq_hz: 5_990.0,
        ..d
    };
    assert_eq!(
        sub(&mut audio, &bad_freq, 0, &SubtractConfig::DEFAULT),
        Err(Error::FrequencyOutOfRange)
    );
    // Silence: nothing to align with, nothing changed.
    let r = sub(&mut audio, &d, 0, &SubtractConfig::DEFAULT).unwrap();
    assert_eq!(r.samples, 0);
    assert!(audio.iter().all(|x| x.to_bits() == 0));
    assert_eq!(r.residue_db(), None);
}

#[test]
fn stack_storage_is_bounded() {
    const _: () = assert!(MAX_SEGMENTS == 158 && STACK_BYTES < 5 * 1024);
    assert_eq!(SCRATCH_BYTES, HELD * 8);
    assert_eq!(SCRATCH_BYTES, 46_080);
    assert_eq!(MIN_SCRATCH_BYTES, 1_920);
    // Every segment count `run` can use fits.
    for seg in
        (SubtractConfig::MIN_SEGMENT..=WAVEFORM_SAMPLES).step_by(SubtractConfig::SEGMENT_STEP)
    {
        assert!(WAVEFORM_SAMPLES.div_ceil(seg) <= MAX_SEGMENTS, "{seg}");
    }
    assert_eq!(STACK_BYTES, 4_656);
}

/// Scratch of `len` bytes holding NaN `f32` patterns.
fn nan_scratch(len: usize) -> Vec<u8> {
    let nan = f32::NAN.to_ne_bytes();
    nan.iter().copied().cycle().take(len).collect()
}

#[test]
fn any_scratch_size_gives_identical_results() {
    // From the least scratch (one block: every segment from a second run of
    // the reference), through rings that hold a segment and a half of the
    // shorter segments but not the longer ones, odd sizes, to more than
    // needed: bit-identical, and contents left from earlier calls (NaN at
    // first) do not matter. Too little is an error and changes nothing.
    // Over the starts, skips and cuts of
    // `held_and_rerun_references_give_identical_results` (a late first
    // estimate overflows a small ring part-way).
    let slot = noisy_slot();
    let lens = [
        MIN_SCRATCH_BYTES,
        MIN_SCRATCH_BYTES + 3,
        3 * 960 * 8,
        3 * 960 * 8 - 8,
        23_045,
        SCRATCH_BYTES - 8,
        SCRATCH_BYTES,
        SCRATCH_BYTES + 1_001,
    ];
    // One buffer per size, kept across the cases.
    let mut scratch: Vec<Vec<u8>> = lens.iter().map(|&len| nan_scratch(len)).collect();
    for dt in [0.31f32, -1.2] {
        let d = decode_near(1_234.5, dt);
        for (skip, cut) in [(0, SLOT_SAMPLES), (30_000, 90_000)] {
            for seg in [960, T, 2 * T] {
                let config = SubtractConfig::DEFAULT.with_segment(seg);
                let mut want = None;
                for s in &mut scratch {
                    let mut a = slot[..cut].to_vec();
                    let r = subtract(&mut a, &d, skip, &config, s).unwrap();
                    let h = hash(&a, &r);
                    let len = s.len();
                    assert_eq!(*want.get_or_insert(h), h, "{dt} {skip} {seg} {len}");
                }
                let mut a = slot[..cut].to_vec();
                let mut small = nan_scratch(MIN_SCRATCH_BYTES - 1);
                assert_eq!(
                    subtract(&mut a, &d, skip, &config, &mut small),
                    Err(Error::BufferTooSmall)
                );
                assert!(a.iter().zip(&slot).all(|(x, y)| x.to_bits() == y.to_bits()));
            }
        }
    }
}

#[test]
fn held_and_rerun_references_give_identical_results() {
    // The reference held between the fit and the subtraction, a second
    // run of it (hold 0), and holds small enough to overflow part-way
    // (the rest from a second run): bit-identical residuals and estimates.
    // 4 080, the shortest segment beyond `FUSED_SEGMENT`, always takes the
    // second run. The scratch starts as NaN and keeps every earlier call's
    // contents: they must not matter.
    let slot = noisy_slot();
    let mut scratch = nan_scratch(SCRATCH_BYTES);
    for (f, dt) in [(1_234.5f32, 0.31f32), (1_234.5, -1.2)] {
        let d = decode_near(f, dt);
        for seg in [960, T, 2 * T, 2 * T + SubtractConfig::SEGMENT_STEP] {
            for (skip, cut) in [(0, SLOT_SAMPLES), (30_000, 90_000)] {
                let config = SubtractConfig::DEFAULT.with_segment(seg);
                let mut want = None;
                for hold in [HELD, 0, 1_000, 2_500] {
                    let mut a = slot[..cut].to_vec();
                    let r = run(
                        &mut a,
                        &d,
                        skip,
                        &config,
                        Fixed::default(),
                        hold,
                        true,
                        &mut scratch,
                    )
                    .unwrap();
                    let h = hash(&a, &r);
                    assert_eq!(*want.get_or_insert(h), h, "{f} {dt} {seg} {skip} {hold}");
                    // Unreported: the same residual and placement, bit for bit.
                    let mut b = slot[..cut].to_vec();
                    let q = run(
                        &mut b,
                        &d,
                        skip,
                        &config,
                        Fixed::default(),
                        hold,
                        false,
                        &mut scratch,
                    )
                    .unwrap();
                    let unreported = Subtraction {
                        energy: r.energy,
                        residue: r.residue,
                        residue_noise: r.residue_noise,
                        ..q
                    };
                    assert_eq!(hash(&b, &unreported), h, "{f} {dt} {seg} {skip} {hold}");
                }
            }
        }
    }
}

/// Residual and estimate hashes first recorded with the subtraction before
/// the reference was held between the fit and the subtraction (one run of
/// the reference per step), and re-pinned when the reference's sine and
/// cosine became a polynomial and the fit's sums lane-split (those change
/// the arithmetic, not the paths): segments beyond `FUSED_SEGMENT` (4 080,
/// 7 680, the whole waveform), the host's 3 840, a skip that delays the
/// first estimate, a cut, drift on and off, a start inside the slot and
/// one before it. The held and the rerun paths must both still give these.
#[test]
fn anchored_to_the_unheld_subtraction() {
    let slot = noisy_slot();
    let mut got = Vec::new();
    let mut fails = Vec::new();
    for &(f, dt, seg, drift, skip, cut, want) in &[
        (
            1_234.5f32,
            0.31f32,
            3840,
            true,
            0,
            SLOT_SAMPLES,
            0x40ab_3e06_7e05_d26cu64,
        ),
        (
            1_234.5,
            0.31,
            7680,
            true,
            0,
            SLOT_SAMPLES,
            0x914e_c5f6_5a31_b48a,
        ),
        (
            1_234.5,
            0.31,
            7680,
            false,
            0,
            SLOT_SAMPLES,
            0x24e2_26be_9142_a923,
        ),
        (
            1_234.5,
            0.31,
            WAVEFORM_SAMPLES,
            true,
            0,
            SLOT_SAMPLES,
            0xbaca_d495_bda7_b030,
        ),
        (
            1_234.5,
            0.31,
            WAVEFORM_SAMPLES,
            false,
            0,
            SLOT_SAMPLES,
            0x9853_3688_6bf8_8fdf,
        ),
        (
            1_234.5,
            0.31,
            4080,
            true,
            0,
            SLOT_SAMPLES,
            0xc7c3_fdda_3711_c2d0,
        ),
        (
            1_234.5,
            0.31,
            4080,
            false,
            0,
            SLOT_SAMPLES,
            0x194f_3fd4_1b23_ce1e,
        ),
        (
            1_234.5,
            0.31,
            3840,
            true,
            30_000,
            SLOT_SAMPLES,
            0x2319_1ca8_211f_ae52,
        ),
        (
            1_234.5,
            0.31,
            3840,
            false,
            30_000,
            90_000,
            0x9b16_06fb_d527_82d8,
        ),
        (1_234.5, 0.31, 1920, true, 0, 90_000, 0xab80_fe0e_7743_f0d9),
        (
            1_234.5,
            0.31,
            4080,
            true,
            30_000,
            90_000,
            0x2e8d_8b3e_44ba_842e,
        ),
        (
            1_234.5,
            0.31,
            7680,
            true,
            30_000,
            90_000,
            0x7820_14d8_c944_8a14,
        ),
        (
            1_234.5,
            -1.2,
            3840,
            true,
            0,
            SLOT_SAMPLES,
            0xbe40_6b0d_9d99_d9f2,
        ),
        (
            1_234.5,
            -1.2,
            7680,
            true,
            0,
            SLOT_SAMPLES,
            0x7817_167e_4738_3fd4,
        ),
        (
            1_234.5,
            -1.2,
            7680,
            false,
            0,
            SLOT_SAMPLES,
            0xbee8_cb75_7d65_2814,
        ),
        (
            1_234.5,
            -1.2,
            WAVEFORM_SAMPLES,
            true,
            0,
            SLOT_SAMPLES,
            0x1cf9_6201_a08c_6a15,
        ),
        (
            1_234.5,
            -1.2,
            WAVEFORM_SAMPLES,
            false,
            0,
            SLOT_SAMPLES,
            0xe7c6_efb6_17d1_98ee,
        ),
        (
            1_234.5,
            -1.2,
            4080,
            true,
            0,
            SLOT_SAMPLES,
            0x7477_f100_ac5e_2c9c,
        ),
        (
            1_234.5,
            -1.2,
            4080,
            false,
            0,
            SLOT_SAMPLES,
            0x2f20_7bac_85ce_bf76,
        ),
        (
            1_234.5,
            -1.2,
            3840,
            true,
            30_000,
            SLOT_SAMPLES,
            0x00bc_1d92_fd50_e164,
        ),
        (
            1_234.5,
            -1.2,
            3840,
            false,
            30_000,
            90_000,
            0x5ceb_e21a_4d68_f799,
        ),
        (1_234.5, -1.2, 1920, true, 0, 90_000, 0xc80e_9649_19bd_84f9),
        (
            1_234.5,
            -1.2,
            4080,
            true,
            30_000,
            90_000,
            0x2c9b_7bf1_db3c_e48c,
        ),
        (
            1_234.5,
            -1.2,
            7680,
            true,
            30_000,
            90_000,
            0x3e70_3a97_6bf1_a1da,
        ),
    ] {
        let d = decode_near(f, dt);
        let config = SubtractConfig::DEFAULT.with_segment(seg).with_drift(drift);
        let mut a = slot[..cut].to_vec();
        let r = sub(&mut a, &d, skip, &config).unwrap();
        let h = hash(&a, &r);
        got.push(h);
        // The decoder's unreported subtraction leaves the same residual.
        let mut b = slot[..cut].to_vec();
        subtract_unreported(&mut b, &d, skip, &config, &mut nan_scratch(SCRATCH_BYTES)).unwrap();
        assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
        if h != want {
            fails.push((f, dt, seg, drift, skip, cut));
        }
    }
    assert!(fails.is_empty(), "{fails:?} {got:x?}");
}

/// [`Correction::skip`] holds for counts of any size, against the closed
/// form in 128-bit arithmetic: n·step and n·(n − 1)/2·accel modulo 2⁶⁴.
#[test]
#[cfg(target_pointer_width = "64")]
fn correction_skip_holds_for_any_count() {
    let c0 = Correction {
        phase: 0x0123_4567_89ab_cdef,
        step: -0x0f1e_2d3c_4b5a_6978,
        accel: 0x7766_5544_3322_1101,
    };
    for n in [
        0u64,
        1,
        2,
        151_680,
        (1 << 32) - 1,
        1 << 32,
        (1 << 32) + 1,
        (1 << 40) + 3,
        1 << 63,
        (1 << 63) + 1,
        u64::MAX - 1,
        u64::MAX,
    ] {
        let mut a = c0;
        a.skip(n as usize);
        let wide = |x: i64| u128::from(x as u64);
        let pairs = (u128::from(n) * u128::from(n.saturating_sub(1)) / 2) as u64;
        let phase = (u128::from(c0.phase) + u128::from(n) * wide(c0.step)) as u64;
        let phase = phase.wrapping_add(pairs.wrapping_mul(c0.accel as u64));
        let step = (wide(c0.step) + u128::from(n) * wide(c0.accel)) as u64 as i64;
        assert_eq!((a.phase, a.step, a.accel), (phase, step, c0.accel), "{n}");
    }
}

/// The frequency correction taken in closed form ([`Correction::skip`]),
/// four lanes at a time ([`Correction::lanes`]) and added in place
/// ([`Correction::apply`]) gives exactly the sample-by-sample sequence of
/// [`Correction::next`], over random states (any phase, step and
/// acceleration, wrapping included).
#[test]
fn correction_in_lanes_and_in_closed_form_follows_the_sequence() {
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let same =
        |a: &Correction, b: &Correction| (a.phase, a.step, a.accel) == (b.phase, b.step, b.accel);
    for k in 0..2_000 {
        let small = |v: u64| if k % 2 == 0 { v } else { v >> 40 };
        let c0 = Correction {
            phase: rnd(),
            step: small(rnd()) as i64,
            accel: small(rnd()) as i64,
        };
        let mut seq = c0;
        let want: Vec<u32> = (0..64).map(|_| seq.next()).collect();
        let mut lanes = c0.lanes();
        let got: Vec<u32> = (0..16).flat_map(|_| lanes.next4()).collect();
        assert_eq!(got, want);
        for n in [0usize, 1, 2, 3, 4, 5, 63, 240, 1_921, 151_680] {
            let (mut a, mut b) = (c0, c0);
            a.skip(n);
            for _ in 0..n {
                b.next();
            }
            assert!(same(&a, &b), "skip {n}");
        }
        for n in [0usize, 1, 3, 4, 7, 240] {
            let phase: Vec<u32> = (0..n).map(|_| rnd() as u32).collect();
            let mut q = phase.clone();
            let (mut a, mut b) = (c0, c0);
            a.apply(&mut q);
            for (q, p) in q.iter().zip(&phase) {
                assert_eq!(*q, p.wrapping_add(b.next()), "apply {n}");
            }
            assert!(same(&a, &b), "apply {n}");
        }
    }
}

/// The lanes: samples `a..b` of a block split at multiples of `LANES`.
#[test]
fn lane_bounds_split_at_multiples_of_the_lanes() {
    for a in 0..=12 {
        for b in a..=12 {
            let (m0, m1) = lane_bounds(a, b);
            assert!(a <= m0 && m0 <= m1 && m1 <= b, "{a} {b}");
            assert!(m0 % LANES == 0 || m0 == b, "{a} {b}");
            assert!(m1 % LANES == 0 || m1 == m0, "{a} {b}");
            assert!(m0 - a < LANES && b - m1 < LANES, "{a} {b}");
        }
    }
}

/// The time search's correlations where the usable audio starts and ends
/// inside a symbol's middle half, at every offset modulo [`LANES`] (and at
/// starts that are not multiples of it): each block's products summed in
/// the documented order, sample i of the block in lane i % `LANES`, each
/// lane in sample order, the lanes added by [`sum_lanes`]. Bit for bit.
#[test]
fn time_search_lanes_follow_the_sample_index_at_any_start() {
    let audio = noisy_slot();
    let tones = encode_payload(&decode_near(1_234.5, 0.31).payload);
    let synth = Synth::new(&tones, 1_234.5, 1.0).unwrap();
    let mut work = std::vec![0u8; SCRATCH_BYTES];
    let core = |j0: usize| (CORE_START..CORE_END).contains(&(j0 % T));
    let mut checked = 0;
    for start in [-1_001i64, 4_001, 4_002, 4_003, 4_004] {
        for r in 0..LANES as i64 {
            let lo_off = (T + CORE_START + 37) as i64 + r;
            let hi_off = (50 * T + CORE_START + 101) as i64 + 3 * r;
            let usable = Usable {
                lo: (start + lo_off).max(0),
                hi: start + hi_off,
            };
            let got = symbol_correlations(&audio, usable, &synth, start, &mut work);
            let mut want = [(0.0f32, 0.0f32); SYMBOL_COUNT];
            let end = usable.range(start, 0, WAVEFORM_SAMPLES).1;
            blocks(&synth, end, core, |j0, phase, _| {
                let (mut re, mut im) = ([0.0f32; LANES], [0.0f32; LANES]);
                let mut any = false;
                // Lane by lane, each in sample order (not a store indexed by
                // `i % LANES`: see `fit_sums_follow_the_sample_index_bit_for_bit`).
                for l in 0..LANES {
                    for i in (l..phase.len()).step_by(LANES) {
                        let s = start + (j0 + i) as i64;
                        if s < usable.lo || s >= usable.hi {
                            continue;
                        }
                        let w = input_branch(audio[s as usize]);
                        let (c, sn) = cos_sin_turns(phase[i]);
                        re[l] += w * c;
                        im[l] += w * sn;
                        any = true;
                    }
                }
                if any {
                    want[j0 / T].0 += sum_lanes(re);
                    want[j0 / T].1 -= sum_lanes(im);
                }
            });
            let bits = |v: &[(f32, f32)]| -> Vec<(u32, u32)> {
                v.iter().map(|&(a, b)| (a.to_bits(), b.to_bits())).collect()
            };
            assert_eq!(bits(&got), bits(&want), "start {start} r {r}");
            assert!(got.iter().any(|&(a, _)| a != 0.0), "start {start} r {r}");
            checked += 1;
        }
    }
    assert_eq!(checked, 20);
}

/// Random 32-bit words, from the tests' generator.
fn words(n: &mut Noise) -> u32 {
    (n.uniform() * 4_294_967_296.0) as u32
}

/// Phases for the reference tests: random, and the quarter turns and the
/// polynomial's interval ends (±2²⁹ around them) with their neighbours.
fn test_phases(n: &mut Noise, len: usize) -> Vec<u32> {
    let mut edges = Vec::new();
    for k in 0..4u32 {
        for d in [0u32, 1, 2, (1 << 29) - 1, 1 << 29, (1 << 29) + 1] {
            edges.push((k << 30).wrapping_add(d));
            edges.push((k << 30).wrapping_sub(d));
        }
    }
    edges.push(u32::MAX);
    (0..len)
        .map(|i| {
            if i % 3 == 0 {
                edges[(words(n) as usize) % edges.len()]
            } else {
                words(n)
            }
        })
        .collect()
}

/// Weights for the reference tests: random audio and envelope values, and
/// the values the front end treats specially (non-finite, beyond the
/// limit, zeros, subnormal).
fn test_weights(n: &mut Noise, len: usize) -> Vec<f32> {
    let edges = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        f32::NAN,
        -f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::MAX,
        f32::MIN,
        INPUT_LIMIT,
        -INPUT_LIMIT,
        INPUT_LIMIT * 1.5,
        -INPUT_LIMIT * 3.0,
        f32::MIN_POSITIVE,
        f32::from_bits(1),
        -f32::from_bits(0x7f_ffff),
    ];
    (0..len)
        .map(|i| {
            if i % 4 == 1 {
                edges[(words(n) as usize) % edges.len()]
            } else {
                (n.gauss() * 0.3) as f32
            }
        })
        .collect()
}

/// A test weight as a reference value can be: finite (the reference is
/// an envelope times a cosine or sine), the non-finite ones replaced.
/// Comparing NaN results bit for bit would not be meaningful: Rust does
/// not specify the sign or payload of a NaN that arithmetic produces (an
/// unoptimised build gave a NaN energy the other sign).
fn finite_reference(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.75
    }
}

/// [`scaled_cos_sin`] as first written: the plain loop, one sample at a
/// time, sample i at slot `k0 + i`.
fn scaled_cos_sin_first(
    out: &mut [u8],
    k0: usize,
    phase: &[u32],
    xs: &[f32],
    weight: impl Fn(f32) -> f32,
) {
    for (i, (&q, &x)) in phase.iter().zip(xs).enumerate() {
        let w = weight(x);
        let (c, s) = cos_sin_turns(q);
        put(out, k0 + i, (w * c, w * s));
    }
}

/// [`scaled_cos_sin`] at `W` samples at a time against its first form,
/// byte for byte over the whole buffer (so no slot outside the samples'
/// is written), from every first slot modulo a group and beyond, for both
/// weights the decoder uses.
fn scaled_cos_sin_matches<const W: usize>(seed: u64) -> usize {
    let mut n = Noise(seed);
    let mut checked = 0;
    let lens = (0..=3 * W + 5).chain([BLOCK, BLOCK + 1, 4 * BLOCK - 3]);
    for len in lens {
        let phase = test_phases(&mut n, len + 2);
        let xs = test_weights(&mut n, len + 2);
        // Equal lengths, and each input the shorter by one or two.
        let shapes = [(len, len), (len, len + 2), (len + 1, len)];
        for (np, nx) in shapes {
            for k0 in 0..=GROUP + 1 {
                for clamp in [false, true] {
                    // The audio through the front end's clamp; the envelope,
                    // which is finite, as it is.
                    let weigh = |x: f32| if clamp { input(x) } else { finite_reference(x) };
                    let mut got = std::vec![0xa5u8; (k0 + len + 2 * GROUP + 2) * PAIR];
                    let mut want = got.clone();
                    scaled_cos_sin::<W, _>(&mut got, k0, &phase[..np], &xs[..nx], weigh);
                    scaled_cos_sin_first(&mut want, k0, &phase[..np], &xs[..nx], weigh);
                    assert_eq!(got, want, "W {W} lengths {np} {nx} slot {k0} clamp {clamp}");
                    checked += 1;
                }
            }
        }
    }
    checked
}

/// The reference and the time search's products, several samples at a
/// time (the hosts' form) and one at a time (where nothing vectorises),
/// against the plain loop bit for bit: random and edge phases and weights,
/// every length up to three blocks of `W` and beyond, unequal inputs.
#[test]
fn scaled_cos_sin_is_its_first_form_bit_for_bit() {
    let mut checked = scaled_cos_sin_matches::<1>(1);
    checked += scaled_cos_sin_matches::<4>(2);
    checked += scaled_cos_sin_matches::<8>(3);
    checked += scaled_cos_sin_matches::<CHAINS>(4);
    checked += scaled_cos_sin_matches::<24>(5);
    assert!(checked > 1_000, "{checked}");
}

/// The reference without an envelope against the reference with an
/// envelope of 1.0, byte for byte: every length up to three blocks of
/// [`CHAINS`] and beyond, random and edge phases.
#[test]
fn reference_without_an_envelope_is_the_reference_times_one() {
    let mut n = Noise(13);
    let mut checked = 0;
    for len in (0..=3 * CHAINS + 5).chain([BLOCK - 1, BLOCK]) {
        let phase = test_phases(&mut n, len);
        let ones = std::vec![1.0f32; len];
        for k0 in 0..=GROUP {
            let mut got = std::vec![0xa5u8; (k0 + len + 2 * GROUP) * PAIR];
            let mut want = got.clone();
            reference(&mut got, k0, &phase, None);
            reference(&mut want, k0, &phase, Some(&ones));
            assert_eq!(got, want, "length {len} slot {k0}");
            checked += 1;
        }
    }
    assert_eq!(checked, (3 * CHAINS + 8) * (GROUP + 1));
}

/// `blocks` passes no envelope exactly for the blocks where it is 1.0 at
/// every sample, the envelope where it is not, and the generator's phase
/// either way: against `Synth::fill_phase` over the whole waveform.
#[test]
fn blocks_leave_out_only_an_envelope_of_one() {
    let tones = encode_payload(&decode_near(1_234.5, 0.31).payload);
    let synth = Synth::new(&tones, 1_234.5, 1.0).unwrap();
    let mut phase = std::vec![0u32; WAVEFORM_SAMPLES];
    let mut env = std::vec![0.0f32; WAVEFORM_SAMPLES];
    assert_eq!(
        synth.clone().fill_phase(&mut phase, &mut env),
        WAVEFORM_SAMPLES
    );
    let (mut seen, mut without) = (0, 0);
    blocks(
        &synth,
        WAVEFORM_SAMPLES,
        |_| true,
        |j0, p, e| {
            assert_eq!(p, &phase[j0..j0 + p.len()], "{j0}");
            let want = &env[j0..j0 + p.len()];
            match e {
                Some(e) => {
                    let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
                    assert_eq!(bits(e), bits(want), "{j0}");
                    assert!(want.iter().any(|&w| w != 1.0), "{j0}");
                }
                None => {
                    assert!(
                        want.iter().all(|&w| w.to_bits() == 1.0f32.to_bits()),
                        "{j0}"
                    );
                    without += 1;
                }
            }
            seen += p.len();
        },
    );
    assert_eq!(seen, WAVEFORM_SAMPLES);
    assert_eq!(without, WAVEFORM_SAMPLES / BLOCK - 2);
}

/// [`input_select`] and [`input`] against [`input_branch`] bit for bit at every `stride`-th
/// `f32` bit pattern (both signs, NaNs and infinities included) and at the
/// values the front end treats specially.
fn input_select_matches(stride: usize) -> usize {
    let mut n = 0usize;
    let edges = [
        0u32,
        0x8000_0000,
        1,
        0x8000_0001,
        INPUT_LIMIT.to_bits(),
        (-INPUT_LIMIT).to_bits(),
        INPUT_LIMIT.to_bits() + 1,
        (-INPUT_LIMIT).to_bits() + 1,
        INPUT_LIMIT.to_bits() - 1,
        f32::MAX.to_bits(),
        f32::MIN.to_bits(),
        f32::INFINITY.to_bits(),
        f32::NEG_INFINITY.to_bits(),
        f32::NAN.to_bits(),
        (-f32::NAN).to_bits(),
        0x7f80_0001,
        0xff80_0001,
        0x7fff_ffff,
        u32::MAX,
    ];
    let strided = (0..=u32::MAX).step_by(stride);
    for bits in strided.chain(edges) {
        let x = f32::from_bits(bits);
        assert_eq!(
            input_select(x).to_bits(),
            input_branch(x).to_bits(),
            "{bits:#x}"
        );
        assert_eq!(input(x).to_bits(), input_branch(x).to_bits(), "{bits:#x}");
        n += 1;
    }
    n
}

#[test]
fn input_select_is_the_branch_form() {
    assert!(input_select_matches(4_099) > 1_000_000);
}

/// Every `f32` bit pattern (2³² values; run in release).
#[test]
#[ignore]
fn input_select_is_the_branch_form_everywhere() {
    input_select_matches(1);
}

/// [`Removal::run_as`] as first written: the ramp index `(j − from) as
/// f32` from `usize`, one sample at a time, the reference of sample
/// `j0 + i` at slot `k0 + i`.
fn run_as_first<const REPORT: bool>(
    r: &mut Removal,
    j0: usize,
    xs: &mut [f32],
    refs: &[u8],
    k0: usize,
) {
    let (from, base, slope) = (r.from, r.base, r.slope);
    let mut energy = r.energy;
    let mut sums = r.sums;
    for (i, x) in xs.iter_mut().enumerate() {
        let (rc, rs) = get(refs, k0 + i);
        let j = j0 + i;
        let t = if j > from { (j - from) as f32 } else { 0.0 };
        let (ra, rb) = (base.0 + t * slope.0, base.1 + t * slope.1);
        let est = ra * rc + rb * rs;
        *x = input_branch(*x) - est;
        if REPORT {
            energy += est * est;
            sums.add(*x, rc, rs);
        }
    }
    r.energy = energy;
    r.sums = sums;
    r.changed += xs.len();
}

/// The removal's state and output, bit for bit.
fn removal_bits(r: &Removal, xs: &[f32]) -> Vec<u32> {
    let f = &r.sums.fit;
    let mut v: Vec<u32> = [r.energy, f.xc, f.xs, f.cc, f.cs, f.ss, r.sums.xx]
        .iter()
        .map(|x| x.to_bits())
        .collect();
    v.push(r.sums.n as u32);
    v.push(r.changed as u32);
    v.extend(xs.iter().map(|x| x.to_bits()));
    v
}

/// The removal with the ramp index in 32-bit integers (and, in whole
/// groups, carried as `f32`) against its first form, bit for bit,
/// reporting and not: runs that start before, at and after the interval's
/// centre and cross it, at every offset modulo the lanes, the reference
/// from every slot modulo a group, up to the waveform's last sample;
/// random and edge audio and references, slopes of either sign.
#[test]
fn removal_ramp_is_its_first_form_bit_for_bit() {
    let mut n = Noise(77);
    let mut checked = 0;
    let froms = [
        0usize,
        1,
        3,
        960,
        1_921,
        WAVEFORM_SAMPLES - 961,
        WAVEFORM_SAMPLES,
    ];
    for &from in &froms {
        // Runs from before the centre (crossing it), and from far after it
        // (ramp indices up to the waveform's length).
        let starts = [0isize, -1, -2, -3, -4, -5, -17, -300, 1, 40_003, 150_000];
        for off in starts {
            for len in [0usize, 1, 3, 4, 5, 8, 31, 240, 961] {
                let j0 = from.saturating_add_signed(off).min(WAVEFORM_SAMPLES);
                let len = len.min(WAVEFORM_SAMPLES - j0);
                let k0 = (j0 + len) % GROUP;
                let mut xs = test_weights(&mut n, len);
                let refs: Vec<u8> = test_weights(&mut n, 2 * (k0 + len + GROUP))
                    .iter()
                    .map(|&v| finite_reference(v))
                    .flat_map(|x| x.to_ne_bytes())
                    .collect();
                for report in [false, true] {
                    let mut a = Removal::new(T, (0.0, 0.0), report);
                    a.from = from;
                    a.base = ((n.gauss() * 0.1) as f32, (n.gauss() * 0.1) as f32);
                    a.slope = ((n.gauss() * 1e-4) as f32, (n.gauss() * -1e-4) as f32);
                    a.energy = (n.uniform() * 3.0) as f32;
                    a.sums.add(0.25, -0.5, 0.75);
                    let mut b = Removal::new(T, (0.0, 0.0), report);
                    (b.from, b.base, b.slope, b.energy, b.sums) =
                        (a.from, a.base, a.slope, a.energy, a.sums);
                    let mut ys = xs.clone();
                    if report {
                        a.run_as::<true>(j0, &mut xs, &refs, k0);
                        run_as_first::<true>(&mut b, j0, &mut ys, &refs, k0);
                    } else {
                        a.run_as::<false>(j0, &mut xs, &refs, k0);
                        run_as_first::<false>(&mut b, j0, &mut ys, &refs, k0);
                    }
                    assert_eq!(
                        removal_bits(&a, &xs),
                        removal_bits(&b, &ys),
                        "from {from} j0 {j0} len {len} slot {k0} report {report}"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, froms.len() * 11 * 9 * 2);
}

/// At and before the interval's centre the ramp index is +0.0, which is
/// observable only through a zero's sign: with base −0.0, a positive
/// slope and positive references, the estimate there is +0.0, and −0.0
/// audio less it stays −0.0 (a ramp of −0.0 would give an estimate of
/// −0.0 and audio of +0.0). Runs at every lane offset that end at,
/// before and past the centre, reporting and not, against the first
/// form and the expected bits.
#[test]
fn removal_ramp_is_plus_zero_at_and_before_the_centre() {
    let from = 1_921usize;
    let mut checked = 0;
    for j0 in from - 9..=from {
        for len in [1usize, 4, 5, 9, 17] {
            let xs = std::vec![-0.0f32; len];
            let k0 = (j0 + 2 * len) % GROUP;
            let refs: Vec<u8> = (0..2 * (k0 + len + GROUP))
                .map(|k| 0.25 + k as f32 / 64.0)
                .flat_map(|x| x.to_ne_bytes())
                .collect();
            for report in [false, true] {
                let mut a = Removal::new(T, (0.0, 0.0), report);
                (a.from, a.base, a.slope) = (from, (-0.0, -0.0), (1e-3, 2e-3));
                let mut b = Removal::new(T, (0.0, 0.0), report);
                (b.from, b.base, b.slope) = (a.from, a.base, a.slope);
                let (mut got, mut want) = (xs.clone(), xs.clone());
                if report {
                    a.run_as::<true>(j0, &mut got, &refs, k0);
                    run_as_first::<true>(&mut b, j0, &mut want, &refs, k0);
                } else {
                    a.run_as::<false>(j0, &mut got, &refs, k0);
                    run_as_first::<false>(&mut b, j0, &mut want, &refs, k0);
                }
                // Samples j0..=from are at or before the centre.
                for (j, x) in (j0..).zip(&got).take_while(|&(j, _)| j <= from) {
                    assert_eq!(x.to_bits(), (-0.0f32).to_bits(), "j {j} report {report}");
                }
                assert_eq!(
                    removal_bits(&a, &got),
                    removal_bits(&b, &want),
                    "j0 {j0} len {len}"
                );
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 10 * 5 * 2);
}

/// The fit's lane sums bit for bit.
fn fit4_bits(f: &Fit4) -> Vec<u32> {
    [f.xc, f.xs, f.cc, f.cs, f.ss]
        .iter()
        .flatten()
        .map(|x| x.to_bits())
        .collect()
}

/// [`Fit4::add_all`] against its definition, sample by sample: sample i
/// of the block in lane i % `LANES`, each lane in sample order, the audio
/// as the front end takes it. Bit for bit, from random lane sums, at every
/// start modulo the lanes and lengths up to a block, the reference from
/// slot `i0` and from a lane on, random and edge audio and references.
#[test]
fn fit_sums_follow_the_sample_index_bit_for_bit() {
    let mut n = Noise(91);
    let mut checked = 0;
    for i0 in 0..2 * LANES + 1 {
        for (len, k0) in (0..=13)
            .chain([BLOCK - i0, 961])
            .flat_map(|len| [(len, i0), (len, i0 + LANES)])
        {
            let xs = test_weights(&mut n, len);
            let refs: Vec<u8> = test_weights(&mut n, 2 * (k0 + len + GROUP))
                .iter()
                .map(|&v| {
                    if v.is_finite() {
                        v.clamp(-4.0, 4.0)
                    } else {
                        0.5
                    }
                })
                .flat_map(|x| x.to_ne_bytes())
                .collect();
            let mut got = Fit4::default();
            for l in 0..LANES {
                (got.xc[l], got.xs[l], got.cc[l]) = (l as f32, -0.5, 2.0);
                (got.cs[l], got.ss[l]) = ((n.gauss()) as f32, 0.125);
            }
            let mut want = got;
            got.add_all(i0, &xs, &refs, k0);
            // Lane by lane, each in sample order. (Not a store indexed by
            // `(i0 + k) % LANES` in one loop over the samples: the loop
            // vectoriser of LLVM 20 and 21 miscompiled that form here;
            // findings O19b.)
            for l in 0..LANES {
                let first = (l + LANES - i0 % LANES) % LANES;
                for k in (first..xs.len()).step_by(LANES) {
                    let (c, s) = get(&refs, k0 + k);
                    let w = input_branch(xs[k]);
                    want.xc[l] += w * c;
                    want.xs[l] += w * s;
                    want.cc[l] += c * c;
                    want.cs[l] += c * s;
                    want.ss[l] += s * s;
                }
            }
            assert_eq!(
                fit4_bits(&got),
                fit4_bits(&want),
                "i0 {i0} len {len} slot {k0}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, (2 * LANES + 1) * 16 * 2);
}

#[test]
fn step_phase_is_the_64_bit_quotient() {
    // Every tone step and every start error of the time search (and
    // beyond), against the 64-bit division it replaced.
    for m in -7..=7 {
        for e in -(T as i32)..=T as i32 {
            let r = (m * e).rem_euclid(T as i32) as u64;
            assert_eq!(step_phase(m, e), ((r << 32) / T as u64) as u32, "{m} {e}");
        }
    }
}
