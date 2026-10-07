# 23. CW over the native protocol: an optional `CwCapability`, an armed transmit session the server enforces, and abort that never waits behind a send

Date: 2026-10-07

## Status

**Accepted** (2026-10-07) by the architect and user, with the decisions
recorded under "Review decisions" below; where a decision differs from the
draft text, the decision governs and the text has been updated to match.
Drafted by the `cw_capability` agent. Builds on
[ADR 0010](0010-capability-model-and-normalized-signal-source.md)
(capability model, native protocol),
[ADR 0019](0019-audio-over-the-native-protocol.md) /
[ADR 0020](0020-server-authored-console-layouts.md) (additive, optional
protocol growth), [ADR 0021](0021-cat-morse.md) (`cat-morse`) and
[ADR 0022](0022-queue-bypassing-ptt-release.md) (de-keying that does not
wait for the queue).

### Review decisions (architect + user, 2026-10-07)

1. **Supervisor home:** `cat_rigctl::native_bridge::cw`. Extracting
   `native_bridge` into its own crate is not part of this ADR.
2. **FT-991A:** CW is read-only in v1 (`send_text: None`). Host-timed DTR
   keying would need its own ADR; `KM` stays rejected.
3. **TS-570D abort:** `DrainOnly` with the ~3 s chunk drain bound is
   accepted until a dummy-load bench test establishes whether `RX;` stops
   a running `KY` sentence.
4. **Callsign ID:** the **server itself sends `DE <call>`** — appended to
   the first job of an armed session, and to a job whenever 10 minutes of
   armed time have passed since the last ID. The server does not refuse a
   job for lacking the callsign (supersedes the draft's "refused, not
   rewritten"). The appended ID counts toward the key-down cap.
5. **Key-down cap:** 60 s per send; a client may lower it, never raise it.
   (Separate from `ts570d`'s 120 s PTT `MAX_KEY_DOWN`.)
6. **Low power:** a fixed radio-declared minimum (`TxGateCapability`).
7. **Mode:** arm and send are refused unless the radio is already in a CW
   mode. The server never switches modes.
8. **rigctl PTT while armed:** while a console holds the CW arm, rigctl
   `T 1` is refused. *Amended after the safety review (2026-10-07):* the
   raw TCP/UDP CAT listeners are refused too -- every frame, answered
   `ERR …` without reaching the radio -- from the arm request until the
   restore is verified, on both platforms. Raw CAT can key (`TX;`) or undo
   the clamp (`PC100;`) in each radio's own words; refusing all of it is
   the only radio-agnostic rule.
9. **Versioning:** workspace 0.9.0 for the `RadioCapabilities.cw` field;
   `PROTOCOL_VERSION` stays 2; `ErrorCode::Forbidden` is accepted.
10. **Dependency:** the new edge `cat-rigctl → cat-morse` is accepted.
11. **Polling:** CW settings are polled on the existing levels clock
    whenever `capabilities.cw` is `Some`.
12. **Renderer parity (ADR 0013):** both consoles (TUI and GUI) show CW
    state (pitch, keyer wpm, break-in, armed/sending).
13. **Added task:** `cat_native` `Connection::connect_timeout` — connect
    and handshake both bounded — needed by the standalone trainer client.

## Context

A standalone Morse trainer is wanted, in its own repository, that works
with **any** radio built on this library — `ts570d`, `ft991a`, `ic7100` —
by connecting to that radio's `server` as a native-protocol client. The
existing TS-570D-only trainer (`ts570d` branch `feat/cw-trainer`,
`src/cw/radio_defaults.rs`) opens the serial port itself and reads two
things: CW pitch (`PT`, index 0-12 → 400 + 50·i Hz) and keyer speed (`KS`).

The native protocol carries nothing about CW today. `RadioCapabilities`,
`CapabilitiesWire`, `Command` and `RadioState` have no pitch, no keyer
speed, no break-in, no way to send text, and no TX power description.

Later phases of the trainer will **transmit**: send text through the
radio's own keyer (TS-570D `KY`, IC-7100 CI-V `17`), abort it, and set
break-in — on a dummy load, behind an operator RF gate. The gate the user
has specified: a dummy-load acknowledgement every session, a callsign ID,
power forced low and read back, original power and break-in restored, a
key-down cap, and abort releasing immediately.

Three facts shape the design:

1. **The radios disagree completely about sending text** (table, §8). The
   TS-570D has a 24-character buffer with a busy flag and no documented
   stop. The IC-7100 takes 30 characters and has a real stop (`17 FF`) but
   no busy flag. The FT-991A has **no free-text CAT command at all** — only
   playback of stored keyer memories.
2. **A client cannot be trusted to un-key.** A trainer that crashes, or
   whose Wi-Fi drops, mid-sentence leaves a transmitter keyed on whatever
   the server last fed it. Anything that ends a transmission has to live
   on the server's side of the socket.
3. **The existing pump starves a sender.** `native_bridge::pump` applies
   queued commands only at the top of a cycle, polls state (several CAT
   exchanges), then sleeps — and backs off to 5 s *while transmitting*
   (`NativeShared::poll_interval`), which is exactly when a sender needs to
   run.

## Decision

### 1. `CwCapability` — an optional model fact

In `cat_framework::capabilities`, beside `MemoryCapability` and
`MenuCapability`:

```rust
pub struct RadioCapabilities {
    // ... existing fields ...
    /// `None`: this radio exposes no CW control over CAT.
    pub cw: Option<CwCapability>,
}

pub struct CwCapability {
    /// Sidetone/receive pitch, in Hz. `None` if CAT cannot read it.
    pub pitch: Option<HzRange>,
    pub pitch_writable: bool,
    /// Internal keyer speed, words per minute (PARIS).
    pub keyer_wpm: Option<WpmRange>,
    pub keyer_wpm_writable: bool,
    /// Break-in modes CAT can select. Empty: CAT cannot set break-in.
    pub break_in: &'static [BreakInKind],
    /// Whether the radio's keyer can send arbitrary text over CAT.
    pub send_text: Option<CwSendText>,
    /// The power the server forces before any keyed CW, and how to
    /// read it back. Required when `send_text` is `Some`.
    pub tx_gate: Option<TxGateCapability>,
}

pub struct HzRange  { pub min_hz: u32, pub max_hz: u32, pub step_hz: Option<u32> } // None: continuous
pub struct WpmRange { pub min: u16, pub max: u16 }

pub enum BreakInKind { Off, Semi, Full }

pub struct CwSendText {
    /// Largest message the radio accepts in one command
    /// (TS-570D 24, IC-7100 30).
    pub max_chunk_chars: u16,
    /// The characters the keyer will send, uppercase. A client maps
    /// prosigns onto these (e.g. AR → `+`, BT → `=`) and the server
    /// rejects anything outside them.
    pub charset: &'static str,
    /// How the server learns a chunk has finished.
    pub pacing: CwPacing,
    /// What abort can actually do on this radio.
    pub abort: CwAbort,
    /// Break-in the radio needs before it keys a sent message (IC-7100:
    /// `Semi`); set on arm, restored on disarm (§3.3). `None`: keys anyway.
    pub requires_break_in: Option<BreakInKind>,
}

pub enum CwPacing {
    /// The radio reports buffer busy/free (TS-570D `KY;`).
    BufferFlag,
    /// No busy flag: the server paces by `cat-morse` duration estimate
    /// at the radio's reported speed, plus a margin (IC-7100).
    Estimated,
}

pub enum CwAbort {
    /// A command that stops the keyer mid-character (IC-7100 `17 FF`).
    RadioStop,
    /// No stop command: abort stops feeding, and the radio finishes the
    /// chunk it holds. The server bounds that by chunk size (§4).
    DrainOnly,
}

pub struct TxGateCapability {
    /// The radio's lowest settable power, in its own units, and a
    /// human label ("5 W"). The server sets exactly this.
    pub low_power_raw: u16,
    pub low_power_label: &'static str,
    /// Inclusive raw range `PC`/`14 0A` reports, for read-back checks.
    pub power_raw: RawRange,
}
```

`HzRange::contains` honours the step; `CwSendText::accepts` folds ASCII
case and always allows a space; `CwCapability::validate()` rejects an
incoherent description (above all: `send_text` without `tx_gate`), and
each radio crate calls it from a test on its own `const`. Enums serialize
PascalCase like `ModeId`/`ModeKind`.

All of it is static per model (ADR 0010 §1, ADR 0015's rule): two
TS-570Ds cannot disagree about any of it. Ranges are in operator units
(Hz, wpm); the radio crate owns the translation to its own encoding
(TS-570D index 0-12, IC-7100 raw 0-255), exactly as it owns mode bytes.

Adding a field to `RadioCapabilities` breaks every radio crate's `const`
struct literal. That is a deliberate compile error: each radio repo
states `cw: None` or a real description on upgrade. It ships as a minor
version bump (0.8 → 0.9).

### 2. On the wire: additive and optional, no version bump

- `CapabilitiesWire` gains `#[serde(default)] pub cw: Option<CwCapabilityWire>`
  (owned mirror: `charset: String`, `break_in: Vec<BreakInKind>`).
- `RadioState` gains `#[serde(default)] pub cw: Option<CwState>`:

```rust
pub struct CwState {
    pub pitch_hz: Option<u32>,
    pub keyer_wpm: Option<u16>,
    pub break_in: Option<BreakInKind>,
    /// Present only while some console holds the transmit arm.
    pub armed: Option<ArmedState>,
    pub sender: SenderState,
}
pub struct ArmedState {
    pub callsign: String,
    /// Read back from the radio after the clamp, raw + label.
    pub power_raw: u16,
    /// Whether *this* connection is the owner. Filled per connection.
    pub mine: bool,
}
pub enum SenderState {
    Idle,
    Sending { job: u32, sent_chars: u32, total_chars: u32 },
    Draining { job: u32 },          // DrainOnly abort in progress
    Stopped { job: u32, reason: StopReason, message: Option<String> },
}
// StopReason: completed | aborted | key_down_cap | disconnected | error
// SenderState is internally tagged: {"state":"draining","job":7}
```

`key_down_cap_s` (optional, seconds) is how a client lowers the 60 s cap
for its arm (decision 5); a value above the server's cap is ignored.
Owned capability mirrors are `CwCapabilityWire`, `CwSendTextWire` and
`TxGateWire`; `HzRange`, `WpmRange`, `BreakInKind`, `CwPacing` and
`CwAbort` cross the wire as they are and are re-exported by `cat-native`.

```rust
```

Pitch, wpm and break-in are CAT reads on the **levels** clock (5 s), and
only when `capabilities.cw` is `Some`. `armed` and `sender` are server
bookkeeping, patched without a CAT exchange, so they are as fresh as the
last `ReadState`. A copy-practice trainer needs nothing but `ReadState`.

- `Command` gains, all serde-tagged and additive:

| Command | Who may send | Reply | Server checks (beyond capability) |
|---|---|---|---|
| `SetKeyerSpeed { wpm }` | any console | `Ack` | in `keyer_wpm`, writable |
| `ArmCwTransmit { callsign, dummy_load_acknowledged, key_down_cap_s? }` | any, if no other owner | `Ack` after clamp + read-back | §3 |
| `SendCwText { text }` | the arm owner only | `Ack` when **queued** (progress in `CwState.sender`) | §3, §4 |
| `AbortCw` | **any** console, armed or not | `Ack` when the stop has been issued | none — never refused |
| `DisarmCw` | the arm owner | `Ack` after restore + read-back | — |

No `ReadCw`: state rides `ReadState`, for the same reason `ReadState` is
one command (ADR 0010 §6 — values from one moment).

- `ErrorCode` gains `Forbidden` ("you may not, now": not armed, armed by
  someone else, no dummy-load acknowledgement, ID rule). It is only ever
  sent in reply to a CW command, which an old client never sends.

- **`PROTOCOL_VERSION` stays 2.** `Hello` refuses on strict inequality, so
  a bump would lock out every deployed console for a change none of them
  needs. Every addition is optional on read (`serde(default)`; nothing in
  the stack uses `deny_unknown_fields`). The pin test grows a case
  decoding a pre-0023 `Welcome` and `RadioState` with no `cw` key.

### 3. The safety split: the server enforces, the client attests and presents

The rule: **anything that ends, bounds or reverses a transmission is the
server's**, because the server outlives the client. The client owns what
only a human can supply.

**Server (shared code, every radio):**

1. *Arm preconditions* — `dummy_load_acknowledged == true`; callsign is
   3-10 chars of `[A-Z0-9/]` with at least one letter and one digit; no
   other console owns the arm; radio not currently transmitting; current
   mode is a `ModeKind::Cw` mode (the server refuses rather than changes
   mode — decision 7). Arming is logged with callsign, peer address
   and time: the acknowledgement is an attestation the server cannot
   verify, so it is recorded.
2. *Power clamp and read-back* — on arm: read power and break-in
   (snapshot), set power to `tx_gate.low_power_raw`, read it back; any
   mismatch → restore snapshot and refuse the arm. Re-read before the
   first chunk of every job; a value above the clamp aborts the job and
   disarms.
3. *Break-in* — if `send_text` requires it (IC-7100 transmits a `17`
   message only with break-in on), set `Semi` on arm, read back, restore
   on disarm.
4. *Restore* (as built: only changed settings are written back, never
   while a `DrainOnly` radio is still draining, and each write is read
   back. A failed write or a wrong read-back is reported
   (`CwState.restore: Option<RestoreStatus>`, additive), keeps the arm
   locked -- no new arm, rigctl `T 1` still refused -- and is retried every
   5 s until verified; a pending `DisarmCw` is answered `Err` on the first
   failure and the server never reports a restore it has not read back)
   — on `DisarmCw`, owner disconnect, arm idle timeout (default
   10 min with no send) and server shutdown (best effort): stop any send,
   restore power and break-in from the snapshot, read back, log
   mismatches. A crash that skips restore leaves power **low**, the safe
   direction.
5. *Send gating* — `SendCwText` refused (`Forbidden`) unless this
   connection owns the arm; text must be within `charset` and a total
   length bound (default 256 chars).
6. *Callsign ID* — the server appends `DE <CALL>` (word-separated) to
   the first job of an armed session and to any job sent once 10 minutes
   of armed time have elapsed since the last ID (FCC §97.119 cadence). A
   job is never refused for lacking the callsign; the appended ID is
   counted in the key-down estimate and reported in `total_chars`.
7. *Key-down cap* — before accepting a job, estimate its duration with
   `cat_morse::timing` at the radio's current wpm; refuse if over the cap
   (default 60 s per job; a client may lower it, never raise it). A
   generation-guarded watchdog (the `ts570d/server/src/ptt_dtr.rs`
   pattern) aborts a job that outlives estimate + margin, and drops the
   arm.
8. *Abort* — §5. Always accepted, from any console, armed or not.
9. *One owner* — rigctl never gains `send_morse`/`stop_morse`: rigctl has
   no arming, and a second way in would be a way around every rule above.
   While CW is live (arm requested → restore verified), rigctl `T 1` is
   refused (`PttGate`) and the raw TCP/UDP CAT listeners refuse every
   frame (`BrokerHandle::gated`, via `cat_rigctl::raw_cat_handle`).

**Client (the trainer, not this library):** the dummy-load acknowledgement
screen every session (never remembered), callsign entry, mapping prosigns
onto `charset`, showing `armed.power_raw`'s label and send progress, a
single key that sends `AbortCw`, and refusing to arm in copy mode at all.
Arming never survives a reconnect; the client must ask again.

**Owed ID (safety review, 2026-10-07).** When text has keyed and an
abort cuts the send before its `DE <CALL>`, the ID is *owed*. The server
does **not** send it on disarm, detach or timeout -- an abort means stop
-- but leads the next send with it, logs it at error level when the arm
ends, and publishes it as `CwState.id_owed` (additive) until an ID is
next sent. Both consoles show it. Nothing is owed if nothing keyed.

### 4. Chunking: the abort bound is a policy, not an accident

A `SendCwText` job is split at word boundaries into chunks of at most
`min(max_chunk_chars, drain_bound)`, where `drain_bound` is the number of
characters the radio keys in **3 s** at its current wpm (configurable).
For a `DrainOnly` radio this is the worst-case time a transmitter keeps
keying after abort. Without it the TS-570D at 10 wpm would drain a full
24-character buffer for ~29 s after the operator hit abort. The cost —
a slightly longer gap at chunk boundaries — is a bench question (§9).

**As built (T5, revised twice per architect, 2026-10-07):** abort
latency is the safety property, and abort beats the ID. On a `DrainOnly`
radio *every* chunk keys within the drain bound: operator text is split at
the bound whatever its word boundaries (word boundaries are preferred
when they fit), and the server's own `DE <CALL>` is chunked at the same
bound -- it may be split, but it never shares a chunk with operator text.
An ID that an abort cuts short stays owed and **leads** the next send. A
single character is never split (there is nothing smaller to send). The
bound does not apply to a `RadioStop` radio, which stops at once.

**Worst-case drain after an abort, `DrainOnly` radio:** the 3 s bound,
for operator text and ID alike, as long as the radio's longest character
keys within it at the slowest keyer speed. For the TS-570D (slowest
10 wpm) the longest character, `0`, is 2.28 s, so the worst case is
**3 s**, plus one in-flight CAT exchange and one CW tick (§5). (Unsplit,
the longest valid ID would have been 27.48 s.) Pinned by
`the_worst_case_drain_is_the_bound_in_the_adr`. The bench (§9) may show
`RX;` stops a TS-570D `KY` sentence, which would make it `RadioStop`.

**Ready is not idle (safety review, 2026-10-07).** `KY;` reports buffer
*space*: a TS-570D answers ready while still keying. The supervisor
therefore never treats `ready` as finished. It tracks the estimated end
of everything handed to the radio (chunks key one after another) and
waits for it: before each next chunk (as well as `ready`, on a
`BufferFlag` radio), before a job counts as complete, after an abort
before the drain counts as over, and before **any** restore -- power is
never raised while sent text may still be keying. A `RadioStop` keyer's
`Stop` clears the estimate. The radio therefore holds at most one chunk,
which is what makes the drain bound real.

**Keyer speed under a send (safety review, 2026-10-07).** `SetKeyerSpeed`
is refused from every console while CW is live (arm requested → restore
verified), including one queued before an arm and applied after it. The
radio's own knob (or raw `KS` before an arm) cannot be refused, so on a
`DrainOnly` radio the supervisor reads the speed again before every
chunk; a change re-chunks what is left, and moves the watchdog, at the
new speed -- or stops the send if what is left would now exceed the cap.

### 5. Abort does not wait behind a send

A CAT stop cannot use ADR 0022's release lines: it is bytes on the wire,
and the broker serialises bytes. What *can* be made immediate:

1. `NativeShared::apply(AbortCw)` runs on the listener thread: it sets an
   atomic abort generation, empties the CW job (no further chunk will
   ever be sent), and puts the radio's stop at the **front** of the
   queue, exempt from `Slot` supersession.
2. The pump checks the abort generation **first** in every cycle, before
   the command queue and before any state poll, and issues `cw_stop`.
3. While a job or an abort is live, the pump runs on a CW tick (50 ms)
   instead of `poll_interval`'s transmit back-off, and skips the full
   state poll (only `KY;`-style buffer reads go on the wire).
4. A radio whose CW keying is a **modem line** (a future FT-991A
   host-timed path) de-keys with `release_now_bypassing_queue`, which is
   what ADR 0022 exists for.

The resulting bound, stated honestly in `docs/`: abort latency = one
in-flight CAT exchange (≤ the 2 s read timeout, typically tens of ms) +
one CW tick + the radio's drain (`RadioStop`: ~one element; `DrainOnly`:
≤ `drain_bound`).

### 6. How a radio implements it: a second trait beside `NativeRadio`

The seam every server already uses is
`cat_rigctl::native_bridge::NativeRadio`. It gains one defaulted accessor,
so `ft991a` and `ic7100` compile unchanged until they opt in:

```rust
#[async_trait::async_trait(?Send)]
pub trait NativeRadio {
    // ... state, apply, levels ...
    fn cw(&mut self) -> Option<&mut dyn NativeCw> { None }
}

#[async_trait::async_trait(?Send)]
pub trait NativeCw {
    async fn settings(&mut self) -> Option<CwSettings>;       // pitch_hz, wpm, break_in
    async fn set_keyer_wpm(&mut self, wpm: u16) -> Result<(), String>;
    async fn power_raw(&mut self) -> Result<u16, String>;
    async fn set_power_raw(&mut self, raw: u16) -> Result<(), String>;
    async fn set_break_in(&mut self, kind: BreakInKind) -> Result<(), String>;
    /// `Ok(true)` when the keyer can take another chunk. Only called
    /// for `CwPacing::BufferFlag`.
    async fn ready(&mut self) -> Result<bool, String>;
    /// One chunk, already validated and within `max_chunk_chars`.
    async fn send_chunk(&mut self, text: &str) -> Result<(), String>;
    /// `RadioStop` radios stop; `DrainOnly` radios return `Ok(())`.
    async fn stop(&mut self) -> Result<(), String>;
}
```

Every method is one radio's own encoding (`KY` + space + 24 padded chars;
`17` + bytes; `17 FF`), written in that radio's crate. The sequencing,
gating, chunking, watchdog and restore are written **once**, as a pure
`CwSupervisor` state machine in a new `cat_rigctl::native_bridge::cw`
module (no I/O, injected clock — testable on every platform), driven by
the pump.

`cat_native::server::RadioHost` gains `serves_cw() -> bool`, defaulted
`false`. Until a host returns `true` -- which it may only do once arm
preconditions, power clamp and read-back, key-down cap and ID are all
enforced -- the handshake publishes `cw: None`, every CW command except
`AbortCw` is `Unsupported`, and `AbortCw` is acknowledged without reaching
the host (`NativeSession::reaches_host`). A partly built supervisor can
therefore never be handed a send.

`RadioHost` also gains connection identity (as built: `ConsoleId`, a
process-unique opaque id; `HostRefusal { code, message }`, `From<String>`
→ `OutOfRange`; and, beyond the sketch below, defaulted
`console_attached_as(id, peer)` for the arm log and `state_as(id)` so
`ArmedState::mine` can be filled per reader) without
breaking implementors: `serve` assigns each connection a `ConsoleId`, and
two defaulted methods — `apply_as(ConsoleId, &Command) -> Result<(),
HostRefusal>` (defaults to `apply`, mapping to `OutOfRange` as today) and
`console_detached_as(ConsoleId)` (defaults to `console_detached`) — let
`NativeShared` know who owns the arm and who just hung up.

**As built (T6, 2026-10-07).** `NativeRadio::cw() -> Option<&mut dyn
NativeCw>` (default `None`) is the opt-in: the pump creates the
supervisor on its first cycle only when that returns `Some` and
`capabilities.cw` validates, and only then does `NativeShared` report
`serves_cw()`. `NativeCw` is `status` / `settings` / `set_keyer_wpm` /
`power_raw` / `set_power_raw` / `break_in` / `set_break_in` / `ready` /
`send_chunk` / `stop`. rigctl `T 1` is refused by a `PttGate` wrapper on
both platforms (Windows rigctl is not wrapped in `Cached`). An arm whose
console stopped waiting is released when it completes.
**Shutdown (revised per architect, 2026-10-07).** `run_with_native`
itself settles CW on its way out, on both platforms: when its listeners
end it calls `shutdown_cw()` and waits (yielding to the pump on Linux,
blocking its own thread on Windows) up to `CW_SHUTDOWN_TIMEOUT` (10 s)
for `cw_settled()`, logging at error level what is left undone if it
does not settle. A radio repo needs nothing extra for that path;
`shutdown_cw()` / `cw_settled()` / `settle_cw_blocking()` / `settle_cw()`
remain for hosts with their own shutdown.

**Signals (architect decision, 2026-10-07).** The library installs no
process-wide signal or console-control handler. Instead
`ServerConfig::stop` is a `StopHandle` (`Clone + Send`, both platforms);
`StopHandle::stop()` ends `run_with_native`'s wait exactly as a listener
ending would, so the restore-and-wait path above runs and the call
returns `Ok(())`. **Every server application must wire its Ctrl-C /
SIGTERM handler to `stop()`** (dispatched as part of the ts570d, ft991a
and ic7100 server tasks); a server killed by the default disposition
runs no restore and leaves CW power at the clamp.

The same work found the pump could sleep a full idle interval (up to
10 s with no console watching) before acting on an arm, abort, disarm or
shutdown. A server that serves CW now sleeps in `CW_TICK` slices and is
woken by any CW request.

**Consoles (T7c, 2026-10-07; decision 12).** `cat_ui::cw::CwPanel` is
the one model both renderers draw. The CW panel is the reserved custom
name `cat_layout::CW_PANEL` (`"cat.cw"`, `PanelKind::cw()`), not a new
`PanelKind` variant: `PanelKind` has no unknown-variant fallback, so a
variant would stop every older console decoding a `Welcome` whose layout
placed it, while a custom name decodes everywhere and an older console
draws its placeholder. Independently of layout, **both consoles' status
strips put CW first**: a failed restore replaces everything else the
strip would say (white on red / the warning fill), and an arm or a send
rides in front of the console's own text. `RadioDisplay.cw` carries the
state to the terminal console; the application fills it from
`RadioState.cw`.

### 7. Against an older server, and an older client

| Peer | What happens | Client's duty |
|---|---|---|
| New trainer → pre-0023 server | `Welcome` has no `cw` → `None`. `RadioState` has no `cw`. | Treat `cw: None` as authoritative. Copy mode uses its own configured pitch/wpm with a visible "server does not report CW settings" note. Send mode is unavailable, with the reason named. **Never probe** with a CW command — an old server answers it `Malformed`, which the client also maps to "unsupported" defensively. |
| Old console → 0023 server | Ignores `cw` in `Welcome` and `RadioState`; never sends a CW command, so never sees `Forbidden`. | none |
| New trainer → 0023 server, radio with `cw` but `send_text: None` (FT-991A v1) | Copy mode fully works; send mode offered nowhere. | Say why. |

### 8. Per-radio mapping

| | TS-570D | FT-991A | IC-7100 |
|---|---|---|---|
| Pitch | `PT` 00-12 → 400-1000 Hz, step 50 (R/W) | `KP` 00-75 → 300-1050 Hz, step 10 (R/W) | `14 09` 0-255 → 300-900 Hz, continuous (R/W) |
| Keyer wpm | `KS` 010-060 (R/W) | `KS` 004-060 (R/W) | `14 0C` 0-255 → 6-48 (R/W) |
| Send text | `KY` + space + ≤24 chars, space-padded to 28 bytes | **none** (`KY` only plays stored memories; `KM` writes them, ≤50 chars, persistent) | `17` + ≤30 chars |
| Pacing | `BufferFlag` (`KY;` → `KY0`/`KY1`) | — | `Estimated` (no busy read documented) |
| Abort | `DrainOnly` — no documented stop; `RX;` effect **unverified** | — | `RadioStop` (`17 FF`) |
| Charset | A-Z 0-9 `" ' ( ) + , - . / : = ?` | — | per manual "character code" table — **to transcribe** |
| Break-in | `VX` on/off; `SD` 0000-1000 ms; Full vs `SD0000` **unverified** | `BI` 0/1; type menu 056 (`EX056`); `SD` 0030-3000 | `16 47` 00/01/02 off/semi/full; `14 0F` delay |
| TX power | `PC` 005-100 W, 5 W steps; low = 5 W | `PC` 005-100 (band max); low = 5 W | `14 0A` 0-255 (min-max); low = raw 0, **watts unknown** |
| In command table today | PT KS KY VX SD PC: yes. `BK` present but **not in the TS-570 manual** — do not use | KP KS KR KY KM BI SD PC: yes | pitch/speed/power/break-in: yes. **`17` absent** |

Sources: `ts570d/radio/src/ts570d_radio.rs:130-216` and
`ts570d/docs/TS-570-English.pdf` (CAT appendix, parameters 42-47, 49);
`ft991a/radio/src/ft991a_radio.rs:1842-1850`, `ft991a/radio/src/ft991a.rs:110-119`
and `ft991a/docs/manuals/FT-991A_CAT_OM_ENG_1711-D.pdf` (`KM KP KR KS KY
BI SD PC`, menus 056/060); `ic7100/radio/src/command.rs:93-99,135,179`
and `ic7100/docs/manuals/ci-v-section.txt:535-611,1147-1178`.

**Existing defect, outside this repo:** `ts570d`'s `Ts570d::send_cw`
(`radio/src/ts570d.rs:1001`) writes `KY<msg>;` with no byte-3 space and no
padding, and its test pins `"KYCQ;"`. The manual's form is
`KY␠<24 chars, space-padded>;`. It must be fixed before the TS-570D
implements `NativeCw::send_chunk`.

**FT-991A send** is out of scope here. Its only free-text path is
host-timed keying of DTR/RTS on the "Standard" COM port (menu 060 PC
KEYING, `EndpointRole::Keying`), timed by `cat-morse` and de-keyed by
ADR 0022 release lines. That is a different mechanism (host timing,
jitter over USB) and gets its own ADR if wanted. Using `KM` would
overwrite the operator's stored keyer messages and is rejected.

### 9. Unknowns that need a bench, on a dummy load

- TS-570D: does `RX;` stop a running `KY` sentence? Does `SD0000` mean
  Full break-in? How long is the gap between back-to-back chunks?
- IC-7100: is a second `17` while sending queued, appended, or refused?
  Minimum watts at raw 0 per band. The `17` character table.
- FT-991A: whether CAT keyer playback needs `KR1` (moot until send exists).

## Radio repo checklist (0.9.0)

What each radio repository (`ts570d`, `ft991a`, `ic7100`) does to adopt
this. Until step 2 is done the server offers no CW at all.

1. **Declare** `RadioCapabilities.cw` — `None`, or a real description —
   and call `CwCapability::validate()` from a test on the `const`. A
   description that does not validate is never served.
2. **Implement `NativeCw`** for the radio's native bridge and return it
   from `NativeRadio::cw()`. This is the opt-in. Only after the radio's
   own commands are right (TS-570D: fix `send_cw` to `KY␠<24 padded>;`
   first).
3. **Add `ServerConfig.stop`** (the 0.9.0 struct literal will not
   compile without it) and **wire the application's Ctrl-C / SIGTERM
   handler to `StopHandle::stop()`**, so a stopped server restores the
   radio before it exits. The library installs no signal handler.
4. **Fill `RadioDisplay.cw`** from `RadioState.cw` in the terminal
   console when it talks to a server; the GUI reads it itself.
5. *Optional:* place `PanelKind::cw()` (`"cat.cw"`) in the
   server-authored layout. The status strip shows the arm, the send and
   any failed-restore alert without it.
6. **Bench on a dummy load** (§9) before declaring `send_text`.

## Consequences

**Good.**
- One trainer binary serves all three radios; copy mode needs one
  `ReadState`.
- Everything that ends a transmission runs where the transmitter is.
  A crashed or disconnected trainer de-keys within one tick plus drain.
- Abort latency on a `DrainOnly` radio is a configured number, not a
  property of whatever the operator typed.
- Nothing deployed breaks: no version bump, all wire additions optional,
  `NativeRadio`/`RadioHost` additions defaulted.

**Costs and risks.**
- `RadioCapabilities` gains a field: every radio repo edits its `const`
  on upgrade (0.9.0).
- `cat-rigctl` takes a dependency on `cat-morse` (duration estimates) and
  grows a CW supervisor inside `native_bridge`, a module that is already
  the native server in all but name. Whether it should move out of
  `cat-rigctl` is a separate question, outside this ADR (decision 1).
- The pump gains a second cadence. Getting that wrong regresses the
  rigctl latency fixes recorded in `pump`'s comments; the WSJT-X
  latency behaviour is a regression gate.
- The dummy-load acknowledgement is an attestation, not a measurement.
  The server records it; it cannot check it.
- FT-991A gets read-only CW in v1.
