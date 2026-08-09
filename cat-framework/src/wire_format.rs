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

//! [`CatWireFormat`]: the seam that lets [`crate::cat`]'s engine adapt to a
//! given CAT protocol's own wire shape, plus [`AsciiLineFormat`], the
//! Kenwood/Yaesu-style implementation this workspace has always used.
//!
//! See `docs/adr/0009-civ-engine-for-binary-addressed-protocols.md` for the
//! full design record — in particular why this is one generalized engine
//! (not a parallel framework per protocol family), and why format
//! implementations own `&self` state (a CI-V radio's own bus address is
//! runtime configuration, not a compile-time fact about "the CI-V protocol
//! class").
//!
//! # Why `find_command` does both parsing and lookup in one method
//!
//! `CommandDefinition::code`'s type is `F::Code`, and for
//! [`AsciiLineFormat`] that's `&'static str` — required so existing
//! command tables can keep writing `code: "FA"` as a plain literal (a
//! `&'static str` already, at zero cost). But parsing a code out of a
//! wire frame yields something borrowed *from that frame*
//! (`frame: &'a [u8]`), which for an arbitrary `'a` cannot honestly become
//! a `&'static str` without allocating.
//!
//! Rather than have [`CatWireFormat`] hand back a bare `Self::Code` of a
//! lifetime that doesn't fit its own associated type, [`find_command`]
//! folds "split the code off the frame" and "look it up in the table"
//! into one method — the comparison between a frame-borrowed code and the
//! table's `'static` entries happens *inside* the implementation, where
//! `&'static str == &'a str` compares just fine (content equality is
//! independent of the two sides' lifetimes; see `core::cmp::PartialEq`'s
//! blanket impl for `&str`). No intermediate value of a mismatched
//! lifetime ever needs to be named.
//!
//! [`find_command`]: CatWireFormat::find_command

use crate::cat::{CommandDefinition, CommandId, CommandTable, ParseError};

/// A concrete CAT wire protocol: how a command is identified on the wire,
/// and how requests/responses are encoded. `cat-framework`'s engine
/// (`CommandTable`, `CatRadio`, `CatFramework`, ...) is generic over this
/// trait — `AsciiLineFormat` (Kenwood/Yaesu) is its first implementation;
/// a CI-V implementation for Icom radios is expected to follow once a
/// consuming radio needs it (see the ADR referenced above).
///
/// Takes `&self` rather than being a set of associated functions on a
/// zero-sized marker: a real protocol can carry configuration (CI-V's bus
/// and controller addresses, for instance) that a stateless marker type
/// cannot represent. `AsciiLineFormat` simply ignores `self` in every
/// method, at zero runtime cost.
pub trait CatWireFormat: 'static + Sized {
    /// How a command is identified on the wire and stored in a
    /// `CommandDefinition`. `&'static str` for ASCII-line protocols;
    /// expected to be `(u8, Option<u8>)` for CI-V's cmd/subcmd pair.
    type Code: Copy + Eq + core::fmt::Debug + 'static;

    /// Split one complete, already-delimited wire frame into the matching
    /// static command definition and the frame's raw parameter/data
    /// bytes. Framing (finding where a frame starts/ends within a
    /// growing byte stream) is a separate, session-layer concern — see
    /// [`FrameScanner`] — this runs once on a frame already known to be
    /// complete. See this module's doc comment for why parsing and
    /// lookup are one method rather than two.
    fn find_command<'a, C: CommandId>(
        &self,
        table: &'static CommandTable<C, Self>,
        frame: &'a [u8],
    ) -> Result<(&'static CommandDefinition<C, Self>, &'a [u8]), ParseError>;

    /// Format one outgoing request (code + already-encoded parameter
    /// bytes) as wire bytes.
    fn encode_request(&self, code: Self::Code, params: &[u8]) -> Vec<u8>;
}

/// Detects a complete frame boundary within a growing byte buffer — needed
/// by session types that read a stream incrementally
/// (`cat-transport-serial::SerialCatSession`). Separate from
/// [`CatWireFormat`]'s methods because framing must run byte-by-byte as
/// bytes arrive, while those run once on an already-complete frame.
pub trait FrameScanner: CatWireFormat {
    /// Return `true` once `buffer` holds one complete frame (and no more
    /// bytes should be read for this response).
    fn frame_complete(&self, buffer: &[u8]) -> bool;
}

/// The ASCII, `;`-terminated, 2-character-code CAT protocol shape shared
/// by every Kenwood (`ts570d`) and Yaesu (`ft991a`) radio this workspace
/// has implemented so far. Zero-sized — carries no configuration, because
/// this protocol shape genuinely has none (no addressing, no bus, no
/// per-radio settings that affect framing).
///
/// This is a pure extraction of the exact framing logic `cat-framework`
/// and `cat-transport-serial::SerialCatSession` always used, moved behind
/// [`CatWireFormat`]/[`FrameScanner`] — not a rewrite. Every existing
/// caller keeps compiling unchanged: this is also the default type
/// parameter (`F = AsciiLineFormat`) on every generic engine type, and the
/// `Default` impl below is what lets `CatClient::new(session, table)`-style
/// single-argument constructors keep working.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AsciiLineFormat;

impl CatWireFormat for AsciiLineFormat {
    type Code = &'static str;

    fn find_command<'a, C: CommandId>(
        &self,
        table: &'static CommandTable<C, Self>,
        frame: &'a [u8],
    ) -> Result<(&'static CommandDefinition<C, Self>, &'a [u8]), ParseError> {
        // Byte-identical to the pre-generalization `CommandTable::parse`:
        // strip the trailing `;`, split the first 2 bytes off as the code.
        let frame = frame
            .strip_suffix(b";")
            .ok_or(ParseError::MissingTerminator)?;
        if frame.len() < 2 {
            return Err(ParseError::InvalidSyntax);
        }
        let (code, parameters) = frame.split_at(2);
        let code = core::str::from_utf8(code).map_err(|_| ParseError::InvalidSyntax)?;

        // `d.code: &'static str` vs `code: &'a str` — compares by content,
        // independent of the two sides' lifetimes (see this module's doc
        // comment). No intermediate `Self::Code` value is ever named here.
        let definition = table
            .definitions()
            .iter()
            .find(|d| d.code == code)
            .ok_or_else(|| ParseError::UnknownCommand(code.to_string()))?;

        Ok((definition, parameters))
    }

    fn encode_request(&self, code: Self::Code, params: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(code.len() + params.len() + 1);
        out.extend_from_slice(code.as_bytes());
        out.extend_from_slice(params);
        out.push(b';');
        out
    }
}

impl FrameScanner for AsciiLineFormat {
    fn frame_complete(&self, buffer: &[u8]) -> bool {
        buffer.last() == Some(&b';')
    }
}
