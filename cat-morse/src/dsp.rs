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

//! Small DSP pieces shared by the synthesizer and the decoder.

use std::f64::consts::PI;

/// Power of `freq_hz` in `samples` by the Goertzel algorithm, scaled so a
/// sine of amplitude `A` centred on the bin reads `A²/2`. For white noise
/// it reads the noise power in a bandwidth of `sample_rate / len`.
pub fn goertzel_power(samples: &[f32], sample_rate: u32, freq_hz: f32) -> f32 {
    let n = samples.len();
    if n == 0 || sample_rate == 0 {
        return 0.0;
    }
    let w = 2.0 * PI * f64::from(freq_hz) / f64::from(sample_rate);
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in samples {
        let s0 = f64::from(x) + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let mag2 = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    (2.0 * mag2 / (n as f64 * n as f64)) as f32
}

/// A precomputed Goertzel bin for repeated use on equal-length windows.
#[allow(dead_code)] // first used by the decoder
#[derive(Debug, Clone, Copy)]
pub(crate) struct GoertzelBin {
    coeff: f64,
    scale: f64,
}

#[allow(dead_code)]
impl GoertzelBin {
    pub(crate) fn new(sample_rate: u32, freq_hz: f32, len: usize) -> Self {
        let w = 2.0 * PI * f64::from(freq_hz) / f64::from(sample_rate);
        GoertzelBin {
            coeff: 2.0 * w.cos(),
            scale: 2.0 / (len as f64 * len as f64),
        }
    }

    /// Power over `window` (whose length should be the `len` given to `new`),
    /// read in two pieces so a ring buffer needs no copy.
    pub(crate) fn power(&self, a: &[f32], b: &[f32]) -> f64 {
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &x in a.iter().chain(b.iter()) {
            let s0 = f64::from(x) + self.coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        (s1 * s1 + s2 * s2 - self.coeff * s1 * s2) * self.scale
    }
}

/// An RBJ band-pass biquad (0 dB peak gain), direct form I.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Biquad {
    b0: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    pub(crate) fn bandpass(sample_rate: u32, center_hz: f32, bandwidth_hz: f32) -> Self {
        let w0 = 2.0 * PI * f64::from(center_hz) / f64::from(sample_rate);
        let q = f64::from(center_hz) / f64::from(bandwidth_hz);
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Biquad {
            b0: alpha / a0,
            b2: -alpha / a0,
            a1: -2.0 * w0.cos() / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    pub(crate) fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(sr: u32, f: f32, a: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| a * (2.0 * PI * f64::from(f) * i as f64 / f64::from(sr)).sin() as f32)
            .collect()
    }

    #[test]
    fn goertzel_reads_sine_power() {
        let s = sine(8000, 700.0, 0.5, 800);
        let p = goertzel_power(&s, 8000, 700.0);
        assert!((p - 0.125).abs() < 0.002, "{p}");
        let off = goertzel_power(&s, 8000, 1500.0);
        assert!(off < 1e-4, "{off}");
    }

    #[test]
    fn bin_matches_free_function() {
        let s = sine(8000, 600.0, 0.3, 96);
        let bin = GoertzelBin::new(8000, 600.0, 96);
        let p = bin.power(&s[..40], &s[40..]);
        assert!((p as f32 - goertzel_power(&s, 8000, 600.0)).abs() < 1e-6);
    }

    #[test]
    fn bandpass_passes_center_and_rejects_far() {
        let mut bq = Biquad::bandpass(8000, 700.0, 500.0);
        let s = sine(8000, 700.0, 1.0, 8000);
        let out: Vec<f32> = s.iter().map(|&x| bq.process(f64::from(x)) as f32).collect();
        let p = goertzel_power(&out[4000..], 8000, 700.0);
        assert!((p - 0.5).abs() < 0.02, "{p}");
        let mut bq = Biquad::bandpass(8000, 700.0, 500.0);
        let s = sine(8000, 2500.0, 1.0, 8000);
        let out: Vec<f32> = s.iter().map(|&x| bq.process(f64::from(x)) as f32).collect();
        assert!(goertzel_power(&out[4000..], 8000, 2500.0) < 0.05);
    }
}
