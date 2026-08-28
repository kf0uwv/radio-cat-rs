# 9. Generalize `cat-framework` over a `CatWireFormat` type parameter, for binary/addressed protocols (Icom CI-V)

Date: 2026-08-09

## Status

Accepted

## Context

A third radio, the Icom IC-7100, is about to be scaffolded (sibling repo
`ic7100`, following the `ft991a` template — same `radio`/`emulator`/
`server`/`ui` crate split, same ADR-gated process). Its CAT protocol is
**Icom CI-V**, architecturally different from both Kenwood's (`ts570d`) and
Yaesu's (`ft991a`) schemes this workspace has built against so far:

| | Kenwood/Yaesu ASCII CAT | Icom CI-V |
|---|---|---|
| Frame | `"<2-char code><params>;"` | `FE FE <to> <from> <cmd> [subcmd] [data] FD` |
| Command identity | 2-char ASCII string code | 1-byte `cmd`, optional 1-byte `subcmd` (nested) |
| Data encoding | ASCII digits | BCD-encoded binary |
| Terminator | `;` (single byte, unambiguous) | `FD` (but `FE`/`FD` also appear as bus preamble/data — needs a real scanner, not a byte match) |
| Addressing | none — point-to-point | every frame carries controller/radio addresses; multi-drop bus; **the radio's own CI-V address is a user-configurable runtime setting (factory default `0x88`, adjustable `01`-`DF`), not a fixed protocol fact** |
| Echo | none | the radio's USB-CI-V bridge echoes the outbound bytes before the real reply |

Auditing the actual code (not just the docs — the ADR 0001 "generic CAT
engine" framing turns out to be generic *only* across ASCII-line protocols)
found four places hard-coded to the ASCII shape above:

1. `cat-framework::CommandTable::parse` (`cat-framework/src/cat.rs`) —
   `frame.strip_suffix(';')` then `frame.split_at(2)`.
2. `cat-client::CatClient::query/query_with_param/set`
   (`cat-client/src/client.rs`) — `format!("{code}{params};")`.
3. `cat-transport-serial::SerialCatSession::execute`
   (`cat-transport-serial/src/session.rs`) — reads one byte at a time
   until `b';'`.
4. `cat-server::Broker::dispatch` (`cat-server/src/broker.rs`) — requires
   `std::str::from_utf8(request)` to succeed.

Confirmed unaffected either way (byte-level, no framing opinion at all):
`cat-transport-core::Transport`/`ModemControlLines`/`NoModemControlLines`,
`cat-transport-serial::SerialPort` (both platform backends),
`cat-transport-tcp`/`cat-transport-udp`, and `cat-rigctl::RigctlRadio`
(already a fully abstract typed get/set trait — never touches wire bytes,
needs no change regardless of how this ADR resolves).

**Revision history of this ADR, while still `Proposed`:**
- Draft 1 proposed a parallel `cat-framework-civ` crate mirroring
  `cat-framework`'s shape with distinct types. Rejected: the goal is one
  adaptable type system, not two independently-maintained copies that
  happen to look alike.
- Draft 2 generalized the existing engine over a `CatWireFormat` trait, but
  modeled it as a **stateless marker type** with associated functions
  (`AsciiLineFormat::extract(frame)`, no `&self`) — mirroring how
  `AsciiLineFormat` genuinely has zero configurable state. **This was
  wrong for CI-V specifically**: a CI-V radio's own bus address and the
  controller's address are runtime configuration (see the Context table
  row above), not a fact about "the CI-V protocol class" that can be baked
  into a zero-sized compile-time marker. Modeling the format as stateless
  would have forced that address configuration to live somewhere else ad
  hoc (a side parameter threaded through every call, or worse, a global).
  This revision fixes that: format implementations own real state,
  constructed once, and the client/session/framework types that use them
  hold one instance rather than re-deriving or re-passing it per call.

## Decision

Add a `CatWireFormat` trait to `cat-framework`, implemented by types that
own whatever state their protocol actually needs (none, for
`AsciiLineFormat`; a configured address pair, for `CivFormat`). The engine
types (`CommandTable`, `CatClient`, `SerialCatSession`, `CatFramework`,
`Broker`) each hold **one instance**, set once at construction — call
sites (`query`/`set`/`execute`/`process_frame`) never see or pass a format
value themselves. Every generic parameter this adds defaults to
`AsciiLineFormat`, so `ts570d`/`ft991a` should need zero source changes
(verify with an early compile-check spike before relying on it).

```rust
/// A concrete CAT wire protocol: how a command is identified, and how
/// requests/responses are encoded. Takes `&self` (not an associated
/// function) because a real protocol can carry configuration — CI-V's
/// bus/controller addresses, for instance — that a stateless marker type
/// cannot represent.
pub trait CatWireFormat: 'static {
    /// `&'static str` for ASCII-line protocols; `(u8, Option<u8>)` for
    /// CI-V's cmd/subcmd pair.
    type Code: Copy + Eq + core::fmt::Debug + 'static;

    /// Split one complete, already-delimited frame into its code and raw
    /// parameter/data bytes. Framing (finding a frame's boundary within a
    /// growing byte stream) is a separate concern — see `FrameScanner`.
    fn extract<'a>(&self, frame: &'a [u8]) -> Result<(Self::Code, &'a [u8]), ParseError>;

    fn encode_request(&self, code: Self::Code, params: &[u8]) -> Vec<u8>;
    fn encode_response(&self, code: Self::Code, payload: &[u8]) -> Vec<u8>;
}

/// Detects a complete frame boundary within a growing byte buffer —
/// needed by session types that read a stream incrementally
/// (`SerialCatSession`).
pub trait FrameScanner: CatWireFormat {
    fn frame_complete(&self, buffer: &[u8]) -> bool;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AsciiLineFormat;   // zero-sized; methods ignore `self` — today's
                              // exact behavior, just moved behind the trait
impl CatWireFormat for AsciiLineFormat { type Code = &'static str; /* ... */ }
impl FrameScanner for AsciiLineFormat { /* buffer.last() == Some(&b';') */ }

#[derive(Debug, Clone, Copy)]
pub struct CivFormat {
    pub radio_addr: u8,        // this radio's configured CI-V address
    pub controller_addr: u8,   // this software's own address on the bus
}
impl CivFormat {
    pub fn new(radio_addr: u8, controller_addr: u8) -> Self { Self { radio_addr, controller_addr } }
}
impl Default for CivFormat {
    /// IC-7100 factory defaults (`0x88` radio, `0xE0` controller,
    /// Icom-wide convention) — a sensible starting point, not a claim
    /// that every Icom radio uses these; callers override via `new()`
    /// when a rig's address has been changed from factory default or a
    /// second CI-V device shares the bus.
    fn default() -> Self { Self { radio_addr: 0x88, controller_addr: 0xE0 } }
}
impl CatWireFormat for CivFormat { type Code = (u8, Option<u8>); /* uses self.radio_addr/self.controller_addr */ }
impl FrameScanner for CivFormat { /* FE FE...FD scanner, echo-aware */ }
```

`CommandTable<C, F>` stays a plain `'static` definitions table — it needs
`F` only as a type tag (so `CommandDefinition::code: F::Code` has the
right shape), never an owned format *value*, so it keeps working exactly
as a compile-time `static TABLE: CommandTable<...> = CommandTable::new(...)`
today, with zero runtime-construction cost added:

```rust
pub struct CommandDefinition<C: CommandId, F: CatWireFormat = AsciiLineFormat> {
    pub id: C,
    pub code: F::Code,                          // was &'static str
    pub name: &'static str,
    pub description: &'static str,
    pub query_forms: &'static [CommandForm],     // UNCHANGED — already
    pub set_forms: &'static [CommandForm],       // length/kind-based, not
    pub action_forms: &'static [CommandForm],    // wire-encoding-specific
    pub response_forms: &'static [CommandForm],
    pub readable: bool,
    pub writable: bool,
}
pub struct CommandTable<C: CommandId, F: CatWireFormat = AsciiLineFormat> { /* &'static [CommandDefinition<C, F>] */ }
impl<C: CommandId, F: CatWireFormat> CommandTable<C, F> {
    pub fn find(&self, code: F::Code) -> Option<&'static CommandDefinition<C, F>>;   // no format instance needed — Eq on Code
    pub fn parse<'a>(&'static self, format: &F, frame: &'a [u8]) -> Result<CommandRequest<'a, C, F>, ParseError> {
        let (code, parameters) = format.extract(frame)?;
        let definition = self.find(code).ok_or_else(...)?;
        // operation classification (Query/Set/Action + selector-read) —
        // UNCHANGED logic, still driven by parameters.len().
        ...
    }
}
```

`parse` takes `format: &F` as a parameter, but the only caller is
`CatClient`/`CatFramework`, each of which already owns its one `format: F`
instance — from outside those types, nothing about calling `.query(...)`
or `.process_frame(...)` changes:

```rust
pub struct CatClient<C: CommandId, S: CatSession, F: CatWireFormat = AsciiLineFormat> {
    session: S,
    table: &'static CommandTable<C, F>,
    format: F,   // constructed once
}
impl<C: CommandId, S: CatSession, F: CatWireFormat + Default> CatClient<C, S, F> {
    /// Unchanged signature — works because `AsciiLineFormat: Default`.
    /// Existing `ts570d`/`ft991a` call sites (`CatClient::new(session, table)`)
    /// need no edits at all.
    pub fn new(session: S, table: &'static CommandTable<C, F>) -> Self {
        Self::with_format(session, table, F::default())
    }
}
impl<C: CommandId, S: CatSession, F: CatWireFormat> CatClient<C, S, F> {
    pub fn with_format(session: S, table: &'static CommandTable<C, F>, format: F) -> Self { Self { session, table, format } }

    pub async fn query(&mut self, code: F::Code) -> Result<String, ClientError<S::Error>> {
        let wire = self.format.encode_request(code, &[]);   // format never appears at the call site
        ...
    }
}
```

`ic7100` constructs once with the real address —
`CatClient::with_format(session, &IC7100_COMMAND_TABLE, CivFormat::new(0x88, 0xE0))`
— and every subsequent `.query(...)`/`.set(...)` call looks exactly like
the ASCII radios' call sites. This is the actual fix this revision makes:
**the format is a construction-time concern of the client, not a per-call
concern of `query`/`set`/`parse`.**

The same pattern (own one `format: F`, expose a `Default`-backed `new()`
plus an explicit `with_format()`) applies to:

- `cat-transport-serial::SerialCatSession<T, F: FrameScanner = AsciiLineFormat>`
  — owns `format: F`, checks `self.format.frame_complete(&response)` per
  byte instead of `buf[0] == b';'`. `Transport`/`ModemControlLines`/
  `SerialPort` themselves stay completely untouched.
- `cat-framework::CatFramework<R, F: CatWireFormat = AsciiLineFormat>` —
  owns `format: F`, passes `&self.format` into `table.parse(...)` inside
  `process_frame`.
- `cat-server::Broker<C, S, F: CatWireFormat = AsciiLineFormat>` — owns
  `format: F`; `dispatch` drops the `str::from_utf8` gate (operates on
  `&[u8]` unconditionally) and uses `self.format` wherever it currently
  calls into `CommandTable::parse`.
- `cat-rigctl::RigctlRadio` — no change of any kind, at any revision of
  this ADR. It's already a fully abstract typed get/set trait that never
  sees wire bytes.

`CommandForm`, `CommandOperation`, `ResponseDisposition`,
`ProtocolErrorKind`, `CommandOutcome<E>` — **zero changes**, at any
revision of this ADR. They were already format-agnostic.

### Verified against real call sites — two gaps found and closed

Before finalizing, audited every `process_frame`/`ParameterValues::raw`/
`ResponseBuilder::push_wire_value`/`.finish` call site in `ts570d` and
`ft991a` (not just their tests — their `radio/src/{ts570d,ft991a}_radio.rs`
files, i.e. the actual command-table `handle_command` match arms
themselves, which call these on every one of their 91–150+ commands).
This found two places the "zero source changes" claim would otherwise have
quietly broken:

1. **`process_frame` is called with `&str` literals at ~150+ sites**
   (`framework.process_frame("FA;", &mut output)`, both apps' test
   suites). A concrete `&[u8]` parameter breaks every one — no auto-
   coercion from `&str`. **Fix**: `process_frame(frame: impl AsRef<[u8]>,
   output: &mut Vec<u8>)`. `str` already implements `AsRef<[u8]>` in
   `std`, so every existing literal call site keeps compiling unchanged,
   and `CivFormat` callers pass real `&[u8]` frames through the same bound
   (`[u8]: AsRef<[u8]>` is also in `std`).

2. **`ParameterValues::raw()` and `ResponseBuilder::push_wire_value`/
   `finish` are called *inside the command-handling logic itself***
   (`ft991a_radio.rs`'s `handle_command`, not just tests) — thousands of
   call sites across both radios' full command surfaces, all assuming
   `&str` in, `&str` out. Naively retyping these to `&[u8]` (as an earlier
   revision of this ADR implied) would mean rewriting every command
   handler in two already-shipping radios — not a "mechanical adjustment."
   **Fix**: make `ParameterValues<'a, F: CatWireFormat = AsciiLineFormat>`
   and `ResponseBuilder<'a, F: CatWireFormat = AsciiLineFormat>` generic,
   but give the `AsciiLineFormat` instantiation a **specialized inherent
   `impl` block reproducing today's exact API, unchanged**:

   ```rust
   pub struct ParameterValues<'a, F: CatWireFormat = AsciiLineFormat> {
       raw: &'a [u8],
       _format: core::marker::PhantomData<F>,
   }
   // Generic — new capability, not used by existing ASCII code:
   impl<'a, F: CatWireFormat> ParameterValues<'a, F> {
       pub fn raw_bytes(&self) -> &'a [u8] { self.raw }
   }
   // AsciiLineFormat specialization — byte-for-byte IDENTICAL surface to
   // today's ParameterValues: same method names, same &str/u64 shapes.
   impl<'a> ParameterValues<'a, AsciiLineFormat> {
       pub fn raw(&self) -> &'a str { /* as today */ }
       pub fn unsigned(&self) -> Result<u64, ParameterAccessError> { /* as today */ }
   }
   ```

   Same pattern for `ResponseBuilder<'a, AsciiLineFormat>`
   (`push_wire_value(&str)`, `finish()`, `write_complete(&str)` — all
   unchanged) versus a `ResponseBuilder<'a, CivFormat>` specialization with
   a genuinely different, binary-frame-oriented API (it needs the format
   instance and the command's code to assemble `FE FE...FD` on `finish()`
   — exact shape decided during `CivFormat` implementation, not here).
   Rust allows specializing inherent impls per concrete type parameter, so
   this is legal and idiomatic, not a workaround.

   `Ft991aRadio`'s existing `impl CatRadio for Ft991aRadio { fn
   handle_command(&mut self, request: CommandRequest<'_, Self::CommandId>,
   ...) }` doesn't need its signature touched either — `CatRadio<F =
   AsciiLineFormat>`'s default absorbs the elided parameter, so the impl
   still resolves to `CatRadio<AsciiLineFormat>`, and every `request.
   parameters.raw()` call inside its body keeps hitting the
   `AsciiLineFormat`-specialized method with identical behavior.

This is exactly what the earlier "verify with an early compile-check
spike" consequence was for — static analysis alone (no compiler needed
yet) already found two real gaps and produced concrete fixes for both.

**The compile-check spike itself then caught a third gap static analysis
missed**: `CommandTable::find` originally took `code: F::Code` (`&'static
str` for `AsciiLineFormat`) — but this crate's pre-ADR-0009 `find` accepted
a plain `&str` of *any* lifetime (compared by content against the table's
`'static` entries, the same cross-lifetime `PartialEq` mechanism
`find_command` uses). Forcing it to `F::Code` exactly was a real narrowing,
caught only by actually building `ts570d` against this change:
`emulator/src/tui.rs::lookup_description(code: &str)` calls
`TS570D_COMMAND_TABLE.find(code)` with a borrowed, non-`'static` `&str`,
and failed to compile with the narrowed signature. Fixed by making `find`
generic over the lookup value (`fn find<Q>(&self, code: Q) -> ... where
F::Code: PartialEq<Q>`) — restoring the original any-lifetime behavior for
`AsciiLineFormat` while staying meaningful for `CivFormat` (`Q =
(u8, Option<u8>)`, trivially `'static` already). Confirmed by building and
running the **entire** test suite of both `ts570d` (436 tests: 277 unit +
100 real end-to-end PTY-backed integration tests + others) and `ft991a`
(1125+ tests, including 103 end-to-end integration tests) against a
temporary local `[patch]` pointing at this branch — zero source changes to
either app, every test green, patch reverted immediately after. This is the
actual proof the "zero source changes" consequence below claims, not an
assumption.

### Explicitly out of scope for this ADR

- `cat-diagnostics` — neither `ts570d` nor `ft991a` actually uses it (both
  hand-roll full-parity diagnostics per their own ADR 0006/0007); `ic7100`
  is expected to do the same.
- A shared BCD-encoding helper module for `CivFormat`'s own internals —
  worth extracting once IC-7100's actual command table shows the real
  repetition, not speculatively now. CI-V is common to Icom's entire
  current lineup (IC-7300, IC-9700, IC-705, ...), so `CivFormat` pays for
  itself again the moment a second Icom radio is added.

## Consequences

- One engine, not two, and the wire format is now a **constructed value**,
  not a bare type marker — `CivFormat`'s address configuration has a real
  place to live (an owned field, set once) instead of needing to be
  threaded through every method call or stashed somewhere ad hoc.
- `ts570d`/`ft991a` need **zero source changes** — every generic parameter
  defaults to `AsciiLineFormat`, and every `new()` constructor defaults to
  `AsciiLineFormat::default()` via the `F: Default` bound. **Confirmed**,
  not assumed: both apps' full workspaces (all crates, all tests, including
  real end-to-end PTY-backed integration suites) were built and run against
  this branch via a temporary local `[patch]`, immediately reverted after —
  see the "Verified against real call sites" section above for the full
  results and the one additional (`CommandTable::find`) gap this caught
  that static analysis alone had missed.
- `ic7100` calls one extra constructor argument (`CivFormat::new(radio_addr,
  controller_addr)`) at the point it builds its `CatClient`/
  `SerialCatSession`/`CatFramework` — everywhere else in its `radio`/`ui`/
  `emulator`/`server` code, method call sites look identical in shape to
  `ft991a`'s.
- `ic7100` cannot begin real `radio/` implementation until `CivFormat` and
  the generalized engine exist and are tagged — mirrors this workspace's
  standing rule that a radio crate never depends on an engine that doesn't
  exist yet (see `ft991a` ADR 0001's precondition-gating).
- Echo-handling, multi-drop bus address-collision behavior, and whether to
  surface CI-V Transceive's unsolicited broadcast frames at all in
  `CivFormat`/`SerialCatSession` are left as open implementation questions
  for the actual dispatch tasks, not resolved here — this ADR fixes the
  generic shape and where format state lives, not every CI-V wire edge
  case.
