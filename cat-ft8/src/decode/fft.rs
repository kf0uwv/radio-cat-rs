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

//! The spectrum of one 3 840-sample window: a Hann window, then a real
//! FFT computed as a 1 920-point complex mixed-radix Cooley–Tukey
//! transform (Cooley and Tukey, 1965; Singleton, 1969) plus the standard
//! real-split step. Own code; `f32` `+ − ×` only, in a fixed order.
//!
//! - **Complex transform.** Decimation in time, in place. The input is
//!   loaded in digit-reversed order (a compile-time table), then six
//!   stages of radix 4, 4, 4, 2, 3, 5 (innermost first) run butterflies
//!   whose twiddles come from one static table of e^(−2πit/1920).
//! - **Real split.** The 3 840 real samples are packed as 1 920 complex
//!   values `z[n] = x[2n] + i·x[2n+1]`; bin k of the real transform is
//!   `X[k] = E[k] + W₃₈₄₀ᵏ·O[k]`, with E and O from `Z[k]` and `Z[1920 − k]`.
//! - **Scale.** Power is |X|² / 960², so a full-scale sine centred in a
//!   bin reads 1.0 (0 dB): the Hann window's coherent gain is 1 920 / 2.
//!
//! The twiddle, real-split and window tables are static data generated
//! with `libm` and checked bit-for-bit by a test (`tables.rs`).

use super::tables::{HANN_HALF, SPLIT_TWIDDLES, TWIDDLES};

/// Samples in one analysis window (two symbols at 12 kHz).
pub(crate) const WINDOW: usize = 3840;
/// Points in the complex transform.
const N: usize = WINDOW / 2;
/// Bins of the real transform below Nyquist (3.125 Hz apart).
pub(crate) const BINS: usize = N;
/// Stage radices, innermost first. Their product is `N`.
const STAGES: [usize; 6] = [4, 4, 4, 2, 3, 5];
/// 1 / 960²: power of a full-scale sine centred in a bin becomes 1.0.
const POWER_SCALE: f32 = 1.0 / 921_600.0;

const _: () = {
    let mut p = 1;
    let mut i = 0;
    while i < STAGES.len() {
        p *= STAGES[i];
        i += 1;
    }
    assert!(p == N);
};

/// Position p of the work buffer holds input element `DIGIT_REV[p]`:
/// the top-level stage (radix `STAGES[5]`) splits the input into
/// interleaved subsequences, stored as contiguous blocks, recursively.
/// Computed at compile time.
static DIGIT_REV: [u16; N] = {
    let mut table = [0u16; N];
    let mut p = 0;
    while p < N {
        let mut rest = p;
        let mut size = N;
        let mut n = 0;
        let mut mult = 1;
        let mut s = STAGES.len();
        while s > 0 {
            s -= 1;
            let r = STAGES[s];
            size /= r;
            n += (rest / size) * mult;
            rest %= size;
            mult *= r;
        }
        table[p] = n as u16;
        p += 1;
    }
    table
};

/// A complex value.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Complex {
    pub(crate) re: f32,
    pub(crate) im: f32,
}

impl Complex {
    const ZERO: Complex = Complex { re: 0.0, im: 0.0 };

    #[inline]
    fn from_pair(w: [f32; 2]) -> Self {
        Complex { re: w[0], im: w[1] }
    }

    #[inline]
    fn mul(self, o: Complex) -> Complex {
        Complex {
            re: self.re * o.re - self.im * o.im,
            im: self.re * o.im + self.im * o.re,
        }
    }

    #[inline]
    fn add(self, o: Complex) -> Complex {
        Complex {
            re: self.re + o.re,
            im: self.im + o.im,
        }
    }
}

/// Working storage for one transform: 1 920 complex values (15 360
/// bytes), caller-owned so it can live in a `static`.
#[derive(Debug, Clone)]
pub(crate) struct FftBuffer {
    work: [Complex; N],
}

impl FftBuffer {
    /// A zeroed buffer (usable in a `static` initialiser).
    pub(crate) const fn new() -> Self {
        FftBuffer {
            work: [Complex::ZERO; N],
        }
    }
}

impl Default for FftBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// The Hann window value for sample `n` of `0..WINDOW`: sin²(πn / 3 840).
#[inline]
fn hann(n: usize) -> f32 {
    HANN_HALF[n.min(WINDOW - n)]
}

/// In-place 1 920-point complex DFT of `buf`, whose input is already in
/// digit-reversed order. Output in natural order.
fn fft(buf: &mut [Complex; N]) {
    let mut l = 1;
    for &r in STAGES.iter() {
        let span = l * r;
        let step = N / span; // W_span^x = W_N^(x · step)
        let mut base = 0;
        while base < N {
            for k in 0..l {
                let mut a = [Complex::ZERO; 5];
                for (m, slot) in a.iter_mut().enumerate().take(r) {
                    *slot = buf[base + k + m * l];
                }
                for q in 0..r {
                    // out[q] = Σ_m a[m] · W_span^(m · (k + q·l))
                    let mut acc = Complex::ZERO;
                    for (m, &am) in a.iter().enumerate().take(r) {
                        let t = (m * (k + q * l) * step) % N;
                        acc = acc.add(am.mul(Complex::from_pair(TWIDDLES[t])));
                    }
                    buf[base + k + q * l] = acc;
                }
            }
            base += span;
        }
        l = span;
    }
}

/// The real-split twiddle W₃₈₄₀ᵏ for k in 0..1 920. The table holds
/// k ≤ 960; above that W₃₈₄₀ᵏ = −i · W₃₈₄₀^(k−960), which is exact.
#[inline]
fn split_twiddle(k: usize) -> Complex {
    if k <= 960 {
        Complex::from_pair(SPLIT_TWIDDLES[k])
    } else {
        let w = SPLIT_TWIDDLES[k - 960];
        Complex {
            re: w[1],
            im: -w[0],
        }
    }
}

/// The windowed spectrum of `samples`: bin `k` of the real transform
/// (k = 0..1 920, 3.125 Hz apart) as a complex value, for every bin.
fn spectrum(samples: &[f32; WINDOW], buf: &mut FftBuffer) {
    for (p, slot) in buf.work.iter_mut().enumerate() {
        let n = usize::from(DIGIT_REV[p]);
        *slot = Complex {
            re: samples[2 * n] * hann(2 * n),
            im: samples[2 * n + 1] * hann(2 * n + 1),
        };
    }
    fft(&mut buf.work);
}

/// Real-transform bin `k` (0..1 920) from the complex transform `z`.
#[inline]
fn real_bin(z: &[Complex; N], k: usize) -> Complex {
    let a = z[k];
    let c = z[(N - k) % N];
    let b = Complex {
        re: c.re,
        im: -c.im,
    }; // conj(Z[N − k])
       // E = (a + b) / 2; O = (a − b) / (2i) = −i · (a − b) / 2
    let e = Complex {
        re: 0.5 * (a.re + b.re),
        im: 0.5 * (a.im + b.im),
    };
    let o = Complex {
        re: 0.5 * (a.im - b.im),
        im: -0.5 * (a.re - b.re),
    };
    e.add(split_twiddle(k).mul(o))
}

/// Normalised power of bins `first_bin .. first_bin + out.len()` of the
/// Hann-windowed spectrum of `samples`: a full-scale sine centred in a
/// bin reads 1.0. Panics if the bins run past 1 919.
#[cfg(test)]
pub(crate) fn power_spectrum(
    samples: &[f32; WINDOW],
    buf: &mut FftBuffer,
    first_bin: usize,
    out: &mut [f32],
) {
    let count = out.len();
    power_bins(samples, buf, first_bin, count, |i, p| out[i] = p);
}

/// Calls `f(i, power)` for bins `first_bin + i`, i in `0..count`, of the
/// Hann-windowed spectrum of `samples` (normalised as in
/// [`power_spectrum`]), without a buffer for the powers.
pub(crate) fn power_bins<F: FnMut(usize, f32)>(
    samples: &[f32; WINDOW],
    buf: &mut FftBuffer,
    first_bin: usize,
    count: usize,
    mut f: F,
) {
    assert!(first_bin + count <= BINS, "bins beyond the transform");
    spectrum(samples, buf);
    for i in 0..count {
        let x = real_bin(&buf.work, first_bin + i);
        f(i, (x.re * x.re + x.im * x.im) * POWER_SCALE);
    }
}

/// Bytes of the static tables this module uses (twiddles, real-split
/// twiddles, half window, digit-reversal permutation).
pub const TABLE_BYTES: usize = core::mem::size_of::<[[f32; 2]; N]>()
    + core::mem::size_of::<[[f32; 2]; 961]>()
    + core::mem::size_of::<[f32; WINDOW / 2 + 1]>()
    + core::mem::size_of::<[u16; N]>();

#[cfg(test)]
pub(crate) fn table_bytes_of_statics() -> usize {
    core::mem::size_of_val(&TWIDDLES)
        + core::mem::size_of_val(&SPLIT_TWIDDLES)
        + core::mem::size_of_val(&HANN_HALF)
        + core::mem::size_of_val(&DIGIT_REV)
}

#[cfg(test)]
mod tests;
