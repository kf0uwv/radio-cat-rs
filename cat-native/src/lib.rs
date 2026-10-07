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

//! The native typed protocol.
//!
//! See `docs/adr/0010-capability-model-and-normalized-signal-source.md` §6.
//! Task 16 of `planning/architect/task_plan.md`.
//!
//! # This is the primary protocol, not an extension of rigctl
//!
//! `cat-rigctl` remains exactly what its name says — a compatibility layer
//! on its own port with its own unchanged wire behaviour, so WSJT-X and
//! stock `rigctl` are unaffected. This protocol owes it nothing and is
//! free to expose things Hamlib's fixed vocabulary cannot express.
//!
//! # Two channels, one connection
//!
//! Control traffic is JSON. Spectrum frames are **not**: a 2048-bin frame
//! at 60 fps is 120 000 floats a second, and putting that through a
//! serializer would make the protocol's cost scale with a feature many
//! clients do not want. They are separately framed binary, on their own
//! frame kind, and **a client that does not ask for them never receives a
//! single byte of them**. That is asserted, not merely intended — see
//! `a_client_that_declines_spectrum_receives_no_frame_traffic`.
//!
//! # Versioned from the first commit
//!
//! [`PROTOCOL_VERSION`] is in the handshake. A protocol that adds
//! versioning later has to guess what the unversioned peers were.

pub mod client;
pub mod server;
pub mod testing;

pub use client::{Client, ClientError, CommandSink, Connection, Event, Streams};
pub use server::{serve, serve_at, ConsoleId, HostRefusal, RadioHost};

// The vocabulary this protocol's own types are written in.
//
// `CapabilitiesWire` embeds `ModeId`, `MeterKind`, `RawRange` and friends
// rather than parallel copies, deliberately: one vocabulary for a fact
// whether it came off a socket or out of a `const`. The consequence is
// that a client cannot read what it receives without naming those types --
// so they are re-exported here. A client crate that makes a caller add a
// second dependency to understand its own return values is badly factored,
// and `ts570d` ADR 0008 forbids the GUI from taking that dependency
// directly at all.
pub use cat_framework::capabilities::{
    BreakInKind, CwAbort, CwPacing, EndpointRole, FrequencyRange, HzRange, MemoryCapability,
    MenuCapability, MeterKind, ModeId, ModeKind, RawRange, SUnitScale, Sideband, SignalSupport,
    VfoCapability, WpmRange, S_UNIT_LABELS,
};
pub use cat_framework::installation::{
    AudioOrigin, Installation, InstalledSource, Session, SourceState,
};
pub use cat_signal::{SignalCapability, SpectrumFrame};

use cat_framework::capabilities::*;
use serde::{Deserialize, Serialize};

/// Wire protocol version, sent in every [`ClientMessage::Hello`] and
/// [`ServerMessage::Welcome`].
///
/// # Bump this whenever a message's shape changes
///
/// A mismatch is refused at the handshake with a sentence naming both
/// versions, which is a good error. Leaving this alone while changing a
/// payload gets the *bad* error instead: the handshake passes, and the
/// decode fails later somewhere inside a nested type.
///
/// That happened on 2026-09-09. `SUnitScale` grew from 13 thresholds to
/// 16 so the labels could reach S9+60, and the version stayed at 1, so a
/// console built an hour earlier connected happily and then died with
/// `undecodable message: invalid length 16, expected 13 elements in
/// sequence` -- a serde-internal complaint that names neither the field,
/// the type, nor the fix, in front of an operator who had done nothing
/// wrong. Version 2 is that change.
///
/// `the_capability_wire_shape_is_pinned_to_the_protocol_version` fails if
/// the shape moves without this constant moving, so the coupling does not
/// depend on anyone remembering it.
pub const PROTOCOL_VERSION: u16 = 2;

/// Longest text one [`Command::SendCwText`] may carry (ADR 0023 §3.5).
pub const MAX_CW_TEXT_CHARS: usize = 256;

/// The ceiling on one CW send's key-down time, in seconds (ADR 0023
/// decision 5). [`Command::ArmCwTransmit`]'s `key_down_cap_s` may lower
/// it for an arm, never raise it.
pub const MAX_KEY_DOWN_CAP_S: u16 = 60;

/// Largest control payload accepted, to bound what a peer can make the
/// other side allocate before it has proved anything.
pub const MAX_CONTROL_BYTES: usize = 1 << 20;

// ---------------------------------------------------------------------------
// Framing
// ---------------------------------------------------------------------------

/// What a frame carries. One byte on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameKind {
    /// A JSON [`ClientMessage`] or [`ServerMessage`].
    Control = 1,
    /// A binary spectrum frame.
    Spectrum = 2,
    /// A binary audio frame: a scope trace and an AF spectrum, from the
    /// same samples.
    ///
    /// A separate kind from `Spectrum` because the two are not
    /// interchangeable and must never be drawn on the same axis. A band
    /// panorama spans tens of kilohertz around a dial; this spans a few
    /// kilohertz from zero. `cat-signal` enforces that with two types, and
    /// so does this.
    Audio = 3,
}

impl FrameKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(FrameKind::Control),
            2 => Some(FrameKind::Spectrum),
            3 => Some(FrameKind::Audio),
            _ => None,
        }
    }
}

/// Why a frame could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// Not enough bytes yet. Not an error — read more and retry.
    Incomplete,
    UnknownKind(u8),
    TooLarge(usize),
}

/// Encode one frame: `[kind: u8][len: u32 BE][payload]`.
pub fn encode_frame(kind: FrameKind, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(kind as u8);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Decode one frame from the front of `buf`.
///
/// Returns the frame and how many bytes it consumed, so a caller can drive
/// this over a stream without the decoder owning the buffer.
pub fn decode_frame(buf: &[u8]) -> Result<(FrameKind, &[u8], usize), FrameError> {
    if buf.len() < 5 {
        return Err(FrameError::Incomplete);
    }
    let kind = FrameKind::from_u8(buf[0]).ok_or(FrameError::UnknownKind(buf[0]))?;
    let len = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
    if kind == FrameKind::Control && len > MAX_CONTROL_BYTES {
        return Err(FrameError::TooLarge(len));
    }
    if buf.len() < 5 + len {
        return Err(FrameError::Incomplete);
    }
    Ok((kind, &buf[5..5 + len], 5 + len))
}

// ---------------------------------------------------------------------------
// Capabilities, in owned form
// ---------------------------------------------------------------------------

/// [`RadioCapabilities`] as a client receives it.
///
/// A separate owned type rather than a `Deserialize` on the original, for
/// a structural reason: `RadioCapabilities` is `Copy` and built from
/// `&'static` data so a radio can declare it as a `const` and the
/// handshake can cost no round trip (ADR 0010 §1). `&'static str` can be
/// serialized but cannot be deserialized — the bytes arriving on a socket
/// do not live forever. The server converts once per connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilitiesWire {
    pub model: String,
    pub endpoints: Vec<EndpointWire>,
    pub vfos: VfoCapability,
    pub modes: Vec<ModeWire>,
    pub tuning_steps_hz: Vec<u32>,
    pub rx_range: FrequencyRange,
    pub filters: FilterWire,
    pub meters: Vec<MeterDescriptorWire>,
    pub memory: Option<MemoryCapability>,
    pub menu: Option<MenuCapability>,
    /// What the radio *model* can accept as a spectrum source.
    pub signal: SignalSupport,
    /// How this radio's console should be arranged.
    ///
    /// Authored by the server, because the server is what knows the rig.
    /// A capability set says what a radio *can do*; this says what to make
    /// of that on screen — and the two are not the same judgement. A
    /// TS-570D with an SDR on its IF tap wants a waterfall dominating the
    /// display; an FT-991A has no spectrum at all and 151 menu items, and
    /// wants that space given to what it does have.
    ///
    /// `None` means the server has no opinion, and a console falls back to
    /// its own default arrangement. That is the honest reading of silence:
    /// an older server has not declined a layout, it has never been asked.
    #[serde(default)]
    pub layout: Option<cat_layout::LayoutSpec>,
    /// What this radio's console should look like.
    ///
    /// The other half of the layout, and the radio's for the same reason.
    /// An operator sitting in front of a TS-570D is looking at an amber
    /// LCD on a charcoal panel; an FT-991A is a colour TFT, blue and
    /// white. A console that matches its rig is one whose readout can be
    /// found without translating between two visual languages.
    ///
    /// The palette only. Structure, type scale and spacing stay the design
    /// system's, so a radio cannot restyle a component into something
    /// another radio's operator would not recognise.
    #[serde(default)]
    pub theme: Option<cat_layout::Theme>,
    /// What this bench actually has wired.
    ///
    /// The two are separate because they answer different questions and
    /// change at different times (ADR 0015). A client asking "should I draw
    /// a waterfall?" reads the installation; a client asking "could this
    /// radio ever have one?" reads the model.
    pub installation: Installation,
    /// CW control over CAT (ADR 0023). `None` both for a radio with none
    /// and for a pre-0023 server, and a client must treat the two alike:
    /// never probe with a CW command to find out.
    #[serde(default)]
    pub cw: Option<CwCapabilityWire>,
}

/// [`CwCapability`] as a client receives it: owned, for the same reason as
/// [`CapabilitiesWire`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CwCapabilityWire {
    pub pitch: Option<HzRange>,
    pub pitch_writable: bool,
    pub keyer_wpm: Option<WpmRange>,
    pub keyer_wpm_writable: bool,
    pub break_in: Vec<BreakInKind>,
    /// `None`: CW is read-only on this radio.
    pub send_text: Option<CwSendTextWire>,
    pub tx_gate: Option<TxGateWire>,
}

/// [`CwSendText`], owned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CwSendTextWire {
    pub max_chunk_chars: u16,
    /// Upper case; a space is always allowed besides these.
    pub charset: String,
    pub pacing: CwPacing,
    pub abort: CwAbort,
    pub requires_break_in: Option<BreakInKind>,
}

/// [`TxGateCapability`], owned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TxGateWire {
    pub low_power_raw: u16,
    pub low_power_label: String,
    pub power_raw: RawRange,
}

impl From<&CwCapability> for CwCapabilityWire {
    fn from(c: &CwCapability) -> Self {
        Self {
            pitch: c.pitch,
            pitch_writable: c.pitch_writable,
            keyer_wpm: c.keyer_wpm,
            keyer_wpm_writable: c.keyer_wpm_writable,
            break_in: c.break_in.to_vec(),
            send_text: c.send_text.map(|t| CwSendTextWire {
                max_chunk_chars: t.max_chunk_chars,
                charset: t.charset.to_string(),
                pacing: t.pacing,
                abort: t.abort,
                requires_break_in: t.requires_break_in,
            }),
            tx_gate: c.tx_gate.map(|g| TxGateWire {
                low_power_raw: g.low_power_raw,
                low_power_label: g.low_power_label.to_string(),
                power_raw: g.power_raw,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EndpointWire {
    pub role: EndpointRole,
    pub required: bool,
    pub shareable_with: Vec<EndpointRole>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeWire {
    pub id: ModeId,
    pub label: String,
    pub kind: ModeKind,
    pub sideband: Option<Sideband>,
    pub default_bandwidth_hz: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterWire {
    pub if_shift_hz: Option<i32>,
    pub widths_hz: Option<Vec<u32>>,
    pub notch: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeterDescriptorWire {
    pub kind: MeterKind,
    pub raw_range: RawRange,
    pub active_on_transmit: bool,
    /// Carried across the wire, unlike the other `&'static` fields here,
    /// because [`SUnitScale`] is a fixed-size array rather than a slice —
    /// it needs no owned mirror. A remote console reads the same S-units
    /// as a local one, which is the point.
    pub s_units: Option<SUnitScale>,
}

impl From<&RadioCapabilities> for CapabilitiesWire {
    fn from(c: &RadioCapabilities) -> Self {
        Self {
            model: c.model.to_string(),
            endpoints: c
                .endpoints
                .endpoints
                .iter()
                .map(|e| EndpointWire {
                    role: e.role,
                    required: e.required,
                    shareable_with: e.shareable_with.to_vec(),
                })
                .collect(),
            vfos: c.vfos,
            modes: c
                .modes
                .iter()
                .map(|m| ModeWire {
                    id: m.id,
                    label: m.label.to_string(),
                    kind: m.kind,
                    sideband: m.sideband,
                    default_bandwidth_hz: m.default_bandwidth_hz,
                })
                .collect(),
            tuning_steps_hz: c.tuning_steps_hz.to_vec(),
            rx_range: c.rx_range,
            filters: FilterWire {
                if_shift_hz: c.filters.if_shift_hz,
                widths_hz: c.filters.widths_hz.map(<[u32]>::to_vec),
                notch: c.filters.notch,
            },
            meters: c
                .meters
                .meters
                .iter()
                .map(|m| MeterDescriptorWire {
                    kind: m.kind,
                    raw_range: m.raw_range,
                    active_on_transmit: m.active_on_transmit,
                    s_units: m.s_units,
                })
                .collect(),
            memory: c.memory,
            menu: c.menu,
            signal: c.signal,
            // Filled by the caller: a `RadioCapabilities` alone cannot know
            // what is plugged into the radio it describes.
            // No layout from a bare capability set: the arrangement is
            // the *server's* to author, and a `RadioCapabilities` is the
            // model's declaration of itself. A server that wants one sets
            // it after converting.
            layout: None,
            theme: None,
            installation: Installation::default(),
            cw: c.cw.as_ref().map(CwCapabilityWire::from),
        }
    }
}

impl CapabilitiesWire {
    /// Publish a model and a bench together.
    pub fn from_session(session: &cat_framework::installation::Session) -> Self {
        Self {
            installation: session.installation.clone(),
            ..Self::from(session.radio)
        }
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Typed commands, validated against the capability set before dispatch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    SetFrequency {
        vfo: u8,
        hz: u64,
    },
    SetMode {
        mode: ModeId,
    },
    SetSplit {
        enabled: bool,
    },
    SetMemoryChannel {
        channel: u16,
    },
    SetFilterWidth {
        hz: u32,
    },
    SetIfShift {
        hz: i32,
    },
    ReadMeter {
        kind: MeterKind,
    },
    /// Move the dial, and with it any IF-tap spectrum source.
    Retune {
        hz: u64,
    },
    /// What signal hardware the *radio's host* can see.
    ///
    /// The command exists because a console is not usually on the same
    /// machine as the radio, and its own sound cards are not the radio's.
    /// A picker that enumerated locally would let an operator on a laptop
    /// select their laptop microphone as the radio's receive audio -- it
    /// would look entirely correct and be wrong.
    ///
    /// Distinct from [`crate::CapabilitiesWire::installation`], which says
    /// what is *already wired and streaming*. This says what *could* be
    /// attached. A console needs both: one to draw the panels it has, one
    /// to offer the panels it could have.
    ReadDevices,
    /// Attach one of the devices [`Command::ReadDevices`] offered.
    ///
    /// `spec` is passed back verbatim from a [`cat_signal::DeviceInfo`],
    /// never composed by the client: sound-card and SDR specs are a host's
    /// own namespace, and a client that built one would be guessing about
    /// a machine it cannot see.
    AttachDevice {
        kind: cat_signal::DeviceKind,
        spec: String,
    },
    /// Report everything the console displays, in one round trip.
    ///
    /// One command rather than a field-per-command set. A console needs
    /// the dial, the mode and the meters *together* — showing a frequency
    /// from one moment beside a mode from another is how a readout ends up
    /// describing a radio that never existed.
    ReadState,
    /// Set the radio's internal keyer speed (ADR 0023 §2).
    SetKeyerSpeed {
        wpm: u16,
    },
    /// Take the CW transmit arm: the server clamps power, reads it back
    /// and only then acknowledges (ADR 0023 §3). One owner at a time.
    ArmCwTransmit {
        callsign: String,
        /// The operator's attestation, asked afresh every session. The
        /// server records it; it cannot verify it.
        dummy_load_acknowledged: bool,
        /// Lower the per-send key-down cap for this arm, in seconds. The
        /// server's cap (60 s) is a ceiling: a larger value is ignored.
        #[serde(default)]
        key_down_cap_s: Option<u16>,
    },
    /// Queue text for the radio's keyer. Arm owner only. `Ack` means
    /// queued; progress rides [`CwState::sender`].
    SendCwText {
        text: String,
    },
    /// Stop any CW send now. Any console, armed or not; never refused
    /// once CW control exists on the server.
    AbortCw,
    /// Release the arm: stop, restore power and break-in, read back.
    DisarmCw,
}

/// One meter's current reading.
///
/// The raw value travels with the kind, and the *range* stays in
/// [`CapabilitiesWire`] where it was published once at handshake. Sending
/// the range with every sample would be repeating a constant sixty times a
/// second; sending the raw value without a way to reach its range would be
/// the bug `MeterReading` exists to prevent. The pairing happens at the
/// console, from data it already has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeterSample {
    pub kind: MeterKind,
    pub raw: u16,
}

/// The settings an operator changes occasionally.
///
/// Polled on a slower clock than the dial: a frequency moves constantly
/// and is worth reading several times a second, while an AF gain is worth
/// reading every few seconds. Reading all fourteen at the fast rate would
/// be most of a 9600-baud link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RadioLevels {
    pub af_gain: u8,
    pub rf_gain: u8,
    pub squelch: u8,
    pub mic_gain: u8,
    pub power_pct: u8,
    pub agc: u8,
    pub noise_reduction: u8,
    pub antenna: u8,
    pub noise_blanker: bool,
    pub preamp: bool,
    pub attenuator: bool,
    pub speech_processor: bool,
    pub vox: bool,
    pub freq_lock: bool,
}

/// What the radio is doing right now.
///
/// Every field is `Option` except the ones every radio has. A radio with
/// no memory reports `memory_channel: None`, and a console must not be
/// able to tell that apart from "channel 0" — which is a real channel on a
/// TS-570D.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RadioState {
    pub vfo_a_hz: u64,
    pub vfo_b_hz: u64,
    pub mode: ModeId,
    pub split: bool,
    /// `true` when the radio is transmitting, by any cause — including a
    /// front-panel PTT this session knows nothing about.
    pub transmitting: bool,
    pub memory_channel: Option<u16>,
    pub if_shift_hz: Option<i32>,
    pub filter_width_hz: Option<u32>,
    /// Every meter the radio currently reports. A meter that is inert —
    /// an SWR meter during receive — may be absent or read zero; which of
    /// those it does is the radio's business, not the protocol's.
    pub meters: Vec<MeterSample>,
    /// The settings an operator changes occasionally, if the server has
    /// read them.
    ///
    /// Separate from the fields above because they are polled on a
    /// different clock: a dial moves constantly and is worth reading five
    /// times a second, while an AF gain is worth reading every few
    /// seconds. Fourteen commands at the fast rate would be most of a
    /// 9600-baud link.
    ///
    /// `None` means nobody has read them, and a console must say so
    /// rather than draw its struct defaults -- which is what made a
    /// network console display `AF 200` at a radio reading `AG034`.
    ///
    /// `#[serde(default)]` so a console built before this field existed,
    /// or a server that does not implement the read, still speaks the
    /// protocol.
    #[serde(default)]
    pub levels: Option<RadioLevels>,
    /// CW settings and the transmit session, when the radio has CW
    /// control (ADR 0023 §2). `None` from a pre-0023 server.
    #[serde(default)]
    pub cw: Option<CwState>,
}

/// CW as it stands now.
///
/// The settings are CAT reads on the levels clock; `armed` and `sender`
/// are the server's own bookkeeping, so they cost no CAT exchange.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CwState {
    pub pitch_hz: Option<u32>,
    pub keyer_wpm: Option<u16>,
    pub break_in: Option<BreakInKind>,
    /// Present only while some console holds the transmit arm.
    #[serde(default)]
    pub armed: Option<ArmedState>,
    #[serde(default)]
    pub sender: SenderState,
}

/// Who holds the CW transmit arm, and what the radio confirmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArmedState {
    pub callsign: String,
    /// Power read back from the radio after the clamp, raw. Its label is
    /// `CwCapabilityWire::tx_gate`'s.
    pub power_raw: u16,
    /// Whether the connection reading this owns the arm. Filled per
    /// connection by the server.
    pub mine: bool,
}

/// What the server's CW sender is doing.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SenderState {
    #[default]
    Idle,
    /// `total_chars` includes any `DE <call>` the server appended.
    Sending {
        job: u32,
        sent_chars: u32,
        total_chars: u32,
    },
    /// A `DrainOnly` radio finishing the chunk it holds after an abort.
    Draining { job: u32 },
    Stopped {
        job: u32,
        reason: StopReason,
        #[serde(default)]
        message: Option<String>,
    },
}

/// Why a CW job ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Completed,
    Aborted,
    /// The job outlived its key-down cap and the watchdog stopped it.
    KeyDownCap,
    /// The arm owner's connection went away.
    Disconnected,
    /// The radio refused or misreported something (see `message`).
    Error,
}

impl RadioState {
    /// The reading for one meter, if the radio reported it.
    pub fn meter(&self, kind: MeterKind) -> Option<u16> {
        self.meters.iter().find(|m| m.kind == kind).map(|m| m.raw)
    }
}

/// The interval [`MAX_FPS`] works out to.
pub const MAX_FPS_INTERVAL: std::time::Duration =
    std::time::Duration::from_micros(1_000_000 / MAX_FPS as u64);

/// The slowest frame rate worth streaming at.
pub const MIN_FPS: u32 = 1;

/// The fastest. Past this the link is being spent on frames nobody's eye
/// separates, and the value arrives from the network, so it is bounded.
pub const MAX_FPS: u32 = 60;

/// How often spectrum and audio frames go to a client that has not said
/// what it can render.
///
/// Deliberately slower than the server's command pump, which governs how
/// quickly a client's commands are noticed and must stay brisk.
///
/// At the pump rate the server pushed about 29 spectrum and 24 audio
/// frames a second -- roughly 345 KB/s -- and a console that renders each
/// one into a terminal cannot keep up. Measured on a TS-570D: the TUI at
/// 56% CPU with 31 KB backed up unread in its socket, which presents as
/// the console hanging. Ten a second is a waterfall that still reads as
/// live and an AF scope nobody can tell apart from thirty.
///
/// This was the rate for *everybody* for a while, and it was the wrong
/// one for a GPU console, where ten a second is a fall visibly stepping.
/// There is no single number that suits both, so a client that knows what
/// it can draw says so and gets it; this is what the rest get.
pub const DEFAULT_FRAME_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello {
        version: u16,
        /// Whether to send this client spectrum frames at all.
        #[serde(default)]
        spectrum: bool,
        /// Whether to send this client audio frames at all.
        ///
        /// `#[serde(default)]`, so a client built before audio existed
        /// asks for none and is sent none. The two streams are opted into
        /// separately because they cost differently and a console may
        /// well want one without the other -- a waterfall with no AF
        /// panels is an ordinary way to run.
        #[serde(default)]
        audio: bool,
        /// How many spectrum and audio frames a second this client can
        /// actually render, if it knows.
        ///
        /// The server used to push at one global rate for everybody, and
        /// there is no rate that suits both consoles. At 29 a second a
        /// terminal console fell behind -- measured on a TS-570D at 56%
        /// CPU with 31 KB backed up unread in its socket, which presents
        /// as the console hanging -- while a GPU console at 10 a second
        /// is a waterfall visibly stepping.
        ///
        /// So the client says. `#[serde(default)]` leaves it `None` for a
        /// client built before this existed, and `None` keeps the old
        /// conservative rate, which is the right answer for a console
        /// that has not told us what it can take.
        #[serde(default)]
        max_fps: Option<u8>,
    },
    Command(Command),
    Ping,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Welcome {
        version: u16,
        capabilities: Box<CapabilitiesWire>,
    },
    Ack,
    Pong,
    /// The answer to [`Command::ReadState`].
    State(Box<RadioState>),
    /// The answer to [`Command::ReadMeter`].
    Meter(MeterSample),
    /// The answer to [`Command::ReadDevices`].
    ///
    /// One list per kind, each carrying its own three-way outcome: found
    /// some, found none, or could not ask. Flattening those into one list
    /// would lose the distinction the whole type exists for.
    ///
    /// A struct variant, not a newtype around the `Vec`. `ServerMessage`
    /// is internally tagged, and serde cannot tag a newtype variant whose
    /// contents serialize as a sequence -- it fails at *serialization*,
    /// so the symptom is the server dropping the connection rather than
    /// anything naming the real problem.
    Devices {
        lists: Vec<cat_signal::DeviceList>,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
}

/// Why a command was refused.
///
/// A code as well as a message, so a client can react programmatically
/// without matching on English.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The client spoke before saying hello.
    NotReady,
    VersionMismatch,
    /// The radio does not have this capability at all.
    Unsupported,
    /// It has the capability, but not with this value.
    OutOfRange,
    Malformed,
    /// "You may not, now": not armed, armed by someone else, no dummy-load
    /// acknowledgement (ADR 0023). Only ever sent in reply to a CW
    /// command, which a pre-0023 client never sends.
    Forbidden,
}

// ---------------------------------------------------------------------------
// The session
// ---------------------------------------------------------------------------

/// One client connection's protocol state.
///
/// Deliberately free of I/O: it takes decoded messages and returns
/// messages to send. That makes the whole protocol — handshake ordering,
/// capability validation, spectrum gating — testable without a socket, on
/// every platform, which is the same reasoning that made `cat-ui` and
/// `cat-signal`'s DSP separable from their hardware.
pub struct NativeSession {
    capabilities: &'static RadioCapabilities,
    installation: Installation,
    handshaken: bool,
    spectrum: bool,
    audio: bool,
    /// What this client said it can render. See `ClientMessage::Hello`.
    max_fps: Option<u8>,
    /// The most recent state the host published.
    ///
    /// A cache, not a source. This session does not own a radio and must
    /// not learn how to talk to one — staying I/O-free is what makes the
    /// whole protocol testable without a socket, on every platform. The
    /// host polls at whatever rate it polls and pushes the answer in with
    /// [`NativeSession::publish_state`].
    ///
    /// `None` means nothing has been published yet, which is a real state
    /// rather than an error: a server that has just started has not heard
    /// from its radio either.
    state: Option<RadioState>,
    /// The arrangement this radio's server published, if it published one.
    layout: Option<cat_layout::LayoutSpec>,
    /// The palette this radio's server published, if it published one.
    theme: Option<cat_layout::Theme>,
    /// What the host last said its machine can see, if it offers the
    /// question at all.
    ///
    /// `None` is not "no devices" -- it is "this server does not offer
    /// device selection", which is what a host that never calls
    /// [`NativeSession::publish_devices`] is saying. The distinction
    /// matters to a console: one means "plug something in", the other
    /// means "ask this server's operator, it cannot help you from here".
    devices: Option<Vec<cat_signal::DeviceList>>,
    /// Whether the host behind this session runs ADR 0023's CW supervisor.
    ///
    /// `false` until a host says otherwise, and while `false` this server
    /// offers no CW at all: `Welcome` carries `cw: None`, CW commands are
    /// `Unsupported`, and `AbortCw` is acknowledged without reaching the
    /// host. A command that could key a transmitter must never reach a
    /// host that cannot bound it.
    serves_cw: bool,
}

impl NativeSession {
    /// A session for a radio with nothing optional attached.
    pub fn new(capabilities: &'static RadioCapabilities) -> Self {
        Self::with_installation(capabilities, Installation::default())
    }

    /// A session for a radio on a particular bench.
    pub fn with_installation(
        capabilities: &'static RadioCapabilities,
        installation: Installation,
    ) -> Self {
        Self {
            capabilities,
            installation,
            handshaken: false,
            spectrum: false,
            audio: false,
            max_fps: None,
            state: None,
            devices: None,
            layout: None,
            theme: None,
            serves_cw: false,
        }
    }

    /// Say whether the host runs a CW supervisor. See the field.
    pub fn set_serves_cw(&mut self, serves_cw: bool) {
        self.serves_cw = serves_cw;
    }

    /// Whether an accepted `command` should be handed to the host.
    ///
    /// Narrower than [`NativeSession::validate`], never wider: the one
    /// command it holds back is an `AbortCw` to a host with no CW
    /// supervisor, which has nothing to stop and must not be asked.
    pub fn reaches_host(&self, command: &Command) -> bool {
        match command {
            Command::AbortCw => self.serves_cw && self.capabilities.cw.is_some(),
            _ => true,
        }
    }

    /// Publish the arrangement this radio's console should use.
    ///
    /// Set once per session, before the handshake, because that is when a
    /// console is told what the radio is and the layout is part of that
    /// answer.
    pub fn set_layout(&mut self, layout: Option<cat_layout::LayoutSpec>) {
        self.layout = layout;
    }

    /// Publish the palette this radio's console should use.
    pub fn set_theme(&mut self, theme: Option<cat_layout::Theme>) {
        self.theme = theme;
    }

    /// Publish what the radio is currently doing.
    ///
    /// Called by the host at whatever rate it polls. Cheap and idempotent:
    /// publishing the same state twice is not an event, and a host that
    /// polls faster than any client asks costs nothing but the clone.
    pub fn publish_state(&mut self, state: RadioState) {
        self.state = Some(state);
    }

    /// The last published state, if any.
    pub fn published_state(&self) -> Option<&RadioState> {
        self.state.as_ref()
    }

    /// Publish what signal hardware this machine can see.
    ///
    /// A host that never calls this is declining the question, and clients
    /// are told so rather than being handed an empty list -- see
    /// [`Command::ReadDevices`].
    pub fn publish_devices(&mut self, devices: Vec<cat_signal::DeviceList>) {
        self.devices = Some(devices);
    }

    /// The last published device list, if the host offers one.
    pub fn published_devices(&self) -> Option<&[cat_signal::DeviceList]> {
        self.devices.as_deref()
    }

    /// Whether this client asked for spectrum frames.
    ///
    /// The one question the frame pump asks. `false` until a successful
    /// `Hello` says otherwise, so a client that never handshakes cannot be
    /// sent frames either.
    /// How often this client should be sent frames.
    ///
    /// Clamped, because this arrives from the network: a client asking
    /// for 0 would divide by zero and one asking for 240 would be handed
    /// the link. Below the floor there is no point having the stream at
    /// all; above the ceiling nothing on the other end can draw it.
    pub fn frame_interval(&self) -> std::time::Duration {
        match self.max_fps {
            Some(fps) => {
                let fps = u32::from(fps).clamp(MIN_FPS, MAX_FPS);
                std::time::Duration::from_micros(u64::from(1_000_000 / fps))
            }
            // A console that has not said keeps the rate that was safe
            // for every console before it could.
            None => DEFAULT_FRAME_INTERVAL,
        }
    }

    pub fn wants_spectrum(&self) -> bool {
        self.handshaken && self.spectrum
    }

    /// Whether this client asked for audio frames.
    ///
    /// Separate from [`NativeSession::wants_spectrum`] and not implied by
    /// it: the two streams are opted into independently, and a console
    /// running a waterfall with no AF panels should not be sent audio it
    /// will throw away.
    pub fn wants_audio(&self) -> bool {
        self.audio
    }

    pub fn is_handshaken(&self) -> bool {
        self.handshaken
    }

    /// Handle one decoded client message.
    pub fn handle(&mut self, message: ClientMessage) -> ServerMessage {
        match message {
            ClientMessage::Hello {
                version,
                spectrum,
                audio,
                max_fps,
            } => {
                if version != PROTOCOL_VERSION {
                    return ServerMessage::Error {
                        code: ErrorCode::VersionMismatch,
                        message: format!(
                            "server speaks version {PROTOCOL_VERSION}, client offered {version}"
                        ),
                    };
                }
                self.handshaken = true;
                // A client only gets frames if it both handshook AND asked.
                self.spectrum = spectrum;
                self.audio = audio;
                self.max_fps = max_fps;
                ServerMessage::Welcome {
                    version: PROTOCOL_VERSION,
                    capabilities: Box::new(CapabilitiesWire {
                        layout: self.layout.clone(),
                        theme: self.theme,
                        installation: self.installation.clone(),
                        // A radio that can do CW behind a host that cannot
                        // supervise it offers none (ADR 0023).
                        cw: self
                            .capabilities
                            .cw
                            .as_ref()
                            .filter(|_| self.serves_cw)
                            .map(CwCapabilityWire::from),
                        ..CapabilitiesWire::from(self.capabilities)
                    }),
                }
            }
            _ if !self.handshaken => ServerMessage::Error {
                code: ErrorCode::NotReady,
                message: "send hello first".to_string(),
            },
            ClientMessage::Ping => ServerMessage::Pong,
            ClientMessage::Command(command) => match self.validate(&command) {
                Err(e) => e,
                // A read is answered here rather than acknowledged. An
                // `Ack` to a question is not an answer, and that was this
                // protocol's shape until a console tried to use it and
                // found it could send and could not see.
                Ok(()) => match &command {
                    Command::ReadState => match &self.state {
                        Some(state) => ServerMessage::State(Box::new(state.clone())),
                        None => ServerMessage::Error {
                            code: ErrorCode::NotReady,
                            message: "the server has not heard from the radio yet".to_string(),
                        },
                    },
                    Command::ReadMeter { kind } => {
                        match self.state.as_ref().and_then(|s| s.meter(*kind)) {
                            Some(raw) => ServerMessage::Meter(MeterSample { kind: *kind, raw }),
                            // The radio has this meter -- `validate`
                            // checked -- but is not reporting it now. A TX
                            // meter during receive is the ordinary case,
                            // and it is not an error.
                            None => ServerMessage::Error {
                                code: ErrorCode::NotReady,
                                message: format!("no current reading for the {kind:?} meter"),
                            },
                        }
                    }
                    Command::ReadDevices => match &self.devices {
                        Some(devices) => ServerMessage::Devices {
                            lists: devices.clone(),
                        },
                        // `Unsupported`, not an empty list. An empty list
                        // would be a claim about this machine's hardware
                        // that a server declining the question has not
                        // made and cannot support.
                        None => ServerMessage::Error {
                            code: ErrorCode::Unsupported,
                            message: "this server does not offer device selection".to_string(),
                        },
                    },
                    _ => ServerMessage::Ack,
                },
            },
        }
    }

    /// Check a command against what the radio can actually do.
    ///
    /// This is the payoff of the capability model: the server rejects
    /// impossible commands *before* they reach the radio, with a reason,
    /// rather than forwarding them and interpreting whatever the radio
    /// says back. Nothing here knows which radio it is.
    pub fn validate(&self, command: &Command) -> Result<(), ServerMessage> {
        let caps = self.capabilities;
        match command {
            Command::SetFrequency { vfo, hz } => {
                if *vfo >= caps.vfos.count {
                    return Err(unsupported(format!(
                        "radio has {} VFOs; asked for index {vfo}",
                        caps.vfos.count
                    )));
                }
                if !caps.rx_range.contains(*hz) {
                    return Err(out_of_range(format!(
                        "{hz} Hz is outside {}-{} Hz",
                        caps.rx_range.min_hz, caps.rx_range.max_hz
                    )));
                }
                Ok(())
            }
            Command::Retune { hz } => {
                if !caps.rx_range.contains(*hz) {
                    return Err(out_of_range(format!(
                        "{hz} Hz is outside this radio's range"
                    )));
                }
                Ok(())
            }
            Command::SetMode { mode } => {
                if !caps.supports_mode(*mode) {
                    return Err(unsupported(format!(
                        "{mode:?} is not a mode this radio has"
                    )));
                }
                Ok(())
            }
            Command::SetSplit { .. } => {
                if !caps.vfos.split {
                    return Err(unsupported("this radio has no split".to_string()));
                }
                Ok(())
            }
            Command::SetMemoryChannel { channel } => {
                let Some(memory) = caps.memory else {
                    return Err(unsupported("this radio has no memory channels".to_string()));
                };
                if *channel < memory.channels.min || *channel > memory.channels.max {
                    return Err(out_of_range(format!(
                        "channel {channel} is outside {}-{}",
                        memory.channels.min, memory.channels.max
                    )));
                }
                Ok(())
            }
            Command::SetFilterWidth { hz } => {
                let Some(widths) = caps.filters.widths_hz else {
                    return Err(unsupported(
                        "this radio exposes no selectable filter widths".to_string(),
                    ));
                };
                if !widths.contains(hz) {
                    return Err(out_of_range(format!(
                        "{hz} Hz is not one of this radio's widths"
                    )));
                }
                Ok(())
            }
            Command::SetIfShift { hz } => {
                let Some(limit) = caps.filters.if_shift_hz else {
                    return Err(unsupported("this radio has no IF shift".to_string()));
                };
                if hz.abs() > limit {
                    return Err(out_of_range(format!("IF shift limit is +/-{limit} Hz")));
                }
                Ok(())
            }
            Command::ReadMeter { kind } => {
                if !caps.meters.has(*kind) {
                    return Err(unsupported(format!("this radio has no {kind:?} meter")));
                }
                Ok(())
            }
            // Every radio has a state. Nothing to validate against.
            Command::ReadState => Ok(()),
            // Nothing in the capability set describes a host's sound
            // cards -- capabilities describe the radio *model*, and these
            // are facts about one machine. The host answers, or declines.
            Command::ReadDevices | Command::AttachDevice { .. } => Ok(()),
            // Never refused: stopping a transmitter is always allowed,
            // armed or not, by anyone. `reaches_host` decides whether
            // there is anything to stop.
            Command::AbortCw => Ok(()),
            Command::SetKeyerSpeed { .. }
            | Command::ArmCwTransmit { .. }
            | Command::SendCwText { .. }
            | Command::DisarmCw => self.validate_cw(command),
        }
    }

    /// ADR 0023's per-connection rules: everything decidable from the
    /// capability set and the command alone. Who owns the arm, the mode,
    /// the power read-back and the ID are the server supervisor's.
    fn validate_cw(&self, command: &Command) -> Result<(), ServerMessage> {
        let cw = match self.capabilities.cw {
            Some(cw) if self.serves_cw => cw,
            _ => {
                return Err(unsupported(
                    "this server does not offer CW control".to_string(),
                ))
            }
        };
        if let Command::SetKeyerSpeed { wpm } = command {
            let range = match cw.keyer_wpm {
                Some(range) if cw.keyer_wpm_writable => range,
                _ => {
                    return Err(unsupported(
                        "this radio's keyer speed cannot be set over CAT".to_string(),
                    ))
                }
            };
            if !range.contains(*wpm) {
                return Err(out_of_range(format!(
                    "{wpm} wpm is outside {}-{} wpm",
                    range.min, range.max
                )));
            }
            return Ok(());
        }
        let Some(send) = cw.send_text else {
            return Err(unsupported(
                "this radio cannot send CW text over CAT".to_string(),
            ));
        };
        match command {
            Command::ArmCwTransmit {
                callsign,
                dummy_load_acknowledged,
                key_down_cap_s,
            } => {
                if !dummy_load_acknowledged {
                    return Err(forbidden(
                        "arming needs the operator's dummy-load acknowledgement".to_string(),
                    ));
                }
                if !is_callsign(callsign) {
                    return Err(forbidden(format!(
                        "{callsign:?} is not a callsign: 3-10 of A-Z 0-9 /, \
                         with at least one letter and one digit"
                    )));
                }
                if let Some(c) = send.rejected_char(callsign) {
                    return Err(forbidden(format!(
                        "this keyer cannot send {c:?}, so {callsign} could not be identified"
                    )));
                }
                if *key_down_cap_s == Some(0) {
                    return Err(out_of_range(
                        "a key-down cap of 0 s would refuse every send".to_string(),
                    ));
                }
                Ok(())
            }
            Command::SendCwText { text } => {
                if text.trim().is_empty() {
                    return Err(out_of_range("nothing to send".to_string()));
                }
                let len = text.chars().count();
                if len > MAX_CW_TEXT_CHARS {
                    return Err(out_of_range(format!(
                        "{len} characters; one send is at most {MAX_CW_TEXT_CHARS}"
                    )));
                }
                if let Some(c) = send.rejected_char(text) {
                    return Err(out_of_range(format!("this keyer cannot send {c:?}")));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Encode a spectrum frame for the wire, or `None` if this client
    /// declined them.
    ///
    /// Returning `None` rather than an empty frame is deliberate: the
    /// caller must be unable to accidentally send a zero-length spectrum
    /// frame to a client that asked for silence.
    pub fn encode_spectrum(&self, frame: &cat_signal::SpectrumFrame) -> Option<Vec<u8>> {
        if !self.wants_spectrum() {
            return None;
        }
        Some(encode_frame(
            FrameKind::Spectrum,
            &encode_spectrum_payload(frame),
        ))
    }
}

fn forbidden(message: String) -> ServerMessage {
    ServerMessage::Error {
        code: ErrorCode::Forbidden,
        message,
    }
}

/// 3-10 of `[A-Za-z0-9/]`, with at least one letter and one digit.
fn is_callsign(call: &str) -> bool {
    (3..=10).contains(&call.len())
        && call.chars().all(|c| c.is_ascii_alphanumeric() || c == '/')
        && call.chars().any(|c| c.is_ascii_alphabetic())
        && call.chars().any(|c| c.is_ascii_digit())
}

fn unsupported(message: String) -> ServerMessage {
    ServerMessage::Error {
        code: ErrorCode::Unsupported,
        message,
    }
}

fn out_of_range(message: String) -> ServerMessage {
    ServerMessage::Error {
        code: ErrorCode::OutOfRange,
        message,
    }
}

/// Binary spectrum payload.
///
/// `[center_hz: u64][span_hz: u32][ref_level_dbm: f32][sequence: u64][bin_count: u32][bins: f32...]`,
/// all big-endian. Not JSON: at 2048 bins and 60 fps this is 120 000
/// floats a second, and the cost of a serializer there is not a
/// micro-optimization.
///
/// **Bin order is preserved exactly** — low frequency first, as ADR 0010
/// requires. A transport that reversed them would be as wrong as a source
/// that did.
pub fn encode_spectrum_payload(frame: &cat_signal::SpectrumFrame) -> Vec<u8> {
    let mut out = Vec::with_capacity(28 + frame.bins.len() * 4);
    out.extend_from_slice(&frame.center_hz.to_be_bytes());
    out.extend_from_slice(&frame.span_hz.to_be_bytes());
    out.extend_from_slice(&frame.ref_level_dbm.to_be_bytes());
    out.extend_from_slice(&frame.sequence.to_be_bytes());
    out.extend_from_slice(&(frame.bins.len() as u32).to_be_bytes());
    for bin in &frame.bins {
        out.extend_from_slice(&bin.to_be_bytes());
    }
    out
}

/// Decode a binary spectrum payload.
pub fn decode_spectrum_payload(payload: &[u8]) -> Option<cat_signal::SpectrumFrame> {
    if payload.len() < 28 {
        return None;
    }
    let center_hz = u64::from_be_bytes(payload[0..8].try_into().ok()?);
    let span_hz = u32::from_be_bytes(payload[8..12].try_into().ok()?);
    let ref_level_dbm = f32::from_be_bytes(payload[12..16].try_into().ok()?);
    let sequence = u64::from_be_bytes(payload[16..24].try_into().ok()?);
    let count = u32::from_be_bytes(payload[24..28].try_into().ok()?) as usize;
    if payload.len() < 28 + count * 4 {
        return None;
    }
    let bins = payload[28..28 + count * 4]
        .chunks_exact(4)
        .map(|c| f32::from_be_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    Some(cat_signal::SpectrumFrame {
        center_hz,
        span_hz,
        ref_level_dbm,
        bins,
        sequence,
    })
}

/// Encode an audio frame: scope and spectrum together, one payload.
///
/// One payload rather than two frame kinds, because the two halves come
/// from the same samples and share a sequence number. Splitting them would
/// let a console draw a trace from one block beside a spectrum from
/// another — exactly what `AudioFrame` exists to make impossible.
pub fn encode_audio_payload(frame: &cat_signal::AudioFrame) -> Vec<u8> {
    let scope = &frame.scope;
    let spectrum = &frame.spectrum;
    let mut out = Vec::with_capacity(28 + scope.samples.len() * 4 + spectrum.bins.len() * 4);
    out.extend_from_slice(&scope.sequence.to_be_bytes());
    out.extend_from_slice(&scope.sample_rate_hz.to_be_bytes());
    out.extend_from_slice(&spectrum.start_hz.to_be_bytes());
    out.extend_from_slice(&spectrum.span_hz.to_be_bytes());
    out.extend_from_slice(&(scope.samples.len() as u32).to_be_bytes());
    out.extend_from_slice(&(spectrum.bins.len() as u32).to_be_bytes());
    for sample in &scope.samples {
        out.extend_from_slice(&sample.to_be_bytes());
    }
    for bin in &spectrum.bins {
        out.extend_from_slice(&bin.to_be_bytes());
    }
    out
}

/// Decode what [`encode_audio_payload`] wrote.
///
/// `None` for anything that does not fit, rather than a partial frame: a
/// scope drawn from half a payload is a waveform the radio never produced.
pub fn decode_audio_payload(payload: &[u8]) -> Option<cat_signal::AudioFrame> {
    if payload.len() < 28 {
        return None;
    }
    let sequence = u64::from_be_bytes(payload[0..8].try_into().ok()?);
    let sample_rate_hz = u32::from_be_bytes(payload[8..12].try_into().ok()?);
    let start_hz = u32::from_be_bytes(payload[12..16].try_into().ok()?);
    let span_hz = u32::from_be_bytes(payload[16..20].try_into().ok()?);
    let samples = u32::from_be_bytes(payload[20..24].try_into().ok()?) as usize;
    let bins = u32::from_be_bytes(payload[24..28].try_into().ok()?) as usize;
    if payload.len() < 28 + samples * 4 + bins * 4 {
        return None;
    }
    let floats = |from: usize, count: usize| -> Vec<f32> {
        payload[from..from + count * 4]
            .chunks_exact(4)
            .map(|c| f32::from_be_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    };
    Some(cat_signal::AudioFrame {
        scope: cat_signal::AudioScopeFrame {
            sample_rate_hz,
            samples: floats(28, samples),
            sequence,
        },
        spectrum: cat_signal::AudioSpectrumFrame {
            start_hz,
            span_hz,
            bins: floats(28 + samples * 4, bins),
            // The shared number, deliberately: both halves came from the
            // same block, and a console checks one against the other.
            sequence,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_signal::SpectrumFrame;

    // A radio described purely for protocol tests. Not a real model: the
    // real ones are cat-framework's Task 13 fixtures, which are test-only
    // to that crate. What matters here is that every capability boundary
    // this protocol validates has a defined edge to test against.
    const MODES: &[ModeDescriptor] = &[
        ModeDescriptor {
            id: ModeId::Lsb,
            label: "LSB",
            kind: ModeKind::Ssb,
            sideband: Some(Sideband::Lower),
            default_bandwidth_hz: 2400,
        },
        ModeDescriptor {
            id: ModeId::Usb,
            label: "USB",
            kind: ModeKind::Ssb,
            sideband: Some(Sideband::Upper),
            default_bandwidth_hz: 2400,
        },
    ];
    const METERS: &[MeterDescriptor] = &[MeterDescriptor {
        kind: MeterKind::S,
        raw_range: RawRange::new(0, 30),
        active_on_transmit: false,
        s_units: None,
    }];
    const ENDPOINTS: &[EndpointDescriptor] = &[EndpointDescriptor {
        role: EndpointRole::Cat,
        required: true,
        shareable_with: &[EndpointRole::Keying],
    }];

    static RADIO: RadioCapabilities = RadioCapabilities {
        model: "Protocol Test Radio",
        endpoints: EndpointSet::new(ENDPOINTS),
        vfos: VfoCapability {
            count: 2,
            split: true,
            rit_hz: Some(9999),
            xit_hz: None,
        },
        modes: MODES,
        tuning_steps_hz: &[10, 100],
        rx_range: FrequencyRange::new(500_000, 60_000_000),
        filters: FilterCapability {
            if_shift_hz: Some(1_000),
            widths_hz: Some(&[500, 2_400]),
            notch: false,
        },
        meters: MeterSet::new(METERS),
        memory: Some(MemoryCapability {
            channels: RawRange::new(0, 99),
            named: false,
            stores_mode: true,
            scan: true,
        }),
        menu: None,
        // A MODEL fact: the radio has a CN4 tap point. Whether a dongle is
        // on it belongs to an `Installation` (ADR 0015).
        signal: SignalSupport::IfTapPoint {
            if_center_hz: 73_050_000,
            inverted: true,
        },
        cw: None,
    };

    static NO_EXTRAS: RadioCapabilities = RadioCapabilities {
        model: "Minimal Radio",
        endpoints: EndpointSet::new(ENDPOINTS),
        vfos: VfoCapability {
            count: 1,
            split: false,
            rit_hz: None,
            xit_hz: None,
        },
        modes: MODES,
        tuning_steps_hz: &[10],
        rx_range: FrequencyRange::new(500_000, 60_000_000),
        filters: FilterCapability {
            if_shift_hz: None,
            widths_hz: None,
            notch: false,
        },
        meters: MeterSet::new(METERS),
        memory: None,
        menu: None,
        signal: SignalSupport::None,
        cw: None,
    };

    #[test]
    fn a_client_that_says_nothing_keeps_the_conservative_rate() {
        // The terminal console does not ask, and must not be sped up
        // under it: at 29 frames a second it fell behind with 31 KB
        // backed up unread in its socket, which presents as a hang.
        let session = handshaken(true);
        assert_eq!(session.frame_interval(), DEFAULT_FRAME_INTERVAL);
    }

    #[test]
    fn a_client_gets_the_rate_it_asked_for() {
        let mut session = NativeSession::new(&RADIO);
        session.handle(ClientMessage::Hello {
            version: PROTOCOL_VERSION,
            spectrum: true,
            audio: true,
            max_fps: Some(30),
        });
        let interval = session.frame_interval();
        assert!(
            interval <= std::time::Duration::from_millis(34)
                && interval >= std::time::Duration::from_millis(33),
            "30 fps is a ~33 ms interval, got {interval:?}"
        );
    }

    #[test]
    fn an_absurd_rate_is_clamped_rather_than_believed() {
        // This arrives from the network. Zero would divide by zero and
        // 255 would hand a client the link.
        for (asked, expect_at_least, expect_at_most) in [
            (0u8, MAX_FPS_INTERVAL, DEFAULT_FRAME_INTERVAL * 10),
            (255u8, MAX_FPS_INTERVAL, MAX_FPS_INTERVAL),
        ] {
            let mut session = NativeSession::new(&RADIO);
            session.handle(ClientMessage::Hello {
                version: PROTOCOL_VERSION,
                spectrum: true,
                audio: false,
                max_fps: Some(asked),
            });
            let interval = session.frame_interval();
            assert!(
                interval >= expect_at_least && interval <= expect_at_most,
                "{asked} fps clamped to {interval:?}"
            );
        }
    }

    #[test]
    fn a_rate_is_only_taken_from_a_handshake_that_matched_versions() {
        // A rejected Hello must not leave its numbers behind.
        let mut session = NativeSession::new(&RADIO);
        session.handle(ClientMessage::Hello {
            version: PROTOCOL_VERSION + 1,
            spectrum: true,
            audio: true,
            max_fps: Some(60),
        });
        assert_eq!(session.frame_interval(), DEFAULT_FRAME_INTERVAL);
    }

    fn handshaken(spectrum: bool) -> NativeSession {
        handshaken_with(spectrum, false)
    }

    fn handshaken_with(spectrum: bool, audio: bool) -> NativeSession {
        let mut session = NativeSession::new(&RADIO);
        session.handle(ClientMessage::Hello {
            version: PROTOCOL_VERSION,
            spectrum,
            audio,
            max_fps: None,
        });
        session
    }

    fn frame() -> SpectrumFrame {
        SpectrumFrame {
            center_hz: 14_074_000,
            span_hz: 48_000,
            ref_level_dbm: -20.0,
            bins: vec![-110.0, -100.0, -40.0, -95.0],
            sequence: 7,
        }
    }

    // -----------------------------------------------------------------
    // The requirement ADR 0010 §6 states outright.
    // -----------------------------------------------------------------

    #[test]
    fn a_client_that_declines_spectrum_receives_no_frame_traffic() {
        let session = handshaken(false);
        assert!(!session.wants_spectrum());
        assert!(
            session.encode_spectrum(&frame()).is_none(),
            "a client that declined spectrum must not be sent a frame, \
             not even an empty one"
        );
    }

    #[test]
    fn a_client_that_asks_for_spectrum_gets_whole_frames() {
        let session = handshaken(true);
        assert!(session.wants_spectrum());
        let encoded = session.encode_spectrum(&frame()).unwrap();

        let (kind, payload, consumed) = decode_frame(&encoded).unwrap();
        assert_eq!(kind, FrameKind::Spectrum);
        assert_eq!(consumed, encoded.len());
        assert_eq!(decode_spectrum_payload(payload).unwrap(), frame());
    }

    #[test]
    fn a_client_that_never_handshakes_cannot_be_sent_frames_either() {
        // wants_spectrum() is the only question the frame pump asks, so it
        // must be false for an unhandshaken client even if some other code
        // path set the flag.
        let session = NativeSession::new(&RADIO);
        assert!(!session.wants_spectrum());
        assert!(session.encode_spectrum(&frame()).is_none());
    }

    // -----------------------------------------------------------------
    // Handshake.
    // -----------------------------------------------------------------

    #[test]
    fn a_radios_s_unit_table_survives_the_crossing() {
        // Every other `&'static` field here needs an owned mirror to
        // deserialize. A fixed-size `SUnitScale` does not, which is the
        // reason it is an array -- a remote console reads the same S-units
        // as a local one instead of silently falling back to a formula.
        let scale = SUnitScale::TS570D;
        let wire = MeterDescriptorWire {
            kind: MeterKind::S,
            raw_range: RawRange::new(0, 15),
            active_on_transmit: false,
            s_units: Some(scale),
        };
        let json = serde_json::to_string(&wire).unwrap();
        let back: MeterDescriptorWire = serde_json::from_str(&json).unwrap();
        assert_eq!(back, wire);
        // And still labels raw 10 the way the radio does, not the way a
        // generic formula would. Measured against the panel: raw 9 is S9
        // and every count above it is another ten dB.
        assert_eq!(back.s_units.unwrap().label(10), "S9+10");
    }

    #[test]
    fn the_handshake_publishes_capabilities_without_asking_the_radio() {
        let mut session = NativeSession::new(&RADIO);
        let reply = session.handle(ClientMessage::Hello {
            version: PROTOCOL_VERSION,
            spectrum: false,
            audio: false,
            max_fps: None,
        });

        let ServerMessage::Welcome {
            version,
            capabilities,
        } = reply
        else {
            panic!("expected Welcome")
        };
        assert_eq!(version, PROTOCOL_VERSION);
        assert_eq!(capabilities.model, "Protocol Test Radio");
        assert_eq!(capabilities.modes.len(), 2);
        assert_eq!(capabilities.meters[0].raw_range, RawRange::new(0, 30));
        // The shared-handle fact survives the crossing.
        assert_eq!(
            capabilities.endpoints[0].shareable_with,
            vec![EndpointRole::Keying]
        );
    }

    #[test]
    fn a_version_mismatch_is_refused_rather_than_guessed_at() {
        let mut session = NativeSession::new(&RADIO);
        let reply = session.handle(ClientMessage::Hello {
            version: PROTOCOL_VERSION + 1,
            spectrum: true,
            audio: false,
            max_fps: None,
        });
        assert!(matches!(
            reply,
            ServerMessage::Error {
                code: ErrorCode::VersionMismatch,
                ..
            }
        ));
        // And a refused handshake leaves the session closed for business.
        assert!(!session.is_handshaken());
        assert!(!session.wants_spectrum());
    }

    #[test]
    fn commands_before_hello_are_refused() {
        let mut session = NativeSession::new(&RADIO);
        let reply = session.handle(ClientMessage::Command(Command::SetMode {
            mode: ModeId::Lsb,
        }));
        assert!(matches!(
            reply,
            ServerMessage::Error {
                code: ErrorCode::NotReady,
                ..
            }
        ));
    }

    // -----------------------------------------------------------------
    // Capability-checked commands. The point of the whole model: the
    // server refuses impossible commands with a reason, before the radio
    // ever sees them, and nothing here knows which radio it is.
    // -----------------------------------------------------------------

    #[test]
    fn a_supported_command_is_acknowledged() {
        let mut session = handshaken(false);
        assert_eq!(
            session.handle(ClientMessage::Command(Command::SetMode {
                mode: ModeId::Usb
            })),
            ServerMessage::Ack
        );
    }

    #[test]
    fn an_unsupported_mode_is_refused_as_unsupported_not_out_of_range() {
        // The distinction matters to a client: "this radio cannot do that
        // at all" and "not with that value" call for different UI.
        let mut session = handshaken(false);
        let reply = session.handle(ClientMessage::Command(Command::SetMode {
            mode: ModeId::C4fm,
        }));
        assert!(matches!(
            reply,
            ServerMessage::Error {
                code: ErrorCode::Unsupported,
                ..
            }
        ));
    }

    #[test]
    fn a_frequency_outside_coverage_is_out_of_range() {
        let mut session = handshaken(false);
        let reply = session.handle(ClientMessage::Command(Command::SetFrequency {
            vfo: 0,
            hz: 144_200_000,
        }));
        assert!(matches!(
            reply,
            ServerMessage::Error {
                code: ErrorCode::OutOfRange,
                ..
            }
        ));
        assert_eq!(
            session.handle(ClientMessage::Command(Command::SetFrequency {
                vfo: 0,
                hz: 14_074_000
            })),
            ServerMessage::Ack
        );
    }

    #[test]
    fn a_vfo_index_beyond_the_radios_count_is_refused() {
        let mut session = handshaken(false);
        assert!(matches!(
            session.handle(ClientMessage::Command(Command::SetFrequency {
                vfo: 5,
                hz: 14_074_000
            })),
            ServerMessage::Error {
                code: ErrorCode::Unsupported,
                ..
            }
        ));
    }

    #[test]
    fn absent_subsystems_refuse_their_commands_wholesale() {
        // The same commands, against a radio that lacks each feature.
        let mut session = NativeSession::new(&NO_EXTRAS);
        session.handle(ClientMessage::Hello {
            version: PROTOCOL_VERSION,
            spectrum: false,
            audio: false,
            max_fps: None,
        });

        for command in [
            Command::SetSplit { enabled: true },
            Command::SetMemoryChannel { channel: 3 },
            Command::SetFilterWidth { hz: 500 },
            Command::SetIfShift { hz: 100 },
        ] {
            assert!(
                matches!(
                    session.handle(ClientMessage::Command(command.clone())),
                    ServerMessage::Error {
                        code: ErrorCode::Unsupported,
                        ..
                    }
                ),
                "{command:?} should be unsupported on a radio without it"
            );
        }
    }

    #[test]
    fn present_subsystems_still_police_their_own_bounds() {
        let mut session = handshaken(false);

        assert_eq!(
            session.handle(ClientMessage::Command(Command::SetMemoryChannel {
                channel: 99
            })),
            ServerMessage::Ack
        );
        assert!(matches!(
            session.handle(ClientMessage::Command(Command::SetMemoryChannel {
                channel: 100
            })),
            ServerMessage::Error {
                code: ErrorCode::OutOfRange,
                ..
            }
        ));

        assert_eq!(
            session.handle(ClientMessage::Command(Command::SetFilterWidth {
                hz: 2_400
            })),
            ServerMessage::Ack
        );
        assert!(matches!(
            session.handle(ClientMessage::Command(Command::SetFilterWidth {
                hz: 1_234
            })),
            ServerMessage::Error {
                code: ErrorCode::OutOfRange,
                ..
            }
        ));

        assert!(matches!(
            session.handle(ClientMessage::Command(Command::SetIfShift { hz: -5_000 })),
            ServerMessage::Error {
                code: ErrorCode::OutOfRange,
                ..
            }
        ));
    }

    #[test]
    fn a_published_meter_reading_comes_back_as_a_reading() {
        let mut session = handshaken(false);
        session.publish_state(RadioState {
            vfo_a_hz: 14_074_000,
            vfo_b_hz: 7_074_000,
            mode: ModeId::Usb,
            split: false,
            transmitting: false,
            memory_channel: None,
            if_shift_hz: None,
            filter_width_hz: None,
            meters: vec![MeterSample {
                kind: MeterKind::S,
                raw: 24,
            }],
            levels: None,
            cw: None,
        });
        assert_eq!(
            session.handle(ClientMessage::Command(Command::ReadMeter {
                kind: MeterKind::S
            })),
            ServerMessage::Meter(MeterSample {
                kind: MeterKind::S,
                raw: 24
            })
        );
        // And the state itself comes back whole, so a console never shows
        // a frequency from one moment beside a mode from another.
        match session.handle(ClientMessage::Command(Command::ReadState)) {
            ServerMessage::State(state) => {
                assert_eq!(state.vfo_a_hz, 14_074_000);
                assert_eq!(state.mode, ModeId::Usb);
                assert_eq!(state.meter(MeterKind::S), Some(24));
            }
            other => panic!("expected state, got {other:?}"),
        }
    }

    #[test]
    fn reading_state_before_saying_hello_is_still_refused() {
        // The handshake gate has to keep working now that reads have real
        // answers -- an unauthenticated peer must not be able to ask what
        // the radio is doing.
        let mut session = NativeSession::new(&RADIO);
        assert!(matches!(
            session.handle(ClientMessage::Command(Command::ReadState)),
            ServerMessage::Error {
                code: ErrorCode::NotReady,
                ..
            }
        ));
    }

    #[test]
    fn reading_a_meter_the_radio_does_not_have_is_refused() {
        let mut session = handshaken(false);
        // A meter the radio *has*, with nothing published yet, is not
        // ready -- distinct from one it does not have at all. This
        // assertion used to read `ServerMessage::Ack`, which is what the
        // protocol answered before it had a read side: a confirmation that
        // the question was well-formed, and no answer to it.
        assert!(matches!(
            session.handle(ClientMessage::Command(Command::ReadMeter {
                kind: MeterKind::S
            })),
            ServerMessage::Error {
                code: ErrorCode::NotReady,
                ..
            }
        ));
        assert!(matches!(
            session.handle(ClientMessage::Command(Command::ReadMeter {
                kind: MeterKind::Comp
            })),
            ServerMessage::Error {
                code: ErrorCode::Unsupported,
                ..
            }
        ));
    }

    #[test]
    fn ping_is_answered_after_the_handshake() {
        let mut session = handshaken(false);
        assert_eq!(session.handle(ClientMessage::Ping), ServerMessage::Pong);
    }

    // -----------------------------------------------------------------
    // Framing.
    // -----------------------------------------------------------------

    #[test]
    fn control_messages_round_trip_as_json() {
        let message = ClientMessage::Command(Command::SetFrequency {
            vfo: 0,
            hz: 14_074_000,
        });
        let json = serde_json::to_vec(&message).unwrap();
        let framed = encode_frame(FrameKind::Control, &json);

        let (kind, payload, _) = decode_frame(&framed).unwrap();
        assert_eq!(kind, FrameKind::Control);
        assert_eq!(
            serde_json::from_slice::<ClientMessage>(payload).unwrap(),
            message
        );
    }

    #[test]
    fn a_partial_frame_is_incomplete_rather_than_an_error() {
        // A stream decoder must be able to tell "read more" from "this
        // peer is broken", or a slow network becomes a disconnection.
        let framed = encode_frame(FrameKind::Control, b"{}");
        for cut in 0..framed.len() {
            assert_eq!(decode_frame(&framed[..cut]), Err(FrameError::Incomplete));
        }
        assert!(decode_frame(&framed).is_ok());
    }

    #[test]
    fn frames_decode_one_at_a_time_from_a_coalesced_read() {
        let mut stream = encode_frame(FrameKind::Control, b"{\"a\":1}");
        stream.extend(encode_frame(FrameKind::Spectrum, b"\x00\x01"));

        let (first_kind, _, consumed) = decode_frame(&stream).unwrap();
        assert_eq!(first_kind, FrameKind::Control);
        let (second_kind, payload, _) = decode_frame(&stream[consumed..]).unwrap();
        assert_eq!(second_kind, FrameKind::Spectrum);
        assert_eq!(payload, b"\x00\x01");
    }

    #[test]
    fn an_unknown_frame_kind_is_rejected_not_skipped() {
        let mut bad = encode_frame(FrameKind::Control, b"{}");
        bad[0] = 99;
        assert_eq!(decode_frame(&bad), Err(FrameError::UnknownKind(99)));
    }

    #[test]
    fn an_absurd_control_length_is_refused_before_allocating() {
        let mut header = vec![FrameKind::Control as u8];
        header.extend_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(
            decode_frame(&header),
            Err(FrameError::TooLarge(u32::MAX as usize))
        );
    }

    #[test]
    fn spectrum_bin_order_survives_the_wire() {
        // ADR 0010's invariant applies to the transport too: a protocol
        // that reversed bins would be as wrong as a source that did.
        let original = frame();
        let payload = encode_spectrum_payload(&original);
        let decoded = decode_spectrum_payload(&payload).unwrap();
        assert_eq!(decoded.bins, original.bins);
        let peak = decoded
            .bins
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(peak, 2, "the peak must not move across the wire");
    }

    #[test]
    fn a_truncated_spectrum_payload_decodes_to_nothing() {
        let payload = encode_spectrum_payload(&frame());
        assert!(decode_spectrum_payload(&payload[..20]).is_none());
        assert!(decode_spectrum_payload(&payload[..30]).is_none());
        assert!(decode_spectrum_payload(&payload).is_some());
    }

    #[test]
    fn the_handshake_publishes_the_model_and_the_bench_separately() {
        // ADR 0015 over the wire. A client asking "should I draw a
        // waterfall?" reads the installation; one asking "could this radio
        // ever have one?" reads the model. Collapsing them would put a
        // per-bench fact into a per-model constant again.
        use cat_framework::installation::{InstalledSource, SourceState};

        let installed = Installation::bare(vec![EndpointRole::Cat, EndpointRole::Keying])
            .with_source(InstalledSource::new(
                cat_signal::SignalCapability::IfTap(cat_signal::IfTapConfig {
                    if_center_hz: 73_050_000,
                    inverted: true,
                    trim_hz: -1_420,
                }),
                SourceState::Streaming,
                "RTL-SDR #0",
            ));

        let mut session = NativeSession::with_installation(&RADIO, installed);
        let reply = session.handle(ClientMessage::Hello {
            version: PROTOCOL_VERSION,
            spectrum: true,
            audio: false,
            max_fps: None,
        });
        let ServerMessage::Welcome { capabilities, .. } = reply else {
            panic!("expected Welcome")
        };

        // The model says a tap is possible...
        assert!(capabilities.signal.is_possible());
        // ...and the bench says one is fitted, with a measured trim that
        // could never have lived in the const.
        let source = capabilities.installation.band_panorama().unwrap();
        let cat_signal::SignalCapability::IfTap(cfg) = source.capability else {
            panic!("expected an IF tap")
        };
        assert_eq!(cfg.trim_hz, -1_420);
    }

    #[test]
    fn a_bare_bench_publishes_the_same_model_with_no_sources() {
        let mut session = NativeSession::new(&RADIO);
        let reply = session.handle(ClientMessage::Hello {
            version: PROTOCOL_VERSION,
            spectrum: true,
            audio: false,
            max_fps: None,
        });
        let ServerMessage::Welcome { capabilities, .. } = reply else {
            panic!("expected Welcome")
        };
        // Unplugging the dongle does not change the radio's description.
        assert!(capabilities.signal.is_possible());
        assert!(capabilities.installation.band_panorama().is_none());
    }

    #[test]
    fn capabilities_survive_a_json_round_trip() {
        // The server holds `&'static` const data; a client receives owned
        // data. This is the crossing that makes CapabilitiesWire exist.
        let wire = CapabilitiesWire::from(&RADIO);
        let json = serde_json::to_string(&wire).unwrap();
        let back: CapabilitiesWire = serde_json::from_str(&json).unwrap();
        assert_eq!(back, wire);
        assert_eq!(back.signal, RADIO.signal);
        assert_eq!(back.filters.widths_hz, Some(vec![500, 2_400]));
    }
}

/// The wire shape, pinned so it cannot change without somebody deciding to.
///
/// These are not tests of behaviour. They exist because a wire format has
/// two ends, and the one that broke on 2026-09-09 was not in this repo:
/// a console binary an hour older than the server connected, passed the
/// handshake, and then died on `invalid length 16, expected 13 elements
/// in sequence` -- serde complaining from inside a nested type, naming
/// neither the field nor the fix, at an operator who had changed nothing.
///
/// The handshake already refuses a version mismatch with a good sentence
/// -- `a_version_mismatch_is_refused_rather_than_guessed_at` covers that,
/// and it was working. The gap was that nothing tied *the shape* to *the
/// version*, so a payload could change while the version sat still, and
/// the refusal never fired. That is what these close.
#[cfg(test)]
mod wire_shape_is_pinned {
    use super::*;

    /// Serialize the stub radio's capabilities the way the wire does.
    fn wire_json() -> String {
        let wire = CapabilitiesWire::from(&crate::testing::STUB_RADIO);
        serde_json::to_string(&wire).expect("capabilities serialize")
    }

    #[test]
    fn the_capability_wire_shape_is_pinned_to_the_protocol_version() {
        // A change to any field name, field order, enum spelling or array
        // width inside `CapabilitiesWire` moves this string. That is the
        // point: the diff is the notification.
        //
        // WHEN THIS FAILS, it is telling you a peer built against the old
        // shape can no longer decode this one. Do both of these:
        //
        //   1. bump `PROTOCOL_VERSION`, so the mismatch is refused at the
        //      handshake with a sentence instead of surfacing later as a
        //      serde error inside a nested type;
        //   2. update the pin below to the new string.
        //
        // Doing only (2) restores the exact failure this exists to stop.
        let pinned_version = 2;
        assert_eq!(
            PROTOCOL_VERSION, pinned_version,
            "PROTOCOL_VERSION changed; update the pinned shape below in the \
             same commit so the two cannot drift apart"
        );

        let json = wire_json();

        // The specific field that broke it, called out rather than left
        // for someone to find in a 2 kB string: `SUnitScale` is a
        // fixed-width array on the wire, so its width is part of the
        // format. 13 -> 16 is what version 2 is.
        let thresholds = json
            .split("\"thresholds\":[")
            .nth(1)
            .expect("the S-unit table is on the wire")
            .split(']')
            .next()
            .expect("the table is a JSON array");
        assert_eq!(
            thresholds.split(',').count(),
            16,
            "the S-unit table changed width; a console built against the \
             old width decodes this as `invalid length N, expected M \
             elements in sequence` -- bump PROTOCOL_VERSION"
        );
    }

    #[test]
    fn a_capability_payload_round_trips_through_its_own_format() {
        // The pin above catches a change. This catches the change being
        // wrong: whatever the shape is, both directions must agree on it.
        let wire = CapabilitiesWire::from(&crate::testing::STUB_RADIO);
        let json = serde_json::to_string(&wire).expect("serialize");
        let back: CapabilitiesWire = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(wire, back, "the wire format does not round trip");
    }
}

/// ADR 0023 §2: CW on the wire. Additive and optional, so the version
/// stays 2 and a pre-0023 peer is unaffected in either direction.
#[cfg(test)]
mod cw_wire {
    use super::*;
    use serde_json::Value;

    const CW: CwCapability = CwCapability {
        pitch: Some(HzRange::stepped(400, 1_000, 50)),
        pitch_writable: true,
        keyer_wpm: Some(WpmRange::new(10, 60)),
        keyer_wpm_writable: true,
        break_in: &[BreakInKind::Off, BreakInKind::Semi],
        send_text: Some(CwSendText {
            max_chunk_chars: 24,
            charset: "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789/?",
            pacing: CwPacing::BufferFlag,
            abort: CwAbort::DrainOnly,
            requires_break_in: Some(BreakInKind::Semi),
        }),
        tx_gate: Some(TxGateCapability {
            low_power_raw: 5,
            low_power_label: "5 W",
            power_raw: RawRange::new(5, 100),
        }),
    };

    fn cw_radio() -> &'static RadioCapabilities {
        Box::leak(Box::new(RadioCapabilities {
            cw: Some(CW),
            ..crate::testing::STUB_RADIO
        }))
    }

    fn state(cw: Option<CwState>) -> RadioState {
        RadioState {
            vfo_a_hz: 7_030_000,
            vfo_b_hz: 7_030_000,
            mode: ModeId::CwUpper,
            split: false,
            transmitting: false,
            memory_channel: None,
            if_shift_hz: None,
            filter_width_hz: None,
            meters: vec![],
            levels: None,
            cw,
        }
    }

    /// Strip a key, recursively, so the JSON is what a pre-0023 peer wrote.
    fn without(mut v: Value, key: &str) -> Value {
        fn strip(v: &mut Value, key: &str) {
            match v {
                Value::Object(map) => {
                    map.remove(key);
                    map.values_mut().for_each(|c| strip(c, key));
                }
                Value::Array(items) => items.iter_mut().for_each(|c| strip(c, key)),
                _ => {}
            }
        }
        strip(&mut v, key);
        v
    }

    #[test]
    fn a_pre_0023_welcome_decodes_with_no_cw() {
        let welcome = ServerMessage::Welcome {
            version: PROTOCOL_VERSION,
            capabilities: Box::new(CapabilitiesWire::from(cw_radio())),
        };
        let old = without(serde_json::to_value(&welcome).unwrap(), "cw");
        assert!(!old.to_string().contains("\"cw\""));
        match serde_json::from_value::<ServerMessage>(old).unwrap() {
            ServerMessage::Welcome { capabilities, .. } => assert_eq!(capabilities.cw, None),
            other => panic!("expected Welcome, got {other:?}"),
        }
    }

    #[test]
    fn a_pre_0023_state_decodes_with_no_cw() {
        let old = without(serde_json::to_value(state(None)).unwrap(), "cw");
        let back: RadioState = serde_json::from_value(old).unwrap();
        assert_eq!(back.cw, None);
    }

    #[test]
    fn cw_capability_crosses_the_wire_intact() {
        let wire = CapabilitiesWire::from(cw_radio());
        let cw = wire.cw.clone().expect("cw published");
        assert_eq!(cw.pitch, CW.pitch);
        assert_eq!(cw.keyer_wpm, CW.keyer_wpm);
        assert!(cw.pitch_writable && cw.keyer_wpm_writable);
        assert_eq!(cw.break_in, vec![BreakInKind::Off, BreakInKind::Semi]);
        let send = cw.send_text.expect("send published");
        assert_eq!(send.max_chunk_chars, 24);
        assert_eq!(send.charset, "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789/?");
        assert_eq!(send.pacing, CwPacing::BufferFlag);
        assert_eq!(send.abort, CwAbort::DrainOnly);
        assert_eq!(send.requires_break_in, Some(BreakInKind::Semi));
        let gate = cw.tx_gate.expect("gate published");
        assert_eq!(gate.low_power_raw, 5);
        assert_eq!(gate.low_power_label, "5 W");
        assert_eq!(gate.power_raw, RawRange::new(5, 100));

        let json = serde_json::to_string(&wire).unwrap();
        let back: CapabilitiesWire = serde_json::from_str(&json).unwrap();
        assert_eq!(back, wire);
    }

    #[test]
    fn a_radio_without_cw_publishes_none_not_an_empty_description() {
        let wire = CapabilitiesWire::from(&crate::testing::STUB_RADIO);
        assert_eq!(wire.cw, None);
    }

    #[test]
    fn cw_state_round_trips_in_every_sender_state() {
        let armed = Some(ArmedState {
            callsign: "N0CALL".to_string(),
            power_raw: 5,
            mine: true,
        });
        for sender in [
            SenderState::Idle,
            SenderState::Sending {
                job: 3,
                sent_chars: 4,
                total_chars: 20,
            },
            SenderState::Draining { job: 3 },
            SenderState::Stopped {
                job: 3,
                reason: StopReason::Aborted,
                message: None,
            },
            SenderState::Stopped {
                job: 4,
                reason: StopReason::Error,
                message: Some("power read back 50".to_string()),
            },
        ] {
            let s = state(Some(CwState {
                pitch_hz: Some(600),
                keyer_wpm: Some(20),
                break_in: Some(BreakInKind::Semi),
                armed: armed.clone(),
                sender,
            }));
            let back: RadioState =
                serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn an_unarmed_idle_cw_state_needs_only_the_settings() {
        // A copy trainer's whole need: pitch and speed. `armed` and
        // `sender` default so a server can omit them.
        let s: CwState =
            serde_json::from_str(r#"{"pitch_hz":700,"keyer_wpm":18,"break_in":null}"#).unwrap();
        assert_eq!(s.pitch_hz, Some(700));
        assert_eq!(s.armed, None);
        assert_eq!(s.sender, SenderState::Idle);
    }

    #[test]
    fn cw_command_tags_are_pinned() {
        let cases = [
            (
                Command::SetKeyerSpeed { wpm: 20 },
                r#"{"cmd":"set_keyer_speed","wpm":20}"#,
            ),
            (
                Command::ArmCwTransmit {
                    callsign: "N0CALL".to_string(),
                    dummy_load_acknowledged: true,
                    key_down_cap_s: None,
                },
                r#"{"cmd":"arm_cw_transmit","callsign":"N0CALL","dummy_load_acknowledged":true,"key_down_cap_s":null}"#,
            ),
            (
                Command::SendCwText {
                    text: "CQ".to_string(),
                },
                r#"{"cmd":"send_cw_text","text":"CQ"}"#,
            ),
            (Command::AbortCw, r#"{"cmd":"abort_cw"}"#),
            (Command::DisarmCw, r#"{"cmd":"disarm_cw"}"#),
        ];
        for (command, json) in cases {
            assert_eq!(serde_json::to_string(&command).unwrap(), json);
            assert_eq!(serde_json::from_str::<Command>(json).unwrap(), command);
        }
    }

    #[test]
    fn the_key_down_cap_may_be_omitted_by_a_client() {
        let c: Command = serde_json::from_str(
            r#"{"cmd":"arm_cw_transmit","callsign":"N0CALL","dummy_load_acknowledged":true}"#,
        )
        .unwrap();
        assert!(matches!(
            c,
            Command::ArmCwTransmit {
                key_down_cap_s: None,
                ..
            }
        ));
    }

    #[test]
    fn cw_enum_spellings_are_pinned() {
        assert_eq!(
            serde_json::to_string(&ErrorCode::Forbidden).unwrap(),
            r#""forbidden""#
        );
        assert_eq!(
            serde_json::to_string(&SenderState::Draining { job: 7 }).unwrap(),
            r#"{"state":"draining","job":7}"#
        );
        assert_eq!(
            serde_json::to_string(&StopReason::KeyDownCap).unwrap(),
            r#""key_down_cap""#
        );
    }

    fn arm(callsign: &str, ack: bool, cap: Option<u16>) -> Command {
        Command::ArmCwTransmit {
            callsign: callsign.to_string(),
            dummy_load_acknowledged: ack,
            key_down_cap_s: cap,
        }
    }

    fn send(text: &str) -> Command {
        Command::SendCwText {
            text: text.to_string(),
        }
    }

    fn all_cw() -> Vec<Command> {
        vec![
            Command::SetKeyerSpeed { wpm: 20 },
            arm("N0CALL", true, None),
            send("CQ"),
            Command::AbortCw,
            Command::DisarmCw,
        ]
    }

    fn serving(radio: &'static RadioCapabilities) -> NativeSession {
        let mut s = NativeSession::new(radio);
        s.set_serves_cw(true);
        s
    }

    fn code(r: Result<(), ServerMessage>) -> Option<ErrorCode> {
        match r {
            Ok(()) => None,
            Err(ServerMessage::Error { code, .. }) => Some(code),
            Err(other) => panic!("not an error: {other:?}"),
        }
    }

    fn handshake(session: &mut NativeSession) -> ServerMessage {
        session.handle(ClientMessage::Hello {
            version: PROTOCOL_VERSION,
            spectrum: false,
            audio: false,
            max_fps: None,
        })
    }

    #[test]
    fn a_host_without_a_cw_supervisor_offers_no_cw_even_on_a_cw_radio() {
        // The architect's gate: nothing may key a radio until the full
        // supervisor exists. A host says it does; the default is no.
        let mut session = NativeSession::new(cw_radio());
        match handshake(&mut session) {
            ServerMessage::Welcome { capabilities, .. } => assert_eq!(capabilities.cw, None),
            other => panic!("{other:?}"),
        }
        let mut served = serving(cw_radio());
        match handshake(&mut served) {
            ServerMessage::Welcome { capabilities, .. } => assert!(capabilities.cw.is_some()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn until_the_supervisor_exists_no_cw_command_reaches_a_host() {
        for radio in [cw_radio(), &crate::testing::STUB_RADIO] {
            let session = NativeSession::new(radio);
            for command in all_cw() {
                let reaches = session.reaches_host(&command);
                match (&command, code(session.validate(&command))) {
                    // Abort is never refused -- but with nothing to stop,
                    // it is answered here and goes no further.
                    (Command::AbortCw, None) => assert!(!reaches),
                    (_, Some(ErrorCode::Unsupported)) => {}
                    (c, other) => panic!("{c:?} was not refused: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn a_radio_without_cw_refuses_everything_but_abort_even_when_served() {
        let session = serving(&crate::testing::STUB_RADIO);
        for command in all_cw() {
            let got = code(session.validate(&command));
            if command == Command::AbortCw {
                assert_eq!(got, None);
            } else {
                assert_eq!(got, Some(ErrorCode::Unsupported), "{command:?}");
            }
        }
    }

    #[test]
    fn abort_is_never_refused_and_always_reaches_a_serving_host() {
        let session = serving(cw_radio());
        assert_eq!(code(session.validate(&Command::AbortCw)), None);
        assert!(session.reaches_host(&Command::AbortCw));
        // Other commands reach a serving host exactly when they validate;
        // `reaches_host` is only ever narrower, never a way around them.
        assert!(session.reaches_host(&send("CQ")));
    }

    #[test]
    fn keyer_speed_is_checked_against_the_radio() {
        let session = serving(cw_radio());
        let at = |wpm| code(session.validate(&Command::SetKeyerSpeed { wpm }));
        assert_eq!(at(10), None);
        assert_eq!(at(60), None);
        assert_eq!(at(9), Some(ErrorCode::OutOfRange));
        assert_eq!(at(61), Some(ErrorCode::OutOfRange));

        let read_only: &'static RadioCapabilities = Box::leak(Box::new(RadioCapabilities {
            cw: Some(CwCapability {
                keyer_wpm_writable: false,
                ..CW
            }),
            ..crate::testing::STUB_RADIO
        }));
        assert_eq!(
            code(serving(read_only).validate(&Command::SetKeyerSpeed { wpm: 20 })),
            Some(ErrorCode::Unsupported)
        );
    }

    #[test]
    fn transmit_commands_need_a_radio_that_can_send() {
        let no_send: &'static RadioCapabilities = Box::leak(Box::new(RadioCapabilities {
            cw: Some(CwCapability {
                send_text: None,
                tx_gate: None,
                ..CW
            }),
            ..crate::testing::STUB_RADIO
        }));
        let session = serving(no_send);
        for command in [arm("N0CALL", true, None), send("CQ"), Command::DisarmCw] {
            assert_eq!(
                code(session.validate(&command)),
                Some(ErrorCode::Unsupported),
                "{command:?}"
            );
        }
        // Read-only CW still has settings to set.
        assert_eq!(
            code(session.validate(&Command::SetKeyerSpeed { wpm: 20 })),
            None
        );
        assert_eq!(code(session.validate(&Command::AbortCw)), None);
    }

    #[test]
    fn arming_needs_the_acknowledgement_and_a_real_callsign() {
        let session = serving(cw_radio());
        let at = |c: &str, ack: bool| code(session.validate(&arm(c, ack, None)));
        assert_eq!(at("N0CALL", true), None);
        assert_eq!(at("n0call", true), None, "case is the operator's");
        assert_eq!(at("VE3/N0CALL", true), None, "10 chars: at the limit");
        assert_eq!(at("W1/K9X", true), None);
        assert_eq!(at("N0CALL", false), Some(ErrorCode::Forbidden));
        for bad in [
            "",
            "N0",
            "NOCALL",
            "123456",
            "N0 CALL",
            "N0-CALL",
            "N0CALLXXXXX",
        ] {
            assert_eq!(at(bad, true), Some(ErrorCode::Forbidden), "{bad:?}");
        }
    }

    #[test]
    fn a_callsign_the_keyer_cannot_send_cannot_be_armed() {
        // The server sends `DE <call>` itself (decision 4); a call with a
        // character outside the charset could never be identified.
        let no_slash: &'static RadioCapabilities = Box::leak(Box::new(RadioCapabilities {
            cw: Some(CwCapability {
                send_text: Some(CwSendText {
                    charset: "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
                    ..CW.send_text.unwrap()
                }),
                ..CW
            }),
            ..crate::testing::STUB_RADIO
        }));
        let session = serving(no_slash);
        assert_eq!(
            code(session.validate(&arm("W1/K9X", true, None))),
            Some(ErrorCode::Forbidden)
        );
        assert_eq!(code(session.validate(&arm("K9X", true, None))), None);
    }

    #[test]
    fn a_client_may_lower_the_key_down_cap_but_zero_is_not_a_cap() {
        let session = serving(cw_radio());
        let at = |cap| code(session.validate(&arm("N0CALL", true, cap)));
        assert_eq!(at(Some(10)), None);
        assert_eq!(at(Some(MAX_KEY_DOWN_CAP_S)), None);
        // Above the ceiling: accepted, and the supervisor holds it at
        // the ceiling. A client cannot raise it, and need not be refused
        // for asking.
        assert_eq!(at(Some(600)), None);
        assert_eq!(at(Some(0)), Some(ErrorCode::OutOfRange));
    }

    #[test]
    fn sent_text_must_be_sendable_and_bounded() {
        let session = serving(cw_radio());
        let at = |t: &str| code(session.validate(&send(t)));
        assert_eq!(at("CQ CQ DE N0CALL"), None);
        assert_eq!(at("cq de n0call"), None);
        assert_eq!(at(""), Some(ErrorCode::OutOfRange));
        assert_eq!(at("   "), Some(ErrorCode::OutOfRange));
        assert_eq!(at("HI!"), Some(ErrorCode::OutOfRange));
        let longest = "E".repeat(MAX_CW_TEXT_CHARS);
        assert_eq!(at(&longest), None);
        assert_eq!(at(&format!("{longest}E")), Some(ErrorCode::OutOfRange));
    }

    #[test]
    fn a_refused_send_names_the_character() {
        let session = serving(cw_radio());
        match session.validate(&send("HI!")) {
            Err(ServerMessage::Error { message, .. }) => assert!(message.contains('!')),
            other => panic!("{other:?}"),
        }
    }
}
