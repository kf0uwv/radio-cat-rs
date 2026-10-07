# cw_capability — task plan

## Goal

Implement ADR 0023 (Accepted 2026-10-07): an optional `CwCapability` in the capability
model and native protocol, an armed CW transmit session enforced by the
shared server, and an abort that does not wait behind a send — so a
standalone trainer (new repo) works against any radio-cat-rs radio's
`server`.

## Current Phase

**T1-T7 done (+2 architect fixes); T7b next, awaiting go-ahead** (worktree `../radio-cat-rs-cw`, branch
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

### T1 — `cat-framework`: CW capability types (status: done 2026-10-07)
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
- Done: as above, plus `CwSendText.requires_break_in: Option<BreakInKind>`
  (§3.3 needed it; ADR §1 updated) and `CwCapabilityError` (thiserror).
  The fixtures file holds real-radio copies (TS570D, FT991A), not
  synthetic radios: TS570D got send + DrainOnly/BufferFlag, FT991A
  read-only (decision 2); the synthetic radio lives in
  `capabilities.rs`'s tests. `cw: None` added to every literal in
  cat-native/cat-rigctl/cat-ui. Version bump to 0.9.0 **not** done in T1
  (left for release/T8 so Cargo.lock does not churn mid-series).

### T2 — `cat-native`: wire types (status: done 2026-10-07)
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
- Done: as above, plus `ArmCwTransmit.key_down_cap_s: Option<u16>`
  (decision 5's "client may lower"), `Stopped.message`, `CwSendTextWire`
  / `TxGateWire`, re-exports of the framework CW leaf types. **Interim:**
  `NativeSession::validate` refuses all five CW commands `Unsupported`
  (pinned by `until_the_supervisor_exists_no_cw_command_reaches_a_host`);
  T3 replaces that, making `AbortCw` always `Ok`. `Slot::of` puts CW
  commands at `None` pending T6; egui `note_request` no-ops them.
  Production `RadioState` builders (native_bridge, cat-ui demo) publish
  `cw: None` until T6.

### T3 — `cat-native`: `NativeSession` validation (status: done 2026-10-07)
- Per-connection rules that need no radio: no `cw` capability →
  `Unsupported`; wpm out of range / not writable → `OutOfRange` /
  `Unsupported`; `ArmCwTransmit` without acknowledgement or with a bad
  callsign → `Forbidden`; `SendCwText` with chars outside `charset` or
  over the total-length bound → `OutOfRange`; `AbortCw` **always** `Ok`,
  even unarmed and even with no `cw` capability (harmless no-op).
- Owner/arming checks are *not* here (cross-connection) — T5/T6.
- Tests: table-driven over a stub radio with and without `cw`.
- **Gate (architect, 2026-10-07): no CW command may reach a host until
  the full supervisor exists.** Mechanism: `RadioHost::serves_cw()`
  (default `false`) → `NativeSession::set_serves_cw`. While `false`:
  `Welcome` publishes `cw: None` (honest: this server offers none);
  every CW command but `AbortCw` is `Unsupported`; `AbortCw` is `Ack`ed
  but **not applied** (`NativeSession::reaches_host` → `false`, checked
  in `serve_one`). Nothing in this repo returns `true` until T6, and T6
  only once arm/clamp/read-back/cap/ID are all enforced.
- Rules when serving: `SetKeyerSpeed` needs `keyer_wpm` + writable
  (`Unsupported`) and in range (`OutOfRange`); Arm/Send/Disarm need
  `send_text` (`Unsupported`); Arm needs the acknowledgement and a
  callsign of 3-10 `[A-Za-z0-9/]` with a letter and a digit, sendable in
  `charset` (`Forbidden`), `key_down_cap_s != 0` (`OutOfRange`; above
  60 is clamped by the supervisor, not refused); Send needs non-blank
  text ≤ `MAX_CW_TEXT_CHARS` (256) within `charset` (`OutOfRange`).

### T4 — `cat-native`: connection identity for hosts (status: done 2026-10-07)
- `ConsoleId` assigned in `serve`; `RadioHost::apply_as` (default →
  `apply`, refusal mapped to `OutOfRange` as today) and
  `console_detached_as` (default → `console_detached`); `HostRefusal {
  code, message }`.
- Tests: a recording host sees distinct ids per connection and a detach
  for each, including the error-return path (loopback via the existing
  `testing::serve_stub` pattern); default impls preserve current behaviour
  for an implementor that overrides neither.
- Windows: `serve` is `std::net` + threads on both platforms — no gating.
- Detail (2026-10-07): `ConsoleId` is an opaque `Copy` id from a
  process-wide counter (unique across listeners). Also defaulted:
  `console_attached_as(id, peer: SocketAddr)` (arm log needs the peer,
  ADR §3.1) and `state_as(id)` → `state()` (so T6 can fill
  `ArmedState.mine` per connection). `HostRefusal { code: ErrorCode,
  message }` with `From<String>` → `OutOfRange`. Hosts in this repo keep
  the defaults; nothing opts into CW.

### T5 — `cat-rigctl`: pure `CwSupervisor` (status: done 2026-10-07)
- New `native_bridge::cw` module. I/O-free state machine; inputs are
  events (`Arm{owner,…}`, `Send{owner,text}`, `Abort`, `Disarm`,
  `Detached(id)`, `Tick(now)`, radio results), outputs are actions
  (`ReadPower`, `SetPower(raw)`, `SetBreakIn`, `Ready?`, `SendChunk`,
  `Stop`, `Restore(snapshot)`, `Publish(CwState)`).
- T6 must not make `NativeShared::serves_cw()` return `true` until every
  gate rule is live and tested (architect, 2026-10-07).
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

- Design as built (2026-10-07, architect requirements added):
  sans-IO `CwSupervisor` — `next_action(now) -> Option<CwAction>` /
  `complete(CwOutcome, now)`, one outstanding action at a time;
  requests `arm/send/abort/disarm/detached/shutdown`; replies to arm and
  disarm as `CwNotice::Reply{ticket, ..}`; `take_notices()` for logging.
  Actions: ReadStatus (cw mode, transmitting, wpm), Read/SetPower,
  Read/SetBreakIn, Ready, SendChunk, Stop.
  - Restore (disarm, owner detach, watchdog, idle TTL, power-above-clamp,
    shutdown) writes back only what was changed, then reads it back. A
    failed write or mismatch is `RestoreFailed` + `CwState.restore`
    alert, retried every 5 s **forever**; the arm stays locked (no new
    arm, rigctl `T 1` still refused) and `Restored` is only ever
    reported after a verified read-back. A pending disarm is answered
    `Err` on the first failure, never `Ok` until verified.
  - Restoring waits for the radio to finish draining before raising
    power back (never raise power under a keyed chunk).
  - Drain bound (3 s) chunking applies to `DrainOnly` radios only; a
    `RadioStop` radio stops at once. **Revised twice per architect (2026-10-07):** every chunk,
    operator text and ID alike, splits at the bound; the ID never
    shares a chunk with operator text; an ID cut by an abort leads the
    next send; the "ID must fit one chunk" arm refusal is gone. Worst-case
    TS-570D drain: 3 s (+ in-flight exchange). ADR §4 states it and a
    test pins it.
  - The stop is always sent when a job ends (even before any chunk);
    a drain wait has its own deadline so a wedged keyer cannot hold a
    restore off.
  - ID: `" DE <CALL>"` appended to a job when none has been sent this arm
    or 10 min have passed since the last; `last_id` advances only when
    the chunk carrying it is sent.
  - `blocks_external_ptt()` → rigctl `T 1` refusal (wired in T6).
  - Wire addition: `CwState.restore: Option<RestoreStatus>`
    (`serde(default)`).

### T6 — `cat-rigctl`: pump + `NativeShared` integration (status: done 2026-10-07)
- `NativeCw` trait; `NativeRadio::cw()` defaulted to `None` (ft991a,
  ic7100 compile unchanged).
- T6 inputs from T5: `ConsoleId::from_raw` exists for non-`serve` hosts
  and tests; `CwSupervisor::{blocks_external_ptt, is_live, is_settled,
  armed_state, sender, restore_status, take_notices}`.
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

- Design as built (2026-10-07): `NativeCw` methods map 1:1 onto
  `CwAction` (`status`, `power_raw`/`set_power_raw`, `break_in`/
  `set_break_in`, `ready`, `send_chunk`, `stop`, plus `settings` for the
  levels clock and `set_keyer_wpm`). `NativeShared` holds the
  `CwSupervisor`; the pump creates it on its first cycle **only if** the
  radio returns `Some` from `cw()` and `capabilities.cw` validates —
  that is the opt-in (`serves_cw()` true from then). `apply_as`:
  `AbortCw` → supervisor on the listener thread, never queued; Arm /
  Disarm wait (15 s) for the supervisor's reply; an arm whose console
  gave up waiting is disarmed when its `Ok` arrives; `SendCwText` is
  immediate; `SetKeyerSpeed` is queued (refused while a send is live).
  Detach → `supervisor.detached`. `state_as` fills `CwState` per
  console. Pump order: CW actions → queued commands → (state poll only
  when CW is not live) → wait 50 ms while live. rigctl `T 1`: a
  `PttGate<R>` wrapper on **both** platforms (Windows rigctl is not
  wrapped in `Cached`, so `Cached` alone would miss it).
  `NativeShared::shutdown_cw()` / `cw_settled()` for apps' shutdown.

### T7 — `cat-native` client conveniences (status: done 2026-10-07)
- `Connection`/`Client`: `cw_capability()`, `arm_cw`, `send_cw`,
  `abort_cw`, `disarm_cw`, `set_keyer_speed`; `Malformed` in reply to a CW
  command from an old server surfaces as `ClientError::Unsupported`-like.
- Tests against `testing` stub with and without `cw`.

- Design (2026-10-07): on `Connection` (the blocking client a trainer
  uses): `cw_capability()`, `cw_state()`, `set_keyer_speed`, `arm_cw(call,
  dummy_load_acknowledged, key_down_cap_s)` (the acknowledgement is an
  explicit argument, never defaulted), `send_cw`, `abort_cw`,
  `disarm_cw`. New `ClientError::Refused { code, message }` for a
  server's refusal. **Never probe** (ADR §7): with `cw: None` every call
  but `abort_cw` returns `Refused(Unsupported)` without sending;
  `abort_cw` is a local no-op. A `Malformed` reply (a pre-0023 server)
  maps to `Refused(Unsupported)`. `Client` gets `cw_capability()`.

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
- ~~Every radio's server must call `shutdown_cw()`~~ — superseded:
  `run_with_native` settles CW on exit itself (fix 2). **Open for the
  architect:** signal-driven exit (Ctrl-C/SIGTERM) runs no library code;
  owning it needs a process-wide handler + new dep (ADR 0023 §6 note).
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
