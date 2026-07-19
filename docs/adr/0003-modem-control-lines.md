# 3. `ModemControlLines`: a separate, additive capability trait for RTS/DTR/CTS/DSR/DCD

Date: 2026-07-18

## Status

Accepted

## Context

The sibling repository `ft991a` (Yaesu FT-991A radio control, depends on
this repo's crates) needed a way to assert and read RS-232 modem control
lines — RTS, DTR (outputs) and CTS, DSR, DCD (inputs) — independent of the
byte-level CAT framing `Transport`/`CatSession` already handle.

The driver is real, manual-documented FT-991A behavior: CAT menu item 060
"PC KEYING" (`OFF`/`DAKY`/`RTS`/`DTR`) lets the radio watch the serial
connection's RTS or DTR hardware line for real-time CW key-down, as an
alternative to a CAT command. This is not FT-991A-specific machinery,
though — any serial-connected radio could use direct modem-line control
(`ts570d`'s own `SerialPort::open` already asserted RTS+DTR high at
construction, for reasons unrelated to CAT framing), so per this repo's own
boundary rule (ADR 0001: generic, reusable capability belongs in
`cat-transport-core`/`cat-transport-serial`, not duplicated in a radio
crate), this is transport-layer infrastructure, not something `ft991a`
should implement locally.

The existing `Transport`/`CatSession` traits are the wrong place for it:
they're implemented by every transport, including `cat-transport-tcp`,
`cat-transport-udp`, and every `ScriptedCatSession`/test double — none of
which have a physical serial line to control. Adding `set_rts`/`read_cts`/
etc. to `Transport` or `CatSession` directly would force every non-serial
implementor to either provide a real implementation for a concept that
doesn't apply to it, or return an error/no-op that misrepresents its own
capabilities.

## Decision

**A new, separate trait, `ModemControlLines`, defined in
`cat-transport-core` (`src/modem.rs`) and never folded into `Transport` or
`CatSession`:**

```rust
pub trait ModemControlLines {
    fn set_rts(&self, asserted: bool) -> Result<(), TransportError>;
    fn set_dtr(&self, asserted: bool) -> Result<(), TransportError>;
    fn read_cts(&self) -> Result<bool, TransportError>;
    fn read_dsr(&self) -> Result<bool, TransportError>;
    fn read_dcd(&self) -> Result<bool, TransportError>;
}
```

- **Additive, not universal.** A consumer that wants this capability bounds
  its own methods on `S: CatSession + ModemControlLines` rather than
  requiring it as part of `CatSession` itself. `cat-transport-tcp`,
  `cat-transport-udp`, `cat-server`, and `cat-transport-core`'s own
  `ScriptedCatSession` implement none of it, and don't need to — they
  simply aren't candidates for `S: ... + ModemControlLines`-bounded code.
- **Plain sync `fn`s, not `#[async_trait]`.** These are direct `ioctl(2)`
  calls with no I/O wait — matching the precedent already set by
  `Transport::flush_rx`/`CatSession::flush_rx` (both plain sync fns on
  otherwise-async traits, for the identical reason).
- **Errors reuse `TransportError`** (specifically its existing
  `Io(#[from] std::io::Error)` variant) rather than a new error type —
  `std::io::Error::last_os_error()` after a failed `ioctl` fits exactly, and
  a second error type for what is, at the OS level, still just an I/O
  failure would be unjustified.

**Implemented for `SerialPort` in `cat-transport-serial`** (`io_uring.rs`),
using `TIOCMBIS`/`TIOCMBIC` (set/clear a bit) and `TIOCMGET` (read the
status register) — generalizing the one-time RTS+DTR-high assert
`SerialPort::open` already performed at construction into runtime `&self`
methods. A blanket delegating `impl<T: Transport + ModemControlLines>
ModemControlLines for SerialCatSession<T>` in `session.rs` forwards every
method to the wrapped transport, mirroring `SerialCatSession`'s existing
`CatSession::flush_rx` → `Transport::flush_rx` delegation shape exactly.

**`SerialConfig` gained `initial_rts: bool` / `initial_dtr: bool`** (both
default `true`), so `SerialPort::open`'s unconditional RTS+DTR-high assert
becomes optional rather than staying hardcoded — now that a second consumer
wants active runtime control over RTS specifically for keying, where the
idle/asserted polarity at connect time matters. Defaulting both to `true`
preserves every existing caller's behavior exactly; only a caller that
explicitly opts out sees any change.

## Consequences

- `cat-transport-core`, `cat-transport-serial` are the only two crates in
  this workspace that know about `ModemControlLines`. `cat-transport-tcp`,
  `cat-transport-udp`, `cat-client`, `cat-server`, and `cat-framework`
  neither implement nor depend on it.
- A radio crate (or application) that wants modem-line control must
  construct its session type against a concrete `S: ModemControlLines`
  implementor (today, only `SerialPort`/`SerialCatSession<SerialPort>`) —
  it cannot be reached generically through `CatSession` alone. This is
  intentional: it keeps the capability visible in the type system rather
  than hidden behind a runtime capability check or an `Option`-returning
  method on the base trait.
- The real-hardware `ioctl` success path (bits actually toggling on a live
  serial port) is not verifiable inside this project's sandboxed test
  environment — a Linux PTY does not implement `TIOCMGET`/`TIOCMBIS`/
  `TIOCMBIC` (confirmed empirically, returns `ENOTTY`). Tests instead prove
  (a) the real `ioctl` path is reached and fails predictably against a PTY,
  and (b) `SerialCatSession`'s delegation forwards correctly, using a
  `Cell`-backed fake `Transport + ModemControlLines`. The success path on
  real hardware is an acknowledged gap, not a claimed guarantee.
- If a future TCP/UDP-based "virtual modem lines" concept is ever needed
  (e.g. a server-mode remote-PTT protocol extension), it does not extend
  this trait — `ModemControlLines` names real RS-232 hardware lines
  specifically. A different capability, and likely a different trait, would
  be the correct home for that.
