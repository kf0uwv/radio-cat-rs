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

//! A small seeded PRNG: xoshiro256** seeded through SplitMix64.
//!
//! Everything here is integer arithmetic or IEEE `+ − × ÷`, which are
//! exactly rounded on every platform, so a seed gives the same stream on
//! Linux and on Windows. [`Rng::gauss`] is Irwin–Hall (sum of twelve
//! uniforms) rather than Box–Muller precisely so it never calls `ln` or
//! `cos`, whose results come from the platform's maths library.

/// A seeded, portable pseudo-random generator (xoshiro256**).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng {
    s: [u64; 4],
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    /// A generator seeded from `seed`. Every seed (including 0) is valid.
    pub fn new(seed: u64) -> Self {
        let mut sm = seed;
        let s = [
            splitmix64(&mut sm),
            splitmix64(&mut sm),
            splitmix64(&mut sm),
            splitmix64(&mut sm),
        ];
        Rng { s }
    }

    /// An independent generator for sub-stream `stream`, derived from this
    /// one's current state without advancing it. Giving each concern its
    /// own stream means adding a draw to one does not perturb another.
    pub fn fork(&self, stream: u64) -> Rng {
        let mut sm = self.s[0]
            ^ self.s[1].rotate_left(17)
            ^ self.s[2].rotate_left(31)
            ^ self.s[3].rotate_left(47)
            ^ stream.wrapping_mul(0xD1B5_4A32_D192_ED03);
        let s = [
            splitmix64(&mut sm),
            splitmix64(&mut sm),
            splitmix64(&mut sm),
            splitmix64(&mut sm),
        ];
        Rng { s }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// A uniform `f64` in `[0, 1)` with 53 bits of precision.
    pub fn unit_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// A uniform `f64` in `[lo, hi)`.
    pub fn range_f64(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit_f64()
    }

    /// A uniform integer in `lo..=hi` (unbiased; Lemire's method).
    /// If `hi < lo` the two are swapped.
    pub fn range_u32(&mut self, lo: u32, hi: u32) -> u32 {
        let (lo, hi) = if hi < lo { (hi, lo) } else { (lo, hi) };
        let span = u64::from(hi - lo) + 1;
        if span > u64::from(u32::MAX) {
            return self.next_u64() as u32;
        }
        let threshold = (1u64 << 32) % span;
        loop {
            let x = self.next_u64() >> 32;
            let m = x * span;
            if (m & 0xFFFF_FFFF) >= threshold {
                return lo + (m >> 32) as u32;
            }
        }
    }

    /// `true` with probability `p` (values outside `[0, 1]` saturate).
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit_f64() < p
    }

    /// An approximately standard-normal deviate (mean 0, σ 1), bounded to
    /// ±6. Irwin–Hall: portable because it uses only additions.
    pub fn gauss(&mut self) -> f64 {
        let mut sum = 0.0;
        for _ in 0..12 {
            sum += self.unit_f64();
        }
        sum - 6.0
    }

    /// A uniformly chosen element of `items`.
    ///
    /// # Panics
    /// If `items` is empty.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        assert!(!items.is_empty(), "Rng::pick on an empty slice");
        let i = self.range_u32(0, (items.len() - 1) as u32) as usize;
        &items[i]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_differ() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn known_first_value_is_stable() {
        // Pins the algorithm: a change here changes every seeded output in
        // the crate, which ADR 0021 promises does not happen silently.
        let mut r = Rng::new(0);
        assert_eq!(r.next_u64(), PINNED_FIRST);
    }

    // The published xoshiro256** first output for SplitMix64(0) seeding.
    const PINNED_FIRST: u64 = 0x99EC_5F36_CB75_F2B4;

    #[test]
    fn fork_is_independent_and_deterministic() {
        let r = Rng::new(7);
        let mut f1 = r.fork(1);
        let mut f1b = r.fork(1);
        let mut f2 = r.fork(2);
        let a = f1.next_u64();
        assert_eq!(a, f1b.next_u64());
        assert_ne!(a, f2.next_u64());
    }

    #[test]
    fn unit_in_range_and_mean_half() {
        let mut r = Rng::new(3);
        let n = 100_000;
        let mut sum = 0.0;
        for _ in 0..n {
            let x = r.unit_f64();
            assert!((0.0..1.0).contains(&x));
            sum += x;
        }
        assert!((sum / n as f64 - 0.5).abs() < 0.01);
    }

    #[test]
    fn range_u32_covers_inclusive_bounds() {
        let mut r = Rng::new(9);
        let mut seen = [false; 5];
        for _ in 0..1000 {
            let v = r.range_u32(3, 7);
            assert!((3..=7).contains(&v));
            seen[(v - 3) as usize] = true;
        }
        assert!(seen.iter().all(|&s| s));
        assert_eq!(r.range_u32(5, 5), 5);
    }

    #[test]
    fn gauss_has_unit_variance() {
        let mut r = Rng::new(11);
        let n = 50_000;
        let (mut s, mut s2) = (0.0, 0.0);
        for _ in 0..n {
            let g = r.gauss();
            s += g;
            s2 += g * g;
        }
        let mean = s / n as f64;
        let var = s2 / n as f64 - mean * mean;
        assert!(mean.abs() < 0.03, "mean {mean}");
        assert!((var - 1.0).abs() < 0.05, "var {var}");
    }
}
