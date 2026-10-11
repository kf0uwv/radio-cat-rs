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

/// A pool of common given names for operators from one part of the
/// world. Plausible first names only; nothing pairs a name with a real
/// callsign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Names {
    Us,
    Anglo,
    German,
    French,
    Italian,
    Iberian,
    LatAm,
    Lusophone,
    Nordic,
    Finnish,
    Baltic,
    Dutch,
    WestSlavic,
    EastSlavic,
    Danube,
    Greek,
    Turkish,
    Caucasus,
    Albanian,
    Arabic,
    Hebrew,
    Persian,
    Indian,
    Japanese,
    Chinese,
    Korean,
    SeAsian,
    Mongol,
    African,
    Expedition,
}

impl Names {
    /// Every pool.
    #[cfg(test)]
    pub(crate) const ALL: &[Names] = &[
        Names::Us,
        Names::Anglo,
        Names::German,
        Names::French,
        Names::Italian,
        Names::Iberian,
        Names::LatAm,
        Names::Lusophone,
        Names::Nordic,
        Names::Finnish,
        Names::Baltic,
        Names::Dutch,
        Names::WestSlavic,
        Names::EastSlavic,
        Names::Danube,
        Names::Greek,
        Names::Turkish,
        Names::Caucasus,
        Names::Albanian,
        Names::Arabic,
        Names::Hebrew,
        Names::Persian,
        Names::Indian,
        Names::Japanese,
        Names::Chinese,
        Names::Korean,
        Names::SeAsian,
        Names::Mongol,
        Names::African,
        Names::Expedition,
    ];

    /// The names.
    pub(crate) fn pool(self) -> &'static [&'static str] {
        match self {
            Names::Us => NAMES_US,
            Names::Anglo => NAMES_ANGLO,
            Names::German => NAMES_GERMAN,
            Names::French => NAMES_FRENCH,
            Names::Italian => NAMES_ITALIAN,
            Names::Iberian => NAMES_IBERIAN,
            Names::LatAm => NAMES_LAT_AM,
            Names::Lusophone => NAMES_LUSOPHONE,
            Names::Nordic => NAMES_NORDIC,
            Names::Finnish => NAMES_FINNISH,
            Names::Baltic => NAMES_BALTIC,
            Names::Dutch => NAMES_DUTCH,
            Names::WestSlavic => NAMES_WEST_SLAVIC,
            Names::EastSlavic => NAMES_EAST_SLAVIC,
            Names::Danube => NAMES_DANUBE,
            Names::Greek => NAMES_GREEK,
            Names::Turkish => NAMES_TURKISH,
            Names::Caucasus => NAMES_CAUCASUS,
            Names::Albanian => NAMES_ALBANIAN,
            Names::Arabic => NAMES_ARABIC,
            Names::Hebrew => NAMES_HEBREW,
            Names::Persian => NAMES_PERSIAN,
            Names::Indian => NAMES_INDIAN,
            Names::Japanese => NAMES_JAPANESE,
            Names::Chinese => NAMES_CHINESE,
            Names::Korean => NAMES_KOREAN,
            Names::SeAsian => NAMES_SE_ASIAN,
            Names::Mongol => NAMES_MONGOL,
            Names::African => NAMES_AFRICAN,
            Names::Expedition => NAMES_EXPEDITION,
        }
    }
}

const NAMES_ANGLO: &[&str] = &[
    "JOHN", "DAVID", "PAUL", "MARK", "IAN", "NIGEL", "ANDY", "CHRIS", "STEVE", "PETER", "MIKE",
    "GRAHAM", "ALAN", "SIMON", "TONY", "MARTIN", "KEVIN", "GARY", "SUE", "JANE",
];
const NAMES_GERMAN: &[&str] = &[
    "HANS", "KLAUS", "JUERGEN", "WOLFGANG", "PETER", "UWE", "DIETER", "FRANK", "THOMAS", "STEFAN",
    "MICHAEL", "ANDREAS", "RALF", "BERND", "GERD", "MARTIN", "SABINE", "KARIN",
];
const NAMES_FRENCH: &[&str] = &[
    "JEAN",
    "PIERRE",
    "MICHEL",
    "ALAIN",
    "PHILIPPE",
    "PATRICK",
    "BERNARD",
    "CHRISTIAN",
    "DANIEL",
    "ERIC",
    "FRANCOIS",
    "LUC",
    "DIDIER",
    "OLIVIER",
    "MARIE",
    "SYLVIE",
];
const NAMES_ITALIAN: &[&str] = &[
    "MARCO", "LUCA", "GIUSEPPE", "PAOLO", "FRANCO", "ROBERTO", "MAURO", "STEFANO", "GIANNI",
    "ANDREA", "SERGIO", "FABIO", "CLAUDIO", "SILVIA",
];
const NAMES_IBERIAN: &[&str] = &[
    "JOSE",
    "ANTONIO",
    "MANUEL",
    "JUAN",
    "FRANCISCO",
    "CARLOS",
    "LUIS",
    "JAVIER",
    "PEDRO",
    "MIGUEL",
    "PACO",
    "JOAO",
    "RUI",
    "CARMEN",
];
const NAMES_LAT_AM: &[&str] = &[
    "JOSE",
    "CARLOS",
    "JORGE",
    "LUIS",
    "MIGUEL",
    "PEDRO",
    "RAUL",
    "OSCAR",
    "RICARDO",
    "ALEJANDRO",
    "DIEGO",
    "FERNANDO",
    "MARIO",
    "GABRIEL",
    "ANA",
];
const NAMES_LUSOPHONE: &[&str] = &[
    "JOAO", "PAULO", "CARLOS", "MARCOS", "JOSE", "RICARDO", "ANDRE", "FABIO", "RODRIGO", "EDSON",
    "MARCIO", "LUIZ", "RENATO",
];
const NAMES_NORDIC: &[&str] = &[
    "LARS", "SVEN", "ERIK", "OLA", "BJORN", "NILS", "PER", "JAN", "ANDERS", "MAGNUS", "KNUT",
    "ARNE", "INGE", "OLE",
];
const NAMES_FINNISH: &[&str] = &[
    "JUHA", "MIKKO", "PEKKA", "JARI", "KARI", "TIMO", "MATTI", "HANNU", "ESA", "ARI",
];
const NAMES_BALTIC: &[&str] = &[
    "JONAS", "ANDRIS", "TOOMAS", "ALGIS", "JANIS", "MART", "AIVARS", "RAIMONDS",
];
const NAMES_DUTCH: &[&str] = &[
    "JAN", "PIET", "HENK", "KEES", "WIM", "JOOP", "GERRIT", "BART", "DIRK", "RUUD", "JEROEN",
    "MARC",
];
const NAMES_WEST_SLAVIC: &[&str] = &[
    "PIOTR", "TOMASZ", "PAVEL", "JAN", "MAREK", "JIRI", "MIRO", "ZORAN", "DRAGAN", "BOJAN",
    "GORAN", "PETAR", "STANKO", "ZDENEK",
];
const NAMES_EAST_SLAVIC: &[&str] = &[
    "IGOR", "SERGEI", "VLADIMIR", "ALEX", "OLEG", "YURI", "ANDREI", "DMITRI", "NIKOLAI", "VICTOR",
    "MIKHAIL", "VALERY", "GENNADY", "OLGA",
];
const NAMES_DANUBE: &[&str] = &[
    "LASZLO", "ZOLTAN", "GABOR", "ISTVAN", "ATTILA", "SANDOR", "ION", "MIHAI", "DAN", "SORIN",
    "ADRIAN", "VASILE",
];
const NAMES_GREEK: &[&str] = &[
    "NIKOS", "GIORGOS", "KOSTAS", "YANNIS", "DIMITRIS", "ANDREAS", "PANOS", "VASILIS",
];
const NAMES_TURKISH: &[&str] = &[
    "MEHMET", "AHMET", "MUSTAFA", "ALI", "HASAN", "MURAT", "EMRE", "OSMAN", "KEMAL", "ELDAR",
];
const NAMES_CAUCASUS: &[&str] = &[
    "GIORGI", "LEVAN", "DAVIT", "ARMEN", "ARAM", "VAHE", "GAGIK", "ZURAB",
];
const NAMES_ALBANIAN: &[&str] = &["ARBEN", "BESNIK", "GENTI", "ILIR", "AGIM", "FATOS"];
const NAMES_ARABIC: &[&str] = &[
    "AHMED", "MOHAMED", "ALI", "OMAR", "KHALID", "HASSAN", "SAMIR", "KARIM", "YOUSEF", "TARIQ",
    "FAISAL", "NABIL",
];
const NAMES_HEBREW: &[&str] = &[
    "DAVID", "MOSHE", "YOSSI", "AVI", "ITZIK", "RONI", "DANI", "ERAN", "GIL",
];
const NAMES_PERSIAN: &[&str] = &["REZA", "ALI", "MEHDI", "HOSSEIN", "AMIR", "DARIUS", "FARID"];
const NAMES_INDIAN: &[&str] = &[
    "RAJ", "RAVI", "SURESH", "ANIL", "VIJAY", "AMIT", "SANJAY", "RAMESH", "PRAKASH", "IMRAN",
    "ASIF", "KUMAR",
];
const NAMES_JAPANESE: &[&str] = &[
    "KENJI", "HIRO", "TAKA", "YOSHI", "KAZU", "AKIRA", "MASA", "TOSHI", "SHIN", "NOBU", "KOJI",
    "MAKOTO", "YUKI",
];
const NAMES_CHINESE: &[&str] = &[
    "WEI", "MING", "JUN", "LEI", "HUA", "GANG", "YONG", "BIN", "JIAN", "FENG", "CHEN", "LIN",
];
const NAMES_KOREAN: &[&str] = &[
    "KIM", "PARK", "SUNG", "JIN", "HYUN", "YOUNG", "MIN", "DONG", "JAE",
];
const NAMES_SE_ASIAN: &[&str] = &[
    "SOMCHAI", "NGUYEN", "BUDI", "AGUS", "AHMAD", "RAMON", "JOJO", "EDDIE", "ANDI", "RIZAL",
];
const NAMES_MONGOL: &[&str] = &["BAT", "GANBAA", "TSOGT", "BOLD", "ENKH"];
const NAMES_AFRICAN: &[&str] = &[
    "JOHN", "EMMANUEL", "SAMUEL", "JOSEPH", "DANIEL", "PATRICK", "MOSES", "FRANCIS", "KOFI",
    "KWAME", "DAVID", "PETER",
];
const NAMES_EXPEDITION: &[&str] = &[
    "DAVE", "HANS", "KEN", "PAUL", "ADRIAN", "DOM", "JUAN", "KAZU", "MAREK", "ROB",
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
