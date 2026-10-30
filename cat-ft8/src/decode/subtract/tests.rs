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
use crate::decode::testutil::{add_noise, add_signal, amplitude_for, empty_slot, symbols_of};
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
            0xf677_81b2_5882_c626,
            0xfc2a_9cb5_fdf7_76ce,
            0xd6d8_05c9_6781_5885
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
    (0..len).map(|i| nan[i % 4]).collect()
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

/// Residual and estimate hashes recorded with the subtraction before the
/// reference was held between the fit and the subtraction (one run of the
/// reference per step): segments beyond `FUSED_SEGMENT` (4 080, 7 680, the
/// whole waveform), the host's 3 840, a skip that delays the first
/// estimate, a cut, drift on and off, a start inside the slot and one
/// before it. The held and the rerun paths must both still give these.
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
            0x8bf8_b71a_ad43_d273u64,
        ),
        (
            1_234.5,
            0.31,
            7680,
            true,
            0,
            SLOT_SAMPLES,
            0xd37e_34a5_2f3f_216f,
        ),
        (
            1_234.5,
            0.31,
            7680,
            false,
            0,
            SLOT_SAMPLES,
            0xcd52_e6d3_905d_f9f0,
        ),
        (
            1_234.5,
            0.31,
            WAVEFORM_SAMPLES,
            true,
            0,
            SLOT_SAMPLES,
            0xd6d8_05c9_6781_5885,
        ),
        (
            1_234.5,
            0.31,
            WAVEFORM_SAMPLES,
            false,
            0,
            SLOT_SAMPLES,
            0x7301_2fd1_6516_cfb0,
        ),
        (
            1_234.5,
            0.31,
            4080,
            true,
            0,
            SLOT_SAMPLES,
            0xf5a3_e203_dd8e_d9c2,
        ),
        (
            1_234.5,
            0.31,
            4080,
            false,
            0,
            SLOT_SAMPLES,
            0x3d59_e784_b821_803f,
        ),
        (
            1_234.5,
            0.31,
            3840,
            true,
            30_000,
            SLOT_SAMPLES,
            0x51b0_c0d7_864c_2a6d,
        ),
        (
            1_234.5,
            0.31,
            3840,
            false,
            30_000,
            90_000,
            0xd17a_4ff8_0543_805d,
        ),
        (1_234.5, 0.31, 1920, true, 0, 90_000, 0x7b8d_3724_992e_efd2),
        (
            1_234.5,
            0.31,
            4080,
            true,
            30_000,
            90_000,
            0x083c_ebfe_16fa_aaec,
        ),
        (
            1_234.5,
            0.31,
            7680,
            true,
            30_000,
            90_000,
            0x75b0_f7a5_a39d_bfec,
        ),
        (
            1_234.5,
            -1.2,
            3840,
            true,
            0,
            SLOT_SAMPLES,
            0xfd0e_5a7e_ea5c_579b,
        ),
        (
            1_234.5,
            -1.2,
            7680,
            true,
            0,
            SLOT_SAMPLES,
            0xa329_0edf_63a4_b1c2,
        ),
        (
            1_234.5,
            -1.2,
            7680,
            false,
            0,
            SLOT_SAMPLES,
            0x794d_80c7_ee1d_02b0,
        ),
        (
            1_234.5,
            -1.2,
            WAVEFORM_SAMPLES,
            true,
            0,
            SLOT_SAMPLES,
            0xfe83_5183_b372_9b9e,
        ),
        (
            1_234.5,
            -1.2,
            WAVEFORM_SAMPLES,
            false,
            0,
            SLOT_SAMPLES,
            0x539b_a53f_2510_78c5,
        ),
        (
            1_234.5,
            -1.2,
            4080,
            true,
            0,
            SLOT_SAMPLES,
            0x3301_e6fa_5b3f_dd75,
        ),
        (
            1_234.5,
            -1.2,
            4080,
            false,
            0,
            SLOT_SAMPLES,
            0x1314_230f_7803_5574,
        ),
        (
            1_234.5,
            -1.2,
            3840,
            true,
            30_000,
            SLOT_SAMPLES,
            0xe27a_42a5_16a1_f909,
        ),
        (
            1_234.5,
            -1.2,
            3840,
            false,
            30_000,
            90_000,
            0xb599_78fc_68ba_5194,
        ),
        (1_234.5, -1.2, 1920, true, 0, 90_000, 0xb94f_12b5_52cb_9d6c),
        (
            1_234.5,
            -1.2,
            4080,
            true,
            30_000,
            90_000,
            0xe9a1_2acf_e399_5b55,
        ),
        (
            1_234.5,
            -1.2,
            7680,
            true,
            30_000,
            90_000,
            0x2cea_3cd8_2d1f_cd42,
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
