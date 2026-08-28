# 10. A radio capability model, multi-endpoint transports, and a normalized `SpectrumSource`, served by a native protocol with rigctl as a compatibility layer

Date: 2026-08-27

## Status

**Accepted** (2026-08-27) — user sign-off; implementation authorized via
`planning/architect/task_plan.md` (Tasks 11-17). No code has been written yet.
Revision 3 (2026-08-27) after user review: adds multi-endpoint transport
capability and delegated per-type spectrum settings; makes the native
protocol primary with **rigctl as a compatibility layer** reimplemented over
the capability model; closes the native-bandscope verification gap.

## Context

Three radios are in scope: `ts570d` (Kenwood ASCII CAT), `ft991a` (Yaesu
ASCII CAT), and `ic7100` (Icom CI-V, scaffolded by
[ADR 0009](0009-civ-engine-for-binary-addressed-protocols.md) but not yet
built — its repo currently contains only `docs/`). A GPU-rendered desktop
console is now wanted, with a live spectrum and waterfall.

The stated direction is that the core library does more, and that the TUIs
and GUIs stay radio-specific while *delegating* to known serial and network
protocols. This ADR is the library half of that: everything a UI would
otherwise have to hardcode per radio, made into data the library serves.

### A radio is not one serial port

`ft991a`'s [ADR 0002](https://github.com/kf0uwv/ft991a/blob/main/docs/adr/0002-rts-dtr-ptt-cw-keying.md)
established that the FT-991A's USB interface is a **Silicon Labs Dual CP210x
bridge presenting two virtual COM ports** — one ("Enhanced") carrying CAT,
the other ("Standard") carrying RTS/DTR-based PTT and CW keying. Separately,
the radio presents a **USB audio codec** used for digital modes, which the
CAT-only application never opens.

That ADR deferred the whole area with an explicit condition:

> **USB dual-port support is explicitly out of scope, not designed
> speculatively.** [...] Revisit if/when USB dual-port support is actually
> requested.

It has now been requested. That ADR also sketched the shape of the fix — a
second, independent handle supplied apart from `S: CatSession`, either as a
generic `M: ModemControlLines` parameter or injected at construction.

The consequence for this ADR: a radio's transport is a **set of endpoints
with roles**, not a port. A capability model that assumes one port cannot
describe the FT-991A honestly.

### The spectrum does not come from the radio — and "signal over USB" is two different things

The FT-710 console being benchmarked against gets its waterfall for free
from a built-in bandscope. **The TS-570D has no bandscope at all.** Its only
spectrum path is the CN4 IF tap (TX-RX Unit (RF), pin 1 = OUT, coupled
through C100, 1 pF, off the Q12/L54 first-IF node) feeding an RTL-SDR parked
on the 73.05 MHz first IF, carrying three corrections that are properties of
*the radio*:

| Correction | TS-570D value | Why |
|---|---|---|
| Fixed IF centre | 73.05 MHz | LO1 runs 73.05–103.05 MHz over 0–30 MHz RX |
| Spectrum inversion | always inverted | high-side LO1 ⇒ `IF = 73.05 + dial − RF` |
| Constant frequency trim | one calibrated Hz value | the tuner never retunes, so dongle crystal error is a fixed Hz offset, not a ppm scaling |

A working shell prototype of that coupling exists at
`ts570d/if-panadapter-bridge.py`. It is a spike. If that math lands in a UI,
it lands in every UI, per radio, forever.

Two genuinely different things travel under "signal over USB", and
conflating them would produce a wrong abstraction:

- **A band panorama** — hundreds of kHz to MHz wide. Comes from a native
  bandscope over CAT, or from an SDR (IF tap or direct). This is what fills
  a waterfall.
- **Receiver audio** — a USB audio codec, AF bandwidth only (roughly
  3 kHz). This can drive an AF FFT and an AF oscilloscope, which the
  reference console shows on its left rail. It **cannot** drive a band
  waterfall, and must never be presented as though it could.

**Verification (2026-08-27): no radio in the current fleet exports a
bandscope.** `FT-991A_CAT_OM_ENG_1711-D.pdf` (in `ft991a/docs/manuals/`)
contains no scope-data command at all — the only scope-related entries are
*settings* the radio accepts over CAT (menu 115 `SCP DISPLAY MODE`, 117
`SPECTRUM COLOR`, 004 `HOME FUNCTION`). The FT-991A renders a scope on its
own display and does not export the data. The TS-570D has no scope
hardware. The IC-7100 has no local manual to check and is understood to
have no bandscope, but that remains unverified here.

Therefore `NativeScope` is **defined but not implemented** in phase 1. It
stays in the enum because the abstraction's whole purpose is that a future
IC-7300 or FT-710 slots in without reshaping anything, but no code is
written for it speculatively.

## Decision

### 1. `cat-framework` gains a `capabilities` module

`RadioCapabilities` is plain data, negotiated once per session, describing
what the attached radio can do.

```rust
pub struct RadioCapabilities {
    pub model: &'static str,
    pub endpoints: EndpointSet,       // part 2
    pub vfos: VfoCapability,
    pub modes: &'static [ModeDescriptor],
    pub tuning_steps_hz: &'static [u32],
    pub filters: FilterCapability,
    pub meters: MeterSet,             // S, PO, SWR, ALC, ID, VDD, COMP
    pub memory: Option<MemoryCapability>,
    pub signal: SignalCapability,     // part 3
    pub menu: Option<MenuCapability>,
}
```

Every field is answerable statically per model, so the handshake costs no
round trip to the radio.

### 2. Multi-endpoint transports

```rust
pub struct EndpointSet {
    pub endpoints: &'static [EndpointDescriptor],
}

pub struct EndpointDescriptor {
    pub role: EndpointRole,
    pub required: bool,
    /// Whether this role may share a handle with another (the TS-570D's
    /// single RS-232C port is Cat + Keying on one handle; the FT-991A's
    /// USB bridge is not).
    pub shareable_with: &'static [EndpointRole],
}

pub enum EndpointRole {
    Cat,      // command/response
    Keying,   // RTS/DTR PTT and CW — FT-991A "Standard" port
    Audio,    // USB codec: RX audio in, TX audio out
}
```

This resolves `ft991a` ADR 0002's deferred question the way that ADR
predicted: `Ft991a` stops assuming "the CAT session's own transport is also
the modem-control handle," and the keying handle is supplied independently.
The TS-570D describes one endpoint filling `Cat` and `Keying` together; the
FT-991A describes two, plus `Audio`.

### 3. New crate `cat-signal` — one frame type, one source trait

```rust
/// One spectrum update, already corrected. No consumer of this type ever
/// knows about IF inversion, LO tracking, or crystal trim.
pub struct SpectrumFrame {
    pub center_hz: u64,
    pub span_hz: u32,
    pub ref_level_dbm: f32,
    pub bins: Vec<f32>,      // dBm, low frequency first, ALWAYS
    pub sequence: u64,
}

#[async_trait::async_trait(?Send)]
pub trait SpectrumSource {
    type Error;
    async fn next_frame(&mut self) -> Result<SpectrumFrame, Self::Error>;
    fn capability(&self) -> SignalCapability;
    fn settings(&self) -> SpectrumSettings;                     // part 4
    fn apply(&mut self, key: &str, value: SettingValue) -> Result<(), Self::Error>;
    /// Called when the radio's dial moves, so an IF-tap source can retrack.
    fn retune(&mut self, dial_hz: u64);
}

pub enum SignalCapability {
    None,
    /// Band panorama from the radio's own scope, over CAT.
    NativeScope { max_span_hz: u32, bins: u16 },
    /// Band panorama from an SDR on a fixed IF tap.
    IfTap(IfTapConfig),
    /// Band panorama from an SDR tuned independently.
    DirectSdr { tunable_range_hz: (u64, u64) },
    /// AF-bandwidth only. Drives an AF FFT / scope, NEVER a band waterfall.
    AudioDerived { max_bandwidth_hz: u32 },
}

/// The three TS-570D corrections from the Context table, as data.
pub struct IfTapConfig {
    pub if_center_hz: u64,   // 73_050_000
    pub inverted: bool,      // true — high-side LO1
    pub trim_hz: i32,        // calibrated once against a known carrier
}
```

`bins` is **always low-frequency-first**. Inversion is corrected inside the
source. That single invariant is what makes a TS-570D IF tap and an
IC-7300 native scope interchangeable to a consumer.

`AudioDerived` carries `max_bandwidth_hz` specifically so a UI can refuse to
render it as a band panorama. A capability that lies by omission is worse
than one that is absent.

`#[async_trait(?Send)]` matches the house binding from
[ADR 0002](0002-async-runtime-binding-for-transport-crates.md).

### 4. Delegated spectrum settings, described per source type

A source's *type* determines which knobs exist. Rather than a UI switching
on `SignalCapability` and hardcoding a panel per variant, each source
**describes its own settings** and the UI renders them generically.

```rust
pub struct SpectrumSettings { pub descriptors: Vec<SettingDescriptor> }

pub struct SettingDescriptor {
    pub key: &'static str,          // "trim_hz", "gain_db", "fft_size"
    pub label: &'static str,
    pub group: SettingGroup,        // Source | Display | Calibration
    pub access: Access,             // ReadOnly | ReadWrite
    pub value: SettingValue,
}

pub enum SettingValue {
    Int   { value: i64, min: i64, max: i64, step: i64, unit: Unit },
    Float { value: f64, min: f64, max: f64, unit: Unit },
    Bool(bool),
    Enum  { value: u16, options: &'static [&'static str] },
}
```

What each type delegates:

| Source | Typical descriptors |
|---|---|
| `IfTap` | `if_center_hz` (RO), `inverted` (RO), **`trim_hz` (RW, Calibration)**, `sdr_device`, `sample_rate`, `gain_db`, `agc`, `fft_size`, `averaging` |
| `NativeScope` | `span_hz`, `ref_level_dbm`, `sweep_speed`, `scope_mode` (centre/fixed) |
| `DirectSdr` | `center_hz`, `gain_db`, `sample_rate`, `antenna`, `fft_size` |
| `AudioDerived` | `input_device`, `fft_size`, `window`, `averaging` |

The TS-570D's `trim_hz` is the sharpest example: it is a real, per-station
calibration a user must be able to set, it exists for no other source type,
and it should never appear in a UI as a hand-written TS-570D special case.

### 5. A concrete RTL-SDR source

`cat-signal-rtlsdr` implements `SpectrumSource`: read IQ, window, FFT,
magnitude, apply `IfTapConfig`, emit `SpectrumFrame`. `retune(dial_hz)` does
what the shell prototype did with gqrx's `LNB_LO`, except no frequency ever
reaches the SDR — the dongle stays parked and `center_hz` is computed as
`dial_hz + trim_hz`. That is why the trim is a constant and no ppm
correction is needed anywhere.

**Deferred to a follow-up ADR:** the librtlsdr binding, whether the FFT runs
on a worker thread or in the frame pump, and backpressure policy.

### 6. The native protocol is primary; rigctl is a compatibility layer

The native typed protocol is the real protocol, not an extension of
something else. `cat-server` serves it on its own port: capability
handshake, typed JSON state, typed commands validated against the
capability set, and separately framed binary `SpectrumFrame`s that a client
which declines them never pays for.

**`cat-rigctl` remains exactly what its name says — a compatibility
layer.** It keeps its own port and its existing wire behaviour, so WSJT-X
and stock `rigctl` are unaffected and ADR 0005's two hard-won interop fixes
(`\dump_state`'s capability-tail field count, `F`'s `%f`-formatted float
parsing) are preserved.

What changes is what it sits *on*. Today each app hand-writes a
`RigctlRadio` impl against its own concrete radio type —
`ts570d/server/src/rigctl_radio.rs` is 269 lines, `ft991a`'s is 221, and
ADR 0005 already had to de-duplicate the layer above them. Instead,
`cat-rigctl` is reimplemented **once**, against `RadioCapabilities` and the
native command model: Hamlib's fixed lowest-common-denominator vocabulary
becomes a translation onto capability-checked native commands.

Two consequences follow directly. `rigctl_radio.rs` disappears from both
apps — a radio gains rigctl support by describing itself, not by writing a
bridge. And `\dump_state`'s capability tail is **generated** from
`RadioCapabilities` instead of hand-maintained, which is precisely where
ADR 0005's field-count bug came from.

### Explicitly out of scope for this ADR

- **Audio transport.** `EndpointRole::Audio` and `AudioDerived` describe
  that an audio endpoint *exists* and what it can feed. Actually streaming
  audio — codec selection, buffering, TX audio — is a separate design.
- **UI structure.** UIs stay radio-specific; see ADR 0011.
- **The GUI and its framework.** See `ts570d` ADR 0008.
- **Retiring `cat-rigctl`.** It remains the compatibility layer, on its own
  port, with its own wire behaviour unchanged.
- **Implementing `NativeScope`.** Defined, not built — no radio in the
  fleet exports a bandscope (see Context).

## Consequences

**Good.**

- The IF-tap correction math is written once, in the only layer that can
  know it.
- `ft991a` ADR 0002's deferred dual-port question gets a real answer instead
  of another deferral, and the FT-991A's keying and audio endpoints become
  describable.
- `rigctl_radio.rs` disappears from both apps (269 + 221 lines), and
  `\dump_state`'s capability tail becomes generated rather than
  hand-maintained — removing the class of bug ADR 0005 had to fix by hand.
- `trim_hz` and every other source-specific knob reach the user without a
  single per-radio settings panel.
- `ic7100` is not yet scaffolded, so it inherits this boundary at zero
  migration cost.

**Costs and risks.**

- `ts570d/CLAUDE.md`'s "`server` and `ui` are contractually TS-570D-shaped"
  rule must be rewritten, and the same rule in `ft991a`. Binding project
  rules; user sign-off required, not an architect's call.
- `RadioCapabilities` is a guess until three radios are described by it.
  Expect a breaking revision; apps pin to a tag, as `ts570d/Cargo.toml`
  already does at `v0.1.0`.
- Reimplementing `cat-rigctl` over the capability model is a rewrite of a
  layer that currently works, with WSJT-X interop as the thing at risk. Its
  existing conformance behaviour is the acceptance bar, and the two ADR 0005
  interop fixes are regression tests, not prose. Mitigation: the rewrite
  lands only once it passes against a live Hamlib client.
- `cat-signal` stays dependency-free (types + trait); only
  `cat-signal-rtlsdr` pulls a driver, so no existing consumer's build
  changes.
