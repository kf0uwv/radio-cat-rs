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

//! End-to-end decoder tests on synthetic slots made with this crate's
//! encoder and seeded `libm` noise (no external decoder): pinned
//! decode-rate floors and golden output hashes for both configurations,
//! overlapping, early, late and partial signals, input that starts late,
//! workspace reuse, call-table repeatability, and the SNR estimate on a
//! steep passband and beside a strong neighbour.
//!
//! The golden hashes cover every output field's bits, the text and the
//! report; they must match on every platform (the determinism promise).
//! The bulk false-decode run is in `decode_bulk.rs`.

mod common;

use cat_ft8::decode::{Band, DecodeConfig, Decoder};
use cat_ft8::message::{CallHashes, CallTable, NoHashes};
use cat_ft8::SLOT_SAMPLES;
use common::*;

const CONFIGS: [(&str, DecodeConfig); 2] =
    [("HOST", DecodeConfig::HOST), ("SMALL", DecodeConfig::SMALL)];

/// (SNR dB, slots, minimum decodes with `HOST`, with `SMALL`): the
/// counts measured when the floors were pinned (2026-10-26). Results are
/// deterministic, so a lower count is a regression; raise a floor when a
/// change improves it.
const FLOORS: [(f64, usize, usize, usize); 3] =
    [(-19.5, 20, 6, 6), (-18.5, 20, 16, 15), (-17.0, 20, 20, 20)];

#[test]
fn decode_rates_meet_their_pinned_floors() {
    for (snr, n, host, small) in FLOORS {
        let slots: Vec<_> = (0..n)
            .map(|i| random_slot(snr, 50_000 + i as u64 + (snr * -100.0) as u64))
            .collect();
        for ((name, config), floor) in CONFIGS.iter().zip([host, small]) {
            let results = par_map(n, |i| {
                let out = decode(&slots[i].0, *config);
                let hit = out.decodes.iter().any(|d| d.payload == slots[i].1);
                (hit, out.decodes.len() - usize::from(hit))
            });
            let decoded = results.iter().filter(|r| r.0).count();
            let false_decodes: usize = results.iter().map(|r| r.1).sum();
            println!("{name} SNR {snr}: {decoded}/{n} decoded, {false_decodes} false");
            assert_eq!(false_decodes, 0, "{name} SNR {snr}");
            assert!(
                decoded >= floor,
                "{name} SNR {snr}: {decoded} < floor {floor}"
            );
        }
    }
}

/// The golden slots: a busy slot (weak, strong, overlapping, early and
/// late signals, band edges), carriers in noise, and the busy slot cut at
/// 13.5 s.
fn golden_slots() -> Vec<Vec<f32>> {
    let busy = [
        Signal {
            message: "CQ K1ABC FN42",
            base_hz: 600.0,
            dt: 0.0,
            snr_db: -19.0,
        },
        Signal {
            message: "K1ABC W9XYZ -11",
            base_hz: 1_000.3,
            dt: 0.37,
            snr_db: -15.0,
        },
        Signal {
            message: "W9XYZ K1ABC R-09",
            base_hz: 1_500.0,
            dt: 0.1,
            snr_db: -8.0,
        },
        Signal {
            message: "CQ DX G4ABC IO91",
            base_hz: 1_520.0,
            dt: 0.6,
            snr_db: -8.0,
        },
        Signal {
            message: "G4ABC JA1XYZ PM95",
            base_hz: 2_200.7,
            dt: -0.2,
            snr_db: 0.0,
        },
        Signal {
            message: "TNX BOB 73 GL",
            base_hz: 2_600.0,
            dt: -1.7,
            snr_db: -10.0,
        },
        Signal {
            message: "CQ VK2ABC QF56",
            base_hz: 300.0,
            dt: 1.9,
            snr_db: -10.0,
        },
        Signal {
            message: "PA0ABC/P DL1XYZ JO62",
            base_hz: 3_090.0,
            dt: 0.2,
            snr_db: -6.0,
        },
    ];
    let (busy, _) = slot(&busy, 1);
    let mut carriers = vec![0.0f32; SLOT_SAMPLES];
    for (f, a) in [(777.7, 0.05), (1_900.0, 0.02), (2_450.3, 0.1)] {
        for (n, x) in carriers.iter_mut().enumerate() {
            *x += (a * libm::cos(2.0 * core::f64::consts::PI * f * n as f64 / 12_000.0)) as f32;
        }
    }
    add_noise(&mut carriers, NOISE_RMS, 2);
    let partial = busy[..27 * SLOT_SAMPLES / 30].to_vec();
    vec![busy, carriers, partial]
}

/// Golden hashes of [`golden_slots`] for `HOST` and `SMALL`.
const GOLDEN: [u64; 2] = [0x5eea_3477_73cf_df7d, 0x1c82_6afc_8074_f291];

#[test]
fn golden_decode_hashes() {
    let slots = golden_slots();
    for ((name, config), want) in CONFIGS.iter().zip(GOLDEN) {
        let outs = par_map(slots.len(), |i| decode(&slots[i], *config));
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for out in &outs {
            hash_output(&mut hash, out);
        }
        for (i, out) in outs.iter().enumerate() {
            println!("{name} slot {i}: {:?} {:?}", out.texts, out.report);
        }
        // Feed blocks do not matter: the busy slot in 1 013-sample blocks.
        let mut w = Work::new(Band::DEFAULT);
        let mut dec = w.decoder(*config);
        for block in slots[0].chunks(1_013) {
            dec.feed(block);
        }
        assert_eq!(run(&mut dec, &mut NoHashes), outs[0], "{name}");
        assert!(outs[0].decodes.len() >= 6, "{name}");
        assert_eq!(hash, want, "{name}: golden decode hash {hash:#018x}");
    }
}

#[test]
fn overlapping_signals_10_to_30_hz_apart() {
    for df in [10.0, 20.0, 30.0] {
        for ddt in [0.0, 0.5] {
            // Equal strength: both decode.
            let (s, p) = slot(
                &[
                    Signal {
                        message: "CQ K1ABC FN42",
                        base_hz: 1_200.0,
                        dt: 0.1,
                        snr_db: -8.0,
                    },
                    Signal {
                        message: "K1ABC W9XYZ -11",
                        base_hz: 1_200.0 + df,
                        dt: 0.1 + ddt,
                        snr_db: -8.0,
                    },
                ],
                7,
            );
            let out = decode(&s, DecodeConfig::HOST);
            assert_eq!(out.decodes.len(), 2, "{df} Hz, {ddt} s: {:?}", out.texts);
            assert!(p
                .iter()
                .all(|q| out.decodes.iter().any(|d| d.payload == *q)));
            // 12 dB apart: the strong one decodes and nothing false appears
            // (the weak one needs subtraction, a later pass).
            let (s, p) = slot(
                &[
                    Signal {
                        message: "CQ K1ABC FN42",
                        base_hz: 1_200.0,
                        dt: 0.1,
                        snr_db: 0.0,
                    },
                    Signal {
                        message: "K1ABC W9XYZ -11",
                        base_hz: 1_200.0 + df,
                        dt: 0.1 + ddt,
                        snr_db: -12.0,
                    },
                ],
                7,
            );
            let out = decode(&s, DecodeConfig::HOST);
            assert!(
                out.decodes.iter().any(|d| d.payload == p[0]),
                "{df} Hz, {ddt} s"
            );
            assert!(
                out.decodes.iter().all(|d| p.contains(&d.payload)),
                "{df} Hz, {ddt} s"
            );
        }
    }
}

#[test]
fn early_late_and_band_edge_signals_decode() {
    let cases = [
        (-2.4, 1_000.0),
        (-1.6, 1_000.0),
        (1.6, 1_000.0),
        (2.4, 1_000.0),
        (0.0, 101.0),
        (0.0, 3_099.0),
    ];
    let results = par_map(cases.len(), |i| {
        let (dt, f) = cases[i];
        let (s, p) = slot(
            &[Signal {
                message: "CQ K1ABC FN42",
                base_hz: f,
                dt,
                snr_db: -12.0,
            }],
            3,
        );
        let out = decode(&s, DecodeConfig::HOST);
        (
            out.decodes.iter().find(|d| d.payload == p[0]).copied(),
            out.decodes.len(),
        )
    });
    for ((dt, f), (d, n)) in cases.iter().zip(results) {
        let d = d.unwrap_or_else(|| panic!("dt {dt}, {f} Hz: not decoded"));
        assert_eq!(n, 1);
        assert!((f64::from(d.dt) - dt).abs() <= 0.03, "{d:?}");
        assert!((f64::from(d.freq_hz) - f).abs() <= 1.0, "{d:?}");
    }
}

#[test]
fn partial_input_decodes_before_the_slot_ends() {
    // A signal at the nominal start ends at about 13.1 s.
    let (s, p) = slot(
        &[Signal {
            message: "K1ABC W9XYZ RR73",
            base_hz: 1_444.0,
            dt: 0.0,
            snr_db: -12.0,
        }],
        9,
    );
    let mut w = Work::new(Band::DEFAULT);
    let mut dec = w.decoder(DecodeConfig::HOST);
    let cut = 13 * 12_000 + 6_000;
    for block in s[..cut].chunks(1_920) {
        dec.feed(block);
    }
    let early = run(&mut dec, &mut NoHashes);
    assert!(early.report.rows_filled < 184);
    assert_eq!(
        early.decodes.iter().map(|d| d.payload).collect::<Vec<_>>(),
        p
    );
    // Feeding the rest and decoding again gives the whole-slot result.
    dec.feed(&s[cut..]);
    let full = run(&mut dec, &mut NoHashes);
    assert_eq!(full, decode(&s, DecodeConfig::HOST));
    assert_eq!(full.report.rows_filled, 184);
}

#[test]
fn input_that_starts_late_still_decodes() {
    // The first second is missing: the signal's first 0.5 s (three
    // symbols) are lost.
    let (s, p) = slot(
        &[Signal {
            message: "CQ DX G4ABC IO91",
            base_hz: 900.0,
            dt: 0.0,
            snr_db: -10.0,
        }],
        4,
    );
    let mut w = Work::new(Band::DEFAULT);
    let mut dec = w.decoder(DecodeConfig::HOST);
    assert_eq!(dec.skip(12_000), 12_000);
    dec.feed(&s[12_000..]);
    let out = run(&mut dec, &mut NoHashes);
    assert_eq!(out.report.skipped_samples, 12_000);
    assert!(
        out.decodes.iter().any(|d| d.payload == p[0]),
        "{:?}",
        out.texts
    );
}

#[test]
fn workspaces_are_reusable_and_may_alternate() {
    let slots: Vec<Vec<f32>> = (0..4).map(|i| random_slot(-14.0, 700 + i).0).collect();
    let fresh: Vec<Output> = slots
        .iter()
        .map(|s| decode(s, DecodeConfig::HOST))
        .collect();
    let (mut a, mut b) = (Work::new(Band::DEFAULT), Work::new(Band::DEFAULT));
    let (mut da, mut db) = (a.decoder(DecodeConfig::HOST), b.decoder(DecodeConfig::HOST));
    for (i, s) in slots.iter().enumerate() {
        let dec: &mut Decoder<'_> = if i % 2 == 0 { &mut da } else { &mut db };
        dec.start_slot();
        for block in s.chunks(4_000) {
            dec.feed(block);
        }
        assert_eq!(run(dec, &mut NoHashes), fresh[i], "slot {i}");
    }
}

#[test]
fn a_seeded_call_table_gives_repeatable_text() {
    let (s, _) = slot(
        &[
            Signal {
                message: "<PJ4/K1ABC> W9XYZ",
                base_hz: 800.0,
                dt: 0.1,
                snr_db: -6.0,
            },
            Signal {
                message: "W9XYZ <PJ4/K1ABC> -11",
                base_hz: 1_700.0,
                dt: 0.3,
                snr_db: -6.0,
            },
        ],
        12,
    );
    let seeded = || {
        let mut t: CallTable<16> = CallTable::new();
        t.remember("PJ4/K1ABC");
        t
    };
    let mut w = Work::new(Band::DEFAULT);
    let mut dec = w.decoder(DecodeConfig::HOST);
    dec.feed(&s);
    let first = run(&mut dec, &mut seeded());
    let second = run(&mut dec, &mut seeded());
    let unseeded = run(&mut dec, &mut NoHashes);
    assert_eq!(first, second);
    assert_eq!(first.texts.len(), 2);
    assert!(
        first.texts.iter().all(|t| t.contains("PJ4/K1ABC")),
        "{:?}",
        first.texts
    );
    assert_ne!(first.texts, unseeded.texts);
}

/// Windowed-sinc band-pass taps (Blackman window), −6 dB at `lo` and `hi`.
fn band_pass(lo: f64, hi: f64, taps: usize) -> Vec<f64> {
    let m = (taps - 1) as f64;
    let pi = core::f64::consts::PI;
    (0..taps)
        .map(|n| {
            let k = n as f64 - m / 2.0;
            let sinc = |f: f64| {
                if k == 0.0 {
                    2.0 * f / 12_000.0
                } else {
                    libm::sin(2.0 * pi * f * k / 12_000.0) / (pi * k)
                }
            };
            let w = 0.42 - 0.5 * libm::cos(2.0 * pi * n as f64 / m)
                + 0.08 * libm::cos(4.0 * pi * n as f64 / m);
            (sinc(hi) - sinc(lo)) * w
        })
        .collect()
}

/// Power gain of `taps` at `f` Hz.
fn gain2(taps: &[f64], f: f64) -> f64 {
    let (mut re, mut im) = (0.0, 0.0);
    for (n, &h) in taps.iter().enumerate() {
        let a = 2.0 * core::f64::consts::PI * f * n as f64 / 12_000.0;
        re += h * libm::cos(a);
        im -= h * libm::sin(a);
    }
    re * re + im * im
}

#[test]
fn snr_on_a_steep_passband() {
    // Noise through a 600–1 600 Hz band-pass (−6 dB at the edges, −43 dB
    // 100 Hz outside them, over 100 dB at 2 kHz) above a white floor 45 dB
    // down. Every signal is 10 dB below the noise density at its own
    // centre frequency.
    let taps = band_pass(600.0, 1_600.0, 255);
    assert!(10.0 * libm::log10(gain2(&taps, 2_000.0)) < -44.0);
    let floor = libm::pow(10.0, -45.0 / 20.0);
    let mut white = vec![0.0f64; SLOT_SAMPLES + taps.len()];
    let mut r = Rng(100);
    for x in white.iter_mut() {
        *x = NOISE_RMS * r.gauss();
    }
    let mut s: Vec<f32> = (0..SLOT_SAMPLES)
        .map(|i| {
            taps.iter()
                .enumerate()
                .map(|(k, &h)| h * white[i + taps.len() - 1 - k])
                .sum::<f64>() as f32
        })
        .collect();
    add_noise(&mut s, NOISE_RMS * floor, 900);
    // (base Hz, message, allowed error dB): within the passband, and with
    // the signal on an edge, where the noise changes by tens of dB across
    // the reference.
    let signals = [
        (900.0, "K1ABC W9XYZ -11", 1.5),
        (1_250.0, "W9XYZ K1ABC R-09", 1.5),
        (650.0, "CQ K1ABC FN42", 3.5),
        (1_500.0, "CQ DX G4ABC IO91", 3.5),
        (1_560.0, "G4ABC JA1XYZ PM95", 3.5),
    ];
    let mut payloads = Vec::new();
    for (f, m, _) in signals {
        let local = gain2(&taps, f + 21.875) + floor * floor;
        let amp = amplitude(-10.0, NOISE_RMS * libm::sqrt(local));
        payloads.push(add_with_amplitude(
            &mut s,
            &Signal {
                message: m,
                base_hz: f,
                dt: 0.2,
                snr_db: 0.0,
            },
            amp,
        ));
    }
    let out = decode(&s, DecodeConfig::HOST);
    for ((f, _, tol), p) in signals.iter().zip(payloads) {
        let d = out
            .decodes
            .iter()
            .find(|d| d.payload == p)
            .unwrap_or_else(|| panic!("{f} Hz not decoded"));
        println!("{f} Hz: {:.1} dB", d.snr_db);
        assert!((d.snr_db + 10.0).abs() <= *tol as f32, "{f} Hz: {d:?}");
    }
}

#[test]
fn snr_beside_a_strong_neighbour() {
    // A signal 10 dB below the noise with one 10 dB stronger 60–75 Hz
    // above or below (inside one side's noise reference, not over the
    // signal). The low quantile keeps most of the neighbour out; when the
    // neighbour lies wholly inside one side's reference the estimate reads
    // up to about 2.5 dB low.
    for (i, offset) in [60.0, -60.0, 75.0, -75.0].into_iter().enumerate() {
        let (s, p) = slot(
            &[
                Signal {
                    message: "CQ K1ABC FN42",
                    base_hz: 1_300.0,
                    dt: 0.2,
                    snr_db: -10.0,
                },
                Signal {
                    message: "K1ABC W9XYZ -11",
                    base_hz: 1_300.0 + offset,
                    dt: 0.5,
                    snr_db: 0.0,
                },
            ],
            30 + i as u64,
        );
        let out = decode(&s, DecodeConfig::HOST);
        let d = out
            .decodes
            .iter()
            .find(|d| d.payload == p[0])
            .unwrap_or_else(|| panic!("{offset}"));
        println!("neighbour at {offset:+} Hz: {:.1} dB", d.snr_db);
        assert!((d.snr_db + 10.0).abs() <= 2.5, "{offset}: {d:?}");
    }
}
