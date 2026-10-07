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

//! Regression tests for the 2026-10-06 code review of `feat/cat-morse`
//! (`planning/code_review/findings.md`), one per confirmed finding.

use std::time::Duration;

use cat_morse::decoder::decoded_to_text;
use cat_morse::timing::key_text;
use cat_morse::{
    decode_all, Awgn, Channel, Decoder, DecoderConfig, Fist, Keying, PitchMode, Rng, Signal, Speed,
};

const SR: u32 = 8000;

fn fixed() -> DecoderConfig {
    DecoderConfig::new(SR, PitchMode::Fixed(700.0))
}

fn pcm(text: &str, wpm: f64, start_ms: u64, pad_ms: u64, pitch: f32) -> Vec<f32> {
    let k = key_text(text, Speed::wpm(wpm).unwrap(), &Fist::perfect(), 0).unwrap();
    let mut ch = Channel::new(SR, 1);
    ch.signals
        .push(Signal::new(k, pitch).starting_at(Duration::from_millis(start_ms)));
    ch.padding = Duration::from_millis(pad_ms);
    ch.render().unwrap()
}

// Finding 1 -----------------------------------------------------------------

#[test]
fn tone_from_t0_decodes_fully() {
    for wpm in [15.0, 20.0, 25.0, 30.0] {
        let p = pcm("PARIS PARIS", wpm, 0, 500, 700.0);
        assert_eq!(decode_all(fixed(), &p).unwrap(), "PARIS PARIS", "{wpm} wpm");
    }
}

#[test]
fn joining_on_the_first_mark_decodes_fully() {
    // Start the stream exactly where the first mark's ramp begins.
    let k = key_text(
        "PARIS PARIS PARIS PARIS",
        Speed::wpm(25.0).unwrap(),
        &Fist::perfect(),
        0,
    )
    .unwrap();
    let mut ch = Channel::new(SR, 1);
    ch.signals
        .push(Signal::new(k, 700.0).starting_at(Duration::from_millis(300)));
    ch.padding = Duration::from_millis(500);
    ch.noise = Some(Awgn::snr(30.0));
    let p = ch.render().unwrap();
    for skip_ms in [297u64, 300] {
        let s = (skip_ms * 8) as usize;
        assert_eq!(
            decode_all(fixed(), &p[s..]).unwrap(),
            "PARIS PARIS PARIS PARIS",
            "joined at {skip_ms} ms"
        );
    }
}

// Finding 2 -----------------------------------------------------------------

fn carrier_then_paris(carrier: Duration, gap: Duration) -> Vec<f32> {
    let mut k = Keying::new();
    k.push(true, carrier);
    let rest = key_text(
        "PARIS PARIS PARIS PARIS",
        Speed::wpm(20.0).unwrap(),
        &Fist::perfect(),
        0,
    )
    .unwrap();
    k.append(&rest, gap);
    let mut ch = Channel::new(SR, 1);
    ch.signals
        .push(Signal::new(k, 700.0).starting_at(Duration::from_millis(300)));
    ch.padding = Duration::from_millis(500);
    ch.render().unwrap()
}

#[test]
fn long_carrier_does_not_poison_speed() {
    let p = carrier_then_paris(Duration::from_secs(3), Duration::from_millis(700));
    let got = decode_all(fixed(), &p).unwrap();
    assert!(got.ends_with("PARIS PARIS PARIS PARIS"), "{got}");
}

#[test]
fn one_second_mark_does_not_poison_speed() {
    let p = carrier_then_paris(Duration::from_secs(1), Duration::from_millis(420));
    let got = decode_all(fixed(), &p).unwrap();
    assert!(got.ends_with("PARIS PARIS PARIS PARIS"), "{got}");
}

#[test]
fn crash_on_weak_signal_keeps_word_spaces() {
    let text = "PARIS PARIS PARIS PARIS PARIS PARIS";
    for (level, snr) in [
        (-20.0f32, None),
        (-20.0, Some(25.0)),
        (0.0, None),
        (-30.0, None),
    ] {
        let k = key_text(text, Speed::wpm(20.0).unwrap(), &Fist::perfect(), 0).unwrap();
        let mut ch = Channel::new(SR, 1);
        ch.signals.push(
            Signal::new(k, 700.0)
                .starting_at(Duration::from_millis(300))
                .at_level(level),
        );
        ch.padding = Duration::from_millis(500);
        ch.noise = snr.map(Awgn::snr);
        let mut p = ch.render().unwrap();
        let mut r = Rng::new(5);
        for x in &mut p[3 * 8000..3 * 8000 + 160] {
            *x = (r.gauss() * 0.5).clamp(-1.0, 1.0) as f32;
        }
        let got = decode_all(fixed(), &p).unwrap();
        let words: Vec<&str> = got.split(' ').collect();
        // The crash may cost the letters it lands on, but not the rest.
        assert!(words.len() >= 6, "level {level} snr {snr:?}: {got}");
        assert!(
            got.ends_with("PARIS PARIS PARIS"),
            "level {level} snr {snr:?}: {got}"
        );
    }
}

// Finding 9 -----------------------------------------------------------------

#[test]
fn a_huge_finite_sample_does_not_deafen_the_decoder() {
    let base = pcm("PARIS PARIS PARIS PARIS", 20.0, 300, 500, 700.0);
    for at in [100usize, 8000 * 3] {
        let mut p = base.clone();
        p[at] = 1e30;
        let got = decode_all(fixed(), &p).unwrap();
        assert!(got.ends_with("PARIS PARIS PARIS"), "1e30 at {at}: {got}");
    }
}

// Finding 3 -----------------------------------------------------------------

#[test]
fn narrow_auto_range_locks() {
    for (lo, hi, tone) in [
        (690.0, 720.0, 705.0),
        (690.0, 720.0, 700.0),
        (690.0, 710.0, 700.0),
    ] {
        let p = pcm("PARIS PARIS PARIS", 20.0, 300, 500, tone);
        let mut d = Decoder::new(DecoderConfig::new(
            SR,
            PitchMode::Auto {
                min_hz: lo,
                max_hz: hi,
            },
        ))
        .unwrap();
        let mut out = d.feed(&p);
        out.extend(d.flush());
        let pitch = d.pitch_hz().expect("locks");
        assert!((pitch - tone).abs() <= 10.0, "[{lo},{hi}] {tone}: {pitch}");
        assert_eq!(decoded_to_text(&out).trim(), "PARIS PARIS PARIS");
    }
}

// Finding 6 -----------------------------------------------------------------

#[test]
fn auto_pitch_reacquires_a_new_station() {
    let a = key_text(
        "CQ CQ DE W1XYZ",
        Speed::wpm(20.0).unwrap(),
        &Fist::perfect(),
        0,
    )
    .unwrap();
    let b = key_text(
        "TEST DE K9ABC",
        Speed::wpm(25.0).unwrap(),
        &Fist::perfect(),
        0,
    )
    .unwrap();
    let mut ch = Channel::new(SR, 3);
    let a_end = Duration::from_millis(300) + a.total();
    ch.signals
        .push(Signal::new(a, 600.0).starting_at(Duration::from_millis(300)));
    ch.signals
        .push(Signal::new(b, 900.0).starting_at(a_end + Duration::from_secs(3)));
    ch.padding = Duration::from_millis(500);
    ch.noise = Some(Awgn::snr(25.0));
    let p = ch.render().unwrap();
    let mut d = Decoder::new(DecoderConfig::new(
        SR,
        PitchMode::Auto {
            min_hz: 400.0,
            max_hz: 1000.0,
        },
    ))
    .unwrap();
    let mut out = d.feed(&p);
    out.extend(d.flush());
    let text = decoded_to_text(&out);
    assert!(text.contains("CQ CQ DE W1XYZ"), "{text}");
    assert!(text.trim_end().ends_with("TEST DE K9ABC"), "{text}");
    assert!((d.pitch_hz().unwrap() - 900.0).abs() <= 10.0);
}

// Finding 7 (decoder half) ----------------------------------------------------

#[test]
fn pitches_near_zero_or_nyquist_are_rejected() {
    use cat_morse::DecoderError;
    for f in [3990.0f32, 3950.0, 50.0] {
        assert!(
            matches!(
                Decoder::new(DecoderConfig::new(SR, PitchMode::Fixed(f))),
                Err(DecoderError::PitchOutOfRange(_))
            ),
            "{f}"
        );
    }
    assert!(Decoder::new(DecoderConfig::new(
        SR,
        PitchMode::Auto {
            min_hz: 300.0,
            max_hz: 3990.0
        }
    ))
    .is_err());
    assert!(Decoder::new(DecoderConfig::new(SR, PitchMode::Fixed(3900.0))).is_ok());
}

#[test]
fn auto_pitch_does_not_lock_on_band_noise() {
    for seed in 0..5 {
        let mut ch = Channel::new(SR, seed);
        ch.noise = Some(Awgn::snr(10.0));
        ch.padding = Duration::from_secs(10);
        let p = ch.render().unwrap();
        let mut d = Decoder::new(DecoderConfig::new(
            SR,
            PitchMode::Auto {
                min_hz: 400.0,
                max_hz: 1000.0,
            },
        ))
        .unwrap();
        d.feed(&p);
        assert_eq!(d.pitch_hz(), None, "seed {seed}");
    }
}

// Finding 5 -----------------------------------------------------------------

#[test]
fn first_sample_is_silent() {
    let k = key_text("E", Speed::wpm(20.0).unwrap(), &Fist::perfect(), 0).unwrap();
    let mut ch = Channel::new(48_000, 3);
    ch.signals.push(Signal::new(k, 700.0));
    let p = ch.render().unwrap();
    assert_eq!(p[0], 0.0);
    // The ramp starts from silence (the bug started it at half amplitude,
    // 0.124 at sample 0).
    assert!(p[..8].iter().all(|x| x.abs() < 1e-3), "{:?}", &p[..8]);
}

#[test]
fn band_channel_qrm_starts_from_silence() {
    use cat_morse::scenario::{band_channel, BandConditions, Difficulty, QrmOdds};
    let mut d = Difficulty::intermediate();
    d.band = BandConditions {
        qrm: Some(QrmOdds {
            chance: 1.0,
            offset_hz: (200.0, 400.0),
            level_db: (0.0, 0.0),
        }),
        ..BandConditions::clean()
    };
    let k = key_text("TEST TEST", Speed::wpm(20.0).unwrap(), &Fist::perfect(), 0).unwrap();
    for seed in 0..40 {
        let (ch, _) =
            band_channel(&d, vec![Signal::new(k.clone(), 700.0)], 8000, 700.0, seed).unwrap();
        assert_eq!(ch.render().unwrap()[0], 0.0, "seed {seed}");
    }
}

// Finding 7 (band half) -------------------------------------------------------

#[test]
fn band_channel_keeps_qrm_inside_the_passband() {
    use cat_morse::scenario::{band_channel, BandConditions, Difficulty, QrmOdds};
    let mut d = Difficulty::intermediate();
    d.band = BandConditions {
        qrm: Some(QrmOdds {
            chance: 1.0,
            offset_hz: (200.0, 400.0),
            level_db: (-6.0, -6.0),
        }),
        ..BandConditions::clean()
    };
    let k = key_text("TEST TEST", Speed::wpm(20.0).unwrap(), &Fist::perfect(), 0).unwrap();
    for seed in 0..60 {
        let (ch, r) =
            band_channel(&d, vec![Signal::new(k.clone(), 3800.0)], 8000, 3800.0, seed).unwrap();
        if let Some(q) = r.qrm {
            let f = 3800.0 + q.offset_hz;
            assert!((100.0..=3900.0).contains(&f), "seed {seed}: {f}");
        }
        ch.render().expect("renders");
    }
    assert!(band_channel(&d, vec![Signal::new(k, 3950.0)], 8000, 3950.0, 0).is_err());
}

// Finding 8 -----------------------------------------------------------------

#[test]
fn speeds_below_the_floor_are_rejected() {
    use cat_morse::timing::MIN_WPM;
    assert_eq!(MIN_WPM, 3.0);
    assert!(Speed::wpm(2.9).is_err());
    assert!(Speed::wpm(1e-6).is_err());
    assert!(Speed::farnsworth(100.0, 1e-9).is_err());
    assert!(Speed::farnsworth(20.0, 2.0).is_err());
    assert!(Speed::wpm(3.0).is_ok());
    assert!(Speed::farnsworth(18.0, 3.0).is_ok());
}

// Finding 10 ----------------------------------------------------------------

#[cfg(feature = "serde")]
#[test]
fn callsign_deserializes_through_parse() {
    use cat_morse::Callsign;
    let ok: Callsign = serde_json::from_str("\"kf0uwv\"").unwrap();
    assert_eq!(ok.as_str(), "KF0UWV");
    assert!(serde_json::from_str::<Callsign>("\"NOTACALL\"").is_err());
    assert!(serde_json::from_str::<Callsign>("\"K1//AB\"").is_err());
    let back: Callsign = serde_json::from_str(&serde_json::to_string(&ok).unwrap()).unwrap();
    assert_eq!(back, ok);
}

#[test]
fn align_unordered_handles_a_huge_copy() {
    use cat_morse::{align_unordered, ScoreOptions};
    let calls: Vec<String> = (0..40)
        .map(|i| format!("W{}A{}B", i % 10, (b'A' + (i / 10) as u8) as char))
        .collect();
    let expected: Vec<&str> = calls.iter().map(String::as_str).collect();
    let mut copied: Vec<&str> = expected.clone();
    copied.reverse();
    let mut copied = copied.join(" ");
    copied.push_str(&" XX".repeat(200));
    let t = std::time::Instant::now();
    let a = align_unordered(&expected, &copied, &ScoreOptions::default()).unwrap();
    assert!(t.elapsed() < std::time::Duration::from_secs(5));
    let s = a.score();
    assert_eq!((s.substitutions, s.deletions), (0, 0));
    assert_eq!(s.insertions, 400);
    assert_eq!(
        a,
        align_unordered(&expected, &copied, &ScoreOptions::default()).unwrap()
    );
}

// Finding 11 ----------------------------------------------------------------
//
// Recorded on Linux x86_64 when these tests were written. ADR 0021 promises
// keying and scripts are bit-identical on every platform for a seed; the
// Windows CI job runs these, so drift there fails here. A deliberate change
// to Rng, timing or scenario generation must re-record them.

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[test]
fn keying_is_pinned_across_platforms() {
    let k = key_text(
        "CQ DE W1AW",
        Speed::wpm(22.0).unwrap(),
        &Fist::typical(),
        42,
    )
    .unwrap();
    let nanos: Vec<u128> = k.segments().iter().map(|s| s.duration.as_nanos()).collect();
    let text = format!("{nanos:?}");
    assert_eq!(&nanos[..4], PINNED_KEYING_HEAD, "{text}");
    assert_eq!(fnv(&text), PINNED_KEYING_FNV, "FNV {:#x}", fnv(&text));
}

const PINNED_KEYING_HEAD: &[u128] = &[169_269_560, 50_109_755, 62_735_823, 50_499_321];
const PINNED_KEYING_FNV: u64 = 0xcca7_8cdd_f630_a914;

#[test]
fn script_is_pinned_across_platforms() {
    use cat_morse::scenario::{Difficulty, Party};
    use cat_morse::{generate, Callsign, ScenarioConfig, ScenarioKind};
    let s = generate(&ScenarioConfig {
        kind: ScenarioKind::DxPileup { callers: 4 },
        my_call: Callsign::parse("KF0UWV").unwrap(),
        difficulty: Difficulty::advanced(),
        seed: 7,
    })
    .unwrap();
    let mut text = String::new();
    for st in &s.stations {
        let g = &st.signal;
        text.push_str(&format!(
            "{} {} {} {} {} {:?} {}|",
            st.persona.call,
            g.speed.char_wpm(),
            g.pitch_offset_hz.to_bits(),
            g.strength_db.to_bits(),
            g.fist_seed,
            g.fist.dash_ratio.to_bits(),
            st.persona.qth
        ));
    }
    for t in &s.transmissions {
        let who = match t.from {
            Party::You => "you".to_string(),
            Party::Them(i) => i.to_string(),
        };
        text.push_str(&format!("{who}:{}:{:?}|", t.text, t.start));
    }
    assert_eq!(
        s.stations[0].persona.call.as_str(),
        PINNED_FIRST_CALL,
        "{text}"
    );
    assert_eq!(fnv(&text), PINNED_SCRIPT_FNV, "FNV {:#x}", fnv(&text));
}

const PINNED_FIRST_CALL: &str = "CU1PZU";
const PINNED_SCRIPT_FNV: u64 = 0x4b3c_333d_abad_0ddc;
