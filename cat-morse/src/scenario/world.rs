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

//! The world's amateur stations: DXCC entities with their real prefix
//! structure, CQ and ITU zones, continent, and call areas where the call's
//! digit means something (US call areas today; JA, VE, UA and others to
//! come).
//!
//! Hand-built from public facts — ITU prefix allocations (Radio
//! Regulations Appendix 42), the DXCC entity list and the CQ/ITU zone maps.
//! No third-party prefix file is embedded. Every string is upper case and
//! Morse-encodable (checked by a test).

use std::fmt;

/// A continent, as DXCC assigns them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // weighting by continent arrives with task 4; `An` with the full list
pub(crate) enum Continent {
    Na,
    Sa,
    Eu,
    Af,
    As,
    Oc,
    An,
}

/// Where a generated call's digit comes from.
pub(crate) enum DigitRule {
    /// The digit is part of the prefix (`HB9`, `S5`), or there is none.
    InPrefix,
    /// One of these digits follows the prefix (`DL` + 1–9).
    OneOf(&'static [u8]),
    /// The call area's digit (`W` + the region's digit).
    FromRegion,
}

/// One shape of call an entity issues: a prefix from `prefixes`, a digit
/// by `digit`, then `suffix` letters (min, max). Formats are drawn by
/// `weight`.
pub(crate) struct CallFormat {
    pub prefixes: &'static [&'static str],
    pub digit: DigitRule,
    pub suffix: (u32, u32),
    pub weight: u32,
}

/// A call area inside an entity: the digit its calls carry, its zones,
/// and its subdivisions (state/province code as sent in an exchange, with
/// a few towns each).
pub(crate) struct Region {
    pub digit: u8,
    pub cq: u8,
    #[cfg_attr(not(test), allow(dead_code))] // read by tests; exchanges using it come later
    pub itu: u8,
    pub subdivisions: &'static [(&'static str, &'static [&'static str])],
}

/// A DXCC entity.
///
/// `prefixes` are what [`home_of`](super::callsign::home_of) matches
/// (longest wins, so `HB9` beats `H` and `KH6` beats `K`); `formats` are
/// what calls are generated from. `cq`/`itu` are the entity's zones when
/// no region says otherwise. `iso` is the ISO 3166 code POTA uses for the
/// entity's park references (since the 2024 re-prefixing, e.g.
/// `DE-0123`), where POTA parks are common enough to appear in a scenario.
pub(crate) struct Entity {
    pub dxcc: u16,
    pub name: &'static str,
    #[allow(dead_code)] // weighting by continent arrives with task 4
    pub continent: Continent,
    pub cq: u8,
    #[cfg_attr(not(test), allow(dead_code))] // read by tests; exchanges using it come later
    pub itu: u8,
    pub prefixes: &'static [&'static str],
    pub formats: &'static [CallFormat],
    pub regions: &'static [Region],
    /// For an entity that shares its parent's prefix block and is told
    /// apart by a `/`-suffix on the call (`3D2AB/R` for Rotuma, `SV2ASP/A`
    /// for Mount Athos): that suffix. Its `prefixes` then only qualify the
    /// tag and take no part in plain longest-prefix matching.
    pub tag: Option<&'static str>,
    pub iso: Option<&'static str>,
    pub towns: &'static [&'static str],
}

impl PartialEq for Entity {
    fn eq(&self, other: &Self) -> bool {
        self.dxcc == other.dxcc
    }
}
impl Eq for Entity {}
impl fmt::Debug for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Entity({} {})", self.dxcc, self.name)
    }
}
/// Regions compare by digit and zones: within one entity the digit is
/// unique (checked by a test), and `Home` compares the entity too.
impl PartialEq for Region {
    fn eq(&self, other: &Self) -> bool {
        (self.digit, self.cq, self.itu) == (other.digit, other.cq, other.itu)
    }
}
impl Eq for Region {}
impl fmt::Debug for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Region({}, CQ {})", self.digit, self.cq)
    }
}

impl Entity {
    /// The region whose calls carry `digit`, if the entity has regions.
    pub(crate) fn region(&self, digit: u8) -> Option<&'static Region> {
        // Regions live in `'static` tables, but `&self` may not; look the
        // entity up again to hand out a `'static` borrow.
        by_dxcc(self.dxcc).regions.iter().find(|r| r.digit == digit)
    }
}

/// The entity with DXCC number `dxcc`. Panics on an unknown number: callers
/// pass constants from this module.
pub(crate) fn by_dxcc(dxcc: u16) -> &'static Entity {
    WORLD
        .iter()
        .find(|e| e.dxcc == dxcc)
        .unwrap_or_else(|| panic!("no DXCC entity {dxcc}"))
}

/// A call format, for the compact table rows below.
const fn cf(
    prefixes: &'static [&'static str],
    digit: DigitRule,
    suffix: (u32, u32),
    weight: u32,
) -> CallFormat {
    CallFormat {
        prefixes,
        digit,
        suffix,
        weight,
    }
}

/// One table row: an entity with no call-area regions. A macro keeps the
/// ~340 rows to a few lines each (rustfmt leaves brace-delimited macro
/// calls alone).
macro_rules! ent {
    (
        dxcc: $dxcc:expr, name: $name:expr, continent: $cont:ident, cq: $cq:expr, itu: $itu:expr,
        prefixes: [$($p:expr),* $(,)?],
        formats: [$($f:expr),* $(,)?],
        tag: $tag:expr, iso: $iso:expr,
        towns: [$($t:expr),* $(,)?] $(,)?
    ) => {
        Entity {
            dxcc: $dxcc,
            name: $name,
            continent: Continent::$cont,
            cq: $cq,
            itu: $itu,
            prefixes: &[$($p),*],
            formats: &[$($f),*],
            regions: &[],
            tag: $tag,
            iso: $iso,
            towns: &[$($t),*],
        }
    };
}

/// DXCC number of the United States.
pub(crate) const USA: u16 = 291;

const D1_9: &[u8] = &[1, 2, 3, 4, 5, 6, 7, 8, 9];
const D0_9: &[u8] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9];

/// US one-letter prefixes.
const US_1: &[&str] = &["K", "N", "W"];
/// US two-letter prefixes issued in the lower 48: `K`/`N`/`W` + any letter
/// but `H`, `L`, `P` (Pacific, Alaska, Caribbean), and `AA`–`AK` less `AH`.
const US_2: &[&str] = &[
    "KA", "KB", "KC", "KD", "KE", "KF", "KG", "KI", "KJ", "KK", "KM", "KN", "KO", "KQ", "KR", "KS",
    "KT", "KU", "KV", "KW", "KX", "KY", "KZ", "NA", "NB", "NC", "ND", "NE", "NF", "NG", "NI", "NJ",
    "NK", "NM", "NN", "NO", "NQ", "NR", "NS", "NT", "NU", "NV", "NW", "NX", "NY", "NZ", "WA", "WB",
    "WC", "WD", "WE", "WF", "WG", "WI", "WJ", "WK", "WM", "WN", "WO", "WQ", "WR", "WS", "WT", "WU",
    "WV", "WW", "WX", "WY", "WZ", "AA", "AB", "AC", "AD", "AE", "AF", "AG", "AI", "AJ", "AK",
];
/// US two-letter prefixes of 2x3 calls (`K`/`W` only).
const US_2_KW: &[&str] = &[
    "KA", "KB", "KC", "KD", "KE", "KF", "KG", "KI", "KJ", "KK", "KM", "KN", "KO", "KQ", "KR", "KS",
    "KT", "KU", "KV", "KW", "KX", "KY", "KZ", "WA", "WB", "WC", "WD", "WE", "WF", "WG", "WI", "WJ",
    "WK", "WM", "WN", "WO", "WQ", "WR", "WS", "WT", "WU", "WV", "WW", "WX", "WY", "WZ",
];

/// What `home_of` matches for the US: the letter blocks, never `AH`/`AL`
/// or `KH`/`KL`/`KP`, which belong to other entities (listed as they are
/// added; until then they fall to the one-letter `K`/`N`/`W`).
const US_MATCH: &[&str] = &[
    "K", "N", "W", "AA", "AB", "AC", "AD", "AE", "AF", "AG", "AI", "AJ", "AK",
];

/// US formats: 1x2, 2x1, 2x2, 1x3 and 2x3, weighted toward the common
/// 1x3 and 2x3.
const US_FORMATS: &[CallFormat] = &[
    CallFormat {
        prefixes: US_1,
        digit: DigitRule::FromRegion,
        suffix: (2, 2),
        weight: 10,
    },
    CallFormat {
        prefixes: US_2,
        digit: DigitRule::FromRegion,
        suffix: (1, 1),
        weight: 5,
    },
    CallFormat {
        prefixes: US_2,
        digit: DigitRule::FromRegion,
        suffix: (2, 2),
        weight: 15,
    },
    CallFormat {
        prefixes: US_1,
        digit: DigitRule::FromRegion,
        suffix: (3, 3),
        weight: 35,
    },
    CallFormat {
        prefixes: US_2_KW,
        digit: DigitRule::FromRegion,
        suffix: (3, 3),
        weight: 35,
    },
];

/// US call areas: digit, the states each historically covers (two-letter
/// postal codes, as sent in a POTA exchange) with a few towns, and zones.
pub(crate) const US_REGIONS: &[Region] = &[
    Region {
        digit: 1,
        cq: 5,
        itu: 8,
        subdivisions: &[
            ("MA", &["BOSTON", "WORCESTER", "SALEM"]),
            ("CT", &["HARTFORD", "NEWINGTON", "MYSTIC"]),
            ("ME", &["PORTLAND", "BANGOR"]),
            ("NH", &["NASHUA", "CONCORD"]),
            ("VT", &["BURLINGTON"]),
            ("RI", &["PROVIDENCE"]),
        ],
    },
    Region {
        digit: 2,
        cq: 5,
        itu: 8,
        subdivisions: &[
            ("NY", &["ALBANY", "BUFFALO", "ROCHESTER", "ITHACA"]),
            ("NJ", &["TRENTON", "PRINCETON", "NEWARK"]),
        ],
    },
    Region {
        digit: 3,
        cq: 5,
        itu: 8,
        subdivisions: &[
            ("PA", &["PITTSBURGH", "ERIE", "LANCASTER"]),
            ("MD", &["BALTIMORE", "FREDERICK"]),
            ("DE", &["DOVER", "WILMINGTON"]),
        ],
    },
    Region {
        digit: 4,
        cq: 5,
        itu: 8,
        subdivisions: &[
            ("FL", &["TAMPA", "ORLANDO", "MIAMI"]),
            ("GA", &["ATLANTA", "SAVANNAH"]),
            ("NC", &["RALEIGH", "ASHEVILLE", "DURHAM"]),
            ("VA", &["RICHMOND", "ROANOKE"]),
            ("TN", &["NASHVILLE", "KNOXVILLE"]),
            ("KY", &["LEXINGTON", "LOUISVILLE"]),
            ("AL", &["HUNTSVILLE", "MOBILE"]),
            ("SC", &["CHARLESTON", "COLUMBIA"]),
        ],
    },
    Region {
        digit: 5,
        cq: 4,
        itu: 7,
        subdivisions: &[
            ("TX", &["AUSTIN", "DALLAS", "EL PASO", "HOUSTON"]),
            ("OK", &["TULSA", "NORMAN"]),
            ("LA", &["BATON ROUGE", "SHREVEPORT"]),
            ("NM", &["SANTA FE", "TAOS", "ALBUQUERQUE"]),
            ("AR", &["LITTLE ROCK"]),
            ("MS", &["JACKSON", "BILOXI"]),
        ],
    },
    Region {
        digit: 6,
        cq: 3,
        itu: 6,
        subdivisions: &[(
            "CA",
            &["SAN DIEGO", "FRESNO", "SACRAMENTO", "SAN JOSE", "BISHOP"],
        )],
    },
    Region {
        digit: 7,
        cq: 3,
        itu: 6,
        subdivisions: &[
            ("WA", &["SEATTLE", "SPOKANE", "OLYMPIA"]),
            ("OR", &["PORTLAND", "BEND", "EUGENE"]),
            ("AZ", &["TUCSON", "PHOENIX", "FLAGSTAFF"]),
            ("UT", &["OGDEN", "MOAB", "PROVO"]),
            ("MT", &["BOZEMAN", "MISSOULA"]),
            ("ID", &["BOISE"]),
            ("NV", &["RENO", "ELKO"]),
            ("WY", &["CHEYENNE", "CODY"]),
        ],
    },
    Region {
        digit: 8,
        cq: 4,
        itu: 8,
        subdivisions: &[
            ("OH", &["DAYTON", "COLUMBUS", "TOLEDO"]),
            ("MI", &["DETROIT", "LANSING", "MARQUETTE"]),
            ("WV", &["MORGANTOWN", "CHARLESTON"]),
        ],
    },
    Region {
        digit: 9,
        cq: 4,
        itu: 8,
        subdivisions: &[
            ("IL", &["CHICAGO", "PEORIA", "SPRINGFIELD"]),
            ("IN", &["FORT WAYNE", "INDIANAPOLIS"]),
            ("WI", &["MADISON", "GREEN BAY"]),
        ],
    },
    Region {
        digit: 0,
        cq: 4,
        itu: 7,
        subdivisions: &[
            ("CO", &["DENVER", "BOULDER", "GOLDEN", "DURANGO"]),
            ("MN", &["DULUTH", "ROCHESTER", "ST PAUL"]),
            ("MO", &["ST LOUIS", "JOPLIN"]),
            ("KS", &["WICHITA", "LAWRENCE"]),
            ("NE", &["OMAHA", "LINCOLN"]),
            ("IA", &["DES MOINES", "AMES"]),
            ("ND", &["FARGO", "MINOT"]),
            ("SD", &["RAPID CITY", "SIOUX FALLS"]),
        ],
    },
];

/// Every entity the trainer knows.
pub(crate) const WORLD: &[Entity] = &[
    Entity {
        dxcc: USA,
        name: "USA",
        continent: Continent::Na,
        cq: 5,
        itu: 8,
        prefixes: US_MATCH,
        formats: US_FORMATS,
        regions: US_REGIONS,
        tag: None,
        iso: Some("US"),
        towns: &[],
    },
    // Europe ------------------------------------------------------------------------------------
    ent! {
        dxcc: 246, name: "SOV MIL ORDER OF MALTA", continent: Eu, cq: 15, itu: 28,
        prefixes: ["1A"],
        formats: [cf(&["1A0"], DigitRule::InPrefix, (1, 2), 1)],
        tag: None, iso: None,
        towns: ["ROMA"],
    },
    ent! {
        dxcc: 260, name: "MONACO", continent: Eu, cq: 14, itu: 27,
        prefixes: ["3A"],
        formats: [cf(&["3A2"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["MONTE CARLO"],
    },
    ent! {
        dxcc: 514, name: "MONTENEGRO", continent: Eu, cq: 15, itu: 28,
        prefixes: ["4O"],
        formats: [cf(&["4O"], DigitRule::OneOf(&[3, 4, 6, 7]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["PODGORICA", "BUDVA"],
    },
    ent! {
        dxcc: 117, name: "ITU HQ", continent: Eu, cq: 14, itu: 28,
        prefixes: ["4U1ITU"],
        formats: [cf(&["4U1ITU"], DigitRule::InPrefix, (0, 0), 1)],
        tag: None, iso: None,
        towns: ["GENEVA"],
    },
    ent! {
        dxcc: 497, name: "CROATIA", continent: Eu, cq: 15, itu: 28,
        prefixes: ["9A"],
        formats: [cf(&["9A"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["ZAGREB", "SPLIT"],
    },
    ent! {
        dxcc: 257, name: "MALTA", continent: Eu, cq: 15, itu: 28,
        prefixes: ["9H"],
        formats: [cf(&["9H"], DigitRule::OneOf(&[1, 5]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["VALLETTA", "MOSTA"],
    },
    ent! {
        dxcc: 203, name: "ANDORRA", continent: Eu, cq: 14, itu: 27,
        prefixes: ["C3"],
        formats: [cf(&["C3"], DigitRule::OneOf(&[1, 7]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["ANDORRA LA VELLA"],
    },
    ent! {
        dxcc: 149, name: "AZORES", continent: Eu, cq: 14, itu: 36,
        prefixes: ["CU"],
        formats: [cf(&["CU"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["PONTA DELGADA", "HORTA"],
    },
    ent! {
        dxcc: 272, name: "PORTUGAL", continent: Eu, cq: 14, itu: 37,
        prefixes: ["CT", "CQ", "CR", "CS"],
        formats: [cf(&["CT", "CS"], DigitRule::OneOf(&[1, 2, 7]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["LISBOA", "PORTO"],
    },
    ent! {
        dxcc: 230, name: "GERMANY", continent: Eu, cq: 14, itu: 28,
        prefixes: ["DA", "DB", "DC", "DD", "DE", "DF", "DG", "DH", "DI", "DJ", "DK", "DL", "DM",
            "DN", "DO", "DP", "DQ", "DR"],
        formats: [
            cf(&["DL", "DK", "DJ", "DF", "DH", "DG", "DO", "DB", "DC", "DD", "DM"],
                DigitRule::OneOf(D1_9), (2, 3), 1),
        ],
        tag: None, iso: Some("DE"),
        towns: ["BERLIN", "MUNICH", "HAMBURG", "BONN"],
    },
    ent! {
        dxcc: 501, name: "BOSNIA", continent: Eu, cq: 15, itu: 28,
        prefixes: ["E7", "T9"],
        formats: [cf(&["E7"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["SARAJEVO", "MOSTAR"],
    },
    ent! {
        dxcc: 281, name: "SPAIN", continent: Eu, cq: 14, itu: 37,
        prefixes: ["EA", "EB", "EC", "ED", "EE", "EF", "EG", "EH", "AM", "AN", "AO"],
        formats: [cf(&["EA", "EB", "EC", "ED"], DigitRule::OneOf(&[1, 2, 3, 4, 5, 7]), (2, 3), 1)],
        tag: None, iso: Some("ES"),
        towns: ["MADRID", "SEVILLA", "VALENCIA"],
    },
    ent! {
        dxcc: 21, name: "BALEARIC IS", continent: Eu, cq: 14, itu: 37,
        prefixes: ["EA6", "EB6", "EC6", "ED6", "EE6", "EF6", "EG6", "EH6", "AM6", "AN6", "AO6"],
        formats: [cf(&["EA6", "EB6", "EC6", "ED6"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["PALMA", "IBIZA"],
    },
    ent! {
        dxcc: 245, name: "IRELAND", continent: Eu, cq: 14, itu: 27,
        prefixes: ["EI", "EJ"],
        formats: [cf(&["EI", "EJ"], DigitRule::OneOf(&[2, 3, 4, 5, 6, 7, 8, 9]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["DUBLIN", "CORK"],
    },
    ent! {
        dxcc: 179, name: "MOLDOVA", continent: Eu, cq: 16, itu: 29,
        prefixes: ["ER"],
        formats: [cf(&["ER"], DigitRule::OneOf(&[1, 2, 3, 4, 5]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["CHISINAU"],
    },
    ent! {
        dxcc: 52, name: "ESTONIA", continent: Eu, cq: 15, itu: 29,
        prefixes: ["ES"],
        formats: [cf(&["ES"], DigitRule::OneOf(&[1, 2, 3, 4, 5, 6, 7, 8]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["TALLINN", "TARTU"],
    },
    ent! {
        dxcc: 27, name: "BELARUS", continent: Eu, cq: 16, itu: 29,
        prefixes: ["EU", "EV", "EW"],
        formats: [cf(&["EU", "EW"], DigitRule::OneOf(&[1, 2, 3, 4, 5, 6, 7, 8]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["MINSK", "GOMEL"],
    },
    ent! {
        dxcc: 227, name: "FRANCE", continent: Eu, cq: 14, itu: 27,
        prefixes: ["F"],
        formats: [cf(&["F"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: Some("FR"),
        towns: ["PARIS", "LYON", "NANTES"],
    },
    ent! {
        dxcc: 223, name: "ENGLAND", continent: Eu, cq: 14, itu: 27,
        prefixes: ["G", "M", "2E"],
        formats: [
            cf(&["G"], DigitRule::OneOf(&[0, 1, 2, 3, 4, 5, 6, 7, 8]), (2, 3), 4),
            cf(&["M"], DigitRule::OneOf(&[0, 1, 3, 5, 6, 7]), (3, 3), 3),
            cf(&["2E"], DigitRule::OneOf(&[0, 1]), (3, 3), 1),
        ],
        tag: None, iso: None,
        towns: ["LONDON", "BRISTOL", "YORK", "LEEDS"],
    },
    ent! {
        dxcc: 114, name: "ISLE OF MAN", continent: Eu, cq: 14, itu: 27,
        prefixes: ["GD", "MD", "2D", "GT", "MT"],
        formats: [cf(&["GD", "MD"], DigitRule::OneOf(&[0, 1, 2, 3, 4, 5, 6, 7, 8]), (3, 3), 1)],
        tag: None, iso: None,
        towns: ["DOUGLAS"],
    },
    ent! {
        dxcc: 265, name: "NORTHERN IRELAND", continent: Eu, cq: 14, itu: 27,
        prefixes: ["GI", "MI", "2I", "GN", "MN"],
        formats: [cf(&["GI", "MI"], DigitRule::OneOf(&[0, 1, 2, 3, 4, 5, 6, 7, 8]), (3, 3), 1)],
        tag: None, iso: None,
        towns: ["BELFAST", "DERRY"],
    },
    ent! {
        dxcc: 122, name: "JERSEY", continent: Eu, cq: 14, itu: 27,
        prefixes: ["GJ", "MJ", "2J", "GH", "MH"],
        formats: [cf(&["GJ", "MJ"], DigitRule::OneOf(&[0, 1, 2, 3, 4, 5, 6, 7, 8]), (3, 3), 1)],
        tag: None, iso: None,
        towns: ["ST HELIER"],
    },
    ent! {
        dxcc: 279, name: "SCOTLAND", continent: Eu, cq: 14, itu: 27,
        prefixes: ["GM", "MM", "2M", "GS", "MS"],
        formats: [cf(&["GM", "MM"], DigitRule::OneOf(&[0, 1, 2, 3, 4, 5, 6, 7, 8]), (3, 3), 1)],
        tag: None, iso: None,
        towns: ["EDINBURGH", "GLASGOW"],
    },
    ent! {
        dxcc: 106, name: "GUERNSEY", continent: Eu, cq: 14, itu: 27,
        prefixes: ["GU", "MU", "2U", "GP", "MP"],
        formats: [cf(&["GU", "MU"], DigitRule::OneOf(&[0, 1, 2, 3, 4, 5, 6, 7, 8]), (3, 3), 1)],
        tag: None, iso: None,
        towns: ["ST PETER PORT"],
    },
    ent! {
        dxcc: 294, name: "WALES", continent: Eu, cq: 14, itu: 27,
        prefixes: ["GW", "MW", "2W", "GC", "MC"],
        formats: [cf(&["GW", "MW"], DigitRule::OneOf(&[0, 1, 2, 3, 4, 5, 6, 7, 8]), (3, 3), 1)],
        tag: None, iso: None,
        towns: ["CARDIFF", "SWANSEA"],
    },
    ent! {
        dxcc: 239, name: "HUNGARY", continent: Eu, cq: 15, itu: 28,
        prefixes: ["HA", "HG"],
        formats: [cf(&["HA", "HG"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["BUDAPEST", "PECS"],
    },
    ent! {
        dxcc: 287, name: "SWITZERLAND", continent: Eu, cq: 14, itu: 28,
        prefixes: ["HB", "HE"],
        formats: [cf(&["HB9"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["BERN", "ZURICH"],
    },
    ent! {
        dxcc: 251, name: "LIECHTENSTEIN", continent: Eu, cq: 14, itu: 28,
        prefixes: ["HB0", "HE0"],
        formats: [cf(&["HB0"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["VADUZ"],
    },
    ent! {
        dxcc: 295, name: "VATICAN", continent: Eu, cq: 15, itu: 28,
        prefixes: ["HV"],
        formats: [cf(&["HV"], DigitRule::OneOf(&[0]), (1, 2), 1)],
        tag: None, iso: None,
        towns: ["VATICAN"],
    },
    ent! {
        dxcc: 248, name: "ITALY", continent: Eu, cq: 15, itu: 28,
        prefixes: ["I"],
        formats: [cf(&["I", "IK", "IZ", "IW", "IU"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: Some("IT"),
        towns: ["ROMA", "MILANO", "TORINO"],
    },
    ent! {
        dxcc: 225, name: "SARDINIA", continent: Eu, cq: 15, itu: 28,
        prefixes: ["IS0", "IM0"],
        formats: [cf(&["IS0", "IM0"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["CAGLIARI", "SASSARI"],
    },
    ent! {
        dxcc: 259, name: "SVALBARD", continent: Eu, cq: 40, itu: 18,
        prefixes: ["JW"],
        formats: [cf(&["JW"], DigitRule::OneOf(D0_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["LONGYEARBYEN"],
    },
    ent! {
        dxcc: 118, name: "JAN MAYEN", continent: Eu, cq: 40, itu: 18,
        prefixes: ["JX"],
        formats: [cf(&["JX"], DigitRule::OneOf(D0_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["OLONKINBYEN"],
    },
    ent! {
        dxcc: 266, name: "NORWAY", continent: Eu, cq: 14, itu: 18,
        prefixes: ["LA", "LB", "LC", "LD", "LE", "LF", "LG", "LH", "LI", "LJ", "LK", "LL", "LM",
            "LN"],
        formats: [cf(&["LA", "LB"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["OSLO", "BERGEN"],
    },
    ent! {
        dxcc: 254, name: "LUXEMBOURG", continent: Eu, cq: 14, itu: 27,
        prefixes: ["LX"],
        formats: [cf(&["LX"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["LUXEMBOURG"],
    },
    ent! {
        dxcc: 146, name: "LITHUANIA", continent: Eu, cq: 15, itu: 29,
        prefixes: ["LY"],
        formats: [cf(&["LY"], DigitRule::OneOf(&[1, 2, 3, 4, 5]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["VILNIUS", "KAUNAS"],
    },
    ent! {
        dxcc: 212, name: "BULGARIA", continent: Eu, cq: 20, itu: 28,
        prefixes: ["LZ"],
        formats: [cf(&["LZ"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["SOFIA", "VARNA"],
    },
    ent! {
        dxcc: 206, name: "AUSTRIA", continent: Eu, cq: 15, itu: 28,
        prefixes: ["OE"],
        formats: [cf(&["OE"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["WIEN", "GRAZ"],
    },
    ent! {
        dxcc: 224, name: "FINLAND", continent: Eu, cq: 15, itu: 18,
        prefixes: ["OH", "OF", "OG", "OI", "OJ"],
        formats: [cf(&["OH"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["HELSINKI", "OULU"],
    },
    ent! {
        dxcc: 5, name: "ALAND IS", continent: Eu, cq: 15, itu: 18,
        prefixes: ["OH0", "OF0", "OG0", "OI0"],
        formats: [cf(&["OH0"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["MARIEHAMN"],
    },
    ent! {
        dxcc: 167, name: "MARKET REEF", continent: Eu, cq: 15, itu: 18,
        prefixes: ["OJ0"],
        formats: [cf(&["OJ0"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["MARKET REEF"],
    },
    ent! {
        dxcc: 503, name: "CZECH REP", continent: Eu, cq: 15, itu: 28,
        prefixes: ["OK", "OL"],
        formats: [cf(&["OK", "OL"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["PRAHA", "BRNO"],
    },
    ent! {
        dxcc: 504, name: "SLOVAKIA", continent: Eu, cq: 15, itu: 28,
        prefixes: ["OM"],
        formats: [cf(&["OM"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["BRATISLAVA", "KOSICE"],
    },
    ent! {
        dxcc: 209, name: "BELGIUM", continent: Eu, cq: 14, itu: 27,
        prefixes: ["ON", "OO", "OP", "OQ", "OR", "OS", "OT"],
        formats: [cf(&["ON", "OO", "OP", "OT"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["GENT", "BRUGGE"],
    },
    ent! {
        dxcc: 222, name: "FAROE IS", continent: Eu, cq: 14, itu: 18,
        prefixes: ["OY"],
        formats: [cf(&["OY"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["TORSHAVN"],
    },
    ent! {
        dxcc: 221, name: "DENMARK", continent: Eu, cq: 14, itu: 18,
        prefixes: ["OZ", "OU", "OV", "OW", "5P", "5Q"],
        formats: [cf(&["OZ", "OU"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["KOBENHAVN", "AARHUS"],
    },
    ent! {
        dxcc: 263, name: "NETHERLANDS", continent: Eu, cq: 14, itu: 27,
        prefixes: ["PA", "PB", "PC", "PD", "PE", "PF", "PG", "PH", "PI"],
        formats: [cf(&["PA", "PD", "PE", "PH"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["UTRECHT", "DELFT"],
    },
    ent! {
        dxcc: 61, name: "FRANZ JOSEF LAND", continent: Eu, cq: 40, itu: 75,
        prefixes: ["R1FJ", "RI1FJ", "UA1FJ"],
        formats: [cf(&["R1FJ"], DigitRule::InPrefix, (1, 2), 1)],
        tag: None, iso: None,
        towns: ["KRENKEL"],
    },
    ent! {
        dxcc: 151, name: "MALYJ VYSOTSKIJ", continent: Eu, cq: 16, itu: 29,
        prefixes: ["R1MV", "RI1MV", "UA1MV"],
        formats: [cf(&["R1MV"], DigitRule::InPrefix, (1, 1), 1)],
        tag: None, iso: None,
        towns: ["VYSOTSK"],
    },
    ent! {
        dxcc: 499, name: "SLOVENIA", continent: Eu, cq: 15, itu: 28,
        prefixes: ["S5"],
        formats: [cf(&["S5"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["LJUBLJANA"],
    },
    ent! {
        dxcc: 284, name: "SWEDEN", continent: Eu, cq: 14, itu: 18,
        prefixes: ["SA", "SB", "SC", "SD", "SE", "SF", "SG", "SH", "SI", "SJ", "SK", "SL", "SM",
            "7S", "8S"],
        formats: [cf(&["SM", "SA", "SK"], DigitRule::OneOf(&[0, 1, 2, 3, 4, 5, 6, 7]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["STOCKHOLM", "LUND"],
    },
    ent! {
        dxcc: 269, name: "POLAND", continent: Eu, cq: 15, itu: 28,
        prefixes: ["SN", "SO", "SP", "SQ", "SR", "3Z", "HF"],
        formats: [cf(&["SP", "SQ", "SO"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["KRAKOW", "GDANSK"],
    },
    ent! {
        dxcc: 236, name: "GREECE", continent: Eu, cq: 20, itu: 28,
        prefixes: ["SV", "SW", "SX", "SY", "SZ", "J4"],
        formats: [cf(&["SV", "SW", "SZ"], DigitRule::OneOf(&[1, 2, 3, 4, 6, 7, 8]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["ATHINA", "PATRA"],
    },
    ent! {
        dxcc: 180, name: "MOUNT ATHOS", continent: Eu, cq: 20, itu: 28,
        prefixes: ["SV2"],
        formats: [cf(&["SV2"], DigitRule::InPrefix, (3, 3), 1)],
        tag: Some("A"), iso: None,
        towns: ["KARYES"],
    },
    ent! {
        dxcc: 45, name: "DODECANESE", continent: Eu, cq: 20, itu: 28,
        prefixes: ["SV5", "SW5", "SX5", "SY5", "SZ5", "J45"],
        formats: [cf(&["SV5", "SX5"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["RODOS", "KOS"],
    },
    ent! {
        dxcc: 40, name: "CRETE", continent: Eu, cq: 20, itu: 28,
        prefixes: ["SV9", "SW9", "SX9", "SY9", "SZ9", "J49"],
        formats: [cf(&["SV9", "SW9"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["HERAKLION", "CHANIA"],
    },
    ent! {
        dxcc: 278, name: "SAN MARINO", continent: Eu, cq: 15, itu: 28,
        prefixes: ["T7"],
        formats: [cf(&["T7"], DigitRule::OneOf(&[0]), (1, 2), 1)],
        tag: None, iso: None,
        towns: ["SAN MARINO"],
    },
    ent! {
        dxcc: 390, name: "TURKEY", continent: Eu, cq: 20, itu: 39,
        prefixes: ["TA", "TB", "TC", "YM"],
        formats: [cf(&["TA", "TC", "YM"], DigitRule::OneOf(D0_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["ANKARA", "ISTANBUL"],
    },
    ent! {
        dxcc: 242, name: "ICELAND", continent: Eu, cq: 40, itu: 17,
        prefixes: ["TF"],
        formats: [cf(&["TF"], DigitRule::OneOf(&[1, 2, 3, 4, 5, 6, 7, 8]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["REYKJAVIK", "AKUREYRI"],
    },
    ent! {
        dxcc: 214, name: "CORSICA", continent: Eu, cq: 15, itu: 28,
        prefixes: ["TK"],
        formats: [cf(&["TK"], DigitRule::OneOf(D0_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["AJACCIO", "BASTIA"],
    },
    ent! {
        dxcc: 54, name: "EUROPEAN RUSSIA", continent: Eu, cq: 16, itu: 29,
        prefixes: ["R1", "R3", "R4", "R6", "R7", "RA1", "RA3", "RA4", "RA6", "RA7", "RB1", "RB3",
            "RB4", "RB6", "RB7", "RC1", "RC3", "RC4", "RC6", "RC7", "RD1", "RD3", "RD4", "RD6",
            "RD7", "RE1", "RE3", "RE4", "RE6", "RE7", "RF1", "RF3", "RF4", "RF6", "RF7", "RG1",
            "RG3", "RG4", "RG6", "RG7", "RH1", "RH3", "RH4", "RH6", "RH7", "RI1", "RI3", "RI4",
            "RI6", "RI7", "RJ1", "RJ3", "RJ4", "RJ6", "RJ7", "RK1", "RK3", "RK4", "RK6", "RK7",
            "RL1", "RL3", "RL4", "RL6", "RL7", "RM1", "RM3", "RM4", "RM6", "RM7", "RN1", "RN3",
            "RN4", "RN6", "RN7", "RO1", "RO3", "RO4", "RO6", "RO7", "RP1", "RP3", "RP4", "RP6",
            "RP7", "RQ1", "RQ3", "RQ4", "RQ6", "RQ7", "RR1", "RR3", "RR4", "RR6", "RR7", "RS1",
            "RS3", "RS4", "RS6", "RS7", "RT1", "RT3", "RT4", "RT6", "RT7", "RU1", "RU3", "RU4",
            "RU6", "RU7", "RV1", "RV3", "RV4", "RV6", "RV7", "RW1", "RW3", "RW4", "RW6", "RW7",
            "RX1", "RX3", "RX4", "RX6", "RX7", "RY1", "RY3", "RY4", "RY6", "RY7", "RZ1", "RZ3",
            "RZ4", "RZ6", "RZ7", "UA1", "UA3", "UA4", "UA6", "UA7", "UB1", "UB3", "UB4", "UB6",
            "UB7", "UC1", "UC3", "UC4", "UC6", "UC7", "UD1", "UD3", "UD4", "UD6", "UD7", "UE1",
            "UE3", "UE4", "UE6", "UE7", "UF1", "UF3", "UF4", "UF6", "UF7", "UG1", "UG3", "UG4",
            "UG6", "UG7", "UH1", "UH3", "UH4", "UH6", "UH7", "UI1", "UI3", "UI4", "UI6", "UI7"],
        formats: [
            cf(&["UA", "RA", "RW", "RN", "RV", "RU", "RX", "RZ", "R", "UB"],
                DigitRule::OneOf(&[1, 3, 4, 6, 7]), (2, 3), 1),
        ],
        tag: None, iso: None,
        towns: ["MOSKVA", "KAZAN"],
    },
    ent! {
        dxcc: 126, name: "KALININGRAD", continent: Eu, cq: 15, itu: 29,
        prefixes: ["R2", "RA2", "RB2", "RC2", "RD2", "RE2", "RF2", "RG2", "RH2", "RI2", "RJ2",
            "RK2", "RL2", "RM2", "RN2", "RO2", "RP2", "RQ2", "RR2", "RS2", "RT2", "RU2", "RV2",
            "RW2", "RX2", "RY2", "RZ2", "UA2", "UB2", "UC2", "UD2", "UE2", "UF2", "UG2", "UH2",
            "UI2"],
        formats: [
            cf(&["UA", "RA", "RW", "RN", "RV", "RU", "RX", "R", "UB"],
                DigitRule::OneOf(&[2]), (2, 3), 1),
        ],
        tag: None, iso: None,
        towns: ["KALININGRAD"],
    },
    ent! {
        dxcc: 288, name: "UKRAINE", continent: Eu, cq: 16, itu: 29,
        prefixes: ["UR", "US", "UT", "UU", "UV", "UW", "UX", "UY", "UZ", "EM", "EN", "EO"],
        formats: [
            cf(&["UR", "UT", "UX", "US", "UY", "UZ", "UW", "UV", "EM", "EO"],
                DigitRule::OneOf(D0_9), (2, 3), 1),
        ],
        tag: None, iso: None,
        towns: ["KYIV", "LVIV", "ODESA"],
    },
    ent! {
        dxcc: 145, name: "LATVIA", continent: Eu, cq: 15, itu: 29,
        prefixes: ["YL"],
        formats: [cf(&["YL"], DigitRule::OneOf(&[1, 2, 3]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["RIGA", "LIEPAJA"],
    },
    ent! {
        dxcc: 275, name: "ROMANIA", continent: Eu, cq: 20, itu: 28,
        prefixes: ["YO", "YP", "YQ", "YR"],
        formats: [cf(&["YO"], DigitRule::OneOf(&[2, 3, 4, 5, 6, 7, 8, 9]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["BUCURESTI", "CLUJ"],
    },
    ent! {
        dxcc: 296, name: "SERBIA", continent: Eu, cq: 15, itu: 28,
        prefixes: ["YT", "YU"],
        formats: [cf(&["YT", "YU"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["BEOGRAD", "NIS"],
    },
    ent! {
        dxcc: 502, name: "NORTH MACEDONIA", continent: Eu, cq: 15, itu: 28,
        prefixes: ["Z3"],
        formats: [cf(&["Z3"], DigitRule::OneOf(D0_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["SKOPJE"],
    },
    ent! {
        dxcc: 522, name: "KOSOVO", continent: Eu, cq: 15, itu: 28,
        prefixes: ["Z6"],
        formats: [cf(&["Z6"], DigitRule::OneOf(D0_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["PRISHTINA"],
    },
    ent! {
        dxcc: 7, name: "ALBANIA", continent: Eu, cq: 15, itu: 28,
        prefixes: ["ZA"],
        formats: [cf(&["ZA"], DigitRule::OneOf(&[1]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["TIRANA"],
    },
    ent! {
        dxcc: 233, name: "GIBRALTAR", continent: Eu, cq: 14, itu: 37,
        prefixes: ["ZB2", "ZG2"],
        formats: [cf(&["ZB2"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["GIBRALTAR"],
    },
    // North America -----------------------------------------------------------------------------
    ent! {
        dxcc: 1, name: "CANADA", continent: Na, cq: 4, itu: 4,
        prefixes: ["VE", "VA", "VO", "VY", "VB", "VC", "VD", "VG", "VX", "XJ", "XK", "XL", "XM",
            "XN", "XO", "CF", "CG", "CH", "CI", "CJ", "CK", "CY"],
        formats: [
            cf(&["VE"], DigitRule::OneOf(D1_9), (2, 3), 6),
            cf(&["VA"], DigitRule::OneOf(&[1, 2, 3, 4, 5, 6, 7]), (2, 3), 3),
            cf(&["VO"], DigitRule::OneOf(&[1, 2]), (1, 3), 1),
            cf(&["VY"], DigitRule::OneOf(&[1, 2]), (1, 3), 1),
        ],
        tag: None, iso: Some("CA"),
        towns: ["TORONTO", "OTTAWA", "CALGARY"],
    },
    ent! {
        dxcc: 211, name: "SABLE IS", continent: Na, cq: 5, itu: 9,
        prefixes: ["CY0"],
        formats: [cf(&["CY0"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["SABLE IS"],
    },
    ent! {
        dxcc: 252, name: "ST PAUL IS", continent: Na, cq: 5, itu: 9,
        prefixes: ["CY9"],
        formats: [cf(&["CY9"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["ST PAUL IS"],
    },
    ent! {
        dxcc: 6, name: "ALASKA", continent: Na, cq: 1, itu: 1,
        prefixes: ["KL", "AL", "NL", "WL"],
        formats: [cf(&["KL", "AL", "NL", "WL"], DigitRule::OneOf(&[7]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["ANCHORAGE", "FAIRBANKS", "JUNEAU"],
    },
    ent! {
        dxcc: 182, name: "NAVASSA IS", continent: Na, cq: 8, itu: 11,
        prefixes: ["KP1", "NP1", "WP1"],
        formats: [cf(&["KP1"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["NAVASSA"],
    },
    ent! {
        dxcc: 285, name: "US VIRGIN IS", continent: Na, cq: 8, itu: 11,
        prefixes: ["KP2", "NP2", "WP2"],
        formats: [cf(&["KP2", "NP2", "WP2"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["ST THOMAS", "ST CROIX"],
    },
    ent! {
        dxcc: 202, name: "PUERTO RICO", continent: Na, cq: 8, itu: 11,
        prefixes: ["KP3", "KP4", "NP3", "NP4", "WP3", "WP4"],
        formats: [cf(&["KP4", "NP4", "WP4", "KP3"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["SAN JUAN", "PONCE"],
    },
    ent! {
        dxcc: 43, name: "DESECHEO IS", continent: Na, cq: 8, itu: 11,
        prefixes: ["KP5", "NP5", "WP5"],
        formats: [cf(&["KP5"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["DESECHEO"],
    },
    ent! {
        dxcc: 105, name: "GUANTANAMO BAY", continent: Na, cq: 8, itu: 11,
        prefixes: ["KG4"],
        formats: [cf(&["KG4"], DigitRule::InPrefix, (2, 2), 1)],
        tag: None, iso: None,
        towns: ["GUANTANAMO"],
    },
    ent! {
        dxcc: 289, name: "UN HQ", continent: Na, cq: 5, itu: 8,
        prefixes: ["4U1UN"],
        formats: [cf(&["4U1UN"], DigitRule::InPrefix, (0, 0), 1)],
        tag: None, iso: None,
        towns: ["NEW YORK"],
    },
    ent! {
        dxcc: 64, name: "BERMUDA", continent: Na, cq: 5, itu: 11,
        prefixes: ["VP9"],
        formats: [cf(&["VP9"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["HAMILTON"],
    },
    ent! {
        dxcc: 277, name: "ST PIERRE", continent: Na, cq: 5, itu: 9,
        prefixes: ["FP"],
        formats: [cf(&["FP"], DigitRule::OneOf(&[5, 8]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["ST PIERRE"],
    },
    ent! {
        dxcc: 237, name: "GREENLAND", continent: Na, cq: 40, itu: 5,
        prefixes: ["OX", "XP"],
        formats: [cf(&["OX"], DigitRule::OneOf(&[3, 5, 7]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["NUUK", "ILULISSAT"],
    },
    ent! {
        dxcc: 60, name: "BAHAMAS", continent: Na, cq: 8, itu: 11,
        prefixes: ["C6"],
        formats: [cf(&["C6"], DigitRule::InPrefix, (3, 3), 1)],
        tag: None, iso: None,
        towns: ["NASSAU", "FREEPORT"],
    },
    ent! {
        dxcc: 70, name: "CUBA", continent: Na, cq: 8, itu: 11,
        prefixes: ["CM", "CO", "CL", "T4"],
        formats: [cf(&["CO", "CM", "CL"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["HABANA", "SANTIAGO"],
    },
    ent! {
        dxcc: 82, name: "JAMAICA", continent: Na, cq: 8, itu: 11,
        prefixes: ["6Y"],
        formats: [cf(&["6Y"], DigitRule::OneOf(&[5]), (2, 2), 1)],
        tag: None, iso: None,
        towns: ["KINGSTON"],
    },
    ent! {
        dxcc: 78, name: "HAITI", continent: Na, cq: 8, itu: 11,
        prefixes: ["HH", "4V"],
        formats: [cf(&["HH"], DigitRule::OneOf(&[2]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["PORT AU PRINCE"],
    },
    ent! {
        dxcc: 72, name: "DOMINICAN REP", continent: Na, cq: 8, itu: 11,
        prefixes: ["HI"],
        formats: [cf(&["HI"], DigitRule::OneOf(&[3, 8]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["SANTO DOMINGO"],
    },
    ent! {
        dxcc: 69, name: "CAYMAN IS", continent: Na, cq: 8, itu: 11,
        prefixes: ["ZF"],
        formats: [cf(&["ZF"], DigitRule::OneOf(&[1, 2]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["GEORGE TOWN"],
    },
    ent! {
        dxcc: 89, name: "TURKS CAICOS", continent: Na, cq: 8, itu: 11,
        prefixes: ["VP5"],
        formats: [cf(&["VP5"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["PROVIDENCIALES"],
    },
    ent! {
        dxcc: 94, name: "ANTIGUA", continent: Na, cq: 8, itu: 11,
        prefixes: ["V2"],
        formats: [cf(&["V2"], DigitRule::OneOf(&[1, 5]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["ST JOHNS"],
    },
    ent! {
        dxcc: 249, name: "ST KITTS", continent: Na, cq: 8, itu: 11,
        prefixes: ["V4"],
        formats: [cf(&["V4"], DigitRule::OneOf(&[1]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["BASSETERRE"],
    },
    ent! {
        dxcc: 12, name: "ANGUILLA", continent: Na, cq: 8, itu: 11,
        prefixes: ["VP2E"],
        formats: [cf(&["VP2E"], DigitRule::InPrefix, (1, 2), 1)],
        tag: None, iso: None,
        towns: ["THE VALLEY"],
    },
    ent! {
        dxcc: 96, name: "MONTSERRAT", continent: Na, cq: 8, itu: 11,
        prefixes: ["VP2M"],
        formats: [cf(&["VP2M"], DigitRule::InPrefix, (1, 2), 1)],
        tag: None, iso: None,
        towns: ["BRADES"],
    },
    ent! {
        dxcc: 65, name: "BRITISH VIRGIN IS", continent: Na, cq: 8, itu: 11,
        prefixes: ["VP2V"],
        formats: [cf(&["VP2V"], DigitRule::InPrefix, (1, 2), 1)],
        tag: None, iso: None,
        towns: ["ROAD TOWN"],
    },
    ent! {
        dxcc: 79, name: "GUADELOUPE", continent: Na, cq: 8, itu: 11,
        prefixes: ["FG"],
        formats: [cf(&["FG"], DigitRule::OneOf(&[4, 5, 8]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["POINTE A PITRE"],
    },
    ent! {
        dxcc: 84, name: "MARTINIQUE", continent: Na, cq: 8, itu: 11,
        prefixes: ["FM"],
        formats: [cf(&["FM"], DigitRule::OneOf(&[5]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["FORT DE FRANCE"],
    },
    ent! {
        dxcc: 516, name: "ST BARTHELEMY", continent: Na, cq: 8, itu: 11,
        prefixes: ["FJ"],
        formats: [cf(&["FJ"], DigitRule::OneOf(&[5]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["GUSTAVIA"],
    },
    ent! {
        dxcc: 213, name: "ST MARTIN", continent: Na, cq: 8, itu: 11,
        prefixes: ["FS"],
        formats: [cf(&["FS"], DigitRule::OneOf(&[4, 5]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["MARIGOT"],
    },
    ent! {
        dxcc: 518, name: "SINT MAARTEN", continent: Na, cq: 8, itu: 11,
        prefixes: ["PJ7"],
        formats: [cf(&["PJ7"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["PHILIPSBURG"],
    },
    ent! {
        dxcc: 519, name: "SABA ST EUSTATIUS", continent: Na, cq: 8, itu: 11,
        prefixes: ["PJ5", "PJ6"],
        formats: [cf(&["PJ5", "PJ6"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["THE BOTTOM", "ORANJESTAD"],
    },
    ent! {
        dxcc: 95, name: "DOMINICA", continent: Na, cq: 8, itu: 11,
        prefixes: ["J7"],
        formats: [cf(&["J7"], DigitRule::OneOf(&[3]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["ROSEAU"],
    },
    ent! {
        dxcc: 97, name: "ST LUCIA", continent: Na, cq: 8, itu: 11,
        prefixes: ["J6"],
        formats: [cf(&["J6"], DigitRule::OneOf(&[8]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["CASTRIES"],
    },
    ent! {
        dxcc: 98, name: "ST VINCENT", continent: Na, cq: 8, itu: 11,
        prefixes: ["J8"],
        formats: [cf(&["J8"], DigitRule::OneOf(&[8]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["KINGSTOWN"],
    },
    ent! {
        dxcc: 62, name: "BARBADOS", continent: Na, cq: 8, itu: 11,
        prefixes: ["8P"],
        formats: [cf(&["8P"], DigitRule::OneOf(&[6]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["BRIDGETOWN"],
    },
    ent! {
        dxcc: 77, name: "GRENADA", continent: Na, cq: 8, itu: 11,
        prefixes: ["J3"],
        formats: [cf(&["J3"], DigitRule::OneOf(&[4]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["ST GEORGES"],
    },
    ent! {
        dxcc: 17, name: "AVES IS", continent: Na, cq: 8, itu: 11,
        prefixes: ["YV0"],
        formats: [cf(&["YV0"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["AVES"],
    },
    ent! {
        dxcc: 50, name: "MEXICO", continent: Na, cq: 6, itu: 10,
        prefixes: ["XA", "XB", "XC", "XD", "XE", "XF", "XG", "XH", "XI", "4A", "4B", "4C", "6D",
            "6E", "6F", "6G", "6H", "6I", "6J"],
        formats: [cf(&["XE", "XF"], DigitRule::OneOf(&[1, 2, 3]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["PUEBLA", "MERIDA"],
    },
    ent! {
        dxcc: 204, name: "REVILLAGIGEDO", continent: Na, cq: 6, itu: 10,
        prefixes: ["XA4", "XB4", "XC4", "XD4", "XE4", "XF4", "4A4", "4B4"],
        formats: [cf(&["XF4"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["SOCORRO"],
    },
    ent! {
        dxcc: 76, name: "GUATEMALA", continent: Na, cq: 7, itu: 11,
        prefixes: ["TG", "TD"],
        formats: [cf(&["TG"], DigitRule::OneOf(&[9]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["GUATEMALA", "ANTIGUA"],
    },
    ent! {
        dxcc: 66, name: "BELIZE", continent: Na, cq: 7, itu: 11,
        prefixes: ["V3"],
        formats: [cf(&["V3"], DigitRule::OneOf(&[1]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["BELMOPAN"],
    },
    ent! {
        dxcc: 74, name: "EL SALVADOR", continent: Na, cq: 7, itu: 11,
        prefixes: ["YS", "HU"],
        formats: [cf(&["YS"], DigitRule::OneOf(&[1]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["SAN SALVADOR"],
    },
    ent! {
        dxcc: 80, name: "HONDURAS", continent: Na, cq: 7, itu: 11,
        prefixes: ["HR", "HQ"],
        formats: [cf(&["HR"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["TEGUCIGALPA"],
    },
    ent! {
        dxcc: 86, name: "NICARAGUA", continent: Na, cq: 7, itu: 11,
        prefixes: ["YN", "H6", "H7", "HT"],
        formats: [cf(&["YN"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["MANAGUA"],
    },
    ent! {
        dxcc: 308, name: "COSTA RICA", continent: Na, cq: 7, itu: 11,
        prefixes: ["TI", "TE"],
        formats: [cf(&["TI"], DigitRule::OneOf(&[2, 3, 4, 5, 8]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["SAN JOSE"],
    },
    ent! {
        dxcc: 37, name: "COCOS IS", continent: Na, cq: 7, itu: 11,
        prefixes: ["TI9", "TE9"],
        formats: [cf(&["TI9"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["COCOS"],
    },
    ent! {
        dxcc: 88, name: "PANAMA", continent: Na, cq: 7, itu: 11,
        prefixes: ["HP", "HO", "3E", "3F", "H3", "H8", "H9"],
        formats: [cf(&["HP"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["PANAMA"],
    },
    ent! {
        dxcc: 216, name: "SAN ANDRES", continent: Na, cq: 7, itu: 11,
        prefixes: ["HK0", "5J0", "5K0"],
        formats: [cf(&["HK0"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["SAN ANDRES"],
    },
    ent! {
        dxcc: 36, name: "CLIPPERTON", continent: Na, cq: 7, itu: 10,
        prefixes: ["FO"],
        formats: [cf(&["FO"], DigitRule::OneOf(&[0]), (2, 3), 1)],
        tag: Some("C"), iso: None,
        towns: ["CLIPPERTON"],
    },
    // South America -----------------------------------------------------------------------------
    ent! {
        dxcc: 100, name: "ARGENTINA", continent: Sa, cq: 13, itu: 14,
        prefixes: ["LU", "LO", "LP", "LQ", "LR", "LS", "LT", "LV", "LW", "AY", "AZ", "L2", "L3",
            "L4", "L5", "L6", "L7", "L8", "L9"],
        formats: [cf(&["LU", "LW"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["CORDOBA", "MENDOZA"],
    },
    ent! {
        dxcc: 112, name: "CHILE", continent: Sa, cq: 12, itu: 14,
        prefixes: ["CE", "CA", "CB", "CC", "CD", "XQ", "XR", "3G"],
        formats: [cf(&["CE", "CA", "XQ"], DigitRule::OneOf(&[1, 2, 3, 4, 5, 6, 7, 8]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["SANTIAGO", "VALPARAISO"],
    },
    ent! {
        dxcc: 47, name: "EASTER IS", continent: Sa, cq: 12, itu: 63,
        prefixes: ["CE0Y", "XR0Y", "CA0Y", "3G0Y"],
        formats: [cf(&["CE0Y", "XR0Y"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["HANGA ROA"],
    },
    ent! {
        dxcc: 125, name: "SAN FELIX", continent: Sa, cq: 12, itu: 14,
        prefixes: ["CE0X", "XR0X", "CA0X", "3G0X"],
        formats: [cf(&["CE0X", "XR0X"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["SAN FELIX"],
    },
    ent! {
        dxcc: 217, name: "JUAN FERNANDEZ", continent: Sa, cq: 12, itu: 14,
        prefixes: ["CE0Z", "XR0Z", "CA0Z", "3G0Z"],
        formats: [cf(&["CE0Z", "XR0Z"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["ROBINSON CRUSOE"],
    },
    ent! {
        dxcc: 144, name: "URUGUAY", continent: Sa, cq: 13, itu: 14,
        prefixes: ["CX", "CV", "CW"],
        formats: [cf(&["CX"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["MONTEVIDEO"],
    },
    ent! {
        dxcc: 132, name: "PARAGUAY", continent: Sa, cq: 11, itu: 14,
        prefixes: ["ZP"],
        formats: [cf(&["ZP"], DigitRule::OneOf(&[5, 6]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["ASUNCION"],
    },
    ent! {
        dxcc: 104, name: "BOLIVIA", continent: Sa, cq: 10, itu: 12,
        prefixes: ["CP"],
        formats: [cf(&["CP"], DigitRule::OneOf(&[1, 2, 3, 4, 5, 6, 7]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["LA PAZ", "SUCRE"],
    },
    ent! {
        dxcc: 136, name: "PERU", continent: Sa, cq: 10, itu: 12,
        prefixes: ["OA", "OB", "OC", "4T"],
        formats: [cf(&["OA"], DigitRule::OneOf(&[4]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["LIMA", "CUSCO"],
    },
    ent! {
        dxcc: 120, name: "ECUADOR", continent: Sa, cq: 10, itu: 12,
        prefixes: ["HC", "HD"],
        formats: [cf(&["HC", "HD"], DigitRule::OneOf(&[1, 2, 3, 4, 5, 6, 7]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["QUITO", "GUAYAQUIL"],
    },
    ent! {
        dxcc: 71, name: "GALAPAGOS", continent: Sa, cq: 10, itu: 12,
        prefixes: ["HC8", "HD8"],
        formats: [cf(&["HC8", "HD8"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["PUERTO AYORA"],
    },
    ent! {
        dxcc: 116, name: "COLOMBIA", continent: Sa, cq: 9, itu: 12,
        prefixes: ["HK", "HJ", "5J", "5K"],
        formats: [cf(&["HK", "HJ"], DigitRule::OneOf(&[1, 2, 3, 4, 5, 6, 7]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["BOGOTA", "MEDELLIN"],
    },
    ent! {
        dxcc: 161, name: "MALPELO", continent: Sa, cq: 9, itu: 12,
        prefixes: ["HK0"],
        formats: [cf(&["HK0"], DigitRule::InPrefix, (2, 3), 1)],
        tag: Some("M"), iso: None,
        towns: ["MALPELO"],
    },
    ent! {
        dxcc: 148, name: "VENEZUELA", continent: Sa, cq: 9, itu: 12,
        prefixes: ["YV", "YW", "YX", "YY", "4M"],
        formats: [cf(&["YV"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["CARACAS"],
    },
    ent! {
        dxcc: 129, name: "GUYANA", continent: Sa, cq: 9, itu: 12,
        prefixes: ["8R"],
        formats: [cf(&["8R"], DigitRule::OneOf(&[1]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["GEORGETOWN"],
    },
    ent! {
        dxcc: 140, name: "SURINAME", continent: Sa, cq: 9, itu: 12,
        prefixes: ["PZ"],
        formats: [cf(&["PZ"], DigitRule::OneOf(&[1]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["PARAMARIBO"],
    },
    ent! {
        dxcc: 63, name: "FRENCH GUIANA", continent: Sa, cq: 9, itu: 12,
        prefixes: ["FY"],
        formats: [cf(&["FY"], DigitRule::OneOf(&[1, 5]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["CAYENNE", "KOUROU"],
    },
    ent! {
        dxcc: 108, name: "BRAZIL", continent: Sa, cq: 11, itu: 15,
        prefixes: ["PP", "PQ", "PR", "PS", "PT", "PU", "PV", "PW", "PX", "PY", "ZV", "ZW", "ZX",
            "ZY", "ZZ"],
        formats: [cf(&["PY", "PU", "PP", "PT", "PR"], DigitRule::OneOf(D1_9), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["RIO", "RECIFE"],
    },
    ent! {
        dxcc: 56, name: "FERNANDO DE NORONHA", continent: Sa, cq: 11, itu: 13,
        prefixes: ["PY0F", "PP0F", "PU0F", "ZY0F"],
        formats: [cf(&["PY0F", "PP0F"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["NORONHA"],
    },
    ent! {
        dxcc: 253, name: "ST PETER ST PAUL", continent: Sa, cq: 11, itu: 13,
        prefixes: ["PY0S", "PW0S", "PP0S"],
        formats: [cf(&["PY0S", "PW0S"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["ST PETER ST PAUL"],
    },
    ent! {
        dxcc: 273, name: "TRINDADE", continent: Sa, cq: 11, itu: 15,
        prefixes: ["PY0T", "PP0T", "PW0T"],
        formats: [cf(&["PY0T", "PP0T"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["TRINDADE"],
    },
    ent! {
        dxcc: 90, name: "TRINIDAD TOBAGO", continent: Sa, cq: 9, itu: 11,
        prefixes: ["9Y", "9Z"],
        formats: [cf(&["9Y"], DigitRule::OneOf(&[4]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["PORT OF SPAIN"],
    },
    ent! {
        dxcc: 91, name: "ARUBA", continent: Sa, cq: 9, itu: 11,
        prefixes: ["P4"],
        formats: [cf(&["P4"], DigitRule::OneOf(&[0]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["ORANJESTAD"],
    },
    ent! {
        dxcc: 517, name: "CURACAO", continent: Sa, cq: 9, itu: 11,
        prefixes: ["PJ2"],
        formats: [cf(&["PJ2"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["WILLEMSTAD"],
    },
    ent! {
        dxcc: 520, name: "BONAIRE", continent: Sa, cq: 9, itu: 11,
        prefixes: ["PJ4"],
        formats: [cf(&["PJ4"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["KRALENDIJK"],
    },
    ent! {
        dxcc: 141, name: "FALKLAND IS", continent: Sa, cq: 13, itu: 16,
        prefixes: ["VP8"],
        formats: [cf(&["VP8"], DigitRule::InPrefix, (2, 3), 1)],
        tag: None, iso: None,
        towns: ["STANLEY"],
    },
    ent! {
        dxcc: 235, name: "SOUTH GEORGIA", continent: Sa, cq: 13, itu: 73,
        prefixes: ["VP8"],
        formats: [cf(&["VP8"], DigitRule::InPrefix, (2, 3), 1)],
        tag: Some("G"), iso: None,
        towns: ["GRYTVIKEN"],
    },
    ent! {
        dxcc: 238, name: "SOUTH ORKNEY", continent: Sa, cq: 13, itu: 73,
        prefixes: ["VP8"],
        formats: [cf(&["VP8"], DigitRule::InPrefix, (2, 3), 1)],
        tag: Some("O"), iso: None,
        towns: ["SIGNY"],
    },
    ent! {
        dxcc: 240, name: "SOUTH SANDWICH", continent: Sa, cq: 13, itu: 73,
        prefixes: ["VP8"],
        formats: [cf(&["VP8"], DigitRule::InPrefix, (2, 3), 1)],
        tag: Some("S"), iso: None,
        towns: ["THULE"],
    },
    ent! {
        dxcc: 241, name: "SOUTH SHETLAND", continent: Sa, cq: 13, itu: 73,
        prefixes: ["VP8"],
        formats: [cf(&["VP8"], DigitRule::InPrefix, (2, 3), 1)],
        tag: Some("H"), iso: None,
        towns: ["KING GEORGE IS"],
    },
    // Africa ------------------------------------------------------------------------------------
    ent! {
        dxcc: 462, name: "SOUTH AFRICA", continent: Af, cq: 38, itu: 57,
        prefixes: ["ZS", "ZR", "ZT", "ZU", "S8"],
        formats: [cf(&["ZS", "ZR"], DigitRule::OneOf(&[1, 2, 3, 4, 5, 6]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["DURBAN", "PRETORIA"],
    },
    // Asia --------------------------------------------------------------------------------------
    ent! {
        dxcc: 339, name: "JAPAN", continent: As, cq: 25, itu: 45,
        prefixes: ["JA", "JB", "JC", "JD", "JE", "JF", "JG", "JH", "JI", "JJ", "JK", "JL", "JM",
            "JN", "JO", "JP", "JQ", "JR", "JS", "7J", "7K", "7L", "7M", "7N", "8J", "8K", "8L",
            "8M", "8N"],
        formats: [
            cf(&["JA", "JE", "JF", "JG", "JH", "JI", "JJ", "JK", "JL", "JM", "JN", "JO", "JP",
                "JQ", "JR"], DigitRule::OneOf(D0_9), (3, 3), 8),
            cf(&["7K", "7L", "7M", "7N"], DigitRule::OneOf(D0_9), (3, 3), 2),
            cf(&["JA"], DigitRule::OneOf(D0_9), (2, 2), 1),
        ],
        tag: None, iso: Some("JP"),
        towns: ["TOKYO", "OSAKA", "SAPPORO"],
    },
    // Oceania -----------------------------------------------------------------------------------
    ent! {
        dxcc: 150, name: "AUSTRALIA", continent: Oc, cq: 30, itu: 59,
        prefixes: ["VK", "AX", "VH", "VI", "VJ", "VL", "VM", "VN", "VZ"],
        formats: [cf(&["VK"], DigitRule::OneOf(&[1, 2, 3, 4, 5, 6, 7, 8]), (2, 3), 1)],
        tag: None, iso: Some("AU"),
        towns: ["SYDNEY", "PERTH", "HOBART"],
    },
    ent! {
        dxcc: 170, name: "NEW ZEALAND", continent: Oc, cq: 32, itu: 60,
        prefixes: ["ZL", "ZK", "ZM"],
        formats: [cf(&["ZL", "ZM"], DigitRule::OneOf(&[1, 2, 3, 4]), (2, 3), 1)],
        tag: None, iso: None,
        towns: ["AUCKLAND", "NELSON"],
    },
];

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;

    #[test]
    fn dxcc_numbers_are_unique() {
        let mut seen = HashSet::new();
        for e in WORLD {
            assert!(seen.insert(e.dxcc), "{e:?}");
        }
    }

    #[test]
    fn no_prefix_belongs_to_two_entities() {
        let mut owner: HashMap<&str, u16> = HashMap::new();
        for e in WORLD.iter().filter(|e| e.tag.is_none()) {
            for p in e.prefixes {
                if let Some(o) = owner.insert(p, e.dxcc) {
                    panic!("{p} in both {o} and {}", e.dxcc);
                }
            }
        }
    }

    #[test]
    fn zones_are_in_range() {
        for e in WORLD {
            assert!((1..=40).contains(&e.cq), "{e:?}");
            assert!((1..=90).contains(&e.itu), "{e:?}");
            for r in e.regions {
                assert!((1..=40).contains(&r.cq), "{e:?} {r:?}");
                assert!((1..=90).contains(&r.itu), "{e:?} {r:?}");
            }
        }
    }

    #[test]
    fn formats_are_drawable() {
        for e in WORLD {
            assert!(!e.prefixes.is_empty(), "{e:?}");
            assert!(!e.formats.is_empty(), "{e:?}");
            for f in e.formats {
                assert!(f.weight > 0, "{e:?}");
                assert!(!f.prefixes.is_empty(), "{e:?}");
                assert!(f.suffix.0 <= f.suffix.1 && f.suffix.1 <= 3, "{e:?}");
                // A bare prefix is a whole call only for a one-off station.
                if f.suffix.0 == 0 {
                    assert!(matches!(f.digit, DigitRule::InPrefix), "{e:?}");
                }
                match f.digit {
                    DigitRule::FromRegion => assert!(!e.regions.is_empty(), "{e:?}"),
                    DigitRule::OneOf(d) => assert!(!d.is_empty() && d.iter().all(|d| *d <= 9)),
                    DigitRule::InPrefix => {
                        assert!(f
                            .prefixes
                            .iter()
                            .all(|p| p.bytes().any(|b| b.is_ascii_digit())))
                    }
                }
            }
            // Something to say for QTH: towns, or subdivisions with towns.
            if e.regions.is_empty() {
                assert!(!e.towns.is_empty(), "{e:?}");
            }
            let mut digits = HashSet::new();
            for r in e.regions {
                assert!(digits.insert(r.digit), "{e:?} {r:?}");
                assert!(!r.subdivisions.is_empty(), "{e:?} {r:?}");
                assert!(r.subdivisions.iter().all(|(_, t)| !t.is_empty()));
            }
        }
    }

    #[test]
    fn tags_qualify_a_parent_block() {
        for e in WORLD.iter().filter(|e| e.tag.is_some()) {
            assert!(!e.prefixes.is_empty(), "{e:?}");
            for f in e.formats {
                for p in f.prefixes {
                    assert!(e.prefixes.iter().any(|q| p.starts_with(q)), "{e:?} {p}");
                }
            }
        }
        let mut pairs = HashSet::new();
        for e in WORLD.iter().filter(|e| e.tag.is_some()) {
            for p in e.prefixes {
                assert!(pairs.insert((*p, e.tag)), "{e:?}");
            }
        }
    }

    #[test]
    fn us_lower_48_never_issues_pacific_alaska_or_caribbean_blocks() {
        for f in by_dxcc(USA).formats {
            for p in f.prefixes {
                let b = p.as_bytes();
                if b.len() == 2 {
                    assert!(!matches!(b[1], b'H' | b'L' | b'P'), "{p}");
                }
            }
        }
    }

    #[test]
    fn region_lookup_by_digit() {
        let us = by_dxcc(USA);
        assert_eq!(us.region(6).unwrap().cq, 3);
        assert_eq!(us.region(1).unwrap().cq, 5);
        assert!(by_dxcc(230).region(1).is_none());
    }

    #[test]
    fn tables_are_encodable() {
        use crate::codec::tokenize;
        for e in WORLD {
            let mut all: Vec<&str> = vec![e.name];
            all.extend(e.towns.iter());
            all.extend(e.prefixes.iter());
            for f in e.formats {
                all.extend(f.prefixes.iter());
            }
            for r in e.regions {
                for (s, towns) in r.subdivisions {
                    all.push(s);
                    all.extend(towns.iter());
                }
            }
            for s in all {
                assert!(tokenize(s).is_ok(), "{s}");
            }
        }
    }
}
