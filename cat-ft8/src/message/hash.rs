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

//! Callsign hash codes (QEX 2020 §2, Table 2: h10, h12, h22) and a
//! fixed-capacity table that maps them back to callsigns.
//!
//! A callsign of up to 11 characters from ` 0-9A-Z/` is read as a base-38
//! number (space = 0, digits 1–10, letters 11–36, `/` = 37), padded with
//! trailing spaces to 11 characters. That number times 47 055 833 459,
//! modulo 2^64, gives the hash in its top bits: h22 is the top 22 bits,
//! h12 and h10 the top 12 and 10. The QEX paper defers both the
//! multiplier and this alphabet's order to its reference programs, which it
//! does not print; they were taken from `ft8_lib` (see NOTICE) and are
//! confirmed by the black-box vectors, which include hashed calls of every
//! length.

use super::text::Call;

const MULTIPLIER: u64 = 47_055_833_459;

/// Which of the three hash lengths a message field carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashSize {
    /// Type 1/2 `c28` fields (`<PJ4/K1ABC> W9XYZ`).
    Bits22,
    /// Type 4 `h12` field.
    Bits12,
    /// Type 0.1 (DXpedition) `h10` field.
    Bits10,
}

impl HashSize {
    /// Reduces a 22-bit hash to this length.
    pub fn from_h22(self, h22: u32) -> u32 {
        match self {
            HashSize::Bits22 => h22,
            HashSize::Bits12 => h22 >> 10,
            HashSize::Bits10 => h22 >> 12,
        }
    }
}

/// Index of `c` in the hash alphabet ` 0-9A-Z/`.
pub(crate) fn char38(c: u8) -> Option<u64> {
    match c {
        b' ' => Some(0),
        b'0'..=b'9' => Some(u64::from(c - b'0') + 1),
        b'A'..=b'Z' => Some(u64::from(c - b'A') + 11),
        b'/' => Some(37),
        _ => None,
    }
}

/// The 22-bit hash of `call`; `None` if it is empty, longer than 11
/// characters, or uses characters outside ` 0-9A-Z/`.
pub fn hash22(call: &str) -> Option<u32> {
    let b = call.as_bytes();
    if b.is_empty() || b.len() > 11 {
        return None;
    }
    let mut n: u64 = 0;
    for i in 0..11 {
        let c = b.get(i).copied().unwrap_or(b' ');
        n = n * 38 + char38(c)?;
    }
    Some((MULTIPLIER.wrapping_mul(n) >> (64 - 22)) as u32)
}

/// The hash of `call` at the given length.
pub fn hash(call: &str, size: HashSize) -> Option<u32> {
    hash22(call).map(|h| size.from_h22(h))
}

/// Somewhere to keep the full callsigns behind hash codes.
///
/// Packing offers every full callsign it encodes, and unpacking every one
/// it decodes, to [`remember`](CallHashes::remember); unpacking asks
/// [`lookup`](CallHashes::lookup) to resolve a hash. A miss is shown as
/// `<...>`, as WSJT-X does.
pub trait CallHashes {
    /// Records `call` (no angle brackets).
    fn remember(&mut self, call: &str);
    /// The most recently remembered call with this hash, if any.
    fn lookup(&self, size: HashSize, hash: u32) -> Option<Call>;
}

/// Remembers nothing; every lookup misses.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoHashes;

impl CallHashes for NoHashes {
    fn remember(&mut self, _call: &str) {}
    fn lookup(&self, _size: HashSize, _hash: u32) -> Option<Call> {
        None
    }
}

/// A fixed-capacity table of the `N` most recently heard callsigns.
///
/// Remembering a call that is already present refreshes it; when the
/// table is full, the call heard longest ago is replaced. A lookup returns
/// the most recently heard call with that hash. No allocation: the table
/// is `N` entries reserved up front.
#[derive(Debug)]
pub struct CallTable<const N: usize> {
    calls: [Call; N],
    h22: [u32; N],
    /// When each entry was last heard (a counter, not a time).
    seen: [u64; N],
    len: usize,
    clock: u64,
}

impl<const N: usize> CallTable<N> {
    /// An empty table.
    pub const fn new() -> Self {
        CallTable {
            calls: [Call::new(); N],
            h22: [0; N],
            seen: [0; N],
            len: 0,
            clock: 0,
        }
    }

    /// Number of callsigns held.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when the table holds no callsigns.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const N: usize> Default for CallTable<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> CallHashes for CallTable<N> {
    fn remember(&mut self, call: &str) {
        let (Some(h), Some(c)) = (hash22(call), Call::from_ascii(call)) else {
            return;
        };
        if N == 0 {
            return;
        }
        self.clock += 1;
        if let Some(i) = (0..self.len).find(|&i| self.h22[i] == h && self.calls[i] == c) {
            self.seen[i] = self.clock;
            return;
        }
        let slot = if self.len < N {
            self.len += 1;
            self.len - 1
        } else {
            // Full: replace the entry heard longest ago.
            (0..N).min_by_key(|&i| self.seen[i]).unwrap_or(0)
        };
        self.calls[slot] = c;
        self.h22[slot] = h;
        self.seen[slot] = self.clock;
    }

    fn lookup(&self, size: HashSize, hash: u32) -> Option<Call> {
        (0..self.len)
            .filter(|&i| size.from_h22(self.h22[i]) == hash)
            .max_by_key(|&i| self.seen[i])
            .map(|i| self.calls[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_lengths_nest() {
        let h = hash22("PJ4/K1ABC").unwrap();
        assert!(h < 1 << 22);
        assert_eq!(hash("PJ4/K1ABC", HashSize::Bits12), Some(h >> 10));
        assert_eq!(hash("PJ4/K1ABC", HashSize::Bits10), Some(h >> 12));
    }

    #[test]
    fn rejects_bad_calls() {
        assert_eq!(hash22(""), None);
        assert_eq!(hash22("ABCDEFGHIJKL"), None);
        assert_eq!(hash22("K1ABC-1"), None);
    }

    #[test]
    fn re_hearing_a_call_keeps_it() {
        // K1ABC, W9XYZ, K1ABC, G4ABC into two slots: W9XYZ (heard longest
        // ago) is evicted, not K1ABC.
        let mut t: CallTable<2> = CallTable::new();
        for c in ["K1ABC", "W9XYZ", "K1ABC", "G4ABC"] {
            t.remember(c);
        }
        let find = |c: &str| t.lookup(HashSize::Bits22, hash22(c).unwrap());
        assert_eq!(find("K1ABC").unwrap(), "K1ABC");
        assert_eq!(find("G4ABC").unwrap(), "G4ABC");
        assert_eq!(find("W9XYZ"), None);
    }

    #[test]
    fn lookup_prefers_the_most_recent_of_colliding_calls() {
        // K1AM and K1MB share a 10-bit hash (348) but not a 22-bit one.
        let (a, b) = ("K1AM", "K1MB");
        let h10 = hash(a, HashSize::Bits10).unwrap();
        assert_eq!(h10, 348);
        assert_eq!(hash(b, HashSize::Bits10), Some(h10));
        assert_ne!(hash22(a), hash22(b));

        let mut t: CallTable<4> = CallTable::new();
        t.remember(a);
        t.remember(b);
        assert_eq!(t.lookup(HashSize::Bits10, h10).unwrap(), b);
        t.remember(a); // heard again: now the newest
        assert_eq!(t.lookup(HashSize::Bits10, h10).unwrap(), a);
        // The 22-bit hashes still find each call exactly.
        assert_eq!(t.lookup(HashSize::Bits22, hash22(b).unwrap()).unwrap(), b);
    }

    #[test]
    fn table_finds_newest_and_evicts_oldest() {
        let mut t: CallTable<2> = CallTable::new();
        t.remember("K1ABC");
        t.remember("W9XYZ");
        t.remember("W9XYZ");
        assert_eq!(t.len(), 2);
        let h = hash22("K1ABC").unwrap();
        assert_eq!(t.lookup(HashSize::Bits22, h).unwrap(), "K1ABC");
        t.remember("PJ4/K1ABC");
        assert_eq!(t.lookup(HashSize::Bits22, h), None);
        let h12 = hash("PJ4/K1ABC", HashSize::Bits12).unwrap();
        assert_eq!(t.lookup(HashSize::Bits12, h12).unwrap(), "PJ4/K1ABC");
    }
}
