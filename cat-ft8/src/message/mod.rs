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

//! Source encoding: human-readable messages ↔ 77-bit payloads (QEX 2020 §2,
//! Appendix A, Tables 1, 2 and 7).
//!
//! | `i3.n3` | Type | Example | This release |
//! |---------|------|---------|--------------|
//! | 0.0 | Free text | `TNX BOB 73 GL` | pack + unpack |
//! | 0.1 | DXpedition | `K1ABC RR73; W9XYZ <KH1/KH7Z> -08` | pack + unpack |
//! | 0.3, 0.4 | ARRL Field Day | `K1ABC W9XYZ 6A WI` | [`Error::Unsupported`] |
//! | 0.5 | Telemetry | `123456789ABCDEF012` | pack + unpack |
//! | 1 | Standard | `K1ABC/R W9XYZ/R R EN37` | pack + unpack |
//! | 2 | EU VHF (`/P`) | `G4ABC/P PA9XYZ JO22` | pack + unpack |
//! | 3 | ARRL RTTY Roundup | `K1ABC W9XYZ 579 WI` | [`Error::Unsupported`] |
//! | 4 | Nonstandard call | `<W9XYZ> PJ4/K1ABC RRR` | pack + unpack |
//! | 5 | EU VHF contest | `<G4ABC> <PA9XYZ> R 570007 JO22DB` | pack + unpack |
//!
//! Field Day and RTTY Roundup need the ARRL/RAC section and state/province
//! lookup tables, whose order is part of the protocol; they are left for a
//! later release rather than guessed. The unassigned types (0.2, 0.6, 0.7,
//! 6, 7) unpack as [`Error::InvalidPayload`].
//!
//! # Text rules
//!
//! Input is upper-cased and runs of whitespace collapse to one space. The
//! type is chosen in this order: DXpedition, standard (1/2), nonstandard
//! call (4), EU VHF contest (5), the unsupported contest shapes (reported as
//! [`Error::Unsupported`] rather than silently sent as text), telemetry,
//! then free text.
//!
//! The choices follow what WSJT-X 3.0 does with the same text, checked
//! black-box against its `ft8sim` (`tests/data/`), so both programs put the
//! same bits on the air. Where WSJT-X would silently drop or truncate part
//! of a message (free text over 13 characters, a report in a type 4
//! message, the prefix of an unbracketed compound call, a report beyond
//! ±50), this crate returns an error instead. A compound or other
//! nonstandard call must be written in angle brackets on one side to be
//! sent as type 4 (`<W9XYZ> PJ4/K1ABC`); it is never hashed implicitly.
//!
//! # Callsign memory
//!
//! Parsing has no side effects. A successful [`pack`] offers every full
//! callsign of the message to [`CallHashes::remember`] once, after the
//! payload is built (the text is the operator's own); a failed `pack`
//! remembers nothing. [`unpack`] remembers what WSJT-X 3.0 remembers when
//! it decodes (checked black-box with `jt9`): only the sending station —
//! the second call of a type 1/2 message, the full call of a type 4 message
//! when it is the sender (`<W9XYZ> PJ4/K1ABC`, `CQ PJ4/K1ABC`), and nothing
//! from DXpedition or type 5 messages. Calls are remembered as written, so
//! after `K1ABC/P` a hash of `K1ABC` alone does not resolve. Because unpacking writes to the table,
//! a decoder that wants reproducible output must unpack its decodes in a
//! fixed order on one thread, and only payloads that passed CRC and LDPC.

mod hash;
mod text;

use core::fmt::{self, Write};

pub use hash::{hash, hash22, CallHashes, CallTable, HashSize, NoHashes};
pub use text::{AsciiBuf, Call, MessageText};

use crate::Error;

/// Number of special `c28` values below the 22-bit hashes (Table 7).
pub const NTOKENS: u32 = 2_063_592;
/// Number of 22-bit hash values.
pub const MAX22: u32 = 1 << 22;
/// Number of 4-character grid locators (18 × 18 × 10 × 10).
pub const MAXGRID4: u32 = 32_400;
/// Number of standard callsigns (37 × 36 × 10 × 27³).
pub const STD_CALLS: u32 = 262_177_560;

// Every standard call fits in the 28-bit field above the tokens and hashes.
const _: () = assert!(NTOKENS + MAX22 + STD_CALLS <= 1 << 28);

const PAYLOAD_MASK: u128 = (1u128 << 77) - 1;

/// A 77-bit message payload. Bit 0 (first transmitted) is the most
/// significant of the 77; `i3` is the last three bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Payload(u128);

impl Payload {
    /// A payload from its 77 bits held in the low end of `bits`; `None` if
    /// any higher bit is set.
    pub fn from_bits(bits: u128) -> Option<Self> {
        (bits & !PAYLOAD_MASK == 0).then_some(Payload(bits))
    }

    /// A payload from a string of 77 `0`/`1` characters.
    pub fn from_bit_str(s: &str) -> Option<Self> {
        if s.len() != 77 {
            return None;
        }
        let mut v = 0u128;
        for c in s.bytes() {
            v = (v << 1)
                | match c {
                    b'0' => 0,
                    b'1' => 1,
                    _ => return None,
                };
        }
        Some(Payload(v))
    }

    /// The 77 bits, in the low end.
    pub fn bits(&self) -> u128 {
        self.0
    }

    /// Bit `i` (0 = first), as 0 or 1. Panics if `i >= 77`.
    pub fn bit(&self, i: usize) -> u8 {
        assert!(i < 77, "payload bit index out of range");
        ((self.0 >> (76 - i)) & 1) as u8
    }

    /// The payload as 10 bytes, MSB first; the last 3 bits are zero.
    pub fn to_bytes(&self) -> [u8; 10] {
        let v = self.0 << 3;
        let mut out = [0u8; 10];
        for (i, b) in out.iter_mut().enumerate() {
            *b = (v >> (72 - 8 * i)) as u8;
        }
        out
    }

    /// The message type number `i3` (last 3 bits).
    pub fn i3(&self) -> u8 {
        (self.0 & 7) as u8
    }

    /// The subtype `n3` (meaningful when `i3 == 0`).
    pub fn n3(&self) -> u8 {
        ((self.0 >> 3) & 7) as u8
    }

    /// The message type.
    pub fn message_type(&self) -> MessageType {
        MessageType::from_i3_n3(self.i3(), self.n3())
    }
}

/// The 77-bit message types (QEX 2020 Table 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageType {
    /// 0.0
    FreeText,
    /// 0.1
    DXpedition,
    /// 0.3 (`n3 = 3`, 1–16 transmitters) or 0.4 (`n3 = 4`, 17–32).
    FieldDay {
        /// 3 or 4.
        n3: u8,
    },
    /// 0.5
    Telemetry,
    /// 1
    Standard,
    /// 2 (the standard layout with `/P` instead of `/R`)
    EuVhfPortable,
    /// 3
    RttyRoundup,
    /// 4
    NonStandardCall,
    /// 5
    EuVhfContest,
    /// Not assigned (0.2, 0.6, 0.7, 6, 7).
    Reserved {
        /// `i3`
        i3: u8,
        /// `n3`, when `i3 == 0`
        n3: u8,
    },
}

impl MessageType {
    /// The type of an `i3.n3` pair.
    pub fn from_i3_n3(i3: u8, n3: u8) -> Self {
        match (i3, n3) {
            (0, 0) => MessageType::FreeText,
            (0, 1) => MessageType::DXpedition,
            (0, 3) | (0, 4) => MessageType::FieldDay { n3 },
            (0, 5) => MessageType::Telemetry,
            (0, n3) => MessageType::Reserved { i3: 0, n3 },
            (1, _) => MessageType::Standard,
            (2, _) => MessageType::EuVhfPortable,
            (3, _) => MessageType::RttyRoundup,
            (4, _) => MessageType::NonStandardCall,
            (5, _) => MessageType::EuVhfContest,
            (i3, _) => MessageType::Reserved { i3, n3: 0 },
        }
    }
}

impl fmt::Display for MessageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MessageType::FreeText => f.write_str("0.0"),
            MessageType::DXpedition => f.write_str("0.1"),
            MessageType::FieldDay { n3 } => write!(f, "0.{n3}"),
            MessageType::Telemetry => f.write_str("0.5"),
            MessageType::Standard => f.write_str("1"),
            MessageType::EuVhfPortable => f.write_str("2"),
            MessageType::RttyRoundup => f.write_str("3"),
            MessageType::NonStandardCall => f.write_str("4"),
            MessageType::EuVhfContest => f.write_str("5"),
            MessageType::Reserved { i3: 0, n3 } => write!(f, "0.{n3}"),
            MessageType::Reserved { i3, .. } => write!(f, "{i3}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Bit assembly

/// Accumulates fields MSB first into a payload.
struct Bits(u128, u32);

impl Bits {
    fn new() -> Self {
        Bits(0, 0)
    }
    fn put(mut self, value: u128, width: u32) -> Self {
        debug_assert!(
            width == 128 || value >> width == 0,
            "field overflows its width"
        );
        self.0 = (self.0 << width) | value;
        self.1 += width;
        self
    }
    fn finish(self) -> Payload {
        debug_assert_eq!(self.1, 77);
        Payload(self.0)
    }
}

/// Reads fields MSB first out of a payload.
struct Reader(u128, u32);

impl Reader {
    fn new(p: &Payload) -> Self {
        Reader(p.0, 77)
    }
    fn take(&mut self, width: u32) -> u128 {
        self.1 -= width;
        (self.0 >> self.1) & ((1u128 << width) - 1)
    }
}

// ---------------------------------------------------------------------------
// Character sets

/// Free-text alphabet (f71): space, digits, letters, `+-./?` (42).
pub(crate) const FREE_TEXT: &[u8; 42] = b" 0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ+-./?";
/// Nonstandard-call alphabet (c58): space, digits, letters, `/` (38).
pub(crate) const CALL38: &[u8; 38] = b" 0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ/";

fn index_in(table: &[u8], c: u8) -> Option<u32> {
    table.iter().position(|&t| t == c).map(|i| i as u32)
}

// ---------------------------------------------------------------------------
// Standard callsigns (c28)

/// The 28-bit number of a standard base callsign (no suffix), without the
/// `NTOKENS + MAX22` offset; `None` if it is not standard.
///
/// A standard call is a 1–2 character prefix (at least one letter), a
/// digit and a suffix of 0–3 letters. It is aligned so the digit is the
/// third of six characters. (Some programs carry `3DA0xxx` and `3XA0xx`
/// calls as `3D0xxx` and `QA0xx`; WSJT-X 3.0 does not, sending them as
/// nonstandard calls, and neither does this crate.)
fn std_call_number(call: &str) -> Option<u32> {
    let b = call.as_bytes();
    let mut c6 = [b' '; 6];
    let n = b.len();
    if (3..=6).contains(&n) && b[2].is_ascii_digit() {
        c6[..n].copy_from_slice(b);
    } else if (3..=5).contains(&n) && b[1].is_ascii_digit() {
        c6[1..=n].copy_from_slice(b);
    } else {
        return None;
    }
    let i0 = match c6[0] {
        b' ' => 0,
        b'0'..=b'9' => u32::from(c6[0] - b'0') + 1,
        b'A'..=b'Z' => u32::from(c6[0] - b'A') + 11,
        _ => return None,
    };
    let i1 = match c6[1] {
        b'0'..=b'9' => u32::from(c6[1] - b'0'),
        b'A'..=b'Z' => u32::from(c6[1] - b'A') + 10,
        _ => return None,
    };
    if !c6[2].is_ascii_digit() {
        return None;
    }
    // At least one letter in the prefix.
    if !(c6[0].is_ascii_uppercase() || c6[1].is_ascii_uppercase()) {
        return None;
    }
    let i2 = u32::from(c6[2] - b'0');
    let mut n28 = ((i0 * 36) + i1) * 10 + i2;
    let mut seen_space = false;
    for &c in &c6[3..] {
        let v = match c {
            b' ' => {
                seen_space = true;
                0
            }
            b'A'..=b'Z' if !seen_space => u32::from(c - b'A') + 1,
            _ => return None,
        };
        n28 = n28 * 27 + v;
    }
    Some(n28)
}

/// Inverse of [`std_call_number`]; `InvalidPayload` for any value that
/// function never returns.
fn std_call_text(n_orig: u32) -> Result<Call, Error> {
    let mut n = n_orig;
    if n >= STD_CALLS {
        return Err(Error::InvalidPayload);
    }
    let mut c6 = [b' '; 6];
    for slot in c6[3..].iter_mut().rev() {
        let v = (n % 27) as u8;
        *slot = if v == 0 { b' ' } else { b'A' + v - 1 };
        n /= 27;
    }
    c6[2] = b'0' + (n % 10) as u8;
    n /= 10;
    let v = (n % 36) as u8;
    c6[1] = if v < 10 { b'0' + v } else { b'A' + v - 10 };
    n /= 36;
    let v = n as u8; // < 37
    c6[0] = match v {
        0 => b' ',
        1..=10 => b'0' + v - 1,
        _ => b'A' + v - 11,
    };
    let s = core::str::from_utf8(&c6)
        .map_err(|_| Error::InvalidPayload)?
        .trim();
    // Only values `pack` could have produced: the text must pack back to
    // `n` (this rejects, for example, an all-digit prefix such as `65`).
    if std_call_number(s) != Some(n_orig) {
        return Err(Error::InvalidPayload);
    }
    Call::from_ascii(s).ok_or(Error::InvalidPayload)
}

/// A callsign suffix carried by the r1/p1 bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Suffix {
    None,
    R,
    P,
}

/// One `c28` field as parsed from text.
#[derive(Debug, Clone, Copy)]
struct C28 {
    value: u32,
    suffix: Suffix,
}

/// True for 1–11 characters from ` 0-9A-Z/` (a hashable call).
fn hashable(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 11
        && s.bytes()
            .all(|c| c != b' ' && index_in(CALL38, c).is_some())
}

/// The call inside `<...>`, if `tok` is bracketed and hashable.
fn bracketed(tok: &str) -> Option<&str> {
    let inner = tok.strip_prefix('<')?.strip_suffix('>')?;
    hashable(inner).then_some(inner)
}

/// A plausible callsign: 3–11 characters of `0-9A-Z/` with at least one
/// letter and one digit.
fn plausible_call(s: &str) -> bool {
    (3..=11).contains(&s.len())
        && hashable(s)
        && s.bytes().any(|c| c.is_ascii_digit())
        && s.bytes().any(|c| c.is_ascii_uppercase())
}

/// The `c28` value of a CQ modifier: three digits (`CQ 000`–`CQ 999`) or
/// one to four letters (`CQ A`–`CQ ZZZZ`), Table 7.
fn cq_modifier(tok: &str) -> Option<u32> {
    let b = tok.as_bytes();
    if b.len() == 3 && b.iter().all(u8::is_ascii_digit) {
        let v = b.iter().fold(0u32, |a, &c| a * 10 + u32::from(c - b'0'));
        return Some(3 + v);
    }
    if (1..=4).contains(&b.len()) && b.iter().all(u8::is_ascii_uppercase) {
        let m = b
            .iter()
            .fold(0u32, |a, &c| a * 27 + u32::from(c - b'A') + 1);
        return Some(1003 + m);
    }
    None
}

/// A `c28` value as it reads in a message.
enum Field {
    /// `DE`, `QRZ`, `CQ` or `CQ` with a modifier.
    Token(AsciiBuf<8>),
    /// A standard call, with its `/R` or `/P` suffix if any.
    Call(Call),
    /// A 22-bit hash.
    Hash22(u32),
}

/// Decodes a `c28` value. `suffix_bit` and `i3` add `/R` (type 1) or `/P`
/// (type 2).
fn c28_field(n: u32, suffix_bit: bool, i3: u8) -> Result<Field, Error> {
    let bad = |_| Error::InvalidPayload;
    if n < NTOKENS {
        let mut t = AsciiBuf::<8>::new();
        match n {
            0 => t.push_str("DE").map_err(bad)?,
            1 => t.push_str("QRZ").map_err(bad)?,
            2 => t.push_str("CQ").map_err(bad)?,
            3..=1002 => write!(t, "CQ {:03}", n - 3).map_err(bad)?,
            1004..=532_443 => {
                let mut m = n - 1003;
                let mut letters = [b' '; 4];
                for slot in letters.iter_mut().rev() {
                    let v = (m % 27) as u8;
                    *slot = if v == 0 { b' ' } else { b'A' + v - 1 };
                    m /= 27;
                }
                let s = core::str::from_utf8(&letters).map_err(|_| Error::InvalidPayload)?;
                let s = s.trim_start();
                if s.contains(' ') {
                    return Err(Error::InvalidPayload);
                }
                write!(t, "CQ {s}").map_err(bad)?;
            }
            _ => return Err(Error::InvalidPayload),
        }
        return Ok(Field::Token(t));
    }
    let n = n - NTOKENS;
    if n < MAX22 {
        return Ok(Field::Hash22(n));
    }
    let mut call = std_call_text(n - MAX22)?;
    if suffix_bit {
        let suffix = match i3 {
            1 => "/R",
            2 => "/P",
            _ => return Err(Error::InvalidPayload),
        };
        call.push_str(suffix).map_err(bad)?;
    }
    Ok(Field::Call(call))
}

/// Writes a decoded `c28` field. Returns the full call it shows, for the
/// caller to remember once every hash in the message has been looked up.
fn write_field<H: CallHashes + ?Sized>(
    out: &mut MessageText,
    field: &Field,
    hashes: &H,
) -> Result<Option<Call>, Error> {
    match field {
        Field::Token(t) => out.push_str(t).map_err(|_| Error::InvalidPayload)?,
        Field::Call(c) => {
            out.push_str(c).map_err(|_| Error::InvalidPayload)?;
            return Ok(Some(*c));
        }
        Field::Hash22(h) => write_hashed(out, HashSize::Bits22, *h, hashes)?,
    }
    Ok(None)
}

/// Writes `<CALL>` for a hash, or `<...>` when the table has no match.
fn write_hashed<H: CallHashes + ?Sized>(
    out: &mut MessageText,
    size: HashSize,
    h: u32,
    hashes: &H,
) -> Result<(), Error> {
    let r = match hashes.lookup(size, h) {
        Some(call) => write!(out, "<{call}>"),
        None => out.push_str("<...>"),
    };
    r.map_err(|_| Error::InvalidPayload)
}

// ---------------------------------------------------------------------------
// Grid, report and acknowledgement (R1 + g15)

/// Parses the words after the two calls of a standard message into the
/// 16-bit `R1 g15` value. `None` if they are not one of the allowed forms.
fn parse_g16(rest: &[&str]) -> Result<Option<u32>, Error> {
    let g = match rest {
        [] => MAXGRID4 + 1,
        ["RRR"] => MAXGRID4 + 2,
        // `RR73` is also the locator RR73 (g15 = 32 373); WSJT-X sends it
        // as that locator, not as the reserved value MAXGRID4 + 3, so this
        // does too. Both unpack to `RR73`.
        ["RR73"] => grid4("RR73").unwrap_or(MAXGRID4 + 3),
        ["73"] => MAXGRID4 + 4,
        ["R", grid] => match grid4(grid) {
            Some(g) => (1 << 15) | g,
            None => return Ok(None),
        },
        [word] => {
            if let Some(g) = grid4(word) {
                g
            } else if let Some(r) = word.strip_prefix('R').and_then(signed_report) {
                (1 << 15) | report_g15(r)?
            } else if let Some(r) = signed_report(word) {
                report_g15(r)?
            } else {
                return Ok(None);
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(g))
}

/// A 4-character Maidenhead locator `AA00`–`RR99` as its g15 value.
fn grid4(s: &str) -> Option<u32> {
    let b = s.as_bytes();
    if b.len() != 4
        || !(b'A'..=b'R').contains(&b[0])
        || !(b'A'..=b'R').contains(&b[1])
        || !b[2].is_ascii_digit()
        || !b[3].is_ascii_digit()
    {
        return None;
    }
    let v = ((u32::from(b[0] - b'A') * 18 + u32::from(b[1] - b'A')) * 10 + u32::from(b[2] - b'0'))
        * 10
        + u32::from(b[3] - b'0');
    Some(v)
}

/// `+nn` / `-nn` (sign required, one or two digits).
fn signed_report(s: &str) -> Option<i32> {
    let (sign, digits) = match s.as_bytes().first()? {
        b'+' => (1, &s[1..]),
        b'-' => (-1, &s[1..]),
        _ => return None,
    };
    if digits.is_empty() || digits.len() > 2 || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(
        sign * digits
            .bytes()
            .fold(0i32, |a, c| a * 10 + i32::from(c - b'0')),
    )
}

/// A type 1 signal report as its g15 value.
///
/// The paper gives −30 to +99 dB as `irpt = dd + 35`. WSJT-X 3.0 instead
/// carries −50 to +50: −30…+50 as `dd + 35` (irpt 5–85) and −50…−31 as
/// `dd + 136` (irpt 86–105), and unpacks any `dd > 50` as `dd − 101`. This
/// crate follows WSJT-X so both programs agree on every report.
fn report_g15(dd: i32) -> Result<u32, Error> {
    let irpt = match dd {
        -30..=50 => dd + 35,
        -50..=-31 => dd + 136,
        _ => return Err(Error::InvalidReport),
    };
    Ok(MAXGRID4 + irpt as u32)
}

fn write_g16<W: Write>(out: &mut W, r: bool, g: u32) -> Result<(), Error> {
    let res = if g < MAXGRID4 {
        let b = [
            b'A' + (g / 1800) as u8,
            b'A' + ((g / 100) % 18) as u8,
            b'0' + ((g / 10) % 10) as u8,
            b'0' + (g % 10) as u8,
        ];
        let s = core::str::from_utf8(&b).map_err(|_| Error::InvalidPayload)?;
        if r {
            write!(out, " R {s}")
        } else {
            write!(out, " {s}")
        }
    } else {
        match g - MAXGRID4 {
            0 => return Err(Error::InvalidPayload),
            1 => Ok(()),
            2 => out.write_str(" RRR"),
            3 => out.write_str(" RR73"),
            4 => out.write_str(" 73"),
            irpt => {
                let mut dd = irpt as i32 - 35;
                if dd > 50 {
                    dd -= 101;
                }
                if r {
                    write!(out, " R{dd:+03}")
                } else {
                    write!(out, " {dd:+03}")
                }
            }
        }
    };
    res.map_err(|_| Error::InvalidPayload)
}

// ---------------------------------------------------------------------------
// Packing

/// Most words any message type takes.
const MAX_WORDS: usize = 8;
/// Longest normalized input considered.
const MAX_INPUT: usize = 64;
/// Stack scratch `pack` reserves: the normalized text and the word list.
pub const PACK_SCRATCH_BYTES: usize = MAX_INPUT + MAX_WORDS * core::mem::size_of::<&str>();

/// Upper-cases `text` and collapses whitespace into `buf`.
fn normalize<'a>(text: &str, buf: &'a mut [u8; MAX_INPUT]) -> Result<&'a str, Error> {
    let mut n = 0;
    for word in text.split_ascii_whitespace() {
        let extra = usize::from(n > 0);
        if n + extra + word.len() > MAX_INPUT || !word.is_ascii() {
            return Err(Error::NotEncodable);
        }
        if extra == 1 {
            buf[n] = b' ';
            n += 1;
        }
        for (d, s) in buf[n..n + word.len()].iter_mut().zip(word.bytes()) {
            *d = s.to_ascii_uppercase();
        }
        n += word.len();
    }
    if n == 0 {
        return Err(Error::Empty);
    }
    core::str::from_utf8(&buf[..n]).map_err(|_| Error::NotEncodable)
}

/// A packed payload and the full callsigns it carries, remembered only
/// once the whole message has packed.
struct Packed<'a> {
    payload: Payload,
    calls: [Option<&'a str>; 3],
}

impl<'a> Packed<'a> {
    fn new(payload: Payload, calls: [Option<&'a str>; 3]) -> Self {
        Packed { payload, calls }
    }
}

/// `Ok(None)`: the words are not this message type. `Err`: they are, but a
/// field is out of range.
type Attempt<'a> = Result<Option<Packed<'a>>, Error>;

/// Packs a message into its 77-bit payload.
///
/// On success, the full callsigns in the message are offered to `hashes`
/// (see [`CallHashes`]), so a later [`unpack`] with the same table can
/// show `<PJ4/K1ABC>` for its hash. On failure nothing is remembered.
pub fn pack<H: CallHashes + ?Sized>(text: &str, hashes: &mut H) -> Result<Payload, Error> {
    let mut buf = [0u8; MAX_INPUT];
    let norm = normalize(text, &mut buf)?;
    let structured = match split_words(norm) {
        Some((words, count)) => pack_words(&words[..count])?,
        None => None,
    };
    let packed = match structured {
        Some(p) => p,
        None => {
            let p = match pack_telemetry(norm) {
                Some(p) => p,
                None => pack_free_text(norm)?,
            };
            Packed::new(p, [None; 3])
        }
    };
    for call in packed.calls.iter().flatten() {
        hashes.remember(call);
    }
    Ok(packed.payload)
}

/// The words of `norm`, or `None` if there are more than any type takes.
fn split_words(norm: &str) -> Option<([&str; MAX_WORDS], usize)> {
    let mut words = [""; MAX_WORDS];
    let mut count = 0;
    for w in norm.split(' ') {
        *words.get_mut(count)? = w;
        count += 1;
    }
    Some((words, count))
}

/// Tries the word-structured types in order.
fn pack_words<'a>(words: &[&'a str]) -> Attempt<'a> {
    if let Some(p) = pack_dxpedition(words)? {
        return Ok(Some(p));
    }
    if let Some(p) = pack_standard(words)? {
        return Ok(Some(p));
    }
    if let Some(p) = pack_nonstandard(words)? {
        return Ok(Some(p));
    }
    if let Some(p) = pack_eu_vhf(words)? {
        return Ok(Some(p));
    }
    match unsupported_shape(words) {
        Some(t) => Err(Error::Unsupported(t)),
        None => Ok(None),
    }
}

/// A word in a callsign position, classified.
#[derive(Debug, Clone, Copy)]
enum CallWord<'a> {
    /// A standard call (the word itself), with its `c28` value and suffix.
    Std(C28, &'a str),
    /// `<CALL>`: sent as a hash. `compound` if the call inside has a `/`
    /// and is not a standard call with `/R` or `/P` (for example
    /// `PJ4/K1ABC`). Special-event calls without `/` (`YW18FIFA`) are not
    /// compound: WSJT-X sends two of those, bracketed, as type 1.
    Hashed { inner: &'a str, compound: bool },
    /// A nonstandard call sent in full (type 4 only).
    Nonstd(&'a str),
}

impl<'a> CallWord<'a> {
    fn classify(w: &'a str) -> Option<Self> {
        if let Some(inner) = bracketed(w) {
            return Some(CallWord::Hashed {
                inner,
                compound: inner.contains('/') && std_call_with_suffix(inner).is_none(),
            });
        }
        if let Some(c) = std_call_with_suffix(w) {
            return Some(CallWord::Std(c, w));
        }
        plausible_call(w).then_some(CallWord::Nonstd(w))
    }

    /// The full call this word names (without brackets).
    fn call(&self) -> &'a str {
        match *self {
            CallWord::Std(_, s) | CallWord::Nonstd(s) => s,
            CallWord::Hashed { inner, .. } => inner,
        }
    }
}

/// Type 1 or 2: `c28 r1 c28 r1 R1 g15 i3`.
///
/// The first field is `CQ` (optionally with a modifier), `DE`, `QRZ` or a
/// call; the second is a call. A `<hashed>` call is allowed beside a
/// standard call without suffix, or beside another hashed call when
/// neither is a compound (`/`) call;
/// other combinations with a hashed call are type 4 messages. These rules
/// follow what WSJT-X 3.0 accepts, so both programs pack the same text the
/// same way.
fn pack_standard<'a>(words: &[&'a str]) -> Attempt<'a> {
    const NO: Suffix = Suffix::None;
    let token = |value| C28 { value, suffix: NO };
    // (first field, the first word if it is a call, the remaining words)
    let (first, first_word, rest) = match words {
        ["CQ", m, call, ..] if CallWord::classify(call).is_some() => match cq_modifier(m) {
            Some(v) => (token(v), None, &words[2..]),
            None => (token(2), None, &words[1..]),
        },
        ["CQ", ..] => (token(2), None, &words[1..]),
        ["DE", ..] => (token(0), None, &words[1..]),
        ["QRZ", ..] => (token(1), None, &words[1..]),
        [w, ..] => match CallWord::classify(w) {
            Some(c) => (token(0), Some(c), &words[1..]),
            None => return Ok(None),
        },
        [] => return Ok(None),
    };
    let Some(second) = rest.first().and_then(|w| CallWord::classify(w)) else {
        return Ok(None);
    };
    let (a, b) = match (first_word, second) {
        // A token, then a standard call.
        (None, CallWord::Std(b, _)) => (first, b),
        (None, _) => return Ok(None),
        (Some(CallWord::Std(a, _)), CallWord::Std(b, _)) => (a, b),
        (Some(CallWord::Hashed { inner, .. }), CallWord::Std(b, _)) if b.suffix == NO => {
            (hashed_c28(inner)?, b)
        }
        (Some(CallWord::Std(a, _)), CallWord::Hashed { inner, .. }) if a.suffix == NO => {
            (a, hashed_c28(inner)?)
        }
        (
            Some(CallWord::Hashed {
                inner: x,
                compound: false,
            }),
            CallWord::Hashed {
                inner: y,
                compound: false,
            },
        ) => (hashed_c28(x)?, hashed_c28(y)?),
        _ => return Ok(None),
    };
    let Some(g16) = parse_g16(&rest[1..])? else {
        return Ok(None);
    };
    let i3 = match (a.suffix, b.suffix) {
        (Suffix::P, Suffix::R) | (Suffix::R, Suffix::P) => return Err(Error::InvalidCallsign),
        (Suffix::P, _) | (_, Suffix::P) => 2,
        _ => 1,
    };
    let p = Bits::new()
        .put(u128::from(a.value), 28)
        .put(u128::from(a.suffix != NO), 1)
        .put(u128::from(b.value), 28)
        .put(u128::from(b.suffix != NO), 1)
        .put(u128::from(g16), 16)
        .put(i3, 3)
        .finish();
    let calls = [first_word.map(|c| c.call()), Some(second.call()), None];
    Ok(Some(Packed::new(p, calls)))
}

/// The `c28` value of a hashed call.
fn hashed_c28(inner: &str) -> Result<C28, Error> {
    let h = hash22(inner).ok_or(Error::InvalidCallsign)?;
    Ok(C28 {
        value: NTOKENS + h,
        suffix: Suffix::None,
    })
}

/// A standard call with an optional `/R` or `/P` suffix, as a `c28` value.
fn std_call_with_suffix(w: &str) -> Option<C28> {
    let (base, suffix) = if let Some(b) = w.strip_suffix("/R") {
        (b, Suffix::R)
    } else if let Some(b) = w.strip_suffix("/P") {
        (b, Suffix::P)
    } else {
        (w, Suffix::None)
    };
    let n = std_call_number(base)?;
    Some(C28 {
        value: NTOKENS + MAX22 + n,
        suffix,
    })
}

/// Type 0.1: `c28 c28 h10 r5 n3 i3`, text `K1ABC RR73; W9XYZ <KH1/KH7Z> -08`.
///
/// The two `c28` calls are standard (no suffix) or `<hashed>`. The report
/// is carried in 2 dB steps from −30 to +32; odd reports are rounded down
/// (−11 is sent as −12), matching WSJT-X.
fn pack_dxpedition<'a>(words: &[&'a str]) -> Attempt<'a> {
    let [c1, "RR73;", c2, h, rpt] = *words else {
        return Ok(None);
    };
    let (Some(fox), Some(dd)) = (bracketed(h), signed_report(rpt)) else {
        return Ok(None);
    };
    let c28 = |w: &'a str| -> Result<(C28, &'a str), Error> {
        match CallWord::classify(w) {
            Some(CallWord::Std(c, s)) if c.suffix == Suffix::None => Ok((c, s)),
            Some(CallWord::Hashed { inner, .. }) => Ok((hashed_c28(inner)?, inner)),
            _ => Err(Error::InvalidCallsign),
        }
    };
    let ((a, call_a), (b, call_b)) = (c28(c1)?, c28(c2)?);
    if !(-30..=32).contains(&dd) {
        return Err(Error::InvalidReport);
    }
    let h10 = hash(fox, HashSize::Bits10).ok_or(Error::InvalidCallsign)?;
    let r5 = ((dd + 30) / 2) as u128;
    let p = Bits::new()
        .put(u128::from(a.value), 28)
        .put(u128::from(b.value), 28)
        .put(u128::from(h10), 10)
        .put(r5, 5)
        .put(1, 3)
        .put(0, 3)
        .finish();
    Ok(Some(Packed::new(
        p,
        [Some(call_a), Some(call_b), Some(fox)],
    )))
}

/// Type 4: `h12 c58 h1 r2 c1 i3`, one call of up to 11 characters sent in
/// full and the other as a 12-bit hash, with an optional `RRR`, `RR73` or
/// `73`.
///
/// Forms (as WSJT-X 3.0 accepts them): `CQ PJ4/K1ABC` (also written
/// `CQ <PJ4/K1ABC>`); `<W9XYZ> PJ4/K1ABC` and `PJ4/K1ABC <W9XYZ>`, where
/// the full call is nonstandard or carries `/R` or `/P`; and two bracketed
/// calls of which at least one is a compound (`/`) call, where the first is
/// hashed and the second sent in full. A grid or report cannot be carried; such
/// text is rejected rather than silently shortened.
fn pack_nonstandard<'a>(words: &[&'a str]) -> Attempt<'a> {
    let full = |w: CallWord<'_>| -> bool {
        matches!(w, CallWord::Nonstd(_))
            || matches!(w, CallWord::Std(c, _) if c.suffix != Suffix::None)
    };
    let (c1, h1, hashed, plain, r2): (u128, u128, Option<&str>, &str, u128) = match *words {
        ["CQ", w] => match CallWord::classify(w) {
            Some(CallWord::Nonstd(s)) => (1, 0, None, s, 0),
            // `CQ <K1ABC>` is refused: a standard call goes in a type 1 CQ.
            Some(CallWord::Hashed {
                inner,
                compound: true,
            }) => (1, 0, None, inner, 0),
            _ => return Ok(None),
        },
        [a, b] | [a, b, _] => {
            let r2 = match words.get(2) {
                None => 0,
                Some(&"RRR") => 1,
                Some(&"RR73") => 2,
                Some(&"73") => 3,
                Some(_) => return Ok(None),
            };
            let (Some(ca), Some(cb)) = (CallWord::classify(a), CallWord::classify(b)) else {
                return Ok(None);
            };
            match (ca, cb) {
                (CallWord::Hashed { inner, .. }, w) if full(w) => (0, 0, Some(inner), b, r2),
                (w, CallWord::Hashed { inner, .. }) if full(w) => (0, 1, Some(inner), a, r2),
                (
                    CallWord::Hashed {
                        inner: x,
                        compound: cx,
                    },
                    CallWord::Hashed {
                        inner: y,
                        compound: cy,
                    },
                ) if cx || cy => (0, 0, Some(x), y, r2),
                _ => return Ok(None),
            }
        }
        _ => return Ok(None),
    };
    // With `CQ` the paper says h12 is ignored; WSJT-X fills it with the
    // hash of the call itself, and so does this crate.
    let h12 = hash(hashed.unwrap_or(plain), HashSize::Bits12).ok_or(Error::InvalidCallsign)?;
    let mut n58: u128 = 0;
    for c in plain.bytes() {
        n58 = n58 * 38 + u128::from(index_in(CALL38, c).ok_or(Error::InvalidCallsign)?);
    }
    let p = Bits::new()
        .put(u128::from(h12), 12)
        .put(n58, 58)
        .put(h1, 1)
        .put(r2, 2)
        .put(c1, 1)
        .put(4, 3)
        .finish();
    Ok(Some(Packed::new(p, [hashed, Some(plain), None])))
}

/// Type 5 (EU VHF contest): `h12 h22 R1 r3 s11 g25 i3`, text
/// `<G4ABC> <PA9XYZ> [R] 570007 JO22DB`: both calls hashed (12 and 22
/// bits), an optional `R`, a report 52–59 and a serial 0–2047 written
/// together as six digits, and a 6-character locator. (Field layout per
/// QEX Table 1; value encodings confirmed black-box against WSJT-X.)
fn pack_eu_vhf<'a>(words: &[&'a str]) -> Attempt<'a> {
    let (a, b, r, rs, grid) = match *words {
        [a, b, "R", rs, grid] => (a, b, true, rs, grid),
        [a, b, rs, grid] => (a, b, false, rs, grid),
        _ => return Ok(None),
    };
    let Some(g25) = grid6(grid) else {
        return Ok(None);
    };
    let digits = rs.as_bytes();
    if digits.len() != 6 || !digits.iter().all(u8::is_ascii_digit) {
        return Ok(None);
    }
    // Shape settled: from here on, errors are reported.
    let (Some(x), Some(y)) = (bracketed(a), bracketed(b)) else {
        return Err(Error::InvalidCallsign);
    };
    let report = u32::from(digits[0] - b'0') * 10 + u32::from(digits[1] - b'0');
    let serial = digits[2..]
        .iter()
        .fold(0u32, |acc, &c| acc * 10 + u32::from(c - b'0'));
    if !(52..=59).contains(&report) || serial > 2047 {
        return Err(Error::InvalidReport);
    }
    let h12 = hash(x, HashSize::Bits12).ok_or(Error::InvalidCallsign)?;
    let h22 = hash22(y).ok_or(Error::InvalidCallsign)?;
    let p = Bits::new()
        .put(u128::from(h12), 12)
        .put(u128::from(h22), 22)
        .put(u128::from(r), 1)
        .put(u128::from(report - 52), 3)
        .put(u128::from(serial), 11)
        .put(u128::from(g25), 25)
        .put(5, 3)
        .finish();
    Ok(Some(Packed::new(p, [Some(x), Some(y), None])))
}

/// Number of 6-character locators (18 × 18 × 10 × 10 × 24 × 24).
const MAXGRID6: u32 = 18_662_400;

/// A 6-character locator `AA00AA`–`RR99XX` as its g25 value.
fn grid6(s: &str) -> Option<u32> {
    let b = s.as_bytes();
    if b.len() != 6 || !(b'A'..=b'X').contains(&b[4]) || !(b'A'..=b'X').contains(&b[5]) {
        return None;
    }
    let g4 = grid4(core::str::from_utf8(&b[..4]).ok()?)?;
    Some((g4 * 24 + u32::from(b[4] - b'A')) * 24 + u32::from(b[5] - b'A'))
}

/// Type 0.5: one word of 1–18 hexadecimal digits (at most 71 bits), as
/// WSJT-X 3.0 does: any single all-hex word is telemetry, so `ABC` and
/// even `73` alone are sent as 0.5, not as free text.
fn pack_telemetry(norm: &str) -> Option<Payload> {
    let b = norm.as_bytes();
    if !(1..=18).contains(&b.len()) || !b.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let mut v: u128 = 0;
    for &c in b {
        v = (v << 4) | u128::from((c as char).to_digit(16)?);
    }
    if v >> 71 != 0 {
        return None;
    }
    Some(Bits::new().put(v, 71).put(5, 3).put(0, 3).finish())
}

/// Type 0.0: up to 13 characters of the free-text alphabet as a base-42
/// number. Shorter text is right-aligned (padded with leading spaces, the
/// digit 0), as WSJT-X 3.0 sends it; unpacking trims spaces either way.
fn pack_free_text(norm: &str) -> Result<Payload, Error> {
    let b = norm.as_bytes();
    if b.len() > 13 {
        return Err(Error::NotEncodable);
    }
    let mut v: u128 = 0;
    for &c in b {
        v = v * 42 + u128::from(index_in(FREE_TEXT, c).ok_or(Error::NotEncodable)?);
    }
    Ok(Bits::new().put(v, 71).put(0, 3).put(0, 3).finish())
}

/// Recognises the contest message shapes this release does not pack.
fn unsupported_shape(words: &[&str]) -> Option<MessageType> {
    let call = |w: &str| plausible_call(w) || bracketed(w).is_some();
    let body = match words {
        ["TU;", rest @ ..] => rest,
        _ => words,
    };
    let tu = body.len() != words.len();
    let (calls_ok, tail) = match body {
        [a, b, tail @ ..] => (call(a) && call(b), tail),
        _ => return None,
    };
    if !calls_ok {
        return None;
    }
    let tail = match tail {
        ["R", t @ ..] => t,
        t => t,
    };
    let letters = |w: &str, n: core::ops::RangeInclusive<usize>| {
        n.contains(&w.len()) && w.bytes().all(|c| c.is_ascii_uppercase())
    };
    let digits = |w: &str, n: core::ops::RangeInclusive<usize>| {
        n.contains(&w.len()) && w.bytes().all(|c| c.is_ascii_digit())
    };
    let [class, section] = *tail else {
        return None;
    };
    // Field Day: `6A WI`, 1–32 transmitters, class A–F, section.
    let cb = class.as_bytes();
    let (num, cls) = cb.split_at(cb.len().saturating_sub(1));
    let num = core::str::from_utf8(num).ok()?;
    if !tu && digits(num, 1..=2) && matches!(cls, [b'A'..=b'F']) && letters(section, 2..=3) {
        let ntx: u32 = num.parse().ok()?;
        if (1..=32).contains(&ntx) {
            return Some(MessageType::FieldDay {
                n3: if ntx <= 16 { 3 } else { 4 },
            });
        }
    }
    // RTTY Roundup: `579 WI` or `579 0012`.
    if digits(class, 3..=3) && (letters(section, 2..=3) || digits(section, 1..=4)) {
        return Some(MessageType::RttyRoundup);
    }
    None
}

// ---------------------------------------------------------------------------
// Unpacking

/// Unpacks a payload to its text.
///
/// Hashed callsigns are resolved through `hashes` (`<...>` on a miss).
/// The sending station's full call (see the module docs) is offered to it
/// afterwards, once every hash in the message has been looked up and the
/// whole payload has decoded, so a call in the message never shadows a
/// hash in the same message, and an invalid payload remembers nothing. Field Day and RTTY
/// Roundup payloads return [`Error::Unsupported`]; unassigned types and
/// field values no message defines return [`Error::InvalidPayload`].
pub fn unpack<H: CallHashes + ?Sized>(
    payload: &Payload,
    hashes: &mut H,
) -> Result<MessageText, Error> {
    let mut out = MessageText::new();
    let mut heard: [Option<Call>; 2] = [None, None];
    let w = |_: fmt::Error| Error::InvalidPayload;
    let mut r = Reader::new(payload);
    match payload.message_type() {
        MessageType::Standard | MessageType::EuVhfPortable => {
            let i3 = payload.i3();
            let a = r.take(28) as u32;
            let ra = r.take(1) == 1;
            let b = r.take(28) as u32;
            let rb = r.take(1) == 1;
            let rr = r.take(1) == 1;
            let g = r.take(15) as u32;
            // Decode every field before writing or remembering anything.
            let (fa, fb) = (c28_field(a, ra, i3)?, c28_field(b, rb, i3)?);
            let mut tail = AsciiBuf::<12>::new();
            write_g16(&mut tail, rr, g)?;
            // Only the sending (second) station is remembered, as WSJT-X does.
            write_field(&mut out, &fa, hashes)?;
            out.push(b' ').map_err(w)?;
            heard[1] = write_field(&mut out, &fb, hashes)?;
            out.push_str(&tail).map_err(w)?;
        }
        MessageType::DXpedition => {
            let a = r.take(28) as u32;
            let b = r.take(28) as u32;
            let h10 = r.take(10) as u32;
            let r5 = r.take(5) as i32;
            let (fa, fb) = (c28_field(a, false, 0)?, c28_field(b, false, 0)?);
            // WSJT-X remembers neither call of a DXpedition message.
            write_field(&mut out, &fa, hashes)?;
            out.push_str(" RR73; ").map_err(w)?;
            write_field(&mut out, &fb, hashes)?;
            out.push(b' ').map_err(w)?;
            write_hashed(&mut out, HashSize::Bits10, h10, hashes)?;
            write!(out, " {:+03}", 2 * r5 - 30).map_err(w)?;
        }
        MessageType::NonStandardCall => {
            let h12 = r.take(12) as u32;
            let mut n58 = r.take(58);
            let h1 = r.take(1) == 1;
            let r2 = r.take(2);
            let c1 = r.take(1) == 1;
            let mut c11 = [b' '; 11];
            for slot in c11.iter_mut().rev() {
                *slot = CALL38[(n58 % 38) as usize];
                n58 /= 38;
            }
            if n58 != 0 {
                return Err(Error::InvalidPayload);
            }
            let plain = core::str::from_utf8(&c11)
                .map_err(|_| Error::InvalidPayload)?
                .trim();
            if plain.is_empty() || plain.contains(' ') {
                return Err(Error::InvalidPayload);
            }
            // The full call is remembered when it is the sender (second, or
            // after CQ), as WSJT-X does.
            if c1 || !h1 {
                heard[0] = Call::from_ascii(plain);
            }
            if c1 {
                write!(out, "CQ {plain}").map_err(w)?;
            } else {
                if h1 {
                    out.push_str(plain).map_err(w)?;
                    out.push(b' ').map_err(w)?;
                    write_hashed(&mut out, HashSize::Bits12, h12, hashes)?;
                } else {
                    write_hashed(&mut out, HashSize::Bits12, h12, hashes)?;
                    out.push(b' ').map_err(w)?;
                    out.push_str(plain).map_err(w)?;
                }
                match r2 {
                    1 => out.push_str(" RRR").map_err(w)?,
                    2 => out.push_str(" RR73").map_err(w)?,
                    3 => out.push_str(" 73").map_err(w)?,
                    _ => {}
                }
            }
        }
        MessageType::EuVhfContest => {
            let h12 = r.take(12) as u32;
            let h22 = r.take(22) as u32;
            let rr = r.take(1) == 1;
            let r3 = r.take(3) as u32;
            let serial = r.take(11) as u32;
            let mut g = r.take(25) as u32;
            if g >= MAXGRID6 {
                return Err(Error::InvalidPayload);
            }
            let mut loc = [0u8; 6];
            loc[5] = b'A' + (g % 24) as u8;
            g /= 24;
            loc[4] = b'A' + (g % 24) as u8;
            g /= 24;
            loc[3] = b'0' + (g % 10) as u8;
            g /= 10;
            loc[2] = b'0' + (g % 10) as u8;
            g /= 10;
            loc[1] = b'A' + (g % 18) as u8;
            loc[0] = b'A' + (g / 18) as u8;
            let loc = core::str::from_utf8(&loc).map_err(|_| Error::InvalidPayload)?;
            write_hashed(&mut out, HashSize::Bits12, h12, hashes)?;
            out.push(b' ').map_err(w)?;
            write_hashed(&mut out, HashSize::Bits22, h22, hashes)?;
            if rr {
                out.push_str(" R").map_err(w)?;
            }
            write!(out, " {}{:04} {loc}", 52 + r3, serial).map_err(w)?;
        }
        MessageType::FreeText => {
            let mut v = r.take(71);
            let mut c13 = [b' '; 13];
            for slot in c13.iter_mut().rev() {
                *slot = FREE_TEXT[(v % 42) as usize];
                v /= 42;
            }
            if v != 0 {
                return Err(Error::InvalidPayload);
            }
            let s = core::str::from_utf8(&c13)
                .map_err(|_| Error::InvalidPayload)?
                .trim();
            out.push_str(s).map_err(w)?;
        }
        MessageType::Telemetry => {
            let v = r.take(71);
            write!(out, "{v:X}").map_err(w)?;
        }
        t @ (MessageType::FieldDay { .. } | MessageType::RttyRoundup) => {
            return Err(Error::Unsupported(t))
        }
        MessageType::Reserved { .. } => return Err(Error::InvalidPayload),
    }
    for call in heard.iter().flatten() {
        hashes.remember(call);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    fn roundtrip(text: &str) -> MessageText {
        let mut t: CallTable<16> = CallTable::new();
        let p = pack(text, &mut t).unwrap();
        unpack(&p, &mut t).unwrap()
    }

    #[test]
    fn cq_modifier_values_match_qex_table_7() {
        assert_eq!(cq_modifier("000"), Some(3));
        assert_eq!(cq_modifier("999"), Some(1002));
        assert_eq!(cq_modifier("A"), Some(1004));
        assert_eq!(cq_modifier("Z"), Some(1029));
        assert_eq!(cq_modifier("AA"), Some(1031));
        assert_eq!(cq_modifier("ZZ"), Some(1731));
        assert_eq!(cq_modifier("AAA"), Some(1760));
        assert_eq!(cq_modifier("ZZZ"), Some(20685));
        assert_eq!(cq_modifier("AAAA"), Some(21443));
        assert_eq!(cq_modifier("ZZZZ"), Some(532_443));
        assert_eq!(cq_modifier("AB1"), None);
    }

    #[test]
    fn standard_call_range_is_all_of_table_7() {
        assert_eq!(NTOKENS + MAX22, 6_257_896);
        assert_eq!(
            std_call_number("K1ABC")
                .map(|n| std_call_text(n).unwrap())
                .unwrap(),
            "K1ABC"
        );
        assert_eq!(std_call_number("3DA0XYZ"), None);
        assert_eq!(std_call_number("3XA0XY"), None);
        assert_eq!(std_call_number("PJ4/K1ABC"), None);
        assert_eq!(std_call_number("K1AB1"), None);
        assert_eq!(std_call_number("11ABC"), None);
    }

    #[test]
    fn unpack_rejects_standard_call_values_pack_never_makes() {
        // "658UBE": an all-digit prefix. Before this check it unpacked.
        let n = ((7 * 36 + 5) * 10 + 8) * 27 * 27 * 27 + (21 * 27 + 2) * 27 + 5;
        assert_eq!(std_call_text(n), Err(Error::InvalidPayload));
        let mut ok = 0;
        for n in (0..STD_CALLS).step_by(9_973) {
            if let Ok(c) = std_call_text(n) {
                assert_eq!(std_call_number(&c), Some(n));
                ok += 1;
            }
        }
        assert!(ok > 1_000);
    }

    #[test]
    fn round_trips() {
        for m in [
            "CQ K1ABC FN42",
            "CQ DX K1ABC FN42",
            "CQ 123 K1ABC",
            "QRZ K1ABC",
            "DE K1ABC FN42",
            "K1ABC W9XYZ",
            "K1ABC W9XYZ R EN37",
            "K1ABC W9XYZ -30",
            "K1ABC W9XYZ R+50",
            "K1ABC W9XYZ -50",
            "<K1ABC> <W9XYZ> -10",
            "K1ABC/P <W9XYZ>",
            "<PJ4/K1ABC> PA9/W9XYZ RR73",
            "K1ABC W9XYZ R RR73",
            "ABC",
            "K1ABC/R W9XYZ/R EN37",
            "G4ABC/P PA9XYZ R JO22",
            "<PJ4/K1ABC> W9XYZ",
            "W9XYZ <PJ4/K1ABC> -11",
            "CQ PJ4/K1ABC",
            "<W9XYZ> PJ4/K1ABC RRR",
            "PJ4/K1ABC <W9XYZ> 73",
            "TNX BOB 73 GL",
            "?+-./",
            "123456789ABCDEF012",
            "K1ABC RR73; W9XYZ <KH1/KH7Z> -08",
            "<K1ABC> RR73; <PJ4/W9XYZ> <KH1/KH7Z> +04",
            "<G4ABC> <PA9XYZ> R 570007 JO22DB",
            "<PA3XYZ/P> <G4ABC> 522047 RR99XX",
            "<K1ABC> <W9XYZ> 590000 AA00AA",
        ] {
            assert_eq!(roundtrip(m), m, "{m}");
        }
    }

    #[test]
    fn normalizes_case_and_spacing() {
        assert_eq!(roundtrip("  cq   k1abc fn42 "), "CQ K1ABC FN42");
    }

    #[test]
    fn unhashed_calls_unpack_as_ellipsis() {
        let p = pack("<PJ4/K1ABC> W9XYZ", &mut NoHashes).unwrap();
        assert_eq!(unpack(&p, &mut NoHashes).unwrap(), "<...> W9XYZ");
    }

    #[test]
    fn a_failed_pack_remembers_nothing() {
        let mut t: CallTable<4> = CallTable::new();
        assert_eq!(
            pack("<PJ4/K1ABC> W9XYZ +60", &mut t),
            Err(Error::InvalidReport)
        );
        assert_eq!(
            pack("<K1ABC> <W9XYZ> 510001 FN42AB", &mut t),
            Err(Error::InvalidReport)
        );
        assert!(t.is_empty());
        pack("<PJ4/K1ABC> W9XYZ", &mut t).unwrap();
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn unpack_remembers_only_after_decoding_every_field() {
        // Second c28 is an undefined token value: nothing may be remembered.
        let k1abc = NTOKENS + MAX22 + std_call_number("K1ABC").unwrap();
        let bad = 532_444;
        let bits = Bits::new()
            .put(u128::from(k1abc), 28)
            .put(0, 1)
            .put(bad, 28)
            .put(0, 1)
            .put(u128::from(MAXGRID4 + 1), 16)
            .put(1, 3)
            .finish();
        let mut t: CallTable<4> = CallTable::new();
        assert_eq!(unpack(&bits, &mut t), Err(Error::InvalidPayload));
        assert!(t.is_empty());
    }

    #[test]
    fn a_call_in_the_message_does_not_shadow_its_hash() {
        // Find a standard call whose 10-bit hash equals the fox's.
        let fox = "VP2E/N8O";
        let h10 = hash(fox, HashSize::Bits10).unwrap();
        let mut found = None;
        'search: for a in b'A'..=b'Z' {
            for b in b'A'..=b'Z' {
                for c in b'A'..=b'Z' {
                    let call = [b'K', b'1', a, b, c];
                    let call = core::str::from_utf8(&call).unwrap();
                    if hash(call, HashSize::Bits10) == Some(h10) {
                        found = Some(Call::from_ascii(call).unwrap());
                        break 'search;
                    }
                }
            }
        }
        let other = found.unwrap();
        let mut t: CallTable<8> = CallTable::new();
        let msg = std::format!("{} RR73; W9XYZ <{fox}> +22", other.as_str());
        let p = pack(&msg, &mut t).unwrap();
        assert_eq!(unpack(&p, &mut t).unwrap().as_str(), msg);
    }

    #[test]
    fn unpack_remembers_the_sender_as_wsjtx_does() {
        // Checked black-box with jt9: only the sending call, as written.
        let heard = |text: &str| {
            let p = pack(text, &mut NoHashes).unwrap();
            let mut t: CallTable<8> = CallTable::new();
            unpack(&p, &mut t).unwrap();
            let mut calls = std::vec::Vec::new();
            for c in ["N7QQ", "N7QQ/P", "G4XYZ", "PJ4/N7QQ"] {
                if t.lookup(HashSize::Bits22, hash22(c).unwrap()).is_some() {
                    calls.push(c);
                }
            }
            calls
        };
        assert_eq!(heard("G4XYZ N7QQ EN37"), ["N7QQ"]);
        assert_eq!(heard("N7QQ G4XYZ EN37"), ["G4XYZ"]);
        assert_eq!(heard("G4XYZ N7QQ/P R-10"), ["N7QQ/P"]);
        assert_eq!(heard("CQ PJ4/N7QQ"), ["PJ4/N7QQ"]);
        assert_eq!(heard("<G4XYZ> PJ4/N7QQ RRR"), ["PJ4/N7QQ"]);
        assert!(heard("PJ4/N7QQ <G4XYZ> RRR").is_empty());
        assert!(heard("N7QQ RR73; G4XYZ <KH1/KH7Z> -12").is_empty());
    }

    #[test]
    fn errors() {
        assert_eq!(pack("", &mut NoHashes), Err(Error::Empty));
        assert_eq!(
            pack("K1ABC W9XYZ -51", &mut NoHashes),
            Err(Error::InvalidReport)
        );
        assert_eq!(
            pack("K1ABC W9XYZ +51", &mut NoHashes),
            Err(Error::InvalidReport)
        );
        // Type 4 cannot carry a report; WSJT-X drops it, this crate refuses.
        assert_eq!(
            pack("K1ABC/P <W9XYZ> -10", &mut NoHashes),
            Err(Error::NotEncodable)
        );
        assert_eq!(
            pack("CQ <K1ABC> FN42", &mut NoHashes),
            Err(Error::NotEncodable)
        );
        assert_eq!(
            pack("K1ABC/P W9XYZ/R", &mut NoHashes),
            Err(Error::InvalidCallsign)
        );
        assert_eq!(
            pack("THIS IS TOO LONG FOR TEXT", &mut NoHashes),
            Err(Error::NotEncodable)
        );
        assert_eq!(pack("HELLO_", &mut NoHashes), Err(Error::NotEncodable));
        assert_eq!(
            pack("K1ABC W9XYZ 6A WI", &mut NoHashes),
            Err(Error::Unsupported(MessageType::FieldDay { n3: 3 }))
        );
        assert_eq!(
            pack("WA9XYZ KA1ABC R 17B EMA", &mut NoHashes),
            Err(Error::Unsupported(MessageType::FieldDay { n3: 4 }))
        );
        assert_eq!(
            pack("TU; K1ABC W9XYZ 579 WI", &mut NoHashes),
            Err(Error::Unsupported(MessageType::RttyRoundup))
        );
        // Type 5 needs both calls in brackets and a serial up to 2047.
        assert_eq!(
            pack("K1ABC W9XYZ R 570007 JO22DB", &mut NoHashes),
            Err(Error::InvalidCallsign)
        );
        assert_eq!(
            pack("<K1ABC> <W9XYZ> 572048 JO22DB", &mut NoHashes),
            Err(Error::InvalidReport)
        );
        assert_eq!(
            pack("K1ABC/R RR73; W9XYZ <KH1/KH7Z> -08", &mut NoHashes),
            Err(Error::InvalidCallsign)
        );
    }

    #[test]
    fn dxpedition_reports_round_down_to_even() {
        assert_eq!(
            roundtrip("K1ABC RR73; W9XYZ <KH1/KH7Z> -11"),
            "K1ABC RR73; W9XYZ <KH1/KH7Z> -12"
        );
        assert_eq!(
            roundtrip("K1ABC RR73; W9XYZ <KH1/KH7Z> +32"),
            "K1ABC RR73; W9XYZ <KH1/KH7Z> +32"
        );
    }

    #[test]
    fn unsupported_and_reserved_payloads() {
        let p = Payload::from_bits(3 << 3).unwrap();
        assert_eq!(
            unpack(&p, &mut NoHashes),
            Err(Error::Unsupported(MessageType::FieldDay { n3: 3 }))
        );
        let p = Payload::from_bits(3).unwrap();
        assert_eq!(
            unpack(&p, &mut NoHashes),
            Err(Error::Unsupported(MessageType::RttyRoundup))
        );
        for bits in [2 << 3, 6 << 3, 7 << 3, 6, 7] {
            let p = Payload::from_bits(bits).unwrap();
            assert_eq!(unpack(&p, &mut NoHashes), Err(Error::InvalidPayload));
        }
    }

    #[test]
    fn bytes_are_msb_first() {
        let p = Payload::from_bits(1u128 << 76).unwrap();
        assert_eq!(p.to_bytes()[0], 0x80);
        assert_eq!(p.bit(0), 1);
        assert_eq!(Payload::from_bits(1u128 << 77), None);
    }
}
