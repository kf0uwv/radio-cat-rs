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

//! The measurement sets: seeds, messages and slots.

use cat_ft8::message::{pack, NoHashes, Payload};
use cat_ft8::waveform::{synthesize, Placement};
use cat_ft8::{encode_payload, SLOT_SAMPLES};

use crate::channel::{self, Channel};
use crate::common::{add_noise, amplitude, fnv1a, Rng, NOISE_RMS};

// ---------------------------------------------------------------- seeds

/// splitmix64 finaliser.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The seed of slot `index` of `set` in `block`.
pub fn slot_seed(set: &str, block: &str, index: usize) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in set.bytes().chain([0]).chain(block.bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    mix(h ^ (index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

/// The noise seed of the slot whose seed is `seed`.
pub fn noise_seed(seed: u64) -> u64 {
    mix(seed ^ 0x006e_6f69_7365)
}

/// A uniform index below `n`.
pub fn below(r: &mut Rng, n: usize) -> usize {
    (r.uniform() * n as f64) as usize % n
}

// ------------------------------------------------------------- messages

/// A random standard callsign (prefix, digit, suffix).
fn callsign(r: &mut Rng) -> String {
    const L: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut s = String::new();
    s.push(L[below(r, 26)] as char);
    if below(r, 2) == 0 {
        s.push(L[below(r, 26)] as char);
    }
    s.push((b'0' + below(r, 10) as u8) as char);
    for _ in 0..1 + below(r, 3) {
        s.push(L[below(r, 26)] as char);
    }
    s
}

/// A random grid square.
fn grid(r: &mut Rng) -> String {
    const F: &[u8] = b"ABCDEFGHIJKLMNOPQR";
    let (a, b) = (F[below(r, 18)] as char, F[below(r, 18)] as char);
    format!("{a}{b}{}{}", below(r, 10), below(r, 10))
}

/// A random packable message whose payload is not in `avoid`.
pub fn message(r: &mut Rng, avoid: &[Payload]) -> (String, Payload) {
    loop {
        let (a, b) = (callsign(r), callsign(r));
        let text = match below(r, 6) {
            0 => format!("CQ {a} {}", grid(r)),
            1 => format!("{a} {b} {}", grid(r)),
            2 => format!("{a} {b} {:+03}", below(r, 51) as i32 - 30),
            3 => format!("{a} {b} R{:+03}", below(r, 51) as i32 - 30),
            4 => format!("{a} {b} RR73"),
            _ => format!("{a} {b} 73"),
        };
        if let Ok(p) = pack(&text, &mut NoHashes) {
            if !avoid.contains(&p) {
                return (text, p);
            }
        }
    }
}

// ---------------------------------------------------------------- slots

/// One transmitted signal.
#[derive(Clone, PartialEq)]
pub struct Sent {
    pub text: String,
    pub payload: u128,
    pub freq: f64,
    pub dt: f64,
    pub snr: f64,
    /// A fade, drift and a start between samples (the `fading` set only):
    /// the waveform starts at exactly (0.5 + dt)·12 000 samples and is made
    /// by the bench's own generator. `None`: `Synth` at the nearest
    /// sample, constant amplitude and frequency.
    pub channel: Option<Channel>,
}

/// A slot: its point label, the samples to feed after `skip` missing ones,
/// and what was sent.
pub struct Slot {
    pub point: String,
    pub samples: Vec<f32>,
    pub skip: usize,
    pub sent: Vec<Sent>,
}

impl Slot {
    /// FNV-1a over the skip count and every sample's bits.
    pub fn sample_hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        fnv1a(&mut h, &(self.skip as u64).to_le_bytes());
        for x in &self.samples {
            fnv1a(&mut h, &x.to_bits().to_le_bytes());
        }
        h
    }
}

pub fn add_signal(out: &mut [f32], s: &Sent) {
    let payload = Payload::from_bits(s.payload).unwrap();
    if let Some(ch) = &s.channel {
        let start = (0.5 + s.dt) * 12_000.0;
        let amp = f64::from(amplitude(s.snr, NOISE_RMS));
        channel::render(out, &encode_payload(&payload), s.freq, start, amp, ch);
        return;
    }
    let mut p = Placement::slot(s.freq as f32, s.dt as f32).unwrap();
    p.amplitude = amplitude(s.snr, NOISE_RMS);
    synthesize(&encode_payload(&payload), &p, out).unwrap();
}

fn add_carriers(out: &mut [f32], r: &mut Rng, lo_db: f64, hi_db: f64) {
    for _ in 0..1 + below(r, 4) {
        let f = 100.0 + 3_000.0 * r.uniform();
        let a = f64::from(amplitude(lo_db + (hi_db - lo_db) * r.uniform(), NOISE_RMS));
        let ph = 2.0 * core::f64::consts::PI * r.uniform();
        for (n, x) in out.iter_mut().enumerate() {
            *x +=
                (a * libm::cos(2.0 * core::f64::consts::PI * f * n as f64 / 12_000.0 + ph)) as f32;
        }
    }
}

/// Slots in each set.
pub fn set_size(set: &str) -> Option<usize> {
    match set {
        "sweep" => Some(15 * 1_000),
        "crowded" => Some(3 * 200),
        "wide" => Some(4 * 200),
        "fading" => Some(3 * 200),
        "edges" => Some(EDGE_CASES.len() * 100),
        "empty" => Some(2_300),
        "masked" => Some(MASKED.iter().map(|m| m.1).sum()),
        _ => None,
    }
}

/// Edge cases: (label, dt, tone 0 in Hz or random, samples kept, samples
/// missing at the start).
const EDGE_CASES: [(&str, f64, Option<f64>, usize, usize); 13] = [
    ("dt-2.4", -2.4, None, SLOT_SAMPLES, 0),
    ("dt-2.0", -2.0, None, SLOT_SAMPLES, 0),
    ("dt-1.6", -1.6, None, SLOT_SAMPLES, 0),
    ("dt+1.6", 1.6, None, SLOT_SAMPLES, 0),
    ("dt+2.0", 2.0, None, SLOT_SAMPLES, 0),
    ("dt+2.4", 2.4, None, SLOT_SAMPLES, 0),
    ("f100", 0.2, Some(100.0), SLOT_SAMPLES, 0),
    ("f3090", 0.2, Some(3_090.0), SLOT_SAMPLES, 0),
    ("f200", 0.2, Some(200.0), SLOT_SAMPLES, 0),
    ("f2890", 0.2, Some(2_890.0), SLOT_SAMPLES, 0),
    ("cut13.5", 0.0, None, 162_000, 0),
    ("late1s", 0.0, None, SLOT_SAMPLES, 12_000),
    ("dt+0", 0.0, None, SLOT_SAMPLES, 0),
];

/// A full slot of `sent` (in order) plus white noise of RMS [`NOISE_RMS`]
/// from `noise_seed`: the input of every set with signals, and of the
/// `ceiling` mode's slots with some signals left out (same noise).
pub fn render(sent: &[Sent], noise_seed: u64) -> Vec<f32> {
    let mut samples = vec![0.0f32; SLOT_SAMPLES];
    for s in sent {
        add_signal(&mut samples, s);
    }
    add_noise(&mut samples, NOISE_RMS, noise_seed);
    samples
}

/// `n` distinct messages at random offsets (200–2 800 Hz, −0.3…+0.7 s)
/// and SNRs uniform in `lo_db`…`lo_db + span_db`.
fn crowd(r: &mut Rng, n: usize, lo_db: f64, span_db: f64) -> Vec<Sent> {
    let mut sent: Vec<Sent> = Vec::new();
    for _ in 0..n {
        let avoid: Vec<Payload> = sent
            .iter()
            .map(|s| Payload::from_bits(s.payload).unwrap())
            .collect();
        let (text, payload) = message(r, &avoid);
        // Field order is draw order.
        sent.push(Sent {
            text,
            payload: payload.bits(),
            freq: 200.0 + 2_600.0 * r.uniform(),
            dt: -0.3 + r.uniform(),
            snr: lo_db + span_db * r.uniform(),
            channel: None,
        });
    }
    sent
}

/// The `masked` set's points: one strong signal (−4…+20 dB) with one
/// (`near`) or two (`near2`) weak ones (−20…−8 dB) 5–60 Hz beside it; a
/// strong one (+5…+20 dB) with a weak one near the decode threshold
/// (−21…−15 dB) anywhere in the band (`far`); two strong ones, each with
/// a weak one beside it (`pair`); a weaker masking signal (−20…0 dB)
/// with one beside it at −22 dB up to 2 dB below it (`low`, 500 slots).
/// Slots where the first pass decodes one or two signals and a weaker
/// one may only show once they are removed.
const MASKED: [(&str, usize); 5] = [
    ("near", 250),
    ("near2", 250),
    ("far", 250),
    ("pair", 250),
    ("low", 500),
];

/// The `masked` point of slot `index`.
fn masked_point(index: usize) -> &'static str {
    let mut end = 0;
    for (kind, n) in MASKED {
        end += n;
        if index < end {
            return kind;
        }
    }
    unreachable!()
}

/// Appends a signal at `freq` and `snr` with a distinct message and a
/// random start (−0.3…+0.7 s).
fn push_signal(r: &mut Rng, sent: &mut Vec<Sent>, freq: f64, snr: f64) {
    let avoid: Vec<Payload> = sent
        .iter()
        .map(|s| Payload::from_bits(s.payload).unwrap())
        .collect();
    let (text, payload) = message(r, &avoid);
    let dt = -0.3 + r.uniform();
    sent.push(Sent {
        text,
        payload: payload.bits(),
        freq,
        dt,
        snr,
        channel: None,
    });
}

/// A frequency 5–60 Hz below or above `f`.
fn beside(r: &mut Rng, f: f64) -> f64 {
    let d = 5.0 + 55.0 * r.uniform();
    if below(r, 2) == 0 {
        f - d
    } else {
        f + d
    }
}

/// The signals of a `masked` slot of point `kind` (draw order fixed).
fn masked(r: &mut Rng, kind: &str) -> Vec<Sent> {
    let mut sent = Vec::new();
    let strong = |r: &mut Rng| -4.0 + 24.0 * r.uniform();
    let weak = |r: &mut Rng| -20.0 + 12.0 * r.uniform();
    if kind == "low" {
        let f = 260.0 + 2_480.0 * r.uniform();
        let snr = -20.0 + 20.0 * r.uniform();
        push_signal(r, &mut sent, f, snr);
        let fw = beside(r, f);
        let weak = -22.0 + (snr - 2.0 + 22.0) * r.uniform();
        push_signal(r, &mut sent, fw, weak);
        return sent;
    }
    let (groups, beside_count) = match kind {
        "near" => (1, 1),
        "near2" => (1, 2),
        "pair" => (2, 1),
        _ => (1, 0),
    };
    for _ in 0..groups {
        let f = 260.0 + 2_480.0 * r.uniform();
        let snr = if kind == "far" {
            5.0 + 15.0 * r.uniform()
        } else {
            strong(r)
        };
        push_signal(r, &mut sent, f, snr);
        for _ in 0..beside_count {
            let (fw, snr) = (beside(r, f), weak(r));
            push_signal(r, &mut sent, fw, snr);
        }
    }
    if kind == "far" {
        let (f, snr) = (200.0 + 2_600.0 * r.uniform(), -21.0 + 6.0 * r.uniform());
        push_signal(r, &mut sent, f, snr);
    }
    sent
}

/// The `empty` set's slot `index`: no signal.
fn empty_slot(index: usize, r: &mut Rng, noise_seed: u64) -> Slot {
    let mut samples = vec![0.0f32; SLOT_SAMPLES];
    let kind = match index {
        0..=999 => "noise",
        1_000..=1_499 => "carriers",
        1_500..=1_699 => "strong",
        1_700..=1_899 => "clipped",
        1_900..=2_099 => "floored",
        _ => "impulsive",
    };
    match kind {
        "carriers" => add_carriers(&mut samples, r, 0.0, 20.0),
        "strong" => add_carriers(&mut samples, r, 20.0, 50.0),
        "clipped" => {
            let f = 200.0 + 2_800.0 * r.uniform();
            for (n, x) in samples.iter_mut().enumerate() {
                *x =
                    (0.8 * libm::cos(2.0 * core::f64::consts::PI * f * n as f64 / 12_000.0)) as f32;
            }
        }
        _ => {}
    }
    let rms = match kind {
        "clipped" => 0.5,
        "floored" => 1e-5,
        _ => NOISE_RMS,
    };
    add_noise(&mut samples, rms, noise_seed);
    if kind == "clipped" {
        for x in samples.iter_mut() {
            *x = x.clamp(-1.0, 1.0);
        }
    }
    if kind == "impulsive" {
        for x in samples.iter_mut() {
            if r.uniform() < 5e-4 {
                let a = (0.3 + 0.7 * r.uniform()) as f32;
                *x += if r.uniform() < 0.5 { a } else { -a };
            }
        }
    }
    Slot {
        point: kind.to_string(),
        samples,
        skip: 0,
        sent: Vec::new(),
    }
}

/// Builds slot `index` of `set` in `block`.
pub fn build(set: &str, block: &str, index: usize) -> Slot {
    let seed = slot_seed(set, block, index);
    let mut r = Rng(seed);
    let noise_seed = noise_seed(seed);
    let one = |r: &mut Rng, snr: f64, dt: Option<f64>, f: Option<f64>| {
        let (text, payload) = message(r, &[]);
        let freq = f.unwrap_or_else(|| 200.0 + 2_600.0 * r.uniform());
        let dt = dt.unwrap_or_else(|| -0.3 + r.uniform());
        Sent {
            text,
            payload: payload.bits(),
            freq,
            dt,
            snr,
            channel: None,
        }
    };
    let (point, sent, skip, cut) = match set {
        "sweep" => {
            let snr = -22.0 + 0.5 * (index / 1_000) as f64;
            let sent = vec![one(&mut r, snr, None, None)];
            (format!("{snr:.1}"), sent, 0, SLOT_SAMPLES)
        }
        "crowded" => {
            let n = [10, 20, 40][index / 200];
            (
                format!("{n}"),
                crowd(&mut r, n, -16.0, 22.0),
                0,
                SLOT_SAMPLES,
            )
        }
        "wide" => {
            let n = [10, 20, 30, 40][index / 200];
            let sent = crowd(&mut r, n, -22.0, 27.0);
            (format!("{n}"), sent, 0, SLOT_SAMPLES)
        }
        "fading" => {
            let n = [10, 20, 40][index / 200];
            let mut sent = crowd(&mut r, n, -16.0, 22.0);
            for s in &mut sent {
                // Field order is draw order.
                s.channel = Some(Channel {
                    fade_hz: 0.05 + 0.95 * r.uniform(),
                    fade_depth: 0.9 * r.uniform(),
                    fade_phase: r.uniform(),
                    drift: -0.2 + 0.4 * r.uniform(),
                });
            }
            (format!("{n}"), sent, 0, SLOT_SAMPLES)
        }
        "edges" => {
            let (label, dt, f, cut, skip) = EDGE_CASES[index / 100];
            let sent = vec![one(&mut r, -18.0, Some(dt), f)];
            (label.to_string(), sent, skip, cut)
        }
        "empty" => return empty_slot(index, &mut r, noise_seed),
        "masked" => {
            let kind = masked_point(index);
            (kind.to_string(), masked(&mut r, kind), 0, SLOT_SAMPLES)
        }
        _ => unreachable!(),
    };
    let mut samples = render(&sent, noise_seed);
    samples.truncate(cut);
    let samples = samples.split_off(skip.min(samples.len()));
    Slot {
        point,
        samples,
        skip,
        sent,
    }
}
