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

//! The client half of the native protocol.
//!
//! # Why this is blocking, and has no runtime
//!
//! [`NativeSession`](crate::NativeSession) is deliberately I/O-free, and
//! the server drives it from monoio. Nothing about the *client* needs
//! that. A GUI is a normal thread with a frame loop, and `ts570d` ADR 0008
//! makes the GUI network-only precisely so it inherits no `!Send`
//! constraint, no executor split and no Windows special case.
//!
//! So [`Connection`] is `std::net` and blocking. The one thing a frame
//! loop must never do is block on a socket, and that is what [`Client`] is
//! for: it owns a reader thread and hands the caller channels.
//!
//! # The spectrum channel is the reason this is not just "send JSON"
//!
//! Two frame kinds share one socket, and they behave nothing alike.
//! Control traffic is request/response and slow; spectrum frames arrive
//! unbidden at up to 60 fps. A client that read the socket looking for its
//! command reply would find frames in the way, and one that queued every
//! frame it received would grow without bound the moment the UI thread
//! fell behind.
//!
//! [`Connection::request`] therefore skips past spectrum frames rather
//! than choking on them, and [`Client`] keeps only the **newest** frame
//! rather than a backlog — a waterfall wants the current spectrum, and a
//! stale one is worse than none.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cat_signal::SpectrumFrame;

use crate::{
    decode_frame, encode_frame, CapabilitiesWire, ClientMessage, Command, FrameError, FrameKind,
    ServerMessage, PROTOCOL_VERSION,
};

/// What went wrong talking to a server.
#[derive(Debug)]
pub enum ClientError {
    Io(io::Error),
    /// The frame decoder rejected the stream.
    Frame(FrameError),
    /// The bytes framed correctly but were not a message.
    Decode(serde_json::Error),
    /// The server answered a handshake with something other than a
    /// `Welcome`, or a command with something unexpected.
    Unexpected(ServerMessage),
    /// The server speaks a different protocol version.
    VersionMismatch {
        ours: u16,
        theirs: u16,
    },
    /// The peer closed the connection.
    Closed,
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Io(e) => write!(f, "io error: {e}"),
            ClientError::Frame(e) => write!(f, "bad frame: {e:?}"),
            ClientError::Decode(e) => write!(f, "undecodable message: {e}"),
            ClientError::Unexpected(m) => write!(f, "unexpected reply: {m:?}"),
            ClientError::VersionMismatch { ours, theirs } => {
                write!(
                    f,
                    "protocol version mismatch: we speak {ours}, server speaks {theirs}"
                )
            }
            ClientError::Closed => write!(f, "connection closed by the server"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<io::Error> for ClientError {
    fn from(e: io::Error) -> Self {
        ClientError::Io(e)
    }
}

type Result<T> = std::result::Result<T, ClientError>;

/// A blocking connection to a native-protocol server.
///
/// Which continuous streams a client wants.
///
/// A struct rather than a pair of bools, because two bools in a row at a
/// call site is the argument order nobody gets right — and `connect(addr,
/// true, false)` says nothing about which is which. Each is a separate
/// opt-in: a console running a waterfall with no AF panels should not be
/// sent audio it will throw away, and one drawing only meters wants
/// neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Streams {
    pub spectrum: bool,
    pub audio: bool,
    /// How many frames a second this console can actually render.
    ///
    /// `None` accepts the server's conservative default. A GPU console
    /// says 30; a terminal console should not, and the measurement that
    /// says so is in `ClientMessage::Hello`.
    pub max_fps: Option<u8>,
}

impl Streams {
    /// Control messages only.
    pub fn none() -> Self {
        Self::default()
    }

    /// The band panorama, and nothing else.
    pub fn spectrum() -> Self {
        Self {
            spectrum: true,
            audio: false,
            max_fps: None,
        }
    }

    /// Everything the server will send.
    pub fn all() -> Self {
        Self {
            spectrum: true,
            audio: true,
            max_fps: None,
        }
    }

    /// Ask for frames at a rate this console can actually render.
    ///
    /// Left unsaid, the server keeps a conservative default that suits a
    /// terminal. A console that can draw faster has to say so, because
    /// the server has no way to find out and guessing high is what makes
    /// a slow console appear to hang.
    pub fn at_fps(mut self, fps: u8) -> Self {
        self.max_fps = Some(fps);
        self
    }
}

/// Handshaken on construction, so a `Connection` that exists has already
/// agreed a version and holds the radio's capabilities. There is no
/// "connected but not yet ready" state for a caller to forget to check.
pub struct Connection {
    stream: TcpStream,
    capabilities: CapabilitiesWire,
    /// Bytes read but not yet consumed by the frame decoder.
    pending: Vec<u8>,
    /// Control messages decoded but not yet handed to a caller.
    ///
    /// A queue rather than a single slot because one read can carry
    /// several: a reply and an unsolicited message can share a packet, and
    /// dropping the second would lose an answer somebody is waiting for.
    inbox: std::collections::VecDeque<Result<ServerMessage>>,
    /// Spectrum frames that arrived while waiting for a control reply.
    /// Only the newest is kept — see this module's docs.
    latest_spectrum: Option<SpectrumFrame>,
    /// The newest audio frame, on the same newest-wins terms.
    latest_audio: Option<cat_signal::AudioFrame>,
    streams: Streams,
}

impl Connection {
    /// Connect and complete the handshake.
    ///
    /// `streams` decides which continuous frames this client receives.
    /// Declining costs nothing and receives nothing: the server sends not
    /// one byte of frame traffic a client did not ask for.
    pub fn connect<A: ToSocketAddrs>(addr: A, streams: Streams) -> Result<Self> {
        let stream = TcpStream::connect(addr)?;
        // Nagle would batch small control writes behind each other, which
        // on a request/response protocol is latency for no benefit.
        stream.set_nodelay(true)?;
        Self::handshake(stream, streams)
    }

    fn handshake(stream: TcpStream, streams: Streams) -> Result<Self> {
        let mut conn = Self {
            stream,
            capabilities: placeholder_capabilities(),
            pending: Vec::new(),
            inbox: std::collections::VecDeque::new(),
            latest_spectrum: None,
            latest_audio: None,
            streams,
        };
        conn.send(&ClientMessage::Hello {
            version: PROTOCOL_VERSION,
            spectrum: streams.spectrum,
            audio: streams.audio,
            max_fps: streams.max_fps,
        })?;
        match conn.read_control()? {
            ServerMessage::Welcome {
                version,
                capabilities,
            } => {
                if version != PROTOCOL_VERSION {
                    return Err(ClientError::VersionMismatch {
                        ours: PROTOCOL_VERSION,
                        theirs: version,
                    });
                }
                conn.capabilities = *capabilities;
                Ok(conn)
            }
            other => Err(ClientError::Unexpected(other)),
        }
    }

    /// What the radio on the other end says it is.
    pub fn capabilities(&self) -> &CapabilitiesWire {
        &self.capabilities
    }

    /// Whether this connection asked for spectrum frames.
    pub fn spectrum_enabled(&self) -> bool {
        self.streams.spectrum
    }

    /// Which continuous streams this connection asked for.
    pub fn streams(&self) -> Streams {
        self.streams
    }

    /// The most recent audio frame seen, if any, clearing it.
    pub fn take_audio(&mut self) -> Option<cat_signal::AudioFrame> {
        self.latest_audio.take()
    }

    /// Send a command and wait for its reply.
    ///
    /// Spectrum frames arriving in the meantime are skipped rather than
    /// treated as a protocol error; the newest is kept and can be taken
    /// with [`Connection::take_spectrum`].
    pub fn command(&mut self, command: Command) -> Result<ServerMessage> {
        self.request(&ClientMessage::Command(command))
    }

    /// Ask what the radio is doing.
    ///
    /// One round trip for everything a console displays. Asking field by
    /// field would let a readout show a frequency from one moment beside a
    /// mode from another, describing a radio that never existed.
    pub fn read_state(&mut self) -> Result<crate::RadioState> {
        match self.command(Command::ReadState)? {
            ServerMessage::State(state) => Ok(*state),
            other => Err(ClientError::Unexpected(other)),
        }
    }

    /// Ask what signal hardware the *radio's* host can see.
    ///
    /// `Ok(None)` means the server declines the question -- it does not
    /// offer device selection. That is deliberately not the same as
    /// `Ok(Some(vec![]))`, which would be the server asserting its machine
    /// has nothing attached. A console shows those differently, because
    /// "plug something in" and "this server cannot help you from here"
    /// send an operator to opposite ends of the shack.
    ///
    /// A server built before this command existed cannot deserialize the
    /// `cmd` tag and answers `Malformed`. That is reported as `Ok(None)`
    /// too: from a console's side an old server and a declining one are
    /// the same situation, and neither is a fault worth an error dialog.
    pub fn read_devices(&mut self) -> Result<Option<Vec<cat_signal::DeviceList>>> {
        match self.command(Command::ReadDevices)? {
            ServerMessage::Devices { lists } => Ok(Some(lists)),
            ServerMessage::Error {
                code: crate::ErrorCode::Unsupported | crate::ErrorCode::Malformed,
                ..
            } => Ok(None),
            other => Err(ClientError::Unexpected(other)),
        }
    }

    /// Ask the radio's host to attach one of the devices it offered.
    ///
    /// `spec` must come from a [`cat_signal::DeviceInfo`] the server sent,
    /// not be composed here: device specs are the host's namespace.
    ///
    /// `Err(ClientError::Unexpected(Error { .. }))` carries the host's own
    /// refusal message, which is the useful thing to show -- "device or
    /// resource busy" tells an operator far more than "attach failed".
    pub fn attach_device(&mut self, kind: cat_signal::DeviceKind, spec: &str) -> Result<()> {
        match self.command(Command::AttachDevice {
            kind,
            spec: spec.to_string(),
        })? {
            ServerMessage::Ack => Ok(()),
            other => Err(ClientError::Unexpected(other)),
        }
    }

    /// Round-trip a ping. Useful as a liveness check that changes nothing.
    pub fn ping(&mut self) -> Result<()> {
        match self.request(&ClientMessage::Ping)? {
            ServerMessage::Pong => Ok(()),
            other => Err(ClientError::Unexpected(other)),
        }
    }

    fn request(&mut self, message: &ClientMessage) -> Result<ServerMessage> {
        self.send(message)?;
        self.read_control()
    }

    /// The most recent spectrum frame seen, if any, clearing it.
    pub fn take_spectrum(&mut self) -> Option<SpectrumFrame> {
        self.latest_spectrum.take()
    }

    /// Read whatever has arrived, without waiting for a control reply.
    ///
    /// Returns the newest spectrum frame if one arrived. `timeout` bounds
    /// the wait; `None` blocks until something comes.
    pub fn poll(&mut self, timeout: Option<Duration>) -> Result<Option<SpectrumFrame>> {
        self.stream.set_read_timeout(timeout)?;
        match self.fill() {
            Ok(()) => {}
            // A timeout means nothing arrived, which is not an error.
            Err(ClientError::Io(e))
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
            }
            Err(e) => return Err(e),
        }
        self.stream.set_read_timeout(None)?;
        self.drain_frames();
        Ok(self.latest_spectrum.take())
    }

    fn send(&mut self, message: &ClientMessage) -> Result<()> {
        let payload = serde_json::to_vec(message).map_err(ClientError::Decode)?;
        self.stream
            .write_all(&encode_frame(FrameKind::Control, &payload))?;
        self.stream.flush()?;
        Ok(())
    }

    /// Read until a control frame arrives, keeping the newest spectrum
    /// frame seen along the way.
    fn read_control(&mut self) -> Result<ServerMessage> {
        loop {
            if let Some(message) = self.inbox.pop_front() {
                return message;
            }
            self.fill()?;
            self.drain_frames();
        }
    }

    /// A control message already decoded, without touching the socket.
    ///
    /// This is what makes [`Connection::poll`] safe to use in a loop that
    /// also expects replies: `poll` decodes whatever arrived, and anything
    /// that was not a spectrum frame waits here rather than being thrown
    /// away.
    pub fn pending_control(&mut self) -> Option<Result<ServerMessage>> {
        self.inbox.pop_front()
    }

    /// Decode every complete frame in `pending`.
    ///
    /// Control messages queue in `inbox`; spectrum frames overwrite
    /// `latest_spectrum`, so a burst collapses to its newest rather than
    /// building a backlog the UI would then have to catch up on.
    fn drain_frames(&mut self) {
        let mut consumed = 0;
        loop {
            match decode_frame(&self.pending[consumed..]) {
                Err(FrameError::Incomplete) => break,
                Err(e) => {
                    self.inbox.push_back(Err(ClientError::Frame(e)));
                    // The stream's framing is broken; there is nothing
                    // sensible to resynchronize to.
                    consumed = self.pending.len();
                    break;
                }
                Ok((kind, payload, used)) => {
                    match kind {
                        FrameKind::Control => {
                            self.inbox.push_back(
                                serde_json::from_slice(payload).map_err(ClientError::Decode),
                            );
                        }
                        FrameKind::Spectrum => {
                            if let Some(frame) = crate::decode_spectrum_payload(payload) {
                                self.latest_spectrum = Some(frame);
                            }
                        }
                        FrameKind::Audio => {
                            if let Some(frame) = crate::decode_audio_payload(payload) {
                                self.latest_audio = Some(frame);
                            }
                        }
                    }
                    consumed += used;
                }
            }
        }
        self.pending.drain(..consumed);
    }

    /// Read one chunk from the socket into `pending`.
    ///
    /// "The peer went away" reaches a reader in more than one shape: a
    /// clean shutdown gives `Ok(0)`, but a peer that closes with unread
    /// data queued sends RST and gives `ConnectionReset` instead, and a
    /// write to that socket gives `BrokenPipe`. Which one a caller sees is
    /// a matter of timing and platform, not of anything it can act on
    /// differently — so all three become [`ClientError::Closed`] rather
    /// than making every caller match three ways to say one thing.
    fn fill(&mut self) -> Result<()> {
        let mut buf = [0u8; 16 * 1024];
        match self.stream.read(&mut buf) {
            Ok(0) => Err(ClientError::Closed),
            Ok(n) => {
                self.pending.extend_from_slice(&buf[..n]);
                Ok(())
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::BrokenPipe
                        | io::ErrorKind::UnexpectedEof
                ) =>
            {
                Err(ClientError::Closed)
            }
            Err(e) => Err(ClientError::Io(e)),
        }
    }
}

/// A `CapabilitiesWire` that exists only between connect and `Welcome`.
///
/// Never observable: `Connection::handshake` either overwrites it or
/// returns an error, so no caller can hold a `Connection` containing it.
fn placeholder_capabilities() -> CapabilitiesWire {
    CapabilitiesWire {
        model: String::new(),
        endpoints: Vec::new(),
        vfos: cat_framework::capabilities::VfoCapability {
            count: 0,
            split: false,
            rit_hz: None,
            xit_hz: None,
        },
        modes: Vec::new(),
        tuning_steps_hz: Vec::new(),
        rx_range: cat_framework::capabilities::FrequencyRange::new(0, 0),
        filters: crate::FilterWire {
            if_shift_hz: None,
            widths_hz: None,
            notch: false,
        },
        meters: Vec::new(),
        memory: None,
        menu: None,
        signal: cat_framework::capabilities::SignalSupport::None,
        layout: None,
        theme: None,
        installation: cat_framework::installation::Installation::default(),
    }
}

// ---------------------------------------------------------------------------
// The threaded client
// ---------------------------------------------------------------------------

/// What a [`Client`]'s reader thread reports back.
#[derive(Debug)]
pub enum Event {
    Reply(ServerMessage),
    /// The connection ended. No further events follow.
    Disconnected(String),
}

/// A handle that can queue commands from anywhere.
///
/// A [`Client`] cannot be shared -- its event receiver is `!Sync` -- but
/// the *sending* half can be, and often has to be: a console's device
/// picker runs in a closure that has no access to the radio it is
/// attached to, and a background task may want to retune without owning
/// the connection.
///
/// Sending is fire-and-forget. The server's answer, `Ack` or `Error`,
/// arrives through the `Client`'s events like any other, so a holder of a
/// sink cannot see the outcome and should not pretend to.
#[derive(Clone)]
pub struct CommandSink(Sender<ClientMessage>);

impl CommandSink {
    /// Queue a command. `false` means the connection is gone.
    pub fn send(&self, command: Command) -> bool {
        self.0.send(ClientMessage::Command(command)).is_ok()
    }
}

/// A [`Connection`] with a reader thread in front of it.
///
/// The shape a frame loop needs: nothing here blocks. Spectrum frames land
/// in a slot that keeps only the newest, and replies queue on a channel.
///
/// Dropping the `Client` closes the connection, which ends the thread.
pub struct Client {
    commands: Sender<ClientMessage>,
    events: Receiver<Event>,
    spectrum: Arc<Mutex<Option<SpectrumFrame>>>,
    audio: Arc<Mutex<Option<cat_signal::AudioFrame>>>,
    capabilities: CapabilitiesWire,
}

impl Client {
    /// Connect, handshake, and start the reader thread.
    ///
    /// Handshaking on *this* thread rather than in the worker is
    /// deliberate: a caller that gets a `Client` back has already been
    /// told the radio's capabilities, so a GUI never has to render an
    /// "unknown radio" state that exists for a few milliseconds.
    pub fn connect<A: ToSocketAddrs>(addr: A, streams: Streams) -> Result<Self> {
        let mut conn = Connection::connect(addr, streams)?;
        let capabilities = conn.capabilities().clone();

        let (command_tx, command_rx) = mpsc::channel::<ClientMessage>();
        let (event_tx, event_rx) = mpsc::channel::<Event>();
        let slot: Arc<Mutex<Option<SpectrumFrame>>> = Arc::new(Mutex::new(None));
        let thread_slot = Arc::clone(&slot);
        let audio_slot: Arc<Mutex<Option<cat_signal::AudioFrame>>> = Arc::new(Mutex::new(None));
        let thread_audio = Arc::clone(&audio_slot);

        std::thread::spawn(move || {
            loop {
                // Send anything queued, then wait briefly for inbound
                // traffic. The poll timeout is what bounds how long a
                // command sits before it goes out.
                match command_rx.try_recv() {
                    Ok(message) => {
                        if let Err(e) = conn.send(&message) {
                            let _ = event_tx.send(Event::Disconnected(e.to_string()));
                            return;
                        }
                    }
                    Err(TryRecvError::Disconnected) => return,
                    Err(TryRecvError::Empty) => {}
                }

                match conn.poll(Some(Duration::from_millis(20))) {
                    Ok(Some(frame)) => {
                        if let Ok(mut guard) = thread_slot.lock() {
                            *guard = Some(frame);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        let _ = event_tx.send(Event::Disconnected(e.to_string()));
                        return;
                    }
                }
                // `poll` decodes every frame kind and reports only the
                // spectrum; the audio it decoded is waiting in the
                // connection. Same newest-wins terms: a console redrawing
                // at its own rate should see the latest trace, not work
                // through a backlog of stale ones.
                if let Some(frame) = conn.take_audio() {
                    if let Ok(mut guard) = thread_audio.lock() {
                        *guard = Some(frame);
                    }
                }

                // Anything decoded as a control frame during that poll is
                // waiting in the connection; hand it on.
                while let Some(reply) = conn.pending_control() {
                    match reply {
                        Ok(message) => {
                            if event_tx.send(Event::Reply(message)).is_err() {
                                return;
                            }
                        }
                        Err(e) => {
                            let e: ClientError = e;
                            let _ = event_tx.send(Event::Disconnected(e.to_string()));
                            return;
                        }
                    }
                }
            }
        });

        Ok(Self {
            commands: command_tx,
            events: event_rx,
            spectrum: slot,
            audio: audio_slot,
            capabilities,
        })
    }

    /// What the radio on the other end says it is.
    pub fn capabilities(&self) -> &CapabilitiesWire {
        &self.capabilities
    }

    /// Queue a command. Returns `false` if the connection has ended.
    pub fn send(&self, command: Command) -> bool {
        self.commands.send(ClientMessage::Command(command)).is_ok()
    }

    /// Take the next event, if one is waiting. Never blocks.
    pub fn try_event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// Ask for the radio's state; the answer arrives as an [`Event`].
    ///
    /// Deliberately not a blocking `read_state` like [`Connection`]'s. A
    /// frame loop that waited for a reply would drop frames whenever the
    /// link hiccuped, and the whole reason this type exists is that a draw
    /// loop must never block on a socket.
    pub fn request_state(&self) -> bool {
        self.commands
            .send(ClientMessage::Command(Command::ReadState))
            .is_ok()
    }

    /// The newest spectrum frame, if one has arrived since the last call.
    ///
    /// Newest-wins rather than a queue: a waterfall wants the current
    /// spectrum, and a backlog of stale frames is worse than none.
    pub fn take_spectrum(&self) -> Option<SpectrumFrame> {
        self.spectrum.lock().ok().and_then(|mut g| g.take())
    }

    /// The most recent audio frame seen, if any, clearing it.
    pub fn take_audio(&self) -> Option<cat_signal::AudioFrame> {
        self.audio.lock().ok().and_then(|mut g| g.take())
    }

    /// The slot the reader thread drops audio frames into.
    ///
    /// For the same reason as [`Client::spectrum_slot`]: the AF panels are
    /// drawn by whatever thread owns them, which is not this one.
    pub fn audio_slot(&self) -> Arc<Mutex<Option<cat_signal::AudioFrame>>> {
        Arc::clone(&self.audio)
    }

    /// A handle for queueing commands from somewhere this client is not.
    pub fn sink(&self) -> CommandSink {
        CommandSink(self.commands.clone())
    }

    /// The slot the reader thread drops frames into.
    ///
    /// A `Client` cannot itself be shared across threads -- its event
    /// receiver is `!Sync` -- but a spectrum consumer is usually somewhere
    /// else entirely: a console's waterfall runs on its own thread so that
    /// an FFT redraw cannot stall the radio poll. Handing out the slot
    /// lets that thread read frames without the rest of the client coming
    /// with it.
    ///
    /// Newest-wins, and `take`-shaped: a consumer that falls behind skips
    /// frames rather than accumulating a backlog it would then draw late.
    /// That is the right trade for a waterfall and the wrong one for
    /// anything that must see every frame, which is why this is a slot
    /// and not a channel.
    pub fn spectrum_slot(&self) -> Arc<Mutex<Option<SpectrumFrame>>> {
        Arc::clone(&self.spectrum)
    }
}
