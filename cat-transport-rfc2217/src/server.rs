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

//! [`Rfc2217Peer`]: the device-server half of one connection, with no I/O
//! in it.
//!
//! A device server's real work is sockets and threads, which differ between
//! one host program and the next. Its *protocol* work does not, so it lives
//! here: feed [`Rfc2217Peer::push`] whatever bytes arrived and it returns
//! what the client asked for and what to write back. `ts570d`'s emulator
//! wraps this in a `TcpListener` to present a virtual radio COM port; this
//! crate's own tests wrap it in a few lines to exercise
//! [`crate::Rfc2217Port`] against something that answers.
//!
//! The point of it being here rather than in the emulator is that the
//! client and the server then read the same [`crate::codec`]. A device
//! server that hand-rolled its own half would agree with the client only
//! until one of them was edited.

use crate::codec::{self, client, control, modem, server, Decoder, Event};

/// Something the client did that the host program has to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerEvent {
    /// Bytes the client sent to the port — for a radio, CAT commands.
    Data(Vec<u8>),
    /// The client drove DTR. On a station whose DTR keys PTT, `true` here
    /// is key-down.
    Dtr(bool),
    /// The client drove RTS. On a TS-570D this is receive-enable: while it
    /// is `false` the radio withholds CAT responses.
    Rts(bool),
    /// The client asked for a `BREAK`. Reported for completeness; no radio
    /// in this workspace does anything with it.
    Break(bool),
}

/// The server side of one RFC 2217 connection.
///
/// Holds the line states the client has asked for and the notification
/// mask it subscribed with. Owns no socket: every method takes bytes in and
/// hands bytes back.
#[derive(Debug)]
pub struct Rfc2217Peer {
    decoder: Decoder,
    dtr: bool,
    rts: bool,
    /// Which modem lines this client asked to be told about
    /// (`SET-MODEMSTATE-MASK`). Zero until it asks, so an unsolicited
    /// notification is never sent to a client that did not subscribe.
    modem_mask: u8,
    /// The last state actually sent, so [`Rfc2217Peer::modem_state`]
    /// reports only genuine changes.
    last_notified: Option<u8>,
}

impl Default for Rfc2217Peer {
    fn default() -> Self {
        Self::new()
    }
}

impl Rfc2217Peer {
    pub fn new() -> Self {
        Self {
            decoder: Decoder::new(),
            // The two lines start differently, and each for its own reason.
            //
            // DTR starts **low**. On a station that keys PTT from DTR, a
            // port that came up asserting it would key a transmitter at
            // accept time, before any client had said anything.
            //
            // RTS starts **high**, matching what a real UART does when a
            // port is opened and what `SerialConfig`'s `initial_rts: true`
            // already assumes. Starting it low looks symmetrical and is
            // wrong: on a radio that treats RTS as receive-enable, a client
            // that never mentions RTS -- a plain telnet session, or
            // anything pointed at a `ser2net`-style endpoint -- would get
            // silence from a port that is working perfectly.
            dtr: false,
            rts: true,
            modem_mask: 0,
            last_notified: None,
        }
    }

    /// Whether the client currently has DTR asserted.
    pub fn dtr(&self) -> bool {
        self.dtr
    }

    /// Whether the client currently has RTS asserted.
    pub fn rts(&self) -> bool {
        self.rts
    }

    /// Feed bytes from the client.
    ///
    /// Returns `(events, reply)`: what the host program must act on, and
    /// the bytes to write back to the client (empty if there are none).
    pub fn push(&mut self, bytes: &[u8]) -> (Vec<PeerEvent>, Vec<u8>) {
        let mut decoded = Vec::new();
        self.decoder.push(bytes, &mut decoded);

        let mut events = Vec::new();
        let mut reply = Vec::new();

        for event in decoded {
            match event {
                Event::Data(data) => events.push(PeerEvent::Data(data)),
                Event::Negotiate { verb, option } => {
                    self.answer_negotiation(verb, option, &mut reply)
                }
                Event::ComPort { command, params } => {
                    self.handle_com_port(command, &params, &mut events, &mut reply)
                }
                Event::OtherSubnegotiation { .. } => {}
            }
        }

        (events, reply)
    }

    /// Agree to the three options this workspace uses; refuse the rest.
    ///
    /// A server answers `WILL` with `DO` and `DO` with `WILL`. Refusals are
    /// sent rather than ignored: a client waiting on an unanswered
    /// negotiation is a hang, not a fallback.
    fn answer_negotiation(&mut self, verb: u8, option: u8, reply: &mut Vec<u8>) {
        let supported = matches!(
            option,
            codec::OPT_BINARY | codec::OPT_SUPPRESS_GO_AHEAD | codec::OPT_COM_PORT
        );
        let answer = match verb {
            codec::WILL => {
                if supported {
                    codec::DO
                } else {
                    codec::DONT
                }
            }
            codec::DO => {
                if supported {
                    codec::WILL
                } else {
                    codec::WONT
                }
            }
            // WONT/DONT need no answer; answering would loop.
            _ => return,
        };
        reply.extend_from_slice(&codec::negotiate(answer, option));
    }

    fn handle_com_port(
        &mut self,
        command: u8,
        params: &[u8],
        events: &mut Vec<PeerEvent>,
        reply: &mut Vec<u8>,
    ) {
        match command {
            client::SET_CONTROL => {
                let Some(&value) = params.first() else { return };
                match value {
                    control::DTR_ON | control::DTR_OFF => {
                        self.dtr = value == control::DTR_ON;
                        events.push(PeerEvent::Dtr(self.dtr));
                    }
                    control::RTS_ON | control::RTS_OFF => {
                        self.rts = value == control::RTS_ON;
                        events.push(PeerEvent::Rts(self.rts));
                    }
                    control::BREAK_ON | control::BREAK_OFF => {
                        events.push(PeerEvent::Break(value == control::BREAK_ON));
                    }
                    control::DTR_REQUEST => {
                        reply.extend_from_slice(&self.control_echo(if self.dtr {
                            control::DTR_ON
                        } else {
                            control::DTR_OFF
                        }));
                        return;
                    }
                    control::RTS_REQUEST => {
                        reply.extend_from_slice(&self.control_echo(if self.rts {
                            control::RTS_ON
                        } else {
                            control::RTS_OFF
                        }));
                        return;
                    }
                    // Flow control: this workspace runs none, and says so.
                    _ => {
                        reply.extend_from_slice(&self.control_echo(control::FLOW_NONE));
                        return;
                    }
                }
                reply.extend_from_slice(&self.control_echo(value));
            }
            client::SET_MODEMSTATE_MASK => {
                let Some(&mask) = params.first() else { return };
                self.modem_mask = mask;
                reply.extend_from_slice(&codec::com_port(server::SET_MODEMSTATE_MASK, &[mask]));
            }
            // RFC 2217 §2: a server confirms a setting by echoing it back
            // with the server-side command code. The values are accepted as
            // given -- a virtual port has no baud generator to disagree
            // with, and reporting a rate it did not apply would be worse
            // than accepting one it did not need.
            client::SET_BAUDRATE
            | client::SET_DATASIZE
            | client::SET_PARITY
            | client::SET_STOPSIZE => {
                reply.extend_from_slice(&codec::com_port(command + codec::SERVER_OFFSET, params));
            }
            client::SIGNATURE => {
                reply.extend_from_slice(&codec::com_port(server::SIGNATURE, params));
            }
            _ => {}
        }
    }

    fn control_echo(&self, value: u8) -> Vec<u8> {
        codec::com_port(server::SET_CONTROL, &[value])
    }

    /// Bytes announcing `state` to this client, if it subscribed to any of
    /// the lines that changed.
    ///
    /// Returns `None` when the client asked for no notifications, or when
    /// nothing it cares about has changed since the last announcement —
    /// so a caller may hand this the current state as often as it likes.
    ///
    /// The **first** call after a client subscribes always announces,
    /// including when every subscribed line is low. A client has no other
    /// way to learn where the lines stand at the moment it connected, and
    /// "no notification yet" and "everything is low" would otherwise be
    /// indistinguishable to it.
    pub fn modem_state(&mut self, state: u8) -> Option<Vec<u8>> {
        if self.modem_mask == 0 {
            return None;
        }
        let masked = state & self.modem_mask;
        let previous = self.last_notified.map(|s| s & self.modem_mask);
        if previous == Some(masked) {
            return None;
        }

        // Deltas describe what moved since the last notification, which is
        // what a 16550's MSR does and what a client reading edges expects.
        let mut out = masked;
        if let Some(previous) = previous {
            let changed = previous ^ masked;
            if changed & modem::CTS != 0 {
                out |= modem::DELTA_CTS;
            }
            if changed & modem::DSR != 0 {
                out |= modem::DELTA_DSR;
            }
            if changed & modem::DCD != 0 {
                out |= modem::DELTA_DCD;
            }
        }

        self.last_notified = Some(masked);
        Some(codec::notify_modem_state(out))
    }

    /// Encode port bytes for the client — for a radio, CAT responses.
    pub fn encode_data(&self, data: &[u8]) -> Vec<u8> {
        codec::encode_data(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive a peer with one client subnegotiation and return what it did.
    fn push(peer: &mut Rfc2217Peer, bytes: &[u8]) -> (Vec<PeerEvent>, Vec<u8>) {
        peer.push(bytes)
    }

    #[test]
    fn dtr_starts_low_and_rts_starts_high() {
        // Not symmetry: two different hazards.
        //
        // DTR asserted at accept would key a transmitter before any client
        // said anything. RTS *deasserted* at accept would make a radio that
        // uses it as receive-enable go silent for any client that never
        // drives the line -- which is most of them, since a real UART
        // asserts RTS on open and `SerialConfig` defaults `initial_rts` to
        // true. That silence was found by pointing a hand-written client at
        // the emulator and getting no answer to `FA;`.
        let peer = Rfc2217Peer::new();
        assert!(!peer.dtr(), "a port must not come up keying");
        assert!(peer.rts(), "a port must not come up muting the radio");
    }

    #[test]
    fn set_control_moves_the_line_and_is_echoed() {
        let mut peer = Rfc2217Peer::new();
        let (events, reply) = push(&mut peer, &codec::set_control(control::DTR_ON));

        assert_eq!(events, vec![PeerEvent::Dtr(true)]);
        assert!(peer.dtr());
        assert_eq!(
            reply,
            codec::com_port(server::SET_CONTROL, &[control::DTR_ON])
        );
    }

    #[test]
    fn rts_is_tracked_separately_from_dtr() {
        let mut peer = Rfc2217Peer::new();
        push(&mut peer, &codec::set_control(control::DTR_ON));
        push(&mut peer, &codec::set_control(control::RTS_OFF));
        assert!(peer.dtr(), "driving RTS must not disturb DTR");
        assert!(!peer.rts());
    }

    #[test]
    fn data_comes_out_as_data() {
        let mut peer = Rfc2217Peer::new();
        let (events, _) = push(&mut peer, &codec::encode_data(b"FA;"));
        assert_eq!(events, vec![PeerEvent::Data(b"FA;".to_vec())]);
    }

    #[test]
    fn no_notification_before_the_client_subscribes() {
        // Sending one unasked is a protocol error, and would also mean a
        // client that never subscribed still saw traffic it must parse.
        let mut peer = Rfc2217Peer::new();
        assert_eq!(peer.modem_state(modem::CTS), None);
    }

    #[test]
    fn subscribing_then_changing_state_notifies_once_per_change() {
        let mut peer = Rfc2217Peer::new();
        push(
            &mut peer,
            &codec::com_port(client::SET_MODEMSTATE_MASK, &[modem::ALL_LEVELS]),
        );

        let first = peer
            .modem_state(modem::CTS)
            .expect("first state is a change");
        assert_eq!(first, codec::notify_modem_state(modem::CTS));

        assert_eq!(
            peer.modem_state(modem::CTS),
            None,
            "the same state twice is not a change"
        );

        let second = peer
            .modem_state(modem::CTS | modem::DSR)
            .expect("DSR rising is a change");
        assert_eq!(
            second,
            codec::notify_modem_state(modem::CTS | modem::DSR | modem::DELTA_DSR),
            "the delta bit must name the line that moved"
        );
    }

    #[test]
    fn a_client_only_hears_about_lines_it_asked_for() {
        let mut peer = Rfc2217Peer::new();
        push(
            &mut peer,
            &codec::com_port(client::SET_MODEMSTATE_MASK, &[modem::CTS]),
        );
        // The first call after subscribing tells the client where the
        // lines currently stand, which it has no other way to learn.
        peer.modem_state(0)
            .expect("the initial state is announced once");

        assert_eq!(
            peer.modem_state(modem::DSR),
            None,
            "DSR moved, but this client subscribed to CTS only"
        );
        assert!(
            peer.modem_state(modem::DSR | modem::CTS).is_some(),
            "CTS moving must still reach a client subscribed to CTS"
        );
    }

    #[test]
    fn negotiation_is_answered_in_the_mirror_verb() {
        let mut peer = Rfc2217Peer::new();
        let (_, reply) = push(
            &mut peer,
            &codec::negotiate(codec::WILL, codec::OPT_COM_PORT),
        );
        assert_eq!(reply, codec::negotiate(codec::DO, codec::OPT_COM_PORT));

        let (_, reply) = push(&mut peer, &codec::negotiate(codec::DO, codec::OPT_BINARY));
        assert_eq!(reply, codec::negotiate(codec::WILL, codec::OPT_BINARY));
    }

    #[test]
    fn an_unsupported_option_is_refused_rather_than_ignored() {
        // Silence would leave a strict client waiting for an answer that
        // never comes.
        let mut peer = Rfc2217Peer::new();
        let (_, reply) = push(&mut peer, &codec::negotiate(codec::DO, 31));
        assert_eq!(reply, codec::negotiate(codec::WONT, 31));
    }

    #[test]
    fn port_settings_are_confirmed_with_the_server_side_code() {
        let mut peer = Rfc2217Peer::new();
        let (_, reply) = push(&mut peer, &codec::set_baud_rate(9600));
        assert_eq!(
            reply,
            codec::com_port(server::SET_BAUDRATE, &9600u32.to_be_bytes())
        );
    }

    #[test]
    fn a_line_request_reports_the_current_state_without_changing_it() {
        let mut peer = Rfc2217Peer::new();
        push(&mut peer, &codec::set_control(control::DTR_ON));

        let (events, reply) = push(&mut peer, &codec::set_control(control::DTR_REQUEST));
        assert!(events.is_empty(), "a request is not a change");
        assert!(peer.dtr());
        assert_eq!(
            reply,
            codec::com_port(server::SET_CONTROL, &[control::DTR_ON])
        );
    }
}
