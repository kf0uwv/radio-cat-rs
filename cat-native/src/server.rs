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

    /// A console connected.
    ///
    /// Defaulted, so a host that does not care is unaffected. A host that
    /// polls the radio to keep its cache warm does care: polling at
    /// display rate when nobody is looking spends a slow serial link on
    /// nothing, and every other client waits behind it.
    fn console_attached(&self) {}

    /// A console went away, however it went -- cleanly, by error, or by
    /// the thread unwinding. Paired with `console_attached` by a guard, so
    /// a count cannot leak.
    fn console_detached(&self) {}

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
/// How long this loop blocks waiting for a client's next command before
/// coming back round to push a frame.
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
            // A guard rather than a call at each exit: `serve_one` returns
            // early on several error paths, and a leaked count would leave
            // the radio polled at display rate forever.
            struct Attached<'a, H: RadioHost>(&'a H);
            impl<H: RadioHost> Drop for Attached<'_, H> {
                fn drop(&mut self) {
                    self.0.console_detached();
                }
            }
            host.console_attached();
            let _guard = Attached(host.as_ref());
            let _ = serve_one(stream, Arc::clone(&host));
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

/// How long to block for a client's next command before pushing a frame.
///
/// The time left until the next frame is due, capped at [`PUMP_INTERVAL`]
/// so a slow-frame client's commands are still noticed briskly, and
/// floored so a deadline already past does not ask for a zero timeout --
/// which on a socket means *block forever*, the one value that must never
/// be passed here.
fn wake_for(last_display: Instant, interval: Duration) -> Duration {
    /// Below this the timeout is not worth arming precisely, and zero is
    /// forbidden outright: `set_read_timeout(Some(ZERO))` is an error on
    /// some platforms and an infinite block on others.
    const FLOOR: Duration = Duration::from_millis(1);
    interval
        .saturating_sub(last_display.elapsed())
        .clamp(FLOOR, PUMP_INTERVAL)
}

fn serve_one<H: RadioHost>(mut stream: TcpStream, host: Arc<H>) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    // Bounded so the loop comes back to the spectrum pump even when the
    // client is silent. Without this a connection that only listens would
    // never be sent a frame. Re-armed each pass to the next frame's
    // deadline -- see `wake_for`.
    stream.set_read_timeout(Some(PUMP_INTERVAL))?;

    let mut session = NativeSession::with_installation(host.capabilities(), host.installation());
    // Published in the handshake, beside what the radio is: a console is
    // told how to arrange itself in the same breath as what it is
    // arranging.
    session.set_layout(host.layout());
    session.set_theme(host.theme());
    let mut pending: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    let mut last_display = Instant::now();
    let mut last_sequence: Option<u64> = None;
    let mut last_audio_sequence: Option<u64> = None;
    // What the read timeout is currently armed for, so it is only
    // re-armed when it actually changes. `set_read_timeout` is a syscall
    // and this loop runs tens of times a second.
    let mut armed = PUMP_INTERVAL;

    loop {
        // Wake in time for the next frame, not merely soon.
        //
        // This used to block for a flat `PUMP_INTERVAL` of 30 ms while
        // the frame check asked for `elapsed >= interval`. At the default
        // 100 ms those never interfered. At 30 fps -- a 33 ms interval --
        // 30 ms is just short of it, so every frame missed its deadline
        // by 3 ms and waited a whole further tick: frames went out every
        // 60 ms and a console that asked for 30 fps measured **17.9**.
        //
        // Sleeping to the deadline instead costs nothing when the client
        // is slow (the cap still applies) and is exact when it is fast.
        let wake = wake_for(last_display, session.frame_interval());
        if wake != armed {
            stream.set_read_timeout(Some(wake))?;
            armed = wake;
        }
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

        // Pictures go out on their own clock, at the rate this client
        // said it can render. Read from the session each time rather than
        // cached: the handshake that sets it arrives on this same loop,
        // so a value read once at the top would always be the default.
        if last_display.elapsed() >= session.frame_interval() {
            last_display = Instant::now();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fast_client_is_woken_in_time_for_its_deadline() {
        // The defect this exists for: a flat 30 ms block against a 33 ms
        // deadline missed it by 3 ms every time, so frames went out every
        // 60 ms and a console asking for 30 fps measured 17.9.
        let interval = Duration::from_millis(33);
        let wake = wake_for(Instant::now(), interval);
        assert!(
            wake <= interval,
            "must not sleep past the deadline: {wake:?}"
        );
    }

    #[test]
    fn a_slow_client_still_has_its_commands_noticed_briskly() {
        // A 10 fps client must not leave the loop blind to input for
        // 100 ms; the cap is what keeps commands responsive.
        let wake = wake_for(Instant::now(), Duration::from_millis(100));
        assert_eq!(wake, PUMP_INTERVAL);
    }

    #[test]
    fn a_deadline_already_past_asks_for_a_short_wait_not_none() {
        // `set_read_timeout(Some(ZERO))` means block forever on some
        // platforms and is an error on others. Either would wedge this
        // loop, so zero must be unreachable.
        let long_ago = Instant::now() - Duration::from_secs(5);
        let wake = wake_for(long_ago, Duration::from_millis(33));
        assert!(!wake.is_zero(), "zero would block forever");
        assert!(wake <= PUMP_INTERVAL);
    }

    #[test]
    fn the_wait_never_exceeds_the_cap_however_slow_the_client() {
        for ms in [1u64, 33, 100, 1000, 60_000] {
            let wake = wake_for(Instant::now(), Duration::from_millis(ms));
            assert!(wake <= PUMP_INTERVAL, "{ms} ms asked for {wake:?}");
            assert!(!wake.is_zero());
        }
    }
}
