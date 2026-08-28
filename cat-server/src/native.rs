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

use cat_framework::capabilities::*;
use serde::{Deserialize, Serialize};

/// Wire protocol version, sent in every [`ClientMessage::Hello`] and
/// [`ServerMessage::Welcome`].
pub const PROTOCOL_VERSION: u16 = 1;

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
}

impl FrameKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(FrameKind::Control),
            2 => Some(FrameKind::Spectrum),
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
    pub signal: cat_signal::SignalCapability,
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
                })
                .collect(),
            memory: c.memory,
            menu: c.menu,
            signal: c.signal,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello {
        version: u16,
        /// Whether to send this client spectrum frames at all.
        #[serde(default)]
        spectrum: bool,
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
    handshaken: bool,
    spectrum: bool,
}

impl NativeSession {
    pub fn new(capabilities: &'static RadioCapabilities) -> Self {
        Self {
            capabilities,
            handshaken: false,
            spectrum: false,
        }
    }

    /// Whether this client asked for spectrum frames.
    ///
    /// The one question the frame pump asks. `false` until a successful
    /// `Hello` says otherwise, so a client that never handshakes cannot be
    /// sent frames either.
    pub fn wants_spectrum(&self) -> bool {
        self.handshaken && self.spectrum
    }

    pub fn is_handshaken(&self) -> bool {
        self.handshaken
    }

    /// Handle one decoded client message.
    pub fn handle(&mut self, message: ClientMessage) -> ServerMessage {
        match message {
            ClientMessage::Hello { version, spectrum } => {
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
                ServerMessage::Welcome {
                    version: PROTOCOL_VERSION,
                    capabilities: Box::new(CapabilitiesWire::from(self.capabilities)),
                }
            }
            _ if !self.handshaken => ServerMessage::Error {
                code: ErrorCode::NotReady,
                message: "send hello first".to_string(),
            },
            ClientMessage::Ping => ServerMessage::Pong,
            ClientMessage::Command(command) => match self.validate(&command) {
                Ok(()) => ServerMessage::Ack,
                Err(e) => e,
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
        signal: cat_signal::SignalCapability::IfTap(cat_signal::IfTapConfig {
            if_center_hz: 73_050_000,
            inverted: true,
            trim_hz: 0,
        }),
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
        signal: cat_signal::SignalCapability::None,
    };

    fn handshaken(spectrum: bool) -> NativeSession {
        let mut session = NativeSession::new(&RADIO);
        session.handle(ClientMessage::Hello {
            version: PROTOCOL_VERSION,
            spectrum,
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
    fn the_handshake_publishes_capabilities_without_asking_the_radio() {
        let mut session = NativeSession::new(&RADIO);
        let reply = session.handle(ClientMessage::Hello {
            version: PROTOCOL_VERSION,
            spectrum: false,
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
    fn reading_a_meter_the_radio_does_not_have_is_refused() {
        let mut session = handshaken(false);
        assert_eq!(
            session.handle(ClientMessage::Command(Command::ReadMeter {
                kind: MeterKind::S
            })),
            ServerMessage::Ack
        );
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
