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

use std::boxed::Box;
use std::string::{String, ToString};
use std::vec;
use std::vec::Vec;

use super::*;
use crate::decode::subtract;
use crate::decode::testutil::{
    add_noise, add_signal, amplitude_for, empty_slot, symbols_of, NOISE_RMS,
};
use crate::message::{CallTable, NoHashes};
use crate::SLOT_SAMPLES;

/// Caller-owned decoder buffers on the heap.
struct Bufs {
    front: Box<FrontEnd>,
    levels: Vec<u8>,
    scratch: Vec<f32>,
    cands: Vec<Candidate>,
    band: Band,
}

impl Bufs {
    fn new(band: Band) -> Self {
        Bufs {
            front: Box::new(FrontEnd::new()),
            levels: vec![0; band.spectrogram_len()],
            scratch: vec![0.0; Decoder::sync_scratch_len(band)],
            cands: vec![Candidate::default(); CANDIDATES],
            band,
        }
    }

    fn decoder(&mut self, config: DecodeConfig) -> Decoder<'_> {
        Decoder::new(
            &mut self.front,
            &mut self.levels,
            &mut self.scratch,
            &mut self.cands,
            self.band,
            config,
        )
        .unwrap()
    }
}

/// Three signals at −12 dB in seeded noise, at and off the grid.
fn busy_slot(seed: u64) -> Vec<f32> {
    let mut s = empty_slot();
    let a = amplitude_for(-12.0);
    add_signal(&mut s, &symbols_of("CQ K1ABC FN42"), 1_000.0, 0.0, a);
    add_signal(&mut s, &symbols_of("K1ABC W9XYZ -11"), 1_501.7, 0.31, a);
    add_signal(&mut s, &symbols_of("W9XYZ K1ABC RR73"), 2_203.4, -0.47, a);
    add_noise(&mut s, NOISE_RMS, seed);
    s
}

fn texts(list: &DecodeList<'_>) -> Vec<String> {
    let mut out = Vec::new();
    list.finish(&mut NoHashes, |_, t| out.push(t.unwrap().to_string()));
    out
}

#[test]
fn configuration_and_buffer_sizes_are_checked() {
    let band = Band::SMALL;
    let mut b = Bufs::new(band);
    for bad in [
        DecodeConfig::HOST.with_llr_scale(0.0),
        DecodeConfig::HOST.with_llr_scale(f32::NAN),
        DecodeConfig::HOST.with_llr_scale(DecodeConfig::MAX_LLR_SCALE * 1.001),
        DecodeConfig::HOST.with_llr_scale(f32::INFINITY),
        DecodeConfig::HOST.with_candidates(0),
        DecodeConfig::HOST.with_candidates(70_000),
    ] {
        let r = Decoder::new(
            &mut b.front,
            &mut b.levels,
            &mut b.scratch,
            &mut b.cands,
            band,
            bad,
        );
        assert_eq!(r.err(), Some(Error::InvalidConfig), "{bad:?}");
    }
    for bad in [
        DecodeConfig::HOST.with_passes(0),
        DecodeConfig::HOST.with_passes(DecodeConfig::MAX_PASSES + 1),
        DecodeConfig::HOST.with_later_candidates(0),
        DecodeConfig::HOST.with_later_candidates(70_000),
        DecodeConfig::HOST.with_subtract(SubtractConfig::DEFAULT.with_segment(100)),
        DecodeConfig::HOST.with_lone_decode_min_snr(Some(f32::NAN)),
    ] {
        let r = Decoder::new(
            &mut b.front,
            &mut b.levels,
            &mut b.scratch,
            &mut b.cands,
            band,
            bad,
        );
        assert_eq!(r.err(), Some(Error::InvalidConfig), "{bad:?}");
    }
    // The later-pass count needs room whatever the pass count.
    let later = DecodeConfig::HOST
        .with_passes(1)
        .with_later_candidates(CANDIDATES + 1);
    assert_eq!(later.candidate_capacity(), CANDIDATES + 1);
    assert_eq!(later.with_passes(2).candidate_capacity(), CANDIDATES + 1);
    assert_eq!(
        [DecodeConfig::HOST, DecodeConfig::SMALL].map(|c| c.candidate_capacity()),
        [100, 75]
    );
    // The most decodes a slot can list.
    assert_eq!(
        [
            DecodeConfig::HOST,
            DecodeConfig::HOST.with_passes(2),
            DecodeConfig::SMALL
        ]
        .map(|c| c.max_decodes()),
        [200, 150, 75]
    );
    assert_eq!(DecodeConfig::HOST.max_subtractions, None);
    // OSD-1 as gated in HOST only.
    assert_eq!(DecodeConfig::HOST.osd, Some(OsdConfig::new(10, 21)));
    assert_eq!(DecodeConfig::SMALL.osd, None);
    assert_eq!(
        DecodeConfig::HOST
            .with_max_subtractions(Some(7))
            .max_subtractions,
        Some(7)
    );
    let r = Decoder::new(
        &mut b.front,
        &mut b.levels,
        &mut b.scratch,
        &mut b.cands,
        band,
        later,
    );
    assert_eq!(r.err(), Some(Error::BufferTooSmall));
    let big = DecodeConfig::HOST.with_candidates(CANDIDATES + 1);
    let r = Decoder::new(
        &mut b.front,
        &mut b.levels,
        &mut b.scratch,
        &mut b.cands,
        band,
        big,
    );
    assert_eq!(r.err(), Some(Error::BufferTooSmall));
    // A band whose sync scores do not fit in the front end needs a scratch.
    let wide = Band::from_hz(100, 4_100).unwrap();
    let mut w = Bufs::new(wide);
    let short = &mut w.scratch[..Decoder::sync_scratch_len(wide) - 1];
    let r = Decoder::new(
        &mut w.front,
        &mut w.levels,
        short,
        &mut w.cands,
        wide,
        DecodeConfig::HOST,
    );
    assert_eq!(r.err(), Some(Error::BufferTooSmall));
    let r = Decoder::new(
        &mut b.front,
        &mut b.levels[..10],
        &mut b.scratch,
        &mut b.cands,
        band,
        DecodeConfig::HOST,
    );
    assert_eq!(r.err(), Some(Error::BufferTooSmall));
    let edge = DecodeConfig::HOST.with_llr_scale(DecodeConfig::MAX_LLR_SCALE);
    assert!(Decoder::new(
        &mut b.front,
        &mut b.levels,
        &mut b.scratch,
        &mut b.cands,
        band,
        edge
    )
    .is_ok());
    assert_eq!(DecodeConfig::default(), DecodeConfig::HOST);
    assert_eq!(
        DecodeConfig::SMALL,
        DecodeConfig::HOST
            .with_rule(CheckRule::MinSum)
            .with_candidates(75)
            .with_passes(1)
            .with_osd(None)
    );
    assert_eq!(DecodeConfig::HOST.candidates, 100);
    let h = DecodeConfig::HOST;
    assert_eq!(
        (h.passes, h.later_candidates, h.subtract.segment),
        (3, 50, 3_840)
    );
    let c = DecodeConfig::HOST.with_max_rounds(5).with_min_score(9);
    assert_eq!((c.max_rounds, c.min_score), (5, 9));
}

#[test]
fn signals_decode_with_their_fields() {
    let mut b = Bufs::new(Band::DEFAULT);
    let mut dec = b.decoder(DecodeConfig::HOST);
    dec.feed(&busy_slot(1));
    let mut store = [Decode::EMPTY; HOST_DECODES];
    let mut list = DecodeList::new(&mut store);
    let report = dec.decode(&mut CandidateScratch::new(), &mut list);
    let mut got = texts(&list);
    got.sort();
    assert_eq!(
        got,
        ["CQ K1ABC FN42", "K1ABC W9XYZ -11", "W9XYZ K1ABC RR73"]
    );
    assert_eq!(report.decoded, 3);
    assert_eq!(report.rows_filled, 184);
    assert_eq!(report.candidates_kept, DecodeConfig::HOST.candidates);
    assert_eq!(
        report.candidates_kept,
        report.decoded
            + report.duplicates
            + report.all_zero_rejected
            + report.crc_failed
            + report.not_converged
    );
    let entries = list.finish_payloads();
    // Candidate order, so increasing indices.
    assert!(entries.windows(2).all(|w| w[0].candidate < w[1].candidate));
    for e in entries {
        let (f, dt) = match unpack(&e.payload, &mut NoHashes).unwrap().as_str() {
            "CQ K1ABC FN42" => (1_000.0, 0.0),
            "K1ABC W9XYZ -11" => (1_501.7, 0.31),
            _ => (2_203.4, -0.47),
        };
        // Refined: well inside the grid's half steps (1.56 Hz, 40 ms).
        assert!((e.freq_hz - f).abs() <= 0.8, "{e:?}");
        assert!((e.dt - dt).abs() <= 0.02, "{e:?}");
        // One slot: within about 3 σ of the estimate's spread (~0.5 dB).
        assert!((e.snr_db + 12.0).abs() <= 1.5, "{e:?}");
        assert_eq!(e.message_type(), e.payload.message_type());
        assert_eq!(e.score, dec.candidates()[usize::from(e.candidate)].score);
    }
}

/// The split API with results computed in `order` on `threads` threads.
fn split_decode(
    dec: &mut Decoder<'_>,
    threads: usize,
    reverse: bool,
    store: &mut [Decode],
) -> (Vec<Decode>, DecodeReport) {
    let kept = dec.find_candidates().kept;
    let dec = &*dec;
    let mut results: Vec<(usize, CandidateResult)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                s.spawn(move || {
                    let mut scratch = Box::new(CandidateScratch::new());
                    let mut out = Vec::new();
                    let mut idx: Vec<usize> = (t..kept).step_by(threads).collect();
                    if reverse {
                        idx.reverse();
                    }
                    for i in idx {
                        out.push((i, dec.decode_candidate(i, &mut scratch).unwrap()));
                    }
                    out
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    });
    results.sort_by_key(|r| r.0);
    let ordered: Vec<CandidateResult> = results.into_iter().map(|r| r.1).collect();
    let mut list = DecodeList::new(store);
    list.merge(&ordered).unwrap();
    (list.finish_payloads().to_vec(), dec.report(&list))
}

#[test]
fn the_split_api_matches_decode_for_any_order_threads_and_blocks() {
    let slot = busy_slot(2);
    for config in [DecodeConfig::HOST, DecodeConfig::SMALL] {
        let mut b = Bufs::new(Band::DEFAULT);
        let mut dec = b.decoder(config);
        dec.feed(&slot);
        let mut store = [Decode::EMPTY; HOST_DECODES];
        let mut list = DecodeList::new(&mut store);
        let want_report = dec.decode(&mut CandidateScratch::new(), &mut list);
        let want: Vec<Decode> = list.finish_payloads().to_vec();
        assert!(want.len() >= 3, "{config:?}");

        for (threads, reverse, block) in [
            (1, true, SLOT_SAMPLES),
            (3, false, 777),
            (4, true, 1),
            (7, false, 4_096),
        ] {
            let mut b = Bufs::new(Band::DEFAULT);
            let mut dec = b.decoder(config);
            for chunk in slot.chunks(block) {
                dec.feed(chunk);
            }
            let mut store = [Decode::EMPTY; HOST_DECODES];
            let (got, report) = split_decode(&mut dec, threads, reverse, &mut store);
            let at = (config.rule, threads, block);
            assert_eq!(got, want, "{at:?}");
            assert_eq!(report, want_report, "{at:?}");
        }
    }
}

#[test]
fn results_must_be_pushed_in_index_order() {
    let mut store = [Decode::EMPTY; 4];
    let mut list = DecodeList::new(&mut store);
    let fail: CandidateResult = Err(BpFailure::NotConverged);
    assert_eq!(list.push(1, &fail), Err(Error::OutOfOrder));
    assert_eq!(list.push(0, &fail), Ok(()));
    assert_eq!(list.push(0, &fail), Err(Error::OutOfOrder));
    list.merge(&[fail, fail]).unwrap();
    assert_eq!(list.pushed(), 3);
    // A decode must carry the index it is pushed under.
    let wrong = Ok(Decode {
        candidate: 7,
        ..Decode::EMPTY
    });
    assert_eq!(list.push(3, &wrong), Err(Error::OutOfOrder));
    assert_eq!(list.pushed(), 3);
    list.clear();
    assert_eq!((list.pushed(), list.finish_payloads().len()), (0, 0));
}

#[test]
fn duplicates_keep_the_lowest_index_and_truncation_is_counted() {
    let entry = |text: &str, i: u16| Decode {
        payload: crate::pack(text, &mut NoHashes).unwrap(),
        candidate: i,
        ..Decode::EMPTY
    };
    let results: [CandidateResult; 8] = [
        Ok(entry("CQ K1ABC FN42", 0)),
        Err(BpFailure::AllZero),
        Ok(entry("CQ K1ABC FN42", 2)),
        Ok(entry("K1ABC W9XYZ -11", 3)),
        Err(BpFailure::Crc),
        Ok(entry("W9XYZ K1ABC RR73", 5)),
        Ok(entry("W9XYZ K1ABC RR73", 6)),
        Err(BpFailure::NotConverged),
    ];
    let mut store = [Decode::EMPTY; 2];
    let mut list = DecodeList::new(&mut store);
    list.merge(&results).unwrap();
    // The same results merged in three chunks give the same list and counts.
    let mut store2 = [Decode::EMPTY; 2];
    let mut chunked = DecodeList::new(&mut store2);
    for part in [&results[..1], &results[1..5], &results[5..]] {
        chunked.merge(part).unwrap();
    }
    assert_eq!(chunked.finish_payloads(), list.finish_payloads());
    assert_eq!(chunked.tally(), list.tally());
    let kept: Vec<u16> = list.finish_payloads().iter().map(|d| d.candidate).collect();
    assert_eq!(kept, [0, 3]);
    let t = list.tally();
    assert_eq!(
        (
            t.duplicates,
            t.truncated,
            t.all_zero,
            t.crc_failed,
            t.not_converged
        ),
        (1, 2, 1, 1, 1)
    );
}

#[test]
fn a_carrier_converges_on_the_rejected_all_zero_word() {
    let mut slot = empty_slot();
    for (n, x) in slot.iter_mut().enumerate() {
        *x = 0.1 * libm::cosf(2.0 * core::f32::consts::PI * 1_234.0 * n as f32 / 12_000.0);
    }
    add_noise(&mut slot, NOISE_RMS, 5);
    let mut b = Bufs::new(Band::DEFAULT);
    // The full buffer (`CANDIDATES`): with the presets' fewer candidates the
    // carrier's candidates rank below the cut-off on this slot, and the
    // guard is not exercised.
    let mut dec = b.decoder(DecodeConfig::HOST.with_candidates(CANDIDATES));
    dec.feed(&slot);
    let mut store = [Decode::EMPTY; 8];
    let mut list = DecodeList::new(&mut store);
    let report = dec.decode(&mut CandidateScratch::new(), &mut list);
    assert_eq!(report.decoded, 0);
    assert!(report.all_zero_rejected > 0, "{report:?}");
}

#[test]
fn finish_reports_payloads_it_cannot_unpack() {
    // i3 = 0, n3 = 3: ARRL Field Day, not unpacked by this release.
    let payload = Payload::from_bits(3 << 3).unwrap();
    let mut store = [Decode::EMPTY; 2];
    let mut list = DecodeList::new(&mut store);
    list.push(
        0,
        &Ok(Decode {
            payload,
            ..Decode::EMPTY
        }),
    )
    .unwrap();
    let mut seen = Vec::new();
    list.finish(&mut NoHashes, |d, t| {
        seen.push((d.payload, t.map(|s| s.to_string())))
    });
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, payload);
    assert!(matches!(seen[0].1, Err(Error::Unsupported(_))), "{seen:?}");
}

#[test]
fn finish_unpacks_with_the_callers_hash_table() {
    // A hashed call is resolved from a table filled by earlier decodes.
    let mut table: CallTable<16> = CallTable::new();
    let enc = crate::encode("<PJ4/K1ABC> W9XYZ", &mut table).unwrap();
    let mut store = [Decode::EMPTY; 2];
    let mut list = DecodeList::new(&mut store);
    list.push(
        0,
        &Ok(Decode {
            payload: enc.payload,
            ..Decode::EMPTY
        }),
    )
    .unwrap();
    let mut with_table = String::new();
    list.finish(&mut table, |_, t| with_table = t.unwrap().to_string());
    let mut without = String::new();
    list.finish(&mut CallTable::<16>::new(), |_, t| {
        without = t.unwrap().to_string()
    });
    assert!(with_table.contains("PJ4/K1ABC"), "{with_table}");
    assert_ne!(with_table, without);
}

#[test]
fn a_new_slot_matches_a_fresh_decoder_and_partial_slots_decode() {
    let (first, second) = (busy_slot(3), busy_slot(4));
    let decode_all = |dec: &mut Decoder<'_>| {
        let mut store = [Decode::EMPTY; HOST_DECODES];
        let mut list = DecodeList::new(&mut store);
        let report = dec.decode(&mut CandidateScratch::new(), &mut list);
        (list.finish_payloads().to_vec(), report)
    };
    let mut b = Bufs::new(Band::DEFAULT);
    let mut reused = b.decoder(DecodeConfig::HOST);
    reused.feed(&first);
    decode_all(&mut reused);
    reused.start_slot();
    assert!(reused.candidates().is_empty());
    reused.feed(&second);
    let again = decode_all(&mut reused);
    let mut b2 = Bufs::new(Band::DEFAULT);
    let mut fresh = b2.decoder(DecodeConfig::HOST);
    fresh.feed(&second);
    assert_eq!(again, decode_all(&mut fresh));

    // The first 14 s only (the signals end by 13.2 s).
    fresh.start_slot();
    fresh.feed(&second[..14 * 12_000]);
    let (partial, report) = decode_all(&mut fresh);
    assert!(report.rows_filled < 184);
    assert_eq!(partial.len(), again.0.len());
}

/// A one-pole low-pass filter's power gain at `f` Hz (pole `a`).
fn one_pole_gain2(a: f64, f: f64) -> f64 {
    let w = 2.0 * core::f64::consts::PI * f / 12_000.0;
    (1.0 - a) * (1.0 - a) / (1.0 - 2.0 * a * libm::cos(w) + a * a)
}

#[test]
fn the_snr_estimate_uses_local_noise() {
    // Noise through a one-pole low-pass: its density falls by about 16 dB
    // from 500 to 2 400 Hz. Two signals, each 10 dB below the noise in
    // 2500 Hz *at its own frequency*, read about −10 dB each; a whole-band
    // noise figure would put them about 16 dB apart.
    let a = 0.9f64;
    let mut slot = empty_slot();
    let mut noise = crate::decode::testutil::Noise(11);
    let mut y = 0.0f64;
    for x in slot.iter_mut() {
        y = a * y + (1.0 - a) * NOISE_RMS * 10.0 * noise.gauss();
        *x = y as f32;
    }
    let rms = NOISE_RMS * 10.0;
    for (msg, f) in [("CQ K1ABC FN42", 500.0f64), ("K1ABC W9XYZ -11", 2_400.0)] {
        let n2500 = rms * rms * one_pole_gain2(a, f + 21.875) * 2500.0 / 6000.0;
        let amp = libm::sqrt(2.0 * n2500 * libm::pow(10.0, -1.0)) as f32;
        add_signal(&mut slot, &symbols_of(msg), f as f32, 0.0, amp);
    }
    let mut b = Bufs::new(Band::DEFAULT);
    let mut dec = b.decoder(DecodeConfig::HOST);
    dec.feed(&slot);
    let mut store = [Decode::EMPTY; 8];
    let mut list = DecodeList::new(&mut store);
    dec.decode(&mut CandidateScratch::new(), &mut list);
    let entries = list.finish_payloads();
    assert_eq!(entries.len(), 2);
    for e in entries {
        std::println!("{:.0} Hz: {:.2} dB", e.freq_hz, e.snr_db);
        // The reference spans 25–100 Hz either side, so on a slope this
        // steep it can be off by about 1 dB.
        assert!((e.snr_db + 10.0).abs() <= 1.5, "{e:?}");
    }
}

#[test]
fn the_snr_estimate_tracks_the_signal_level() {
    let mut last = f32::MIN;
    for snr in [-16.0, -8.0, 0.0, 10.0, 20.0] {
        let mut slot = empty_slot();
        add_signal(
            &mut slot,
            &symbols_of("CQ K1ABC FN42"),
            1_234.0,
            0.1,
            amplitude_for(snr),
        );
        add_noise(&mut slot, NOISE_RMS, 21);
        let mut b = Bufs::new(Band::DEFAULT);
        let mut dec = b.decoder(DecodeConfig::HOST);
        dec.feed(&slot);
        let mut store = [Decode::EMPTY; 8];
        let mut list = DecodeList::new(&mut store);
        dec.decode(&mut CandidateScratch::new(), &mut list);
        let e = list.finish_payloads()[0];
        assert!((f64::from(e.snr_db) - snr).abs() <= 1.5, "{snr}: {e:?}");
        assert!(e.snr_db > last);
        last = e.snr_db;
    }
}

/// Everything a decode leaves, as text, so that a difference in any bit of
/// a number (the sign of zero included) shows: the list, the report and
/// each pass's counts. The residual is compared by `same_bits`.
fn outcome(list: &DecodeList<'_>, report: &PassesReport, per: &[PassReport]) -> String {
    std::format!("{:?} {report:?} {per:?}", list.finish_payloads())
}

/// The slots of the tests below, different in content and length: a
/// crowded slot, an empty one, noise alone, a slot cut short, a busy one.
fn varied_slots() -> Vec<Vec<f32>> {
    let mut noise = empty_slot();
    add_noise(&mut noise, NOISE_RMS, 11);
    let short = busy_slot(13)[..11 * 12_000].to_vec();
    vec![
        crowded_slot(20, 12).0,
        empty_slot(),
        noise,
        short,
        busy_slot(14),
        crowded_slot(8, 15).0,
    ]
}

#[test]
fn successive_slots_on_one_decoder_match_fresh_decoders() {
    let slots = varied_slots();
    let configs = [
        (DecodeConfig::HOST, Band::DEFAULT),
        (DecodeConfig::HOST.with_passes(1), Band::DEFAULT),
        (DecodeConfig::SMALL, Band::SMALL),
    ];
    for (config, band) in configs {
        let mut b = Bufs::new(band);
        let mut reused = b.decoder(config);
        let mut scratch = Box::new(CandidateScratch::new());
        let mut decoded = 0;
        for (n, slot) in slots.iter().enumerate() {
            // One decoder and one scratch for every slot.
            let mut store = vec![Decode::EMPTY; HOST_DECODES];
            let mut list = DecodeList::new(&mut store);
            let mut audio = slot.clone();
            let mut per = Vec::new();
            let report = reused.decode_passes_staged(&mut audio, 0, &mut scratch, &mut list, |e| {
                if let PassEvent::Done(r) = e {
                    per.push(*r);
                }
                ControlFlow::Continue(())
            });
            // Fresh buffers for this slot alone.
            let mut b2 = Bufs::new(band);
            let mut fresh = b2.decoder(config);
            let mut store2 = vec![Decode::EMPTY; HOST_DECODES];
            let mut list2 = DecodeList::new(&mut store2);
            let mut audio2 = slot.clone();
            let mut per2 = Vec::new();
            let report2 = fresh.decode_passes_staged(
                &mut audio2,
                0,
                &mut Box::new(CandidateScratch::new()),
                &mut list2,
                |e| {
                    if let PassEvent::Done(r) = e {
                        per2.push(*r);
                    }
                    ControlFlow::Continue(())
                },
            );
            assert_eq!(
                outcome(&list, &report, &per),
                outcome(&list2, &report2, &per2),
                "{config:?} slot {n}"
            );
            assert!(same_bits(&audio, &audio2), "{config:?} slot {n}");
            // The rows the last pass left.
            for r in 0..crate::decode::ROWS {
                assert_eq!(
                    reused.spectrogram().row(r),
                    fresh.spectrogram().row(r),
                    "{config:?} slot {n} row {r}"
                );
            }
            decoded += report.report.decoded;
        }
        assert!(decoded >= 20, "{config:?}: {decoded} decodes");
    }
}

/// `decode` on `dec`, returning what it listed and reported.
fn decode_text(dec: &mut Decoder<'_>) -> String {
    let mut store = vec![Decode::EMPTY; HOST_DECODES];
    let mut list = DecodeList::new(&mut store);
    let report = dec.decode(&mut CandidateScratch::new(), &mut list);
    std::format!("{:?} {report:?}", list.finish_payloads())
}

#[test]
fn a_search_and_candidates_mid_slot_leave_the_feeding_undisturbed() {
    // The split API may search whenever it likes, so the samples held for
    // the next row have to survive a search and candidates. Stops before
    // the row origin, inside the first window, inside a later one, on a row
    // boundary and after the last row.
    let slot = crowded_slot(12, 21).0;
    for band in [Band::DEFAULT, Band::SMALL] {
        let mut b = Bufs::new(band);
        let mut whole = b.decoder(DecodeConfig::HOST);
        whole.feed(&slot);
        let want = decode_text(&mut whole);
        let want_rows: Vec<Vec<u8>> = (0..crate::decode::ROWS)
            .map(|r| whole.spectrogram().row(r).unwrap().to_vec())
            .collect();
        for stop in [100, 2_000, 90_123, 240 + 960 * 100 + 3_840, 179_900] {
            let mut b2 = Bufs::new(band);
            let mut interrupted = b2.decoder(DecodeConfig::HOST);
            interrupted.feed(&slot[..stop]);
            let mid = decode_text(&mut interrupted);
            // What a decoder fed only this far finds.
            let mut b3 = Bufs::new(band);
            let mut prefix = b3.decoder(DecodeConfig::HOST);
            prefix.feed(&slot[..stop]);
            assert_eq!(mid, decode_text(&mut prefix), "stop {stop}");
            // The rest of the slot, as if nothing had happened.
            interrupted.feed(&slot[stop..]);
            for (r, row) in want_rows.iter().enumerate() {
                assert_eq!(
                    interrupted.spectrogram().row(r),
                    Some(&row[..]),
                    "stop {stop} row {r}"
                );
            }
            assert_eq!(want, decode_text(&mut interrupted), "stop {stop}");
        }
    }
}

/// The FFT work area, set to `stale`.
fn spoil_work_area(dec: &mut Decoder<'_>, stale: f32) {
    dec.spec.rows_and_work().1.fill(stale);
}

#[test]
fn what_the_shared_cells_held_does_not_reach_the_result() {
    // The cells hold scores between rows and transform values during one.
    // Set to values that would show if read, at every point of a slot they
    // are free: between all rows, before a search, before a further pass's
    // feed. Each is compared with a decoder left alone.
    let slot = crowded_slot(12, 22).0;
    for config in [DecodeConfig::HOST, DecodeConfig::SMALL] {
        let mut b = Bufs::new(Band::DEFAULT);
        let mut plain = b.decoder(config);
        plain.feed(&slot);
        let want = decode_text(&mut plain);
        for stale in [f32::NAN, f32::INFINITY, 1.0e30, -1.0, f32::from_bits(1)] {
            let mut b2 = Bufs::new(Band::DEFAULT);
            let mut dec = b2.decoder(config);
            for block in slot.chunks(960) {
                spoil_work_area(&mut dec, stale);
                dec.feed(block);
            }
            spoil_work_area(&mut dec, stale);
            assert_eq!(want, decode_text(&mut dec), "{stale}");
        }
    }
    // Passes: from `decode_further_passes`, with the cells spoiled between
    // the first pass and the rest.
    let config = DecodeConfig::HOST;
    let (want, want_report, want_residual, _) = passes(&slot, 0, config);
    let mut b = Bufs::new(Band::DEFAULT);
    let mut dec = b.decoder(config);
    dec.feed(&slot);
    let mut store = vec![Decode::EMPTY; HOST_DECODES];
    let mut list = DecodeList::new(&mut store);
    let mut scratch = Box::new(CandidateScratch::new());
    dec.decode(&mut scratch, &mut list);
    spoil_work_area(&mut dec, f32::NAN);
    let mut audio = slot.clone();
    let report = dec.decode_further_passes(&mut audio, 0, &mut scratch, &mut list);
    assert_eq!(list.finish_payloads(), &want[..]);
    assert_eq!(report, want_report);
    assert!(same_bits(&audio, &want_residual));
}

#[test]
fn a_search_finds_the_same_wherever_its_scores_are_kept() {
    // The presets' bands: the front end against a separate scratch.
    let slot = crowded_slot(12, 23).0;
    for band in [Band::DEFAULT, Band::SMALL] {
        let mut b = Bufs::new(band);
        let mut dec = b.decoder(DecodeConfig::HOST);
        dec.feed(&slot);
        let report = dec.find_candidates();
        let mut separate = vec![f32::NAN; sync::scratch_len(band.base_bins())];
        let mut out = vec![Candidate::default(); DecodeConfig::HOST.candidates];
        let other = sync::find_candidates(dec.spectrogram(), 0, &mut separate, &mut out).unwrap();
        assert_eq!(report, other);
        assert_eq!(dec.candidates(), &out[..other.kept]);
        assert!(other.kept > 0);
    }
    // Either side of the limit: the widest band `Band::from_hz` keeps in
    // the front end, the narrowest it does not. Signals in both bands
    // decode the same, and the wider one's scratch is all that it uses:
    // it does not read what the scratch held.
    let (narrower, wider) = (
        Band::from_hz(100, 4_075).unwrap(),
        Band::from_hz(100, 4_100).unwrap(),
    );
    assert_eq!(Decoder::sync_scratch_len(narrower), 0);
    assert_eq!(Decoder::sync_scratch_len(wider), 3 * wider.base_bins());
    let mut slot = busy_slot(24);
    add_signal(
        &mut slot,
        &symbols_of("CQ W1AW FN31"),
        3_500.0,
        0.1,
        amplitude_for(-12.0),
    );
    let mut text = Vec::new();
    for (band, stale) in [(narrower, 0.0), (wider, 0.0), (wider, f32::NAN)] {
        let mut b = Bufs::new(band);
        b.scratch.fill(stale);
        let mut dec = b.decoder(DecodeConfig::SMALL);
        dec.feed(&slot);
        let mut store = vec![Decode::EMPTY; HOST_DECODES];
        let mut list = DecodeList::new(&mut store);
        dec.decode(&mut CandidateScratch::new(), &mut list);
        let mut found = texts(&list);
        found.sort();
        text.push(found);
    }
    assert!(text[0].iter().any(|t| t == "CQ W1AW FN31"), "{text:?}");
    assert_eq!(text[0].len(), 4);
    assert_eq!(text[0], text[1]);
    assert_eq!(text[1], text[2]);
}

#[test]
fn memory_per_item_is_bounded() {
    use core::mem::size_of;
    std::println!(
        "Decode {} B, CandidateScratch {} B, Candidate {} B, Decoder {} B",
        size_of::<Decode>(),
        size_of::<CandidateScratch>(),
        size_of::<Candidate>(),
        size_of::<Decoder<'static>>()
    );
    assert!(size_of::<Decode>() <= 48);
    assert!(size_of::<CandidateScratch>() <= 5_200);
    assert!(size_of::<Candidate>() <= 8);
    // Borrows, counters and the configuration (200 B with 64-bit
    // pointers).
    assert!(size_of::<Decoder<'static>>() <= 224);
}

#[test]
fn debug_output_leaves_out_the_buffers() {
    let mut b = Bufs::new(Band::DEFAULT);
    let mut dec = b.decoder(DecodeConfig::HOST);
    dec.feed(&busy_slot(1));
    let mut store = [Decode::EMPTY; 8];
    let mut list = DecodeList::new(&mut store);
    dec.decode(&mut CandidateScratch::new(), &mut list);
    let text = std::format!(
        "{dec:?} {list:?} {:?} {:?}",
        CandidateScratch::new(),
        FrontEnd::new()
    );
    assert!(text.len() < 4_000, "{} bytes", text.len());
    assert!(
        text.contains("rows_filled: 184") && text.contains("capacity: 8"),
        "{text}"
    );
}

#[test]
fn staged_decoding_reports_its_stages_and_gives_the_same_result() {
    let mut b = Bufs::new(Band::DEFAULT);
    let mut dec = b.decoder(DecodeConfig::HOST);
    dec.feed(&busy_slot(4));
    let kept = dec.find_candidates().kept;
    let mut b_off = Bufs::new(Band::DEFAULT);
    let mut off = b_off.decoder(DecodeConfig::HOST.with_osd(None));
    off.feed(&busy_slot(4));
    assert_eq!(off.find_candidates().kept, kept);
    let mut off_scratch = Box::new(CandidateScratch::new());
    let mut scratch = CandidateScratch::new();
    let mut decoded = 0;
    for i in 0..kept {
        let mut stages = Vec::new();
        let staged = dec
            .decode_candidate_staged(i, &mut scratch, |s| stages.push(s))
            .unwrap();
        assert_eq!(
            staged,
            dec.decode_candidate(i, &mut scratch).unwrap(),
            "candidate {i}"
        );
        let mut want = vec![Stage::Refine, Stage::SoftBits, Stage::BeliefPropagation];
        // HOST runs OSD-1 on the first 10 candidates belief propagation
        // fails on (the same decoder with OSD off says which those are).
        if i < 10 && matches!(off.decode_candidate(i, &mut off_scratch), Some(Err(_))) {
            want.push(Stage::Osd);
        }
        if staged.is_ok() {
            want.push(Stage::Snr);
            decoded += 1;
        }
        assert_eq!(stages, want, "candidate {i}");
    }
    assert!(decoded >= 3);
    assert!(dec
        .decode_candidate_staged(kept, &mut scratch, |_| {})
        .is_none());
    for (i, stage) in Stage::ALL.into_iter().enumerate() {
        assert_eq!(stage.index(), i);
    }
    assert_eq!(
        Stage::ALL.map(Stage::name),
        ["refine", "soft_bits", "belief_propagation", "osd", "snr"]
    );
}

/// A crowded slot: `n` free-text messages at −16…+6 dB, 200–2 800 Hz,
/// −0.3…+0.7 s (seeded), in noise. Returns the slot and the texts.
fn crowded_slot(n: usize, seed: u64) -> (Vec<f32>, Vec<String>) {
    let mut r = crate::decode::testutil::Noise(seed);
    let mut s = empty_slot();
    let mut sent = Vec::new();
    for k in 0..n {
        let text = std::format!("CROWD {k:03} {seed}");
        let snr = -16.0 + 22.0 * r.uniform();
        let freq = (200.0 + 2_600.0 * r.uniform()) as f32;
        let dt = (-0.3 + r.uniform()) as f32;
        add_signal(&mut s, &symbols_of(&text), freq, dt, amplitude_for(snr));
        sent.push(text);
    }
    add_noise(&mut s, NOISE_RMS, seed ^ 0x55);
    (s, sent)
}

/// `decode_passes` with `config` on a copy of `audio`: the list, the
/// report, the residual and every pass's counts.
fn passes(
    audio: &[f32],
    skip: usize,
    config: DecodeConfig,
) -> (Vec<Decode>, PassesReport, Vec<f32>, Vec<PassReport>) {
    passes_into(audio, skip, config, HOST_DECODES, |_| {
        ControlFlow::Continue(())
    })
}

/// [`passes`] into a list of `capacity` entries, `event` also seeing each
/// event (and deciding whether to go on).
fn passes_into(
    audio: &[f32],
    skip: usize,
    config: DecodeConfig,
    capacity: usize,
    mut event: impl FnMut(&PassEvent<'_>) -> ControlFlow<()>,
) -> (Vec<Decode>, PassesReport, Vec<f32>, Vec<PassReport>) {
    let mut b = Bufs::new(Band::DEFAULT);
    let mut dec = b.decoder(config);
    let mut store = vec![Decode::EMPTY; capacity];
    let mut list = DecodeList::new(&mut store);
    let mut residual = audio.to_vec();
    let mut per = Vec::new();
    let report = dec.decode_passes_staged(
        &mut residual,
        skip,
        &mut Box::new(CandidateScratch::new()),
        &mut list,
        |e| {
            if let PassEvent::Done(r) = e {
                per.push(*r);
            }
            event(&e)
        },
    );
    (list.finish_payloads().to_vec(), report, residual, per)
}

/// Whether two sample buffers are equal bit for bit.
fn same_bits(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

/// The slot totals the pass reports add up to: summed search and list
/// counts, pass 1's input counters.
fn summed(per: &[PassReport]) -> DecodeReport {
    let mut t = per[0].report;
    for p in &per[1..] {
        let r = &p.report;
        t.candidates_found += r.candidates_found;
        t.candidates_kept += r.candidates_kept;
        t.candidates_dropped += r.candidates_dropped;
        t.decoded += r.decoded;
        t.decodes_truncated += r.decodes_truncated;
        t.duplicates += r.duplicates;
        t.all_zero_rejected += r.all_zero_rejected;
        t.crc_failed += r.crc_failed;
        t.not_converged += r.not_converged;
    }
    t
}

#[test]
fn one_pass_is_feed_and_decode_and_leaves_the_audio_alone() {
    for config in [DecodeConfig::HOST.with_passes(1), DecodeConfig::SMALL] {
        for skip in [0, 12_000] {
            let mut slot = busy_slot(4);
            slot[..skip].fill(0.0);
            let mut b = Bufs::new(Band::DEFAULT);
            let mut dec = b.decoder(config);
            dec.skip(skip);
            dec.feed(&slot[skip..]);
            let mut store = [Decode::EMPTY; HOST_DECODES];
            let mut list = DecodeList::new(&mut store);
            let want = dec.decode(&mut CandidateScratch::new(), &mut list);
            let (got, report, residual, per) = passes(&slot, skip, config);
            assert_eq!(got, list.finish_payloads(), "{config:?} {skip}");
            assert_eq!(report.report, want, "{config:?} {skip}");
            assert_eq!(
                (report.passes_run, report.end, report.subtracted),
                (1, PassesEnd::AllPasses, 0)
            );
            assert!(same_bits(&residual, &slot));
            assert_eq!(per.len(), 1);
            assert_eq!(per[0].report, want);
        }
    }
}

#[test]
fn further_passes_add_decodes_once_each() {
    let (slot, sent) = crowded_slot(30, 9);
    let host = DecodeConfig::HOST.with_passes(1);
    let (one, r1, _, _) = passes(&slot, 0, host);
    let (two, r2, res2, per2) = passes(&slot, 0, host.with_passes(2));
    let (three, r3, res3, per3) = passes(&slot, 0, host.with_passes(3));
    std::println!(
        "decoded of {}: {} / {} / {}; passes {per3:?}",
        sent.len(),
        one.len(),
        two.len(),
        three.len()
    );
    // Each run starts with the one before it, and adds.
    assert_eq!(two[..one.len()], one[..]);
    assert_eq!(three[..two.len()], two[..]);
    assert!(two.len() > one.len() && three.len() >= two.len());
    // The slot's report is the pass reports' totals (input counters from
    // pass 1), and its decoded count the list's.
    assert_eq!(r2.report, summed(&per2));
    assert_eq!(r3.report, summed(&per3));
    assert_eq!(per2[0].report, r1.report);
    assert_eq!(
        (r2.report.decoded, r3.report.decoded),
        (two.len(), three.len())
    );
    assert_eq!(
        r3.report.candidates_kept,
        per3.iter().map(|p| p.report.candidates_kept).sum::<usize>()
    );
    assert_eq!(
        (r2.passes_run, r3.passes_run, r3.end),
        (2, 3, PassesEnd::AllPasses)
    );
    assert_eq!(
        r3.subtracted,
        per3.iter().map(|p| p.subtracted).sum::<usize>()
    );
    assert_eq!((r3.subtract_failed, r3.stopped_early()), (0, false));
    // Every decode was sent, and none is listed twice.
    let mut payloads: Vec<u128> = three.iter().map(|d| d.payload.bits()).collect();
    payloads.sort_unstable();
    payloads.dedup();
    assert_eq!(payloads.len(), three.len());
    let mut store = [Decode::EMPTY; HOST_DECODES];
    let mut list = DecodeList::new(&mut store);
    for d in &three {
        list.push(0, &Ok(Decode { candidate: 0, ..*d })).unwrap();
        list.next_pass();
    }
    for t in texts(&list) {
        assert!(sent.contains(&t), "{t}");
    }
    // The pass reports add up and say where each pass began.
    assert_eq!(per2.len(), 2);
    assert_eq!(per3[..2], per2[..]);
    assert_eq!(per3[1].first, one.len());
    assert_eq!(per3[1].subtracted, one.len());
    assert_eq!(per3[0].report.decoded + per3[1].report.decoded, two.len());
    // The residual of two passes is the slot less pass 1's decodes, so the
    // third pass starts from it and subtracts pass 2's.
    assert!(res2.iter().zip(&slot).any(|(a, b)| a != b));
    assert!(res3.iter().zip(&res2).any(|(a, b)| a != b));
    // Deterministic.
    let again = passes(&slot, 0, host.with_passes(3));
    assert_eq!((again.0, again.1, again.3), (three, r3, per3));
    assert!(same_bits(&again.2, &res3));
}

#[test]
fn a_pass_that_adds_nothing_ends_the_loop() {
    let mut noise = empty_slot();
    add_noise(&mut noise, NOISE_RMS, 3);
    let (got, report, residual, per) = passes(&noise, 0, DecodeConfig::HOST.with_passes(3));
    assert!(got.is_empty());
    assert_eq!(per.len(), 1);
    assert_eq!((report.passes_run, report.end), (1, PassesEnd::NothingNew));
    assert!(!report.stopped_early());
    assert!(same_bits(&residual, &noise));
    // One signal: pass 2 finds nothing new, so there is no pass 3.
    let mut one = noise.clone();
    add_signal(
        &mut one,
        &symbols_of("CQ K1ABC FN42"),
        1_200.0,
        0.1,
        amplitude_for(-5.0),
    );
    let (got, report, _, per) = passes(&one, 0, DecodeConfig::HOST.with_passes(3));
    assert_eq!(got.len(), 1);
    assert_eq!(per.len(), 2);
    assert_eq!((report.passes_run, report.end), (2, PassesEnd::NothingNew));
    assert_eq!(
        (
            per[1].subtracted,
            per[1].report.decoded,
            per[1].earlier_duplicates
        ),
        (1, 0, 0)
    );
}

#[test]
fn a_payload_sent_twice_is_found_again_as_an_earlier_duplicate() {
    // The same message on two frequencies: pass 1 lists it once (the
    // second copy is a duplicate) and subtracts only the listed copy, so
    // pass 2 decodes the other copy: a duplicate of an earlier pass.
    let mut slot = empty_slot();
    let sym = symbols_of("CQ K1ABC FN42");
    add_signal(&mut slot, &sym, 900.0, 0.1, amplitude_for(-5.0));
    add_signal(&mut slot, &sym, 2_100.0, 0.3, amplitude_for(-5.0));
    add_noise(&mut slot, NOISE_RMS, 21);
    let (got, report, _, per) = passes(&slot, 0, DecodeConfig::HOST);
    assert_eq!(got.len(), 1);
    assert_eq!(per.len(), 2);
    assert!(per[0].report.duplicates >= 1, "{per:?}");
    assert_eq!(per[0].earlier_duplicates, 0);
    assert!(per[1].earlier_duplicates >= 1, "{per:?}");
    assert_eq!(per[1].earlier_duplicates, per[1].report.duplicates);
    assert_eq!(report.end, PassesEnd::NothingNew);
    assert_eq!(
        report.report.duplicates,
        per[0].report.duplicates + per[1].report.duplicates
    );
}

#[test]
fn a_full_list_ends_the_passes() {
    let (slot, _) = crowded_slot(30, 9);
    let (one, r1, _, _) = passes(&slot, 0, DecodeConfig::HOST.with_passes(1));
    assert!(one.len() > 6);
    // Pass 1 overflows a list of 6: no further pass runs (it could only
    // repeat what was truncated), and every count is pass 1's.
    let (got, report, residual, per) = passes_into(&slot, 0, DecodeConfig::HOST, 6, |_| {
        ControlFlow::Continue(())
    });
    assert_eq!(got[..], one[..6]);
    assert_eq!(per.len(), 1);
    assert_eq!((report.passes_run, report.end), (1, PassesEnd::ListFull));
    assert!(report.stopped_early());
    // (A truncated payload decoded again counts as truncated again.)
    let r = &report.report;
    assert_eq!(r.decoded, 6 + r.decodes_truncated);
    assert!(r.decoded >= r1.report.decoded && r.decodes_truncated >= one.len() - 6);
    assert!(same_bits(&residual, &slot));
    // Filled exactly, with nothing truncated: the same.
    let n = one.len();
    let (got, report, _, _) = passes_into(&slot, 0, DecodeConfig::HOST, n, |_| {
        ControlFlow::Continue(())
    });
    assert_eq!(got, one);
    assert_eq!(
        (report.end, report.report.decodes_truncated),
        (PassesEnd::ListFull, 0)
    );
    // One entry more: pass 2 runs and fills it.
    let (got, report, _, per) = passes_into(&slot, 0, DecodeConfig::HOST, n + 1, |_| {
        ControlFlow::Continue(())
    });
    assert_eq!(got.len(), n + 1);
    assert_eq!(per.len(), 2);
    assert_eq!(report.end, PassesEnd::ListFull);
    assert_eq!(report.report, summed(&per));
}

#[test]
fn the_subtraction_cap_runs_a_prefix_of_the_passes() {
    let (slot, _) = crowded_slot(30, 9);
    let host = DecodeConfig::HOST;
    let (all, all_report, all_res, all_per) = passes(&slot, 0, host);
    assert_eq!(all_per.len(), 3);
    let first = all_per[1].first;
    // No subtraction allowed: pass 1 only.
    let (got, report, residual, per) = passes(&slot, 0, host.with_max_subtractions(Some(0)));
    assert_eq!(got[..], all[..first]);
    assert_eq!(per[..], all_per[..1]);
    assert_eq!(
        (report.passes_run, report.end),
        (1, PassesEnd::SubtractionCap)
    );
    assert!(report.stopped_early());
    assert!(same_bits(&residual, &slot));
    // Room for pass 1's decodes but not pass 2's as well: passes 1 and 2,
    // exactly as without the cap.
    let cap = Some(u16::try_from(first).unwrap());
    let (got, report, _, per) = passes(&slot, 0, host.with_max_subtractions(cap));
    assert_eq!(got[..], all[..all_per[2].first]);
    assert_eq!(per[..], all_per[..2]);
    assert_eq!(
        (report.passes_run, report.end, report.subtracted),
        (2, PassesEnd::SubtractionCap, first)
    );
    assert_eq!(report.report, summed(&per));
    // A cap the slot does not reach changes nothing.
    let cap = Some(u16::try_from(all_report.subtracted).unwrap());
    let (got, report, residual, _) = passes(&slot, 0, host.with_max_subtractions(cap));
    assert_eq!((got, report), (all, all_report));
    assert!(same_bits(&residual, &all_res));
}

#[test]
fn a_weak_lone_decode_ends_the_passes_when_asked() {
    // One signal at each SNR, with and without the minimum.
    let lone = |snr: f64| {
        let mut slot = empty_slot();
        add_noise(&mut slot, NOISE_RMS, 3);
        add_signal(
            &mut slot,
            &symbols_of("CQ K1ABC FN42"),
            1_200.0,
            0.1,
            amplitude_for(snr),
        );
        slot
    };
    let always = DecodeConfig::HOST.with_lone_decode_min_snr(None);
    for (snr, min, skipped) in [
        (-5.0, Some(-17.0), false),
        (-19.0, Some(-17.0), true),
        (-19.0, None, false),
        (-19.0, Some(-30.0), false),
        (-5.0, Some(30.0), true),
    ] {
        let slot = lone(snr);
        let config = always.with_lone_decode_min_snr(min);
        let (got, report, residual, per) = passes(&slot, 0, config);
        let (all, _, _, all_per) = passes(&slot, 0, always);
        // The same decodes either way (nothing hides behind one signal).
        assert_eq!(got.len(), 1, "{snr} {min:?}");
        assert_eq!(got, all);
        // The decode's SNR is against the minimum as the rule reads it.
        let s = got[0].snr_db;
        assert_eq!(skipped, min.is_some_and(|m| s < m), "{snr} {min:?} {s}");
        // At the boundary: a minimum equal to the decode's SNR runs pass
        // 2, one just above it does not.
        let at = passes(&slot, 0, always.with_lone_decode_min_snr(Some(s))).1;
        assert_eq!((at.passes_run, at.end), (2, PassesEnd::NothingNew));
        let above = passes(&slot, 0, always.with_lone_decode_min_snr(Some(s + 0.001))).1;
        assert_eq!(
            (above.passes_run, above.end),
            (1, PassesEnd::WeakLoneDecode)
        );
        if skipped {
            assert_eq!(per[..], all_per[..1]);
            assert_eq!(
                (report.passes_run, report.end, report.subtracted),
                (1, PassesEnd::WeakLoneDecode, 0)
            );
            assert!(!report.stopped_early());
            assert!(same_bits(&residual, &slot));
        } else {
            assert_eq!(per[..], all_per[..]);
            assert_eq!((report.passes_run, report.end), (2, PassesEnd::NothingNew));
        }
    }
    assert_eq!(PassesEnd::WeakLoneDecode.name(), "weak_lone_decode");
    // HOST's minimum, and a slot of many decodes is not affected however
    // high it is.
    assert_eq!(DecodeConfig::HOST.lone_decode_min_snr, Some(-17.0));
    let (slot, _) = crowded_slot(30, 9);
    let (all, all_report, all_res, _) = passes(&slot, 0, always);
    let (got, report, residual, _) = passes(&slot, 0, always.with_lone_decode_min_snr(Some(99.0)));
    assert_eq!((got, report), (all, all_report));
    assert!(same_bits(&residual, &all_res));
    assert_eq!(report.passes_run, 3);
    // The configured end is reported before a full list: a list of one
    // entry that pass 1 fills with a weak lone decode.
    let slot = lone(-19.0);
    let (got, report, _, _) = passes_into(&slot, 0, DecodeConfig::HOST, 1, |_| {
        ControlFlow::Continue(())
    });
    assert_eq!((got.len(), report.end), (1, PassesEnd::WeakLoneDecode));
    let (_, report, _, _) = passes_into(&slot, 0, always, 1, |_| ControlFlow::Continue(()));
    assert_eq!(report.end, PassesEnd::ListFull);
    // Two weak signals in a list of one: pass 1 keeps one and truncates
    // the other, so it did not decode a single message and the list is
    // full. In a longer list pass 2 runs after them.
    let mut slot = lone(-19.0);
    add_signal(
        &mut slot,
        &symbols_of("CQ W9XYZ EN37"),
        2_000.0,
        0.3,
        amplitude_for(-18.0),
    );
    let (got, report, _, per) = passes(&slot, 0, DecodeConfig::HOST);
    assert_eq!(per[0].report.decoded, 2);
    assert!(got[..2].iter().all(|d| d.snr_db < -17.0), "{got:?}");
    assert!(report.passes_run >= 2);
    let (got, report, _, per) = passes_into(&slot, 0, DecodeConfig::HOST, 1, |_| {
        ControlFlow::Continue(())
    });
    assert_eq!((got.len(), per[0].report.decodes_truncated), (1, 1));
    assert_eq!((report.passes_run, report.end), (1, PassesEnd::ListFull));
    assert!(report.stopped_early());
}

#[test]
fn a_weak_pass_2_decode_does_not_end_pass_3() {
    // A strong signal hiding a weak one 15 Hz away: pass 1 decodes the
    // strong one, pass 2 the weak one alone, below HOST's minimum; pass 3
    // still runs, as with no minimum.
    let mut slot = empty_slot();
    add_noise(&mut slot, NOISE_RMS, 3);
    add_signal(
        &mut slot,
        &symbols_of("CQ K1ABC FN42"),
        1_200.0,
        0.1,
        amplitude_for(6.0),
    );
    add_signal(
        &mut slot,
        &symbols_of("K1ABC W9XYZ EN37"),
        1_215.0,
        0.3,
        amplitude_for(-18.0),
    );
    let always = DecodeConfig::HOST.with_lone_decode_min_snr(None);
    let (got, report, residual, per) = passes(&slot, 0, DecodeConfig::HOST);
    let (all, all_report, all_res, _) = passes(&slot, 0, always);
    let counts: Vec<usize> = per.iter().map(|p| p.report.decoded).collect();
    assert_eq!(counts, [1, 1, 0]);
    assert!(got[0].snr_db >= -17.0 && got[1].snr_db < -17.0, "{got:?}");
    assert_eq!((report.passes_run, report.end), (3, PassesEnd::AllPasses));
    assert_eq!((got, report), (all, all_report));
    assert!(same_bits(&residual, &all_res));
}

#[test]
fn the_weak_test_reads_nan_as_below_any_minimum() {
    let mut b = Bufs::new(Band::DEFAULT);
    let at = |snr_db: f32| Decode {
        snr_db,
        ..Decode::EMPTY
    };
    {
        let dec = b.decoder(DecodeConfig::HOST);
        assert!(dec.weak(&at(f32::NAN)));
        assert!(dec.weak(&at(-17.5)));
        assert!(!dec.weak(&at(-17.0)));
        assert!(!dec.weak(&at(f32::INFINITY)));
        assert!(dec.weak(&at(f32::NEG_INFINITY)));
    }
    let dec = b.decoder(DecodeConfig::HOST.with_lone_decode_min_snr(None));
    assert!(!dec.weak(&at(f32::NAN)));
    assert!(!dec.weak(&at(-99.0)));
}

#[test]
fn the_spectrogram_is_the_last_pass_or_empty_after_a_stop() {
    // The subtractions use the spectrogram's buffer: on return it holds the
    // last pass's rows, or none if the decode stopped after a subtraction.
    let (slot, _) = crowded_slot(30, 9);
    for stop in [false, true] {
        let mut b = Bufs::new(Band::DEFAULT);
        let mut dec = b.decoder(DecodeConfig::HOST);
        let mut store = vec![Decode::EMPTY; HOST_DECODES];
        let mut list = DecodeList::new(&mut store);
        let mut audio = slot.clone();
        let report = dec.decode_passes_staged(
            &mut audio,
            0,
            &mut Box::new(CandidateScratch::new()),
            &mut list,
            |e| {
                let sub = matches!(
                    e,
                    PassEvent::Step {
                        step: PassStep::Subtract,
                        ..
                    }
                );
                if stop && sub {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        );
        if stop {
            // As after `start_slot`: no rows and no candidates (the report
            // still counts pass 1's).
            assert_eq!(report.end, PassesEnd::Stopped);
            assert_eq!(dec.spectrogram().rows_filled(), 0);
            assert!(dec.candidates().is_empty());
            assert!(dec
                .decode_candidate(0, &mut Box::new(CandidateScratch::new()))
                .is_none());
            assert!(report.report.candidates_kept > 0);
        } else {
            assert!(report.passes_run > 1);
            assert!(!dec.candidates().is_empty());
            // The last pass's rows: those of the residual it decoded (what
            // the same passes leave in the audio).
            let rows = dec.spectrogram().rows_filled();
            assert_eq!(rows, crate::decode::ROWS);
            let mut b2 = Bufs::new(Band::DEFAULT);
            let mut fresh = b2.decoder(DecodeConfig::HOST);
            let (_, _, before_last, _) =
                passes(&slot, 0, DecodeConfig::HOST.with_passes(report.passes_run));
            fresh.feed(&before_last);
            for r in 0..rows {
                assert_eq!(dec.spectrogram().row(r), fresh.spectrogram().row(r), "{r}");
            }
        }
    }
}

#[test]
fn a_narrow_band_level_buffer_gives_the_padded_result() {
    // 1000–1500 Hz: the level buffer is smaller than one run's reference,
    // so the subtractions run it twice where it does not fit. Padded to
    // that size, they run it once. The same decodes, report and residual,
    // whole and with a skip and a cut.
    let band = Band::from_hz(1_000, 1_500).unwrap();
    assert!(band.spectrogram_len() < crate::decode::SUBTRACT_SCRATCH_BYTES);
    let mut r = crate::decode::testutil::Noise(21);
    let mut slot = empty_slot();
    for k in 0..12 {
        let snr = -14.0 + 18.0 * r.uniform();
        let freq = (1_010.0 + 440.0 * r.uniform()) as f32;
        let dt = (-0.3 + r.uniform()) as f32;
        let text = std::format!("NARROW {k:02}");
        add_signal(&mut slot, &symbols_of(&text), freq, dt, amplitude_for(snr));
    }
    add_noise(&mut slot, NOISE_RMS, 22);
    for (skip, cut) in [(0, SLOT_SAMPLES), (12_000, 150_000)] {
        let mut got = Vec::new();
        for len in [
            band.spectrogram_len(),
            crate::decode::SUBTRACT_SCRATCH_BYTES,
        ] {
            let mut b = Bufs::new(band);
            b.levels = vec![0; len];
            let mut dec = b.decoder(DecodeConfig::HOST.with_passes(2));
            let mut store = vec![Decode::EMPTY; HOST_DECODES];
            let mut list = DecodeList::new(&mut store);
            let mut audio = slot[..cut].to_vec();
            let report = dec.decode_passes(
                &mut audio,
                skip,
                &mut Box::new(CandidateScratch::new()),
                &mut list,
            );
            assert!(report.subtracted > 0 && report.passes_run == 2, "{skip}");
            got.push((list.finish_payloads().to_vec(), report, audio));
        }
        assert_eq!((&got[0].0, &got[0].1), (&got[1].0, &got[1].1), "{skip}");
        assert!(same_bits(&got[0].2, &got[1].2), "{skip}");
    }
}

#[test]
fn the_event_callback_can_stop_the_passes() {
    let (slot, _) = crowded_slot(30, 9);
    let host = DecodeConfig::HOST;
    let (all, _, _, all_per) = passes(&slot, 0, host);
    let first = all_per[1].first;
    // Asked during pass 1: pass 1 completes, nothing is subtracted.
    let mut events = 0;
    let (got, report, residual, per) = passes_into(&slot, 0, host, HOST_DECODES, |_| {
        events += 1;
        if events == 5 {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    assert_eq!(got[..], all[..first]);
    assert_eq!(per[..], all_per[..1]);
    assert_eq!((report.passes_run, report.end), (1, PassesEnd::Stopped));
    assert!(report.stopped_early());
    assert!(same_bits(&residual, &slot));
    // Asked after the first subtraction: the residual holds that one only.
    let (got, report, residual, per) = passes_into(&slot, 0, host, HOST_DECODES, |e| {
        if matches!(
            e,
            PassEvent::Step {
                step: PassStep::Subtract,
                ..
            }
        ) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    assert_eq!(got[..], all[..first]);
    assert_eq!(per.len(), 1);
    assert_eq!(
        (report.passes_run, report.end, report.subtracted),
        (1, PassesEnd::Stopped, 1)
    );
    let mut want = slot.clone();
    subtract(
        &mut want,
        &all[0],
        0,
        &host.subtract,
        &mut vec![0u8; crate::decode::SUBTRACT_SCRATCH_BYTES],
    )
    .unwrap();
    assert!(same_bits(&residual, &want));
    // With one pass there is nothing to stop.
    let (_, report, _, _) = passes_into(&slot, 0, host.with_passes(1), HOST_DECODES, |_| {
        ControlFlow::Break(())
    });
    assert_eq!(report.end, PassesEnd::AllPasses);
}

#[test]
fn further_passes_after_a_pass_fed_as_i16_blocks_match_decode_passes() {
    // The slot as 16-bit samples, and as the `f32` values `feed_i16`
    // takes them as.
    let (slot, _) = crowded_slot(30, 5);
    let pcm: Vec<i16> = slot
        .iter()
        .map(|&x| (x * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16)
        .collect();
    let audio: Vec<f32> = pcm.iter().map(|&x| f32::from(x) / 32_768.0).collect();
    for skip in [0, 12_000] {
        let mut want_audio = audio.clone();
        want_audio[..skip].fill(0.0);
        let (want, want_report, want_res, _) = passes(&want_audio, skip, DecodeConfig::HOST);
        let mut b = Bufs::new(Band::DEFAULT);
        let mut dec = b.decoder(DecodeConfig::HOST);
        let mut store = [Decode::EMPTY; HOST_DECODES];
        let mut list = DecodeList::new(&mut store);
        let mut scratch = Box::new(CandidateScratch::new());
        // Pass 1 as the audio arrives: the missing start, then blocks.
        dec.skip(skip);
        for block in pcm[skip..].chunks(1_000) {
            dec.feed_i16(block);
        }
        dec.decode(&mut scratch, &mut list);
        // The residual buffer: slot sample 0 on, the skipped part unread.
        let mut residual = audio.clone();
        let report = dec.decode_further_passes(&mut residual, skip, &mut scratch, &mut list);
        assert_eq!(list.finish_payloads(), &want[..], "skip {skip}");
        assert_eq!(report, want_report, "skip {skip}");
        assert!(
            same_bits(&residual[skip..], &want_res[skip..]),
            "skip {skip}"
        );
        assert!(same_bits(&residual[..skip], &audio[..skip]), "skip {skip}");
        assert!(report.passes_run > 1);
    }
}

#[test]
fn audio_past_the_slot_or_a_skip_past_the_audio() {
    let (slot, _) = crowded_slot(20, 7);
    let (want, want_report, want_res, _) = passes(&slot, 0, DecodeConfig::HOST);
    // Longer than a slot: decoded to the slot's end. These signals end
    // before it, so subtraction leaves the extra samples alone.
    let mut long = slot.clone();
    long.extend((0..6_000).map(|i| if i % 2 == 0 { 0.01 } else { -0.01 }));
    let (got, report, residual, _) = passes(&long, 0, DecodeConfig::HOST);
    assert_eq!((got, report), (want, want_report));
    assert!(same_bits(&residual[..SLOT_SAMPLES], &want_res));
    assert!(same_bits(&residual[SLOT_SAMPLES..], &long[SLOT_SAMPLES..]));
    // A skip at or past the end of the audio feeds nothing.
    for skip in [slot.len(), slot.len() + 5, 3 * SLOT_SAMPLES] {
        let (got, report, residual, per) = passes(&slot, skip, DecodeConfig::HOST);
        assert!(got.is_empty(), "{skip}");
        assert_eq!(per.len(), 1);
        assert_eq!(report.end, PassesEnd::NothingNew);
        assert_eq!(report.report.skipped_samples, SLOT_SAMPLES.min(skip));
        assert!(same_bits(&residual, &slot));
    }
}

#[test]
fn staged_passes_report_their_steps_in_order() {
    let (slot, _) = crowded_slot(12, 3);
    let mut b = Bufs::new(Band::DEFAULT);
    let mut dec = b.decoder(DecodeConfig::HOST.with_passes(2));
    let mut store = [Decode::EMPTY; HOST_DECODES];
    let mut list = DecodeList::new(&mut store);
    let mut steps: Vec<(u8, PassStep)> = Vec::new();
    let mut audio = slot.clone();
    let report = dec.decode_passes_staged(
        &mut audio,
        0,
        &mut Box::new(CandidateScratch::new()),
        &mut list,
        |e| {
            if let PassEvent::Step { pass, step } = e {
                if !matches!(step, PassStep::Candidate(_)) {
                    steps.push((pass, step));
                }
            }
            ControlFlow::Continue(())
        },
    );
    // One subtraction step per decode of pass 1.
    let mut want = vec![(1, PassStep::Feed), (1, PassStep::Sync)];
    want.extend(core::iter::repeat((2, PassStep::Subtract)).take(report.subtracted));
    want.extend([(2, PassStep::Feed), (2, PassStep::Sync)]);
    assert!(report.subtracted > 0);
    assert_eq!(steps, want);
}

#[test]
fn next_pass_restarts_the_index_and_keeps_the_entries() {
    let d = |p: u128, c: u16| Decode {
        payload: Payload::from_bits(p).unwrap(),
        candidate: c,
        ..Decode::EMPTY
    };
    let mut store = [Decode::EMPTY; 4];
    let mut list = DecodeList::new(&mut store);
    list.push(0, &Ok(d(1, 0))).unwrap();
    list.push(1, &Ok(d(2, 1))).unwrap();
    assert_eq!(list.push(0, &Ok(d(3, 0))), Err(Error::OutOfOrder));
    list.next_pass();
    assert_eq!(list.pushed(), 0);
    list.push(0, &Ok(d(2, 0))).unwrap(); // an earlier pass's payload
    list.push(1, &Ok(d(3, 1))).unwrap();
    list.push(2, &Ok(d(3, 2))).unwrap(); // this pass's own
    assert_eq!(list.len(), 3);
    assert_eq!((list.tally.duplicates, list.tally.earlier), (2, 1));
    assert_eq!(list.finish_payloads()[1].candidate, 1);
}

/// With belief propagation held to round 0 (the hard decisions only), a
/// signal whose hard decisions hold a few errors fails it; OSD-1 on the
/// best candidates recovers it, reports its stage, keeps belief
/// propagation's round count, and with zero candidates (or `None`) the
/// result is the decoder's without it.
#[test]
fn osd_recovers_what_belief_propagation_missed() {
    let mut s = empty_slot();
    add_signal(
        &mut s,
        &symbols_of("CQ K1ABC FN42"),
        1_234.5,
        0.1,
        amplitude_for(-15.0),
    );
    add_noise(&mut s, NOISE_RMS, 31);
    let base = DecodeConfig::HOST.with_max_rounds(0).with_osd(None);
    let run = |config: DecodeConfig| {
        let mut b = Bufs::new(Band::DEFAULT);
        let mut dec = b.decoder(config);
        dec.feed(&s);
        let mut store = [Decode::EMPTY; 8];
        let mut list = DecodeList::new(&mut store);
        let mut scratch = Box::new(CandidateScratch::new());
        let report = dec.decode(&mut scratch, &mut list);
        let mut osd_stages = 0;
        let mut errors = Vec::new();
        for i in 0..report.candidates_kept {
            let r = dec.decode_candidate_staged(i, &mut scratch, |st| {
                osd_stages += usize::from(st == Stage::Osd);
            });
            assert_eq!(scratch.last_rounds(), 0);
            if let Some(Ok(d)) = r {
                errors.push(d.hard_errors);
            }
        }
        (texts(&list), report, osd_stages, errors)
    };
    let (none, r0, s0, _) = run(base);
    assert!(none.is_empty(), "{none:?}");
    assert_eq!(s0, 0);
    let off = run(base.with_osd(Some(OsdConfig::new(0, 40))));
    assert_eq!((off.0, off.1, off.2), (none, r0, 0));
    let (on, r1, s1, errors) = run(base.with_osd(Some(OsdConfig::new(10, 40))));
    assert_eq!(on, ["CQ K1ABC FN42"]);
    // Every one of the best ten failed belief propagation (no rounds).
    assert_eq!(s1, r1.candidates_kept.min(10));
    assert!(errors.iter().all(|&e| (1..=40).contains(&e)), "{errors:?}");
    // A distance bound below the decode's rejects it.
    let least = *errors.iter().min().unwrap();
    let (tight, _, _, _) = run(base.with_osd(Some(OsdConfig::new(10, least - 1))));
    assert!(tight.is_empty());
}

/// With the option on, a candidate decoded after one OSD-1 ran on (its
/// storage then OSD-1's) gives what it gives with fresh storage: belief
/// propagation's storage is re-initialised whatever ran before.
#[test]
fn a_candidate_after_osd_decodes_as_with_fresh_storage() {
    let s = busy_slot(7);
    let config = DecodeConfig::HOST.with_osd(Some(OsdConfig::new(CANDIDATES as u16, 21)));
    let mut b = Bufs::new(Band::DEFAULT);
    let mut dec = b.decoder(config);
    dec.feed(&s);
    let kept = dec.find_candidates().kept;
    // The first candidate OSD-1 runs on.
    let mut used = Box::new(CandidateScratch::new());
    let osd_ran = (0..kept)
        .find(|&i| {
            let mut ran = false;
            dec.decode_candidate_staged(i, &mut used, |st| ran |= st == Stage::Osd);
            ran
        })
        .expect("some candidate fails belief propagation");
    let mut decoded = 0;
    for j in 0..kept {
        let mut fresh = Box::new(CandidateScratch::new());
        let mut osd = false;
        let want = dec.decode_candidate_staged(j, &mut fresh, |st| osd |= st == Stage::Osd);
        dec.decode_candidate(osd_ran, &mut used);
        let got = dec.decode_candidate(j, &mut used);
        assert_eq!(got, want, "candidate {j}");
        assert_eq!(used.last_rounds(), fresh.last_rounds(), "candidate {j}");
        decoded += usize::from(!osd && matches!(want, Some(Ok(_))));
    }
    // Belief propagation's own decodes are among them.
    assert!(decoded >= 1);
}

/// An OSD-1 decode after belief propagation ran its rounds and failed
/// carries those rounds (a −18 dB signal one round of belief propagation
/// does not decode).
#[test]
fn an_osd_decode_carries_belief_propagations_rounds() {
    let mut s = empty_slot();
    add_signal(
        &mut s,
        &symbols_of("CQ K1ABC FN42"),
        1_234.5,
        0.1,
        amplitude_for(-18.0),
    );
    add_noise(&mut s, NOISE_RMS, 31);
    let max_rounds = 1;
    let config = DecodeConfig::HOST
        .with_max_rounds(max_rounds)
        .with_osd(Some(OsdConfig::new(10, 40)));
    let mut b = Bufs::new(Band::DEFAULT);
    let mut dec = b.decoder(config);
    dec.feed(&s);
    let kept = dec.find_candidates().kept;
    let mut scratch = Box::new(CandidateScratch::new());
    let mut osd_decodes = 0;
    for i in 0..kept {
        let mut ran = false;
        let r = dec.decode_candidate_staged(i, &mut scratch, |st| ran |= st == Stage::Osd);
        if let (true, Some(Ok(d))) = (ran, r) {
            assert_eq!(d.rounds, max_rounds, "candidate {i}");
            assert_eq!(scratch.last_rounds(), max_rounds);
            osd_decodes += 1;
        }
    }
    assert!(osd_decodes >= 1);
}
