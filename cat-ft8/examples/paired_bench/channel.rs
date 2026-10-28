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

//! A second waveform generator, in `f64` with `libm`, for signals the
//! crate's `Synth` does not make: a start between samples, an amplitude
//! fade and a linear frequency drift. It follows the same definition
//! (QEX 2020 §5: the Gaussian-smoothed frequency pulse with BT = 2, the
//! first and last tones extended, the raised-cosine ramps over T/8, phase
//! accumulated sample by sample), evaluated at the continuous waveform
//! time of each output sample. With a whole-sample start and no fade or
//! drift it matches `Synth` to 6.6 × 10⁻⁴ of full scale (measured by
//! [`check_against_synth`]; `suppression` fails above 10⁻³).

use std::sync::OnceLock;

use cat_ft8::waveform::{synthesize, Placement};
use cat_ft8::{SAMPLES_PER_SYMBOL, SAMPLE_RATE, SYMBOL_COUNT, TONE_SPACING_HZ, WAVEFORM_SAMPLES};

/// A signal's channel: a sinusoidal amplitude fade and a linear drift.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Channel {
    /// Fade rate, Hz: the amplitude is 1 − depth·(1 − cos 2π(rate·t +
    /// phase))/2, t in seconds from the waveform's first sample.
    pub fade_hz: f64,
    /// Fade depth, 0…1 (the amplitude dips to 1 − depth).
    pub fade_depth: f64,
    /// Fade phase, turns.
    pub fade_phase: f64,
    /// Frequency drift, Hz/s, zero at the middle of the waveform.
    pub drift: f64,
}

/// Pulse samples per waveform sample in the table.
const OVERSAMPLE: usize = 16;

/// The frequency pulse over three symbols, `OVERSAMPLE` points per sample.
fn pulse_table() -> &'static [f64] {
    static TABLE: OnceLock<Vec<f64>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let k = core::f64::consts::PI * libm::sqrt(2.0 / core::f64::consts::LN_2);
        let ns = SAMPLES_PER_SYMBOL as f64;
        (0..=3 * SAMPLES_PER_SYMBOL * OVERSAMPLE)
            .map(|i| {
                let t = (i as f64 / OVERSAMPLE as f64 - 1.5 * ns) / ns;
                0.5 * (libm::erf(k * 2.0 * (t + 0.5)) - libm::erf(k * 2.0 * (t - 0.5)))
            })
            .collect()
    })
}

/// The pulse at `x` samples from its start (0…3 symbols), interpolated.
fn pulse(x: f64) -> f64 {
    let t = pulse_table();
    let u = x * OVERSAMPLE as f64;
    if u <= 0.0 || u >= (t.len() - 1) as f64 {
        return 0.0;
    }
    let i = u as usize;
    let f = u - i as f64;
    t[i] + f * (t[i + 1] - t[i])
}

/// Tone deviation (in tone steps) at waveform time `tau` samples.
fn deviation(tones: &[u8; SYMBOL_COUNT], tau: f64) -> f64 {
    let ns = SAMPLES_PER_SYMBOL as f64;
    let sym = (tau / ns).floor() as i64;
    let mut d = 0.0;
    for m in sym - 1..=sym + 1 {
        let b = f64::from(tones[m.clamp(0, SYMBOL_COUNT as i64 - 1) as usize]);
        // Symbol m's pulse starts at (m − 1)·T.
        d += b * pulse(tau - (m - 1) as f64 * ns);
    }
    d
}

/// The raised-cosine envelope at waveform time `tau` samples.
fn envelope(tau: f64) -> f64 {
    let ramp = (SAMPLES_PER_SYMBOL / 8) as f64;
    let r = tau.min((WAVEFORM_SAMPLES - 1) as f64 - tau);
    if r < 0.0 {
        0.0
    } else if r >= ramp {
        1.0
    } else {
        0.5 * (1.0 - libm::cos(core::f64::consts::PI * r / ramp))
    }
}

/// Adds the waveform of `tones` with tone 0 at `base_hz` (at the middle),
/// its first sample at slot sample `start` (any real number), peak
/// amplitude `amp` and `channel`, into `out`.
pub fn render(
    out: &mut [f32],
    tones: &[u8; SYMBOL_COUNT],
    base_hz: f64,
    start: f64,
    amp: f64,
    channel: &Channel,
) {
    let fs = f64::from(SAMPLE_RATE);
    let middle = WAVEFORM_SAMPLES as f64 / 2.0;
    let freq = |tau: f64| {
        base_hz
            + channel.drift * (tau - middle) / fs
            + f64::from(TONE_SPACING_HZ) * deviation(tones, tau)
    };
    let tau = |n: i64| n as f64 - start;
    let first = start.ceil() as i64;
    let last = (start + (WAVEFORM_SAMPLES - 1) as f64).floor() as i64;
    // Phase at the first sample: the first tone's frequency over the
    // fraction of a sample since the waveform began (as `Synth`, whose
    // phase is 0 at its first sample).
    let mut phase = 2.0 * core::f64::consts::PI * tau(first) * freq(0.0) / fs;
    for n in first..=last {
        let t = tau(n);
        if let Some(x) = usize::try_from(n).ok().and_then(|i| out.get_mut(i)) {
            let fade = 1.0
                - channel.fade_depth
                    * (1.0
                        - libm::cos(
                            2.0 * core::f64::consts::PI
                                * (channel.fade_hz * t / fs + channel.fade_phase),
                        ))
                    / 2.0;
            *x += (amp * fade * envelope(t) * libm::sin(phase)) as f32;
        }
        phase += 2.0 * core::f64::consts::PI * freq(t) / fs;
    }
}

/// The largest difference between this generator and `Synth` for a few
/// whole-sample starts (peak amplitude 1, no fade or drift).
pub fn check_against_synth() -> f64 {
    let mut worst = 0.0f64;
    let tones: [u8; SYMBOL_COUNT] = core::array::from_fn(|k| ((k * 5 + k / 3) % 8) as u8);
    for (f, start) in [(1_500.0f32, 8_400i64), (212.5, -4_800), (2_987.0, 21_600)] {
        let mut a = vec![0.0f32; cat_ft8::SLOT_SAMPLES];
        let mut b = a.clone();
        let p = Placement {
            base_hz: f,
            start,
            amplitude: 1.0,
        };
        synthesize(&tones, &p, &mut a).unwrap();
        render(
            &mut b,
            &tones,
            f64::from(f),
            start as f64,
            1.0,
            &Channel::default(),
        );
        for (x, y) in a.iter().zip(&b) {
            worst = worst.max(f64::from((x - y).abs()));
        }
    }
    worst
}
