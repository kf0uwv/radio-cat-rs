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

//! Icom CI-V: a binary, addressed protocol on a shared bus.
//!
//! Everything here is from the IC-7100 Full Manual, section 20 (CONTROL
//! COMMAND), which is in `ic7100/docs/manuals/`. Where a comment states a
//! fact about the wire it is because the manual states it, not because it
//! is conventional.
//!
//! # How it differs from the ASCII protocols
//!
//! Kenwood and Yaesu speak lines of text terminated by `;`, with no
//! addressing: whatever is on the other end of the cable is the radio.
//! CI-V is none of those things.
//!
//! ```text
//! FE FE 88 E0  Cn [Sc] [data...] FD
//! ^^^^^ ^^ ^^  ^^  ^^   ^^^^^^^  ^^
//! |     |  |   |   |    |        end of message
//! |     |  |   |   |    BCD, little-endian pairs
//! |     |  |   sub-command, on some commands and not others
//! |     |  |   command
//! |     |  controller address (E0 by convention)
//! |     radio address (88 for an IC-7100 from the factory)
//! preamble, twice
//! ```
//!
//! Three consequences the ASCII shape never had to deal with:
//!
//! - **A frame is addressed.** Up to four radios share the bus, so a frame
//!   carries who it is for and who it is from, and a controller must
//!   ignore what is not addressed to it.
//! - **The radio echoes.** On a single-wire bus the controller's own frame
//!   comes back before the answer does. Discarding an echo is *not*
//!   optional cleanup — a client that treated it as the reply would parse
//!   its own request as the radio's state.
//! - **A command's identity is one or two bytes.** `Cn` alone for some,
//!   `Cn Sc` for others, and which is which is per command. There is no
//!   syntactic rule that separates them, so the table decides.
//!
//! # Why `Code` is `(u8, Option<u8>)`
//!
//! Because the manual's table is. Command `03` is "read the operating
//! frequency" with no sub-command; command `15 02` is "read the S-meter
//! level" and `15` alone means nothing. Making the sub-command optional in
//! the type is what lets a lookup tell "command 15, no sub-command" from
//! "command 15, sub-command 02" instead of guessing from the length of
//! what followed.

use crate::wire_format::{CatWireFormat, FrameScanner};
use crate::{CommandDefinition, CommandId, CommandTable, ParseError};

/// The byte that opens a frame, twice.
pub const PREAMBLE: u8 = 0xFE;

/// The byte that ends one.
pub const END_OF_MESSAGE: u8 = 0xFD;

/// The radio's reply to a command it accepted.
///
/// A bare acknowledgement with no data — `FE FE E0 88 FB FD`.
pub const OK: u8 = 0xFB;

/// The radio's reply to a command it refused.
pub const NG: u8 = 0xFA;

/// What a controller calls itself, by convention throughout Icom's
/// documentation and every third-party tool.
pub const DEFAULT_CONTROLLER_ADDRESS: u8 = 0xE0;

/// A command's identity: a command byte, and a sub-command byte on the
/// commands that have one.
pub type CivCode = (u8, Option<u8>);

/// CI-V, configured for one radio on the bus.
///
/// Carries addresses, unlike `AsciiLineFormat`, because this protocol
/// genuinely has them: two IC-7100s on one bus differ only by address, and
/// a format that could not hold one could not tell them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CivFormat {
    /// The radio's address. `0x88` for an IC-7100 as it leaves the
    /// factory, and changeable from its set mode — which is the whole
    /// point of a bus.
    pub radio: u8,
    /// This controller's address.
    pub controller: u8,
}

impl CivFormat {
    pub const fn new(radio: u8, controller: u8) -> Self {
        Self { radio, controller }
    }

    /// A format for a radio at `radio`, with the conventional controller
    /// address.
    pub const fn for_radio(radio: u8) -> Self {
        Self::new(radio, DEFAULT_CONTROLLER_ADDRESS)
    }

    /// Whether a frame is addressed to this controller by this radio.
    ///
    /// Both halves matter. A frame from another radio on the bus is not
    /// ours; a frame *to* another controller is not ours either, and on a
    /// bus with two PCs on it both cases happen.
    pub fn is_for_us(&self, frame: &[u8]) -> bool {
        matches!(parse_envelope(frame), Some(env)
            if env.to == self.controller && env.from == self.radio)
    }

    /// Whether a frame is this controller's own request, echoed back.
    ///
    /// On a single-wire bus every byte a controller sends comes back to
    /// it. A client that took the echo for the answer would read its own
    /// request as the radio's state — which is the failure this exists to
    /// make impossible rather than unlikely.
    pub fn is_echo(&self, frame: &[u8]) -> bool {
        matches!(parse_envelope(frame), Some(env)
            if env.to == self.radio && env.from == self.controller)
    }
}

impl CivFormat {
    /// Frame a reply from the radio to the controller.
    ///
    /// The addresses swap: a request is `FE FE <radio> <controller> …` and
    /// its answer is `FE FE <controller> <radio> …`. Building a reply with
    /// [`CatWireFormat::encode_request`] would send it to the radio's own
    /// address, which on a bus means the frame is addressed to whoever the
    /// radio is — and the controller, seeing a frame not for it, would
    /// ignore its own answer.
    pub fn encode_response(&self, code: CivCode, params: &[u8]) -> Vec<u8> {
        let (command, sub) = code;
        let mut out = Vec::with_capacity(7 + params.len());
        out.push(PREAMBLE);
        out.push(PREAMBLE);
        out.push(self.controller);
        out.push(self.radio);
        out.push(command);
        if let Some(sub) = sub {
            out.push(sub);
        }
        out.extend_from_slice(params);
        out.push(END_OF_MESSAGE);
        out
    }

    /// The bare acknowledgement a radio sends for a command it accepted.
    pub fn encode_ok(&self) -> Vec<u8> {
        vec![
            PREAMBLE,
            PREAMBLE,
            self.controller,
            self.radio,
            OK,
            END_OF_MESSAGE,
        ]
    }

    /// The refusal a radio sends for one it did not.
    pub fn encode_ng(&self) -> Vec<u8> {
        vec![
            PREAMBLE,
            PREAMBLE,
            self.controller,
            self.radio,
            NG,
            END_OF_MESSAGE,
        ]
    }
}

impl Default for CivFormat {
    /// An IC-7100 at its factory address.
    fn default() -> Self {
        Self::for_radio(0x88)
    }
}

/// A frame's addressing and payload, once the wrapper is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Envelope<'a> {
    pub to: u8,
    pub from: u8,
    pub payload: &'a [u8],
}

/// Split `FE FE to from ... FD` into its parts.
///
/// `None` for anything that is not a whole, well-formed frame. Deliberately
/// total rather than panicking: bytes arriving on a shared bus include
/// other people's traffic and the tail of whatever was mid-flight when a
/// controller opened the port.
pub fn parse_envelope(frame: &[u8]) -> Option<Envelope<'_>> {
    // Preamble, two addresses, at least one payload byte, terminator.
    if frame.len() < 6 {
        return None;
    }
    if frame[0] != PREAMBLE || frame[1] != PREAMBLE {
        return None;
    }
    if *frame.last()? != END_OF_MESSAGE {
        return None;
    }
    let payload = &frame[4..frame.len() - 1];
    if payload.is_empty() {
        return None;
    }
    Some(Envelope {
        to: frame[2],
        from: frame[3],
        payload,
    })
}

impl CatWireFormat for CivFormat {
    type Code = CivCode;

    fn find_command<'a, C: CommandId>(
        &self,
        table: &'static CommandTable<C, Self>,
        frame: &'a [u8],
    ) -> Result<(&'static CommandDefinition<C, Self>, &'a [u8]), ParseError> {
        let envelope = parse_envelope(frame).ok_or(ParseError::InvalidSyntax)?;
        let payload = envelope.payload;
        let command = payload[0];
        let rest = &payload[1..];

        // Sub-commanded first. `15` alone means nothing on an IC-7100 and
        // `15 02` is the S-meter; matching the bare command first would
        // shadow every sub-command it has.
        if let Some(&sub) = rest.first() {
            if let Some(d) = table
                .definitions()
                .iter()
                .find(|d| d.code == (command, Some(sub)))
            {
                return Ok((d, &rest[1..]));
            }
        }

        table
            .definitions()
            .iter()
            .find(|d| d.code == (command, None))
            .map(|d| (d, rest))
            .ok_or_else(|| {
                ParseError::UnknownCommand(match rest.first() {
                    Some(sub) => format!("{command:02X} {sub:02X}"),
                    None => format!("{command:02X}"),
                })
            })
    }

    fn encode_request(&self, code: Self::Code, params: &[u8]) -> Vec<u8> {
        let (command, sub) = code;
        let mut out = Vec::with_capacity(7 + params.len());
        out.push(PREAMBLE);
        out.push(PREAMBLE);
        out.push(self.radio);
        out.push(self.controller);
        out.push(command);
        if let Some(sub) = sub {
            out.push(sub);
        }
        out.extend_from_slice(params);
        out.push(END_OF_MESSAGE);
        out
    }
}

impl FrameScanner for CivFormat {
    fn frame_complete(&self, buffer: &[u8]) -> bool {
        // `FD` never appears inside a frame: every data byte is BCD, so no
        // nibble reaches 0xD in a position that could be mistaken for it,
        // and the addresses are constrained. The manual relies on this and
        // so does every other CI-V implementation.
        buffer.last() == Some(&END_OF_MESSAGE)
    }
}

// ---------------------------------------------------------------------------
// BCD, the way this protocol carries numbers
// ---------------------------------------------------------------------------

/// Encode `value` as `bytes` of little-endian packed BCD.
///
/// Little-endian *by pairs*: the least significant two digits come first.
/// A frequency of 14.074 MHz goes out as `00 00 74 14 00`, which reads
/// backwards to anyone expecting a number and is what the manual's digit
/// diagram at 20-11 specifies.
///
/// A value too large for `bytes` is truncated at the top rather than
/// wrapping, because a truncated frequency is out of band and will be
/// refused, where a wrapped one is a different valid frequency.
pub fn encode_bcd(value: u64, bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes);
    let mut v = value;
    for _ in 0..bytes {
        let low = (v % 10) as u8;
        v /= 10;
        let high = (v % 10) as u8;
        v /= 10;
        out.push((high << 4) | low);
    }
    out
}

/// Encode `value` as `bytes` of **big-endian** packed BCD.
///
/// The order everything except the frequency uses. A level written
/// `0255` in the manual goes out as `02 55`, read left to right like the
/// number it is printed as.
///
/// # The asymmetry, and why it is real
///
/// Frequency is little-endian and everything else is big-endian, which
/// reads like a mistake and is not. The manual gives frequency its own
/// digit diagram at 20-11, annotating each byte — "1 Hz digit", "10 Hz
/// digit" — precisely because it reverses the order the rest of the
/// document uses. Every other multi-byte value is printed as a plain
/// decimal (`0000 to 0255`, `0001–0099`) and goes on the wire in the
/// order it is printed.
///
/// This trap is worth the two functions: a level sent little-endian is
/// accepted by the radio and sets something else entirely, and 96 becomes
/// 150 without anything reporting an error.
pub fn encode_bcd_be(value: u64, bytes: usize) -> Vec<u8> {
    let mut out = encode_bcd(value, bytes);
    out.reverse();
    out
}

/// Decode big-endian packed BCD.
pub fn decode_bcd_be(bytes: &[u8]) -> Option<u64> {
    let mut value: u64 = 0;
    for byte in bytes {
        let high = byte >> 4;
        let low = byte & 0x0F;
        if high > 9 || low > 9 {
            return None;
        }
        value = value * 100 + u64::from(high) * 10 + u64::from(low);
    }
    Some(value)
}

/// Decode little-endian packed BCD.
///
/// `None` on a nibble above 9. That is not pedantry: a misframed read
/// lands here as `0x0A`-and-up, and returning a plausible number from it
/// would put a frequency on screen that the radio never reported.
pub fn decode_bcd(bytes: &[u8]) -> Option<u64> {
    let mut value: u64 = 0;
    for byte in bytes.iter().rev() {
        let high = byte >> 4;
        let low = byte & 0x0F;
        if high > 9 || low > 9 {
            return None;
        }
        value = value * 100 + u64::from(high) * 10 + u64::from(low);
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RADIO: u8 = 0x88;
    const ME: u8 = 0xE0;

    fn civ() -> CivFormat {
        CivFormat::new(RADIO, ME)
    }

    #[test]
    fn a_request_is_framed_the_way_the_manual_draws_it() {
        // Manual 20-2: FE FE <radio> <controller> Cn Sc ... FD.
        let out = civ().encode_request((0x03, None), &[]);
        assert_eq!(out, vec![0xFE, 0xFE, 0x88, 0xE0, 0x03, 0xFD]);
    }

    #[test]
    fn a_sub_command_goes_where_the_manual_puts_it() {
        // 15 02 is "read the S-meter level". The sub-command is part of
        // the command's identity, not the first byte of its data.
        let out = civ().encode_request((0x15, Some(0x02)), &[]);
        assert_eq!(out, vec![0xFE, 0xFE, 0x88, 0xE0, 0x15, 0x02, 0xFD]);
    }

    #[test]
    fn frequencies_are_little_endian_bcd_pairs() {
        // 20-11's digit diagram, worked through for 14.074.000 Hz. Each
        // byte holds two digits, the higher one in the high nibble, and
        // the bytes run from the 1 Hz end:
        //
        //   byte 0  (10 Hz, 1 Hz)      = 0, 0  -> 0x00
        //   byte 1  (1 kHz, 100 Hz)    = 4, 0  -> 0x40
        //   byte 2  (100 kHz, 10 kHz)  = 0, 7  -> 0x07
        //   byte 3  (10 MHz, 1 MHz)    = 1, 4  -> 0x14
        //   byte 4  (1 GHz, 100 MHz)   = 0, 0  -> 0x00
        //
        // Written out because getting it wrong produces a frequency that
        // is plausible and off by a factor of a hundred, and because the
        // first version of this test asserted the wrong bytes.
        assert_eq!(
            encode_bcd(14_074_000, 5),
            vec![0x00, 0x40, 0x07, 0x14, 0x00]
        );

        // 1.800.000 Hz, the bottom of 160 m, as a second worked case.
        assert_eq!(encode_bcd(1_800_000, 5), vec![0x00, 0x00, 0x80, 0x01, 0x00]);
    }

    #[test]
    fn levels_and_memories_are_big_endian_and_frequency_is_not() {
        // The asymmetry, pinned. A level sent in the frequency's order is
        // accepted by the radio and sets something else -- 96 arrives as
        // 150 -- and nothing anywhere reports an error.
        assert_eq!(encode_bcd_be(255, 2), vec![0x02, 0x55]);
        assert_eq!(encode_bcd_be(96, 2), vec![0x00, 0x96]);
        assert_eq!(encode_bcd_be(1, 2), vec![0x00, 0x01]);

        // The same numbers the other way round, which is what frequency
        // uses and what these must not be confused with.
        assert_eq!(encode_bcd(255, 2), vec![0x55, 0x02]);
        assert_eq!(encode_bcd(96, 2), vec![0x96, 0x00]);
    }

    #[test]
    fn big_endian_bcd_round_trips() {
        for value in [0u64, 1, 9, 96, 99, 100, 255, 9999] {
            let bytes = encode_bcd_be(value, 2);
            assert_eq!(decode_bcd_be(&bytes), Some(value), "{value}");
        }
    }

    #[test]
    fn a_bad_nibble_is_refused_in_either_order() {
        assert_eq!(decode_bcd_be(&[0x0A, 0x00]), None);
        assert_eq!(decode_bcd_be(&[0x00, 0xFF]), None);
    }

    #[test]
    fn bcd_round_trips() {
        for value in [0u64, 1, 9, 10, 99, 100, 1_800_000, 14_074_000, 450_000_000] {
            let bytes = encode_bcd(value, 5);
            assert_eq!(decode_bcd(&bytes), Some(value), "{value}");
        }
    }

    #[test]
    fn a_nibble_above_nine_is_refused_rather_than_guessed() {
        // A misframed read arrives here as 0x0A and up. Returning a
        // plausible number from it would put a frequency on screen the
        // radio never reported.
        assert_eq!(decode_bcd(&[0xAB]), None);
        assert_eq!(decode_bcd(&[0x00, 0x1F]), None);
        assert_eq!(decode_bcd(&[0x99]), Some(99));
    }

    #[test]
    fn a_frame_from_the_radio_is_recognised_and_our_own_echo_is_not() {
        // The single-wire bus. A client that took its own echo for the
        // answer would read its request back as the radio's state, and
        // every frequency it displayed would be the one it just asked for.
        let f = civ();
        let echo = f.encode_request((0x03, None), &[]);
        assert!(f.is_echo(&echo));
        assert!(!f.is_for_us(&echo));

        let reply = vec![
            0xFE, 0xFE, ME, RADIO, 0x03, 0x00, 0x00, 0x07, 0x40, 0x14, 0xFD,
        ];
        assert!(f.is_for_us(&reply));
        assert!(!f.is_echo(&reply));
    }

    #[test]
    fn another_radio_on_the_bus_is_not_ours() {
        // Up to four radios share the wire. Answering to a different one's
        // address is how two consoles end up showing each other's dial.
        let f = civ();
        let other = vec![0xFE, 0xFE, ME, 0x94, 0x03, 0x00, 0xFD];
        assert!(!f.is_for_us(&other));
    }

    #[test]
    fn a_frame_for_another_controller_is_not_ours_either() {
        let f = civ();
        let theirs = vec![0xFE, 0xFE, 0xE1, RADIO, 0x03, 0x00, 0xFD];
        assert!(!f.is_for_us(&theirs));
    }

    #[test]
    fn malformed_frames_are_refused_rather_than_read() {
        for bad in [
            &[][..],
            &[0xFE, 0xFE, RADIO, ME, 0xFD][..], // no payload
            &[0xFE, RADIO, ME, 0x03, 0xFD][..], // one preamble byte
            &[0xFE, 0xFE, RADIO, ME, 0x03][..], // no terminator
        ] {
            assert!(parse_envelope(bad).is_none(), "{bad:02X?}");
        }
    }

    #[test]
    fn a_frame_is_complete_at_the_end_of_message_byte() {
        let f = civ();
        assert!(!f.frame_complete(&[0xFE, 0xFE, ME, RADIO, 0x03]));
        assert!(f.frame_complete(&[0xFE, 0xFE, ME, RADIO, 0x03, 0xFD]));
    }

    #[test]
    fn a_reply_is_addressed_back_to_the_controller() {
        // The addresses swap. Framing a reply as a request would address
        // it to the radio itself, and the controller -- correctly ignoring
        // a frame that is not for it -- would drop its own answer.
        let f = civ();
        let reply = f.encode_response((0x03, None), &[0x00, 0x40, 0x07, 0x14, 0x00]);
        assert_eq!(&reply[..4], &[0xFE, 0xFE, ME, RADIO]);
        assert!(f.is_for_us(&reply));
        assert!(!f.is_echo(&reply));
    }

    #[test]
    fn ok_and_ng_are_the_manuals_bare_frames() {
        // 20-2: FE FE E0 88 FB FD and ... FA FD.
        let f = civ();
        assert_eq!(f.encode_ok(), vec![0xFE, 0xFE, ME, RADIO, 0xFB, 0xFD]);
        assert_eq!(f.encode_ng(), vec![0xFE, 0xFE, ME, RADIO, 0xFA, 0xFD]);
    }

    #[test]
    fn the_default_address_is_the_one_an_ic7100_ships_with() {
        // 0x88, per the manual's frame diagram at 20-2. A wrong default
        // means a radio that answers nothing out of the box.
        assert_eq!(CivFormat::default().radio, 0x88);
        assert_eq!(CivFormat::default().controller, 0xE0);
    }
}
