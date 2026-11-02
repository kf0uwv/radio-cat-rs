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

//! Deterministic numeric helpers for the decoder: no `f64`, no `libm`,
//! only `f32` `+ − × ÷`, comparisons and bit operations (ADR 0024,
//! amended scope rule 3).

/// Smallest power fed to [`log2`]: 2⁻¹⁰⁰, a normal `f32`. Flooring here
/// means no subnormal value reaches the logarithm, so results do not
/// depend on flush-to-zero settings at this stage.
pub(crate) const POWER_FLOOR: f32 = 7.888_609e-31;

/// √2, the upper end of the reduced mantissa range.
const SQRT2: f32 = core::f32::consts::SQRT_2;

// Series coefficients 2 / (k · ln 2) for k = 1, 3, 5, 7, 9.
const C1: f32 = 2.885_39;
const C3: f32 = 0.961_796_7;
const C5: f32 = 0.577_078;
const C7: f32 = 0.412_198_6;
const C9: f32 = 0.320_598_9;

/// `log2(x)` for a positive, finite, normal `x` (callers floor with
/// [`POWER_FLOOR`] and cap at `f32::MAX`).
///
/// The exponent comes from the IEEE bits; the mantissa m is reduced to
/// [√½, √2] and `log2(m) = (2 / ln 2) · atanh(z)`, z = (m − 1) / (m + 1),
/// with the series z + z³/3 + z⁵/5 + z⁷/7 + z⁹/9 (|z| ≤ 0.172, so the
/// first omitted term is below 1e-9). Absolute error is a few f32 ulps.
#[inline]
pub(crate) fn log2(x: f32) -> f32 {
    let bits = x.to_bits();
    let e0 = ((bits >> 23) & 0xff) as i32 - 127;
    // The mantissa in [1, 2).
    let m0 = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    // Selects rather than a branch, so a loop over values can vectorise.
    let high = m0 > SQRT2;
    let m = if high { m0 * 0.5 } else { m0 }; // exact
    let e = e0 + i32::from(high);
    let z = (m - 1.0) / (m + 1.0);
    let z2 = z * z;
    let series = z * (C1 + z2 * (C3 + z2 * (C5 + z2 * (C7 + z2 * C9))));
    e as f32 + series
}

/// `log2` of a power value after flooring and capping, for any input
/// (NaN maps to the floor).
#[inline]
// Not `clamp`: `f32::clamp` returns NaN for a NaN input, while `max` and
// `min` with a NaN operand return the other operand.
#[allow(clippy::manual_clamp)]
pub(crate) fn log2_power(p: f32) -> f32 {
    log2(p.max(POWER_FLOOR).min(f32::MAX))
}

/// log2(e).
pub(crate) const LOG2_E: f32 = core::f32::consts::LOG2_E;

// Taylor coefficients (ln 2)^k / k! for 2^f = e^(f ln 2), k = 1..7.
const E1: f32 = core::f32::consts::LN_2;
const E2: f32 = 0.240_226_5;
const E3: f32 = 0.055_504_11;
const E4: f32 = 0.009_618_129;
const E5: f32 = 0.001_333_355_8;
const E6: f32 = 0.000_154_035_3;
const E7: f32 = 0.000_015_252_734;

/// `2^y` for `y ≤ 0`: exactly 0.0 below −126 (no subnormal results).
///
/// y = n + f with n = ⌊y⌋ and f ∈ [0, 1): 2ⁿ is built from the IEEE bits
/// and 2ᶠ from the Taylor series of e^(f ln 2) to the 7th power (first
/// omitted term below 1.5e-6 relative).
#[inline]
pub(crate) fn exp2_neg(y: f32) -> f32 {
    // Selects rather than branches, so a loop over values can vectorise:
    // out-of-range lanes compute on 0.0 and are replaced by 0.0 at the end.
    let in_range = y >= -126.0; // false for NaN
    let y = if !in_range || y > 0.0 { 0.0 } else { y };
    // The same ⌊y⌋ either way; on x86-64 without the conversion, which does
    // not vectorise there.
    #[cfg(target_arch = "x86_64")]
    let n = floor_small(y);
    #[cfg(not(target_arch = "x86_64"))]
    let n = floor_by_truncation(y);
    let f = y - n as f32;
    let p = 1.0 + f * (E1 + f * (E2 + f * (E3 + f * (E4 + f * (E5 + f * (E6 + f * E7))))));
    let r = p * f32::from_bits(((n + 127) as u32) << 23);
    if in_range {
        r
    } else {
        0.0
    }
}

/// ⌊y⌋ for y in [−2²², 2²²]: truncate, then one less if that is above y.
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn floor_by_truncation(y: f32) -> i32 {
    let t = y as i32; // truncates toward zero
    if (t as f32) > y {
        t - 1
    } else {
        t
    }
}

/// [`floor_by_truncation`] with no float-to-integer conversion (Rust's
/// saturating `as i32` does not vectorise on x86-64 SSE2, which converted
/// each lane separately in `exp2_neg`'s loop; elsewhere the conversion is
/// one instruction, saturating in hardware, and is kept). Adding 1.5·2²³
/// lands in [2²³, 2²⁴), where the spacing is 1, so the sum is y rounded
/// to an integer r, and r is the sum's low mantissa bits; one less if
/// that rounded up. Every operation is exact but the rounding to r, and r
/// is then corrected to ⌊y⌋, so the result equals truncating and
/// correcting (`y as i32`, minus one if that is above y).
///
/// Domain: y in [−2²², 2²²] (not NaN), where the sum lies in [2²³, 2²⁴]
/// and r in its bits is exact (at 2²⁴ the carry into the exponent still
/// gives 2²²); outside it the result is wrong. Callers: `exp2_neg`
/// (clamped to [−126, 0]) and the waveform's phase increment (in
/// [−2¹⁵, 2¹⁵]). Relies on round-to-nearest without excess precision (SSE2
/// on x86-64, where it is used).
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
#[inline(always)]
pub(crate) fn floor_small(y: f32) -> i32 {
    debug_assert!(
        (-4_194_304.0..=4_194_304.0).contains(&y),
        "floor_small: {y} outside [-2^22, 2^22]"
    );
    const MAGIC: f32 = 12_582_912.0; // 1.5·2²³
    let big = y + MAGIC;
    let r = (big.to_bits() as i32).wrapping_sub(MAGIC.to_bits() as i32);
    if big - MAGIC > y {
        r - 1
    } else {
        r
    }
}

// A six-term odd polynomial for atan(z) on [−1, 1]: a1 … a11 =
// 0.99997726, −0.33262347, 0.19354346, −0.11643287, 0.05265332,
// −0.01172120 (used here to seven digits), as given by F. Mazzoli, "Speeding up
// atan2f by 50x" (2021, mazzo.li/posts/vectorized-atan2.html), who
// attributes them to C. Hastings Jr., "Approximations for Digital
// Computers" (Princeton University Press, 1955); the book itself was not
// checked. It is not in Abramowitz and Stegun, whose 4.4.47 is Hastings'
// five-term version (error 1e-5) and 4.4.49 a degree-17 polynomial from
// Carlson and Goldstein (2e-8). Checked here in
// f64 against atan on [0, 1]: largest error 1.7e-6 rad (2.7e-7 turn) with
// the values as written. Divided by 2π so the result is in turns.
const A1: f32 = 0.999_977_3 / core::f32::consts::TAU;
const A3: f32 = -0.332_623_5 / core::f32::consts::TAU;
const A5: f32 = 0.193_543_5 / core::f32::consts::TAU;
const A7: f32 = -0.116_432_9 / core::f32::consts::TAU;
const A9: f32 = 0.052_653_32 / core::f32::consts::TAU;
const A11: f32 = -0.011_721_2 / core::f32::consts::TAU;

/// `|x|` (`f32::abs` is not in `core` on the minimum Rust).
#[inline]
pub(crate) fn abs(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & 0x7fff_ffff)
}

/// The angle of `(x, y)` in turns, in [−½, ½] (`atan2(y, x) / 2π`): 0
/// for `(0, 0)` and for non-finite input. Absolute error below 5 × 10⁻⁷
/// turn (3 × 10⁻⁶ rad; the polynomial's own error is 1.7 × 10⁻⁶ rad).
///
/// The argument is reduced to the first octant (|z| ≤ 1, z = min/max of
/// |x| and |y|) and `atan(z)` taken from an odd polynomial of degree 11.
pub(crate) fn atan2_turns(y: f32, x: f32) -> f32 {
    let (ax, ay) = (abs(x), abs(y));
    let big = if ax > ay { ax } else { ay };
    // False for NaN as well; infinities would give NaN in z.
    let usable = big > 0.0 && big.is_finite();
    if !usable {
        return 0.0;
    }
    let small = if ax > ay { ay } else { ax };
    let z = small / big;
    let z2 = z * z;
    let mut t = z * (A1 + z2 * (A3 + z2 * (A5 + z2 * (A7 + z2 * (A9 + z2 * A11)))));
    if ay > ax {
        t = 0.25 - t;
    }
    if x < 0.0 {
        t = 0.5 - t;
    }
    if y < 0.0 {
        -t
    } else {
        t
    }
}

/// Lanes of the decoder's four-wide loops: samples taken four at a time,
/// each lane with its own sums, so that the loops map onto 128-bit vector
/// registers (four `f32` or `u32`). The lanes and the order in which they
/// are added are fixed by the sample index, not by the target. Lane loops
/// are written index by index (`for l in 0..LANES`), one statement per
/// vector operation, which is the form the compiler maps onto registers.
pub(crate) const LANES: usize = 4;

/// Whether the compiler vectorises the decoder's lane loops: on x86-64 and
/// AArch64, which always have SSE2 or NEON (the targets measured). Not on
/// the Cortex-M33, and not on 32-bit ARM with NEON, whose vector floating
/// point is not IEEE, so these loops stay scalar there. Where a loop's form
/// depends on it, the form is a const parameter, so that the tests run both
/// on any host.
pub(crate) const VECTOR: bool = cfg!(any(target_arch = "x86_64", target_arch = "aarch64"));

// sin(2πt) = t·(S1 + S3·t² + S5·t⁴) and cos(2πt) = C0 + C2·t² + C4·t⁴:
// fitted for this crate by linear programming (minimax absolute error on
// 4 001 evenly spaced points of 0 ≤ t ≤ 1/8 turn, in `f64`), each
// coefficient then rounded to `f32`. The grid fit is near, not at, the
// continuous minimax: S5 is weakly determined (other grids and fitters put
// it between about 79.52 and 79.53), and no coefficient was adjusted for
// `f32` evaluation. These bits are the definition (`cos_sin_turns_pinned`
// holds them). Evaluated in `f32` as written below (Horner, no fused
// multiply-add), the error at every `u32` phase is below 1.1 × 10⁻⁵ (the
// cosine's; the sine's is 6.6 × 10⁻⁷), against the quarter-wave table's
// 3.5 × 10⁻⁷. The degree is the least whose error does not show in the
// subtraction: clean signals leave −100 dB of their energy (the table and
// degrees 7 and 6, −103 dB), against −53 dB from a start between samples;
// degrees 7 and 6 (error 1.6 × 10⁻⁷) cost 2.8% more of an A76 three-pass
// slot.
const S1: f32 = 6.283_154_5;
const S3: f32 = -41.325_558;
const S5: f32 = 79.528_26;
const C0: f32 = 0.999_990_05;
const C2: f32 = -19.727_69;
const C4: f32 = 62.962_955;

/// `(cos, sin)` of `2π · phase / 2³²`, by polynomial: the phase is reduced
/// in integer arithmetic to the nearest quarter turn k and a remainder of
/// at most ⅛ turn, both polynomials are taken on the remainder, and k
/// swaps them and sets the signs (bit operations, no branch), so a loop
/// over samples vectorises. Not the waveform's table
/// ([`sin_turns`](crate::waveform::sin_turns)): the encoder keeps that.
#[inline(always)]
pub(crate) fn cos_sin_turns(phase: u32) -> (f32, f32) {
    let u = phase.wrapping_add(1 << 29);
    let k = u >> 30;
    // The remainder in [−2²⁹, 2²⁹), exactly; one rounding to `f32`, then
    // an exact scaling to turns.
    let r = (u & 0x3fff_ffff) as i32 - (1 << 29);
    let t = r as f32 * (1.0 / 4_294_967_296.0);
    let z = t * t;
    let s = t * (S1 + z * (S3 + z * S5));
    let c = C0 + z * (C2 + z * C4);
    // Quarter k: (cos, sin) = (c, s), (−s, c), (−c, −s), (s, −c).
    let swap = 0u32.wrapping_sub(k & 1);
    let (sb, cb) = (s.to_bits(), c.to_bits());
    let sin = ((cb & swap) | (sb & !swap)) ^ ((k & 2) << 30);
    let cos = ((sb & swap) | (cb & !swap)) ^ ((k.wrapping_add(1) & 2) << 30);
    (f32::from_bits(cos), f32::from_bits(sin))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_libm_closely_over_the_whole_range() {
        let mut worst = 0.0f64;
        let mut x = POWER_FLOOR;
        while x < 1.0e30 {
            for k in 0..64 {
                let v = x * (1.0 + k as f32 / 64.0);
                let exact = libm::log2(f64::from(v));
                // Within a few f32 ulps of the result (whose ulp grows
                // with |log2|): the sum `e + series` rounds to f32.
                let tol = 1.0e-6 + 6.0e-8 * exact.abs();
                worst = worst.max((f64::from(log2(v)) - exact).abs() / tol);
            }
            x *= 1.7;
        }
        assert!(worst <= 1.0, "{worst}");
    }

    #[test]
    fn exact_at_powers_of_two_and_pinned_bits() {
        for e in -100..=100 {
            let v = f32::from_bits(((e + 127) as u32) << 23);
            assert_eq!(log2(v), e as f32, "2^{e}");
        }
        // Pinned outputs: any change to the arithmetic changes these.
        let pinned: [(f32, u32); 4] = [
            (3.0, 0x3fca_e00d),
            (0.1, 0xc054_9a78),
            (1.0e-12, 0xc21f_73da),
            (12_345.678, 0x4159_77ae),
        ];
        for (v, want) in pinned {
            assert_eq!(log2(v).to_bits(), want, "log2({v}) = {}", log2(v));
        }
    }

    #[test]
    fn exp2_coefficients_are_pinned_and_correctly_rounded() {
        let pinned: [(f32, u32); 6] = [
            (E2, 0x3e75_fdf0),
            (E3, 0x3d63_5847),
            (E4, 0x3c1d_955b),
            (E5, 0x3aae_c3ff),
            (E6, 0x3921_8489),
            (E7, 0x377f_e5fe),
        ];
        let mut term = 1.0f64; // (ln 2)^k / k! in f64
        let ln2 = core::f64::consts::LN_2;
        term *= ln2; // k = 1
        assert_eq!(E1.to_bits(), (term as f32).to_bits());
        for (k, (value, bits)) in (2..).zip(pinned) {
            term *= ln2 / f64::from(k);
            assert_eq!(value.to_bits(), bits, "E{k}");
            assert_eq!(bits, (term as f32).to_bits(), "E{k} not (ln 2)^k / k!");
        }
    }

    #[test]
    fn exp2_matches_libm_closely() {
        let mut worst = 0.0f64;
        let mut y = 0.0f32;
        while y > -126.0 {
            let exact = libm::exp2(f64::from(y));
            worst = worst.max((f64::from(exp2_neg(y)) - exact).abs() / exact);
            y -= 0.013;
        }
        assert!(worst < 3.0e-6, "{worst}");
        assert_eq!(exp2_neg(0.0), 1.0);
        assert_eq!(exp2_neg(-1.0), 0.5);
        assert_eq!(exp2_neg(-127.0), 0.0);
        assert_eq!(exp2_neg(f32::NAN), 0.0);
        assert_eq!(exp2_neg(3.0), 1.0); // positive arguments are clamped to 0
    }

    #[test]
    fn atan2_turns_matches_libm_closely() {
        let mut worst = 0.0f64;
        for i in 0..3_600 {
            let a = f64::from(i) * core::f64::consts::TAU / 3_600.0;
            for r in [1e-30f64, 1e-3, 1.0, 7.5e3, 1e30] {
                let (x, y) = ((r * libm::cos(a)) as f32, (r * libm::sin(a)) as f32);
                let exact = libm::atan2(f64::from(y), f64::from(x)) / core::f64::consts::TAU;
                let mut d = (f64::from(atan2_turns(y, x)) - exact).abs();
                d = d.min((d - 1.0).abs()); // ±½ turn are the same angle
                worst = worst.max(d);
            }
        }
        assert!(worst < 5e-7, "worst {worst}");
        assert_eq!(atan2_turns(0.0, 0.0), 0.0);
        assert_eq!(atan2_turns(f32::NAN, 1.0), 0.0);
        assert_eq!(atan2_turns(1.0, f32::INFINITY), 0.0);
        assert_eq!(atan2_turns(0.0, 1.0), 0.0);
        assert_eq!(atan2_turns(1.0, 0.0), 0.25);
        assert_eq!(atan2_turns(0.0, -1.0), 0.5);
        assert_eq!(atan2_turns(-1.0, 0.0), -0.25);
    }

    /// `cos_sin_turns` against `libm` in `f64` at every `stride`-th phase
    /// (and at both ends of each quarter's reduced range): largest error
    /// of either part, and the zero part at the quarter turns exact.
    fn cos_sin_turns_accurate(stride: usize) -> f64 {
        let mut worst = 0.0f64;
        let mut check = |p: u32| {
            let (c, s) = cos_sin_turns(p);
            let a = f64::from(p) * core::f64::consts::TAU / 4_294_967_296.0;
            worst = worst
                .max((f64::from(c) - libm::cos(a)).abs())
                .max((f64::from(s) - libm::sin(a)).abs());
        };
        for p in (0..=u32::MAX).step_by(stride) {
            check(p);
        }
        for k in 0..4u32 {
            for d in [0u32, 1, (1 << 29) - 1, 1 << 29, (1 << 29) + 1] {
                check((k << 30).wrapping_add(d));
                check((k << 30).wrapping_sub(d));
            }
        }
        assert!(worst < 1.1e-5, "worst {worst:e}");
        // At the quarter turns the remainder is 0: the sine part exactly 0,
        // the cosine part ±C0.
        for (k, (c, s)) in [(C0, 0.0), (0.0, C0), (-C0, 0.0), (0.0, -C0)]
            .into_iter()
            .enumerate()
        {
            assert_eq!(cos_sin_turns((k as u32) << 30), (c, s), "quarter {k}");
        }
        worst
    }

    #[test]
    fn cos_sin_turns_matches_libm_closely() {
        cos_sin_turns_accurate(997);
    }

    /// Every `u32` phase (about 4.3·10⁹; run in release).
    #[test]
    #[ignore]
    fn cos_sin_turns_matches_libm_closely_everywhere() {
        // Measured: 1.006 × 10⁻⁵ (the cosine's). The lower bound pins that
        // value, so a better polynomial fails here too: re-pin it if the
        // coefficients change.
        assert!(cos_sin_turns_accurate(1) > 1.0e-5);
    }

    #[test]
    fn cos_sin_turns_pinned() {
        // The coefficients' bits, and a hash of the outputs over a sweep
        // of phases: any change in the arithmetic (even one ulp) fails.
        let bits = [S1, S3, S5, C0, C2, C4].map(f32::to_bits);
        assert_eq!(
            bits,
            [
                0x40c9_0f9a,
                0xc225_4d5f,
                0x429f_0e78,
                0x3f7f_ff59,
                0xc19d_d24f,
                0x427b_da11
            ]
        );
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for p in (0..=u32::MAX).step_by(65_537) {
            let (c, s) = cos_sin_turns(p);
            for b in c
                .to_bits()
                .to_le_bytes()
                .into_iter()
                .chain(s.to_bits().to_le_bytes())
            {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        assert_eq!(h, 0xc2ee_b714_7593_8caf, "sweep hash {h:#018x}");
    }

    #[test]
    fn floor_and_cap() {
        assert_eq!(log2_power(0.0), -100.0);
        assert_eq!(log2_power(-5.0), -100.0);
        assert_eq!(log2_power(f32::NAN), -100.0);
        assert_eq!(log2_power(1.0e-40), -100.0); // subnormal floored
        assert!(log2_power(f32::INFINITY) > 127.9);
    }

    #[test]
    fn exp2_pinned_bits() {
        // Bit patterns of `exp2_neg` at fixed inputs, including the edges;
        // any change in its arithmetic (even one ulp) fails here.
        let pinned: [(f32, u32); 19] = [
            (0.0, 0x3f80_0000),
            (-0.0, 0x3f80_0000),
            (-1.0e-7, 0x3f7f_fff3),
            (-0.25, 0x3f57_44fc),
            (-0.5, 0x3f35_04f3),
            (-0.999_999_9, 0x3f00_0001),
            (-1.0, 0x3f00_0000),
            (-1.3, 0x3ecf_efc6),
            (-3.7, 0x3d9d_9624),
            (-10.25, 0x3a57_44fc),
            (-27.5, 0x31b5_04f3),
            (-50.123, 0x266b_1416),
            (-100.0, 0x0d80_0000),
            (-125.9, 0x0089_2fd6),
            (-126.0, 0x0080_0000),
            (-126.000_01, 0x0000_0000), // below −126: exactly 0
            (-1.0e9, 0x0000_0000),
            (1.0, 0x3f80_0000), // positive inputs are taken as 0
            (f32::NAN, 0x0000_0000),
        ];
        for (x, want) in pinned {
            assert_eq!(exp2_neg(x).to_bits(), want, "exp2_neg({x})");
        }
        // And a hash over a sweep of −126…0, which reaches the fractional
        // parts where the series' last terms matter.
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for k in 0..=10_000u32 {
            let y = -(k as f32) * 0.0126;
            for b in exp2_neg(y).to_bits().to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        assert_eq!(h, 0x0d68_2590_515e_d35b, "sweep hash {h:#018x}");
    }

    /// `floor_small` against truncation at every `f32` from −126 to 0 in
    /// `stride` steps of the bit pattern, and at both zeros.
    fn floor_small_matches(stride: usize) {
        // Negative floats' bit patterns grow with the magnitude.
        let mut n = 0;
        for bits in ((-0.0f32).to_bits()..=(-126.0f32).to_bits()).step_by(stride) {
            n += 1;
            let y = f32::from_bits(bits);
            assert_eq!(floor_small(y), floor_by_truncation(y), "{y}");
        }
        assert!(n > 1_000_000_000 / stride, "{n}");
        for y in [0.0f32, -0.0, -126.0, -125.5, -0.5, -1.5, -2.5] {
            assert_eq!(floor_small(y), floor_by_truncation(y), "{y}");
        }
    }

    #[test]
    fn floor_small_matches_truncation() {
        floor_small_matches(9_973);
    }

    /// Every `f32` in −126…0 (about 1.1·10⁹ values; run in release).
    #[test]
    #[ignore]
    fn floor_small_matches_truncation_everywhere() {
        floor_small_matches(1);
    }
}
