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
    let mut e = ((bits >> 23) & 0xff) as i32 - 127;
    let mut m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000); // [1, 2)
    if m > SQRT2 {
        m *= 0.5; // exact
        e += 1;
    }
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
    fn floor_and_cap() {
        assert_eq!(log2_power(0.0), -100.0);
        assert_eq!(log2_power(-5.0), -100.0);
        assert_eq!(log2_power(f32::NAN), -100.0);
        assert_eq!(log2_power(1.0e-40), -100.0); // subnormal floored
        assert!(log2_power(f32::INFINITY) > 127.9);
    }
}
