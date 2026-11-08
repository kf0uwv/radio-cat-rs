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

//! Black-box encoder vectors: `tests/data/encoder-vectors.jsonl`, generated
//! from WSJT-X 3.0.0-rc1 `ft8sim` output (1 020 messages: 20 fixed cases
//! covering every message type, then 1 000 random standard messages). Each
//! line holds the input `message`, ft8sim's own unpack of it (`decoded`),
//! `i3`, `n3`, the 77 payload bits and the 79 channel symbols.
//!
//! Known quirks of the oracle, handled explicitly rather than hidden:
//! - one vector's `decoded` is `*** bad message ***` (an EU VHF-style text
//!   ft8sim packed as a type it then could not unpack) — skipped and
//!   counted;
//! - DXpedition reports are carried in 2 dB steps, so `decoded` is the
//!   quantized report (−11 → −12); compared against `decoded`.

use std::collections::BTreeMap;

use cat_ft8::message::{pack, unpack, CallTable, MessageType, Payload};
use cat_ft8::{crc, encode_payload, ldpc, symbols, Error};

const BAD_MESSAGE: &str = "*** bad message ***";

struct Vector {
    message: String,
    decoded: String,
    i3: u8,
    n3: u8,
    payload: Payload,
    symbols: [u8; 79],
}

fn vectors() -> Vec<Vector> {
    let text = include_str!("data/encoder-vectors.jsonl");
    text.lines()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).expect("json line");
            let sym_str = v["symbols"].as_str().expect("symbols");
            let mut symbols = [0u8; 79];
            assert_eq!(sym_str.len(), 79);
            for (s, c) in symbols.iter_mut().zip(sym_str.bytes()) {
                *s = c - b'0';
            }
            Vector {
                message: v["message"].as_str().unwrap().to_string(),
                decoded: v["decoded"].as_str().unwrap().to_string(),
                i3: v["i3"].as_u64().unwrap() as u8,
                n3: v["n3"].as_u64().unwrap() as u8,
                payload: Payload::from_bit_str(v["bits77"].as_str().unwrap()).expect("77 bits"),
                symbols,
            }
        })
        .collect()
}

fn type_label(i3: u8, n3: u8) -> String {
    if i3 == 0 {
        format!("{i3}.{n3}")
    } else {
        format!("{i3}")
    }
}

#[test]
fn vector_file_is_complete() {
    assert_eq!(vectors().len(), 1020);
}

/// Stages 2–4 (CRC, LDPC, tones) for every vector, whatever its type: the
/// oracle's 77 bits must give the oracle's 79 symbols.
#[test]
fn every_vector_payload_encodes_to_its_symbols() {
    let mut checked = 0;
    for v in vectors() {
        if v.decoded == BAD_MESSAGE {
            // ft8sim printed all-zero symbols for the payload it could not
            // unpack, so there is nothing to compare.
            assert!(v.symbols.iter().all(|&s| s == 0));
            continue;
        }
        checked += 1;
        assert_eq!(encode_payload(&v.payload), v.symbols, "{}", v.message);
        let cw = symbols::unmap(&v.symbols).unwrap();
        assert!(cw.satisfies_parity_checks(), "{}", v.message);
        assert!(crc::check(cw.message()), "{}", v.message);
        assert_eq!(ldpc::encode(crc::append_crc(&v.payload)), cw);
    }
    println!("CRC/LDPC/tones checked on {checked} vectors");
    assert_eq!(checked, 1019);
}

/// Stage 1 and the full chain: pack(message) == bits77, encode == symbols,
/// unpack(pack(message)) == decoded, and unpack(bits77) == decoded. Every
/// vector is either matched or skipped with a counted reason.
#[test]
fn vectors_match_end_to_end() {
    let mut matched: BTreeMap<String, usize> = BTreeMap::new();
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    let all = vectors();
    for v in &all {
        let label = type_label(v.i3, v.n3);
        if v.decoded == BAD_MESSAGE {
            *skipped
                .entry(format!("{label} (oracle: bad message)"))
                .or_default() += 1;
            continue;
        }
        let mut table: CallTable<16> = CallTable::new();
        match pack(&v.message, &mut table) {
            Ok(p) => {
                assert_eq!(p, v.payload, "pack {:?}", v.message);
                assert_eq!(p.message_type(), MessageType::from_i3_n3(v.i3, v.n3));
                assert_eq!(encode_payload(&p), v.symbols, "symbols {:?}", v.message);
                let text = unpack(&p, &mut table).expect("unpack");
                assert_eq!(text.as_str(), v.decoded, "unpack(pack) {:?}", v.message);
                let text = unpack(&v.payload, &mut table).expect("unpack oracle bits");
                assert_eq!(text.as_str(), v.decoded, "unpack(bits77) {:?}", v.message);
                *matched.entry(label).or_default() += 1;
            }
            Err(Error::Unsupported(t)) => {
                assert_eq!(t, MessageType::from_i3_n3(v.i3, v.n3), "{:?}", v.message);
                assert_eq!(unpack(&v.payload, &mut table), Err(Error::Unsupported(t)));
                *skipped.entry(format!("{label} (unsupported)")).or_default() += 1;
            }
            Err(e) => panic!("{:?}: {e}", v.message),
        }
    }
    let n_matched: usize = matched.values().sum();
    let n_skipped: usize = skipped.values().sum();
    println!("matched {n_matched} by type: {matched:?}");
    println!("skipped {n_skipped} by reason: {skipped:?}");
    assert_eq!(n_matched + n_skipped, all.len());
    // Pin the counts so a change in what is supported is a deliberate act.
    assert_eq!(n_matched, 1017);
    assert_eq!(skipped.get("0.3 (unsupported)"), Some(&2));
    assert_eq!(skipped.get("3 (oracle: bad message)"), Some(&1));
}

/// Where this crate deliberately differs from the oracle on the extra
/// vectors: WSJT-X 3.0 silently drops or truncates part of these messages
/// (or cannot unpack its own packing); this crate refuses them instead.
const DEVIATIONS: &[(&str, Error)] = &[
    // Reports outside −50…+50 wrap on unpack in WSJT-X (+99 shows as −02).
    ("K1ABC W9XYZ +99", Error::InvalidReport),
    ("K1ABC W9XYZ +51", Error::InvalidReport),
    // Free text longer than 13 characters is truncated by WSJT-X.
    ("K1ABC 3DA0XYZ -10", Error::NotEncodable),
    ("CQ DX PJ4/K1ABC", Error::NotEncodable),
    ("CQ DX <K1ABC>", Error::NotEncodable),
    // WSJT-X drops the prefix of an unbracketed compound call.
    ("PJ4/K1ABC W9XYZ", Error::NotEncodable),
    ("W9XYZ PJ4/K1ABC RRR", Error::NotEncodable),
    // Type 4 cannot carry a report; WSJT-X drops it.
    ("<PJ4/K1ABC> <W9XYZ> -10", Error::NotEncodable),
    ("K1ABC/R <W9XYZ> -10", Error::NotEncodable),
    ("<PJ4/K1ABC> W9XYZ/R -10", Error::NotEncodable),
    // WSJT-X packs these but cannot unpack them.
    ("CQ <K1ABC>", Error::NotEncodable),
    ("CQ <K1ABC> FN42", Error::NotEncodable),
];

/// Extra oracle vectors (`tests/data/encoder-vectors-extra.jsonl`, also
/// from WSJT-X 3.0.0-rc1 `ft8sim`): type 4, CQ modifiers, report limits,
/// telemetry/free-text boundaries and bracketed-call combinations.
#[test]
fn extra_vectors_match_or_deviate_as_documented() {
    let text = include_str!("data/encoder-vectors-extra.jsonl");
    let (mut matched, mut deviated, mut oracle_failed) = (0, 0, 0);
    let mut by_type: BTreeMap<String, usize> = BTreeMap::new();
    for l in text.lines() {
        let v: serde_json::Value = serde_json::from_str(l).unwrap();
        let message = v["message"].as_str().unwrap();
        let mut table: CallTable<16> = CallTable::new();
        let result = pack(message, &mut table);
        let Some(decoded) = v["decoded"].as_str() else {
            // ft8sim printed bits and symbols but no text: compare those.
            let p = result.unwrap_or_else(|e| panic!("{message}: {e}"));
            let oracle = Payload::from_bit_str(v["bits77"].as_str().unwrap()).unwrap();
            assert_eq!(p, oracle, "pack {message:?}");
            let sym: Vec<u8> = v["symbols"]
                .as_str()
                .unwrap()
                .bytes()
                .map(|c| c - b'0')
                .collect();
            assert_eq!(&encode_payload(&p)[..], &sym[..], "symbols {message:?}");
            oracle_failed += 1;
            continue;
        };
        if let Some((_, e)) = DEVIATIONS.iter().find(|(m, _)| *m == message) {
            assert_eq!(result, Err(*e), "{message}");
            deviated += 1;
            continue;
        }
        let oracle = Payload::from_bit_str(v["bits77"].as_str().unwrap()).unwrap();
        let p = result.unwrap_or_else(|e| panic!("{message}: {e}"));
        assert_eq!(p, oracle, "pack {message:?}");
        let sym: Vec<u8> = v["symbols"]
            .as_str()
            .unwrap()
            .bytes()
            .map(|c| c - b'0')
            .collect();
        assert_eq!(&encode_payload(&p)[..], &sym[..], "symbols {message:?}");
        let ours = unpack(&p, &mut table).unwrap();
        if message == "0" {
            // Telemetry zero: WSJT-X shows an empty message, this crate "0".
            assert_eq!((ours.as_str(), decoded), ("0", ""));
        } else {
            assert_eq!(ours.as_str(), decoded, "unpack {message:?}");
        }
        *by_type.entry(type_label(p.i3(), p.n3())).or_default() += 1;
        matched += 1;
    }
    println!("extra: matched {matched} {by_type:?}, documented deviations {deviated}, oracle failed {oracle_failed}");
    assert_eq!(deviated, DEVIATIONS.len());
    assert_eq!(oracle_failed, 1);
    assert!(by_type.get("4").copied().unwrap_or(0) >= 10);
}

/// Random oracle vectors (`tests/data/encoder-vectors-random.jsonl`, also
/// from WSJT-X 3.0.0-rc1 `ft8sim`): 60 random messages per class — type 4,
/// free text, telemetry, DXpedition, `/P`, type 5, and type 1 with `/R`,
/// hashed calls, CQ modifiers and reports over −50…+50. Every message the
/// oracle printed a decode for must match exactly, except the listed ones.
#[test]
fn random_vectors_match() {
    // Inputs the oracle changes and this crate sends as written.
    const RANDOM_DEVIATIONS: &[&str] = &[
        // WSJT-X sends `+1` as telemetry 1 (dropping the `+`); this crate
        // sends free text `+1`.
        "+1",
    ];
    let text = include_str!("data/encoder-vectors-random.jsonl");
    let (mut matched, mut deviated, mut oracle_failed) = (0, 0, 0);
    let mut by_type: BTreeMap<String, usize> = BTreeMap::new();
    for l in text.lines() {
        let v: serde_json::Value = serde_json::from_str(l).unwrap();
        let message = v["message"].as_str().unwrap();
        let mut table: CallTable<16> = CallTable::new();
        let p = pack(message, &mut table).unwrap_or_else(|e| panic!("{message}: {e}"));
        let Some(decoded) = v["decoded"].as_str() else {
            // The oracle printed bits and symbols but no text: compare
            // those, skip only the text.
            let oracle = Payload::from_bit_str(v["bits77"].as_str().unwrap()).unwrap();
            assert_eq!(p, oracle, "pack {message:?}");
            let sym: Vec<u8> = v["symbols"]
                .as_str()
                .unwrap()
                .bytes()
                .map(|c| c - b'0')
                .collect();
            assert_eq!(&encode_payload(&p)[..], &sym[..], "symbols {message:?}");
            oracle_failed += 1;
            continue;
        };
        if RANDOM_DEVIATIONS.contains(&message) {
            assert_eq!(unpack(&p, &mut table).unwrap().as_str(), message);
            deviated += 1;
            continue;
        }
        let oracle = Payload::from_bit_str(v["bits77"].as_str().unwrap()).unwrap();
        assert_eq!(p, oracle, "pack {message:?}");
        let sym: Vec<u8> = v["symbols"]
            .as_str()
            .unwrap()
            .bytes()
            .map(|c| c - b'0')
            .collect();
        assert_eq!(&encode_payload(&p)[..], &sym[..], "symbols {message:?}");
        assert_eq!(
            unpack(&p, &mut table).unwrap().as_str(),
            decoded,
            "unpack {message:?}"
        );
        *by_type.entry(type_label(p.i3(), p.n3())).or_default() += 1;
        matched += 1;
    }
    println!(
        "random: matched {matched} {by_type:?}, deviations {deviated}, \
         bits and symbols only (oracle printed no text) {oracle_failed}"
    );
    assert_eq!(deviated, RANDOM_DEVIATIONS.len());
    assert_eq!(matched + deviated + oracle_failed, text.lines().count());
    for t in ["0.0", "0.1", "0.5", "1", "2", "4", "5"] {
        assert!(
            by_type.get(t).copied().unwrap_or(0) >= 30,
            "type {t}: {by_type:?}"
        );
    }
}
