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

//! A blocking listener, for a host that has no async runtime.
//!
//! [`NativeSession`] is I/O-free and `cat-server` drives it from monoio.
//! Not every host wants that: the TS-570D emulator is a plain synchronous
//! program with a PTY and a TUI, and making it adopt an io_uring runtime
//! to answer a socket would be absurd.
//!
//! So this is `std::net`, one thread per connection, and no runtime —
//! matching the client in `client.rs` for the same reason.
//!
//! # What a host has to provide
//!
//! [`RadioHost`] is three methods: what the radio *is*, what it is
//! *doing*, and how to *change* it. Everything else — the handshake,
//! version checking, capability validation, spectrum gating, framing — is
//! already in `NativeSession` and does not get reimplemented per host.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cat_signal::SpectrumFrame;

use crate::{
    decode_frame, encode_frame, ClientMessage, Command, ErrorCode, FrameKind, NativeSession,
    RadioCapabilities, RadioState, ServerMessage,
};

/// The radio behind a [`serve`] listener.
///
/// `&self` throughout, not `&mut self`: one radio serves every connected
/// client, so a host with mutable state owns its own lock. Making that the
/// host's problem rather than this module's is deliberate — a `Mutex` here
/// would serialise every client behind whichever one is slowest, and a
/// host that is already synchronised (or immutable) would pay for nothing.
pub trait RadioHost: Send + Sync + 'static {
    /// What this radio is. Published once per connection, at handshake.
    fn capabilities(&self) -> &'static RadioCapabilities;

    /// What the radio is doing right now.
    fn state(&self) -> RadioState;

    /// How this radio's console should be arranged.
    ///
    /// The server authors it because the server is what knows the rig.
    /// `None` — the default — leaves a console to its own arrangement,
    /// which is what an older server means by saying nothing.
    fn layout(&self) -> Option<cat_layout::LayoutSpec> {
        None
    }

    /// What this radio's console should look like.
    ///
    /// `None` leaves a console its own palette, which is what an older
    /// server means by saying nothing.
    fn theme(&self) -> Option<cat_layout::Theme> {
        None
    }

    /// What this bench actually has wired.
    ///
    /// Published once per connection, in the handshake, beside the model's
    /// own capabilities — the two answer different questions and change at
    /// different times (ADR 0015). A console asking "should I draw a
    /// waterfall?" reads this; one asking "could this radio ever have
    /// one?" reads the model.
    ///
    /// The default is an empty installation, which says *nothing is
    /// wired*. That is a real answer and the right one for a host that
    /// serves CAT alone — but a host with a source attached must say so,
    /// or a console cannot tell "nothing here" from "nothing yet".
    fn installation(&self) -> cat_framework::installation::Installation {
        cat_framework::installation::Installation::default()
    }

    /// Apply a command that has already been validated against
    /// capabilities.
    ///
    /// `Err(message)` refuses it. Validation the capability set can do has
    /// happened already; this is for what only the radio knows — a memory
    /// channel that is empty, a mode the radio will not enter on this
    /// band.
    ///
    /// [`Command::AttachDevice`] arrives here too, for a host that offers
    /// [`RadioHost::devices`]. Returning the OS error text verbatim is
    /// worth more than a tidy message: "device or resource busy" names
    /// the other program holding the dongle.
    fn apply(&self, command: &Command) -> Result<(), String>;

    /// What signal hardware this machine can see, if this host offers
    /// device selection at all.
    ///
    /// `None` -- the default -- declines the question, and a client is
    /// told exactly that. Returning `Some(vec![])` instead would assert
    /// that the machine has no sound cards and no SDRs, which a host that
    /// simply never looked is in no position to claim.
    ///
    /// Enumerated per call rather than cached, because it changes: a
    /// dongle plugged in after the server started should appear without
    /// restarting it.
    fn devices(&self) -> Option<Vec<cat_signal::DeviceList>> {
        None
    }

    /// The newest spectrum frame, if this radio produces one.
    ///
    /// Polled; returning `None` simply sends nothing. A host with no
    /// spectrum source leaves this alone.
    fn spectrum(&self) -> Option<SpectrumFrame> {
        None
    }

    /// The newest audio frame, if this radio's host is capturing one.
    ///
    /// Separate from [`RadioHost::spectrum`] because they are separate
    /// hardware answering separate questions: a bench can have an SDR on
    /// the IF tap and nothing on the audio pair, or the reverse. A host
    /// with no audio leaves this alone.
    fn audio(&self) -> Option<cat_signal::AudioFrame> {
        None
    }
}

/// How often a connection re-reads state and spectrum.
///
/// Spectrum is the fast lane and this is the rate it goes out at. 30 ms is
/// a little over 30 fps — fast enough that a waterfall scrolls smoothly,
/// slow enough that a dummy radio does not saturate a loopback socket with
/// frames nobody asked to be that fresh.
const PUMP_INTERVAL: Duration = Duration::from_millis(30);

/// Serve the native protocol until the listener fails.
///
/// Blocks. One thread per connection, so a client that stops reading
/// stalls only itself.
pub fn serve<H: RadioHost>(listener: TcpListener, host: Arc<H>) -> std::io::Result<()> {
    loop {
        let (stream, _) = listener.accept()?;
        let host = Arc::clone(&host);
        std::thread::spawn(move || {
            let _ = serve_one(stream, host);
        });
    }
}

/// Bind and serve. Convenience for the common case.
pub fn serve_at<A: std::net::ToSocketAddrs, H: RadioHost>(
    addr: A,
    host: Arc<H>,
) -> std::io::Result<()> {
    serve(TcpListener::bind(addr)?, host)
}

fn serve_one<H: RadioHost>(mut stream: TcpStream, host: Arc<H>) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    // Bounded so the loop comes back to the spectrum pump even when the
    // client is silent. Without this a connection that only listens would
    // never be sent a frame.
    stream.set_read_timeout(Some(PUMP_INTERVAL))?;

    let mut session = NativeSession::with_installation(host.capabilities(), host.installation());
    // Published in the handshake, beside what the radio is: a console is
    // told how to arrange itself in the same breath as what it is
    // arranging.
    session.set_layout(host.layout());
    session.set_theme(host.theme());
    let mut pending: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    let mut last_pump = Instant::now();
    let mut last_sequence: Option<u64> = None;
    let mut last_audio_sequence: Option<u64> = None;

    loop {
        match stream.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => pending.extend_from_slice(&buf[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e),
        }

        let mut consumed = 0;
        loop {
            let Ok((kind, payload, used)) = decode_frame(&pending[consumed..]) else {
                break;
            };
            if kind == FrameKind::Control {
                let reply = match serde_json::from_slice::<ClientMessage>(payload) {
                    Ok(message) => {
                        // State is refreshed before every message, so a
                        // read is answered from now rather than from
                        // whenever the last client happened to ask.
                        session.publish_state(host.state());
                        // Republished per message for the same reason as
                        // state: an answer assembled from whenever the
                        // last client happened to ask would describe a
                        // machine that has since had hardware plugged in.
                        if let Some(devices) = host.devices() {
                            session.publish_devices(devices);
                        }
                        let reply = session.handle(message.clone());
                        // Only apply what the session accepted. A command
                        // it refused never reaches the radio, which is the
                        // point of validating against capabilities.
                        if let (ClientMessage::Command(command), ServerMessage::Ack) =
                            (&message, &reply)
                        {
                            match host.apply(command) {
                                Ok(()) => reply,
                                Err(message) => ServerMessage::Error {
                                    code: ErrorCode::OutOfRange,
                                    message,
                                },
                            }
                        } else {
                            reply
                        }
                    }
                    Err(e) => ServerMessage::Error {
                        code: ErrorCode::Malformed,
                        message: e.to_string(),
                    },
                };
                write_control(&mut stream, &reply)?;
            }
            consumed += used;
        }
        pending.drain(..consumed);

        if last_pump.elapsed() >= PUMP_INTERVAL {
            last_pump = Instant::now();
            if session.wants_audio() {
                if let Some(frame) = host.audio() {
                    // Same don't-resend rule as spectrum, and it matters
                    // more here: a scope trace repeated is a waveform the
                    // radio is not producing, and a stalled capture would
                    // look like a steady tone.
                    if last_audio_sequence != Some(frame.sequence()) {
                        last_audio_sequence = Some(frame.sequence());
                        let payload = crate::encode_audio_payload(&frame);
                        stream.write_all(&encode_frame(FrameKind::Audio, &payload))?;
                        stream.flush()?;
                    }
                }
            }
            if session.wants_spectrum() {
                if let Some(frame) = host.spectrum() {
                    // Don't resend a frame the client already has. A
                    // source slower than the pump would otherwise have its
                    // newest frame sent repeatedly, which costs bandwidth
                    // and makes a stalled source look live.
                    if last_sequence != Some(frame.sequence) {
                        last_sequence = Some(frame.sequence);
                        let payload = crate::encode_spectrum_payload(&frame);
                        stream.write_all(&encode_frame(FrameKind::Spectrum, &payload))?;
                        stream.flush()?;
                    }
                }
            }
        }
    }
}

fn write_control(stream: &mut TcpStream, message: &ServerMessage) -> std::io::Result<()> {
    let payload = serde_json::to_vec(message)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    stream.write_all(&encode_frame(FrameKind::Control, &payload))?;
    stream.flush()
}
