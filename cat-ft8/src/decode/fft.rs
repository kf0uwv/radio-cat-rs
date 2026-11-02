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
//! - **Complex transform.** Decimation in time, in place, on separate
//!   real and imaginary arrays. Six stages of radix 4, 4, 4, 2, 3, 5
//!   (innermost first) run butterflies whose twiddles come from one static
//!   table of e^(−2πit/1920). The first stage loads its inputs in
//!   digit-reversed order (a compile-time table, read once per group of
//!   blocks) and applies the window as it loads them; the later stages
//!   work in place, independent butterflies side by side ([`LANES`] at a
//!   time) so that vector units take them together. Each butterfly keeps
//!   its own operations in the same order, so the result does not depend
//!   on the group size, the lane count or the target.
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
static DIGIT_REV: [u16; N] = DIGIT_REV_TABLE;

/// [`DIGIT_REV`]'s contents, as a constant so that its layout can be
/// checked at compile time (a `static` cannot be read in a constant on the
/// minimum Rust).
const DIGIT_REV_TABLE: [u16; N] = {
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

/// Samples between a first-stage block's inputs m and m + 1 (elements
/// N/4 apart, two samples each): 960.
const INPUT_STEP: usize = 2 * (N / 4);
/// Samples from a first-stage block's first input to its last, inclusive:
/// 2 882.
const BLOCK_SAMPLES: usize = 3 * INPUT_STEP + 2;
/// Samples between the first inputs of blocks 4c + j and 4c + j + 1
/// (elements N/16 apart): 240.
const BLOCK_SPREAD: usize = 2 * (N / 16);
/// Blocks whose base elements are [`BLOCK_SPREAD`] / 2 apart: the second
/// stage's radix.
const SPREAD_BLOCKS: usize = STAGES[1];

const _: () = assert!(WINDOW == 4 * INPUT_STEP);

// The layout [`first_stage`] relies on. Block b (positions 4b..4b + 4)
// holds elements base(b) + (N/4)·m, m = 0..4, where base(b) =
// DIGIT_REV[4b] < N/4; and block 4c + j (j < 4) has base(4c + j) =
// base(4c) + (N/16)·j, so base(4c) + (N/16)·3 < N/4 too.
const _: () = {
    let mut b = 0;
    while b < N / 4 {
        let group = DIGIT_REV_TABLE[4 * SPREAD_BLOCKS * (b / SPREAD_BLOCKS)] as usize;
        let base = DIGIT_REV_TABLE[4 * b] as usize;
        assert!(base < N / 4);
        assert!(group + (N / 16) * (SPREAD_BLOCKS - 1) < N / 4);
        assert!(base == group + (N / 16) * (b % SPREAD_BLOCKS));
        let mut m = 0;
        while m < 4 {
            assert!(DIGIT_REV_TABLE[4 * b + m] as usize == base + (N / 4) * m);
            m += 1;
        }
        b += 1;
    }
};

/// Group sizes [`first_stage`] takes: `Group::<G>::OK` fails to compile
/// unless `G` divides [`SPREAD_BLOCKS`], so that every block of a group
/// is placed from the group's first table entry.
struct Group<const G: usize>;

impl<const G: usize> Group<G> {
    const OK: () = assert!(G > 0 && SPREAD_BLOCKS % G == 0, "group size must divide 4");
}

/// Blocks per group in [`first_stage`]: four where the target has vectors,
/// one on a Cortex-M, where four measured slower and larger.
const GROUP: usize = LANES;

/// A complex value (the reference transform in the tests).
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Complex {
    pub(crate) re: f32,
    pub(crate) im: f32,
}

#[cfg(test)]
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

/// Working storage for one transform: 1 920 complex values as separate
/// real and imaginary parts (15 360 bytes), caller-owned so it can live
/// in a `static`.
#[derive(Debug, Clone)]
pub(crate) struct FftBuffer {
    re: [f32; N],
    im: [f32; N],
}

impl FftBuffer {
    /// A zeroed buffer (usable in a `static` initialiser).
    pub(crate) const fn new() -> Self {
        FftBuffer {
            re: [0.0; N],
            im: [0.0; N],
        }
    }
}

impl Default for FftBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// The Hann window value for sample `n` of `0..WINDOW`: sin²(πn / 3 840).
#[cfg(test)]
fn hann(n: usize) -> f32 {
    HANN_HALF[n.min(WINDOW - n)]
}

/// Lane counts the transform takes: `Lanes::<L>::OK` fails to compile
/// unless every stage after the first has a multiple of `L` butterflies per
/// block (`l`, at least 4, so `L` is 1, 2 or 4) and the last stage's
/// twiddle lanes cannot wrap (see [`stage`]): N a multiple of `L`·m for
/// every m below the last radix. Both bounds come from [`STAGES`].
struct Lanes<const L: usize>;

impl<const L: usize> Lanes<L> {
    const OK: () = {
        let mut l = STAGES[0];
        let mut i = 1;
        while i < STAGES.len() {
            assert!(l % L == 0, "lane count must divide every stage's l");
            l *= STAGES[i];
            i += 1;
        }
        let mut m = 1;
        while m < STAGES[STAGES.len() - 1] {
            assert!(N % (L * m) == 0, "last stage's twiddle lanes would wrap");
            m += 1;
        }
    };
}

/// Butterflies computed side by side: four where the target has 128-bit
/// vectors of `f32`, else one (a Cortex-M then runs the plain loops). The
/// result is the same for any value: lanes are independent butterflies.
const LANES: usize = if cfg!(any(target_feature = "sse2", target_feature = "neon")) {
    4
} else {
    1
};

const _: () = Lanes::<LANES>::OK;

/// In-place 1 920-point complex DFT of (`re`, `im`), whose input is
/// already in digit-reversed order. Output in natural order. `L`
/// butterflies at a time. (The decoder runs [`first_stage`] from the
/// samples instead of the first stage here; see [`spectrum`].)
#[cfg(test)]
fn fft<const L: usize>(re: &mut [f32; N], im: &mut [f32; N]) {
    // One call per entry of `STAGES`, with its `l` (checked at compile
    // time below). The first stage (`l` = 1) has one butterfly per block:
    // one lane.
    stage::<4, 1, 1>(re, im);
    later_stages::<L>(re, im);
}

/// The stages after the first, in place, `L` butterflies at a time.
#[inline(always)]
fn later_stages<const L: usize>(re: &mut [f32; N], im: &mut [f32; N]) {
    let () = Lanes::<L>::OK;
    stage::<4, L, 4>(re, im);
    stage::<4, L, 16>(re, im);
    stage::<2, L, 64>(re, im);
    stage::<3, L, 128>(re, im);
    stage::<5, L, 384>(re, im);
}

const _: () = {
    let calls = [(4, 1), (4, 4), (4, 16), (2, 64), (3, 128), (5, 384)];
    let mut l = 1;
    let mut i = 0;
    while i < STAGES.len() {
        assert!(STAGES[i] == calls[i].0 && l == calls[i].1);
        l *= STAGES[i];
        i += 1;
    }
};

/// One radix-`R` stage over sub-transforms of length `S` (a multiple of
/// `L`; `l` below): for each `k < l` and each block of `l · R` values,
/// out[q] = Σ_m a[m] · W_span^(m · (k + q·l)), summed in the order of `m`.
/// The `m = 0` twiddle is exactly 1 (`TWIDDLES[0]` = [1.0, −0.0]), so that
/// product is the value itself and is not computed. (Its sign of zero may
/// differ from the product's, which no later `+ − ×` or the power can
/// tell.) The `L` butterflies of adjacent `k` run side by side.
///
/// The twiddles depend only on `k`. With several blocks they are read once
/// per group of `k` and reused. The last stage has one block, so each is
/// read where it is used: lane j of (m, q) is `TWIDDLES[f + j·m]`, where
/// f = m·(k + q·l) mod N is a multiple of `L`·m (k and l are multiples of
/// `L`, and N of `L`·m), so f + (`L` − 1)·m < N and the lanes do not wrap.
///
/// **Precondition: finite input.** Skipping the product equals computing it
/// only for finite values: with an infinite component the product's
/// `inf · −0` is NaN where the value itself stays infinite. The input is
/// finite and bounded by construction (the front end's `classify` turns a
/// non-finite sample into 0 and clamps the rest to ±[`INPUT_LIMIT`]; the
/// window is at most 1), and no intermediate can overflow: every output is
/// at most `N` × 2¹⁶ × √2 ≈ 1.8·10⁸ in magnitude.
///
/// [`INPUT_LIMIT`]: super::frontend::INPUT_LIMIT
#[inline(always)]
fn stage<const R: usize, const L: usize, const S: usize>(re: &mut [f32; N], im: &mut [f32; N]) {
    let l = S;
    let span = l * R;
    let step = N / span; // W_span^x = W_N^(x · step)
    let mut k0 = 0;
    if span == N {
        while k0 < l {
            butterflies::<R, L, S>(re, im, k0, |m, q| {
                let f = (m * (k0 + q * l)) % N;
                let t = &TWIDDLES[f..f + (L - 1) * m + 1];
                (
                    core::array::from_fn(|j| t[j * m][0]),
                    core::array::from_fn(|j| t[j * m][1]),
                )
            });
            k0 += L;
        }
    } else {
        while k0 < l {
            // Row m = 0 is filled too, though unused: with every entry
            // written the array needs no zeroing first.
            let mut w = [[([0.0f32; L], [0.0f32; L]); R]; R];
            for (m, wm) in w.iter_mut().enumerate() {
                for (q, wmq) in wm.iter_mut().enumerate() {
                    for j in 0..L {
                        let t = TWIDDLES[(m * (k0 + j + q * l) * step) % N];
                        wmq.0[j] = t[0];
                        wmq.1[j] = t[1];
                    }
                }
            }
            let mut base = k0;
            while base < N {
                butterflies::<R, L, S>(re, im, base, |m, q| w[m][q]);
                base += span;
            }
            k0 += L;
        }
    }
}

/// The sums of `L` radix-`R` butterflies side by side, from their inputs
/// `ar`, `ai` (`[[f32; L]; R]`: m = 0..R, lane j): output q is
/// a[0] + Σ_{m ≥ 1} a[m] · w(m, q)[j], the products added in the order of
/// m, each lane on its own. The m = 0 product is not computed (see
/// [`stage`]). The twiddle expression gives (re, im) for (m, q), each
/// indexed by lane (an array); the store
/// block takes output q as (`sr`, `si`). A macro rather than a function,
/// so that each caller compiles as its own loops did (a function, even
/// inlined, moved the transform's code on every target; rustc 1.93.0).
macro_rules! radix_sums {
    ($r:expr, $l:expr, $ar:ident, $ai:ident, ($m:ident, $q:ident) => $w:expr,
     ($sr:ident, $si:ident) => $out:block) => {
        for $q in 0..$r {
            let mut $sr = $ar[0];
            let mut $si = $ai[0];
            for $m in 1..$r {
                let (wr, wi) = $w;
                for j in 0..$l {
                    let pr = $ar[$m][j] * wr[j] - $ai[$m][j] * wi[j];
                    let pi = $ar[$m][j] * wi[j] + $ai[$m][j] * wr[j];
                    $sr[j] += pr;
                    $si[j] += pi;
                }
            }
            $out
        }
    };
}

/// `L` radix-`R` butterflies side by side, at `k` = `base` + j of a block
/// (inputs at `base + m · S`), with the twiddles of (m, q) from `w`, called
/// for m = 1.. within q = 0.. in order.
#[inline(always)]
fn butterflies<const R: usize, const L: usize, const S: usize>(
    re: &mut [f32; N],
    im: &mut [f32; N],
    base: usize,
    mut w: impl FnMut(usize, usize) -> ([f32; L], [f32; L]),
) {
    let l = S;
    let len = (R - 1) * l + L;
    let (r, i) = (&mut re[base..base + len], &mut im[base..base + len]);
    let mut ar = [[0.0f32; L]; R];
    let mut ai = [[0.0f32; L]; R];
    for m in 0..R {
        for j in 0..L {
            ar[m][j] = r[m * l + j];
            ai[m][j] = i[m * l + j];
        }
    }
    radix_sums!(R, L, ar, ai, (m, q) => w(m, q), (sr, si) => {
        for j in 0..L {
            r[q * l + j] = sr[j];
            i[q * l + j] = si[j];
        }
    });
}

/// The transform as first written (every product, `% N` per twiddle): the
/// reference [`fft`] is checked against.
#[cfg(test)]
pub(crate) fn fft_reference(buf: &mut [Complex; N]) {
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

/// The windowed spectrum of `samples`: the complex transform of the
/// packed samples (Z[k], k = 0..1 920) in `buf`.
fn spectrum(samples: &[f32; WINDOW], buf: &mut FftBuffer) {
    first_stage::<GROUP>(samples, &mut buf.re, &mut buf.im);
    later_stages::<LANES>(&mut buf.re, &mut buf.im);
}

/// The first stage (radix 4, `l` = 1) from the samples: position p's input
/// is element n = `DIGIT_REV[p]` windowed, `samples[2n] · hann(2n)` and
/// `samples[2n + 1] · hann(2n + 1)`, computed as it is loaded rather than
/// stored first. Every value and product is the one that storing the
/// windowed input and running `stage::<4, 1, 1>` computes, in the same
/// order. The result is claimed, and tested, to be the same bit for bit
/// for finite input within ±[`INPUT_LIMIT`], which is what the front end
/// passes (like `stage`, it skips the m = 0 product; see there).
///
/// Block b takes elements base(b) + (N/4)·m, m = 0..4, with base(b) < N/4
/// (checked at compile time), so the window's mirror point is known for
/// each m: `hann` reads `HANN_HALF[i]` for sample i = 2n + e (e = 0, 1)
/// when m is 0 or 1, and `HANN_HALF[WINDOW − i]` when m is 2 or 3. Those
/// entries are read directly.
///
/// The blocks are taken `G` at a time. Within a group of up to four the
/// base elements are N/16 apart, so one `DIGIT_REV` read places the whole
/// group. That, and the loop unrolled over the group, is the gain over
/// one block at a time. The group is not a set of vector lanes: the
/// compiler vectorises each block's butterfly over q, not across the
/// blocks (rustc 1.93.0).
///
/// [`INPUT_LIMIT`]: super::frontend::INPUT_LIMIT
#[inline(always)]
fn first_stage<const G: usize>(samples: &[f32; WINDOW], re: &mut [f32; N], im: &mut [f32; N]) {
    let () = Group::<G>::OK;
    // W₄^(m·q) as `stage::<4, 1, 1>` reads it: TWIDDLES[m·q·N/4 mod N].
    let mut w = [[[0.0f32; 2]; 4]; 4];
    for (m, wm) in w.iter_mut().enumerate() {
        for (q, wmq) in wm.iter_mut().enumerate() {
            *wmq = TWIDDLES[(m * q * (N / 4)) % N];
        }
    }
    for (c, (r, i)) in re
        .chunks_exact_mut(4 * G)
        .zip(im.chunks_exact_mut(4 * G))
        .enumerate()
    {
        // The first sample of the group's first block, 2·base(Gc); with
        // d, the spread of the group's first samples, first + d < INPUT_STEP.
        let d = BLOCK_SPREAD * (G - 1);
        let first = 2 * usize::from(DIGIT_REV[4 * G * c]);
        // low[t] = HANN_HALF[first + t]; high[t] =
        // HANN_HALF[INPUT_STEP − 1 − d − first + t].
        let x = &samples[first..][..BLOCK_SAMPLES + d];
        let low = &HANN_HALF[first..][..INPUT_STEP + 2 + d];
        let high = &HANN_HALF[INPUT_STEP - 1 - d - first..][..INPUT_STEP + 2 + d];
        let mut ar = [[0.0f32; G]; 4];
        let mut ai = [[0.0f32; G]; 4];
        for j in 0..G {
            // Block j's first sample is first + o; its mirrored window
            // entries WINDOW − (first + o + 2·INPUT_STEP + e) and
            // WINDOW − (first + o + 3·INPUT_STEP + e) are
            // high[g + INPUT_STEP + 1 − e] and high[g + 1 − e].
            let o = BLOCK_SPREAD * j;
            let g = d - o;
            ar[0][j] = x[o] * low[o];
            ai[0][j] = x[o + 1] * low[o + 1];
            ar[1][j] = x[o + INPUT_STEP] * low[o + INPUT_STEP];
            ai[1][j] = x[o + INPUT_STEP + 1] * low[o + INPUT_STEP + 1];
            ar[2][j] = x[o + 2 * INPUT_STEP] * high[g + INPUT_STEP + 1];
            ai[2][j] = x[o + 2 * INPUT_STEP + 1] * high[g + INPUT_STEP];
            ar[3][j] = x[o + 3 * INPUT_STEP] * high[g + 1];
            ai[3][j] = x[o + 3 * INPUT_STEP + 1] * high[g];
        }
        // Each block's butterfly as `stage::<4, 1, 1>` computes it.
        radix_sums!(4, G, ar, ai, (m, q) => ([w[m][q][0]; G], [w[m][q][1]; G]), (sr, si) => {
            for j in 0..G {
                r[4 * j + q] = sr[j];
                i[4 * j + q] = si[j];
            }
        });
    }
}

/// Normalised power of real-transform bin k from a = Z[k], c = Z[N − k]
/// and the real-split twiddle w = W₃₈₄₀ᵏ: |E + w·O|² / 960², with
/// E = (a + b) / 2, O = (a − b) / (2i) = −i · (a − b) / 2, b = conj(c).
#[inline(always)]
fn bin_power(ar: f32, ai: f32, cr: f32, ci: f32, wr: f32, wi: f32) -> f32 {
    let (br, bi) = (cr, -ci);
    let (er, ei) = (0.5 * (ar + br), 0.5 * (ai + bi));
    let (or, oi) = (0.5 * (ai - bi), -0.5 * (ar - br));
    let xr = er + (wr * or - wi * oi);
    let xi = ei + (wr * oi + wi * or);
    (xr * xr + xi * xi) * POWER_SCALE
}

/// Powers of bins `k .. k + out.len()` (1 ≤ k, k + out.len() ≤ N) into
/// `out`, the real-split twiddle of bin k + j being `tw[j]`, or, if
/// `ROTATE`, −i · `tw[j]` (exact: W₃₈₄₀ᵏ = −i · W₃₈₄₀^(k−960)). A plain
/// loop over bins, so it vectorises.
#[inline(always)]
fn powers<const ROTATE: bool>(
    re: &[f32; N],
    im: &[f32; N],
    k: usize,
    out: &mut [f32],
    tw: &[[f32; 2]],
) {
    let n = out.len();
    let (ar, ai) = (&re[k..k + n], &im[k..k + n]);
    let tw = &tw[..n];
    // Z[N − k − j] for j = 0..n, in reverse order.
    let (cr, ci) = (&re[N + 1 - k - n..N + 1 - k], &im[N + 1 - k - n..N + 1 - k]);
    let c = cr.iter().rev().zip(ci.iter().rev());
    for ((((slot, &ar), &ai), (&cr, &ci)), w) in out.iter_mut().zip(ar).zip(ai).zip(c).zip(tw) {
        let (wr, wi) = if ROTATE { (w[1], -w[0]) } else { (w[0], w[1]) };
        *slot = bin_power(ar, ai, cr, ci, wr, wi);
    }
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
    power_bins(samples, buf, first_bin, count, |i, p| {
        out[i..i + p.len()].copy_from_slice(p)
    });
}

/// Powers handed to the caller at a time by [`power_bins`].
pub(crate) const POWER_CHUNK: usize = 64;

/// Calls `f(i, powers)` with the powers of bins `first_bin + i ..`, in
/// order and in chunks of at most [`POWER_CHUNK`], for bins `first_bin ..
/// first_bin + count` of the Hann-windowed spectrum of `samples`
/// (normalised as in [`power_spectrum`]). Chunks let the caller's
/// per-bin work run as a plain loop over values (which vectorises).
///
/// `samples` must be finite and within ±[`INPUT_LIMIT`], as the front end's
/// `classify` leaves them: the transform's stages rely on it to be bit for
/// bit the full set of products (see `stage`).
///
/// [`INPUT_LIMIT`]: super::frontend::INPUT_LIMIT
pub(crate) fn power_bins<F: FnMut(usize, &[f32])>(
    samples: &[f32; WINDOW],
    buf: &mut FftBuffer,
    first_bin: usize,
    count: usize,
    mut f: F,
) {
    assert!(first_bin + count <= BINS, "bins beyond the transform");
    spectrum(samples, buf);
    let mut p = [0.0f32; POWER_CHUNK];
    let mut i = 0;
    while i < count {
        let n = (count - i).min(POWER_CHUNK);
        bin_powers(&buf.re, &buf.im, first_bin + i, &mut p[..n]);
        f(i, &p[..n]);
        i += n;
    }
}

/// Powers of real-transform bins `k0 .. k0 + out.len()` into `out`: bin 0
/// (which pairs with itself), the bins to 960 and those above as separate
/// plain loops, which vectorise where the target has vectors.
fn bin_powers(re: &[f32; N], im: &[f32; N], k0: usize, out: &mut [f32]) {
    let n = out.len();
    let zero = usize::from(k0 == 0);
    if zero == 1 {
        let w = SPLIT_TWIDDLES[0];
        out[0] = bin_power(re[0], im[0], re[0], im[0], w[0], w[1]);
    }
    let k1 = k0 + zero;
    let low = (k0 + n).min(961).saturating_sub(k1);
    if low > 0 {
        powers::<false>(
            re,
            im,
            k1,
            &mut out[zero..zero + low],
            &SPLIT_TWIDDLES[k1..],
        );
    }
    let k2 = k1 + low;
    if zero + low < n {
        powers::<true>(
            re,
            im,
            k2,
            &mut out[zero + low..],
            &SPLIT_TWIDDLES[k2 - 960..],
        );
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
