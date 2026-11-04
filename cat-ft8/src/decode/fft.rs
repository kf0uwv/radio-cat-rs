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
//!   work in place. With vectors ([`LANES`] above one), independent
//!   butterflies run side by side so that vector units take them together
//!   (the last stage, whose twiddles are strided, as a plain loop the
//!   compiler vectorises, over a table that runs past one turn); with one
//!   lane ([`ONE_LANE`]), each butterfly is written out term by term and
//!   the first stage reads the window as its four quarters. Each
//!   butterfly keeps its own operations in the same order, so the result
//!   does not depend on the group size, the lane count or the target.
//! - **Real split.** The 3 840 real samples are packed as 1 920 complex
//!   values `z[n] = x[2n] + i·x[2n+1]`; bin k of the real transform is
//!   `X[k] = E[k] + W₃₈₄₀ᵏ·O[k]`, with E and O from `Z[k]` and `Z[1920 − k]`.
//! - **Scale.** Power is |X|² / 960², so a full-scale sine centred in a
//!   bin reads 1.0 (0 dB): the Hann window's coherent gain is 1 920 / 2.
//!
//! The twiddle, real-split and window tables are static data generated
//! with `libm` and checked bit-for-bit by a test (`tables.rs`; the twiddles
//! as one turn, which [`TWIDDLES`] repeats, and which with one lane
//! [`FIFTH_TWIDDLES`] and [`LAST_ROWS`] take in other orders).

use super::math::SCALAR_FORMS;
use super::tables::{HANN_HALF, SPLIT_TWIDDLES, TWIDDLE_TURN};

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
        // Redundant (base < N/4 and the equality below give it at
        // b = 4c + 3); stated for the reader.
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
    const OK: () = assert!(
        G > 0 && SPREAD_BLOCKS % G == 0,
        "group size must divide SPREAD_BLOCKS"
    );
}

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
///
/// A transform writes every cell before it reads one, and nothing reads
/// them after [`power_bins`] returns, so between transforms the cells are
/// free for other working storage ([`FftBuffer::cells`]).
#[derive(Debug, Clone)]
pub(crate) struct FftBuffer {
    cells: [f32; FftBuffer::CELLS],
}

impl FftBuffer {
    /// The cells: both parts.
    pub(crate) const CELLS: usize = 2 * N;

    /// A zeroed buffer (usable in a `static` initialiser).
    pub(crate) const fn new() -> Self {
        FftBuffer {
            cells: [0.0; Self::CELLS],
        }
    }

    /// The real and imaginary parts.
    #[inline(always)]
    fn parts(&mut self) -> (&mut [f32; N], &mut [f32; N]) {
        let (re, im) = self.cells.split_at_mut(N);
        match (re.try_into(), im.try_into()) {
            (Ok(re), Ok(im)) => (re, im),
            _ => unreachable!(),
        }
    }

    /// Every cell, for use between transforms. A transform overwrites them.
    pub(crate) fn cells(&mut self) -> &mut [f32] {
        &mut self.cells
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
/// block (`l`, at least 4, so `L` is 1, 2 or 4). (The last stage runs one
/// butterfly per k whatever `L` is: [`last_stage`].)
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
    };
}

/// Butterflies computed side by side: four where the target has 128-bit
/// vectors of `f32` (SSE2 or NEON: x86-64, AArch64 and 32-bit x86, and
/// 32-bit ARM with NEON where `cfg` shows the feature, which stable Rust
/// 1.93 does not), else one (a Cortex-M then runs `straight_stages`; the
/// Cortex-M33 measured slower and larger with a first-stage group of
/// four). One also in the tests built with `--cfg cat_ft8_test_group_one`,
/// so that a host runs the one-lane forms end to end. The result is the
/// same for any value: lanes are independent butterflies.
const LANES: usize = if cfg!(any(target_feature = "sse2", target_feature = "neon"))
    && !cfg!(all(test, cat_ft8_test_group_one))
{
    4
} else {
    1
};

/// Whether the transform takes one lane: the one source of every choice
/// that goes with it. With one lane the stages are written out term by
/// term ([`straight_stages`]) and read their twiddles from
/// [`FIFTH_TWIDDLES`] and [`LAST_ROWS`] ([`TABLE_BYTES`]), and the front
/// end holds its window as a ring of quarters, which the first stage reads
/// in place ([`Quarters`]). Not [`SCALAR_FORMS`], which is also true on
/// 32-bit x86 (and on 32-bit ARM with NEON): there the transform takes
/// vectors and the window slides (findings O31b).
pub(crate) const ONE_LANE: bool = LANES == 1;

const _: () = Lanes::<LANES>::OK;

/// In-place 1 920-point complex DFT of (`re`, `im`), whose input is
/// already in digit-reversed order. Output in natural order. `L`
/// butterflies at a time. (The decoder takes the first stage from the
/// samples instead of running it here; see [`spectrum_lanes`].)
#[cfg(test)]
fn fft<const L: usize>(re: &mut [f32; N], im: &mut [f32; N]) {
    // One call per entry of `STAGES`, with its `l` (checked at compile
    // time below). The first stage (`l` = 1) has one butterfly per block:
    // one lane.
    stage::<4, 1, 1>(re, im);
    later_stages::<L>(re, im);
}

/// The stages after the first, in place, `L` butterflies at a time. With
/// one lane (as on a Cortex-M) each butterfly is written out term by term
/// ([`straight_stages`]); the operations, their values and their order are
/// those of [`stage`]. (The decoder chooses in [`spectrum_lanes`].)
#[cfg(test)]
fn later_stages<const L: usize>(re: &mut [f32; N], im: &mut [f32; N]) {
    if L == 1 {
        straight_stages(re, im);
        return;
    }
    side_by_side_stages::<L>(re, im);
}

/// The stages after the first through [`stage`], `L` butterflies at a
/// time, and the last through [`last_stage`].
#[inline(always)]
fn side_by_side_stages<const L: usize>(re: &mut [f32; N], im: &mut [f32; N]) {
    let () = Lanes::<L>::OK;
    stage::<4, L, 4>(re, im);
    stage::<4, L, 16>(re, im);
    stage::<2, L, 64>(re, im);
    stage::<3, L, 128>(re, im);
    last_stage(re, im);
}

const _: () = {
    // The last is `last_stage`'s (`LAST_R`, `LAST_L`).
    let calls = [(4, 1), (4, 4), (4, 16), (2, 64), (3, 128), (LAST_R, LAST_L)];
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
/// per group of `k` and reused. (The last stage, which has one block, runs
/// as [`last_stage`] where vectors are taken.)
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

/// The sums of `L` radix-`R` butterflies side by side, from their inputs
/// `ar`, `ai` (`[[f32; L]; R]`: m = 0..R, lane j): output q is
/// a[0] + Σ_{m ≥ 1} a[m] · w(m, q)[j], the products added in the order of
/// m, each lane on its own. The m = 0 product is not computed (see
/// [`stage`]). The twiddle expression gives (re, im) for (m, q), each
/// indexed by lane (an array); the store
/// block takes output q as (`sr`, `si`). A macro rather than a function,
/// so that each caller compiles as its own loops did: a function, even
/// inlined, produced different code for the transform on every target
/// (rustc 1.93.0). That code was not timed. Re-check after a toolchain
/// change.
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

/// The last stage's radix and sub-transform length (one block of `N`).
const LAST_R: usize = STAGES[STAGES.len() - 1];
const LAST_L: usize = N / LAST_R;

/// One past the last stage's largest twiddle index ([`last_stage`]),
/// m·k + `LAST_L`·(m·q mod `LAST_R`) at m = `LAST_R` − 1, k = `LAST_L` − 1
/// and m·q mod `LAST_R` = `LAST_R` − 1: 3 069.
const LAST_TWIDDLES: usize = (LAST_R - 1) * (LAST_L - 1) + LAST_L * (LAST_R - 1) + 1;

/// The twiddles W_N^t = e^(−2πit/N) as [cos, −sin], t = 0..[`LAST_TWIDDLES`]:
/// `TWIDDLE_TURN` (one turn, generated) and past N its entries again, so
/// that [`last_stage`] reads its index without reducing it mod N. Every
/// other reader takes its index mod N. The transform with vectors reads
/// it; with one lane (a Cortex-M) the transform reads
/// [`FIFTH_TWIDDLES`] and [`LAST_ROWS`] instead, so a build with one lane
/// links neither this table nor the vector form.
static TWIDDLES: [[f32; 2]; LAST_TWIDDLES] = {
    let mut table = [[0.0f32; 2]; LAST_TWIDDLES];
    let mut t = 0;
    while t < LAST_TWIDDLES {
        table[t] = TWIDDLE_TURN[t % N];
        t += 1;
    }
    table
};

/// Twiddles a step of [`FIFTH_TWIDDLES`]: every stage but the last reads
/// W_N^t only at t a multiple of this, as each of its indices is one of
/// its N / span and of its N / R, multiples of 5 (checked at compile time
/// in [`straight_stage`], and by a test).
const FIFTH: usize = 5;

/// W_N^(5i) = `TWIDDLE_TURN[5i]`, i = 0..N/5: the twiddles the stages
/// before the last read with one lane ([`straight_stage`],
/// [`straight_first_stage`]), at [`fifth_index`]. A fifth of one turn
/// (3 072 bytes).
static FIFTH_TWIDDLES: [[f32; 2]; N / FIFTH] = {
    let mut table = [[0.0f32; 2]; N / FIFTH];
    let mut i = 0;
    while i < N / FIFTH {
        table[i] = TWIDDLE_TURN[FIFTH * i];
        i += 1;
    }
    table
};

/// One row of [`LAST_ROWS`]: the last stage's twiddles W_N^(r + `LAST_L`·j),
/// j = 0..`LAST_R`.
type LastRow = [[f32; 2]; LAST_R];

/// The last stage's twiddles by row, for [`last_straight_stage`]: row r
/// holds W_N^(r + `LAST_L`·j) = `TWIDDLE_TURN[r + LAST_L·j]`, j =
/// 0..`LAST_R`. One turn in another order (15 360 bytes). The twiddle the
/// butterfly at k reads for (m, q), W_N^((m·k + `LAST_L`·(m·q mod `LAST_R`))
/// mod N), is with m·k = `LAST_L`·s + r (r < `LAST_L`) entry
/// (s + m·q mod `LAST_R`) mod `LAST_R` of row r: so a butterfly reads five
/// entries of one row for each m, all within 40 bytes.
static LAST_ROWS: [LastRow; LAST_L] = {
    let mut table = [[[0.0f32; 2]; LAST_R]; LAST_L];
    let mut r = 0;
    while r < LAST_L {
        let mut j = 0;
        while j < LAST_R {
            table[r][j] = TWIDDLE_TURN[r + LAST_L * j];
            j += 1;
        }
        r += 1;
    }
    table
};

/// The last stage (radix 5, `l` = 384, one block) where vectors are taken:
/// one butterfly per k in a plain loop, each with its own operations in
/// [`stage`]'s order ([`radix_sums`] with one lane). The twiddle of (m, q)
/// at k, W_N^(m·(k + q·`LAST_L`)), is read at m·k + `LAST_L`·(m·q mod
/// `LAST_R`), below [`LAST_TWIDDLES`] and not reduced mod N: [`TWIDDLES`]
/// runs that far. Each index is m·k plus a constant, so the compiler
/// vectorises the loop across adjacent k and loads each m's twiddles, m
/// apart in the table, as interleaved vectors (`ld2` and `ld4` on AArch64)
/// rather than lane by lane (rustc 1.93.0): findings O26 and O26b. The
/// result is that of `stage` with any lane count, and of
/// [`last_straight_stage`], bit for bit.
#[inline(always)]
fn last_stage(re: &mut [f32; N], im: &mut [f32; N]) {
    for k in 0..LAST_L {
        let ar: [[f32; 1]; LAST_R] = core::array::from_fn(|m| [re[k + m * LAST_L]]);
        let ai: [[f32; 1]; LAST_R] = core::array::from_fn(|m| [im[k + m * LAST_L]]);
        radix_sums!(LAST_R, 1, ar, ai, (m, q) => {
            let t = m * k + LAST_L * ((m * q) % LAST_R);
            let [wr, wi] = TWIDDLES[t];
            ([wr], [wi])
        }, (sr, si) => {
            re[k + q * LAST_L] = sr[0];
            im[k + q * LAST_L] = si[0];
        });
    }
}

/// Index into `TWIDDLES` of W_span^(m·(k + q·l)) for a radix-`R` stage over
/// sub-transforms of length l (span = l·`R`, `k` < l, m < `R`, q < `R`):
/// the index `stage` reads, m·(k + q·l)·(N/span) mod N, as l·(N/span) is
/// N/`R`. `k_step` is k·(N/span). The tests check [`fifth_index`] and
/// the entries of [`LAST_ROWS`] against it.
#[cfg(test)]
fn twiddle_index<const R: usize>(m: usize, q: usize, k_step: usize) -> usize {
    (m * k_step + (N / R) * (m * q)) % N
}

/// [`twiddle_index`] / [`FIFTH`]: the index into [`FIFTH_TWIDDLES`] of
/// W_span^(m·(k + q·l)) for a radix-`R` stage, `k_fifths` being
/// k·(N/span)/5 (N/span is a multiple of 5 for every stage but the last,
/// checked in [`straight_stage`]).
#[inline(always)]
fn fifth_index<const R: usize>(m: usize, q: usize, k_fifths: usize) -> usize {
    (m * k_fifths + (N / R / FIFTH) * (m * q)) % (N / FIFTH)
}

/// One radix-R butterfly written out term by term: inputs m = 0 .. R at
/// `$re[m·$stride]`, `$im[m·$stride]` (`$ms` lists m = 1 .. R, `$qs`
/// q = 0 .. R), output q stored at `$re[q·$stride]`, `$im[q·$stride]`; the
/// twiddle expression gives [re, im] for
/// (m, q). Every input is read before any output is stored. Output q is
/// a[0] + Σ_{m ≥ 1} a[m]·w(m, q), the products added in the order of m:
/// the operations of [`radix_sums`] with one lane, in its order, so the
/// result is the same bit for bit. Written out because opt-level "s" does
/// not unroll the loops (rustc 1.93.0): this way the inputs stay in registers
/// and no loop counter or index is kept per term.
macro_rules! straight_butterfly {
    ($re:ident, $im:ident, $stride:expr, $ms:tt, $qs:tt, ($m_name:ident, $q_name:ident) => $w:expr) => {{
        let ar = straight_butterfly!(@load $re, $stride, $ms);
        let ai = straight_butterfly!(@load $im, $stride, $ms);
        straight_butterfly!(@store ar, ai, $re, $im, $stride, $ms, $qs, ($m_name, $q_name) => $w);
    }};
    // Outputs q in `$qs` of inputs `$ar`, `$ai`, stored at `$re[q·stride]`,
    // `$im[q·stride]`.
    (@store $ar:ident, $ai:ident, $re:ident, $im:ident, $stride:expr, $ms:tt, [$($q:tt)*],
     ($m_name:ident, $q_name:ident) => $w:expr) => {
        $(
            let (sr, si) = straight_butterfly!(@sum $ar, $ai, $q, $ms, ($m_name, $q_name) => $w);
            $re[$q * $stride] = sr;
            $im[$q * $stride] = si;
        )*
    };
    (@load $x:ident, $stride:expr, [$($m:tt)*]) => {
        [$x[0] $(, $x[$m * $stride])*]
    };
    (@sum $ar:ident, $ai:ident, $q:tt, [$($m:tt)*], ($m_name:ident, $q_name:ident) => $w:expr) => {{
        let mut sr = $ar[0];
        let mut si = $ai[0];
        $(
            let [wr, wi] = {
                let ($m_name, $q_name): (usize, usize) = ($m, $q);
                $w
            };
            let pr = $ar[$m] * wr - $ai[$m] * wi;
            let pi = $ar[$m] * wi + $ai[$m] * wr;
            sr += pr;
            si += pi;
        )*
        (sr, si)
    }};
}

/// The twiddles of (m, q) for m in `$ms`, q in `$qs` (one row per m), from
/// [`FIFTH_TWIDDLES`], for the k whose k·(N/span)/5 is `$k_fifths`.
macro_rules! straight_twiddles {
    ($radix:literal, $k_fifths:expr, [$($m:tt)*], $qs:tt) => {
        [$(straight_twiddles!(@row $radix, $k_fifths, $m, $qs)),*]
    };
    (@row $radix:literal, $k_fifths:expr, $m:tt, [$($q:tt)*]) => {
        [$(FIFTH_TWIDDLES[fifth_index::<$radix>($m, $q, $k_fifths)]),*]
    };
}

/// One radix-`$radix` stage over sub-transforms of length `$l` (not the last
/// stage, which has one block: see [`last_straight_stage`]), one butterfly
/// at a time, as [`stage`] computes it with one lane: for each k, the
/// twiddles of every (m ≥ 1, q) are read once from [`FIFTH_TWIDDLES`] into
/// an array, then the butterfly at k runs in every block.
macro_rules! straight_stage {
    ($re:ident, $im:ident, $radix:literal, $l:literal, $ms:tt, $qs:tt) => {{
        const SPAN: usize = $radix * $l;
        const STEP: usize = N / SPAN;
        // Every twiddle index a multiple of `FIFTH`: k·STEP and N/R.
        const _: () = assert!(STEP % FIFTH == 0 && (N / $radix) % FIFTH == 0);
        for k in 0..$l {
            // Row m − 1 holds the twiddles of m.
            let w: [[[f32; 2]; $radix]; $radix - 1] =
                straight_twiddles!($radix, k * (STEP / FIFTH), $ms, $qs);
            let mut base = k;
            while base < N {
                let (r, i) = (
                    &mut $re[base..base + ($radix - 1) * $l + 1],
                    &mut $im[base..base + ($radix - 1) * $l + 1],
                );
                straight_butterfly!(r, i, $l, $ms, $qs, (m, q) => w[m - 1][q]);
                base += SPAN;
            }
        }
    }};
}

/// [`stage`]'s operations with one lane, each butterfly written out term
/// by term (see [`straight_butterfly`]).
#[inline(always)]
fn straight_stages(re: &mut [f32; N], im: &mut [f32; N]) {
    straight_stage!(re, im, 4, 4, [1 2 3], [0 1 2 3]);
    straight_stage!(re, im, 4, 16, [1 2 3], [0 1 2 3]);
    straight_stage!(re, im, 2, 64, [1], [0 1]);
    straight_stage!(re, im, 3, 128, [1 2], [0 1 2]);
    last_straight_stage(re, im);
}

/// The last stage's ranges of k, [lo, hi), for [`last_straight_stage`]:
/// in each, s(m) = ⌊m·k / `LAST_L`⌋ is the same for every m ≥ 1, so the
/// entry of [`LAST_ROWS`] each (m, q) reads is a constant there. Their
/// ends are the k where some s(m) steps: `LAST_L`·s / m for m = 2, 3, 4.
const LAST_RANGES: [(usize, usize); 6] = [
    (0, 96),
    (96, 128),
    (128, 192),
    (192, 256),
    (256, 288),
    (288, 384),
];

// The ranges cover 0..LAST_L in order; s(m) is constant in each; and the
// rows m reads, m·lo − LAST_L·s .. m·hi − LAST_L·s (one every m), lie in
// the table.
const _: () = {
    let mut end = 0;
    let mut i = 0;
    while i < LAST_RANGES.len() {
        let (lo, hi) = LAST_RANGES[i];
        assert!(lo == end && lo < hi);
        let mut m = 1;
        while m < LAST_R {
            let s = m * lo / LAST_L;
            assert!(m * (hi - 1) / LAST_L == s);
            assert!(m * hi - LAST_L * s <= LAST_L);
            m += 1;
        }
        end = hi;
        i += 1;
    }
    assert!(end == LAST_L);
};

/// The last stage (radix 5, `l` = 384, one block) with one lane, range by
/// range ([`last_range`]). Out of line: inlined into the decoder's pass,
/// the Cortex-M33 took more instructions in other stages of the pass
/// (findings O31). The tests run it on every target; the targets with
/// vectors run [`last_stage`] instead.
#[inline(never)]
fn last_straight_stage(re: &mut [f32; N], im: &mut [f32; N]) {
    last_range::<0>(re, im);
    last_range::<1>(re, im);
    last_range::<2>(re, im);
    last_range::<3>(re, im);
    last_range::<4>(re, im);
    last_range::<5>(re, im);
}

/// The butterflies of the last stage at k in range `I` of [`LAST_RANGES`],
/// each written out term by term ([`straight_butterfly`]). For each m the
/// butterfly reads one row of [`LAST_ROWS`], m·k − `LAST_L`·s(m), and
/// within it entry (s(m) + m·q mod `LAST_R`) mod `LAST_R`, a constant in
/// the range: the twiddle `twiddle_index` gives, W_N^((m·k + `LAST_L`·m·q)
/// mod N) (see [`LAST_ROWS`]). The four rows advance by m a butterfly, as
/// chunks of m rows, so each twiddle is a load at a constant offset from
/// one of four pointers.
///
/// The rows' slices pass through `black_box`, so the compiler takes their
/// starts as unknown. Seeing them as constants, it moved each start into
/// the loads' offsets, past the reach of the Cortex-M33's floating-point
/// load (±1 020 bytes), and added an address for each load: about 247
/// instructions a butterfly in the ranges whose rows do not start at 0,
/// 211 with `black_box` (rustc 1.93.0, opt-level "s"; findings O31). Only
/// the code depends on it; the values do not. Its guards are
/// codegen-check.sh's `m33s last_straight_stage` entries: `fploop60` (the
/// shortest range's loop) and `insns` fail without it.
#[inline(always)]
fn last_range<const I: usize>(re: &mut [f32; N], im: &mut [f32; N]) {
    let (lo, hi) = LAST_RANGES[I];
    let s = |m: usize| m * lo / LAST_L;
    let rows = |m: usize| &LAST_ROWS[m * lo - LAST_L * s(m)..m * hi - LAST_L * s(m)];
    let [r1, r2, r3, r4] = core::hint::black_box([rows(1), rows(2), rows(3), rows(4)]);
    let rows = (lo..hi)
        .zip(r1)
        .zip(r2.chunks_exact(2))
        .zip(r3.chunks_exact(3))
        .zip(r4.chunks_exact(4));
    for ((((k, w1), w2), w3), w4) in rows {
        let w = [w1, &w2[0], &w3[0], &w4[0]];
        let (r, i) = (
            &mut re[k..k + (LAST_R - 1) * LAST_L + 1],
            &mut im[k..k + (LAST_R - 1) * LAST_L + 1],
        );
        straight_butterfly!(r, i, LAST_L, [1 2 3 4], [0 1 2 3 4], (m, q) => w[m - 1][(s(m) + (m * q) % LAST_R) % LAST_R]);
    }
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
/// packed samples (Z[k], k = 0..1 920) in `buf`, with [`LANES`] lanes.
pub(crate) fn spectrum(samples: &[f32; WINDOW], buf: &mut FftBuffer) {
    spectrum_lanes::<LANES>(samples, buf);
}

/// [`spectrum`] with `L` lanes: with one, [`spectrum_of_quarters`] of the
/// window's quarters; with more, groups of `L` blocks in the first stage
/// and `L` butterflies side by side after it. The lane count is a parameter
/// so that the tests run every form on any host.
#[inline(always)]
fn spectrum_lanes<const L: usize>(samples: &[f32; WINDOW], buf: &mut FftBuffer) {
    if L == 1 {
        spectrum_of_quarters(quarters(samples), buf);
        return;
    }
    let (re, im) = buf.parts();
    first_stage::<L>(samples, re, im);
    side_by_side_stages::<L>(re, im);
}

/// The window as its four quarters in order: `quarters[m]` holds samples
/// m·`INPUT_STEP` .. (m + 1)·`INPUT_STEP` (`INPUT_STEP` is the front end's
/// hop). The one-lane transform reads the window in this form, so that the
/// front end can hand it the quarters of its ring in place.
pub(crate) type Quarters<'a> = [&'a [f32; INPUT_STEP]; 4];

/// The joined window `samples` as its [`Quarters`].
#[inline(always)]
fn quarters(samples: &[f32; WINDOW]) -> Quarters<'_> {
    let mut chunks = samples.chunks_exact(INPUT_STEP);
    core::array::from_fn(
        |_| match chunks.next().map(<&[f32; INPUT_STEP]>::try_from) {
            Some(Ok(quarter)) => quarter,
            _ => unreachable!(),
        },
    )
}

/// The windowed spectrum of the window held as its [`Quarters`], as
/// [`spectrum`] gives it bit for bit, by the transform with one lane:
/// [`straight_first_stage`], then the later stages written out term by
/// term ([`straight_stages`]).
#[inline(always)]
pub(crate) fn spectrum_of_quarters(quarters: Quarters<'_>, buf: &mut FftBuffer) {
    let (re, im) = buf.parts();
    straight_first_stage(quarters, re, im);
    straight_stages(re, im);
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

/// [`first_stage`] with a group of one, its butterfly written out term by
/// term (see [`straight_butterfly`]), from the window held as its
/// [`Quarters`]: the same window products, then the operations of
/// `stage::<4, 1, 1>` in its order. Block b's inputs m = 0..4 are samples
/// first + m·`INPUT_STEP` (and the next), with first + 1 < `INPUT_STEP`, so
/// input m lies in quarter m at `first`, whichever buffer holds it.
#[inline(always)]
fn straight_first_stage(quarters: Quarters<'_>, re: &mut [f32; N], im: &mut [f32; N]) {
    // W₄^(m·q) as `stage::<4, 1, 1>` reads it: TWIDDLES[m·q·N/4 mod N],
    // here FIFTH_TWIDDLES[m·q·N/20 mod N/5] (row m − 1).
    let w: [[[f32; 2]; 4]; 3] = straight_twiddles!(4, 0, [1 2 3], [0 1 2 3]);
    for (c, (r, i)) in re
        .chunks_exact_mut(4)
        .zip(im.chunks_exact_mut(4))
        .enumerate()
    {
        // As in `first_stage` with one block: its first sample, and the
        // window entries low[t] = HANN_HALF[first + t] and high[t] =
        // HANN_HALF[INPUT_STEP − 1 − first + t].
        let first = 2 * usize::from(DIGIT_REV[4 * c]);
        // Each quarter's pair named (an array of them was built on the
        // stack each block on the Cortex-M33: findings O31).
        let (x0, x1, x2, x3) = (
            &quarters[0][first..first + 2],
            &quarters[1][first..first + 2],
            &quarters[2][first..first + 2],
            &quarters[3][first..first + 2],
        );
        let low = &HANN_HALF[first..][..INPUT_STEP + 2];
        let high = &HANN_HALF[INPUT_STEP - 1 - first..][..INPUT_STEP + 2];
        let ar = [
            x0[0] * low[0],
            x1[0] * low[INPUT_STEP],
            x2[0] * high[INPUT_STEP + 1],
            x3[0] * high[1],
        ];
        let ai = [
            x0[1] * low[1],
            x1[1] * low[INPUT_STEP + 1],
            x2[1] * high[INPUT_STEP],
            x3[1] * high[0],
        ];
        straight_butterfly!(@store ar, ai, r, i, 1, [1 2 3], [0 1 2 3], (m, q) => w[m - 1][q]);
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
/// loop over bins, so it vectorises. With `INDEXED` (the targets without
/// a vector unit, [`bin_powers_out_of_line`]) the loop indexes its slices
/// rather than zipping their iterators, which leaves out the zipped
/// lengths' checks: on the Cortex-M33 35 instructions a bin instead of 42 (rustc
/// 1.93.0, opt-level "s"; findings O26). The same values either way.
#[inline(always)]
fn powers<const ROTATE: bool, const INDEXED: bool>(
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
    if INDEXED {
        for j in 0..n {
            let w = tw[j];
            let (wr, wi) = if ROTATE { (w[1], -w[0]) } else { (w[0], w[1]) };
            out[j] = bin_power(ar[j], ai[j], cr[n - 1 - j], ci[n - 1 - j], wr, wi);
        }
        return;
    }
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
    power_bins(
        |b| spectrum(samples, b),
        buf,
        first_bin,
        count,
        |i, p| out[i..i + p.len()].copy_from_slice(p),
    );
}

/// Powers handed to the caller at a time by [`power_bins`].
pub(crate) const POWER_CHUNK: usize = 64;

/// Calls `f(i, powers)` with the powers of bins `first_bin + i ..`, in
/// order and in chunks of at most [`POWER_CHUNK`], for bins `first_bin ..
/// first_bin + count` of the Hann-windowed spectrum that `transform`
/// leaves in `buf` (normalised as in [`power_spectrum`]): [`spectrum`] of
/// the window's samples in order, or [`spectrum_of_quarters`] of its
/// quarters, as the front end's ring holds them where the transform takes
/// one lane. Chunks let the caller's per-bin work run as a plain loop over
/// values (which vectorises).
///
/// The samples must be finite and within ±[`INPUT_LIMIT`], as the front
/// end's `classify` leaves them: the transform's stages rely on it to be
/// bit for bit the full set of products (see `stage`).
///
/// [`INPUT_LIMIT`]: super::frontend::INPUT_LIMIT
pub(crate) fn power_bins<T: FnOnce(&mut FftBuffer), F: FnMut(usize, &[f32])>(
    transform: T,
    buf: &mut FftBuffer,
    first_bin: usize,
    count: usize,
    mut f: F,
) {
    assert!(first_bin + count <= BINS, "bins beyond the transform");
    transform(buf);
    let (re, im) = buf.parts();
    let mut p = [0.0f32; POWER_CHUNK];
    let mut i = 0;
    while i < count {
        let n = (count - i).min(POWER_CHUNK);
        bin_powers(re, im, first_bin + i, &mut p[..n]);
        f(i, &p[..n]);
        i += n;
    }
}

/// Powers of real-transform bins `k0 .. k0 + out.len()` into `out`: bin 0
/// (which pairs with itself), the bins to 960 and those above as separate
/// plain loops, which vectorise where the target has vectors. Where it has
/// none ([`SCALAR_FORMS`]), [`bin_powers_out_of_line`].
fn bin_powers(re: &[f32; N], im: &[f32; N], k0: usize, out: &mut [f32]) {
    if SCALAR_FORMS {
        bin_powers_out_of_line(re, im, k0, out);
    } else {
        bin_powers_as::<false>(re, im, k0, out);
    }
}

/// [`bin_powers`] for the targets without a vector unit: out of line, so
/// that its loops keep their pointers in registers (inlined into the front
/// end's row, the Cortex-M33 recomputed each address from the buffer's
/// base), and with [`powers`]' indexed loop.
#[inline(never)]
fn bin_powers_out_of_line(re: &[f32; N], im: &[f32; N], k0: usize, out: &mut [f32]) {
    bin_powers_as::<true>(re, im, k0, out);
}

/// [`bin_powers`], with [`powers`]' `INDEXED` form or not.
#[inline(always)]
fn bin_powers_as<const INDEXED: bool>(re: &[f32; N], im: &[f32; N], k0: usize, out: &mut [f32]) {
    let n = out.len();
    let zero = usize::from(k0 == 0);
    if zero == 1 {
        let w = SPLIT_TWIDDLES[0];
        out[0] = bin_power(re[0], im[0], re[0], im[0], w[0], w[1]);
    }
    let k1 = k0 + zero;
    let low = (k0 + n).min(961).saturating_sub(k1);
    if low > 0 {
        powers::<false, INDEXED>(
            re,
            im,
            k1,
            &mut out[zero..zero + low],
            &SPLIT_TWIDDLES[k1..],
        );
    }
    let k2 = k1 + low;
    if zero + low < n {
        powers::<true, INDEXED>(
            re,
            im,
            k2,
            &mut out[zero + low..],
            &SPLIT_TWIDDLES[k2 - 960..],
        );
    }
}

/// Bytes of the static tables this module uses (twiddles, real-split
/// twiddles, half window, digit-reversal permutation): [`table_bytes`] of
/// [`ONE_LANE`].
pub const TABLE_BYTES: usize = table_bytes(ONE_LANE);

/// [`TABLE_BYTES`] where the transform takes one lane or not: the twiddles
/// are [`FIFTH_TWIDDLES`] and [`LAST_ROWS`] with one lane, else
/// [`TWIDDLES`].
const fn table_bytes(one_lane: bool) -> usize {
    let twiddles = if one_lane {
        core::mem::size_of::<[[f32; 2]; N / FIFTH]>() + core::mem::size_of::<[LastRow; LAST_L]>()
    } else {
        core::mem::size_of::<[[f32; 2]; LAST_TWIDDLES]>()
    };
    twiddles
        + core::mem::size_of::<[[f32; 2]; 961]>()
        + core::mem::size_of::<[f32; WINDOW / 2 + 1]>()
        + core::mem::size_of::<[u16; N]>()
}

/// The sizes of the statics [`TABLE_BYTES`] counts.
#[cfg(test)]
pub(crate) fn table_bytes_of_statics() -> usize {
    table_bytes_of_statics_as(ONE_LANE)
}

/// [`table_bytes_of_statics`] where the transform takes one lane or not.
#[cfg(test)]
fn table_bytes_of_statics_as(one_lane: bool) -> usize {
    let twiddles = if one_lane {
        core::mem::size_of_val(&FIFTH_TWIDDLES) + core::mem::size_of_val(&LAST_ROWS)
    } else {
        core::mem::size_of_val(&TWIDDLES)
    };
    twiddles
        + core::mem::size_of_val(&SPLIT_TWIDDLES)
        + core::mem::size_of_val(&HANN_HALF)
        + core::mem::size_of_val(&DIGIT_REV)
}

#[cfg(test)]
mod tests;
