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

//! Keying to f32 PCM, and a simulated band to hear it through.
//!
//! [`render_tone`] turns one [`Keying`] into a clean, click-free tone.
//! [`Channel`] mixes any number of [`Signal`]s — each with its own pitch,
//! start time, strength and QSB — and adds white noise at a stated SNR,
//! QRN crashes and an optional receiver filter. QRM is simply another
//! `Signal` at a pitch offset; a pileup is several.
//!
//! # Edges
//!
//! Every key edge is a raised-cosine ramp of [`ToneShape::rise`] (5 ms by
//! default) whose half-amplitude point sits exactly on the nominal edge,
//! so the keying's timing is preserved and there are no key clicks. A
//! ramp is shortened where a mark or the gap beside it is shorter than
//! the rise time. A rise of zero is hard keying.
//!
//! # Levels and SNR
//!
//! A signal at `level_db = 0` is a sine of amplitude [`REFERENCE_AMPLITUDE`].
//! [`Awgn::snr_db`] is the ratio of that reference sine's power to the
//! noise power within [`Awgn::ref_bandwidth_hz`] (500 Hz by default, a CW
//! filter). So a signal's own SNR is `snr_db + level_db`. The noise is
//! generated white up to Nyquist; [`RxFilter`] band-limits it the way a
//! receiver's CW filter would.
//!
//! # Determinism
//!
//! Output is a pure function of the inputs and the seed. It is bit-for-bit
//! repeatable on one platform; across platforms it agrees to within float
//! tolerance, because the tone itself uses `sin`/`cos` from the platform's
//! maths library (ADR 0021).

use std::f64::consts::PI;
use std::time::Duration;

use crate::dsp::Biquad;
use crate::rng::Rng;
use crate::timing::Keying;

/// The amplitude of a signal at `level_db = 0`: leaves headroom for noise,
/// QRN and several signals at once before the ±1.0 clamp.
pub const REFERENCE_AMPLITUDE: f32 = 0.25;

/// Parameters that cannot be rendered.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SynthError {
    /// The sample rate is zero.
    #[error("sample rate must be non-zero")]
    ZeroSampleRate,
    /// A frequency is not strictly between 0 and Nyquist.
    #[error("frequency {hz} Hz must be in (0, {nyquist}) Hz")]
    FrequencyOutOfRange { hz: f32, nyquist: f32 },
    /// Some other parameter is out of range.
    #[error("invalid parameter: {0}")]
    InvalidParameter(&'static str),
}

/// The shape of key edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ToneShape {
    /// Raised-cosine rise (and fall) time; zero is hard keying.
    pub rise: Duration,
}

impl Default for ToneShape {
    fn default() -> Self {
        ToneShape {
            rise: Duration::from_millis(5),
        }
    }
}

/// Slow fading: a sinusoidal dip of `depth_db` every `period`, with a
/// phase drawn from the channel seed.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Qsb {
    /// Peak-to-trough depth in dB (≥ 0).
    pub depth_db: f32,
    /// One full fade cycle (> 0).
    pub period: Duration,
}

/// One transmitted signal in a [`Channel`].
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    /// What is sent.
    pub keying: Keying,
    /// When the first key-down edge happens, from the channel's start.
    pub start: Duration,
    /// Audio pitch.
    pub pitch_hz: f32,
    /// Strength relative to the reference sine (see the module docs).
    pub level_db: f32,
    /// Optional fading.
    pub qsb: Option<Qsb>,
    /// Edge shape.
    pub shape: ToneShape,
}

impl Signal {
    /// A signal at `pitch_hz`, level 0 dB, starting at zero, no fading,
    /// 5 ms edges.
    pub fn new(keying: Keying, pitch_hz: f32) -> Self {
        Signal {
            keying,
            start: Duration::ZERO,
            pitch_hz,
            level_db: 0.0,
            qsb: None,
            shape: ToneShape::default(),
        }
    }

    /// Sets the start time.
    pub fn starting_at(mut self, start: Duration) -> Self {
        self.start = start;
        self
    }

    /// Sets the level in dB.
    pub fn at_level(mut self, level_db: f32) -> Self {
        self.level_db = level_db;
        self
    }

    /// Adds fading.
    pub fn with_qsb(mut self, qsb: Qsb) -> Self {
        self.qsb = Some(qsb);
        self
    }

    /// When the last ramp has finished.
    pub fn end(&self) -> Duration {
        self.start + self.keying.total() + self.shape.rise / 2
    }
}

/// Additive white Gaussian noise at a stated SNR.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Awgn {
    /// Reference-sine power over noise power in `ref_bandwidth_hz`, in dB.
    pub snr_db: f32,
    /// The bandwidth the SNR is stated in.
    pub ref_bandwidth_hz: f32,
}

impl Awgn {
    /// Noise at `snr_db` in the default 500 Hz reference bandwidth.
    pub fn snr(snr_db: f32) -> Self {
        Awgn {
            snr_db,
            ref_bandwidth_hz: 500.0,
        }
    }
}

/// Impulse noise: static crashes arriving at random.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Qrn {
    /// Mean crashes per second (≥ 0).
    pub crashes_per_sec: f32,
    /// Peak crash level relative to the reference sine, in dB.
    pub level_db: f32,
    /// Time for a crash to decay by 1/e (> 0).
    pub decay: Duration,
}

/// A receiver's CW filter: a two-pole band-pass on the whole mix.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RxFilter {
    /// Centre frequency.
    pub center_hz: f32,
    /// −3 dB bandwidth.
    pub bandwidth_hz: f32,
}

/// A simulated band: signals plus impairments, rendered to mono f32 PCM.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    /// Output sample rate.
    pub sample_rate: u32,
    /// Everything transmitting.
    pub signals: Vec<Signal>,
    /// Background noise.
    pub noise: Option<Awgn>,
    /// Static crashes.
    pub qrn: Option<Qrn>,
    /// Receiver filter applied to the mix.
    pub filter: Option<RxFilter>,
    /// Linear gain applied last, before the ±1.0 clamp.
    pub gain: f32,
    /// Extra time rendered after the last signal ends.
    pub padding: Duration,
    /// Seed for noise, QRN and per-signal phase/fade.
    pub seed: u64,
}

impl Channel {
    /// An empty, noiseless channel.
    pub fn new(sample_rate: u32, seed: u64) -> Self {
        Channel {
            sample_rate,
            signals: Vec::new(),
            noise: None,
            qrn: None,
            filter: None,
            gain: 1.0,
            padding: Duration::ZERO,
            seed,
        }
    }

    /// Total rendered duration: the latest signal end plus `padding`.
    pub fn duration(&self) -> Duration {
        self.signals
            .iter()
            .map(Signal::end)
            .max()
            .unwrap_or(Duration::ZERO)
            + self.padding
    }

    /// Total samples [`Channel::render`] produces.
    pub fn len_samples(&self) -> usize {
        (self.duration().as_secs_f64() * f64::from(self.sample_rate)).round() as usize
    }

    /// Renders the whole channel.
    pub fn render(&self) -> Result<Vec<f32>, SynthError> {
        let mut r = self.renderer()?;
        let mut out = vec![0.0; self.len_samples()];
        let n = r.fill(&mut out);
        out.truncate(n);
        Ok(out)
    }

    /// A streaming renderer, for feeding a sound card block by block.
    pub fn renderer(&self) -> Result<ChannelRenderer, SynthError> {
        ChannelRenderer::new(self)
    }
}

#[derive(Debug, Clone, Copy)]
struct MarkWin {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    r: f64,
}

#[derive(Debug, Clone)]
struct SignalState {
    marks: Vec<MarkWin>,
    cursor: usize,
    omega: f64,
    phase: f64,
    amp: f64,
    qsb: Option<(f64, f64, f64)>, // depth_db, period_s, phase
}

impl SignalState {
    fn new(sig: &Signal, amp: f64, phase: f64, qsb_phase: f64) -> Self {
        let rise = sig.shape.rise.as_secs_f64();
        let segs = sig.keying.segments();
        let mut marks = Vec::new();
        let mut t = sig.start.as_secs_f64();
        for (i, s) in segs.iter().enumerate() {
            let len = s.duration.as_secs_f64();
            if s.key_down {
                let mut r = rise.min(len);
                if i > 0 && !segs[i - 1].key_down {
                    r = r.min(segs[i - 1].duration.as_secs_f64());
                }
                if let Some(n) = segs.get(i + 1) {
                    if !n.key_down {
                        r = r.min(n.duration.as_secs_f64());
                    }
                }
                marks.push(MarkWin {
                    a: t - r / 2.0,
                    b: t + r / 2.0,
                    c: t + len - r / 2.0,
                    d: t + len + r / 2.0,
                    r,
                });
            }
            t += len;
        }
        SignalState {
            marks,
            cursor: 0,
            omega: 2.0 * PI * f64::from(sig.pitch_hz),
            phase,
            amp,
            qsb: sig
                .qsb
                .map(|q| (f64::from(q.depth_db), q.period.as_secs_f64(), qsb_phase)),
        }
    }

    fn envelope(&mut self, t: f64) -> f64 {
        while let Some(m) = self.marks.get(self.cursor) {
            if t >= m.d {
                self.cursor += 1;
            } else {
                break;
            }
        }
        let Some(m) = self.marks.get(self.cursor) else {
            return 0.0;
        };
        if t < m.a {
            0.0
        } else if m.r <= 0.0 {
            1.0
        } else if t < m.b {
            0.5 - 0.5 * (PI * (t - m.a) / m.r).cos()
        } else if t < m.c {
            1.0
        } else {
            0.5 - 0.5 * (PI * (m.d - t) / m.r).cos()
        }
    }

    fn sample(&mut self, t: f64) -> f64 {
        let env = self.envelope(t);
        if env == 0.0 {
            return 0.0;
        }
        let mut gain = self.amp * env;
        if let Some((depth, period, ph)) = self.qsb {
            let fade_db = -depth * (0.5 - 0.5 * (2.0 * PI * t / period + ph).cos());
            gain *= 10f64.powf(fade_db / 20.0);
        }
        gain * (self.omega * t + self.phase).sin()
    }
}

/// Streams a [`Channel`] in blocks of any size; the blocks concatenate to
/// exactly what [`Channel::render`] returns.
#[derive(Debug, Clone)]
pub struct ChannelRenderer {
    sample_rate: f64,
    n: usize,
    total: usize,
    signals: Vec<SignalState>,
    noise_sigma: f64,
    noise_rng: Rng,
    qrn: Option<(f64, f64, f64)>, // p per sample, peak amp, decay factor
    qrn_rng: Rng,
    qrn_env: f64,
    filter: Option<Biquad>,
    gain: f64,
    clipped: u64,
}

const STREAM_NOISE: u64 = 1;
const STREAM_QRN: u64 = 2;
const STREAM_SIGNALS: u64 = 3;

fn check_freq(hz: f32, sr: u32) -> Result<(), SynthError> {
    let nyquist = sr as f32 / 2.0;
    if hz.is_finite() && hz > 0.0 && hz < nyquist {
        Ok(())
    } else {
        Err(SynthError::FrequencyOutOfRange { hz, nyquist })
    }
}

impl ChannelRenderer {
    fn new(ch: &Channel) -> Result<Self, SynthError> {
        let sr = ch.sample_rate;
        if sr == 0 {
            return Err(SynthError::ZeroSampleRate);
        }
        if !ch.gain.is_finite() {
            return Err(SynthError::InvalidParameter("gain must be finite"));
        }
        let root = Rng::new(ch.seed);
        let mut sig_rng = root.fork(STREAM_SIGNALS);
        let mut signals = Vec::with_capacity(ch.signals.len());
        for s in &ch.signals {
            check_freq(s.pitch_hz, sr)?;
            if !s.level_db.is_finite() {
                return Err(SynthError::InvalidParameter("signal level must be finite"));
            }
            if let Some(q) = s.qsb {
                if !(q.depth_db.is_finite() && q.depth_db >= 0.0) || q.period.is_zero() {
                    return Err(SynthError::InvalidParameter(
                        "QSB needs depth ≥ 0 dB and a non-zero period",
                    ));
                }
            }
            let amp = f64::from(REFERENCE_AMPLITUDE) * 10f64.powf(f64::from(s.level_db) / 20.0);
            let phase = 2.0 * PI * sig_rng.unit_f64();
            let qsb_phase = 2.0 * PI * sig_rng.unit_f64();
            signals.push(SignalState::new(s, amp, phase, qsb_phase));
        }
        let noise_sigma = match ch.noise {
            None => 0.0,
            Some(a) => {
                if !a.snr_db.is_finite() {
                    return Err(SynthError::InvalidParameter("SNR must be finite"));
                }
                check_freq(a.ref_bandwidth_hz, sr).map_err(|_| {
                    SynthError::InvalidParameter("reference bandwidth must be in (0, Nyquist)")
                })?;
                let p_ref = f64::from(REFERENCE_AMPLITUDE).powi(2) / 2.0;
                let p_in_band = p_ref / 10f64.powf(f64::from(a.snr_db) / 10.0);
                let nyquist = f64::from(sr) / 2.0;
                (p_in_band * nyquist / f64::from(a.ref_bandwidth_hz)).sqrt()
            }
        };
        let qrn = match ch.qrn {
            None => None,
            Some(q) => {
                if !(q.crashes_per_sec.is_finite() && q.crashes_per_sec >= 0.0)
                    || q.decay.is_zero()
                    || !q.level_db.is_finite()
                {
                    return Err(SynthError::InvalidParameter(
                        "QRN needs a rate ≥ 0, a finite level and a non-zero decay",
                    ));
                }
                let p = f64::from(q.crashes_per_sec) / f64::from(sr);
                let peak =
                    f64::from(REFERENCE_AMPLITUDE) * 10f64.powf(f64::from(q.level_db) / 20.0);
                let k = (-1.0 / (q.decay.as_secs_f64() * f64::from(sr))).exp();
                Some((p, peak, k))
            }
        };
        let filter = match ch.filter {
            None => None,
            Some(f) => {
                check_freq(f.center_hz, sr)?;
                if !(f.bandwidth_hz.is_finite() && f.bandwidth_hz > 0.0) {
                    return Err(SynthError::InvalidParameter("filter bandwidth must be > 0"));
                }
                Some(Biquad::bandpass(sr, f.center_hz, f.bandwidth_hz))
            }
        };
        Ok(ChannelRenderer {
            sample_rate: f64::from(sr),
            n: 0,
            total: ch.len_samples(),
            signals,
            noise_sigma,
            noise_rng: root.fork(STREAM_NOISE),
            qrn,
            qrn_rng: root.fork(STREAM_QRN),
            qrn_env: 0.0,
            filter,
            gain: f64::from(ch.gain),
            clipped: 0,
        })
    }

    /// Samples not yet rendered.
    pub fn remaining(&self) -> usize {
        self.total - self.n
    }

    /// Renders into `out`; returns how many samples were written. Zero
    /// means the channel is finished.
    pub fn fill(&mut self, out: &mut [f32]) -> usize {
        let count = out.len().min(self.remaining());
        for slot in out.iter_mut().take(count) {
            let t = self.n as f64 / self.sample_rate;
            let mut x = 0.0;
            for s in &mut self.signals {
                x += s.sample(t);
            }
            if self.noise_sigma > 0.0 {
                x += self.noise_sigma * self.noise_rng.gauss();
            }
            if let Some((p, peak, k)) = self.qrn {
                if self.qrn_rng.chance(p) {
                    let a = peak * self.qrn_rng.range_f64(0.5, 1.5);
                    if a > self.qrn_env {
                        self.qrn_env = a;
                    }
                }
                if self.qrn_env > 1e-9 {
                    x += self.qrn_env * self.qrn_rng.gauss();
                    self.qrn_env *= k;
                }
            }
            if let Some(f) = &mut self.filter {
                x = f.process(x);
            }
            let y = x * self.gain;
            let c = y.clamp(-1.0, 1.0);
            if c != y {
                self.clipped += 1;
            }
            *slot = c as f32;
            self.n += 1;
        }
        count
    }

    /// Samples clamped to ±1.0 so far; lower [`Channel::gain`] if non-zero.
    pub fn clipped(&self) -> u64 {
        self.clipped
    }
}

/// Renders `keying` alone as a clean tone of `amplitude` at `pitch_hz`.
///
/// The keying is placed `shape.rise / 2` after the start so the first
/// ramp is whole, and the output runs `shape.rise / 2` past its end:
/// `round((keying.total() + shape.rise) · sample_rate)` samples.
pub fn render_tone(
    keying: &Keying,
    sample_rate: u32,
    pitch_hz: f32,
    amplitude: f32,
    shape: &ToneShape,
) -> Result<Vec<f32>, SynthError> {
    if sample_rate == 0 {
        return Err(SynthError::ZeroSampleRate);
    }
    check_freq(pitch_hz, sample_rate)?;
    if !amplitude.is_finite() {
        return Err(SynthError::InvalidParameter("amplitude must be finite"));
    }
    let sig = Signal {
        keying: keying.clone(),
        start: shape.rise / 2,
        pitch_hz,
        level_db: 0.0,
        qsb: None,
        shape: *shape,
    };
    let mut st = SignalState::new(&sig, f64::from(amplitude), 0.0, 0.0);
    let sr = f64::from(sample_rate);
    let n = ((keying.total() + shape.rise).as_secs_f64() * sr).round() as usize;
    Ok((0..n).map(|i| st.sample(i as f64 / sr) as f32).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::goertzel_power;
    use crate::timing::{key_text, Fist, Speed};

    fn keying(text: &str, wpm: f64) -> Keying {
        key_text(text, Speed::wpm(wpm).unwrap(), &Fist::perfect(), 0).unwrap()
    }

    #[test]
    fn length_is_total_plus_rise() {
        let k = keying("PARIS", 20.0);
        let pcm = render_tone(&k, 48_000, 700.0, 0.5, &ToneShape::default()).unwrap();
        assert_eq!(pcm.len(), ((2.58 + 0.005) * 48_000.0f64).round() as usize);
    }

    #[test]
    fn no_discontinuities_anywhere() {
        let k = keying("CQ CQ DE W1XYZ <AR>", 30.0);
        // 713 Hz so key edges do not all fall on zero crossings of the tone
        // (at 700 Hz and 30 wpm every edge is a whole number of cycles in).
        let (sr, f, a) = (48_000u32, 713.0f32, 0.8f32);
        let shape = ToneShape::default();
        let pcm = render_tone(&k, sr, f, a, &shape).unwrap();
        let omega = 2.0 * PI * f64::from(f) / f64::from(sr);
        let env_slope = PI / (2.0 * shape.rise.as_secs_f64() * f64::from(sr));
        let bound = f64::from(a) * (omega + env_slope) * 1.02;
        let worst = pcm
            .windows(2)
            .map(|w| f64::from((w[1] - w[0]).abs()))
            .fold(0.0, f64::max);
        assert!(worst <= bound, "worst step {worst} > bound {bound}");

        // Hard keying, by contrast, jumps.
        let hard = render_tone(
            &k,
            sr,
            f,
            a,
            &ToneShape {
                rise: Duration::ZERO,
            },
        )
        .unwrap();
        let worst_hard = hard
            .windows(2)
            .map(|w| f64::from((w[1] - w[0]).abs()))
            .fold(0.0, f64::max);
        assert!(worst_hard > bound * 2.0);
    }

    #[test]
    fn marks_start_and_end_at_zero() {
        let k = keying("EEE", 20.0);
        let sr = 48_000;
        let pcm = render_tone(&k, sr, 700.0, 1.0, &ToneShape::default()).unwrap();
        assert!(pcm[0].abs() < 1e-3);
        assert!(pcm[pcm.len() - 1].abs() < 1e-2);
        // Middle of each inter-element gap is silent.
        let gap_mid = ((0.0025 + 0.060 + 0.030) * f64::from(sr)) as usize;
        assert_eq!(pcm[gap_mid], 0.0);
    }

    #[test]
    fn ramps_cut_key_clicks() {
        let k = keying("EEEEEEEEEEEEEEEEEEEE", 25.0);
        let sr = 8000;
        let soft = render_tone(&k, sr, 700.0, 1.0, &ToneShape::default()).unwrap();
        let hard = render_tone(
            &k,
            sr,
            700.0,
            1.0,
            &ToneShape {
                rise: Duration::ZERO,
            },
        )
        .unwrap();
        // Sideband energy 300 to 1300 Hz above the carrier, sampled densely
        // (2 Hz) so the keying's spectral lines are not stepped over.
        let splatter = |x: &[f32]| -> f32 {
            (0..500)
                .map(|i| goertzel_power(x, sr, 1000.0 + 2.0 * i as f32))
                .sum()
        };
        let ratio_db = 10.0 * (splatter(&hard) / splatter(&soft)).log10();
        assert!(ratio_db >= 20.0, "only {ratio_db} dB less splatter");
    }

    #[test]
    fn no_drift_over_ten_minutes() {
        // 10 minutes of dots at 20 wpm: 5000 dot/gap pairs.
        let mut k = Keying::new();
        for _ in 0..5000 {
            k.push(true, Duration::from_millis(60));
            k.push(false, Duration::from_millis(60));
        }
        let sr = 8000;
        let mut ch = Channel::new(sr, 0);
        ch.signals
            .push(Signal::new(k, 700.0).starting_at(Duration::from_millis(100)));
        let mut r = ch.renderer().unwrap();
        let mut buf = vec![0.0; ch.len_samples()];
        r.fill(&mut buf);
        // The last dot starts at 0.1 + 4999·0.12 s; its centre must be loud
        // and the gap centre after it silent.
        let start = 0.1 + 4999.0 * 0.12;
        let at = |s: f64| buf[(s * f64::from(sr)) as usize];
        let peak = (0..40)
            .map(|i| at(start + 0.01 + i as f64 / 8000.0).abs())
            .fold(0.0f32, f32::max);
        assert!(peak > 0.2);
        assert_eq!(at(start + 0.09), 0.0);
    }

    #[test]
    fn awgn_matches_stated_snr() {
        let sr = 8000u32;
        let mut ch = Channel::new(sr, 3);
        ch.noise = Some(Awgn::snr(6.0));
        ch.padding = Duration::from_secs(20);
        let noise = ch.render().unwrap();
        // Goertzel over N = sr/500 samples reads noise power in 500 Hz.
        let n = (sr / 500) as usize;
        let windows = noise.chunks_exact(n);
        let count = windows.len() as f64;
        let p_noise: f64 = windows
            .map(|w| f64::from(goertzel_power(w, sr, 1000.0)))
            .sum::<f64>()
            / count;
        let p_ref = f64::from(REFERENCE_AMPLITUDE).powi(2) / 2.0;
        let snr = 10.0 * (p_ref / p_noise).log10();
        assert!((snr - 6.0).abs() < 0.5, "measured {snr} dB");
    }

    #[test]
    fn qsb_fades_by_its_depth() {
        let mut k = Keying::new();
        k.push(true, Duration::from_secs(4));
        let sr = 8000;
        let mut ch = Channel::new(sr, 1);
        ch.signals.push(Signal::new(k, 600.0).with_qsb(Qsb {
            depth_db: 12.0,
            period: Duration::from_secs(2),
        }));
        let pcm = ch.render().unwrap();
        let powers: Vec<f32> = pcm[400..31_600]
            .chunks_exact(400)
            .map(|w| goertzel_power(w, sr, 600.0))
            .collect();
        let max = powers.iter().copied().fold(0.0, f32::max);
        let min = powers.iter().copied().fold(f32::MAX, f32::min);
        let depth = 10.0 * (max / min).log10();
        assert!((depth - 12.0).abs() < 1.0, "depth {depth}");
    }

    #[test]
    fn qrn_crash_rate_is_roughly_right() {
        let sr = 8000;
        let mut ch = Channel::new(sr, 5);
        ch.qrn = Some(Qrn {
            crashes_per_sec: 2.0,
            level_db: 6.0,
            decay: Duration::from_millis(10),
        });
        ch.padding = Duration::from_secs(60);
        let pcm = ch.render().unwrap();
        // Count onsets: 5 ms windows going from quiet to loud.
        let loud: Vec<bool> = pcm
            .chunks_exact(40)
            .map(|w| w.iter().map(|x| x * x).sum::<f32>() / 40.0 > 0.002)
            .collect();
        let onsets = loud.windows(2).filter(|w| !w[0] && w[1]).count();
        assert!((80..=160).contains(&onsets), "{onsets} crashes in 60 s");
    }

    #[test]
    fn same_seed_same_pcm_and_streaming_matches() {
        let mut ch = Channel::new(8000, 77);
        ch.signals.push(Signal::new(keying("TEST", 20.0), 700.0));
        ch.signals.push(
            Signal::new(keying("QRM QRM", 28.0), 1100.0)
                .at_level(-6.0)
                .starting_at(Duration::from_millis(150)),
        );
        ch.noise = Some(Awgn::snr(10.0));
        ch.qrn = Some(Qrn {
            crashes_per_sec: 1.0,
            level_db: 0.0,
            decay: Duration::from_millis(20),
        });
        ch.filter = Some(RxFilter {
            center_hz: 800.0,
            bandwidth_hz: 800.0,
        });
        let a = ch.render().unwrap();
        let b = ch.render().unwrap();
        assert_eq!(a, b);
        let mut r = ch.renderer().unwrap();
        let mut streamed = Vec::new();
        let mut buf = [0.0f32; 37];
        loop {
            let n = r.fill(&mut buf);
            if n == 0 {
                break;
            }
            streamed.extend_from_slice(&buf[..n]);
        }
        assert_eq!(streamed, a);
        ch.seed = 78;
        assert_ne!(ch.render().unwrap(), a);
    }

    #[test]
    fn clipping_is_counted() {
        let mut ch = Channel::new(8000, 0);
        ch.signals.push(Signal::new(keying("T", 20.0), 700.0));
        ch.gain = 10.0;
        let mut r = ch.renderer().unwrap();
        let mut buf = vec![0.0; ch.len_samples()];
        r.fill(&mut buf);
        assert!(r.clipped() > 0);
        assert!(buf.iter().all(|x| x.abs() <= 1.0));
    }

    #[test]
    fn bad_parameters_are_errors() {
        let k = keying("E", 20.0);
        assert_eq!(
            render_tone(&k, 0, 700.0, 1.0, &ToneShape::default()),
            Err(SynthError::ZeroSampleRate)
        );
        assert!(matches!(
            render_tone(&k, 8000, 5000.0, 1.0, &ToneShape::default()),
            Err(SynthError::FrequencyOutOfRange { .. })
        ));
        let mut ch = Channel::new(8000, 0);
        ch.noise = Some(Awgn {
            snr_db: 0.0,
            ref_bandwidth_hz: 0.0,
        });
        assert!(ch.render().is_err());
    }
}
