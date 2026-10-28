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
use crate::decode::testutil::{
    add_noise, add_signal, amplitude_for, empty_slot, symbols_of, NOISE_RMS,
};
use crate::message::{CallTable, NoHashes};
use crate::SLOT_SAMPLES;

/// Caller-owned decoder buffers on the heap.
struct Bufs {
    front: Box<FrontEnd>,
    levels: Vec<u8>,
    scratch: Vec<i32>,
    cands: Vec<Candidate>,
    band: Band,
}

impl Bufs {
    fn new(band: Band) -> Self {
        Bufs {
            front: Box::new(FrontEnd::new()),
            levels: vec![0; band.spectrogram_len()],
            scratch: vec![0; Decoder::sync_scratch_len(band)],
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
    // The later-pass count needs room only when there are later passes.
    let later = DecodeConfig::HOST
        .with_passes(1)
        .with_later_candidates(CANDIDATES + 1);
    assert_eq!(later.candidate_capacity(), 100);
    assert!(Decoder::new(
        &mut b.front,
        &mut b.levels,
        &mut b.scratch,
        &mut b.cands,
        band,
        later
    )
    .is_ok());
    let later = later.with_passes(2);
    assert_eq!(later.candidate_capacity(), CANDIDATES + 1);
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
    let short = &mut b.scratch[..Decoder::sync_scratch_len(band) - 1];
    let r = Decoder::new(
        &mut b.front,
        &mut b.levels,
        short,
        &mut b.cands,
        band,
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
    assert_eq!(
        (
            chunked.duplicates,
            chunked.truncated,
            chunked.all_zero,
            chunked.crc_failed
        ),
        (
            list.duplicates,
            list.truncated,
            list.all_zero,
            list.crc_failed
        )
    );
    let kept: Vec<u16> = list.finish_payloads().iter().map(|d| d.candidate).collect();
    assert_eq!(kept, [0, 3]);
    assert_eq!(
        (
            list.duplicates,
            list.truncated,
            list.all_zero,
            list.crc_failed,
            list.not_converged
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
        ["refine", "soft_bits", "belief_propagation", "snr"]
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
) -> (Vec<Decode>, DecodeReport, Vec<f32>, Vec<PassReport>) {
    let mut b = Bufs::new(Band::DEFAULT);
    let mut dec = b.decoder(config);
    let mut store = [Decode::EMPTY; HOST_DECODES];
    let mut list = DecodeList::new(&mut store);
    let mut residual = audio.to_vec();
    let mut per = Vec::new();
    let report = dec.decode_passes_staged(
        &mut residual,
        skip,
        &mut CandidateScratch::new(),
        &mut list,
        |e| {
            if let PassEvent::Done(r) = e {
                per.push(*r);
            }
        },
    );
    (list.finish_payloads().to_vec(), report, residual, per)
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
            assert_eq!(report, want, "{config:?} {skip}");
            assert!(residual
                .iter()
                .zip(&slot)
                .all(|(a, b)| a.to_bits() == b.to_bits()));
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
    // Pass 1's counts, the whole list's decoded.
    assert_eq!(
        (r2.candidates_kept, r2.crc_failed),
        (r1.candidates_kept, r1.crc_failed)
    );
    assert_eq!((r2.decoded, r3.decoded), (two.len(), three.len()));
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
    assert!(again
        .2
        .iter()
        .zip(&res3)
        .all(|(a, b)| a.to_bits() == b.to_bits()));
}

#[test]
fn a_pass_that_adds_nothing_ends_the_loop() {
    let mut noise = empty_slot();
    add_noise(&mut noise, NOISE_RMS, 3);
    let (got, _, residual, per) = passes(&noise, 0, DecodeConfig::HOST.with_passes(3));
    assert!(got.is_empty());
    assert_eq!(per.len(), 1);
    assert!(residual
        .iter()
        .zip(&noise)
        .all(|(a, b)| a.to_bits() == b.to_bits()));
    // One signal: pass 2 finds nothing new, so there is no pass 3.
    let mut one = noise.clone();
    add_signal(
        &mut one,
        &symbols_of("CQ K1ABC FN42"),
        1_200.0,
        0.1,
        amplitude_for(-5.0),
    );
    let (got, _, _, per) = passes(&one, 0, DecodeConfig::HOST.with_passes(3));
    assert_eq!(got.len(), 1);
    assert_eq!(per.len(), 2);
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
fn staged_passes_report_their_steps_in_order() {
    let (slot, _) = crowded_slot(12, 3);
    let mut b = Bufs::new(Band::DEFAULT);
    let mut dec = b.decoder(DecodeConfig::HOST.with_passes(2));
    let mut store = [Decode::EMPTY; HOST_DECODES];
    let mut list = DecodeList::new(&mut store);
    let mut steps: Vec<(u8, PassStep)> = Vec::new();
    let mut audio = slot.clone();
    dec.decode_passes_staged(
        &mut audio,
        0,
        &mut CandidateScratch::new(),
        &mut list,
        |e| {
            if let PassEvent::Step { pass, step } = e {
                if !matches!(step, PassStep::Candidate(_)) {
                    steps.push((pass, step));
                }
            }
        },
    );
    assert_eq!(
        steps,
        [
            (1, PassStep::Feed),
            (1, PassStep::Sync),
            (2, PassStep::Subtract),
            (2, PassStep::Feed),
            (2, PassStep::Sync)
        ]
    );
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
    assert_eq!((list.duplicates, list.earlier), (2, 1));
    assert_eq!(list.finish_payloads()[1].candidate, 1);
}
