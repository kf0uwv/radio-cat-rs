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

//! Callsigns: validation, generation from real prefix structure, and
//! where a call is "from".

use std::fmt;

use super::data::{CallArea, DxEntity, DX, LETTERS, RESERVED_CALLS, SUFFIX_FIRST, US_AREAS};
use super::ScenarioError;
use crate::rng::Rng;

/// A syntactically valid amateur callsign, upper case, possibly with a
/// `/`-separated prefix or suffix (`KF0UWV/P`, `VE3/W1XYZ`).
///
/// With the `serde` feature it serializes as its text, and deserializing
/// goes through [`Callsign::parse`], so an invalid call cannot be smuggled
/// in from a file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "String", into = "String")
)]
pub struct Callsign(String);

impl TryFrom<String> for Callsign {
    type Error = ScenarioError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Callsign::parse(&s)
    }
}

impl From<Callsign> for String {
    fn from(c: Callsign) -> String {
        c.0
    }
}

impl Callsign {
    /// Validates `s`: 3–14 characters of `A–Z`, `0–9` and `/`; at least
    /// one letter and one digit; no leading, trailing or doubled `/`.
    /// Lower case is accepted and folded.
    pub fn parse(s: &str) -> Result<Self, ScenarioError> {
        let up = s.trim().to_ascii_uppercase();
        let bad = || ScenarioError::InvalidCallsign(s.to_string());
        if !(3..=14).contains(&up.len()) {
            return Err(bad());
        }
        if !up
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'/')
        {
            return Err(bad());
        }
        if up.starts_with('/') || up.ends_with('/') || up.contains("//") {
            return Err(bad());
        }
        let base = base_part(&up);
        if !base.bytes().any(|b| b.is_ascii_digit())
            || !base.bytes().any(|b| b.is_ascii_uppercase())
        {
            return Err(bad());
        }
        Ok(Callsign(up))
    }

    /// The callsign text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The part without any `/P`-style suffix or `VE3/`-style prefix:
    /// the longest `/`-separated piece.
    pub fn base(&self) -> &str {
        base_part(&self.0)
    }

    /// The call with a portable designator, e.g. `/P`.
    pub(crate) fn with_suffix(&self, suffix: &str) -> Callsign {
        Callsign(format!("{}/{}", self.0, suffix))
    }
}

fn base_part(s: &str) -> &str {
    s.split('/').max_by_key(|p| p.len()).unwrap_or(s)
}

impl fmt::Display for Callsign {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a callsign is from, as far as a QSO needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Home {
    Us(&'static CallArea),
    Dx(&'static DxEntity),
}

#[cfg(test)]
impl Home {
    fn zone(&self) -> u8 {
        match self {
            Home::Us(a) => a.zone,
            Home::Dx(e) => e.zone,
        }
    }
}

impl PartialEq for CallArea {
    fn eq(&self, other: &Self) -> bool {
        self.digit == other.digit
    }
}
impl Eq for CallArea {}
impl fmt::Debug for CallArea {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CallArea({})", self.digit)
    }
}
impl PartialEq for DxEntity {
    fn eq(&self, other: &Self) -> bool {
        self.country == other.country
    }
}
impl Eq for DxEntity {}
impl fmt::Debug for DxEntity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DxEntity({})", self.country)
    }
}

fn is_us_prefix(p: &str) -> bool {
    let b = p.as_bytes();
    match b {
        [k] => matches!(k, b'K' | b'N' | b'W'),
        [a, c] => {
            (matches!(a, b'K' | b'N' | b'W') && c.is_ascii_uppercase())
                || (*a == b'A' && (b'A'..=b'L').contains(c))
        }
        _ => false,
    }
}

/// Infers where `call` is from by its prefix. Unknown prefixes return
/// `None`; the caller picks a generic home.
pub(crate) fn home_of(call: &Callsign) -> Option<Home> {
    let base = call.base();
    let digit_at = base.find(|c: char| c.is_ascii_digit())?;
    // Longest DX prefix match first (HB9 before H, 9A before 9).
    let mut best: Option<(&'static DxEntity, usize)> = None;
    for e in DX {
        for p in e.prefixes {
            if base.starts_with(p) && best.map_or(true, |(_, l)| p.len() > l) {
                best = Some((e, p.len()));
            }
        }
    }
    if let Some((e, _)) = best {
        return Some(Home::Dx(e));
    }
    let prefix = &base[..digit_at];
    if is_us_prefix(prefix) {
        let d = base.as_bytes()[digit_at] - b'0';
        return US_AREAS.iter().find(|a| a.digit == d).map(Home::Us);
    }
    None
}

fn letters(rng: &mut Rng, n: u32) -> String {
    let mut s = String::new();
    for i in 0..n {
        let set = if i == 0 { SUFFIX_FIRST } else { LETTERS };
        s.push(*rng.pick(set) as char);
    }
    s
}

/// A US call of a real format: 1x2, 2x1, 2x2, 1x3 or 2x3, weighted
/// towards the common 1x3 and 2x3. Occasionally Alaska or Hawaii.
pub(crate) fn us_call(rng: &mut Rng, area: Option<&CallArea>) -> String {
    let one = [b'K', b'N', b'W'];
    let two_first = [b'K', b'N', b'W', b'A'];
    let digit = area.map_or_else(|| rng.range_u32(0, 9), |a| u32::from(a.digit));
    let two = |rng: &mut Rng, kw_only: bool| -> String {
        let first = if kw_only {
            *rng.pick(b"KW")
        } else {
            *rng.pick(&two_first)
        };
        let second = if first == b'A' {
            b'A' + rng.range_u32(0, 11) as u8
        } else {
            *rng.pick(LETTERS)
        };
        format!("{}{}", first as char, second as char)
    };
    let roll = rng.range_u32(0, 99);
    let (prefix, n) = match roll {
        0..=9 => ((*rng.pick(&one) as char).to_string(), 2),
        10..=14 => (two(rng, false), 1),
        15..=29 => (two(rng, false), 2),
        30..=64 => ((*rng.pick(&one) as char).to_string(), 3),
        _ => (two(rng, true), 3),
    };
    format!("{prefix}{digit}{}", letters(rng, n))
}

/// A call from DX entity `e`.
pub(crate) fn dx_call(rng: &mut Rng, e: &DxEntity) -> String {
    let p = rng.pick(e.prefixes);
    let digit = if e.add_digit {
        rng.range_u32(1, 9).to_string()
    } else {
        String::new()
    };
    let n = rng.range_u32(e.suffix_len.0, e.suffix_len.1);
    format!("{p}{digit}{}", letters(rng, n))
}

/// A fresh call not equal to `my_call` or any in `taken`, nor reserved.
pub(crate) fn fresh_call(
    rng: &mut Rng,
    make: &mut dyn FnMut(&mut Rng) -> String,
    my_call: &Callsign,
    taken: &[Callsign],
) -> Callsign {
    loop {
        let s = make(rng);
        if RESERVED_CALLS.contains(&s.as_str()) {
            continue;
        }
        let c = Callsign::parse(&s).expect("generated calls are well formed");
        if c.base() == my_call.base() || taken.iter().any(|t| t.base() == c.base()) {
            continue;
        }
        return c;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_real_shapes() {
        for c in [
            "KF0UWV",
            "w1aw",
            "VE3/W1XYZ",
            "KF0UWV/P",
            "HB9ABC",
            "9A2AB",
            "N0A",
        ] {
            assert!(Callsign::parse(c).is_ok(), "{c}");
        }
        assert_eq!(Callsign::parse("kf0uwv").unwrap().as_str(), "KF0UWV");
        assert_eq!(Callsign::parse("KF0UWV/P").unwrap().base(), "KF0UWV");
    }

    #[test]
    fn parse_rejects_garbage() {
        for c in [
            "", "AB", "ABCDEF", "123", "K F0UWV", "/K1AB", "K1AB/", "K1//AB", "K1AB#",
        ] {
            assert!(Callsign::parse(c).is_err(), "{c}");
        }
    }

    #[test]
    fn us_calls_follow_fcc_formats() {
        let mut rng = Rng::new(1);
        for _ in 0..2000 {
            let s = us_call(&mut rng, None);
            let d = s.find(|c: char| c.is_ascii_digit()).unwrap();
            let (pre, suf) = (&s[..d], &s[d + 1..]);
            assert!(is_us_prefix(pre), "{s}");
            assert!(
                matches!(
                    (pre.len(), suf.len()),
                    (1, 2) | (2, 1) | (2, 2) | (1, 3) | (2, 3)
                ),
                "{s}"
            );
            assert!(!suf.starts_with('Q') && !suf.starts_with('X'), "{s}");
            assert!(Callsign::parse(&s).is_ok());
        }
    }

    #[test]
    fn home_is_inferred_from_prefix() {
        let h = |c: &str| home_of(&Callsign::parse(c).unwrap());
        assert_eq!(h("KF0UWV").unwrap().zone(), 4);
        assert_eq!(h("W6ABC").unwrap().zone(), 3);
        assert!(matches!(h("HB9XYZ"), Some(Home::Dx(e)) if e.country == "SWITZERLAND"));
        assert!(matches!(h("DL1ABC"), Some(Home::Dx(e)) if e.country == "GERMANY"));
        assert!(matches!(h("AA1A"), Some(Home::Us(_))));
        assert!(h("4X1AB").is_none());
    }
}
