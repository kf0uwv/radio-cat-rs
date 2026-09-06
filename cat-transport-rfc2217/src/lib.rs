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

//! RFC 2217: a serial port, **with its modem control lines**, over TCP.
//!
//! # Why this crate exists
//!
//! Every other transport here moves CAT bytes. This one moves the wires as
//! well, and that is the whole reason for it.
//!
//! A station that keys its transmitter from DTR — the common
//! opto-isolator-on-ACC2 arrangement — has behaviour that lives entirely in
//! the RS-232 control lines and not at all in the CAT stream: DTR keys PTT,
//! RTS is the radio's receive-enable, CTS says the radio's COM port is
//! alive. None of that can be exercised over a pseudo-terminal, because
//! **Linux ptys implement no modem-control ioctls at all** — `TIOCMGET`,
//! `TIOCMBIS` and `TIOCMBIC` all fail with `ENOTTY` on both ends. So a
//! virtual radio on a PTY can be a faithful CAT peer and still cannot be
//! keyed, and the software that keys it cannot be tested.
//!
//! RFC 2217 is the protocol that already solves this: `ser2net` speaks it,
//! and so do Moxa, Digi and USR device servers. Using it rather than
//! inventing a private side-channel means a virtual radio and a real
//! serial device server are interchangeable to everything upstream — the
//! same reasoning that put `rtl_tcp` rather than a bespoke protocol on the
//! IF tap.
//!
//! # Shape
//!
//! - [`codec`] — the wire, as pure functions. Telnet framing and the Com
//!   Port Control Option, both directions, no I/O.
//! - [`port`] — [`Rfc2217Port`], the client: a
//!   [`Transport`](cat_transport_core::Transport) +
//!   [`ModemControlLines`](cat_transport_core::ModemControlLines).
//! - [`server`] — [`Rfc2217Peer`], the device-server half of one
//!   connection, also with no I/O in it, so a host program supplies only
//!   its sockets.
//!
//! # A monoio caller must enable monoio's `sync` feature
//!
//! [`port`]'s reader thread wakes the task suspended in
//! [`Transport::read`](cat_transport_core::Transport::read) from a
//! **different OS thread**. That is sound against `std::task::Waker`'s
//! documented contract, and it is what
//! [`cat_transport_core::completion`] exists for — but **monoio's waker
//! panics on a cross-thread wake unless its `sync` feature is on**:
//!
//! ```text
//! thread 'rfc2217-reader' panicked at monoio/src/task/harness.rs:197:17:
//! waker can only be sent across threads when `sync` feature enabled
//! ```
//!
//! So an application driving this transport from a monoio runtime must
//! declare `monoio = { version = "…", features = ["sync"] }`. `ts570d` does,
//! with the reason written at the dependency.
//!
//! This crate cannot enforce it: it has no monoio dependency of its own —
//! it is plain `std::net` and `std::thread` — and that is the point of the
//! design, not an oversight. Nor can any test here catch a caller getting
//! it wrong: this crate's tests drive it with
//! `futures::executor::block_on`, whose waker *is* thread-safe.
//! `ts570d/tests/rfc2217_under_monoio.rs` is the guard, and it lives there
//! because that is where the runtime and the feature flag both are.
//!
//! # Framing
//!
//! Framing is deliberately *not* here. An RFC 2217 endpoint is a serial
//! port, so `cat_transport_serial::SerialCatSession<Rfc2217Port>` frames it
//! exactly as it frames a local one, and forwards the modem lines through
//! its own blanket impl. See [`port`]'s module doc.
//!
//! # Example
//!
//! ```ignore
//! use cat_transport_rfc2217::{Rfc2217Config, Rfc2217Port};
//! use cat_transport_serial::SerialCatSession;
//!
//! // 9600 8N2, RTS asserted (the TS-570D's receive-enable), DTR low so a
//! // DTR-keyed PTT interface does not transmit at startup.
//! let port = Rfc2217Port::connect("radio.local:4001", Rfc2217Config::default())?;
//! let session = SerialCatSession::new(port);
//! # Ok::<(), cat_transport_core::TransportError>(())
//! ```

pub mod codec;
pub mod port;
pub mod server;

pub use codec::{Decoder, Event};
pub use port::{Parity, Rfc2217Config, Rfc2217Port};
pub use server::{PeerEvent, Rfc2217Peer};
