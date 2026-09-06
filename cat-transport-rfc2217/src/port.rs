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

//! [`Rfc2217Port`]: a remote serial port, with its modem control lines,
//! as a [`Transport`] + [`ModemControlLines`].
//!
//! # Why this is a `Transport` and not a `CatSession`
//!
//! An RFC 2217 endpoint *is* a serial port; the only thing TCP changes is
//! how the bytes and the line states get there. So this crate supplies the
//! byte level and nothing else, and
//! `cat_transport_serial::SerialCatSession<Rfc2217Port>` supplies the
//! read-until-`;` framing exactly as it does for a local port — including
//! its blanket `impl<T: Transport + ModemControlLines> ModemControlLines
//! for SerialCatSession<T>`, which forwards the lines through the framing
//! layer for free. Writing a second `CatSession` here would mean a second
//! copy of the framing logic, free to drift from the first.
//!
//! # A reader thread, not a request/response worker
//!
//! `cat-transport-tcp`'s Windows backend hands each `execute()` to a worker
//! thread as one write-then-read unit. That shape cannot work here, and the
//! reason is worth stating because it is not obvious: [`ModemControlLines`]
//! is a set of **synchronous** methods, and a serial link is quiet for long
//! stretches. If the worker were blocked in a read waiting for a radio that
//! has nothing to say, a `set_dtr` queued behind it would never reach the
//! socket — the PTT key would be swallowed by an idle receive. Keying is
//! precisely the thing that must not wait.
//!
//! So the socket is split. A reader thread owns the read half for the
//! port's whole life, decoding continuously and never blocking anything
//! else; writes go straight out through a mutex-guarded write half, from
//! whichever side wants them. `write` and `set_dtr` are both small writes
//! into a socket buffer and neither waits on the other.
//!
//! # Line state is a cache, and that is not a compromise
//!
//! `read_cts`/`read_dsr`/`read_dcd` answer from the last
//! `NOTIFY-MODEMSTATE` the reader thread saw. That is RFC 2217's own model
//! — the option is notification-driven, and there is no "read the lines
//! now" round trip in the protocol to make instead. Because the reader
//! thread is always reading, the cache tracks the peer continuously rather
//! than going stale between CAT commands, which is what makes
//! `ts570d-line status` (a tool that sends no CAT traffic at all) work
//! against a remote port.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use async_trait::async_trait;
use cat_transport_core::completion;
use cat_transport_core::{ModemControlLines, Transport, TransportError};

use crate::codec::{self, client, control, modem, server, Decoder, Event};

/// How long [`Rfc2217Port::connect`] waits for the peer's first modem-state
/// notification before giving up on it. See [`Rfc2217Port::settle`].
const SETTLE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

/// How a remote port is opened.
///
/// Deliberately its own type rather than `cat-transport-serial`'s
/// `SerialConfig`: this crate depends only on `cat-transport-core`, the
/// same dependency rule every other transport crate here follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rfc2217Config {
    pub baud_rate: u32,
    /// 5, 6, 7 or 8.
    pub data_bits: u8,
    /// 1 or 2. (RFC 2217 also defines 3 for 1.5 stop bits.)
    pub stop_bits: u8,
    pub parity: Parity,
    /// Whether RTS is asserted as part of opening.
    ///
    /// On a TS-570D this must be `true`: the radio's RTS input is
    /// receive-enable and it withholds CAT responses while the line is low
    /// (instruction manual p. 70).
    pub initial_rts: bool,
    /// Whether DTR is asserted as part of opening.
    ///
    /// `false` for any station whose DTR keys PTT. Asserting it here is
    /// key-down the moment the program starts.
    pub initial_dtr: bool,
}

impl Default for Rfc2217Config {
    /// What a TS-570D's COM port wants: 9600 8N2, RTS up, DTR down.
    fn default() -> Self {
        Self {
            baud_rate: 9600,
            data_bits: 8,
            stop_bits: 2,
            parity: Parity::None,
            initial_rts: true,
            initial_dtr: false,
        }
    }
}

/// RFC 2217 `SET-PARITY` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    None,
    Odd,
    Even,
    Mark,
    Space,
}

impl Parity {
    fn wire(self) -> u8 {
        match self {
            Parity::None => 1,
            Parity::Odd => 2,
            Parity::Even => 3,
            Parity::Mark => 4,
            Parity::Space => 5,
        }
    }
}

/// Bytes received, plus whoever is waiting for them.
struct Inbox {
    data: VecDeque<u8>,
    /// A reader suspended in [`Transport::read`], to be woken when data
    /// arrives or the link ends.
    waiter: Option<completion::CompletionTx<()>>,
    /// Set once the link has ended, with why. `read` drains whatever
    /// arrived before the end, then reports it.
    closed: Option<String>,
}

struct Shared {
    inbox: Mutex<Inbox>,
    /// The last `NOTIFY-MODEMSTATE` byte seen. See the module doc.
    modem_state: Mutex<u8>,
    /// Whether any `NOTIFY-MODEMSTATE` has arrived at all.
    ///
    /// Distinct from the state being zero: "nothing has been reported yet"
    /// and "every line is low" are the same byte and very different facts.
    modem_seen: AtomicBool,
    /// The write half. Shared because the reader thread answers Telnet
    /// negotiations on it.
    writer: Mutex<TcpStream>,
}

impl Shared {
    /// Write raw (already Telnet-encoded) bytes to the peer.
    fn write_raw(&self, bytes: &[u8]) -> Result<(), TransportError> {
        let mut w = self.writer.lock().expect("rfc2217 writer lock");
        w.write_all(bytes).map_err(TransportError::Io)?;
        w.flush().map_err(TransportError::Io)
    }

    /// Record that the link has ended, and wake any suspended reader so it
    /// reports the end instead of hanging.
    fn close(&self, why: String) {
        let mut inbox = self.inbox.lock().expect("rfc2217 inbox lock");
        if inbox.closed.is_none() {
            inbox.closed = Some(why);
        }
        if let Some(waiter) = inbox.waiter.take() {
            waiter.send(());
        }
    }
}

/// A serial port on the far end of an RFC 2217 connection.
pub struct Rfc2217Port {
    shared: Arc<Shared>,
}

impl Rfc2217Port {
    /// Connect to a device server and negotiate the port settings.
    ///
    /// Returns once the negotiation has been *sent*. RFC 2217 has no
    /// handshake to wait for — a server answers `SET-BAUDRATE` and friends
    /// with its own confirming subnegotiations, but it is not required to
    /// do so before it starts passing data, so waiting for them would be
    /// waiting for something the protocol does not promise.
    pub fn connect<A: ToSocketAddrs>(
        addr: A,
        config: Rfc2217Config,
    ) -> Result<Self, TransportError> {
        let stream = TcpStream::connect(addr).map_err(TransportError::Io)?;
        // Small control writes must not sit in Nagle's queue waiting for
        // company: a PTT key is six bytes and needs to leave now.
        stream.set_nodelay(true).map_err(TransportError::Io)?;
        let reader = stream.try_clone().map_err(TransportError::Io)?;

        let shared = Arc::new(Shared {
            inbox: Mutex::new(Inbox {
                data: VecDeque::new(),
                waiter: None,
                closed: None,
            }),
            modem_state: Mutex::new(0),
            modem_seen: AtomicBool::new(false),
            writer: Mutex::new(stream),
        });

        let port = Rfc2217Port {
            shared: Arc::clone(&shared),
        };

        thread::Builder::new()
            .name("rfc2217-reader".to_string())
            .spawn(move || reader_loop(reader, shared))
            .map_err(TransportError::Io)?;

        port.negotiate(config)?;
        port.settle(SETTLE_TIMEOUT);
        Ok(port)
    }

    /// Wait, briefly, for the peer's first `NOTIFY-MODEMSTATE`.
    ///
    /// Without this, `read_cts()` immediately after `connect()` reports
    /// every line low — not because they are, but because nothing has been
    /// reported yet. That is a real answer to the wrong question, and it
    /// bites the tool most likely to ask it: a bench utility whose whole
    /// job is to print CTS/DSR/DCD does so the instant it opens the port.
    ///
    /// The wait is bounded and its expiry is not an error. A server that
    /// never notifies (one that ignored `SET-MODEMSTATE-MASK`) costs this
    /// timeout once, and its lines then read low — which is the same answer
    /// it would have given anyway, arrived at honestly.
    fn settle(&self, timeout: std::time::Duration) {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if self.shared.modem_seen.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// Whether the peer has reported the modem lines at least once.
    ///
    /// A caller that needs to distinguish "the lines are low" from "this
    /// server does not report lines" can ask.
    pub fn modem_state_reported(&self) -> bool {
        self.shared.modem_seen.load(Ordering::SeqCst)
    }

    /// The opening exchange: agree binary transmission, claim the Com Port
    /// option, then state the port settings and the lines.
    fn negotiate(&self, config: Rfc2217Config) -> Result<(), TransportError> {
        let mut out = Vec::new();
        for verb in [codec::WILL, codec::DO] {
            out.extend_from_slice(&codec::negotiate(verb, codec::OPT_BINARY));
            out.extend_from_slice(&codec::negotiate(verb, codec::OPT_SUPPRESS_GO_AHEAD));
        }
        out.extend_from_slice(&codec::negotiate(codec::WILL, codec::OPT_COM_PORT));

        out.extend_from_slice(&codec::set_baud_rate(config.baud_rate));
        out.extend_from_slice(&codec::com_port(client::SET_DATASIZE, &[config.data_bits]));
        out.extend_from_slice(&codec::com_port(
            client::SET_PARITY,
            &[config.parity.wire()],
        ));
        out.extend_from_slice(&codec::com_port(client::SET_STOPSIZE, &[config.stop_bits]));
        out.extend_from_slice(&codec::set_control(control::FLOW_NONE));

        // Lines last, and explicitly in both directions. A port that
        // inherited whatever the previous client left asserted would key a
        // transmitter on connect.
        out.extend_from_slice(&codec::set_control(if config.initial_dtr {
            control::DTR_ON
        } else {
            control::DTR_OFF
        }));
        out.extend_from_slice(&codec::set_control(if config.initial_rts {
            control::RTS_ON
        } else {
            control::RTS_OFF
        }));

        // Ask to be told about every line, so the cache the sync readers
        // answer from is kept current by the peer rather than by polling.
        out.extend_from_slice(&codec::com_port(
            client::SET_MODEMSTATE_MASK,
            &[modem::ALL_LEVELS],
        ));

        self.shared.write_raw(&out)
    }

    /// The last modem state byte received, for callers that want the raw
    /// RFC 2217 value rather than one line at a time.
    pub fn modem_state(&self) -> u8 {
        *self.shared.modem_state.lock().expect("rfc2217 modem lock")
    }

    fn set_line(&self, on: bool, on_value: u8, off_value: u8) -> Result<(), TransportError> {
        let value = if on { on_value } else { off_value };
        self.shared.write_raw(&codec::set_control(value))
    }
}

/// Dropping the port disconnects, and it has to be said explicitly.
///
/// The reader thread holds its own `Arc<Shared>`, so letting the last
/// `Rfc2217Port` fall out of scope frees nothing: the socket stays open,
/// the thread stays parked in a read, and the peer never sees a
/// disconnect. On a station whose DTR keys PTT that is not a leak, it is a
/// **transmitter left keyed by a program that has exited** — the remote
/// equivalent of a serial port whose lines never dropped.
///
/// Shutting the socket down makes the reader's next `read` return 0, so it
/// closes the inbox (waking any suspended reader) and exits, releasing the
/// last reference.
impl Drop for Rfc2217Port {
    fn drop(&mut self) {
        if let Ok(writer) = self.shared.writer.lock() {
            // Both directions: shutting down only the write half would
            // leave the reader parked until the peer happened to close.
            let _ = writer.shutdown(Shutdown::Both);
        }
    }
}

/// The reader thread body: own the read half, decode forever, and never
/// block anything else in the process.
fn reader_loop(mut stream: TcpStream, shared: Arc<Shared>) {
    let mut decoder = Decoder::new();
    let mut buf = [0u8; 4096];
    let mut events = Vec::new();

    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) => {
                shared.close("the remote port closed the connection".to_string());
                return;
            }
            Ok(n) => n,
            Err(e) => {
                shared.close(format!("reading the remote port failed: {e}"));
                return;
            }
        };

        events.clear();
        decoder.push(&buf[..n], &mut events);

        let mut arrived = false;
        for event in &events {
            match event {
                Event::Data(bytes) => {
                    let mut inbox = shared.inbox.lock().expect("rfc2217 inbox lock");
                    inbox.data.extend(bytes.iter().copied());
                    arrived = true;
                }
                Event::ComPort { command, params } => {
                    if *command == server::NOTIFY_MODEMSTATE {
                        if let Some(&state) = params.first() {
                            *shared.modem_state.lock().expect("rfc2217 modem lock") = state;
                            shared.modem_seen.store(true, Ordering::SeqCst);
                        }
                    }
                }
                Event::Negotiate { verb, option } => {
                    answer_negotiation(&shared, *verb, *option);
                }
                Event::OtherSubnegotiation { .. } => {}
            }
        }

        if arrived {
            let waiter = shared
                .inbox
                .lock()
                .expect("rfc2217 inbox lock")
                .waiter
                .take();
            if let Some(waiter) = waiter {
                waiter.send(());
            }
        }
    }
}

/// Answer a peer's negotiation.
///
/// Only the three options this crate actually wants are agreed to;
/// everything else is refused. The refusal matters as much as the
/// agreement: an unanswered `DO` leaves a strict Telnet peer waiting.
///
/// Nothing is sent in reply to `WILL`/`DO` for an option already agreed
/// during [`Rfc2217Port::negotiate`], which is what keeps two polite peers
/// from negotiating the same option at each other forever.
fn answer_negotiation(shared: &Shared, verb: u8, option: u8) {
    let supported = matches!(
        option,
        codec::OPT_BINARY | codec::OPT_SUPPRESS_GO_AHEAD | codec::OPT_COM_PORT
    );
    let reply = match verb {
        codec::DO if !supported => Some(codec::WONT),
        codec::WILL if !supported => Some(codec::DONT),
        // Agreed already, in the opening exchange. Answering again would
        // be an echo the peer is entitled to answer in turn.
        _ => None,
    };
    if let Some(verb) = reply {
        let _ = shared.write_raw(&codec::negotiate(verb, option));
    }
}

#[async_trait(?Send)]
impl Transport for Rfc2217Port {
    async fn write(&mut self, data: &[u8]) -> Result<usize, TransportError> {
        self.shared.write_raw(&codec::encode_data(data))?;
        // The count the caller cares about is their own bytes, not the
        // escaped length that went on the wire.
        Ok(data.len())
    }

    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            let receiver = {
                let mut inbox = self.shared.inbox.lock().expect("rfc2217 inbox lock");
                if !inbox.data.is_empty() {
                    let n = inbox.data.len().min(buf.len());
                    for slot in buf.iter_mut().take(n) {
                        *slot = inbox.data.pop_front().expect("checked non-empty");
                    }
                    return Ok(n);
                }
                if let Some(why) = &inbox.closed {
                    return Err(TransportError::Other(why.clone()));
                }
                // Registered while the lock is held, so a byte arriving
                // between the emptiness check and the registration cannot
                // leave this task asleep with data waiting.
                let (tx, rx) = completion::channel();
                inbox.waiter = Some(tx);
                rx
            };
            // Canceled means the sender was dropped without sending, which
            // only happens if a second reader replaced our waiter. Looping
            // re-checks the inbox, which is the right answer either way.
            let _ = receiver.await;
        }
    }

    async fn flush(&mut self) -> Result<(), TransportError> {
        let mut w = self.shared.writer.lock().expect("rfc2217 writer lock");
        w.flush().map_err(TransportError::Io)
    }

    fn flush_rx(&mut self) {
        self.shared
            .inbox
            .lock()
            .expect("rfc2217 inbox lock")
            .data
            .clear();
    }
}

impl ModemControlLines for Rfc2217Port {
    fn set_rts(&self, asserted: bool) -> Result<(), TransportError> {
        self.set_line(asserted, control::RTS_ON, control::RTS_OFF)
    }

    fn set_dtr(&self, asserted: bool) -> Result<(), TransportError> {
        self.set_line(asserted, control::DTR_ON, control::DTR_OFF)
    }

    fn read_cts(&self) -> Result<bool, TransportError> {
        Ok(codec::modem_cts(self.modem_state()))
    }

    fn read_dsr(&self) -> Result<bool, TransportError> {
        Ok(codec::modem_dsr(self.modem_state()))
    }

    fn read_dcd(&self) -> Result<bool, TransportError> {
        Ok(codec::modem_dcd(self.modem_state()))
    }
}
