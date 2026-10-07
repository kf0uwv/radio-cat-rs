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

use super::data::{LETTERS, RESERVED_CALLS, SUFFIX_FIRST};
use super::world::{by_dxcc, CallFormat, DigitRule, Entity, Region, USA, WORLD};
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

/// Where a callsign is from, as far as a QSO needs to know: its DXCC
/// entity and, where the entity has call areas, which one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Home {
    pub entity: &'static Entity,
    pub region: Option<&'static Region>,
}

impl Home {
    /// CQ zone: the region's if known, else the entity's.
    pub(crate) fn cq(&self) -> u8 {
        self.region.map_or(self.entity.cq, |r| r.cq)
    }

    /// A home in `entity`, in a random region if it has any.
    pub(crate) fn random_in(rng: &mut Rng, entity: &'static Entity) -> Home {
        let region = if entity.regions.is_empty() {
            None
        } else {
            Some(rng.pick(entity.regions))
        };
        Home { entity, region }
    }

    /// A home somewhere in the US.
    pub(crate) fn random_us(rng: &mut Rng) -> Home {
        Home::random_in(rng, by_dxcc(USA))
    }

    /// A home anywhere outside the US.
    pub(crate) fn random_dx(rng: &mut Rng) -> Home {
        let dx: Vec<&'static Entity> = WORLD.iter().filter(|e| e.dxcc != USA).collect();
        let e = *rng.pick(&dx);
        Home::random_in(rng, e)
    }
}

/// `/`-pieces that say how a station operates, not where.
const MODIFIERS: &[&str] = &["P", "M", "A", "B", "QRP"];
/// `/`-pieces that put a station in no DXCC entity at all.
const NO_ENTITY: &[&str] = &["MM", "AM"];

/// The entity whose prefix is the longest one `piece` starts with.
/// Tagged entities (`3D2AB/R`) are found by their tag, not here.
fn entity_of(piece: &str) -> Option<&'static Entity> {
    let mut best: Option<(&'static Entity, usize)> = None;
    for e in WORLD.iter().filter(|e| e.tag.is_none()) {
        for p in e.prefixes {
            if piece.starts_with(p) && best.map_or(true, |(_, l)| p.len() > l) {
                best = Some((e, p.len()));
            }
        }
    }
    best.map(|(e, _)| e)
}

fn first_digit(piece: &str) -> Option<u8> {
    piece.bytes().find(u8::is_ascii_digit).map(|b| b - b'0')
}

/// The tagged entity `tag` names for a call whose base is `base`.
fn tagged(base: &str, tag: &str) -> Option<&'static Entity> {
    WORLD
        .iter()
        .find(|e| e.tag == Some(tag) && e.prefixes.iter().any(|p| base.starts_with(p)))
}

/// Infers where `call` is from by its prefix, longest match first (`HB9`
/// before `H`, `9A` before `9`).
///
/// An entity that shares its parent's block is named by a tag: `3D2AB/R`
/// is Rotuma, `SV2ASP/A` Mount Athos. A tag is checked before the piece is
/// read as a modifier, so `/A` only means "alternate address" off that block.
///
/// A location designator wins over the home call: `EA8/DL1ABC` is in the
/// Canaries, `W1ABC/KH6` in Hawaii, `W1ABC/4` in US call area 4. Operating
/// modifiers (`/P`, `/M`, `/A`, `/QRP`) are ignored; `/MM` and `/AM` are in
/// no entity. Unknown prefixes return `None`; the caller picks a generic
/// home.
pub(crate) fn home_of(call: &Callsign) -> Option<Home> {
    let base = call.base();
    let mut designator: Option<&str> = None;
    for piece in call.as_str().split('/') {
        if std::ptr::eq(piece, base) {
            continue;
        }
        if let Some(entity) = tagged(base, piece) {
            return Some(Home {
                entity,
                region: None,
            });
        }
        if MODIFIERS.contains(&piece) {
            continue;
        }
        if NO_ENTITY.contains(&piece) {
            return None;
        }
        designator = Some(piece);
    }
    let (entity, digit_from) = match designator {
        Some(d) if d.bytes().all(|b| b.is_ascii_digit()) => (entity_of(base)?, d),
        Some(d) => (entity_of(d)?, d),
        None => (entity_of(base)?, base),
    };
    let region = first_digit(digit_from).and_then(|d| entity.region(d));
    Some(Home { entity, region })
}

fn letters(rng: &mut Rng, n: u32) -> String {
    let mut s = String::new();
    for i in 0..n {
        let set = if i == 0 { SUFFIX_FIRST } else { LETTERS };
        s.push(*rng.pick(set) as char);
    }
    s
}

/// One of `items`, drawn in proportion to `weight`.
fn pick_weighted<'a, T>(rng: &mut Rng, items: &'a [T], weight: impl Fn(&T) -> u32) -> &'a T {
    let total: u32 = items.iter().map(&weight).sum();
    let mut roll = rng.range_u32(0, total - 1);
    for it in items {
        let w = weight(it);
        if roll < w {
            return it;
        }
        roll -= w;
    }
    items.last().expect("non-empty")
}

/// A call of format `f`, from `home`.
fn call_in_format(rng: &mut Rng, f: &CallFormat, home: &Home) -> String {
    let p = rng.pick(f.prefixes);
    let digit = match f.digit {
        DigitRule::InPrefix => String::new(),
        DigitRule::OneOf(d) => rng.pick(d).to_string(),
        DigitRule::FromRegion => home
            .region
            .unwrap_or_else(|| rng.pick(home.entity.regions))
            .digit
            .to_string(),
    };
    let n = rng.range_u32(f.suffix.0, f.suffix.1);
    let mut s = format!("{p}{digit}{}", letters(rng, n));
    if let Some(tag) = home.entity.tag {
        s.push('/');
        s.push_str(tag);
    }
    s
}

/// Draws for [`call_from`] before it settles for a near miss.
const CALL_TRIES: usize = 64;

/// A call of a real format for `home`'s entity (and call area).
///
/// Formats are drawn from whole blocks, and a few blocks have carve-outs
/// that belong to another entity (`KH7K` Kure inside Hawaii's `KH7`,
/// `KC4AA` Antarctica inside the US `KC`). A draw that lands in one is
/// redrawn, so the call's home is `home`.
pub(crate) fn call_from(rng: &mut Rng, home: &Home) -> String {
    let mut s = String::new();
    for _ in 0..CALL_TRIES {
        let f = pick_weighted(rng, home.entity.formats, |f| f.weight);
        s = call_in_format(rng, f, home);
        let back = Callsign::parse(&s).ok().and_then(|c| home_of(&c));
        if back == Some(*home) {
            break;
        }
    }
    s
}

/// Draws [`fresh_call`] makes before deciding a home has no call left.
const FRESH_TRIES: usize = 256;

/// A fresh call not equal to `my_call` or any in `taken`, nor reserved;
/// `None` if `make` keeps producing taken calls (a one-call entity such as
/// `4U1UN` that is already on the air).
pub(crate) fn fresh_call(
    rng: &mut Rng,
    make: &mut dyn FnMut(&mut Rng) -> String,
    my_call: &Callsign,
    taken: &[Callsign],
) -> Option<Callsign> {
    for _ in 0..FRESH_TRIES {
        let s = make(rng);
        if RESERVED_CALLS.contains(&s.as_str()) {
            continue;
        }
        let c = Callsign::parse(&s).expect("generated calls are well formed");
        if c.base() == my_call.base() || taken.iter().any(|t| t.base() == c.base()) {
            continue;
        }
        return Some(c);
    }
    None
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

    /// A lower-48 US prefix: `K`/`N`/`W`, or two letters of those plus
    /// any letter but `H`/`L`/`P`, or `AA`–`AK` less `AH`.
    fn is_us_prefix(p: &str) -> bool {
        match p.as_bytes() {
            [k] => matches!(k, b'K' | b'N' | b'W'),
            [a, c] => {
                (matches!(a, b'K' | b'N' | b'W')
                    && c.is_ascii_uppercase()
                    && !matches!(c, b'H' | b'L' | b'P'))
                    || (*a == b'A' && (b'A'..=b'K').contains(c) && *c != b'H')
            }
            _ => false,
        }
    }

    #[test]
    fn us_calls_follow_fcc_formats() {
        let mut rng = Rng::new(1);
        for _ in 0..2000 {
            let home = Home::random_us(&mut rng);
            let s = call_from(&mut rng, &home);
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
            assert_eq!(s.as_bytes()[d] - b'0', home.region.unwrap().digit, "{s}");
            assert!(Callsign::parse(&s).is_ok());
        }
    }

    /// Every format of every entity mostly lands in its own entity (a
    /// block's carve-outs are rare), and `call_from` always does.
    #[test]
    fn every_format_round_trips_to_its_entity_and_region() {
        let mut rng = Rng::new(7);
        for e in WORLD {
            for f in e.formats {
                let mut hits = 0;
                for _ in 0..50 {
                    let home = Home::random_in(&mut rng, e);
                    let s = call_in_format(&mut rng, f, &home);
                    let c = Callsign::parse(&s).unwrap_or_else(|_| panic!("{s}"));
                    if home_of(&c) == Some(home) {
                        hits += 1;
                    }
                }
                assert!(hits >= 45, "{e:?}: {hits}/50 of {:?}", f.prefixes);
            }
            for _ in 0..50 {
                let home = Home::random_in(&mut rng, e);
                let s = call_from(&mut rng, &home);
                let c = Callsign::parse(&s).unwrap_or_else(|_| panic!("{s}"));
                assert_eq!(home_of(&c), Some(home), "{s}");
            }
        }
    }

    #[test]
    fn a_one_call_entity_runs_out_instead_of_spinning() {
        let mut rng = Rng::new(1);
        let me = Callsign::parse("KF0UWV").unwrap();
        let mut make = |_: &mut Rng| "W1AB".to_string();
        let first = fresh_call(&mut rng, &mut make, &me, &[]).unwrap();
        assert!(fresh_call(&mut rng, &mut make, &me, &[first]).is_none());
    }

    #[test]
    fn weighted_formats_are_all_reached() {
        let mut rng = Rng::new(3);
        let us = by_dxcc(USA);
        let mut seen = vec![0u32; us.formats.len()];
        for _ in 0..5000 {
            let f = pick_weighted(&mut rng, us.formats, |f| f.weight);
            let i = us.formats.iter().position(|g| std::ptr::eq(g, f)).unwrap();
            seen[i] += 1;
        }
        // 1x3 (weight 35) is drawn far more often than 2x1 (weight 5).
        assert!(seen.iter().all(|n| *n > 0), "{seen:?}");
        assert!(seen[3] > 4 * seen[1], "{seen:?}");
    }

    fn h(c: &str) -> Option<Home> {
        home_of(&Callsign::parse(c).unwrap())
    }

    fn name(c: &str) -> &'static str {
        h(c).unwrap_or_else(|| panic!("{c} has no home"))
            .entity
            .name
    }

    #[test]
    fn home_is_inferred_from_prefix() {
        assert_eq!(h("KF0UWV").unwrap().cq(), 4);
        assert_eq!(h("W6ABC").unwrap().cq(), 3);
        assert_eq!(name("HB9XYZ"), "SWITZERLAND");
        assert_eq!(name("DL1ABC"), "GERMANY");
        assert_eq!(name("9A2AB"), "CROATIA");
        assert_eq!(h("AA1A").unwrap().entity.dxcc, USA);
        // No entity issues `Q`-prefixed calls.
        assert!(h("Q1AB").is_none());
    }

    #[test]
    fn a_location_designator_beats_the_home_call() {
        assert_eq!(name("F/DL1ABC"), "FRANCE");
        assert_eq!(name("HB9/DL1ABC"), "SWITZERLAND");
        let ve = h("VE3/W1XYZ").unwrap();
        assert_eq!(ve.entity.name, "CANADA");
        assert_eq!(name("JA1ABC/VE3"), "CANADA");
        // A bare digit moves the call area, not the entity.
        let w = h("W1ABC/4").unwrap();
        assert_eq!(w.entity.dxcc, USA);
        assert_eq!(w.region.unwrap().digit, 4);
        assert_eq!(h("W6ABC/7").unwrap().region.unwrap().digit, 7);
        // An unknown designator is not silently the home call's entity.
        assert!(h("Q/DL1ABC").is_none());
    }

    #[test]
    fn europe_splits_blocks_by_digit_and_tag() {
        assert_eq!(name("UA3ABC"), "EUROPEAN RUSSIA");
        assert_eq!(name("RA2FA"), "KALININGRAD");
        assert_eq!(name("R1FJL"), "FRANZ JOSEF LAND");
        assert_eq!(name("IS0ABC"), "SARDINIA");
        assert_eq!(name("IK0ABC"), "ITALY");
        assert_eq!(name("EA6AB"), "BALEARIC IS");
        assert_eq!(name("SV9ABC"), "CRETE");
        assert_eq!(name("SV2ASP/A"), "MOUNT ATHOS");
        // `/A` off Mount Athos's block is a modifier, not a tag.
        assert_eq!(name("SV1ABC/A"), "GREECE");
        assert_eq!(name("OH0ABC"), "ALAND IS");
        assert_eq!(name("OJ0B"), "MARKET REEF");
        assert_eq!(name("HB0ABC"), "LIECHTENSTEIN");
        assert_eq!(name("2E0ABC"), "ENGLAND");
        assert_eq!(name("MW0ABC"), "WALES");
        assert_eq!(name("4U1ITU"), "ITU HQ");
    }

    #[test]
    fn north_america_carves_territories_out_of_the_us_blocks() {
        assert_eq!(name("KL7ABC"), "ALASKA");
        assert_eq!(name("WP4ABC"), "PUERTO RICO");
        assert_eq!(name("KP2AB"), "US VIRGIN IS");
        assert_eq!(name("KG4AB"), "GUANTANAMO BAY");
        assert_eq!(name("4U1UN"), "UN HQ");
        assert_eq!(name("CY0ABC"), "SABLE IS");
        assert_eq!(name("VE3ABC"), "CANADA");
        assert_eq!(name("C6AGU"), "BAHAMAS");
        assert_eq!(name("XF4ABC"), "REVILLAGIGEDO");
        assert_eq!(name("XE1ABC"), "MEXICO");
        assert_eq!(name("TI9ABC"), "COCOS IS");
        assert_eq!(name("VP2EAB"), "ANGUILLA");
        assert_eq!(name("FO0ABC/C"), "CLIPPERTON");
    }

    #[test]
    fn south_america_islands_and_tags() {
        assert_eq!(name("CE0YAB"), "EASTER IS");
        assert_eq!(name("CE3ABC"), "CHILE");
        assert_eq!(name("PY0FAB"), "FERNANDO DE NORONHA");
        assert_eq!(name("PY2ABC"), "BRAZIL");
        assert_eq!(name("HC8AB"), "GALAPAGOS");
        assert_eq!(name("HK0ABC/M"), "MALPELO");
        assert_eq!(name("VP8ABC"), "FALKLAND IS");
        assert_eq!(name("VP8ABC/G"), "SOUTH GEORGIA");
        assert_eq!(name("VP8AB/H"), "SOUTH SHETLAND");
        assert_eq!(name("PJ2AB"), "CURACAO");
        assert_eq!(name("YV5ABC"), "VENEZUELA");
    }

    #[test]
    fn africa_islands_and_tags() {
        assert_eq!(name("EA8ABC"), "CANARY IS");
        assert_eq!(name("CT3AB"), "MADEIRA");
        assert_eq!(name("3B8AB"), "MAURITIUS");
        assert_eq!(name("3B9AB"), "RODRIGUES");
        assert_eq!(name("3C0AB"), "ANNOBON");
        assert_eq!(name("3C3AB"), "EQUATORIAL GUINEA");
        assert_eq!(name("FT5ZM"), "AMSTERDAM ST PAUL");
        assert_eq!(name("FT4XU"), "KERGUELEN");
        assert_eq!(name("ZS8Z"), "PRINCE EDWARD MARION");
        assert_eq!(name("ZS6ABC"), "SOUTH AFRICA");
        assert_eq!(name("3Y0AB/B"), "BOUVET");
        assert_eq!(name("VK0AB/H"), "HEARD IS");
        assert_eq!(name("ZD8AB"), "ASCENSION IS");
        assert_eq!(name("5Z4ABC"), "KENYA");
    }

    #[test]
    fn asia_splits_shared_blocks() {
        assert_eq!(name("UA9ABC"), "ASIATIC RUSSIA");
        assert_eq!(name("RA0AB"), "ASIATIC RUSSIA");
        assert_eq!(name("UK8ABC"), "UZBEKISTAN");
        assert_eq!(name("BY1ABC"), "CHINA");
        assert_eq!(name("BV2AB"), "TAIWAN");
        assert_eq!(name("BV9PAB"), "PRATAS IS");
        assert_eq!(name("BS7H"), "SCARBOROUGH REEF");
        assert_eq!(name("JA1ABC"), "JAPAN");
        assert_eq!(name("JD1ABC"), "OGASAWARA");
        assert_eq!(name("VU2ABC"), "INDIA");
        assert_eq!(name("VU4AB"), "ANDAMAN NICOBAR");
        assert_eq!(name("9M2AB"), "WEST MALAYSIA");
        assert_eq!(name("9M0AB"), "SPRATLY IS");
        assert_eq!(name("4X1AB"), "ISRAEL");
        assert_eq!(name("5B4AB"), "CYPRUS");
    }

    #[test]
    fn oceania_islands_and_tags() {
        assert_eq!(name("KH6ABC"), "HAWAII");
        assert_eq!(name("KH7KA"), "KURE IS");
        assert_eq!(name("KH8AB"), "AMERICAN SAMOA");
        assert_eq!(name("KH8AB/S"), "SWAINS IS");
        assert_eq!(name("VK9XAB"), "CHRISTMAS IS");
        assert_eq!(name("VK2ABC"), "AUSTRALIA");
        assert_eq!(name("VK0AB/M"), "MACQUARIE IS");
        assert_eq!(name("ZL7AB"), "CHATHAM IS");
        assert_eq!(name("3D2AB"), "FIJI");
        assert_eq!(name("3D2AB/R"), "ROTUMA");
        assert_eq!(name("3D2AB/C"), "CONWAY REEF");
        assert_eq!(name("E51AB/N"), "NORTH COOK IS");
        assert_eq!(name("FO0AB/M"), "MARQUESAS IS");
        assert_eq!(name("FO5AB"), "FRENCH POLYNESIA");
        assert_eq!(name("JD1AB/M"), "MINAMI TORISHIMA");
        assert_eq!(name("T33AB"), "BANABA");
        assert_eq!(name("H40AB"), "TEMOTU");
        assert_eq!(name("H44AB"), "SOLOMON IS");
        // Mobile is not a tag off any block but those that define one.
        assert_eq!(name("KH6ABC/M"), "HAWAII");
    }

    #[test]
    fn modifiers_do_not_move_a_station() {
        assert_eq!(name("DL1ABC/P"), "GERMANY");
        // `/M` is mobile, not the `M` prefix of England.
        assert_eq!(name("DL1ABC/M"), "GERMANY");
        assert_eq!(name("G4ABC/M"), "ENGLAND");
        assert_eq!(name("M0ABC/P"), "ENGLAND");
        let me = h("KF0UWV/QRP").unwrap();
        assert_eq!((me.entity.dxcc, me.region.unwrap().digit), (USA, 0));
        assert!(h("DL1ABC/MM").is_none());
        assert!(h("W1ABC/AM").is_none());
    }
}
