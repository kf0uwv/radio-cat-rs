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

//! Synthesizer → decoder, end to end.

use std::time::Duration;

use cat_morse::decoder::decoded_to_text;
use cat_morse::timing::key_text;
use cat_morse::{
    decode_all, Awgn, Channel, Decoder, DecoderConfig, Fist, PitchMode, Signal, Speed,
};

const SR: u32 = 8000;
const PITCH: f32 = 700.0;

const SHORT: &str = "CQ CQ DE W1XYZ W1XYZ <AR> K";
const LONG: &str = "W1XYZ DE K0ABC = TNX FER CALL UR RST 579 579 = NAME IS BOB BOB \
QTH NR DENVER, CO = RIG IS IC-7300 ES ANT IS DIPOLE @ 40 FT = WX CLOUDY 52F = \
HW? <AR> W1XYZ DE K0ABC <KN>";

/// What the decoder prints for `text` in its default style: `=` is the
/// `<BT>` pattern.
fn as_decoded(text: &str) -> String {
    text.replace('=', "<BT>")
}

/// Character error rate by edit distance over chars (spaces included).
fn cer(expected: &str, got: &str) -> f64 {
    let a: Vec<char> = expected.chars().collect();
    let b: Vec<char> = got.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        prev = cur;
    }
    prev[b.len()] as f64 / a.len() as f64
}

fn channel(text: &str, speed: Speed, fist: &Fist, seed: u64) -> Channel {
    let k = key_text(text, speed, fist, seed).unwrap();
    let mut ch = Channel::new(SR, seed);
    ch.signals
        .push(Signal::new(k, PITCH).starting_at(Duration::from_millis(300)));
    ch.padding = Duration::from_secs(1);
    ch
}

fn fixed() -> DecoderConfig {
    DecoderConfig::new(SR, PitchMode::Fixed(PITCH))
}

#[test]
fn clean_round_trip_at_15_20_30_wpm() {
    for wpm in [15.0, 20.0, 30.0] {
        let pcm = channel(SHORT, Speed::wpm(wpm).unwrap(), &Fist::perfect(), 1)
            .render()
            .unwrap();
        assert_eq!(decode_all(fixed(), &pcm).unwrap(), SHORT, "at {wpm} wpm");
    }
}

#[test]
fn clean_long_text_at_25_wpm() {
    let pcm = channel(LONG, Speed::wpm(25.0).unwrap(), &Fist::perfect(), 2)
        .render()
        .unwrap();
    assert_eq!(decode_all(fixed(), &pcm).unwrap(), as_decoded(LONG));
}

#[test]
fn auto_pitch_finds_the_tone() {
    for wpm in [15.0, 30.0] {
        let k = key_text(SHORT, Speed::wpm(wpm).unwrap(), &Fist::perfect(), 0).unwrap();
        let mut ch = Channel::new(SR, 3);
        ch.signals
            .push(Signal::new(k, 640.0).starting_at(Duration::from_millis(300)));
        ch.padding = Duration::from_secs(1);
        let pcm = ch.render().unwrap();
        let cfg = DecoderConfig::new(
            SR,
            PitchMode::Auto {
                min_hz: 300.0,
                max_hz: 1200.0,
            },
        );
        let mut d = Decoder::new(cfg).unwrap();
        let mut out = d.feed(&pcm);
        out.extend(d.flush());
        assert_eq!(decoded_to_text(&out).trim(), SHORT, "at {wpm} wpm");
        let p = d.pitch_hz().unwrap();
        assert!((p - 640.0).abs() <= 15.0, "pitch {p}");
    }
}

#[test]
fn moderate_snr_keeps_cer_low() {
    for seed in 10..14 {
        let mut ch = channel(LONG, Speed::wpm(20.0).unwrap(), &Fist::perfect(), seed);
        ch.noise = Some(Awgn::snr(6.0));
        let pcm = ch.render().unwrap();
        let got = decode_all(fixed(), &pcm).unwrap();
        let e = cer(&as_decoded(LONG), &got);
        assert!(e <= 0.02, "seed {seed}: CER {e:.3}\n{got}");
    }
}

#[test]
fn human_fist_keeps_cer_low() {
    // A typical fist makes genuinely ambiguous elements now and then, so
    // this bounds the average and the worst case rather than demanding
    // perfection from every seed.
    let seeds = 20..30u64;
    let mut total = 0.0;
    for seed in seeds.clone() {
        let pcm = channel(LONG, Speed::wpm(20.0).unwrap(), &Fist::typical(), seed)
            .render()
            .unwrap();
        let got = decode_all(fixed(), &pcm).unwrap();
        let e = cer(&as_decoded(LONG), &got);
        assert!(e <= 0.05, "seed {seed}: CER {e:.3}\n{got}");
        total += e;
    }
    let mean = total / seeds.count() as f64;
    assert!(mean <= 0.01, "mean CER {mean:.4}");
}

#[test]
fn farnsworth_word_spaces_are_right() {
    let text = "TNX FER CALL ES GE OM UR RST 599 HR IN DENVER";
    let pcm = channel(
        text,
        Speed::farnsworth(18.0, 8.0).unwrap(),
        &Fist::perfect(),
        4,
    )
    .render()
    .unwrap();
    assert_eq!(decode_all(fixed(), &pcm).unwrap(), text);
}

#[test]
fn qrm_400_hz_away_is_rejected() {
    let speed = Speed::wpm(22.0).unwrap();
    let mut ch = channel(SHORT, speed, &Fist::perfect(), 5);
    let qrm = key_text(
        "TEST TEST DE N2QRM N2QRM TEST",
        Speed::wpm(28.0).unwrap(),
        &Fist::perfect(),
        0,
    )
    .unwrap();
    ch.signals.push(
        Signal::new(qrm, PITCH + 400.0)
            .at_level(-3.0)
            .starting_at(Duration::from_millis(100)),
    );
    let pcm = ch.render().unwrap();
    assert_eq!(decode_all(fixed(), &pcm).unwrap(), SHORT);
}

#[test]
fn qrm_on_a_noisy_band_is_rejected() {
    let speed = Speed::wpm(22.0).unwrap();
    let mut ch = channel(SHORT, speed, &Fist::perfect(), 8);
    let qrm = key_text(
        "TEST TEST DE N2QRM N2QRM TEST",
        Speed::wpm(28.0).unwrap(),
        &Fist::typical(),
        1,
    )
    .unwrap();
    ch.signals
        .push(Signal::new(qrm, PITCH - 400.0).starting_at(Duration::from_millis(50)));
    ch.noise = Some(Awgn::snr(15.0));
    let pcm = ch.render().unwrap();
    assert_eq!(decode_all(fixed(), &pcm).unwrap(), SHORT);
}

#[test]
fn block_size_does_not_change_output() {
    let mut ch = channel(SHORT, Speed::wpm(24.0).unwrap(), &Fist::good(), 6);
    ch.noise = Some(Awgn::snr(10.0));
    let pcm = ch.render().unwrap();
    let reference = decode_all(fixed(), &pcm).unwrap();
    for block in [1usize, 37, 4096] {
        let mut d = Decoder::new(fixed()).unwrap();
        let mut out = Vec::new();
        for chunk in pcm.chunks(block) {
            out.extend(d.feed(chunk));
        }
        out.extend(d.flush());
        assert_eq!(decoded_to_text(&out).trim(), reference, "block {block}");
    }
}

#[test]
fn speed_estimate_tracks_sender() {
    let pcm = channel(LONG, Speed::wpm(30.0).unwrap(), &Fist::perfect(), 7)
        .render()
        .unwrap();
    let mut d = Decoder::new(fixed()).unwrap();
    d.feed(&pcm);
    let w = d.wpm_estimate().unwrap();
    assert!((w - 30.0).abs() < 2.0, "estimated {w} wpm");
}
