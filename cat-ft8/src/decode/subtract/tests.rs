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

fn energy(x: &[f32]) -> f64 {
    x.iter().map(|&v| f64::from(v) * f64::from(v)).sum()
}

fn db(a: f64, b: f64) -> f64 {
    10.0 * libm::log10(a / b)
}

#[test]
fn noiseless_signal_is_suppressed_below_the_floor() {
    for (freq, dt) in [(1_500.0f32, 0.2f32), (212.5, -0.9), (2_987.0, 1.3)] {
        let clean = slot_with(freq, dt, 0.1);
        let mut audio = clean.clone();
        let r = subtract(
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
    let r = subtract(
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
    let r = subtract(
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
    let r = subtract(
        &mut early,
        &decode_near(freq, dt),
        0,
        &SubtractConfig::DEFAULT,
    )
    .unwrap();
    assert_eq!(r.samples, 0);
    assert_eq!(early, clean[..6_000]);
    // Nor does an empty one.
    let r = subtract(&mut [], &decode_near(freq, dt), 0, &SubtractConfig::DEFAULT).unwrap();
    assert_eq!(r.samples, 0);
}

#[test]
fn a_skipped_region_stays_zero() {
    // A signal starting at 0.1 s (sample 1 200) in a slot whose first
    // second is missing (zero).
    let (freq, dt) = (1_700.0, -0.4);
    let mut audio = slot_with(freq, dt, 0.1);
    let skip = 12_000;
    audio[..skip].fill(0.0);
    let clean = audio.clone();
    let r = subtract(
        &mut audio,
        &decode_near(freq, dt),
        skip,
        &SubtractConfig::DEFAULT,
    )
    .unwrap();
    assert!(audio[..skip].iter().all(|x| x.to_bits() == 0));
    let start = Placement::slot(freq, dt).unwrap().start;
    assert_eq!(r.start, start);
    assert_eq!(r.samples, start as usize + WAVEFORM_SAMPLES - skip);
    let left = db(energy(&audio), energy(&clean));
    assert!(left < -40.0, "{left:.1} dB {r:?}");
    // Skipping everything changes nothing.
    let mut all = clean.clone();
    let r = subtract(
        &mut all,
        &decode_near(freq, dt),
        SLOT_SAMPLES,
        &SubtractConfig::DEFAULT,
    );
    assert_eq!(r.unwrap().samples, 0);
    assert_eq!(all, clean);
}

#[test]
fn deterministic_and_pinned() {
    let d = decode_near(1_234.5, 0.31);
    let mut pins = Vec::new();
    for config in [
        SubtractConfig::DEFAULT,
        SubtractConfig::DEFAULT.with_segment(480).with_drift(false),
        SubtractConfig::DEFAULT.with_segment(WAVEFORM_SAMPLES),
    ] {
        let mut a = noisy_slot();
        let mut b = noisy_slot();
        let ra = subtract(&mut a, &d, 0, &config).unwrap();
        let rb = subtract(&mut b, &d, 0, &config).unwrap();
        assert_eq!(hash(&a, &ra), hash(&b, &rb));
        pins.push(hash(&a, &ra));
    }
    // Residual bits and every estimate, pinned (new outputs: these change
    // only with the subtraction itself).
    assert_eq!(
        pins,
        [
            0x0c03_f1fa_aa51_307a,
            0x2e0b_f0eb_311b_0c28,
            0xfa07_29dc_78e8_af7d
        ],
        "{pins:x?}"
    );
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
        assert_eq!(
            subtract(&mut audio, &d, 0, &config),
            Err(Error::InvalidConfig)
        );
    }
    let bad_dt = Decode { dt: f32::NAN, ..d };
    assert_eq!(
        subtract(&mut audio, &bad_dt, 0, &SubtractConfig::DEFAULT),
        Err(Error::NonFinite)
    );
    let bad_freq = Decode {
        freq_hz: 5_990.0,
        ..d
    };
    assert_eq!(
        subtract(&mut audio, &bad_freq, 0, &SubtractConfig::DEFAULT),
        Err(Error::FrequencyOutOfRange)
    );
    // Silence: nothing to align with, nothing changed.
    let r = subtract(&mut audio, &d, 0, &SubtractConfig::DEFAULT).unwrap();
    assert_eq!(r.samples, 0);
    assert!(audio.iter().all(|x| x.to_bits() == 0));
    assert_eq!(r.residue_db(), None);
}

#[test]
fn stack_storage_is_bounded() {
    const _: () = assert!(MAX_SEGMENTS == 316 && STACK_BYTES < 8 * 1024);
    assert_eq!(STACK_BYTES, 6_836);
}
