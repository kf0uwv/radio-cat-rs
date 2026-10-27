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

//! Helpers shared by the decoder's integration tests: seeded slots made
//! with this crate's encoder and `libm` noise (pure Rust, so the inputs
//! are the same bits on every platform), caller-owned decoder buffers,
//! and a parallel map over slots.

#![allow(dead_code)] // each test file uses part of this module

use cat_ft8::decode::{
    Band, Candidate, CandidateScratch, Decode, DecodeConfig, DecodeList, DecodeReport, Decoder,
    FrontEnd, CANDIDATES, HOST_DECODES,
};
use cat_ft8::message::{CallHashes, NoHashes, Payload};
use cat_ft8::waveform::{synthesize, Placement};
use cat_ft8::{encode, SLOT_SAMPLES};

/// Noise RMS of every slot (−30.5 dBFS).
pub const NOISE_RMS: f64 = 0.03;

/// splitmix64 with Box–Muller normals (`libm`).
pub struct Rng(pub u64);

impl Rng {
    pub fn uniform(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64 + 1e-300
    }

    pub fn gauss(&mut self) -> f64 {
        let (u, v) = (self.uniform(), self.uniform());
        libm::sqrt(-2.0 * libm::log(u)) * libm::cos(2.0 * core::f64::consts::PI * v)
    }
}

/// Peak amplitude for `snr_db` in 2500 Hz against noise of density
/// `noise_rms²` over 6 kHz.
pub fn amplitude(snr_db: f64, noise_rms: f64) -> f32 {
    let n2500 = noise_rms * noise_rms * 2500.0 / 6000.0;
    libm::sqrt(2.0 * n2500 * libm::pow(10.0, snr_db / 10.0)) as f32
}

/// One signal in a slot.
#[derive(Debug, Clone, Copy)]
pub struct Signal<'a> {
    pub message: &'a str,
    pub base_hz: f64,
    pub dt: f64,
    pub snr_db: f64,
}

/// Adds `signal` to `slot` (SNR against [`NOISE_RMS`] white noise);
/// returns its payload.
pub fn add(slot: &mut [f32], signal: &Signal<'_>) -> Payload {
    add_with_amplitude(slot, signal, amplitude(signal.snr_db, NOISE_RMS))
}

/// Adds `signal` at peak amplitude `amp`; returns its payload.
pub fn add_with_amplitude(slot: &mut [f32], signal: &Signal<'_>, amp: f32) -> Payload {
    let enc = encode(signal.message, &mut NoHashes).unwrap();
    let mut p = Placement::slot(signal.base_hz as f32, signal.dt as f32).unwrap();
    p.amplitude = amp;
    synthesize(&enc.symbols, &p, slot).unwrap();
    enc.payload
}

/// Adds seeded white noise of RMS `rms`.
pub fn add_noise(slot: &mut [f32], rms: f64, seed: u64) {
    let mut n = Rng(seed);
    for x in slot.iter_mut() {
        *x += (rms * n.gauss()) as f32;
    }
}

/// A slot with `signals` in white noise; also returns their payloads.
pub fn slot(signals: &[Signal<'_>], seed: u64) -> (Vec<f32>, Vec<Payload>) {
    let mut s = vec![0.0f32; SLOT_SAMPLES];
    let payloads = signals.iter().map(|g| add(&mut s, g)).collect();
    add_noise(&mut s, NOISE_RMS, seed);
    (s, payloads)
}

/// Messages to draw from (standard, nonstandard and free text).
pub const MESSAGES: [&str; 12] = [
    "CQ K1ABC FN42",
    "K1ABC W9XYZ -11",
    "W9XYZ K1ABC R-09",
    "K1ABC W9XYZ RR73",
    "CQ DX G4ABC IO91",
    "G4ABC JA1XYZ PM95",
    "JA1XYZ G4ABC +02",
    "CQ VK2ABC QF56",
    "TNX BOB 73 GL",
    "VE3XYZ K1ABC 73",
    "CQ TEST N0ABC EN34",
    "PA0ABC/P DL1XYZ JO62",
];

/// A slot with one signal drawn by `seed`: a message from [`MESSAGES`],
/// tone 0 at 200–2 800 Hz and a start −0.3…+0.7 s, off the analysis grid.
pub fn random_slot(snr_db: f64, seed: u64) -> (Vec<f32>, Payload) {
    let mut r = Rng(seed.wrapping_mul(0x2545_f491_4f6c_dd1d) ^ 0xa5a5);
    let message = MESSAGES[(r.uniform() * MESSAGES.len() as f64) as usize % MESSAGES.len()];
    let signal = Signal {
        message,
        base_hz: 200.0 + 2_600.0 * r.uniform(),
        dt: -0.3 + r.uniform(),
        snr_db,
    };
    let (s, p) = slot(&[signal], seed);
    (s, p[0])
}

/// Caller-owned decoder buffers on the heap.
pub struct Work {
    front: Box<FrontEnd>,
    levels: Vec<u8>,
    sync: Vec<i32>,
    cands: Vec<Candidate>,
    band: Band,
}

impl Work {
    pub fn new(band: Band) -> Self {
        Work {
            front: Box::new(FrontEnd::new()),
            levels: vec![0; band.spectrogram_len()],
            sync: vec![0; Decoder::sync_scratch_len(band)],
            cands: vec![Candidate::default(); CANDIDATES],
            band,
        }
    }

    pub fn decoder(&mut self, config: DecodeConfig) -> Decoder<'_> {
        Decoder::new(
            &mut self.front,
            &mut self.levels,
            &mut self.sync,
            &mut self.cands,
            self.band,
            config,
        )
        .unwrap()
    }
}

/// Everything a decode produced: entries, report and texts (or the unpack
/// error's text).
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub decodes: Vec<Decode>,
    pub report: DecodeReport,
    pub texts: Vec<String>,
}

/// Runs `decode()` and `finish` with `hashes`, into a list of
/// [`HOST_DECODES`] entries.
pub fn run(dec: &mut Decoder<'_>, hashes: &mut impl CallHashes) -> Output {
    run_into(dec, hashes, HOST_DECODES)
}

/// [`run`] into a list of `capacity` entries.
pub fn run_into(dec: &mut Decoder<'_>, hashes: &mut impl CallHashes, capacity: usize) -> Output {
    let mut store = vec![Decode::EMPTY; capacity];
    let mut list = DecodeList::new(&mut store);
    let report = dec.decode(&mut Box::new(CandidateScratch::new()), &mut list);
    let mut texts = Vec::new();
    list.finish(hashes, |_, t| {
        texts.push(match t {
            Ok(s) => s.to_string(),
            Err(e) => format!("error: {e}"),
        })
    });
    Output {
        decodes: list.finish_payloads().to_vec(),
        report,
        texts,
    }
}

/// Feeds `samples` in one block to a fresh decoder (default band) and
/// decodes.
pub fn decode(samples: &[f32], config: DecodeConfig) -> Output {
    decode_setup(samples, Band::DEFAULT, config, HOST_DECODES)
}

/// [`decode`] over `band` into a list of `capacity` entries.
pub fn decode_setup(samples: &[f32], band: Band, config: DecodeConfig, capacity: usize) -> Output {
    let mut w = Work::new(band);
    let mut dec = w.decoder(config);
    dec.feed(samples);
    run_into(&mut dec, &mut NoHashes, capacity)
}

/// `f(i)` for `i` in `0..n`, on all available threads, in index order.
pub fn par_map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism()
        .map_or(4, |x| x.get())
        .min(n.max(1));
    let f = &f;
    let mut out: Vec<(usize, T)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                s.spawn(move || {
                    (t..n)
                        .step_by(threads)
                        .map(|i| (i, f(i)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    });
    out.sort_by_key(|p| p.0);
    out.into_iter().map(|p| p.1).collect()
}

/// FNV-1a 64 over bytes.
pub fn fnv1a(hash: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *hash ^= u64::from(b);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

/// Hashes every field of `out` (bit patterns for floats, the texts and
/// every report count).
pub fn hash_output(hash: &mut u64, out: &Output) {
    for d in &out.decodes {
        fnv1a(hash, &d.payload.bits().to_le_bytes());
        fnv1a(hash, &d.freq_hz.to_bits().to_le_bytes());
        fnv1a(hash, &d.dt.to_bits().to_le_bytes());
        fnv1a(hash, &d.snr_db.to_bits().to_le_bytes());
        fnv1a(hash, &d.score.to_le_bytes());
        fnv1a(hash, &d.candidate.to_le_bytes());
        fnv1a(hash, &[d.hard_errors, d.rounds]);
    }
    for t in &out.texts {
        fnv1a(hash, t.as_bytes());
        fnv1a(hash, &[0]);
    }
    let r = &out.report;
    for v in [
        r.candidates_found,
        r.candidates_kept,
        r.candidates_dropped,
        r.decoded,
        r.decodes_truncated,
        r.duplicates,
        r.all_zero_rejected,
        r.crc_failed,
        r.not_converged,
        r.rows_filled,
        r.sanitised_samples,
        r.clamped_samples,
        r.skipped_samples,
        r.floored_levels,
        r.top_levels,
    ] {
        fnv1a(hash, &(v as u64).to_le_bytes());
    }
}
