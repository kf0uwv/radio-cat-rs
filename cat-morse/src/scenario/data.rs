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

//! Static tables for plausible QSO content. Every string here is upper
//! case and uses only characters in the Morse table (checked by a test).

/// A US call area: its digit, the states it historically covers (two-letter
/// postal codes, as sent in a POTA exchange) with a few towns each, and
/// its CQ zone.
pub(crate) struct CallArea {
    pub digit: u8,
    pub zone: u8,
    pub states: &'static [(&'static str, &'static [&'static str])],
}

pub(crate) const US_AREAS: &[CallArea] = &[
    CallArea {
        digit: 1,
        zone: 5,
        states: &[
            ("MA", &["BOSTON", "WORCESTER", "SALEM"]),
            ("CT", &["HARTFORD", "NEWINGTON", "MYSTIC"]),
            ("ME", &["PORTLAND", "BANGOR"]),
            ("NH", &["NASHUA", "CONCORD"]),
            ("VT", &["BURLINGTON"]),
            ("RI", &["PROVIDENCE"]),
        ],
    },
    CallArea {
        digit: 2,
        zone: 5,
        states: &[
            ("NY", &["ALBANY", "BUFFALO", "ROCHESTER", "ITHACA"]),
            ("NJ", &["TRENTON", "PRINCETON", "NEWARK"]),
        ],
    },
    CallArea {
        digit: 3,
        zone: 5,
        states: &[
            ("PA", &["PITTSBURGH", "ERIE", "LANCASTER"]),
            ("MD", &["BALTIMORE", "FREDERICK"]),
            ("DE", &["DOVER", "WILMINGTON"]),
        ],
    },
    CallArea {
        digit: 4,
        zone: 5,
        states: &[
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
    CallArea {
        digit: 5,
        zone: 4,
        states: &[
            ("TX", &["AUSTIN", "DALLAS", "EL PASO", "HOUSTON"]),
            ("OK", &["TULSA", "NORMAN"]),
            ("LA", &["BATON ROUGE", "SHREVEPORT"]),
            ("NM", &["SANTA FE", "TAOS", "ALBUQUERQUE"]),
            ("AR", &["LITTLE ROCK"]),
            ("MS", &["JACKSON", "BILOXI"]),
        ],
    },
    CallArea {
        digit: 6,
        zone: 3,
        states: &[(
            "CA",
            &["SAN DIEGO", "FRESNO", "SACRAMENTO", "SAN JOSE", "BISHOP"],
        )],
    },
    CallArea {
        digit: 7,
        zone: 3,
        states: &[
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
    CallArea {
        digit: 8,
        zone: 4,
        states: &[
            ("OH", &["DAYTON", "COLUMBUS", "TOLEDO"]),
            ("MI", &["DETROIT", "LANSING", "MARQUETTE"]),
            ("WV", &["MORGANTOWN", "CHARLESTON"]),
        ],
    },
    CallArea {
        digit: 9,
        zone: 4,
        states: &[
            ("IL", &["CHICAGO", "PEORIA", "SPRINGFIELD"]),
            ("IN", &["FORT WAYNE", "INDIANAPOLIS"]),
            ("WI", &["MADISON", "GREEN BAY"]),
        ],
    },
    CallArea {
        digit: 0,
        zone: 4,
        states: &[
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

/// A DX entity: prefixes as issued, call shape, CQ zone and towns.
///
/// `iso` is the ISO 3166 code POTA uses for the entity's park references
/// (since the 2024 re-prefixing, e.g. `DE-0123`), where POTA parks are
/// common enough to appear in a scenario.
pub(crate) struct DxEntity {
    pub prefixes: &'static [&'static str],
    /// Whether a digit follows the prefix (`DL1ABC`) or is part of it
    /// (`HB9ABC`, `9A2AB`).
    pub add_digit: bool,
    pub suffix_len: (u32, u32),
    pub zone: u8,
    pub country: &'static str,
    pub iso: Option<&'static str>,
    pub towns: &'static [&'static str],
}

pub(crate) const DX: &[DxEntity] = &[
    DxEntity {
        prefixes: &["G", "M"],
        add_digit: true,
        suffix_len: (3, 3),
        zone: 14,
        country: "ENGLAND",
        iso: None,
        towns: &["LONDON", "BRISTOL", "YORK", "LEEDS"],
    },
    DxEntity {
        prefixes: &["DL", "DK", "DJ", "DF"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 14,
        country: "GERMANY",
        iso: Some("DE"),
        towns: &["BERLIN", "MUNICH", "HAMBURG", "BONN"],
    },
    DxEntity {
        prefixes: &["F"],
        add_digit: true,
        suffix_len: (3, 3),
        zone: 14,
        country: "FRANCE",
        iso: Some("FR"),
        towns: &["PARIS", "LYON", "NANTES"],
    },
    DxEntity {
        prefixes: &["I", "IK", "IZ"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 15,
        country: "ITALY",
        iso: Some("IT"),
        towns: &["ROMA", "MILANO", "TORINO"],
    },
    DxEntity {
        prefixes: &["EA"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 14,
        country: "SPAIN",
        iso: Some("ES"),
        towns: &["MADRID", "SEVILLA", "VALENCIA"],
    },
    DxEntity {
        prefixes: &["ON"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 14,
        country: "BELGIUM",
        iso: None,
        towns: &["GENT", "BRUGGE"],
    },
    DxEntity {
        prefixes: &["PA", "PD"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 14,
        country: "NETHERLANDS",
        iso: None,
        towns: &["UTRECHT", "DELFT"],
    },
    DxEntity {
        prefixes: &["OH"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 15,
        country: "FINLAND",
        iso: None,
        towns: &["HELSINKI", "OULU"],
    },
    DxEntity {
        prefixes: &["SM"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 14,
        country: "SWEDEN",
        iso: None,
        towns: &["STOCKHOLM", "LUND"],
    },
    DxEntity {
        prefixes: &["LA"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 14,
        country: "NORWAY",
        iso: None,
        towns: &["OSLO", "BERGEN"],
    },
    DxEntity {
        prefixes: &["OK"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 15,
        country: "CZECH REP",
        iso: None,
        towns: &["PRAHA", "BRNO"],
    },
    DxEntity {
        prefixes: &["SP"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 15,
        country: "POLAND",
        iso: None,
        towns: &["KRAKOW", "GDANSK"],
    },
    DxEntity {
        prefixes: &["HA"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 15,
        country: "HUNGARY",
        iso: None,
        towns: &["BUDAPEST", "PECS"],
    },
    DxEntity {
        prefixes: &["S5"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 15,
        country: "SLOVENIA",
        iso: None,
        towns: &["LJUBLJANA"],
    },
    DxEntity {
        prefixes: &["9A"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 15,
        country: "CROATIA",
        iso: None,
        towns: &["ZAGREB", "SPLIT"],
    },
    DxEntity {
        prefixes: &["EI"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 14,
        country: "IRELAND",
        iso: None,
        towns: &["DUBLIN", "CORK"],
    },
    DxEntity {
        prefixes: &["GM", "MM"],
        add_digit: true,
        suffix_len: (3, 3),
        zone: 14,
        country: "SCOTLAND",
        iso: None,
        towns: &["EDINBURGH", "GLASGOW"],
    },
    DxEntity {
        prefixes: &["OE"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 15,
        country: "AUSTRIA",
        iso: None,
        towns: &["WIEN", "GRAZ"],
    },
    DxEntity {
        prefixes: &["HB9"],
        add_digit: false,
        suffix_len: (2, 3),
        zone: 14,
        country: "SWITZERLAND",
        iso: None,
        towns: &["BERN", "ZURICH"],
    },
    DxEntity {
        prefixes: &["CT"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 14,
        country: "PORTUGAL",
        iso: None,
        towns: &["LISBOA", "PORTO"],
    },
    DxEntity {
        prefixes: &["JA", "JH", "JR", "JE"],
        add_digit: true,
        suffix_len: (3, 3),
        zone: 25,
        country: "JAPAN",
        iso: Some("JP"),
        towns: &["TOKYO", "OSAKA", "SAPPORO"],
    },
    DxEntity {
        prefixes: &["VK"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 30,
        country: "AUSTRALIA",
        iso: Some("AU"),
        towns: &["SYDNEY", "PERTH", "HOBART"],
    },
    DxEntity {
        prefixes: &["ZL"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 32,
        country: "NEW ZEALAND",
        iso: None,
        towns: &["AUCKLAND", "NELSON"],
    },
    DxEntity {
        prefixes: &["VE", "VA"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 4,
        country: "CANADA",
        iso: Some("CA"),
        towns: &["TORONTO", "OTTAWA", "CALGARY"],
    },
    DxEntity {
        prefixes: &["XE"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 6,
        country: "MEXICO",
        iso: None,
        towns: &["PUEBLA", "MERIDA"],
    },
    DxEntity {
        prefixes: &["LU"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 13,
        country: "ARGENTINA",
        iso: None,
        towns: &["CORDOBA", "MENDOZA"],
    },
    DxEntity {
        prefixes: &["PY"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 11,
        country: "BRAZIL",
        iso: None,
        towns: &["RIO", "RECIFE"],
    },
    DxEntity {
        prefixes: &["ZS"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 38,
        country: "SOUTH AFRICA",
        iso: None,
        towns: &["DURBAN", "PRETORIA"],
    },
    DxEntity {
        prefixes: &["UA", "RA", "RW"],
        add_digit: true,
        suffix_len: (2, 3),
        zone: 16,
        country: "RUSSIA",
        iso: None,
        towns: &["MOSKVA", "KAZAN"],
    },
];

/// SOTA associations and regions, as written in a summit reference
/// (`W0C/FR-001`).
pub(crate) const SOTA_REGIONS: &[(&str, &[&str])] = &[
    ("W0C", &["FR", "PR", "SR", "SP"]),
    ("W7W", &["KG", "LC", "CW"]),
    ("W6", &["CT", "SN", "NC"]),
    ("W4C", &["CM", "EM"]),
    ("W7A", &["AW", "CN"]),
    ("G", &["LD", "SP", "NP"]),
    ("GM", &["WS", "ES", "CS"]),
    ("DL", &["AL"]),
    ("HB", &["BE", "VS"]),
    ("VE7", &["GV"]),
];

pub(crate) const NAMES_US: &[&str] = &[
    "BOB", "JIM", "JOHN", "MIKE", "DAVE", "TOM", "BILL", "STEVE", "GARY", "DON", "RON", "KEN",
    "JOE", "PAUL", "MARK", "RICK", "DAN", "JEFF", "CHUCK", "ED", "AL", "ART", "CARL", "FRED",
    "HANK", "JACK", "LARRY", "PETE", "RAY", "SAM", "TED", "WALT", "ANN", "SUE", "MARY", "JAN",
    "KAREN", "LIZ", "PAT", "DEB",
];

pub(crate) const NAMES_DX: &[&str] = &[
    "HANS", "KLAUS", "PIERRE", "JEAN", "MARCO", "LUCA", "JOSE", "KENJI", "HIRO", "IAN", "NIGEL",
    "SVEN", "LARS", "JAN", "PIOTR", "LASZLO", "ANDY", "PETER", "MIGUEL", "IGOR",
];

pub(crate) const RIGS: &[&str] = &[
    "IC-7300",
    "IC-7610",
    "IC-705",
    "K3",
    "K4",
    "KX2",
    "KX3",
    "FT-991A",
    "FTDX10",
    "FT-710",
    "FT-818",
    "TS-590SG",
    "TS-890S",
    "TS-570D",
    "FLEX 6600",
    "QCX",
    "K2",
];

pub(crate) const POWERS: &[u32] = &[5, 10, 50, 100, 100, 100, 200, 500];
pub(crate) const QRP_POWERS: &[u32] = &[5, 5, 10];

pub(crate) const ANTENNAS: &[&str] = &[
    "DIPOLE",
    "EFHW",
    "VERTICAL",
    "YAGI",
    "HEX BEAM",
    "LOOP",
    "G5RV",
    "INV V",
    "OCF DIPOLE",
    "LONG WIRE",
];

pub(crate) const PORTABLE_ANTENNAS: &[&str] = &["EFHW", "VERTICAL", "LINKED DIPOLE", "WIRE"];

pub(crate) const WX: &[&str] = &[
    "SUNNY", "CLOUDY", "RAIN", "SNOW", "FOGGY", "WINDY", "CLEAR", "OVERCAST", "HOT", "COLD",
];

pub(crate) const RST: &[&str] = &[
    "599", "589", "579", "569", "559", "549", "449", "559", "579",
];

/// Call suffix letters, avoiding a leading `Q` (Q-signals) and `X`.
pub(crate) const SUFFIX_FIRST: &[u8] = b"ABCDEFGHIJKLMNOPRSTUVWYZ";
pub(crate) const LETTERS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// Calls never generated: club and special-event calls an operator
/// would recognise as not being an ordinary station.
pub(crate) const RESERVED_CALLS: &[&str] = &["W1AW", "K1JT"];
