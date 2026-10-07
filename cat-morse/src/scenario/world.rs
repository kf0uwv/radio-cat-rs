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
        iso: Some("US"),
        towns: &[],
    },
    Entity {
        dxcc: 223,
        name: "ENGLAND",
        continent: Continent::Eu,
        cq: 14,
        itu: 27,
        prefixes: &["G", "M"],
        formats: &[CallFormat {
            prefixes: &["G", "M"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (3, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["LONDON", "BRISTOL", "YORK", "LEEDS"],
    },
    Entity {
        dxcc: 230,
        name: "GERMANY",
        continent: Continent::Eu,
        cq: 14,
        itu: 28,
        prefixes: &["DL", "DK", "DJ", "DF"],
        formats: &[CallFormat {
            prefixes: &["DL", "DK", "DJ", "DF"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: Some("DE"),
        towns: &["BERLIN", "MUNICH", "HAMBURG", "BONN"],
    },
    Entity {
        dxcc: 227,
        name: "FRANCE",
        continent: Continent::Eu,
        cq: 14,
        itu: 27,
        prefixes: &["F"],
        formats: &[CallFormat {
            prefixes: &["F"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (3, 3),
            weight: 1,
        }],
        regions: &[],
        iso: Some("FR"),
        towns: &["PARIS", "LYON", "NANTES"],
    },
    Entity {
        dxcc: 248,
        name: "ITALY",
        continent: Continent::Eu,
        cq: 15,
        itu: 28,
        prefixes: &["I", "IK", "IZ"],
        formats: &[CallFormat {
            prefixes: &["I", "IK", "IZ"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: Some("IT"),
        towns: &["ROMA", "MILANO", "TORINO"],
    },
    Entity {
        dxcc: 281,
        name: "SPAIN",
        continent: Continent::Eu,
        cq: 14,
        itu: 37,
        prefixes: &["EA"],
        formats: &[CallFormat {
            prefixes: &["EA"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: Some("ES"),
        towns: &["MADRID", "SEVILLA", "VALENCIA"],
    },
    Entity {
        dxcc: 209,
        name: "BELGIUM",
        continent: Continent::Eu,
        cq: 14,
        itu: 27,
        prefixes: &["ON"],
        formats: &[CallFormat {
            prefixes: &["ON"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["GENT", "BRUGGE"],
    },
    Entity {
        dxcc: 263,
        name: "NETHERLANDS",
        continent: Continent::Eu,
        cq: 14,
        itu: 27,
        prefixes: &["PA", "PD"],
        formats: &[CallFormat {
            prefixes: &["PA", "PD"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["UTRECHT", "DELFT"],
    },
    Entity {
        dxcc: 224,
        name: "FINLAND",
        continent: Continent::Eu,
        cq: 15,
        itu: 18,
        prefixes: &["OH"],
        formats: &[CallFormat {
            prefixes: &["OH"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["HELSINKI", "OULU"],
    },
    Entity {
        dxcc: 284,
        name: "SWEDEN",
        continent: Continent::Eu,
        cq: 14,
        itu: 18,
        prefixes: &["SM"],
        formats: &[CallFormat {
            prefixes: &["SM"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["STOCKHOLM", "LUND"],
    },
    Entity {
        dxcc: 266,
        name: "NORWAY",
        continent: Continent::Eu,
        cq: 14,
        itu: 18,
        prefixes: &["LA"],
        formats: &[CallFormat {
            prefixes: &["LA"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["OSLO", "BERGEN"],
    },
    Entity {
        dxcc: 503,
        name: "CZECH REP",
        continent: Continent::Eu,
        cq: 15,
        itu: 28,
        prefixes: &["OK"],
        formats: &[CallFormat {
            prefixes: &["OK"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["PRAHA", "BRNO"],
    },
    Entity {
        dxcc: 269,
        name: "POLAND",
        continent: Continent::Eu,
        cq: 15,
        itu: 28,
        prefixes: &["SP"],
        formats: &[CallFormat {
            prefixes: &["SP"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["KRAKOW", "GDANSK"],
    },
    Entity {
        dxcc: 239,
        name: "HUNGARY",
        continent: Continent::Eu,
        cq: 15,
        itu: 28,
        prefixes: &["HA"],
        formats: &[CallFormat {
            prefixes: &["HA"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["BUDAPEST", "PECS"],
    },
    Entity {
        dxcc: 499,
        name: "SLOVENIA",
        continent: Continent::Eu,
        cq: 15,
        itu: 28,
        prefixes: &["S5"],
        formats: &[CallFormat {
            prefixes: &["S5"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["LJUBLJANA"],
    },
    Entity {
        dxcc: 497,
        name: "CROATIA",
        continent: Continent::Eu,
        cq: 15,
        itu: 28,
        prefixes: &["9A"],
        formats: &[CallFormat {
            prefixes: &["9A"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["ZAGREB", "SPLIT"],
    },
    Entity {
        dxcc: 245,
        name: "IRELAND",
        continent: Continent::Eu,
        cq: 14,
        itu: 27,
        prefixes: &["EI"],
        formats: &[CallFormat {
            prefixes: &["EI"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["DUBLIN", "CORK"],
    },
    Entity {
        dxcc: 279,
        name: "SCOTLAND",
        continent: Continent::Eu,
        cq: 14,
        itu: 27,
        prefixes: &["GM", "MM"],
        formats: &[CallFormat {
            prefixes: &["GM", "MM"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (3, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["EDINBURGH", "GLASGOW"],
    },
    Entity {
        dxcc: 206,
        name: "AUSTRIA",
        continent: Continent::Eu,
        cq: 15,
        itu: 28,
        prefixes: &["OE"],
        formats: &[CallFormat {
            prefixes: &["OE"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["WIEN", "GRAZ"],
    },
    Entity {
        dxcc: 287,
        name: "SWITZERLAND",
        continent: Continent::Eu,
        cq: 14,
        itu: 28,
        prefixes: &["HB9"],
        formats: &[CallFormat {
            prefixes: &["HB9"],
            digit: DigitRule::InPrefix,
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["BERN", "ZURICH"],
    },
    Entity {
        dxcc: 272,
        name: "PORTUGAL",
        continent: Continent::Eu,
        cq: 14,
        itu: 37,
        prefixes: &["CT"],
        formats: &[CallFormat {
            prefixes: &["CT"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["LISBOA", "PORTO"],
    },
    Entity {
        dxcc: 339,
        name: "JAPAN",
        continent: Continent::As,
        cq: 25,
        itu: 45,
        prefixes: &["JA", "JH", "JR", "JE"],
        formats: &[CallFormat {
            prefixes: &["JA", "JH", "JR", "JE"],
            digit: DigitRule::OneOf(D0_9),
            suffix: (3, 3),
            weight: 1,
        }],
        regions: &[],
        iso: Some("JP"),
        towns: &["TOKYO", "OSAKA", "SAPPORO"],
    },
    Entity {
        dxcc: 150,
        name: "AUSTRALIA",
        continent: Continent::Oc,
        cq: 30,
        itu: 59,
        prefixes: &["VK"],
        formats: &[CallFormat {
            prefixes: &["VK"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: Some("AU"),
        towns: &["SYDNEY", "PERTH", "HOBART"],
    },
    Entity {
        dxcc: 170,
        name: "NEW ZEALAND",
        continent: Continent::Oc,
        cq: 32,
        itu: 60,
        prefixes: &["ZL"],
        formats: &[CallFormat {
            prefixes: &["ZL"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["AUCKLAND", "NELSON"],
    },
    Entity {
        dxcc: 1,
        name: "CANADA",
        continent: Continent::Na,
        cq: 4,
        itu: 4,
        prefixes: &["VE", "VA"],
        formats: &[CallFormat {
            prefixes: &["VE", "VA"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: Some("CA"),
        towns: &["TORONTO", "OTTAWA", "CALGARY"],
    },
    Entity {
        dxcc: 50,
        name: "MEXICO",
        continent: Continent::Na,
        cq: 6,
        itu: 10,
        prefixes: &["XE"],
        formats: &[CallFormat {
            prefixes: &["XE"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["PUEBLA", "MERIDA"],
    },
    Entity {
        dxcc: 100,
        name: "ARGENTINA",
        continent: Continent::Sa,
        cq: 13,
        itu: 14,
        prefixes: &["LU"],
        formats: &[CallFormat {
            prefixes: &["LU"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["CORDOBA", "MENDOZA"],
    },
    Entity {
        dxcc: 108,
        name: "BRAZIL",
        continent: Continent::Sa,
        cq: 11,
        itu: 15,
        prefixes: &["PY"],
        formats: &[CallFormat {
            prefixes: &["PY"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["RIO", "RECIFE"],
    },
    Entity {
        dxcc: 462,
        name: "SOUTH AFRICA",
        continent: Continent::Af,
        cq: 38,
        itu: 57,
        prefixes: &["ZS"],
        formats: &[CallFormat {
            prefixes: &["ZS"],
            digit: DigitRule::OneOf(D1_9),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["DURBAN", "PRETORIA"],
    },
    Entity {
        dxcc: 54,
        name: "RUSSIA",
        continent: Continent::Eu,
        cq: 16,
        itu: 29,
        prefixes: &["UA", "RA", "RW"],
        formats: &[CallFormat {
            prefixes: &["UA", "RA", "RW"],
            digit: DigitRule::OneOf(&[1, 3, 4, 6]),
            suffix: (2, 3),
            weight: 1,
        }],
        regions: &[],
        iso: None,
        towns: &["MOSKVA", "KAZAN"],
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
        for e in WORLD {
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
                assert!(f.suffix.0 >= 1 && f.suffix.0 <= f.suffix.1, "{e:?}");
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
