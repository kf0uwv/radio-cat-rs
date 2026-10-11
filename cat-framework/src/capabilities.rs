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

//! [`RadioCapabilities`]: what the attached radio can do, as plain data.
//!
//! See `docs/adr/0010-capability-model-and-normalized-signal-source.md`
//! (sections 1-2) for the design record. Task 11 of
//! `planning/architect/task_plan.md`.
//!
//! # What belongs here, and what does not
//!
//! This module is **data only**. It carries no wire format, no protocol
//! opinion, and no behaviour: every type is a `Copy` description that a
//! radio crate can write as a `const`, because every field is answerable
//! statically per model. That is what lets the session handshake publish
//! capabilities without a single round trip to the radio.
//!
//! In particular, nothing here records a radio's *own* encoding for
//! anything. [`ModeDescriptor`] deliberately does not carry the byte a
//! given radio puts on the wire for that mode — the TS-570D sends `6` for
//! FSK and the FT-991A sends `6` for RTTY-LSB, so a shared table of wire
//! codes would be a table of coincidences. Translating between a
//! [`ModeId`] and a radio's own encoding is the radio crate's job, and it
//! already has the command table to do it with.
//!
//! # Model facts only
//!
//! Everything here is answerable statically per model, which is what lets a
//! radio crate declare its capabilities as a `const` and the session
//! handshake cost no round trip. That claim used to be *nearly* true: this
//! type once carried a `SignalCapability` asserting a TS-570D **had** an IF
//! tap fitted, when the truth is it has a CN4 header and whether a dongle
//! hangs off it is a fact about one bench.
//!
//! ADR 0015 split the two. What this radio model can do lives here; what
//! this deployment has wired lives in [`crate::installation`]. The rule for
//! arguments at the boundary: **if two units of the same model can disagree
//! about it, it is installation data.**
//!
//! # Why the descriptors carry ranges rather than values
//!
//! A capability says what the radio *can* report, not what it *is*
//! reporting. [`MeterDescriptor::raw_range`] is the load-bearing example:
//! the TS-570D reports its S-meter as 0-30 and the FT-991A as 0-255, and
//! that difference is a property of the radio, not a display preference.
//! A UI that scales a bar against this range is correct for both radios
//! with one implementation — which is precisely the seam
//! `docs/adr/0011-cat-ui-base-widgets-radio-specific-layout.md` draws
//! between a shared widget and a radio-specific layout.

/// What a radio model can *accept* as a spectrum source.
///
/// This is a fact about circuitry, not about a bench. Every TS-570D has a
/// CN4 header at 73.05 MHz with an inverted IF, whether or not a dongle is
/// plugged into it — see `docs/adr/0015-model-facts-versus-installation-facts.md`.
/// What is actually *connected* lives in [`crate::installation::Installation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum SignalSupport {
    /// No way to get spectrum out of this radio at all.
    None,
    /// The radio has its own bandscope, reachable over CAT.
    ///
    /// Defined and not implemented: no radio in this fleet exports one.
    NativeScope { max_span_hz: u32, bins: u16 },
    /// The radio exposes an IF tap point.
    ///
    /// Both fields are properties of the mixing arrangement and true of
    /// every unit ever built: the TS-570D's first IF is 73.05 MHz, and its
    /// spectrum is mirrored because LO1 is high-side injection. The
    /// per-station calibration that goes with a tap — `trim_hz`, one
    /// dongle's crystal error — is installation data and is not here.
    IfTapPoint { if_center_hz: u64, inverted: bool },
}

impl SignalSupport {
    /// Whether a spectrum source could ever be attached to this radio.
    ///
    /// Says nothing about whether one *is*. A console asking "should I draw
    /// a waterfall?" wants the installation, not this.
    pub fn is_possible(&self) -> bool {
        !matches!(self, SignalSupport::None)
    }
}

/// An inclusive range of raw values a radio reports for some quantity.
///
/// Deliberately not `std::ops::RangeInclusive`: that type is not `Copy`,
/// which would stop the whole capability tree from being a `const`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RawRange {
    pub min: u16,
    pub max: u16,
}

impl RawRange {
    pub const fn new(min: u16, max: u16) -> Self {
        Self { min, max }
    }

    /// Where `value` sits in this range, as 0.0-1.0, clamped at both ends.
    ///
    /// The one piece of behaviour in this module, and it earns its place:
    /// without it every consumer re-derives the same clamp-and-divide, and
    /// they will not all remember the clamp.
    pub fn fraction(&self, value: u16) -> f32 {
        if self.max <= self.min {
            return 0.0;
        }
        let clamped = value.clamp(self.min, self.max);
        f32::from(clamped - self.min) / f32::from(self.max - self.min)
    }
}

/// An inclusive frequency range in Hz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FrequencyRange {
    pub min_hz: u64,
    pub max_hz: u64,
}

impl FrequencyRange {
    pub const fn new(min_hz: u64, max_hz: u64) -> Self {
        Self { min_hz, max_hz }
    }

    pub fn contains(&self, hz: u64) -> bool {
        hz >= self.min_hz && hz <= self.max_hz
    }
}

// ---------------------------------------------------------------------------
// Endpoints (ADR 0010 section 2)
// ---------------------------------------------------------------------------

/// What a physical transport handle is *for*.
///
/// Resolves `ft991a`'s ADR 0002 deferred USB dual-port question: a radio
/// stops assuming "the CAT session's own transport is also the
/// modem-control handle," and the keying handle is supplied independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum EndpointRole {
    /// Command/response CAT traffic.
    Cat,
    /// RTS/DTR PTT and CW keying. The FT-991A's "Standard" port.
    Keying,
    /// USB audio codec: RX audio in, TX audio out.
    Audio,
}

/// One physical handle the radio expects, and what it may be shared with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct EndpointDescriptor {
    pub role: EndpointRole,
    /// Whether the radio is unusable without this endpoint. A missing
    /// `Audio` endpoint costs you audio; a missing `Cat` endpoint costs
    /// you the radio.
    pub required: bool,
    /// Roles this endpoint may serve *simultaneously* on one handle.
    ///
    /// The TS-570D's single RS-232C port is `Cat` + `Keying` together, so
    /// its `Cat` endpoint lists `Keying` here. The FT-991A's two CP210x
    /// ports are separate handles and list nothing.
    pub shareable_with: &'static [EndpointRole],
}

/// Every handle a radio expects, in no particular order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct EndpointSet {
    pub endpoints: &'static [EndpointDescriptor],
}

impl EndpointSet {
    pub const fn new(endpoints: &'static [EndpointDescriptor]) -> Self {
        Self { endpoints }
    }

    /// The descriptor that *natively* carries `role`.
    ///
    /// Does not consider [`EndpointDescriptor::shareable_with`]; see
    /// [`serves`](Self::serves) for the question consumers usually mean.
    pub fn find(&self, role: EndpointRole) -> Option<&'static EndpointDescriptor> {
        self.endpoints.iter().find(|e| e.role == role)
    }

    /// Whether some endpoint can carry `role`, natively or by sharing.
    pub fn serves(&self, role: EndpointRole) -> bool {
        self.endpoints
            .iter()
            .any(|e| e.role == role || e.shareable_with.contains(&role))
    }

    /// How many distinct handles must be opened.
    ///
    /// One for the TS-570D (its RS-232C port does CAT and keying at once);
    /// three for the FT-991A (two CP210x ports plus a USB codec).
    pub fn handle_count(&self) -> usize {
        self.endpoints.len()
    }
}

// ---------------------------------------------------------------------------
// VFOs
// ---------------------------------------------------------------------------

/// VFO count and the offset features layered on top of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VfoCapability {
    /// Independently tunable VFOs. Two on both radios in this fleet.
    pub count: u8,
    /// Transmit on one VFO while receiving on another.
    pub split: bool,
    /// Receive incremental tuning offset range, if supported.
    pub rit_hz: Option<i32>,
    /// Transmit incremental tuning offset range, if supported.
    pub xit_hz: Option<i32>,
}

// ---------------------------------------------------------------------------
// Modes
// ---------------------------------------------------------------------------

/// Broad family a mode belongs to, for grouping and for deciding which
/// controls are meaningful (an AGC setting means something in SSB and
/// nothing in FM).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum ModeKind {
    Ssb,
    Cw,
    Am,
    Fm,
    /// FSK/RTTY and the radio's own DATA modes — anything whose audio path
    /// is meant for a machine rather than an ear.
    Data,
    /// Vendor digital voice (Yaesu C4FM, and its equivalents elsewhere).
    DigitalVoice,
}

/// Which sideband a mode uses, where that is a meaningful question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Sideband {
    Lower,
    Upper,
}

/// A radio-independent identity for an operating mode.
///
/// `#[non_exhaustive]` because this is a shared vocabulary that grows as
/// radios join the fleet, not a closed set. Adding a variant when a new
/// radio brings a genuinely new mode is expected and is **not** the
/// escape hatch ADR 0010's gate forbids — an escape hatch would be a free
/// string or a `model` field that downstream code matches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum ModeId {
    Lsb,
    Usb,
    /// CW, upper sideband injection.
    CwUpper,
    /// CW, lower sideband injection ("CW reverse" on the TS-570D).
    CwLower,
    Am,
    AmNarrow,
    Fm,
    FmNarrow,
    /// FSK/RTTY, lower sideband.
    RttyLsb,
    /// FSK/RTTY, upper sideband ("FSK reverse" on the TS-570D).
    RttyUsb,
    DataLsb,
    DataUsb,
    DataFm,
    /// Yaesu C4FM digital voice.
    C4fm,
}

/// One operating mode the radio supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ModeDescriptor {
    pub id: ModeId,
    /// Short label for a mode button or readout, in the form operators
    /// expect to see it ("LSB", "CW-R", "DATA-U").
    pub label: &'static str,
    pub kind: ModeKind,
    /// `None` where the question does not apply (FM, AM).
    pub sideband: Option<Sideband>,
    /// Default receive bandwidth, for a UI that must draw a passband
    /// before it has asked the radio anything.
    pub default_bandwidth_hz: u32,
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

/// IF filtering the radio exposes to CAT control.
///
/// Split into three independent `Option`s rather than one "has filters"
/// flag because radios genuinely differ in which of these they expose:
/// offering a width control for a radio that only has IF shift produces a
/// dead control, which is worse than an absent one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct FilterCapability {
    /// Passband shift range, symmetric around centre, in Hz.
    pub if_shift_hz: Option<i32>,
    /// Selectable passband widths in Hz, narrowest first.
    pub widths_hz: Option<&'static [u32]>,
    /// Whether a notch filter is CAT-controllable.
    pub notch: bool,
}

// ---------------------------------------------------------------------------
// Meters
// ---------------------------------------------------------------------------

/// Which physical quantity a meter reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum MeterKind {
    /// Received signal strength.
    S,
    /// Transmit power output.
    Po,
    /// Standing wave ratio.
    Swr,
    /// Automatic level control.
    Alc,
    /// Final-stage drain current.
    Id,
    /// Final-stage drain voltage.
    Vdd,
    /// Speech compression level.
    Comp,
}

/// One meter, and the raw range the radio reports it over.
///
/// `raw_range` is the whole point of this type. The TS-570D reports its
/// S-meter over 0-30 and the FT-991A over 0-255; a consumer that scales
/// against this range is correct for both with one implementation, and a
/// consumer that hardcodes either is silently wrong on the other radio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct MeterDescriptor {
    pub kind: MeterKind,
    pub raw_range: RawRange,
    /// Whether this meter reads during receive (`S`) or transmit (the
    /// rest). A UI can use this to decide what to show without knowing
    /// what each meter means.
    pub active_on_transmit: bool,
    /// Where this meter's S-unit boundaries fall, when the radio publishes
    /// them. `None` for a meter that has no S-units at all — every meter
    /// but `S` — and for an `S` meter whose table is not known.
    pub s_units: Option<SUnitScale>,
}

/// The labels [`SUnitScale`] assigns, in order.
///
/// Up to `S9+60`, which is where the scale on a receiver's meter ends.
/// This stopped at `S9+30` and every reading above it collapsed onto that
/// one label -- on a TS-570D that was the top four of its sixteen raw
/// values, and on an IC-7100 the top quarter of its range, drawn as up to
/// 30 dB weaker than the radio was reporting.
pub const S_UNIT_LABELS: [&str; 16] = [
    "S0", "S1", "S2", "S3", "S4", "S5", "S6", "S7", "S8", "S9", "S9+10", "S9+20", "S9+30", "S9+40",
    "S9+50", "S9+60",
];

/// Where each S-unit boundary falls on a radio's raw meter scale.
///
/// **S-meter law is radio data, not a display preference.** Where S9 sits
/// and how the units below it space out is a property of the meter
/// circuit, and it is not linear on real radios. The TS-570D's own table
/// gives S0 three raw counts and every other unit two, which no clean
/// formula reproduces — substituting one changes the reading at 8 of its
/// 31 raw values.
///
/// So this sits on [`MeterDescriptor`] beside [`RawRange`], for the same
/// reason `RawRange` does: a widget that has to be *told* the scale is a
/// widget that can be told the wrong one. Both travel with the reading.
///
/// `thresholds` is the **inclusive upper bound** of each label in
/// [`S_UNIT_LABELS`], ascending. It is a fixed-size array rather than a
/// slice so
/// that a scale is `Copy`, is `const`-constructible, crosses the native
/// protocol without an owned mirror, and cannot be built with a length
/// that does not match the labels. A reading above the last threshold
/// takes the last label, so the top entry is conventionally `u16::MAX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SUnitScale {
    thresholds: [u16; S_UNIT_LABELS.len()],
}

impl SUnitScale {
    /// A scale from explicit upper bounds, one per label in
    /// [`S_UNIT_LABELS`].
    pub const fn new(thresholds: [u16; S_UNIT_LABELS.len()]) -> Self {
        Self { thresholds }
    }

    /// The Kenwood TS-570D's table, **measured against its own meter**.
    ///
    /// One raw count per S-unit up to S9, then ten dB a count above it.
    /// Read off the radio on 2026-09-08 by an operator watching the panel
    /// while the same signal was sampled over CAT:
    ///
    /// | raw | panel |
    /// |---|---|
    /// | 9 | S9 |
    /// | 10 | S9+10 |
    /// | 11 | S9+20 |
    ///
    /// Three points, one front-end step apart, on one signal. They fix the
    /// top half; the bottom half follows from raw 0 being S0 and raw 9
    /// being S9 — nine counts for nine S-units — which also puts full
    /// scale (raw 15, per the manual's `SM` range of `0000~0015`) at
    /// S9+60, the conventional top of an S-meter. A scale that lands on
    /// both of those independently is unlikely to be a coincidence.
    ///
    /// The table this replaces was inherited from the old TUI and was out
    /// by about five S-units: it put S9 at raw 20, which this radio never
    /// reaches, so a signal the panel called S9+10 was drawn as S4 and a
    /// ten dB change did not move the display at all. That is the
    /// "S meter doesn't seem to match" reported from the bench, and ADR
    /// 0011 rev 4's "the operator sees no change" was preserving a
    /// mistake.
    ///
    /// The top four counts continue the same ten dB a count: raw 12 is
    /// S9+30 and raw 15, the top of the meter, is S9+60. Those were not
    /// measured -- the three readings above are the evidence, and they
    /// stop at S9+20 -- but they are the same straight line, and the line
    /// arriving exactly at S9+60 on the last raw value the radio can
    /// report is the reason to believe it.
    pub const TS570D: Self =
        Self::new([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, u16::MAX]);

    /// The label for a raw reading.
    pub fn label(&self, raw: u16) -> &'static str {
        for (i, bound) in self.thresholds.iter().enumerate() {
            if raw <= *bound {
                return S_UNIT_LABELS[i];
            }
        }
        S_UNIT_LABELS[S_UNIT_LABELS.len() - 1]
    }
}

/// Every meter a radio reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct MeterSet {
    pub meters: &'static [MeterDescriptor],
}

impl MeterSet {
    pub const fn new(meters: &'static [MeterDescriptor]) -> Self {
        Self { meters }
    }

    pub fn find(&self, kind: MeterKind) -> Option<&'static MeterDescriptor> {
        self.meters.iter().find(|m| m.kind == kind)
    }

    pub fn has(&self, kind: MeterKind) -> bool {
        self.find(kind).is_some()
    }
}

// ---------------------------------------------------------------------------
// Memory and menu
// ---------------------------------------------------------------------------

/// Memory channel storage, where the radio exposes it to CAT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemoryCapability {
    /// Inclusive channel-number range, as the radio numbers them.
    pub channels: RawRange,
    /// Whether a channel can carry an alphanumeric name.
    pub named: bool,
    /// Whether a stored channel records its own mode as well as frequency.
    pub stores_mode: bool,
    /// Whether memory scan is CAT-controllable.
    pub scan: bool,
}

/// The radio's configuration menu, where CAT can reach it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MenuCapability {
    /// How many menu entries exist.
    pub item_count: u16,
    /// Whether CAT can write menu entries, not merely read them.
    pub writable: bool,
}

// ---------------------------------------------------------------------------
// CW (ADR 0023 §1)
// ---------------------------------------------------------------------------

/// An inclusive range of audio frequencies, in Hz, optionally stepped.
///
/// Operator units, not the radio's encoding: the TS-570D's `PT` index
/// 0-12 and the IC-7100's raw 0-255 both become Hz here, and translating
/// back is the radio crate's job, as it is for mode bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HzRange {
    pub min_hz: u32,
    pub max_hz: u32,
    /// `None` (or `Some(0)`): any whole Hz in range is settable.
    pub step_hz: Option<u32>,
}

impl HzRange {
    pub const fn stepped(min_hz: u32, max_hz: u32, step_hz: u32) -> Self {
        Self {
            min_hz,
            max_hz,
            step_hz: Some(step_hz),
        }
    }

    pub const fn continuous(min_hz: u32, max_hz: u32) -> Self {
        Self {
            min_hz,
            max_hz,
            step_hz: None,
        }
    }

    /// Whether `hz` is a value this radio can actually be set to: inside
    /// the range and, for a stepped range, on a step counted from `min_hz`.
    pub fn contains(&self, hz: u32) -> bool {
        if hz < self.min_hz || hz > self.max_hz {
            return false;
        }
        match self.step_hz {
            Some(step) if step > 0 => (hz - self.min_hz) % step == 0,
            _ => true,
        }
    }
}

/// An inclusive internal-keyer speed range, in words per minute (PARIS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WpmRange {
    pub min: u16,
    pub max: u16,
}

impl WpmRange {
    pub const fn new(min: u16, max: u16) -> Self {
        Self { min, max }
    }

    pub fn contains(&self, wpm: u16) -> bool {
        wpm >= self.min && wpm <= self.max
    }
}

/// A break-in (QSK) setting CAT can select.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum BreakInKind {
    Off,
    /// Semi break-in: the transmitter drops back to receive after a delay.
    Semi,
    /// Full break-in: receive between elements.
    Full,
}

/// How the server learns that the radio's keyer has finished a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CwPacing {
    /// The radio reports whether its keyer buffer has space (TS-570D
    /// `KY;`). Space, not idle: it reads "ready" while still keying.
    BufferFlag,
    /// No busy flag: the server paces by a `cat-morse` duration estimate
    /// at the radio's reported speed, plus a margin (IC-7100).
    Estimated,
}

/// What an abort can actually do on this radio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CwAbort {
    /// A command stops the keyer mid-character (IC-7100 `17 FF`).
    RadioStop,
    /// No stop command: abort stops feeding, and the radio finishes the
    /// chunk it holds. The server bounds that by chunk size (ADR 0023 §4).
    DrainOnly,
}

/// The radio's keyer can send arbitrary text over CAT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct CwSendText {
    /// Largest message the radio accepts in one command.
    pub max_chunk_chars: u16,
    /// The characters the keyer will send, uppercase. A client maps
    /// prosigns onto these (AR → `+`, BT → `=`) and the server rejects
    /// anything outside them. A space is always allowed and need not be
    /// listed.
    pub charset: &'static str,
    pub pacing: CwPacing,
    pub abort: CwAbort,
    /// The break-in the radio needs before it will key a sent message
    /// (the IC-7100 transmits a `17` message only with break-in on). The
    /// server sets it on arm and restores it on disarm. `None`: the radio
    /// keys regardless.
    pub requires_break_in: Option<BreakInKind>,
}

impl CwSendText {
    /// The first character of `text` this keyer cannot send, if any.
    ///
    /// ASCII letters are folded to upper case; nothing else is, so no
    /// Unicode letter can upper-case its way into the charset. A space is
    /// always allowed; any other whitespace is not.
    pub fn rejected_char(&self, text: &str) -> Option<char> {
        text.chars()
            .find(|&c| c != ' ' && !self.charset.contains(c.to_ascii_uppercase()))
    }

    pub fn accepts(&self, text: &str) -> bool {
        self.rejected_char(text).is_none()
    }
}

/// What the server forces before any keyed CW, and how it checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct TxGateCapability {
    /// The radio's lowest settable power, in its own units. The server
    /// sets exactly this on arm and reads it back (ADR 0023 decision 6).
    pub low_power_raw: u16,
    /// What `low_power_raw` means to an operator ("5 W").
    pub low_power_label: &'static str,
    /// Inclusive raw range the radio's power read reports.
    pub power_raw: RawRange,
}

/// CW control the radio exposes over CAT. See ADR 0023.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct CwCapability {
    /// Sidetone/receive pitch. `None`: CAT cannot read it.
    pub pitch: Option<HzRange>,
    pub pitch_writable: bool,
    /// Internal keyer speed. `None`: CAT cannot read it.
    pub keyer_wpm: Option<WpmRange>,
    pub keyer_wpm_writable: bool,
    /// Break-in settings CAT can select. Empty: CAT cannot set break-in.
    pub break_in: &'static [BreakInKind],
    /// `None`: CW is read-only over CAT (FT-991A in v1).
    pub send_text: Option<CwSendText>,
    /// Required whenever `send_text` is `Some`.
    pub tx_gate: Option<TxGateCapability>,
}

/// Why a [`CwCapability`] description is incoherent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CwCapabilityError {
    #[error("pitch range is empty (min above max)")]
    EmptyPitchRange,
    #[error("keyer speed range is empty (min above max)")]
    EmptyWpmRange,
    #[error("a setting is marked writable but has no range")]
    WritableWithoutRange,
    #[error("send_text.max_chunk_chars is zero")]
    ZeroChunk,
    #[error("send_text.charset is empty")]
    EmptyCharset,
    #[error("send_text.charset contains a lower-case letter")]
    CharsetNotUppercase,
    #[error("send_text.requires_break_in is not one of break_in")]
    RequiredBreakInNotSelectable,
    #[error("send_text is present but tx_gate is not: nothing says how to clamp power")]
    SendWithoutTxGate,
    #[error("tx_gate.low_power_raw lies outside tx_gate.power_raw")]
    LowPowerOutOfRange,
}

impl CwCapability {
    /// Check that this description is one a server can act on safely.
    ///
    /// A radio crate calls this from a test on its own `const`; the
    /// server may call it at startup. The rule that matters most:
    /// a radio that can send must say how its power is clamped.
    pub fn validate(&self) -> Result<(), CwCapabilityError> {
        if let Some(p) = self.pitch {
            if p.min_hz > p.max_hz {
                return Err(CwCapabilityError::EmptyPitchRange);
            }
        }
        if let Some(w) = self.keyer_wpm {
            if w.min > w.max {
                return Err(CwCapabilityError::EmptyWpmRange);
            }
        }
        if (self.pitch_writable && self.pitch.is_none())
            || (self.keyer_wpm_writable && self.keyer_wpm.is_none())
        {
            return Err(CwCapabilityError::WritableWithoutRange);
        }
        if let Some(send) = self.send_text {
            if send.max_chunk_chars == 0 {
                return Err(CwCapabilityError::ZeroChunk);
            }
            if send.charset.is_empty() {
                return Err(CwCapabilityError::EmptyCharset);
            }
            if send.charset.chars().any(char::is_lowercase) {
                return Err(CwCapabilityError::CharsetNotUppercase);
            }
            if let Some(kind) = send.requires_break_in {
                if !self.break_in.contains(&kind) {
                    return Err(CwCapabilityError::RequiredBreakInNotSelectable);
                }
            }
            if self.tx_gate.is_none() {
                return Err(CwCapabilityError::SendWithoutTxGate);
            }
        }
        if let Some(gate) = self.tx_gate {
            let r = gate.power_raw;
            if gate.low_power_raw < r.min || gate.low_power_raw > r.max {
                return Err(CwCapabilityError::LowPowerOutOfRange);
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The whole picture
// ---------------------------------------------------------------------------

/// Everything a session needs to know about the attached radio.
///
/// Negotiated once, at connect. Every field is answerable statically per
/// model, so publishing this costs no round trip.
///
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct RadioCapabilities {
    /// Human-readable model name, for display and logging.
    ///
    /// **Not a discriminator.** Downstream code that matches on this
    /// string has defeated the entire point of this type: the whole reason
    /// capabilities are data is so consumers ask what a radio *can do*
    /// rather than what it *is*. ADR 0010's acceptance gate names exactly
    /// this as a failure.
    pub model: &'static str,
    pub endpoints: EndpointSet,
    pub vfos: VfoCapability,
    pub modes: &'static [ModeDescriptor],
    /// Tuning step sizes in Hz, smallest first.
    pub tuning_steps_hz: &'static [u32],
    /// Frequency coverage the radio will accept.
    pub rx_range: FrequencyRange,
    pub filters: FilterCapability,
    pub meters: MeterSet,
    pub memory: Option<MemoryCapability>,
    pub menu: Option<MenuCapability>,
    /// What kind of spectrum source this radio can *accept*.
    ///
    /// Not what is connected. A TS-570D reports `IfTapPoint` on every
    /// bench, because the CN4 header is part of the radio; whether an SDR
    /// hangs off it is [`crate::installation::Installation`]'s business.
    pub signal: SignalSupport,
    /// CW control over CAT (ADR 0023). `None`: this radio exposes none.
    pub cw: Option<CwCapability>,
}

impl RadioCapabilities {
    /// Whether `id` is one of this radio's modes.
    pub fn supports_mode(&self, id: ModeId) -> bool {
        self.modes.iter().any(|m| m.id == id)
    }

    pub fn mode(&self, id: ModeId) -> Option<&'static ModeDescriptor> {
        self.modes.iter().find(|m| m.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A deliberately synthetic radio. The real TS-570D and FT-991A
    // fixtures belong to Task 13, which is the gate on whether this model
    // actually fits them -- writing them here would pre-empt that check
    // and, worse, would let this module be tuned to pass its own exam.
    const SHARED_PORT: &[EndpointRole] = &[EndpointRole::Keying];
    const ENDPOINTS: &[EndpointDescriptor] = &[EndpointDescriptor {
        role: EndpointRole::Cat,
        required: true,
        shareable_with: SHARED_PORT,
    }];
    const MODES: &[ModeDescriptor] = &[
        ModeDescriptor {
            id: ModeId::Lsb,
            label: "LSB",
            kind: ModeKind::Ssb,
            sideband: Some(Sideband::Lower),
            default_bandwidth_hz: 2400,
        },
        ModeDescriptor {
            id: ModeId::Fm,
            label: "FM",
            kind: ModeKind::Fm,
            sideband: None,
            default_bandwidth_hz: 12000,
        },
    ];
    const METERS: &[MeterDescriptor] = &[
        MeterDescriptor {
            kind: MeterKind::S,
            raw_range: RawRange::new(0, 30),
            active_on_transmit: false,
            s_units: None,
        },
        MeterDescriptor {
            kind: MeterKind::Swr,
            raw_range: RawRange::new(0, 255),
            active_on_transmit: true,
            s_units: None,
        },
    ];

    /// The whole capability tree is a compile-time constant.
    ///
    /// This is not a stylistic preference. ADR 0010 section 1 claims "the
    /// handshake costs no round trip to the radio," and that claim is only
    /// true if a radio crate can write its capabilities as a `const`. If
    /// any type here stopped being `Copy`/`const`-constructible, this item
    /// would fail to compile and the claim would have quietly become false.
    const RADIO: RadioCapabilities = RadioCapabilities {
        model: "Synthetic Test Radio",
        endpoints: EndpointSet::new(ENDPOINTS),
        vfos: VfoCapability {
            count: 2,
            split: true,
            rit_hz: Some(9990),
            xit_hz: None,
        },
        modes: MODES,
        tuning_steps_hz: &[10, 100, 1000],
        rx_range: FrequencyRange::new(500_000, 30_000_000),
        filters: FilterCapability {
            if_shift_hz: Some(1000),
            widths_hz: Some(&[500, 2400]),
            notch: true,
        },
        meters: MeterSet::new(METERS),
        memory: Some(MemoryCapability {
            channels: RawRange::new(0, 99),
            named: false,
            stores_mode: true,
            scan: true,
        }),
        menu: Some(MenuCapability {
            item_count: 50,
            writable: true,
        }),
        signal: SignalSupport::None,
        cw: Some(CW),
    };

    // Synthetic, like RADIO: no real radio's numbers, so nothing here is
    // tuned to pass. Stepped pitch, a keyer that sends text with a busy
    // flag and no stop.
    const CW_SEND: CwSendText = CwSendText {
        max_chunk_chars: 20,
        charset: "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789/?=+",
        pacing: CwPacing::BufferFlag,
        abort: CwAbort::DrainOnly,
        requires_break_in: None,
    };
    const CW_GATE: TxGateCapability = TxGateCapability {
        low_power_raw: 5,
        low_power_label: "5 W",
        power_raw: RawRange::new(5, 100),
    };
    const CW: CwCapability = CwCapability {
        pitch: Some(HzRange::stepped(400, 1000, 50)),
        pitch_writable: true,
        keyer_wpm: Some(WpmRange::new(10, 60)),
        keyer_wpm_writable: true,
        break_in: &[BreakInKind::Off, BreakInKind::Semi],
        send_text: Some(CW_SEND),
        tx_gate: Some(CW_GATE),
    };

    #[test]
    fn raw_range_fraction_spans_zero_to_one() {
        let r = RawRange::new(0, 30);
        assert_eq!(r.fraction(0), 0.0);
        assert_eq!(r.fraction(30), 1.0);
        assert_eq!(r.fraction(15), 0.5);
    }

    #[test]
    fn raw_range_fraction_clamps_out_of_range_readings() {
        // A radio reporting outside its own declared range is a real
        // possibility (firmware quirks, a mode the manual does not cover).
        // A UI must not be asked to draw a bar at 1.4.
        let r = RawRange::new(0, 30);
        assert_eq!(r.fraction(u16::MAX), 1.0);

        let offset = RawRange::new(10, 20);
        assert_eq!(offset.fraction(0), 0.0);
        assert_eq!(offset.fraction(10), 0.0);
        assert_eq!(offset.fraction(20), 1.0);
    }

    #[test]
    fn raw_range_fraction_survives_a_degenerate_range() {
        // min == max would divide by zero. Guarding it here means no
        // consumer has to.
        assert_eq!(RawRange::new(5, 5).fraction(5), 0.0);
        assert_eq!(RawRange::new(9, 2).fraction(5), 0.0);
    }

    #[test]
    fn the_same_reading_scales_differently_per_radio() {
        // The reason MeterDescriptor carries a range at all. A raw 15 is
        // mid-scale on a 0-30 S-meter and nearly nothing on a 0-255 one;
        // one shared widget gets both right only by asking.
        let coarse = RawRange::new(0, 30);
        let fine = RawRange::new(0, 255);
        assert_eq!(coarse.fraction(15), 0.5);
        assert!(fine.fraction(15) < 0.06);
    }

    #[test]
    fn frequency_range_is_inclusive_at_both_ends() {
        let r = FrequencyRange::new(500_000, 30_000_000);
        assert!(r.contains(500_000));
        assert!(r.contains(30_000_000));
        assert!(!r.contains(499_999));
        assert!(!r.contains(30_000_001));
    }

    #[test]
    fn a_shared_port_serves_a_role_it_does_not_natively_hold() {
        // The TS-570D shape: one RS-232C handle doing CAT and keying at
        // once. `find` answers "which endpoint IS this role"; `serves`
        // answers "can this radio do this role at all", and they differ
        // exactly here.
        assert!(RADIO.endpoints.serves(EndpointRole::Cat));
        assert!(RADIO.endpoints.serves(EndpointRole::Keying));
        assert!(RADIO.endpoints.find(EndpointRole::Keying).is_none());
        assert_eq!(RADIO.endpoints.handle_count(), 1);
    }

    #[test]
    fn an_absent_role_is_absent_both_ways() {
        assert!(!RADIO.endpoints.serves(EndpointRole::Audio));
        assert!(RADIO.endpoints.find(EndpointRole::Audio).is_none());
    }

    #[test]
    fn mode_support_is_answered_from_the_table_not_from_the_model_name() {
        assert!(RADIO.supports_mode(ModeId::Lsb));
        assert!(!RADIO.supports_mode(ModeId::C4fm));
        assert_eq!(RADIO.mode(ModeId::Lsb).unwrap().label, "LSB");
        assert!(RADIO.mode(ModeId::C4fm).is_none());
    }

    #[test]
    fn sideband_is_absent_where_the_question_does_not_apply() {
        assert_eq!(
            RADIO.mode(ModeId::Lsb).unwrap().sideband,
            Some(Sideband::Lower)
        );
        assert_eq!(RADIO.mode(ModeId::Fm).unwrap().sideband, None);
    }

    #[test]
    fn meters_are_looked_up_by_kind_and_carry_their_own_scale() {
        assert!(RADIO.meters.has(MeterKind::S));
        assert!(!RADIO.meters.has(MeterKind::Comp));

        let s = RADIO.meters.find(MeterKind::S).unwrap();
        assert_eq!(s.raw_range, RawRange::new(0, 30));
        assert!(!s.active_on_transmit);

        let swr = RADIO.meters.find(MeterKind::Swr).unwrap();
        assert_eq!(swr.raw_range, RawRange::new(0, 255));
        assert!(swr.active_on_transmit);
    }

    #[test]
    fn optional_subsystems_are_absent_rather_than_empty() {
        // `None` and "present but zero-sized" are different states, and a
        // UI must be able to tell them apart: no memory system at all is
        // not the same as a memory system with no channels stored.
        assert!(RADIO.memory.is_some());
        assert_eq!(RADIO.memory.unwrap().channels, RawRange::new(0, 99));

        let no_memory = RadioCapabilities {
            memory: None,
            menu: None,
            ..RADIO
        };
        assert!(no_memory.memory.is_none());
        assert!(no_memory.menu.is_none());
    }

    #[test]
    fn filter_features_are_independently_optional() {
        // A radio with IF shift but no selectable widths must be
        // expressible, or a UI will offer a dead width control.
        let shift_only = FilterCapability {
            if_shift_hz: Some(1000),
            widths_hz: None,
            notch: false,
        };
        assert!(shift_only.if_shift_hz.is_some());
        assert!(shift_only.widths_hz.is_none());
    }

    // -----------------------------------------------------------------
    // CW (ADR 0023 §1)
    // -----------------------------------------------------------------

    #[test]
    fn a_stepped_pitch_range_holds_only_its_steps() {
        let r = HzRange::stepped(400, 1000, 50);
        assert!(r.contains(400));
        assert!(r.contains(1000));
        assert!(r.contains(650));
        assert!(!r.contains(675), "between steps");
        assert!(!r.contains(350));
        assert!(!r.contains(1050));
    }

    #[test]
    fn a_continuous_pitch_range_holds_every_hz_inside_it() {
        let r = HzRange::continuous(300, 900);
        assert_eq!(r.step_hz, None);
        assert!(r.contains(300));
        assert!(r.contains(677));
        assert!(r.contains(900));
        assert!(!r.contains(299));
        assert!(!r.contains(901));
    }

    #[test]
    fn a_zero_step_is_treated_as_continuous_not_as_a_division_by_zero() {
        let r = HzRange {
            min_hz: 300,
            max_hz: 900,
            step_hz: Some(0),
        };
        assert!(r.contains(301));
    }

    #[test]
    fn wpm_range_is_inclusive_at_both_ends() {
        let r = WpmRange::new(10, 60);
        assert!(r.contains(10));
        assert!(r.contains(60));
        assert!(!r.contains(9));
        assert!(!r.contains(61));
    }

    #[test]
    fn send_text_folds_case_and_always_allows_a_space() {
        assert!(CW_SEND.accepts("CQ CQ DE N0CALL"));
        assert!(CW_SEND.accepts("cq de n0call k"));
        assert!(CW_SEND.accepts(""));
        assert_eq!(CW_SEND.rejected_char("TNX FER QSO"), None);
    }

    #[test]
    fn send_text_names_the_first_character_the_keyer_cannot_send() {
        assert_eq!(CW_SEND.rejected_char("HI!"), Some('!'));
        assert_eq!(CW_SEND.rejected_char("A,B!"), Some(','));
        // Not a space: a tab or newline would be sent as nothing, or as
        // something the operator did not mean.
        assert_eq!(CW_SEND.rejected_char("A\tB"), Some('\t'));
        assert_eq!(CW_SEND.rejected_char("A\nB"), Some('\n'));
        // Case folding is ASCII only: no Unicode letter sneaks in by
        // upper-casing into something in the charset.
        assert_eq!(CW_SEND.rejected_char("ß"), Some('ß'));
        assert!(!CW_SEND.accepts("HI!"));
    }

    #[test]
    fn a_coherent_cw_description_validates() {
        assert_eq!(CW.validate(), Ok(()));
        assert_eq!(RADIO.cw.unwrap().validate(), Ok(()));
    }

    #[test]
    fn a_radio_that_can_send_must_say_how_power_is_clamped() {
        let ungated = CwCapability {
            tx_gate: None,
            ..CW
        };
        assert_eq!(
            ungated.validate(),
            Err(CwCapabilityError::SendWithoutTxGate)
        );
    }

    #[test]
    fn a_read_only_cw_radio_needs_no_tx_gate() {
        let read_only = CwCapability {
            send_text: None,
            tx_gate: None,
            ..CW
        };
        assert_eq!(read_only.validate(), Ok(()));
    }

    #[test]
    fn the_low_power_setting_must_be_a_power_the_radio_reports() {
        let bad = CwCapability {
            tx_gate: Some(TxGateCapability {
                low_power_raw: 0,
                ..CW_GATE
            }),
            ..CW
        };
        assert_eq!(bad.validate(), Err(CwCapabilityError::LowPowerOutOfRange));
    }

    #[test]
    fn inverted_ranges_are_refused() {
        let pitch = CwCapability {
            pitch: Some(HzRange::continuous(900, 300)),
            ..CW
        };
        assert_eq!(pitch.validate(), Err(CwCapabilityError::EmptyPitchRange));
        let wpm = CwCapability {
            keyer_wpm: Some(WpmRange::new(60, 10)),
            ..CW
        };
        assert_eq!(wpm.validate(), Err(CwCapabilityError::EmptyWpmRange));
    }

    #[test]
    fn a_writable_setting_must_have_a_range_to_write_into() {
        let pitch = CwCapability { pitch: None, ..CW };
        assert_eq!(
            pitch.validate(),
            Err(CwCapabilityError::WritableWithoutRange)
        );
        let wpm = CwCapability {
            keyer_wpm: None,
            ..CW
        };
        assert_eq!(wpm.validate(), Err(CwCapabilityError::WritableWithoutRange));
    }

    #[test]
    fn a_send_description_must_be_usable() {
        let zero_chunk = CwCapability {
            send_text: Some(CwSendText {
                max_chunk_chars: 0,
                ..CW_SEND
            }),
            ..CW
        };
        assert_eq!(zero_chunk.validate(), Err(CwCapabilityError::ZeroChunk));

        let lower = CwCapability {
            send_text: Some(CwSendText {
                charset: "abc",
                ..CW_SEND
            }),
            ..CW
        };
        assert_eq!(
            lower.validate(),
            Err(CwCapabilityError::CharsetNotUppercase)
        );

        let empty = CwCapability {
            send_text: Some(CwSendText {
                charset: "",
                ..CW_SEND
            }),
            ..CW
        };
        assert_eq!(empty.validate(), Err(CwCapabilityError::EmptyCharset));
    }

    #[test]
    fn a_required_break_in_must_be_one_cat_can_select() {
        let needs_full = CwCapability {
            send_text: Some(CwSendText {
                requires_break_in: Some(BreakInKind::Full),
                ..CW_SEND
            }),
            ..CW
        };
        assert_eq!(
            needs_full.validate(),
            Err(CwCapabilityError::RequiredBreakInNotSelectable)
        );
        let needs_semi = CwCapability {
            send_text: Some(CwSendText {
                requires_break_in: Some(BreakInKind::Semi),
                ..CW_SEND
            }),
            ..CW
        };
        assert_eq!(needs_semi.validate(), Ok(()));
    }

    #[test]
    fn a_radio_with_no_cw_over_cat_says_none() {
        let no_cw = RadioCapabilities { cw: None, ..RADIO };
        assert!(no_cw.cw.is_none());
        assert!(RADIO.cw.is_some());
    }

    #[test]
    fn break_in_kinds_have_stable_wire_names() {
        // T2 mirrors these onto the native protocol; pin them here so the
        // wire never changes by accident.
        let names: Vec<String> = [BreakInKind::Off, BreakInKind::Semi, BreakInKind::Full]
            .iter()
            .map(|k| serde_json::to_string(k).unwrap())
            .collect();
        // PascalCase, like every other capability enum (`ModeId`,
        // `ModeKind`) that already rides in `CapabilitiesWire`.
        assert_eq!(names, ["\"Off\"", "\"Semi\"", "\"Full\""]);
        let back: BreakInKind = serde_json::from_str("\"Semi\"").unwrap();
        assert_eq!(back, BreakInKind::Semi);
    }
}
