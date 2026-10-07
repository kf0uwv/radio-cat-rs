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
use super::world::{
    by_dxcc, Activity, CallFormat, DigitRule, Entity, Region, EXACT_SUFFIX, USA, WORLD,
};
use super::{HfBand, ScenarioError};
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

    /// A home in `entity`, in one of its call areas (if it has any) drawn
    /// by how busy each is.
    pub(crate) fn random_in(rng: &mut Rng, entity: &'static Entity) -> Home {
        let region = if entity.regions.is_empty() {
            None
        } else {
            Some(pick_weighted(rng, entity.regions, |r| r.weight))
        };
        Home { entity, region }
    }

    /// A home somewhere in the US.
    pub(crate) fn random_us(rng: &mut Rng) -> Home {
        Home::random_in(rng, by_dxcc(USA))
    }

    /// A station local to an operator at `me`: in the same entity, or the
    /// US when the operator's home is unknown.
    pub(crate) fn random_local(rng: &mut Rng, me: Option<Home>) -> Home {
        let entity = me.map_or_else(|| by_dxcc(USA), |h| h.entity);
        Home::random_in(rng, entity)
    }

    /// A DX station for an operator at `me` (the US when unknown): any
    /// other entity, drawn by how active it is, and [`NEAR_BOOST`] times as
    /// often from the operator's own continent.
    pub(crate) fn random_dx(rng: &mut Rng, me: Option<Home>, band: Option<HfBand>) -> Home {
        let home = me.map_or_else(|| by_dxcc(USA), |h| h.entity);
        Home::random_caller(rng, me, home.dxcc, band)
    }

    /// A station in a pileup heard by an operator at `me` (the US when
    /// unknown): drawn by activity, [`NEAR_BOOST`] times as often from the
    /// operator's continent (or as `band`'s path model says), and never
    /// from entity `not` (the DX being called is not called by its own
    /// countrymen).
    pub(crate) fn random_caller(
        rng: &mut Rng,
        me: Option<Home>,
        not: u16,
        band: Option<HfBand>,
    ) -> Home {
        let near = me.map_or_else(|| by_dxcc(USA), |h| h.entity).continent;
        let (near_w, far_w) = band.map_or((NEAR_BOOST, 1), HfBand::path_weights);
        let weight = |e: &&'static Entity| {
            if e.dxcc == not {
                0
            } else if e.continent == near {
                e.activity.weight() * near_w
            } else {
                e.activity.weight() * far_w
            }
        };
        let all: Vec<&'static Entity> = WORLD.iter().collect();
        let e = *pick_weighted(rng, &all, weight);
        Home::random_in(rng, e)
    }

    /// A DX worth chasing for an operator at `me`: a rare entity, or now
    /// and then an uncommon one; never the operator's own.
    pub(crate) fn random_rare(rng: &mut Rng, me: Option<Home>) -> Home {
        let own = me.map(|h| h.entity.dxcc);
        let weight = |e: &&'static Entity| match e.activity {
            _ if Some(e.dxcc) == own => 0,
            Activity::Rare => 5,
            Activity::Uncommon => 1,
            _ => 0,
        };
        let all: Vec<&'static Entity> = WORLD.iter().collect();
        let e = *pick_weighted(rng, &all, weight);
        Home::random_in(rng, e)
    }
}

/// How much likelier a DX station is from the operator's own continent:
/// short paths are louder, and neighbours are on at the same hours.
const NEAR_BOOST: u32 = 3;

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
            let fits = EXACT_SUFFIX
                .iter()
                .find(|(q, _)| q == p)
                .map_or(true, |(_, n)| piece.len() == p.len() + n);
            if fits && piece.starts_with(p) && best.map_or(true, |(_, l)| p.len() > l) {
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
/// modifiers after the base (`/P`, `/M`, `/A`, `/QRP`) are ignored; `/MM`
/// and `/AM` after it are in no entity. Before the base every piece is a
/// location prefix, so `M/DL1ABC` is England and `MM/DL1ABC` Scotland. Unknown prefixes return `None`; the caller picks a generic
/// home.
pub(crate) fn home_of(call: &Callsign) -> Option<Home> {
    let base = call.base();
    // A piece before the base is a location prefix (`EA8/`, and `M/` or
    // `MM/` for England and Scotland); only pieces after it can be tags,
    // modifiers or `/MM`. A prefix designator wins over a suffix one.
    let (mut before, mut after): (Option<&str>, Option<&str>) = (None, None);
    let mut past_base = false;
    for piece in call.as_str().split('/') {
        if std::ptr::eq(piece, base) {
            past_base = true;
            continue;
        }
        if !past_base {
            before = Some(piece);
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
        after = Some(piece);
    }
    let designator = before.or(after);
    // The piece naming the call area's prefix, and the one carrying its digit.
    let (entity, prefix_from, digit_from) = match designator {
        Some(d) if d.bytes().all(|b| b.is_ascii_digit()) => (entity_of(base)?, base, d),
        Some(d) => (entity_of(d)?, d, d),
        None => (entity_of(base)?, base, base),
    };
    let region = first_digit(digit_from).and_then(|d| entity.region(prefix_from, d));
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
    let (p, digit) = prefix_and_digit(rng, f, home);
    let n = rng.range_u32(f.suffix.0, f.suffix.1);
    let mut s = format!("{p}{digit}{}", letters(rng, n));
    if let Some(tag) = home.entity.tag {
        s.push('/');
        s.push_str(tag);
    }
    s
}

/// The prefix and digit of a call of format `f` from `home`.
fn prefix_and_digit(rng: &mut Rng, f: &CallFormat, home: &Home) -> (&'static str, String) {
    match f.digit {
        DigitRule::InPrefix => (*rng.pick(f.prefixes), String::new()),
        DigitRule::OneOf(d) => (*rng.pick(f.prefixes), rng.pick(d).to_string()),
        DigitRule::FromRegion => {
            let r = home.region.unwrap_or_else(|| rng.pick(home.entity.regions));
            // An area with its own letters (VO1, VY2) draws from them.
            let prefixes = if r.prefixes.is_empty() {
                f.prefixes
            } else {
                r.prefixes
            };
            (*rng.pick(prefixes), r.digit.to_string())
        }
    }
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

/// A prefix designator that puts a visiting station in `home`: `F`,
/// `EA8`, `VE3`, `HB0`. `None` for an entity that cannot be named so (a
/// tagged entity, or one whose shortest designator does not resolve).
fn designator(rng: &mut Rng, home: &Home) -> Option<String> {
    if home.entity.tag.is_some() {
        return None;
    }
    for _ in 0..16 {
        let f = pick_weighted(rng, home.entity.formats, |f| f.weight);
        let (p, d) = prefix_and_digit(rng, f, home);
        // CEPT style drops the digit where the prefix alone names the
        // entity (`F/`, `DL/`); an area or a numbered prefix keeps it.
        let mut tries = vec![format!("{p}{d}")];
        if matches!(f.digit, DigitRule::OneOf(_)) {
            tries.insert(0, p.to_string());
        }
        for t in tries {
            let region = first_digit(&t).and_then(|dg| home.entity.region(&t, dg));
            if entity_of(&t) == Some(home.entity) && region == home.region {
                return Some(t);
            }
        }
    }
    None
}

/// Chances that a station's call is `/P` (or `/M`, `/QRP`), signed abroad
/// (`EA8/DL1ABC`), or, for the US, signed in another call area
/// (`W1ABC/4`).
const P_PORTABLE: f64 = 0.04;
const P_ABROAD: f64 = 0.03;
const P_OTHER_AREA: f64 = 0.03;

/// A station's call as sent: usually [`call_from`], sometimes portable,
/// mobile, visiting from another entity, or (in the US) out of its area.
/// The call's home is always `home`.
pub(crate) fn station_call(rng: &mut Rng, home: &Home) -> String {
    let roll = rng.unit_f64();
    // A tagged entity's call already carries its `/` suffix.
    if home.entity.tag.is_some() {
        return call_from(rng, home);
    }
    if roll < P_PORTABLE {
        let suffix = *rng.pick(&["P", "P", "P", "M", "QRP"]);
        let s = format!("{}/{suffix}", call_from(rng, home));
        // `JD1AB/M` would read as Minami Torishima's tag: send it plain.
        if Callsign::parse(&s).ok().and_then(|c| home_of(&c)) == Some(*home) {
            return s;
        }
    }
    if roll < P_PORTABLE + P_ABROAD {
        let licensed = Home::random_dx(rng, Some(*home), None);
        let licence = call_from(rng, &licensed);
        if let Some(d) = designator(rng, home) {
            let s = format!("{d}/{licence}");
            let ok = !licence.contains('/')
                && licence.len() > d.len()
                && Callsign::parse(&s).ok().and_then(|c| home_of(&c)) == Some(*home);
            if ok {
                return s;
            }
        }
    } else if roll < P_PORTABLE + P_ABROAD + P_OTHER_AREA && home.entity.dxcc == USA {
        if let Some(area) = home.region {
            // A ham licensed in another area, now operating in this one.
            let licensed = Home::random_us(rng);
            if licensed.region != home.region {
                return format!("{}/{}", call_from(rng, &licensed), area.digit);
            }
        }
    }
    call_from(rng, home)
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
    use std::collections::HashMap;

    use super::*;
    use crate::scenario::world::Continent;

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
        // Malyj Vysotskij is a deleted entity: R1MV is European Russia.
        assert_eq!(name("R1MVA"), "EUROPEAN RUSSIA");
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
        // Only a 2x2 KG4 call is Guantanamo; 1- and 3-letter suffixes are US.
        let us4 = |c: &str| {
            let home = h(c).unwrap();
            (home.entity.dxcc, home.region.map(|r| r.digit))
        };
        assert_eq!(us4("KG4A"), (USA, Some(4)));
        assert_eq!(us4("KG4ABC"), (USA, Some(4)));
        assert_eq!(us4("KG4ABC/P"), (USA, Some(4)));
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
    fn antarctica_and_peter_i() {
        assert_eq!(name("KC4AAA"), "ANTARCTICA");
        assert_eq!(name("KC4USV"), "ANTARCTICA");
        // An ordinary US KC4 call is still in the US.
        assert_eq!(h("KC4ABC").unwrap().entity.dxcc, USA);
        assert_eq!(name("CE9AB"), "ANTARCTICA");
        assert_eq!(name("RI1ANA"), "ANTARCTICA");
        assert_eq!(name("3Y0AB/P"), "PETER I IS");
    }

    #[test]
    fn call_areas_carry_their_own_zones() {
        let zones = |c: &str| {
            let home = h(c).unwrap_or_else(|| panic!("{c}"));
            let r = home
                .region
                .unwrap_or_else(|| panic!("{c} has no call area"));
            (home.cq(), r.itu)
        };
        // Canada: the letters matter as well as the digit (VE1 vs VO1).
        assert_eq!(zones("VE1ABC"), (5, 9));
        assert_eq!(zones("VO1ABC"), (5, 9));
        assert_eq!(zones("VO2AB"), (2, 9));
        assert_eq!(zones("VE3ABC"), (4, 4));
        assert_eq!(zones("VA3ABC"), (4, 4));
        assert_eq!(zones("VE7ABC"), (3, 2));
        assert_eq!(zones("VY1AB"), (1, 2));
        assert_eq!(zones("VE3/W1XYZ"), (4, 4));
        assert_eq!(zones("JA1ABC"), (25, 45));
        assert_eq!(zones("UA3ABC"), (16, 29));
        assert_eq!(zones("UA9ABC"), (17, 30));
        assert_eq!(zones("UA0ABC"), (19, 34));
        assert_eq!(zones("BY1ABC"), (24, 44));
        assert_eq!(zones("BY0ABC"), (23, 42));
        assert_eq!(zones("VK6ABC"), (29, 58));
        assert_eq!(zones("VK4ABC"), (30, 55));
        assert_eq!(zones("ZL3ABC"), (32, 60));
        assert_eq!(zones("PY2ABC"), (11, 15));
        assert_eq!(zones("PY8ABC"), (11, 13));
        // A call area off the table leaves the entity's default zone.
        assert!(h("VE0ABC").unwrap().region.is_none());
    }

    fn tally(n: usize, seed: u64, mut draw: impl FnMut(&mut Rng) -> Home) -> HashMap<u16, usize> {
        let mut rng = Rng::new(seed);
        let mut counts = HashMap::new();
        for _ in 0..n {
            *counts.entry(draw(&mut rng).entity.dxcc).or_insert(0) += 1;
        }
        counts
    }

    #[test]
    fn dx_is_weighted_by_activity_and_every_entity_is_heard() {
        let n = 100_000;
        let counts = tally(n, 11, |r| Home::random_dx(r, None, None));
        let share = |a: Activity| -> f64 {
            let hits: usize = WORLD
                .iter()
                .filter(|e| e.activity == a)
                .map(|e| counts.get(&e.dxcc).copied().unwrap_or(0))
                .sum();
            hits as f64 / n as f64
        };
        assert!(share(Activity::Common) > 0.3, "{}", share(Activity::Common));
        assert!(share(Activity::Rare) < 0.05, "{}", share(Activity::Rare));
        // The busiest common entity far outdraws the busiest rare one.
        let top = |a: Activity| {
            WORLD
                .iter()
                .filter(|e| e.activity == a && e.dxcc != USA)
                .map(|e| counts.get(&e.dxcc).copied().unwrap_or(0))
                .max()
                .unwrap()
        };
        assert!(top(Activity::Common) > 20 * top(Activity::Rare));
        // The US is the local side of a draw, never DX for a US operator.
        assert!(!counts.contains_key(&USA));
        for e in WORLD.iter().filter(|e| e.dxcc != USA) {
            assert!(counts.contains_key(&e.dxcc), "{e:?} never drawn");
        }
    }

    #[test]
    fn dx_leans_toward_the_operator_s_continent() {
        let n = 20_000;
        let continent_share = |me: Option<Home>, c: Continent| {
            let counts = tally(n, 5, |r| Home::random_dx(r, me, None));
            let hits: usize = counts
                .iter()
                .filter(|(d, _)| by_dxcc(**d).continent == c)
                .map(|(_, k)| k)
                .sum();
            hits as f64 / n as f64
        };
        let ja = h("JA1ABC");
        let dl = h("DL1ABC");
        assert!(continent_share(ja, Continent::As) > 1.5 * continent_share(None, Continent::As));
        assert!(continent_share(dl, Continent::Eu) > continent_share(None, Continent::Eu));
        // An operator's own entity is local, not DX.
        assert!(!tally(n, 5, |r| Home::random_dx(r, dl, None)).contains_key(&230));
    }

    #[test]
    fn low_bands_stay_close_and_high_bands_reach_out() {
        use crate::scenario::HfBand;
        let dl = h("DL1ABC");
        let eu = |band: Option<HfBand>| {
            let n = 20_000;
            let counts = tally(n, 8, |r| Home::random_dx(r, dl, band));
            let hits: usize = counts
                .iter()
                .filter(|(d, _)| by_dxcc(**d).continent == Continent::Eu)
                .map(|(_, k)| k)
                .sum();
            hits as f64 / n as f64
        };
        let (b160, b40, b20, b10) = (
            eu(Some(HfBand::M160)),
            eu(Some(HfBand::M40)),
            eu(Some(HfBand::M20)),
            eu(Some(HfBand::M10)),
        );
        assert!(b160 > 0.8, "{b160}");
        assert!(
            b160 > b40 && b40 > b20 && b20 > b10,
            "{b160} {b40} {b20} {b10}"
        );
        // No band: the plain near-continent lean.
        let none = eu(None);
        assert!(b40 > none && none > b10, "{none}");
    }

    #[test]
    fn local_stations_come_from_the_operator_s_entity() {
        let dl = h("DL1ABC");
        let counts = tally(1000, 3, |r| Home::random_local(r, dl));
        assert_eq!(counts.keys().copied().collect::<Vec<_>>(), vec![230]);
        let none = tally(1000, 3, |r| Home::random_local(r, None));
        assert_eq!(none.keys().copied().collect::<Vec<_>>(), vec![USA]);
    }

    #[test]
    fn busy_call_areas_outdraw_empty_ones() {
        let mut rng = Rng::new(9);
        let ve = by_dxcc(1);
        let mut areas: HashMap<(u8, &str), usize> = HashMap::new();
        for _ in 0..20_000 {
            let r = Home::random_in(&mut rng, ve).region.unwrap();
            *areas.entry((r.digit, r.prefixes[0])).or_insert(0) += 1;
        }
        let ve3 = areas[&(3, "VE")];
        let vy0 = areas.get(&(0, "VY")).copied().unwrap_or(0);
        assert!(ve3 > 10 * vy0.max(1), "VE3 {ve3} VY0 {vy0}");
        let mut rng = Rng::new(9);
        let ja1 = (0..10_000)
            .filter(|_| {
                Home::random_in(&mut rng, by_dxcc(339))
                    .region
                    .unwrap()
                    .digit
                    == 1
            })
            .count();
        assert!(ja1 > 2_000, "{ja1}");
    }

    #[test]
    fn station_calls_are_sometimes_portable_and_always_home() {
        let mut rng = Rng::new(21);
        let (mut plain, mut suffix, mut abroad, mut area) = (0, 0, 0, 0);
        let n = 20_000;
        for i in 0..n {
            let home = if i % 2 == 0 {
                Home::random_us(&mut rng)
            } else {
                Home::random_dx(&mut rng, None, None)
            };
            let s = station_call(&mut rng, &home);
            let c = Callsign::parse(&s).unwrap_or_else(|_| panic!("{s}"));
            assert_eq!(home_of(&c), Some(home), "{s}");
            let pieces: Vec<&str> = s.split('/').collect();
            match pieces.as_slice() {
                [_] => plain += 1,
                [_, "P" | "M" | "QRP"] => suffix += 1,
                [_, d] if d.bytes().all(|b| b.is_ascii_digit()) => area += 1,
                [d, b] if b.len() > d.len() => abroad += 1,
                // A tagged entity's own suffix (3D2AB/R) is a plain call.
                [_, t] if home.entity.tag == Some(*t) => plain += 1,
                _ => panic!("unexpected shape {s}"),
            }
        }
        assert!(plain as f64 > 0.85 * n as f64, "{plain}");
        assert!(
            suffix > 200 && abroad > 200 && area > 50,
            "{suffix} {abroad} {area}"
        );
    }

    #[test]
    fn slash_calls_score_strictly() {
        use crate::score::{align, align_unordered, ScoreOptions};
        let o = ScoreOptions::default();
        // Copying the whole call is perfect.
        let s = align("EA8/DL1ABC", "EA8/DL1ABC", &o).unwrap().score();
        assert_eq!((s.matches, s.deletions), (10, 0));
        // Dropping the designator is four errors, not a pass.
        let s = align("EA8/DL1ABC", "DL1ABC", &o).unwrap().score();
        assert_eq!((s.matches, s.deletions), (6, 4));
        // In a pileup the base still pairs with its call, charged the same.
        let u = align_unordered(&["EA8/DL1ABC", "K1ABC"], "K1ABC DL1ABC", &o).unwrap();
        assert_eq!(u.matched, vec![Some(1), Some(0)]);
        assert_eq!(u.score().deletions, 4);
        // A numeral in a prefix is an ordinary character.
        let s = align("3D2AB/R", "3D2AB/P", &o).unwrap().score();
        assert_eq!((s.matches, s.substitutions), (6, 1));
    }

    #[test]
    fn modifiers_only_follow_the_base() {
        // Before the base, `M`, `B` and `MM` are prefixes, not modifiers.
        assert_eq!(name("M/DL1ABC"), "ENGLAND");
        assert_eq!(name("B/DL1ABC"), "CHINA");
        assert_eq!(name("MM/DL1ABC"), "SCOTLAND");
        assert_eq!(name("MM/DL1ABC/P"), "SCOTLAND");
        // After it, they still are.
        assert!(h("DL1ABC/MM").is_none());
        assert_eq!(name("DL1ABC/M"), "GERMANY");
        // And a visitor can be signed into England as M/ and Scotland as MM/.
        let mut rng = Rng::new(4);
        let mut seen = std::collections::HashSet::new();
        for dxcc in [223, 279] {
            for _ in 0..400 {
                let home = Home::random_in(&mut rng, by_dxcc(dxcc));
                if let Some(d) = designator(&mut rng, &home) {
                    seen.insert(d);
                }
            }
        }
        assert!(seen.contains("M") && seen.contains("MM"), "{seen:?}");
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
