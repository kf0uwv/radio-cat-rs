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

//! The band an over is copied through, and the [`Channel`] that renders it.
//!
//! [`BandConditions`] says how likely and how bad each impairment is;
//! [`band_channel`] draws one concrete band from it with a seed and wraps
//! an over's [`Signal`]s in it, so a caller never assembles noise, fading,
//! crashes and QRM by hand. The same seed gives the same draw on every
//! platform (only integer and basic float arithmetic decide it).

use std::time::Duration;

use super::callsign::{dx_call, us_call, Callsign};
use super::data::DX;
use super::{Difficulty, ScenarioError};
use crate::rng::Rng;
use crate::synth::{Awgn, Channel, Qrn, Qsb, RxFilter, Signal};
use crate::timing::{key_text, Fist, Speed};

/// How likely, and how deep, slow fading is — drawn per station.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct QsbOdds {
    /// Probability (0–1) that a station fades.
    pub chance: f64,
    /// Fade depth range, dB.
    pub depth_db: (f32, f32),
    /// Fade period range, seconds.
    pub period_s: (f32, f32),
}

/// How likely, and how bad, static crashes are — drawn per over.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct QrnOdds {
    /// Probability (0–1) that an over has QRN.
    pub chance: f64,
    /// Crash rate range, per second.
    pub crashes_per_sec: (f32, f32),
    /// Crash level range, dB relative to the synth reference.
    pub level_db: (f32, f32),
    /// Crash decay range, milliseconds.
    pub decay_ms: (u32, u32),
}

/// How likely, and how close and loud, an interfering station is — drawn
/// per over.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct QrmOdds {
    /// Probability (0–1) that an over has QRM.
    pub chance: f64,
    /// Distance from the operator's pitch, Hz (either side).
    pub offset_hz: (f32, f32),
    /// Level range, dB relative to the synth reference.
    pub level_db: (f32, f32),
}

/// The band an over is copied through.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BandConditions {
    /// AWGN SNR range in dB, in the synth's 500 Hz reference bandwidth;
    /// `None` is a noiseless band.
    pub snr_db: Option<(f32, f32)>,
    /// Fading.
    pub qsb: Option<QsbOdds>,
    /// Static crashes.
    pub qrn: Option<QrnOdds>,
    /// An interfering station.
    pub qrm: Option<QrmOdds>,
    /// A receive CW filter of this bandwidth centred on the pitch.
    pub rx_bandwidth_hz: Option<f32>,
}

impl Default for BandConditions {
    fn default() -> Self {
        BandConditions::clean()
    }
}

impl BandConditions {
    /// No noise, no fading, no crashes, no QRM, no filter.
    pub fn clean() -> Self {
        BandConditions {
            snr_db: None,
            qsb: None,
            qrn: None,
            qrm: None,
            rx_bandwidth_hz: None,
        }
    }

    /// A quiet band: 20 dB SNR and nothing else.
    pub fn beginner() -> Self {
        BandConditions {
            snr_db: Some((19.0, 21.0)),
            rx_bandwidth_hz: Some(500.0),
            ..BandConditions::clean()
        }
    }

    /// An ordinary evening: ~10 dB, some fading, light static, some QRM.
    pub fn intermediate() -> Self {
        BandConditions {
            snr_db: Some((9.0, 11.0)),
            qsb: Some(QsbOdds {
                chance: 0.4,
                depth_db: (5.0, 7.0),
                period_s: (6.0, 10.0),
            }),
            qrn: Some(QrnOdds {
                chance: 1.0,
                crashes_per_sec: (0.2, 0.4),
                level_db: (-11.0, -9.0),
                decay_ms: (30, 50),
            }),
            qrm: Some(QrmOdds {
                chance: 0.3,
                offset_hz: (200.0, 400.0),
                level_db: (-7.0, -5.0),
            }),
            rx_bandwidth_hz: Some(500.0),
        }
    }

    /// A hard band: ~3 dB, deep fading, heavy static, close QRM.
    pub fn advanced() -> Self {
        BandConditions {
            snr_db: Some((2.0, 4.0)),
            qsb: Some(QsbOdds {
                chance: 0.7,
                depth_db: (10.0, 14.0),
                period_s: (4.0, 6.0),
            }),
            qrn: Some(QrnOdds {
                chance: 1.0,
                crashes_per_sec: (0.8, 1.2),
                level_db: (-4.0, -2.0),
                decay_ms: (50, 70),
            }),
            qrm: Some(QrmOdds {
                chance: 0.6,
                offset_hz: (100.0, 300.0),
                level_db: (-4.0, -2.0),
            }),
            rx_bandwidth_hz: Some(500.0),
        }
    }

    pub(super) fn validate(&self) -> Result<(), ScenarioError> {
        let bad = |why| Err(ScenarioError::InvalidSetting(why));
        let range = |(a, b): (f32, f32)| a.is_finite() && b.is_finite() && a <= b;
        let p = |x: f64| (0.0..=1.0).contains(&x);
        if let Some(r) = self.snr_db {
            if !range(r) {
                return bad("SNR range must be finite with lo ≤ hi");
            }
        }
        if let Some(q) = self.qsb {
            if !p(q.chance) || !range(q.depth_db) || q.depth_db.0 < 0.0 {
                return bad("QSB needs a chance in [0, 1] and a depth range ≥ 0");
            }
            if !range(q.period_s) || q.period_s.0 <= 0.0 {
                return bad("QSB period range must be > 0");
            }
        }
        if let Some(q) = self.qrn {
            if !p(q.chance) || !range(q.crashes_per_sec) || q.crashes_per_sec.0 < 0.0 {
                return bad("QRN needs a chance in [0, 1] and a rate range ≥ 0");
            }
            if !range(q.level_db) || q.decay_ms.0 == 0 || q.decay_ms.0 > q.decay_ms.1 {
                return bad("QRN needs a level range and a decay range > 0 ms");
            }
        }
        if let Some(q) = self.qrm {
            if !p(q.chance) || !range(q.offset_hz) || q.offset_hz.0 < 0.0 || !range(q.level_db) {
                return bad("QRM needs a chance in [0, 1], an offset range ≥ 0 and a level range");
            }
        }
        if let Some(b) = self.rx_bandwidth_hz {
            if !(b.is_finite() && b > 0.0) {
                return bad("receive filter bandwidth must be > 0");
            }
        }
        Ok(())
    }
}

/// The interfering station [`band_channel`] added.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct QrmReport {
    /// Its callsign.
    pub call: Callsign,
    /// What it sent (possibly cut off by the end of the over).
    pub text: String,
    /// Its pitch minus the operator's pitch, Hz.
    pub offset_hz: f32,
    /// Its level, dB.
    pub level_db: f32,
}

/// What [`band_channel`] drew, for logging or showing the operator.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BandReport {
    /// The SNR, if there is noise.
    pub snr_db: Option<f32>,
    /// Fading per input signal, in the order given (`None`: steady, or the
    /// signal already had its own QSB and kept it).
    pub qsb: Vec<Option<Qsb>>,
    /// Static crashes, if any.
    pub qrn: Option<Qrn>,
    /// The QRM station, if any.
    pub qrm: Option<QrmReport>,
}

const STREAM_BAND: u64 = 0x4241_4E44; // "BAND"

fn draw(rng: &mut Rng, (lo, hi): (f32, f32)) -> f32 {
    let x = rng.range_f64(f64::from(lo), f64::from(hi));
    ((x * 10.0).round() / 10.0) as f32
}

/// Wraps an over's `signals` in a band drawn from `difficulty.band` with
/// `seed`: AWGN, per-signal QSB (a signal that already has QSB keeps its
/// own), QRN, an interfering station and a receive filter at `pitch_hz`.
///
/// The QRM station is generated here — a random call sending a CQ or a
/// contest call at its own speed and fist — starts somewhere in the first
/// half of the over and is cut off where the over ends, so it never makes
/// the audio longer. Set [`Channel::padding`] and [`Channel::gain`] on the
/// result as usual.
pub fn band_channel(
    difficulty: &Difficulty,
    signals: Vec<Signal>,
    sample_rate: u32,
    pitch_hz: f32,
    seed: u64,
) -> Result<(Channel, BandReport), ScenarioError> {
    let band = &difficulty.band;
    band.validate()?;
    let margin = crate::decoder::PITCH_MARGIN_HZ;
    let top = sample_rate as f32 / 2.0 - margin;
    if !(pitch_hz.is_finite() && pitch_hz >= margin && pitch_hz <= top) {
        return Err(ScenarioError::InvalidSetting(
            "pitch must be at least 100 Hz from 0 Hz and from Nyquist",
        ));
    }
    let root = Rng::new(seed).fork(STREAM_BAND);
    let mut rng_noise = root.fork(1);
    let mut rng_qsb = root.fork(2);
    let mut rng_qrn = root.fork(3);
    let mut rng_qrm = root.fork(4);

    let mut ch = Channel::new(sample_rate, seed);
    let mut report = BandReport {
        snr_db: None,
        qsb: Vec::with_capacity(signals.len()),
        qrn: None,
        qrm: None,
    };

    if let Some(r) = band.snr_db {
        let snr = draw(&mut rng_noise, r);
        ch.noise = Some(Awgn::snr(snr));
        report.snr_db = Some(snr);
    }

    let over_end = signals.iter().map(Signal::end).max().unwrap_or_default();
    for mut s in signals {
        // Draw for every signal, used or not, so one signal's own QSB does
        // not shift the draws for the next.
        let fade = band.qsb.map(|q| {
            let hit = rng_qsb.chance(q.chance);
            let depth = draw(&mut rng_qsb, q.depth_db);
            let period = draw(&mut rng_qsb, q.period_s);
            (hit, depth, period)
        });
        let added = match (s.qsb, fade) {
            (None, Some((true, depth, period))) => {
                let q = Qsb {
                    depth_db: depth,
                    period: Duration::from_millis((f64::from(period) * 1000.0).round() as u64),
                };
                s.qsb = Some(q);
                Some(q)
            }
            _ => None,
        };
        report.qsb.push(added);
        ch.signals.push(s);
    }

    if let Some(q) = band.qrn {
        if rng_qrn.chance(q.chance) {
            let qrn = Qrn {
                crashes_per_sec: draw(&mut rng_qrn, q.crashes_per_sec),
                level_db: draw(&mut rng_qrn, q.level_db),
                decay: Duration::from_millis(u64::from(
                    rng_qrn.range_u32(q.decay_ms.0, q.decay_ms.1),
                )),
            };
            ch.qrn = Some(qrn);
            report.qrn = Some(qrn);
        }
    }

    if let Some(q) = band.qrm {
        let hit = rng_qrm.chance(q.chance);
        if hit && over_end >= Duration::from_secs(1) {
            let call_text = if rng_qrm.chance(0.7) {
                us_call(&mut rng_qrm, None)
            } else {
                let e = rng_qrm.pick(DX);
                dx_call(&mut rng_qrm, e)
            };
            let call = Callsign::parse(&call_text).expect("generated calls are well formed");
            let text = match rng_qrm.range_u32(0, 2) {
                0 => format!("CQ CQ DE {call} {call} K"),
                1 => format!("TEST {call} {call} TEST"),
                _ => format!("CQ TEST {call} {call}"),
            };
            let magnitude = draw(&mut rng_qrm, q.offset_hz);
            // Either side, as long as it stays inside the passband the
            // decoder accepts; if neither side fits there is no QRM.
            let fits = |f: f32| f >= margin && f <= top;
            let want_below = rng_qrm.chance(0.5);
            let (lo, hi) = (pitch_hz - magnitude, pitch_hz + magnitude);
            let offset = match (want_below, fits(lo), fits(hi)) {
                (true, true, _) | (false, true, false) => Some(-magnitude),
                (_, _, true) => Some(magnitude),
                _ => None,
            };
            let level = draw(&mut rng_qrm, q.level_db);
            let offset = offset.unwrap_or(0.0);
            let wpm = f64::from(rng_qrm.range_u32(18, 30));
            let fist_seed = rng_qrm.next_u64();
            let max_ms = (over_end.as_millis() / 2) as u32;
            let start = Duration::from_millis(u64::from(rng_qrm.range_u32(0, max_ms)));
            let keying = key_text(&text, Speed::wpm(wpm)?, &Fist::typical(), fist_seed)
                .map_err(ScenarioError::from)?
                .truncated(over_end - start);
            if !keying.is_empty() && offset != 0.0 {
                ch.signals.push(
                    Signal::new(keying, pitch_hz + offset)
                        .starting_at(start)
                        .at_level(level),
                );
                report.qrm = Some(QrmReport {
                    call,
                    text,
                    offset_hz: offset,
                    level_db: level,
                });
            }
        }
    }

    if let Some(bw) = band.rx_bandwidth_hz {
        ch.filter = Some(RxFilter {
            center_hz: pitch_hz,
            bandwidth_hz: bw,
        });
    }
    Ok((ch, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::Keying;

    fn over() -> Vec<Signal> {
        let k = key_text(
            "CQ CQ DE W1XYZ W1XYZ K",
            Speed::wpm(20.0).unwrap(),
            &Fist::perfect(),
            0,
        )
        .unwrap();
        vec![Signal::new(k, 700.0).starting_at(Duration::from_millis(300))]
    }

    fn with_band(band: BandConditions) -> Difficulty {
        Difficulty {
            band,
            ..Difficulty::intermediate()
        }
    }

    #[test]
    fn same_seed_same_band() {
        let d = with_band(BandConditions::advanced());
        let a = band_channel(&d, over(), 8000, 700.0, 9).unwrap();
        let b = band_channel(&d, over(), 8000, 700.0, 9).unwrap();
        assert_eq!(a, b);
        let reports: Vec<BandReport> = (0..20)
            .map(|s| band_channel(&d, over(), 8000, 700.0, s).unwrap().1)
            .collect();
        assert!(reports.windows(2).any(|w| w[0] != w[1]));
    }

    #[test]
    fn a_clean_band_adds_nothing() {
        let d = with_band(BandConditions::clean());
        let (ch, r) = band_channel(&d, over(), 8000, 700.0, 1).unwrap();
        assert!(ch.noise.is_none() && ch.qrn.is_none() && ch.filter.is_none());
        assert_eq!(ch.signals, over());
        assert_eq!(r.snr_db, None);
        assert_eq!(r.qsb, vec![None]);
        assert!(r.qrm.is_none());
    }

    #[test]
    fn beginner_is_quiet_noise_only() {
        let d = Difficulty::beginner();
        let (ch, r) = band_channel(&d, over(), 8000, 700.0, 2).unwrap();
        let snr = r.snr_db.unwrap();
        assert!((19.0..=21.0).contains(&snr));
        assert!(ch.qrn.is_none() && r.qrm.is_none());
        assert_eq!(ch.signals.len(), 1);
        assert_eq!(ch.filter.unwrap().center_hz, 700.0);
    }

    #[test]
    fn odds_are_respected_over_many_seeds() {
        let d = Difficulty::advanced();
        let n = 400;
        let (mut qsb, mut qrm) = (0, 0);
        for seed in 0..n {
            let (ch, r) = band_channel(&d, over(), 8000, 700.0, seed).unwrap();
            qsb += usize::from(r.qsb[0].is_some());
            if let Some(q) = &r.qrm {
                qrm += 1;
                assert!(
                    (100.0..=300.0).contains(&q.offset_hz.abs()),
                    "{}",
                    q.offset_hz
                );
                assert!((-4.0..=-2.0).contains(&q.level_db));
                // Never longer than the over itself.
                let end = over()[0].end();
                assert!(ch.signals[1].start + ch.signals[1].keying.total() <= end);
            }
            let qrn = ch.qrn.unwrap();
            assert!((0.8..=1.2).contains(&qrn.crashes_per_sec));
        }
        let p_qsb = qsb as f64 / n as f64;
        let p_qrm = qrm as f64 / n as f64;
        assert!((p_qsb - 0.7).abs() < 0.08, "QSB {p_qsb}");
        assert!((p_qrm - 0.6).abs() < 0.08, "QRM {p_qrm}");
    }

    #[test]
    fn a_signals_own_qsb_is_kept() {
        let mut d = with_band(BandConditions::advanced());
        if let Some(q) = &mut d.band.qsb {
            q.chance = 1.0;
        }
        let own = Qsb {
            depth_db: 1.0,
            period: Duration::from_secs(1),
        };
        let mut sigs = over();
        sigs[0].qsb = Some(own);
        let (ch, r) = band_channel(&d, sigs, 8000, 700.0, 3).unwrap();
        assert_eq!(ch.signals[0].qsb, Some(own));
        assert_eq!(r.qsb, vec![None]);
    }

    #[test]
    fn the_channel_renders_and_short_overs_get_no_qrm() {
        let mut d = Difficulty::advanced();
        if let Some(q) = &mut d.band.qrm {
            q.chance = 1.0;
        }
        let (ch, r) = band_channel(&d, over(), 8000, 700.0, 4).unwrap();
        assert!(r.qrm.is_some());
        let pcm = ch.render().unwrap();
        assert_eq!(pcm.len(), ch.len_samples());
        let mut short = Keying::new();
        short.push(true, Duration::from_millis(60));
        let (_, r) = band_channel(&d, vec![Signal::new(short, 700.0)], 8000, 700.0, 4).unwrap();
        assert!(r.qrm.is_none());
    }

    #[test]
    fn bad_conditions_are_rejected() {
        let mut d = Difficulty::intermediate();
        d.band.snr_db = Some((10.0, 5.0));
        assert!(band_channel(&d, over(), 8000, 700.0, 0).is_err());
        let mut d = Difficulty::intermediate();
        if let Some(q) = &mut d.band.qrm {
            q.chance = 2.0;
        }
        assert!(band_channel(&d, over(), 8000, 700.0, 0).is_err());
        assert!(band_channel(&Difficulty::intermediate(), over(), 8000, 0.0, 0).is_err());
    }
}
