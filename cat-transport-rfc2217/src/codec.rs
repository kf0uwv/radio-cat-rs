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

//! The RFC 2217 wire, as pure functions: Telnet framing plus the Com Port
//! Control Option. No sockets, no threads, no I/O.
//!
//! Both directions live here on purpose. A client and a server that each
//! wrote their own half of a protocol would agree right up until one of
//! them was edited, and the failure would surface as a radio that quietly
//! stopped keying rather than as a test going red. `ts570d`'s emulator
//! builds its device server on this module and
//! [`crate::port::Rfc2217Port`] builds its client on it, so there is one
//! implementation of the protocol and one place to correct it.
//!
//! # What is implemented
//!
//! RFC 2217 §2's Com Port Control Option, restricted to what a CAT link
//! with a keyed PTT line actually needs: the port settings a client sends
//! once at connect (baud rate, data size, parity, stop size), `SET-CONTROL`
//! for DTR and RTS, and `NOTIFY-MODEMSTATE` for CTS/DSR/RI/DCD coming back.
//! Line-state notification, flow-control suspend/resume and `PURGE-DATA`
//! are decoded and passed to the caller but nothing here acts on them.
//!
//! # The escaping rule, which is the easy thing to get wrong
//!
//! `IAC` (255) is the Telnet escape byte, so a data byte that happens to be
//! 255 is sent as `IAC IAC`. This applies to subnegotiation *parameters*
//! too, not just to the data stream — a baud rate whose big-endian encoding
//! contains a 255 byte would otherwise terminate its own subnegotiation.
//! [`escape`] is therefore used by [`com_port`] as well as by
//! [`encode_data`], and [`Decoder`] un-doubles in both contexts.

/// Interpret As Command — the Telnet escape byte.
pub const IAC: u8 = 255;
/// Subnegotiation end.
pub const SE: u8 = 240;
/// Subnegotiation begin.
pub const SB: u8 = 250;
pub const WILL: u8 = 251;
pub const WONT: u8 = 252;
pub const DO: u8 = 253;
pub const DONT: u8 = 254;

/// Telnet Binary Transmission (RFC 856). Negotiated both ways so 8-bit CAT
/// bytes survive; without it a Telnet peer is entitled to mangle them.
pub const OPT_BINARY: u8 = 0;
/// Suppress Go Ahead (RFC 858). Negotiated for the same reason every
/// Telnet-derived protocol does: it turns off line-at-a-time discipline.
pub const OPT_SUPPRESS_GO_AHEAD: u8 = 3;
/// Com Port Control Option (RFC 2217 §2).
pub const OPT_COM_PORT: u8 = 44;

/// Client-to-server Com Port command codes (RFC 2217 §2).
pub mod client {
    pub const SIGNATURE: u8 = 0;
    pub const SET_BAUDRATE: u8 = 1;
    pub const SET_DATASIZE: u8 = 2;
    pub const SET_PARITY: u8 = 3;
    pub const SET_STOPSIZE: u8 = 4;
    pub const SET_CONTROL: u8 = 5;
    pub const NOTIFY_LINESTATE: u8 = 6;
    pub const NOTIFY_MODEMSTATE: u8 = 7;
    pub const FLOWCONTROL_SUSPEND: u8 = 8;
    pub const FLOWCONTROL_RESUME: u8 = 9;
    pub const SET_LINESTATE_MASK: u8 = 10;
    pub const SET_MODEMSTATE_MASK: u8 = 11;
    pub const PURGE_DATA: u8 = 12;
}

/// Server-to-client command codes are the client codes plus 100
/// (RFC 2217 §2), so one number identifies both the command and which end
/// sent it.
pub const SERVER_OFFSET: u8 = 100;

/// Server-to-client Com Port command codes.
pub mod server {
    use super::SERVER_OFFSET;
    pub const SIGNATURE: u8 = super::client::SIGNATURE + SERVER_OFFSET;
    pub const SET_BAUDRATE: u8 = super::client::SET_BAUDRATE + SERVER_OFFSET;
    pub const SET_DATASIZE: u8 = super::client::SET_DATASIZE + SERVER_OFFSET;
    pub const SET_PARITY: u8 = super::client::SET_PARITY + SERVER_OFFSET;
    pub const SET_STOPSIZE: u8 = super::client::SET_STOPSIZE + SERVER_OFFSET;
    pub const SET_CONTROL: u8 = super::client::SET_CONTROL + SERVER_OFFSET;
    pub const NOTIFY_LINESTATE: u8 = super::client::NOTIFY_LINESTATE + SERVER_OFFSET;
    pub const NOTIFY_MODEMSTATE: u8 = super::client::NOTIFY_MODEMSTATE + SERVER_OFFSET;
    pub const FLOWCONTROL_SUSPEND: u8 = super::client::FLOWCONTROL_SUSPEND + SERVER_OFFSET;
    pub const FLOWCONTROL_RESUME: u8 = super::client::FLOWCONTROL_RESUME + SERVER_OFFSET;
    pub const SET_LINESTATE_MASK: u8 = super::client::SET_LINESTATE_MASK + SERVER_OFFSET;
    pub const SET_MODEMSTATE_MASK: u8 = super::client::SET_MODEMSTATE_MASK + SERVER_OFFSET;
    pub const PURGE_DATA: u8 = super::client::PURGE_DATA + SERVER_OFFSET;
}

/// `SET-CONTROL` parameter values (RFC 2217 §2, "Control Suboption").
///
/// Only the DTR and RTS members are driven by this workspace; the flow
/// control and BREAK values are named so a decoded frame can be described
/// accurately rather than reported as an unknown number.
pub mod control {
    pub const REQUEST_FLOW: u8 = 0;
    pub const FLOW_NONE: u8 = 1;
    pub const FLOW_XON_XOFF: u8 = 2;
    pub const FLOW_HARDWARE: u8 = 3;
    pub const BREAK_REQUEST: u8 = 4;
    pub const BREAK_ON: u8 = 5;
    pub const BREAK_OFF: u8 = 6;
    pub const DTR_REQUEST: u8 = 7;
    pub const DTR_ON: u8 = 8;
    pub const DTR_OFF: u8 = 9;
    pub const RTS_REQUEST: u8 = 10;
    pub const RTS_ON: u8 = 11;
    pub const RTS_OFF: u8 = 12;
}

/// `NOTIFY-MODEMSTATE` bits (RFC 2217 §2, and the 16550 MSR it mirrors).
///
/// The four high bits are the current line levels; the four low bits are
/// "this changed since the last notification" deltas. A reader that wants
/// levels must mask off the deltas — [`modem_cts`] and friends do.
pub mod modem {
    /// Clear To Send is asserted.
    pub const CTS: u8 = 0x10;
    /// Data Set Ready is asserted.
    pub const DSR: u8 = 0x20;
    /// Ring Indicator is asserted.
    pub const RI: u8 = 0x40;
    /// Data Carrier Detect is asserted.
    pub const DCD: u8 = 0x80;

    pub const DELTA_CTS: u8 = 0x01;
    pub const DELTA_DSR: u8 = 0x02;
    pub const TRAILING_EDGE_RI: u8 = 0x04;
    pub const DELTA_DCD: u8 = 0x08;

    /// Every level bit — the mask a client sends as `SET-MODEMSTATE-MASK`
    /// to be told about all four lines.
    pub const ALL_LEVELS: u8 = CTS | DSR | RI | DCD;
}

/// Whether CTS is asserted in a `NOTIFY-MODEMSTATE` byte.
pub fn modem_cts(state: u8) -> bool {
    state & modem::CTS != 0
}

/// Whether DSR is asserted in a `NOTIFY-MODEMSTATE` byte.
pub fn modem_dsr(state: u8) -> bool {
    state & modem::DSR != 0
}

/// Whether DCD is asserted in a `NOTIFY-MODEMSTATE` byte.
pub fn modem_dcd(state: u8) -> bool {
    state & modem::DCD != 0
}

/// Double every `IAC` in `bytes`, appending to `out`.
///
/// The whole of Telnet's transparency rule. Applied to data and to
/// subnegotiation parameters alike — see the module doc.
pub fn escape(bytes: &[u8], out: &mut Vec<u8>) {
    for &b in bytes {
        out.push(b);
        if b == IAC {
            out.push(IAC);
        }
    }
}

/// Encode `data` as a Telnet data stream: the bytes, with `IAC` doubled.
pub fn encode_data(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    escape(data, &mut out);
    out
}

/// One three-byte Telnet negotiation, e.g. `IAC WILL COM-PORT-OPTION`.
pub fn negotiate(verb: u8, option: u8) -> [u8; 3] {
    [IAC, verb, option]
}

/// One Com Port subnegotiation: `IAC SB 44 <command> <params…> IAC SE`,
/// with `IAC` doubled inside `params`.
pub fn com_port(command: u8, params: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(params.len() + 8);
    out.extend_from_slice(&[IAC, SB, OPT_COM_PORT, command]);
    escape(params, &mut out);
    out.extend_from_slice(&[IAC, SE]);
    out
}

/// `SET-CONTROL` with one of the [`control`] values.
pub fn set_control(value: u8) -> Vec<u8> {
    com_port(client::SET_CONTROL, &[value])
}

/// `SET-BAUDRATE`, whose parameter is a big-endian `u32` (RFC 2217 §2).
pub fn set_baud_rate(baud: u32) -> Vec<u8> {
    com_port(client::SET_BAUDRATE, &baud.to_be_bytes())
}

/// The server's answer to a client `NOTIFY-MODEMSTATE` mask, and its
/// unsolicited notification whenever a line changes.
pub fn notify_modem_state(state: u8) -> Vec<u8> {
    com_port(server::NOTIFY_MODEMSTATE, &[state])
}

/// What [`Decoder`] produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Payload bytes, already un-escaped. For a CAT link these are the
    /// radio's own protocol bytes and nothing else.
    Data(Vec<u8>),
    /// A completed Com Port subnegotiation. `command` distinguishes the
    /// direction by itself (see [`SERVER_OFFSET`]).
    ComPort { command: u8, params: Vec<u8> },
    /// A Telnet negotiation for some option: `WILL`/`WONT`/`DO`/`DONT`.
    Negotiate { verb: u8, option: u8 },
    /// A subnegotiation for an option this decoder does not model. Kept as
    /// an event rather than dropped so a peer's unexpected traffic is
    /// visible in a log instead of silently vanishing.
    OtherSubnegotiation { option: u8, body: Vec<u8> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Ordinary data.
    Data,
    /// Saw `IAC`.
    Iac,
    /// Saw `IAC WILL|WONT|DO|DONT`, waiting for the option byte.
    Verb(u8),
    /// Saw `IAC SB`, waiting for the option byte.
    SubOption,
    /// Inside a subnegotiation body.
    SubBody,
    /// Inside a subnegotiation body, having just seen `IAC`.
    SubIac,
}

/// A streaming Telnet/RFC 2217 decoder.
///
/// Byte-at-a-time and restartable: bytes arrive from a socket in whatever
/// sizes the network chose, and a negotiation or a subnegotiation may be
/// split across any number of reads. Feeding the same total byte sequence
/// in different chunkings always produces the same events, which is the
/// property [`Decoder`]'s tests pin.
#[derive(Debug)]
pub struct Decoder {
    state: State,
    /// Option byte of the subnegotiation currently being accumulated.
    sub_option: u8,
    /// Body of the subnegotiation currently being accumulated, un-escaped.
    sub_body: Vec<u8>,
    /// Run of ordinary data bytes not yet emitted.
    data: Vec<u8>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            state: State::Data,
            sub_option: 0,
            sub_body: Vec::new(),
            data: Vec::new(),
        }
    }

    /// Feed `bytes`, appending whatever became complete to `out`.
    ///
    /// Contiguous data bytes are coalesced into one [`Event::Data`] per
    /// call rather than one per byte.
    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<Event>) {
        for &b in bytes {
            self.step(b, out);
        }
        self.flush_data(out);
    }

    /// Emit any accumulated data bytes as a single event.
    fn flush_data(&mut self, out: &mut Vec<Event>) {
        if !self.data.is_empty() {
            out.push(Event::Data(std::mem::take(&mut self.data)));
        }
    }

    fn step(&mut self, b: u8, out: &mut Vec<Event>) {
        match self.state {
            State::Data => {
                if b == IAC {
                    self.state = State::Iac;
                } else {
                    self.data.push(b);
                }
            }
            State::Iac => match b {
                // A doubled IAC is one literal 255 in the data stream.
                IAC => {
                    self.data.push(IAC);
                    self.state = State::Data;
                }
                WILL | WONT | DO | DONT => self.state = State::Verb(b),
                SB => self.state = State::SubOption,
                // Any other two-byte command (NOP, DM, BRK, …) is not
                // meaningful on a CAT link; consume it and carry on rather
                // than desynchronising the stream.
                _ => self.state = State::Data,
            },
            State::Verb(verb) => {
                // Data ordering matters: a negotiation that arrived
                // between two runs of data must be reported between them.
                self.flush_data(out);
                out.push(Event::Negotiate { verb, option: b });
                self.state = State::Data;
            }
            State::SubOption => {
                self.sub_option = b;
                self.sub_body.clear();
                self.state = State::SubBody;
            }
            State::SubBody => {
                if b == IAC {
                    self.state = State::SubIac;
                } else {
                    self.sub_body.push(b);
                }
            }
            State::SubIac => match b {
                IAC => {
                    self.sub_body.push(IAC);
                    self.state = State::SubBody;
                }
                SE => {
                    self.flush_data(out);
                    let body = std::mem::take(&mut self.sub_body);
                    if self.sub_option == OPT_COM_PORT {
                        // An empty body has no command byte and cannot be
                        // acted on; report it as the malformed frame it is
                        // rather than panicking on the split.
                        if let Some((&command, params)) = body.split_first() {
                            out.push(Event::ComPort {
                                command,
                                params: params.to_vec(),
                            });
                        }
                    } else {
                        out.push(Event::OtherSubnegotiation {
                            option: self.sub_option,
                            body,
                        });
                    }
                    self.state = State::Data;
                }
                // `IAC <something>` inside a subnegotiation is malformed.
                // Abandon the subnegotiation rather than accumulate a body
                // that will never be terminated correctly.
                _ => {
                    self.sub_body.clear();
                    self.state = State::Data;
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(bytes: &[u8]) -> Vec<Event> {
        let mut d = Decoder::new();
        let mut out = Vec::new();
        d.push(bytes, &mut out);
        out
    }

    #[test]
    fn plain_data_passes_through_unchanged() {
        assert_eq!(
            decode_all(b"FA00014250000;"),
            vec![Event::Data(b"FA00014250000;".to_vec())]
        );
    }

    #[test]
    fn a_literal_255_survives_the_round_trip() {
        // The transparency rule, end to end: one 255 in, one 255 out.
        let data = [b'F', 255, b'A'];
        let wire = encode_data(&data);
        assert_eq!(
            wire,
            vec![b'F', 255, 255, b'A'],
            "IAC must be doubled on the wire"
        );
        assert_eq!(decode_all(&wire), vec![Event::Data(data.to_vec())]);
    }

    #[test]
    fn negotiation_is_lifted_out_of_the_data_stream() {
        let mut wire = b"AB".to_vec();
        wire.extend_from_slice(&negotiate(WILL, OPT_COM_PORT));
        wire.extend_from_slice(b"CD");

        assert_eq!(
            decode_all(&wire),
            vec![
                Event::Data(b"AB".to_vec()),
                Event::Negotiate {
                    verb: WILL,
                    option: OPT_COM_PORT
                },
                Event::Data(b"CD".to_vec()),
            ],
            "data either side of a negotiation must stay in order around it"
        );
    }

    #[test]
    fn com_port_subnegotiation_round_trips() {
        let wire = set_control(control::DTR_ON);
        assert_eq!(
            decode_all(&wire),
            vec![Event::ComPort {
                command: client::SET_CONTROL,
                params: vec![control::DTR_ON],
            }]
        );
    }

    #[test]
    fn a_255_inside_a_subnegotiation_parameter_does_not_end_it() {
        // 4294967295 is 0xFFFFFFFF: four IAC bytes as parameters. Encoded
        // naively this terminates its own subnegotiation and desynchronises
        // everything after it.
        let wire = set_baud_rate(u32::MAX);
        assert_eq!(
            decode_all(&wire),
            vec![Event::ComPort {
                command: client::SET_BAUDRATE,
                params: vec![255, 255, 255, 255],
            }]
        );
    }

    #[test]
    fn baud_rate_is_big_endian() {
        let wire = set_baud_rate(9600);
        let Event::ComPort { params, .. } = &decode_all(&wire)[0] else {
            panic!("expected a com port event");
        };
        assert_eq!(u32::from_be_bytes(params[..4].try_into().unwrap()), 9600);
    }

    #[test]
    fn chunking_never_changes_the_events() {
        // The property that matters against a real socket: the network
        // chooses the read sizes, not the protocol.
        let mut wire = b"FA".to_vec();
        wire.extend_from_slice(&set_control(control::RTS_OFF));
        wire.extend_from_slice(&negotiate(DO, OPT_BINARY));
        wire.extend_from_slice(&notify_modem_state(modem::CTS | modem::DELTA_CTS));
        wire.extend_from_slice(b";");

        let whole = decode_all(&wire);

        for chunk in 1..=wire.len() {
            let mut d = Decoder::new();
            let mut out = Vec::new();
            for part in wire.chunks(chunk) {
                d.push(part, &mut out);
            }
            // Byte-at-a-time decoding emits one Data event per byte, so
            // compare the concatenated data rather than the event list.
            assert_eq!(
                flatten(&out),
                flatten(&whole),
                "chunk size {chunk} decoded differently"
            );
        }
    }

    /// Collapse adjacent `Data` events so two decodings that differ only in
    /// how data was coalesced compare equal.
    fn flatten(events: &[Event]) -> Vec<Event> {
        let mut out: Vec<Event> = Vec::new();
        for e in events {
            match (out.last_mut(), e) {
                (Some(Event::Data(acc)), Event::Data(more)) => acc.extend_from_slice(more),
                _ => out.push(e.clone()),
            }
        }
        out
    }

    #[test]
    fn modem_state_bits_read_levels_and_ignore_deltas() {
        // A notification whose only set bits are deltas describes no
        // asserted line. Reading it as "CTS is up" is the bug this guards.
        let deltas_only = modem::DELTA_CTS | modem::DELTA_DSR | modem::DELTA_DCD;
        assert!(!modem_cts(deltas_only));
        assert!(!modem_dsr(deltas_only));
        assert!(!modem_dcd(deltas_only));

        let cts_up = modem::CTS | modem::DELTA_CTS;
        assert!(modem_cts(cts_up));
        assert!(!modem_dsr(cts_up));
    }

    #[test]
    fn server_codes_are_client_codes_plus_one_hundred() {
        // RFC 2217 §2. Pinned because the whole direction-disambiguation
        // scheme rests on it.
        assert_eq!(server::SET_CONTROL, client::SET_CONTROL + 100);
        assert_eq!(server::NOTIFY_MODEMSTATE, client::NOTIFY_MODEMSTATE + 100);
        assert_eq!(server::SIGNATURE, client::SIGNATURE + 100);
    }

    #[test]
    fn a_subnegotiation_for_another_option_is_reported_not_swallowed() {
        let mut wire = vec![IAC, SB, 31, 0, 80, 0, 24];
        wire.extend_from_slice(&[IAC, SE]);
        assert_eq!(
            decode_all(&wire),
            vec![Event::OtherSubnegotiation {
                option: 31,
                body: vec![0, 80, 0, 24]
            }]
        );
    }

    #[test]
    fn an_empty_com_port_subnegotiation_is_dropped_rather_than_panicking() {
        // No command byte to split off. Malformed input from a peer must
        // not be able to take the decoder down.
        let wire = [IAC, SB, OPT_COM_PORT, IAC, SE];
        assert_eq!(decode_all(&wire), vec![]);
    }

    #[test]
    fn a_malformed_subnegotiation_does_not_swallow_what_follows() {
        // IAC <not SE> inside a body: abandon it, then keep decoding.
        let mut wire = vec![IAC, SB, OPT_COM_PORT, client::SET_CONTROL, IAC, WILL];
        wire.extend_from_slice(b"FA;");
        assert_eq!(decode_all(&wire), vec![Event::Data(b"FA;".to_vec())]);
    }
}
