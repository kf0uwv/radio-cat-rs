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

//! Seeded property tests for message packing.
//!
//! 1. Round trips: random valid messages of every supported type pack and
//!    unpack back to the same text.
//! 2. Payload fuzz: random payloads never make `unpack` panic, and every
//!    payload that unpacks to text without an unresolved hash packs back to
//!    the same bits, except in the non-canonical cases named in
//!    [`non_canonical`] (bit patterns that `pack` never produces but that
//!    unpack to the same text as a canonical one).

use std::collections::BTreeMap;

use cat_ft8::message::{
    hash22, pack, unpack, CallHashes, CallTable, MessageType, Payload, MAX22, MAXGRID4, NTOKENS,
    STD_CALLS,
};

/// splitmix64: a small seeded generator, so the tests need no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + self.below((hi - lo + 1) as u64) as i64
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len() as u64) as usize]
    }
    fn letters(&mut self, n: usize, alphabet: &[u8]) -> String {
        (0..n)
            .map(|_| alphabet[self.below(alphabet.len() as u64) as usize] as char)
            .collect()
    }
}

const AZ: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const AR: &[u8] = b"ABCDEFGHIJKLMNOPQR";
const AX: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWX";
const D: &[u8] = b"0123456789";

fn std_call(r: &mut Rng) -> String {
    let prefix = r.pick(&[
        "K", "W", "N", "AA", "KD", "JA", "G", "M", "DL", "F", "VK", "9A", "S5", "4X", "A",
    ]);
    let suffix_len = 1 + r.below(3) as usize;
    format!("{prefix}{}{}", r.below(10), r.letters(suffix_len, AZ))
}

fn compound(r: &mut Rng) -> String {
    format!(
        "{}/{}",
        r.pick(&["PJ4", "KH6", "VP2E", "FO", "3D2", "HB0"]),
        std_call(r)
    )
}

fn special(r: &mut Rng) -> String {
    let (prefix, num, n) = (
        r.pick(&["YW", "GB", "TM"]),
        r.range(10, 99),
        2 + r.below(3) as usize,
    );
    format!("{prefix}{num}{}", r.letters(n, AZ))
}

fn grid4(r: &mut Rng) -> String {
    format!("{}{}", r.letters(2, AR), r.letters(2, D))
}

fn report(r: &mut Rng, lo: i64, hi: i64) -> String {
    format!("{:+03}", r.range(lo, hi))
}

/// One random valid message of the given class, as `pack` would print it.
fn message(r: &mut Rng, class: usize) -> String {
    match class {
        // Type 1: CQ forms, calls with /R, hashed calls, every report.
        0 => {
            let first = match r.below(5) {
                0 => "CQ".to_string(),
                1 => format!("CQ {:03}", r.below(1000)),
                2 => {
                    let n = 1 + r.below(4) as usize;
                    format!("CQ {}", r.letters(n, AZ))
                }
                3 => std_call(r) + if r.below(3) == 0 { "/R" } else { "" },
                _ => r.pick(&["DE", "QRZ"]).to_string(),
            };
            let second = std_call(r) + if r.below(4) == 0 { "/R" } else { "" };
            let extra = match r.below(7) {
                0 => String::new(),
                1 => format!(" {}", grid4(r)),
                2 => format!(" R {}", grid4(r)),
                3 => format!(" {}", report(r, -50, 50)),
                4 => format!(" R{}", report(r, -50, 50)),
                _ => format!(" {}", r.pick(&["RRR", "RR73", "73"])),
            };
            format!("{first} {second}{extra}")
        }
        // Type 1 with one hashed call beside a plain standard call.
        1 => {
            let h = if r.below(2) == 0 {
                compound(r)
            } else {
                special(r)
            };
            let (a, b) = if r.below(2) == 0 {
                (format!("<{h}>"), std_call(r))
            } else {
                (std_call(r), format!("<{h}>"))
            };
            format!("{a} {b} {}", report(r, -50, 50))
        }
        // Type 2.
        2 => format!("{}/P {} {}", std_call(r), std_call(r), grid4(r)),
        // Type 4.
        3 => match r.below(3) {
            0 => format!(
                "CQ {}",
                if r.below(2) == 0 {
                    compound(r)
                } else {
                    special(r)
                }
            ),
            1 => format!(
                "<{}> {} {}",
                std_call(r),
                compound(r),
                r.pick(&["RRR", "RR73", "73"])
            ),
            _ => format!("{} <{}>", special(r), std_call(r)),
        },
        // 0.0 free text (not a single hex word, no word that makes it
        // another type: lower-case-free random from the alphabet).
        4 => loop {
            let n = 1 + r.below(13) as usize;
            let s = r.letters(n, b" 0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ+-./?");
            let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
            let hex_word = !s.contains(' ') && s.bytes().all(|c| c.is_ascii_hexdigit());
            if !s.is_empty()
                && !hex_word
                && pack(&s, &mut CallTable::<1>::new())
                    .map(|p| p.i3() == 0 && p.n3() == 0)
                    .unwrap_or(false)
            {
                break s;
            }
        },
        // 0.5 telemetry.
        5 => {
            let n = 1 + r.below(18) as usize;
            let first = if n == 18 {
                r.letters(1, b"1234567")
            } else {
                r.letters(1, b"123456789ABCDEF")
            };
            first + &r.letters(n - 1, b"0123456789ABCDEF")
        }
        // 0.1 DXpedition (even reports round-trip exactly).
        6 => format!(
            "{} RR73; {} <{}> {:+03}",
            std_call(r),
            std_call(r),
            compound(r),
            2 * r.range(-15, 16)
        ),
        // 5 EU VHF contest.
        _ => format!(
            "<{}> <{}> {}5{}{:04} {}{}",
            std_call(r),
            std_call(r),
            if r.below(2) == 0 { "R " } else { "" },
            r.range(2, 9),
            r.below(2048),
            grid4(r),
            r.letters(2, AX)
        ),
    }
}

/// True if two different calls in `m` share a 10-bit hash (so possibly
/// a 12- or 22-bit one too): the protocol cannot tell them apart, and a
/// hashed field may show the other call.
fn calls_collide(m: &str) -> bool {
    let calls: Vec<&str> = m
        .split(' ')
        .map(|w| w.trim_start_matches('<').trim_end_matches('>'))
        .filter(|w| {
            w.bytes().any(|c| c.is_ascii_digit()) && w.bytes().any(|c| c.is_ascii_uppercase())
        })
        .filter(|w| hash22(w).is_some())
        .collect();
    calls.iter().enumerate().any(|(i, a)| {
        calls[i + 1..]
            .iter()
            .any(|b| a != b && hash22(a).map(|h| h >> 12) == hash22(b).map(|h| h >> 12))
    })
}

#[test]
fn random_messages_round_trip() {
    let mut r = Rng(0x5eed_0001);
    let mut per_type: BTreeMap<String, usize> = BTreeMap::new();
    let mut collisions = 0;
    for class in 0..8 {
        for _ in 0..2_000 {
            let m = message(&mut r, class);
            if calls_collide(&m) {
                collisions += 1;
                continue;
            }
            let mut t: CallTable<8> = CallTable::new();
            let p = pack(&m, &mut t).unwrap_or_else(|e| panic!("pack {m:?}: {e}"));
            let back = unpack(&p, &mut t).unwrap_or_else(|e| panic!("unpack {m:?}: {e}"));
            assert_eq!(back.as_str(), m, "class {class}");
            *per_type.entry(p.message_type().to_string()).or_default() += 1;
        }
    }
    println!("round trips by type: {per_type:?}; skipped for 10-bit hash collisions: {collisions}");
    assert!(collisions < 40, "{collisions}");
    for t in ["0.0", "0.1", "0.5", "1", "2", "4", "5"] {
        assert!(per_type.get(t).copied().unwrap_or(0) >= 1_000, "{t}");
    }
}

/// Field values that unpack to text `pack` writes differently. Returns the
/// specific reason, or `None` if the payload should repack exactly.
fn non_canonical(p: &Payload, text: &str) -> Option<&'static str> {
    let b = p.bits();
    let field = |from_end: u32, width: u32| ((b >> from_end) & ((1u128 << width) - 1)) as u32;
    let is_call = |c28: u32| c28 >= NTOKENS + MAX22;
    let is_hash = |c28: u32| (NTOKENS..NTOKENS + MAX22).contains(&c28);
    let words: Vec<&str> = text.split(' ').collect();
    match p.message_type() {
        MessageType::Standard | MessageType::EuVhfPortable => {
            let (a, ra) = (field(49, 28), field(48, 1));
            let (b2, rb) = (field(20, 28), field(19, 1));
            let (r, g) = (field(18, 1), field(3, 15));
            if (ra == 1 && !is_call(a)) || (rb == 1 && !is_call(b2)) {
                return Some("suffix bit on a token or hash");
            }
            if !is_call(b2) && !is_hash(b2) {
                return Some("token in the second call field");
            }
            if !is_call(a) && !is_hash(a) && is_hash(b2) {
                return Some("token before a hashed call (type 4 form, or not packable)");
            }
            if p.i3() == 2 && ra == 0 && rb == 0 {
                return Some("type 2 without /P");
            }
            let compound = |w: &str| {
                let c = w.trim_matches(|c| c == '<' || c == '>');
                c.contains('/') && !c.ends_with("/R") && !c.ends_with("/P")
            };
            if (is_hash(a) && rb == 1) || (is_hash(b2) && ra == 1) {
                return Some("hashed call beside a /R or /P call (type 4 form)");
            }
            if is_hash(a) && is_hash(b2) && (compound(words[0]) || compound(words[1])) {
                return Some("two hashed calls, one compound (type 4 form)");
            }
            if g == MAXGRID4 + 3 {
                return Some("reserved RR73 value (sent as locator RR73)");
            }
            if r == 1 && (MAXGRID4 + 1..=MAXGRID4 + 4).contains(&g) {
                return Some("R with blank/RRR/RR73/73");
            }
            if g > MAXGRID4 + 105 {
                return Some("report beyond -50..+50 (shown wrapped)");
            }
            None
        }
        MessageType::DXpedition => {
            let (a, b2) = (field(49, 28), field(21, 28));
            if (!is_call(a) && !is_hash(a)) || (!is_call(b2) && !is_hash(b2)) {
                return Some("token in a DXpedition call field");
            }
            None
        }
        MessageType::NonStandardCall => {
            let (h1, c1) = (field(6, 1) == 1, field(3, 1) == 1);
            let plain = match (c1, h1) {
                (true, _) => words[1],
                (false, true) => words[0],
                (false, false) => words[1],
            };
            let std_no_suffix = |c: &str| {
                let mut t: CallTable<1> = CallTable::new();
                pack(&format!("{c} K1ABC"), &mut t)
                    .map(|q| q.i3() == 1)
                    .unwrap_or(false)
                    && !c.contains('/')
            };
            let callsign = |c: &str| {
                (3..=11).contains(&c.len())
                    && c.bytes().any(|x| x.is_ascii_digit())
                    && c.bytes().any(|x| x.is_ascii_uppercase())
            };
            if !callsign(plain) {
                return Some("type 4 full call that is not a callsign");
            }
            if ((b >> 7) & ((1u128 << 58) - 1)) % 38 == 0 {
                return Some("type 4 full call with trailing spaces (sent right-aligned)");
            }
            if c1 {
                if field(4, 3) != 0 {
                    return Some("type 4 CQ with h1/r2 bits set");
                }
                if hash22(plain).map(|h| h >> 10) != Some(field(65, 12)) {
                    return Some("type 4 CQ whose h12 is not the call's hash");
                }
                if std_no_suffix(plain) || plain.ends_with("/R") || plain.ends_with("/P") {
                    return Some("type 4 CQ with a standard call (sent as type 1/2)");
                }
                return None;
            }
            if std_no_suffix(plain) {
                return Some("type 4 whose full call is a standard call (type 1 form)");
            }
            None
        }
        MessageType::FreeText => {
            // The 13 characters as sent (base 42, first character most
            // significant).
            const FREE: &[u8] = b" 0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ+-./?";
            let mut v = b >> 6;
            let mut raw = [b' '; 13];
            for c in raw.iter_mut().rev() {
                *c = FREE[(v % 42) as usize];
                v /= 42;
            }
            let raw = std::str::from_utf8(&raw).unwrap();
            let mut t: CallTable<1> = CallTable::new();
            match pack(text, &mut t) {
                Err(_) => Some("blank free text"),
                Ok(q) if q.message_type() != MessageType::FreeText => {
                    Some("free text that reads as another type")
                }
                _ if raw.trim_start().contains("  ") => Some("free text with repeated spaces"),
                _ if raw.ends_with(' ') => Some("free text with trailing spaces"),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A random call for a hashed field: standard, compound or special-event.
fn any_call(r: &mut Rng) -> String {
    match r.below(3) {
        0 => std_call(r),
        1 => compound(r),
        _ => special(r),
    }
}

/// The `c58` value of a call (base 38, right-aligned).
fn c58(call: &str) -> u128 {
    const A: &[u8] = b" 0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ/";
    call.bytes().fold(0u128, |n, c| {
        n * 38 + A.iter().position(|&x| x == c).unwrap() as u128
    })
}

/// Sets `width` bits at `from_end` (counted from the payload's last bit).
fn put(bits: u128, from_end: u32, width: u32, v: u128) -> u128 {
    let mask = ((1u128 << width) - 1) << from_end;
    (bits & !mask) | ((v << from_end) & mask)
}

/// A random payload of the given supported type whose hash fields hold
/// hashes of real calls, which are returned so the table can be seeded
/// with them (as a receiver would have heard them).
fn structured(r: &mut Rng, bits: u128, calls: &mut Vec<String>) -> u128 {
    let h = |c: &str| u128::from(hash22(c).unwrap());
    let c28 = |r: &mut Rng, calls: &mut Vec<String>| -> u128 {
        match r.below(10) {
            0..=6 => u128::from(NTOKENS + MAX22) + u128::from(r.below(u64::from(STD_CALLS)) as u32),
            7 | 8 => {
                let c = any_call(r);
                let v = u128::from(NTOKENS) + h(&c);
                calls.push(c);
                v
            }
            _ => u128::from(r.below(1 << 28) as u32),
        }
    };
    let (i3, n3) = [(0, 0), (0, 1), (0, 5), (1, 0), (2, 0), (4, 0), (5, 0)][r.below(7) as usize];
    let mut bits = (bits & !0x3f) | (n3 << 3) | i3;
    match (i3, n3) {
        (1, _) | (2, _) => {
            bits = put(bits, 49, 28, c28(r, calls));
            bits = put(bits, 20, 28, c28(r, calls));
        }
        (0, 1) => {
            bits = put(bits, 49, 28, c28(r, calls));
            bits = put(bits, 21, 28, c28(r, calls));
            let fox = any_call(r);
            bits = put(bits, 11, 10, h(&fox) >> 12);
            calls.push(fox);
        }
        (4, _) => {
            let other = any_call(r);
            bits = put(bits, 65, 12, h(&other) >> 10);
            calls.push(other);
            if r.below(2) == 0 {
                let full = any_call(r);
                bits = put(bits, 7, 58, c58(&full));
            }
        }
        (5, _) => {
            let (x, y) = (any_call(r), any_call(r));
            bits = put(bits, 65, 12, h(&x) >> 10);
            bits = put(bits, 43, 22, h(&y));
            calls.push(x);
            calls.push(y);
        }
        _ => {}
    }
    bits
}

#[test]
fn payload_fuzz() {
    let mut r = Rng(0xf022_0001);
    let mut ok = 0usize;
    let mut exact = 0usize;
    let mut unresolved = 0usize;
    let mut exact_by_type: BTreeMap<String, usize> = BTreeMap::new();
    let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
    let mut errors: BTreeMap<String, usize> = BTreeMap::new();
    for k in 0..200_000u32 {
        // Half uniform 77-bit payloads (hashes there are unknown to the
        // table); half of a supported type with the table seeded with the
        // calls behind its hash fields.
        let mut bits = (u128::from(r.next()) << 64 | u128::from(r.next())) & ((1u128 << 77) - 1);
        let mut calls = Vec::new();
        if k % 2 == 0 {
            bits = structured(&mut r, bits, &mut calls);
        }
        let p = Payload::from_bits(bits).unwrap();
        let mut t: CallTable<8> = CallTable::new();
        for c in &calls {
            t.remember(c);
        }
        let text = match unpack(&p, &mut t) {
            Ok(text) => text,
            Err(e) => {
                *errors.entry(e.to_string()).or_default() += 1;
                continue;
            }
        };
        ok += 1;
        if text.contains("<...>") {
            unresolved += 1; // a hash no call in the table has
            continue;
        }
        let mut t2: CallTable<8> = CallTable::new();
        match pack(&text, &mut t2) {
            Ok(q) if q == p => {
                exact += 1;
                *exact_by_type
                    .entry(p.message_type().to_string())
                    .or_default() += 1;
            }
            _ => match non_canonical(&p, &text) {
                Some(reason) => *reasons.entry(reason).or_default() += 1,
                None => panic!(
                    "{:077b} unpacks to {:?} but packs to {:?}",
                    bits,
                    text.as_str(),
                    pack(&text, &mut t2)
                ),
            },
        }
    }
    println!("fuzz: {ok} unpacked ({exact} exact repack, {unresolved} with unresolved hashes)");
    println!("exact repack by type: {exact_by_type:?}");
    println!("non-canonical: {reasons:?}");
    println!("unpack errors: {errors:?}");
    // Every supported type, hashed ones included, reaches the repack check.
    for t in ["0.0", "0.1", "0.5", "1", "2", "4", "5"] {
        assert!(
            exact_by_type.get(t).copied().unwrap_or(0) >= 1_000,
            "{t}: {exact_by_type:?}"
        );
    }
}
