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

//! The Morse table: text ↔ tokens ↔ element groups.
//!
//! Covers ITU letters, figures and punctuation `. , ? / = - ' : " ( ) @ +`,
//! the non-ITU `!` and `&`, and the prosigns AR SK BT KN AS HH SOS, written
//! in text as `<AR>` and so on.
//!
//! # Shared patterns
//!
//! Four patterns are a character *and* a prosign, by design of the code:
//! `=`/`<BT>`, `+`/`<AR>`, `(`/`<KN>` and `&`/`<AS>`. Encoding either form
//! sends the same elements; decoding picks one form by [`DecodeStyle`]
//! (prosigns by default, as hams write them in a QSO), and [`canonical`]
//! folds the pair so a scorer can treat them as equal.

use std::fmt;
use std::sync::OnceLock;

/// One Morse element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Element {
    /// A dot: one unit key-down.
    Dit,
    /// A dash: three units key-down (for a perfect fist).
    Dah,
}

/// A procedural signal sent as one run-together group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Prosign {
    /// `<AR>` end of message (same pattern as `+`).
    Ar,
    /// `<SK>` end of contact.
    Sk,
    /// `<BT>` break / new paragraph (same pattern as `=`).
    Bt,
    /// `<KN>` go ahead, named station only (same pattern as `(`).
    Kn,
    /// `<AS>` wait (same pattern as `&`).
    As,
    /// `<HH>` error: eight dots.
    Hh,
    /// `<SOS>` distress.
    Sos,
}

impl Prosign {
    /// Every prosign, in table order.
    pub const ALL: [Prosign; 7] = [
        Prosign::Ar,
        Prosign::Sk,
        Prosign::Bt,
        Prosign::Kn,
        Prosign::As,
        Prosign::Hh,
        Prosign::Sos,
    ];

    /// The name written between angle brackets, e.g. `"AR"`.
    pub fn name(self) -> &'static str {
        match self {
            Prosign::Ar => "AR",
            Prosign::Sk => "SK",
            Prosign::Bt => "BT",
            Prosign::Kn => "KN",
            Prosign::As => "AS",
            Prosign::Hh => "HH",
            Prosign::Sos => "SOS",
        }
    }

    /// Parse a prosign name (case-insensitive), without brackets.
    pub fn from_name(name: &str) -> Option<Prosign> {
        Prosign::ALL
            .iter()
            .copied()
            .find(|p| p.name().eq_ignore_ascii_case(name))
    }
}

impl fmt::Display for Prosign {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}>", self.name())
    }
}

/// One unit of text: a character, a prosign, or the gap between words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Token {
    /// An upper-case letter, figure or punctuation mark in the table.
    Char(char),
    /// A prosign.
    Prosign(Prosign),
    /// A word space.
    WordSpace,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Char(c) => write!(f, "{c}"),
            Token::Prosign(p) => write!(f, "{p}"),
            Token::WordSpace => f.write_str(" "),
        }
    }
}

/// Which form a shared pattern decodes to (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DecodeStyle {
    /// `<BT>`, `<AR>`, `<KN>`, `<AS>` — how they are meant in a QSO.
    #[default]
    Prosigns,
    /// `=`, `+`, `(`, `&`.
    Characters,
}

/// Text that cannot be encoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    /// A character with no Morse pattern. `offset` is a byte offset.
    #[error("no Morse code for {ch:?} at byte {offset}")]
    UnknownChar { ch: char, offset: usize },
    /// `<...>` naming no known prosign.
    #[error("unknown prosign <{name}> at byte {offset}")]
    UnknownProsign { name: String, offset: usize },
    /// A `<` with no closing `>`.
    #[error("unterminated prosign starting at byte {offset}")]
    UnterminatedProsign { offset: usize },
}

const TEXT_TABLE: &[(Token, &str)] = &[
    (Token::Char('A'), ".-"),
    (Token::Char('B'), "-..."),
    (Token::Char('C'), "-.-."),
    (Token::Char('D'), "-.."),
    (Token::Char('E'), "."),
    (Token::Char('F'), "..-."),
    (Token::Char('G'), "--."),
    (Token::Char('H'), "...."),
    (Token::Char('I'), ".."),
    (Token::Char('J'), ".---"),
    (Token::Char('K'), "-.-"),
    (Token::Char('L'), ".-.."),
    (Token::Char('M'), "--"),
    (Token::Char('N'), "-."),
    (Token::Char('O'), "---"),
    (Token::Char('P'), ".--."),
    (Token::Char('Q'), "--.-"),
    (Token::Char('R'), ".-."),
    (Token::Char('S'), "..."),
    (Token::Char('T'), "-"),
    (Token::Char('U'), "..-"),
    (Token::Char('V'), "...-"),
    (Token::Char('W'), ".--"),
    (Token::Char('X'), "-..-"),
    (Token::Char('Y'), "-.--"),
    (Token::Char('Z'), "--.."),
    (Token::Char('0'), "-----"),
    (Token::Char('1'), ".----"),
    (Token::Char('2'), "..---"),
    (Token::Char('3'), "...--"),
    (Token::Char('4'), "....-"),
    (Token::Char('5'), "....."),
    (Token::Char('6'), "-...."),
    (Token::Char('7'), "--..."),
    (Token::Char('8'), "---.."),
    (Token::Char('9'), "----."),
    (Token::Char('.'), ".-.-.-"),
    (Token::Char(','), "--..--"),
    (Token::Char('?'), "..--.."),
    (Token::Char('/'), "-..-."),
    (Token::Char('='), "-...-"),
    (Token::Char('-'), "-....-"),
    (Token::Char('\''), ".----."),
    (Token::Char(':'), "---..."),
    (Token::Char('"'), ".-..-."),
    (Token::Char('('), "-.--."),
    (Token::Char(')'), "-.--.-"),
    (Token::Char('@'), ".--.-."),
    (Token::Char('+'), ".-.-."),
    (Token::Char('!'), "-.-.--"),
    (Token::Char('&'), ".-..."),
    (Token::Prosign(Prosign::Ar), ".-.-."),
    (Token::Prosign(Prosign::Sk), "...-.-"),
    (Token::Prosign(Prosign::Bt), "-...-"),
    (Token::Prosign(Prosign::Kn), "-.--."),
    (Token::Prosign(Prosign::As), ".-..."),
    (Token::Prosign(Prosign::Hh), "........"),
    (Token::Prosign(Prosign::Sos), "...---..."),
];

/// The (prosign, character) pairs that share one pattern.
pub const SHARED_PATTERNS: [(Prosign, char); 4] = [
    (Prosign::Bt, '='),
    (Prosign::Ar, '+'),
    (Prosign::Kn, '('),
    (Prosign::As, '&'),
];

fn table_storage() -> &'static [(Token, Vec<Element>)] {
    static TABLE: OnceLock<Vec<(Token, Vec<Element>)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        TEXT_TABLE
            .iter()
            .map(|(t, s)| {
                let els = s
                    .chars()
                    .map(|c| if c == '.' { Element::Dit } else { Element::Dah })
                    .collect();
                (*t, els)
            })
            .collect()
    })
}

/// Every entry of the table, with its elements.
pub fn table() -> impl Iterator<Item = (Token, &'static [Element])> {
    table_storage().iter().map(|(t, e)| (*t, e.as_slice()))
}

/// The elements for `token`, or `None` for [`Token::WordSpace`] and
/// characters not in the table.
pub fn pattern(token: Token) -> Option<&'static [Element]> {
    let token = match token {
        Token::Char(c) => Token::Char(c.to_ascii_uppercase()),
        other => other,
    };
    table_storage()
        .iter()
        .find(|(t, _)| *t == token)
        .map(|(_, e)| e.as_slice())
}

/// Folds each shared pair to its prosign form, and letters to upper case;
/// every other token is returned unchanged.
pub fn canonical(token: Token) -> Token {
    match token {
        Token::Char(c) => {
            let c = c.to_ascii_uppercase();
            SHARED_PATTERNS
                .iter()
                .find(|(_, ch)| *ch == c)
                .map(|(p, _)| Token::Prosign(*p))
                .unwrap_or(Token::Char(c))
        }
        other => other,
    }
}

/// The token for one element group, in the default [`DecodeStyle`].
pub fn decode_group(elements: &[Element]) -> Option<Token> {
    decode_group_with(elements, DecodeStyle::default())
}

/// The token for one element group, choosing the form of a shared pattern
/// by `style`. `None` if no entry has this pattern.
pub fn decode_group_with(elements: &[Element], style: DecodeStyle) -> Option<Token> {
    let mut found = None;
    for (t, e) in table_storage() {
        if e.as_slice() == elements {
            let wanted = matches!(
                (style, t),
                (DecodeStyle::Prosigns, Token::Prosign(_))
                    | (DecodeStyle::Characters, Token::Char(_))
            );
            if wanted {
                return Some(*t);
            }
            found.get_or_insert(*t);
        }
    }
    found
}

/// Splits text into tokens: case-insensitive, `<XX>` read as a prosign,
/// any run of whitespace becomes one [`Token::WordSpace`], and leading or
/// trailing whitespace is dropped. Every non-space character must be in
/// the table.
pub fn tokenize(text: &str) -> Result<Vec<Token>, CodecError> {
    let mut out = Vec::new();
    let mut pending_space = false;
    let mut iter = text.char_indices().peekable();
    while let Some((offset, ch)) = iter.next() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        let token = if ch == '<' {
            let mut name = String::new();
            let mut closed = false;
            for (_, c) in iter.by_ref() {
                if c == '>' {
                    closed = true;
                    break;
                }
                name.push(c);
            }
            if !closed {
                return Err(CodecError::UnterminatedProsign { offset });
            }
            match Prosign::from_name(&name) {
                Some(p) => Token::Prosign(p),
                None => return Err(CodecError::UnknownProsign { name, offset }),
            }
        } else {
            let up = ch.to_ascii_uppercase();
            if pattern(Token::Char(up)).is_none() {
                return Err(CodecError::UnknownChar { ch, offset });
            }
            Token::Char(up)
        };
        if pending_space {
            out.push(Token::WordSpace);
            pending_space = false;
        }
        out.push(token);
    }
    Ok(out)
}

/// Tokenizes `text` and pairs each token with its elements
/// ([`Token::WordSpace`] gets an empty slice).
pub fn encode(text: &str) -> Result<Vec<(Token, &'static [Element])>, CodecError> {
    tokenize(text).map(|tokens| {
        tokens
            .into_iter()
            .map(|t| (t, pattern(t).unwrap_or(&[])))
            .collect()
    })
}

/// Renders tokens back to text (`<AR>` for prosigns, one space per gap).
pub fn tokens_to_text(tokens: &[Token]) -> String {
    tokens.iter().map(|t| t.to_string()).collect()
}

/// Renders elements as dots and dashes, e.g. `".-"`.
pub fn elements_to_string(elements: &[Element]) -> String {
    elements
        .iter()
        .map(|e| match e {
            Element::Dit => '.',
            Element::Dah => '-',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn whole_table_round_trips() {
        for (t, els) in table() {
            let back = decode_group(els).expect("decodes");
            assert_eq!(back, canonical(t), "{t} / {}", elements_to_string(els));
            let back_c = decode_group_with(els, DecodeStyle::Characters).unwrap();
            assert_eq!(canonical(back_c), canonical(t));
        }
    }

    #[test]
    fn only_documented_patterns_collide() {
        let mut by_pattern: HashMap<&[Element], Vec<Token>> = HashMap::new();
        for (t, els) in table() {
            by_pattern.entry(els).or_default().push(t);
        }
        let mut collisions: Vec<(Prosign, char)> = by_pattern
            .values()
            .filter(|v| v.len() > 1)
            .map(|v| {
                assert_eq!(v.len(), 2);
                let p = v
                    .iter()
                    .find_map(|t| match t {
                        Token::Prosign(p) => Some(*p),
                        _ => None,
                    })
                    .unwrap();
                let c = v
                    .iter()
                    .find_map(|t| match t {
                        Token::Char(c) => Some(*c),
                        _ => None,
                    })
                    .unwrap();
                (p, c)
            })
            .collect();
        collisions.sort();
        let mut expected = SHARED_PATTERNS.to_vec();
        expected.sort();
        assert_eq!(collisions, expected);
    }

    #[test]
    fn table_covers_required_set() {
        let required = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.,?/=-':\"()@+!&";
        for c in required.chars() {
            assert!(pattern(Token::Char(c)).is_some(), "{c}");
        }
        for p in Prosign::ALL {
            assert!(pattern(Token::Prosign(p)).is_some(), "{p}");
        }
        assert_eq!(pattern(Token::Prosign(Prosign::Hh)).unwrap().len(), 8);
        assert_eq!(
            elements_to_string(pattern(Token::Prosign(Prosign::Sos)).unwrap()),
            "...---..."
        );
    }

    #[test]
    fn tokenize_folds_case_spaces_and_prosigns() {
        let t = tokenize("  cq   de <ar> k ").unwrap();
        assert_eq!(
            t,
            vec![
                Token::Char('C'),
                Token::Char('Q'),
                Token::WordSpace,
                Token::Char('D'),
                Token::Char('E'),
                Token::WordSpace,
                Token::Prosign(Prosign::Ar),
                Token::WordSpace,
                Token::Char('K'),
            ]
        );
        assert_eq!(tokens_to_text(&t), "CQ DE <AR> K");
        assert_eq!(tokenize("").unwrap(), vec![]);
    }

    #[test]
    fn unknown_input_is_an_error() {
        assert_eq!(
            tokenize("AB#"),
            Err(CodecError::UnknownChar { ch: '#', offset: 2 })
        );
        assert_eq!(
            tokenize("x <ZZ>"),
            Err(CodecError::UnknownProsign {
                name: "ZZ".into(),
                offset: 2
            })
        );
        assert_eq!(
            tokenize("<AR"),
            Err(CodecError::UnterminatedProsign { offset: 0 })
        );
    }

    #[test]
    fn encode_pairs_tokens_with_elements() {
        let e = encode("e t").unwrap();
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].1, &[Element::Dit]);
        assert!(e[1].1.is_empty());
        assert_eq!(e[2].1, &[Element::Dah]);
    }

    #[test]
    fn decode_unknown_group_is_none() {
        assert_eq!(decode_group(&[Element::Dah; 7]), None);
    }

    #[test]
    fn canonical_folds_shared_pairs() {
        assert_eq!(canonical(Token::Char('=')), Token::Prosign(Prosign::Bt));
        assert_eq!(canonical(Token::Char('a')), Token::Char('A'));
        assert_eq!(
            canonical(Token::Prosign(Prosign::Sk)),
            Token::Prosign(Prosign::Sk)
        );
    }
}
