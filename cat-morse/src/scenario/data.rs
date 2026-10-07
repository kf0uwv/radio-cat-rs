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
