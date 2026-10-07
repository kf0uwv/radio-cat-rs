# cw_capability — task plan

## Goal

Implement ADR 0023 (Accepted 2026-10-07): an optional `CwCapability` in the capability
model and native protocol, an armed CW transmit session enforced by the
shared server, and an abort that does not wait behind a send — so a
standalone trainer (new repo) works against any radio-cat-rs radio's
`server`.

## Current Phase

**T1 approved and in progress** (worktree `../radio-cat-rs-cw`, branch
`feat/cw-capability` off `feat/cat-morse`). ADR 0023 Accepted 2026-10-07;
open questions answered (see "Decisions" at the end). One task at a time;
report after each. Do not push.

## Hard constraints

- No tokio. monoio Linux-only and target-gated; Windows
  (`x86_64-pc-windows-msvc`) paths use the existing thread/`block_on`
  backends. New logic is pure + clock-injected so it tests identically on
  both.
- Unit tests use fakes only: a `FakeCw` (scripted `NativeCw`), the
  existing `cat_native::testing` stub host, `ScriptedCatSession` where a
  session is needed. Never a concrete transport from another crate.
- `cat-framework` stays radio-agnostic: no `KY`, no `17`, no radio names.
- `PROTOCOL_VERSION` stays 2; every wire addition is
  `#[serde(default)]`/optional.
- thiserror + `Result`; imports std → external → local; Apache-2.0 header.
- TDD: failing test committed with (or before) the code that passes it;
  `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo fmt`, and `cargo xwin check --target
  x86_64-pc-windows-msvc --workspace` before each task is reported done.

## Tasks

### T1 — `cat-framework`: CW capability types (status: pending)
- Add `CwCapability`, `HzRange`, `WpmRange`, `BreakInKind`, `CwSendText`,
  `CwPacing`, `CwAbort`, `TxGateCapability`; `RadioCapabilities.cw:
  Option<CwCapability>`.
- Tests first: the whole tree is still a `const`; `HzRange::contains` /
  stepping (stepped and continuous); `WpmRange::contains`;
  `CwSendText::accepts(text)` charset check (case-folding, rejects
  outside chars, space allowed); `send_text.is_some() ⇒ tx_gate.is_some()`
  checked by a `validate()` helper fixtures call.
- Update synthetic fixtures (`capabilities_fixtures.rs`) with a CW-capable
  and a CW-less radio; no real radio fixture tuned to pass.
- Note: breaking struct-literal change → workspace version 0.9.0.

### T2 — `cat-native`: wire types (status: pending)
- `CwCapabilityWire` + `From`; `CapabilitiesWire.cw` (`serde(default)`);
  `CwState`/`ArmedState`/`SenderState`/`StopReason`;
  `RadioState.cw` (`serde(default)`); `Command::{SetKeyerSpeed,
  ArmCwTransmit, SendCwText, AbortCw, DisarmCw}`; `ErrorCode::Forbidden`.
- Tests first: a **literal pre-0023 `Welcome` JSON** and `RadioState`
  JSON (no `cw` key) decode with `cw == None`; new shapes round-trip;
  command tag spellings pinned (`"cmd":"send_cw_text"` etc.);
  `wire_shape_is_pinned` extended — version still 2.
- Update every exhaustive `match` on `Command` (`Slot::of`,
  `NativeSession::validate`, testing stub).

### T3 — `cat-native`: `NativeSession` validation (status: pending)
- Per-connection rules that need no radio: no `cw` capability →
  `Unsupported`; wpm out of range / not writable → `OutOfRange` /
  `Unsupported`; `ArmCwTransmit` without acknowledgement or with a bad
  callsign → `Forbidden`; `SendCwText` with chars outside `charset` or
  over the total-length bound → `OutOfRange`; `AbortCw` **always** `Ok`,
  even unarmed and even with no `cw` capability (harmless no-op).
- Owner/arming checks are *not* here (cross-connection) — T5/T6.
- Tests: table-driven over a stub radio with and without `cw`.

### T4 — `cat-native`: connection identity for hosts (status: pending)
- `ConsoleId` assigned in `serve`; `RadioHost::apply_as` (default →
  `apply`, refusal mapped to `OutOfRange` as today) and
  `console_detached_as` (default → `console_detached`); `HostRefusal {
  code, message }`.
- Tests: a recording host sees distinct ids per connection and a detach
  for each, including the error-return path (loopback via the existing
  `testing::serve_stub` pattern); default impls preserve current behaviour
  for an implementor that overrides neither.
- Windows: `serve` is `std::net` + threads on both platforms — no gating.

### T5 — `cat-rigctl`: pure `CwSupervisor` (status: pending)
- New `native_bridge::cw` module. I/O-free state machine; inputs are
  events (`Arm{owner,…}`, `Send{owner,text}`, `Abort`, `Disarm`,
  `Detached(id)`, `Tick(now)`, radio results), outputs are actions
  (`ReadPower`, `SetPower(raw)`, `SetBreakIn`, `Ready?`, `SendChunk`,
  `Stop`, `Restore(snapshot)`, `Publish(CwState)`).
- Encodes ADR §3-§5: single owner; mode must already be CW (refuse, never
  switch); snapshot → clamp → read-back → refuse on mismatch; server
  **appends `DE <CALL>`** to the first job and to any job once 10 min of
  armed time have passed since the last ID (never refuses for a missing
  call; the ID counts toward the cap and `total_chars`); 60 s cap,
  client-lowerable only; key-down estimate via
  `cat_morse::timing` (**new dependency edge cat-rigctl → cat-morse**);
  word-boundary chunking at `min(max_chunk, drain_bound)`; watchdog with
  generation guard; arm idle TTL; disconnect → abort + restore; abort
  generation pre-empts everything.
- Tests (all with an injected clock, no threads): every precondition
  refusal (incl. not-in-CW-mode); `DE <CALL>` appended on first job, not
  on the second within 10 min, again after 10 min; ID pushes a job over the
  cap → refused; read-back mismatch restores and refuses; chunking never splits
  a word unless the word exceeds the chunk; a 10 wpm TS-570D job never
  emits a chunk longer than 3 s; `BufferFlag` waits on `ready`;
  `Estimated` waits estimate + margin; abort mid-job emits `Stop` first and
  no later `SendChunk`; stale watchdog from job N cannot stop job N+1;
  owner detach restores; non-owner detach does not; restore runs once.

### T6 — `cat-rigctl`: pump + `NativeShared` integration (status: pending)
- `NativeCw` trait; `NativeRadio::cw()` defaulted to `None` (ft991a,
  ic7100 compile unchanged).
- Refactor `pump` into a testable `pump_once(&shared, &mut radio, now)`
  plus the loop (same behaviour; existing tests stay green first).
- Per cycle order: abort → CW supervisor actions → queued commands →
  (skip full poll while a job/abort is live) → state/levels poll (CW
  settings on the levels clock) → wait = CW tick (50 ms) while live, else
  `poll_interval`.
- rigctl `T 1` refused while a console holds the CW arm (decision 8),
  via a flag on `NativeShared` the rigctl side reads.
- `NativeShared::apply_as`: `AbortCw` sets an atomic abort generation and
  clears the job on the listener thread before enqueuing; never
  superseded; `SetKeyerSpeed` gets its own `Slot`.
- Tests with `FakeCw` + fake `NativeRadio`: abort issued before a queued
  `SetFrequency`; no chunk after abort; state poll suppressed while
  sending; levels-clock CW read only when capability present; existing
  latency tests unchanged.
- Windows: pump runs under `cat_server::block_on` on a thread and sleeps
  with `std::thread::sleep` — the 50 ms tick must use the same
  platform split already in `pump` (and must not reintroduce the double
  sleep the comment there records). `cargo xwin check` + CI `cargo test`.

### T7 — `cat-native` client conveniences (status: pending)
- `Connection`/`Client`: `cw_capability()`, `arm_cw`, `send_cw`,
  `abort_cw`, `disarm_cw`, `set_keyer_speed`; `Malformed` in reply to a CW
  command from an old server surfaces as `ClientError::Unsupported`-like.
- Tests against `testing` stub with and without `cw`.

### T7b — `cat-native`: `Connection::connect_timeout` (status: pending)
- Connect **and** handshake (`Hello`/`Welcome`) bounded by one timeout;
  needed by the standalone cw-trainer client. Both platforms (`std::net`
  `connect_timeout` + read timeout during handshake, restored after).
- Tests: a listener that accepts but never answers `Hello` → timeout
  error within bound; a stub host connects normally.

### T7c — consoles: CW state display (status: pending, decision 12)
- `cat-ui` model for `RadioState.cw` (pitch, wpm, break-in, armed/sending)
  rendered by **both** `cat-ui-ratatui` and `cat-ui-egui` (ADR 0013
  parity); hidden when `cw` is `None`.

### T8 — docs + bookkeeping (status: pending)
- ADR 0023 Accepted + `docs/adr/README.md` row: done with the ADR commit.
  Remaining: crate docs stating the abort-latency bound honestly.

### Out of this repo (for architect dispatch, after T6)
- **ts570d**: fix `send_cw` to the manual's `KY␠<24 padded>;` form (and
  its test pinning `KYCQ;`) and the emulator handler; implement
  `NativeCw`; `cw: Some(...)` with `BufferFlag` + `DrainOnly`; bench on a
  dummy load: `RX;` vs running KY, `SD0000`, chunk gap.
- **ic7100**: add `17` send-CW command id (+ `FF` stop) to the CI-V
  table; break-in as a kind not a bool; `NativeCw`; transcribe the `17`
  character table; bench: second `17` while sending, watts at raw 0.
- **ft991a**: `cw: Some` with `send_text: None` (pitch/wpm/break-in only).
- **trainer repo**: the client half of ADR §3 and §7.

## Decisions (architect + user, 2026-10-07)

1. Supervisor lives in `cat_rigctl::native_bridge::cw`.
2. FT-991A: CW read-only in v1.
3. TS-570D abort: `DrainOnly` with the ~3 s chunk bound until the `RX;`
   bench test.
4. ID: the server itself sends `DE <call>` on the first send and at least
   every 10 minutes (server-appended, not refuse-unless-present).
5. Key-down cap 60 s per send; a client may lower, never raise.
6. Low power: fixed radio-declared minimum.
7. Refuse arm/send unless already in CW mode; never switch modes.
8. While a console holds the CW arm, rigctl `T 1` is refused.
9. 0.9.0 for the capability field; `PROTOCOL_VERSION` stays 2.
10. cat-rigctl → cat-morse dependency accepted.
11. Poll CW settings on the existing levels clock when `cw` is `Some`.
12. Both consoles show CW state (TUI + GUI parity, ADR 0013) → T7c.
13. Added: `cat_native` `Connection::connect_timeout` → T7b.

## Errors encountered

| Error | Attempt | Resolution |
|---|---|---|
| Harness refused `Write` of `findings.md` as a "report file" | 1 | Findings returned to caller in the hand-off and summarised in ADR §8; progress noted below. |
