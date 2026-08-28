# Task Plan — architect

Extraction (task A) and TCP/UDP/server-mode implementation (task B) are now
authorized by explicit user go-ahead, dated 2026-07-16. `ts570d`'s
`refactor/generic-cat-framework` work (the `CatSession` migration) has
landed. This supersedes the previous "no extraction yet" placeholder.

Governing documents, in priority order: `docs/adr/0001-scope-and-crate-boundaries.md`
(as amended), `docs/adr/0002-async-runtime-binding-for-transport-crates.md`,
and `planning/architect/findings.md` (the reasoning and code-reading behind
every decision below — read it before dispatching any task; it is not
optional background).

## Ground rules for every task below

- One task per subagent dispatch. Architect + user review checkpoint after
  each, before the next is dispatched — no chaining.
- Each subagent creates/updates its own `./planning/{agent}/` files
  (`task_plan.md` before any code, `progress.md` after) per its own agent
  definition and this repo's `CLAUDE.md`.
- Each subagent's first action on its first task is creating the root
  `Cargo.toml` (if it doesn't exist yet) or adding its new crate(s) to
  `[workspace] members` (if it does), using the versions recorded in
  `findings.md` §2.
- `cargo test -p <crate>`, `cargo clippy`, `cargo fmt` must pass before a task
  is reported done.
- Every new crate's first commit message cites the exact `ts570d` source
  commit (`1585e1e`) and file paths it was extracted from (see `findings.md`
  §3 — this is the chosen substitute for history-preserving git-subtree
  import).
- `ts570d` and `ft991a` are not touched by any task below. Migrating `ts570d`
  onto these crates is an explicit follow-on, planned separately later (see
  `findings.md` §4) — not silently dropped, just not in this queue.
- Per [ADR 0002](../../docs/adr/0002-async-runtime-binding-for-transport-crates.md)'s
  "Consequences" amendment: any crate below that takes a `monoio` dependency
  (`cat-transport-core`, `cat-transport-serial`, `cat-transport-tcp`,
  `cat-transport-udp`, `cat-server`) MUST declare it under
  `[target.'cfg(target_os = "linux")'.dependencies]` in that crate's
  `Cargo.toml`, never a plain unconditional `[dependencies]` entry. This
  applies to the production dependency; `monoio` dev-dependencies (e.g.
  `cat-framework`'s/`cat-client`'s `#[monoio::test]` usage) should be gated
  the same way (`[target.'cfg(target_os = "linux")'.dev-dependencies]`).

## Dispatch queue

### Task 1 — `cat_framework` agent: create `cat-framework`

Create the root workspace `Cargo.toml` and the `cat-framework` crate. Move
from `ts570d`'s `framework/src/cat.rs` (commit `1585e1e`): `CommandId`,
`CommandOperation`, `CommandForm`, `CommandDefinition<C>`, `CommandTable<C>`,
`CommandRequest`, `ParameterValues`, `ParameterAccessError`, `ParseError`,
`ProtocolErrorKind`, `ResponseDisposition`, `CommandOutcome`,
`ResponseBuildError`, `ResponseBuilder`, `CatCommandCatalog`, `CatRadio`,
`CatFrameworkError<E>`, `CatFramework<R>`. Migrate the existing unit tests
as-is (they already use an in-crate fake `TestCommand`/`CommandTable` — no
radio import to strip).

**Explicitly excluded** (see `findings.md` §8–9): `framework/src/state_machine.rs`
(`ApplicationStateMachine`, `State`) and `framework/src/errors.rs`'s
`FrameworkError`/`FrameworkResult` — neither is CAT-engine machinery, neither
is named in ADR 0001's `cat-framework` scope, and `state_machine.rs` appears
unused anywhere in `ts570d` outside its own re-export. Leave both behind in
`ts570d`, unmoved.

Dependencies: none in this workspace (verify with `cargo tree -p
cat-framework`). External: `thiserror` only — no `async-trait`, no `monoio`
(`cat.rs` has no `async` code today).

Done when: `cargo test -p cat-framework` is green; `cargo tree -p
cat-framework` shows no other local crate.

### Task 2 — `cat_transport` agent: create `cat-transport-core` + `cat-transport-serial`

Two crates, one task (transport-core must exist before serial can build on
it; sequence internally within this dispatch slot).

**`cat-transport-core`**: move from `framework/src/transport.rs` (the
`Transport` trait), `framework/src/session.rs` (the `CatSession` trait only —
not `SerialCatSession`, which belongs to `cat-transport-serial`), and
`framework/src/test_support.rs` (`Exchange`, `ScriptedCatSession`, the
`conformance` module) — plus `TransportError` moved out of
`framework/src/errors.rs` (leave `FrameworkError`/`FrameworkResult` behind,
per Task 1's exclusion). Per ADR 0001 Amendment 2 (`findings.md` §7),
`cat-transport-core` takes a one-way dependency on `cat-framework` for
`ResponseDisposition`/`ProtocolErrorKind` reuse — this is corrected guidance,
not a deviation to flag. Per ADR 0002, also move the
`pub use monoio::{RuntimeBuilder, io::{AsyncReadRent, AsyncWriteRent}}`
convenience re-export here from `framework/src/lib.rs` — `cat-transport-core`
is now the crate that owns the runtime binding.

**`cat-transport-serial`**: move `SerialCatSession<T: Transport>` (the
generic wrapper, from `framework/src/session.rs`) **and** the concrete
`Transport for SerialPort` io_uring implementation from `ts570d`'s separate
`serial` crate (`serial/src/io_uring.rs`, `serial/src/lib.rs` — `SerialConfig`,
`SerialPort`, termios/`libc`/`nix` plumbing). See `findings.md` §6 for why
both sources are required — `session.rs` alone has no hardware behind it.

Dependencies: `cat-transport-core` → `cat-framework`, `async-trait`,
`thiserror`, `monoio` (dev-dep for tests is not enough here — the trait
definitions themselves are `#[async_trait(?Send)]`, per ADR 0002, so this is
a real dependency). `cat-transport-serial` → `cat-transport-core`, `monoio`,
`async-trait`, `thiserror`, `libc`, `nix` (with the `term` feature, matching
`ts570d`'s `serial/Cargo.toml`).

Done when: `cargo test -p cat-transport-core -p cat-transport-serial` is
green; the conformance test module runs against `ScriptedCatSession`; `cargo
tree -p cat-transport-core` shows only `cat-framework`.

**Depends on Task 1** (needs `cat-framework::{ResponseDisposition,
ProtocolErrorKind}` to exist).

### Task 3 — `cat_framework` agent: create `cat-client`

Not a pure move — see `findings.md` §5. Genericize `ts570d`'s
`radio/src/client.rs::RadioClient<S: CatSession>` by parameterizing it over a
radio-supplied `C: CommandId` / `&'static CommandTable<C>` instead of the
hardcoded `Ts570dCommandId`/`TS570D_COMMAND_TABLE`, and introduce a new
generic client error type (e.g. `ClientError<E>` with
`UnknownCommand`/`CommandNotReadable`/`CommandNotWritable`/`Transport(E)`
variants) in place of `ts570d`'s radio-specific `RadioError`. Preserve the
existing method shape (`query`, `query_with_param`, `set`) and behavior
exactly — only the type parameters and error type change, not the logic.
Before writing code, write the exact generic signature to
`planning/cat_framework/task_plan.md` for architect/user review — this is
the design decision `findings.md` §5 flags, and it is reviewed before
implementation per the standing "plan before code" rule.

Unit tests: an in-crate fake `CommandId`/`CommandTable` (never import a real
radio crate), mirroring how `cat-framework`'s own tests already work and how
`ts570d`'s `framework` tests never import `radio`.

Dependencies: `cat-framework` (for `CommandId`, `CommandTable<C>`,
`CommandDefinition<C>`), `cat-transport-core` (for `CatSession` — never a
concrete transport crate), `async-trait`. `monoio` as a dev-dependency only
(for `#[monoio::test]`), not a production dependency — per ADR 0002.

Done when: `cargo test -p cat-client` is green using only the in-crate fake
command table.

**Depends on Task 1 and Task 2.**

### Task 4a — `cat_transport` agent: implement `cat-transport-tcp`

New code — no `ts570d` source to move (TCP transport does not exist there).
`TcpCatSession` implementing `CatSession` over `monoio::net::TcpStream`
(consistent with ADR 0002), using **length-prefixed frames** — do not reuse
`SerialCatSession`'s semicolon-scanning loop. Document the exact frame layout
(prefix width, endianness, whether it includes the terminator) in
`planning/cat_transport/progress.md` precisely enough for Task 5 to build a
wire-compatible server-side listener from the writeup alone (see
`findings.md` §10). Reuse the `conformance` test module from
`cat-transport-core` against `TcpCatSession`, plus tests for partial reads,
oversized frames, and disconnect mid-frame.

Dependencies: `cat-transport-core`, `monoio`, `async-trait`, `thiserror`.

Done when: `cargo test -p cat-transport-tcp` is green, including conformance
tests reused unchanged from `cat-transport-core`.

**Depends on Task 2.**

### Task 4b — `cat_transport` agent: implement `cat-transport-udp`

New code. `UdpCatSession` implementing `CatSession` over
`monoio::net::UdpSocket`, using an **envelope format** (request/session IDs)
plus a **deduplication cache** — UDP guarantees neither delivery nor
ordering and is not connection-oriented; do not force connection-oriented
semantics onto it. Document the exact envelope layout and the dedup cache's
key/eviction policy in `planning/cat_transport/progress.md`, same
wire-compatibility requirement as Task 4a. Reuse the `conformance` test
module; add tests for duplicate delivery, out-of-order delivery, and a
never-answered request.

Dependencies: `cat-transport-core`, `monoio`, `async-trait`, `thiserror`.

Done when: `cargo test -p cat-transport-udp` is green, including reused
conformance tests and dedup-specific tests.

**Depends on Task 2. Independent of Task 4a** (could run in parallel as a
separate agent instance, but per the one-task-at-a-time workflow, run
sequentially after 4a unless the user asks to parallelize).

### Task 5 — `cat_server` agent: implement `cat-server`

New code. The request broker: client session management, ownership of the
physical radio session, a single ordered worker serializing all access,
request/response correlation by ID, timeout handling (a request the radio
never answers must not wedge the worker or starve other clients), disconnect
handling (client disappearing mid-request must not wedge anything), and
malformed-request rejection at the broker boundary (before reaching the
physical radio session). Server-side TCP/UDP accept/dispatch loops live here,
**wire-compatible with Task 4a/4b's exact framing** — read
`planning/cat_transport/progress.md`'s framing writeup before implementing
the listeners, don't re-derive the format.

Depends on `cat-client` (Task 3) — generic, not a concrete radio's client
type, per explicit instruction (a future `ft991a` server should reuse this
crate unchanged) — and a `CatSession` implementation for testing
(`ScriptedCatSession` from `cat-transport-core`, Task 2, is sufficient for
the broker's own unit tests; it does not need to wait on Task 4a/4b to unit
test the broker logic itself, only to exercise real TCP/UDP listener framing
end-to-end).

Must never add broker/client-id/queueing concepts to a radio's `CatRadio`
state machine — there is no radio state machine in this repository to touch,
but keep this in mind when documenting the contract `cat-server` expects from
whatever `CatRadio` implementation a consuming application supplies later.

Tests: happy path, timeout, disconnect, and malformed-request paths, not just
happy path, per the agent's own charter.

Done when: `cargo test -p cat-server` is green, including timeout/disconnect/
malformed-request tests.

**Depends on Task 3 for the client dependency; depends on Task 4a and 4b for
wire-compatible listener framing (see `findings.md` §10) even though it does
not take a Cargo dependency on either transport crate.**

## Summary / ordering

```
Task 1 (cat_framework: cat-framework)
   │
   ▼
Task 2 (cat_transport: cat-transport-core + cat-transport-serial)
   │
   ▼
Task 3 (cat_framework: cat-client)
   │
   ▼
Task 4a (cat_transport: cat-transport-tcp)
   │
   ▼
Task 4b (cat_transport: cat-transport-udp)
   │
   ▼
Task 5 (cat_server: cat-server)
```

Task 4a/4b depend only on Task 2 and could in principle run in parallel
across two agent instances; drawn sequentially above because they share one
subagent role and this repo's workflow processes one task per subagent at a
time with a review checkpoint between. Task 5 is the only task with a
same-crate ordering *and* a cross-crate wire-compatibility dependency, both
recorded above so it isn't planned in isolation from Task 4's framing
choices.

Not in this queue, and not dropped: migrating `ts570d` itself onto these
crates once they exist (see `findings.md` §4) — a separate planning pass,
later.

## Planning pass 2026-07-19: Windows serial backend (ADR 0002's revisit trigger fired)

Governing document: [ADR 0004](../../docs/adr/0004-windows-serial-backend.md)
— read it in full before dispatching any task below; it carries the async-
execution reasoning, the crate/module-structure decision, the full
`SerialConfig` ↔ `DCB` field mapping, and the `ModemControlLines` mapping
that these tasks implement. `planning/architect/findings.md` §11 records the
supporting research (what was read in `ft991a`/`ts570d`, and why each
rejected option was rejected).

**Scope note carried over unchanged from the ground rules above**: `ts570d`
and `ft991a` are not touched by any task below — read-only reference only.
This queue only touches `cat-transport-serial` (and, trivially, its
`Cargo.toml`). Both applications' own Windows entry-point follow-on work
(replacing `#[monoio::main]`, since `monoio` cannot compile on Windows at
all) is out of scope here, per ADR 0004 §1's "informational" note — it is a
future planning pass in each of those repos, not this one.

**Verification boundary, different from every task above**: this sandboxed
environment is Linux-only and cannot execute Windows binaries. Tasks 7–8's
"done when" is `cargo check --target x86_64-pc-windows-gnu -p
cat-transport-serial` (compiles cleanly cross-compiled) — never `cargo
test` for the Windows-specific code paths themselves. The user has stated
they will validate against real Windows hardware. Task 6's new code
(`oneshot.rs`) has no OS dependency of its own and gets real, executable
`cargo test` coverage on Linux, same as every other task in this file.

### Task 6 — `cat_transport` agent: extract shared `SerialConfig`/`Parity`/`FlowControl`; add the portable completion primitive

Two pieces of groundwork, one dispatch slot, both prerequisites for Task 7/8:

1. **Behavior-preserving refactor of the Linux path.** Move
   `SerialConfig`, `Parity`, `FlowControl` (pure data, no platform-specific
   code, currently defined inline in `io_uring.rs`) into a new, ungated
   `cat-transport-serial/src/config.rs` — same fields, same `Default` impl,
   same doc comments, byte-for-byte behavior. Update `io_uring.rs` to `use
   crate::config::{SerialConfig, Parity, FlowControl};` instead of defining
   them. Update `lib.rs`'s re-exports to pull these three from `config`
   rather than `io_uring` (still `pub use config::{FlowControl, Parity,
   SerialConfig};`, `SerialPort` re-export unchanged for now). This must not
   change any existing test's behavior — run the full existing
   `cat-transport-serial` suite before and after and confirm no diff in
   pass/fail.
2. **New portable completion primitive**, `cat-transport-serial/src/
   oneshot.rs` (private module, not part of the crate's public API — it is
   an internal implementation detail of the Windows worker-thread design,
   not a `Transport`-facing type): `struct Completion<T>` /
   `fn channel<T>() -> (CompletionTx<T>, CompletionRx<T>)`, a
   `Mutex<Option<T>> + Option<Waker>` pair; `CompletionRx<T>: Future<Output
   = T>` (or `Result<T, Canceled>` if the sender can be dropped before
   sending — needed for the worker-thread-exits-mid-request case). See ADR
   0004 §1 for the exact shape and the reasoning for why this is not "a
   third async runtime." Real unit tests here (this file has zero OS
   dependency, runs on Linux CI): poll-before-ready returns `Pending` and
   registers a waker; a value sent from a spawned `std::thread` after a
   delay wakes and resolves the receiver with the correct value; dropping
   the sender before sending resolves the receiver to an error rather than
   hanging forever.

Dependencies: no new external crate for this task (`oneshot.rs` is pure
`std`). Cargo.toml is otherwise untouched in this task — `windows-sys` is
Task 7's addition, not this one's.

Done when: `cargo test -p cat-transport-serial` is green (existing Linux
tests unaffected, new `oneshot` tests passing); `cargo clippy`, `cargo fmt`
pass.

### Task 7 — `cat_transport` agent: Windows `SerialPort::open` — `CreateFileW`, `DCB` configuration, `SetCommTimeouts`

Add the `windows-sys` dependency and the Windows module's open/configure
path only — no `Transport`/`ModemControlLines` implementation yet (that is
Task 8), so this task's surface is independently reviewable and
independently `cargo check`-able.

- `Cargo.toml`: add
  `[target.'cfg(target_os = "windows")'.dependencies] windows-sys = { version
  = "0.59", features = ["Win32_Foundation", "Win32_Storage_FileSystem",
  "Win32_Devices_Communication"] }` (confirm the exact current `windows-sys`
  version against what's already available/vendored; do not add
  `Win32_System_IO` — see ADR 0004 §3 for why overlapped I/O is deliberately
  out of scope). Mirrors the existing
  `[target.'cfg(target_os = "linux")'.dependencies] monoio` entry exactly —
  same section shape, same "fails fast on a non-matching host" property.
- New `cat-transport-serial/src/windows.rs`, `#[cfg(target_os =
  "windows")]`-gated from `lib.rs`. Implement:
  - A raw `HANDLE` newtype (`unsafe impl Send` — justify the safety
    argument in a doc comment, mirroring `io_uring.rs`'s existing SAFETY
    comment style for its own unsafe blocks).
  - `SerialPort::open(path: &str, config: SerialConfig) ->
    crate::SerialResult<Self>`: prepend `\\.\` to `path` if not already
    present (ADR 0004 §3 — required for COM10+, harmless for lower
    numbers), `CreateFileW`, map `ERROR_FILE_NOT_FOUND`/
    `ERROR_PATH_NOT_FOUND` → `SerialError::DeviceNotFound`,
    `ERROR_ACCESS_DENIED` → `SerialError::PermissionDenied`, else →
    `SerialError::Io`.
  - `configure_dcb(handle, &SerialConfig) -> SerialResult<()>`:
    `GetCommState`/mutate `DCB`/`SetCommState`, implementing every field
    per ADR 0004 §5's table exactly, including the deliberate
    baud-rate-validation parity choice (reuse `baud_rate_from_u32`'s
    validated rate set rather than accepting Windows' more permissive
    arbitrary-`u32` `DCB.BaudRate`).
  - `SetCommTimeouts`: `ReadIntervalTimeout = 0`,
    `ReadTotalTimeoutMultiplier = 0`, `ReadTotalTimeoutConstant =
    READ_TIMEOUT.as_millis()` (reuse the existing `#[cfg(not(test))]`
    2s / `#[cfg(test)]` 100ms split — do not redefine a second timeout
    constant), `WriteTotalTimeoutConstant` set to a generous bound (state
    the chosen value and reasoning in `progress.md`; ADR 0004 suggests 5s
    as a starting point, not a hard requirement).
  - `SerialPort::path(&self) -> &str` for parity with the Linux type's
    existing method.
  - Do not yet implement `Transport`, `ModemControlLines`, or the worker
    thread — stub or `todo!()` is acceptable for this task's boundary, but
    prefer leaving them entirely for Task 8 to add cleanly rather than
    half-writing them here.

Done when: `cargo check --target x86_64-pc-windows-gnu -p
cat-transport-serial` compiles cleanly (install the target via `rustup
target add x86_64-pc-windows-gnu` if not already present — this is a
`cargo check` cross-compile, not a link/run, so the `-gnu` target should not
require an actual Windows toolchain to type-check successfully; if it does
turn out to require one that isn't available in this environment, STOP and
report it rather than working around it by skipping verification — this is
exactly the kind of obstacle the agent's charter says to escalate, not
route around). `cargo clippy`/`cargo fmt` for whatever compiles.

**Depends on Task 6** (needs `config.rs`'s `SerialConfig`/`Parity`/
`FlowControl` to exist and be shared, not duplicated).

### Task 8 — `cat_transport` agent: Windows `Transport`/`ModemControlLines` — worker thread, completion wiring, `EscapeCommFunction`/`GetCommModemStatus`

Completes the Windows `SerialPort` from Task 7:

- Worker-thread request enum and `std::sync::mpsc` channel, per ADR 0004
  §1's design: `Write(Vec<u8>, CompletionTx<Result<usize, TransportError>>)`,
  `Read(usize, CompletionTx<Result<Vec<u8>, TransportError>>)`. The worker
  thread owns the `HANDLE`, performs blocking, non-overlapped
  `ReadFile`/`WriteFile`, and reports results via the `oneshot.rs`
  primitive from Task 6. A `ReadFile` that succeeds with 0 bytes (timeout
  elapsed, no data) maps to `Err(TransportError::ReadTimeout)` — `Ok(0)`
  must never reach a caller, matching the Linux contract exactly.
- `impl Transport for SerialPort`: `write`/`read` send a request and
  `.await` the `CompletionRx`; `flush_rx` calls `PurgeComm(handle,
  PURGE_RXCLEAR)` directly and synchronously (not via the worker thread —
  same reasoning as `ModemControlLines`); `flush` calls
  `FlushFileBuffers(handle)` directly and synchronously inside the `async
  fn` body (ADR 0004 §3's deliberate, narrow exception, mirroring the
  Linux `tcdrain` precedent — do not route this through the worker thread).
- `impl ModemControlLines for SerialPort`: direct, synchronous calls on the
  calling thread against the same `HANDLE` the worker thread uses — `
  set_rts`/`set_dtr` via `EscapeCommFunction(SETRTS/CLRRTS/SETDTR/CLRDTR)`,
  `read_cts`/`read_dsr`/`read_dcd` via `GetCommModemStatus` testing
  `MS_CTS_ON`/`MS_DSR_ON`/`MS_RLSD_ON`. Do not route these through the
  worker thread or the completion primitive.
- `SerialPort::open` (from Task 7) now also spawns the worker thread after
  `configure_dcb`/`SetCommTimeouts` succeed, and calls `self.set_rts(true)`/
  `self.set_dtr(true)` post-open when `config.initial_rts`/`initial_dtr`
  are set — identical sequencing to the Linux implementation.
- `Drop for SerialPort` (Windows): drop the request sender (worker's
  `recv()` then returns `Err`, its loop exits), join the thread, then
  `CloseHandle`.
- Update `lib.rs`: `#[cfg(target_os = "windows")] pub mod windows;
  #[cfg(target_os = "windows")] pub use windows::SerialPort;` alongside the
  existing Linux `pub use io_uring::SerialPort;`, both now gated (today
  only the module is implicitly Linux-only; after this task both platforms'
  `SerialPort` re-export must be explicitly `cfg`-gated so exactly one
  compiles per target).

Done when: `cargo check --target x86_64-pc-windows-gnu -p
cat-transport-serial` compiles cleanly with `Transport`/`ModemControlLines`
fully implemented (same verification boundary as Task 7 — no `cargo test`
for Windows-specific code in this sandbox). `cargo clippy`/`cargo fmt` pass.
Existing Linux `cargo test -p cat-transport-serial` remains green and
unaffected (confirm explicitly in `progress.md` — this is the same
"Linux path completely unaffected" bar every prior task in this queue was
held to).

**Depends on Task 6 and Task 7.**

## Summary / ordering (Windows backend)

```
Task 6 (cat_transport: config.rs extraction + oneshot.rs primitive)
   │
   ▼
Task 7 (cat_transport: Windows open/DCB/SetCommTimeouts)
   │
   ▼
Task 8 (cat_transport: Windows Transport/ModemControlLines + worker thread)
```

Not in this queue, and not dropped: `ft991a`'s and `ts570d`'s own Windows
entry-point work (replacing `#[monoio::main]`), per ADR 0004 §1 — a future
planning pass in each of those repositories, gated on Task 8 landing here
first and, per this repo's own ground rules, on `ts570d`/`ft991a` not being
touched by this session.

### Task 9 — `cat_rigctl` agent: `BrokerCatSession` moves into `cat-server`; new `cat-rigctl` crate (generic rigctld bridge + server orchestration)

Dispatched directly (not from a prior architect review cycle — recorded
here retroactively per this repo's planning-with-files requirement).
Source: both `ft991a` and `ts570d` independently hand-wrote a Hamlib
rigctld-compatible TCP bridge in their own `server` crates —
`broker_session.rs` (100% duplicated), `rigctl.rs` (~90% duplicated;
`ft991a`'s copy carried two real interop bugfixes `ts570d`'s copy lacked),
and `lib.rs::run()` (~90% duplicated; `ts570d`'s copy carried a real
error-propagation bugfix `ft991a`'s copy lacked). See ADR 0005 for the full
design record.

Done:
- `BrokerCatSession` relocated into `cat-server/src/broker_session.rs`
  (`pub use`d from `cat-server`'s root, `mod` kept private) — doc comments
  genericized, unit tests reuse `cat-server::test_fixtures`'s existing
  `FakeCommand`/`TABLE` fixture instead of a third private copy.
- New workspace member `cat-rigctl/`: `RigctlRadio` trait (see ADR 0005 for
  the exact signature), `ServerConfig`, `run<C, S, R, F>(session, table,
  config, make_radio)` (ported from `ft991a`'s `rigctl.rs`/`run()` shape
  with `ts570d`'s error-propagation fix applied), plus a private `rigctl`
  module (`dispatch`/`dump_state`/`LineReader`/`serve`, generic over
  `R: RigctlRadio`). Unit tests ported to an in-crate `FakeRadio:
  RigctlRadio`/`FakeMode` fixture (mirroring `cat-server::test_fixtures`'s
  pattern) — no dependency on either app's `radio` crate. Both the
  `run_with_no_listeners_configured_returns_an_error` test (from `ft991a`)
  and the `select_all_over_a_failing_listener_task_propagates_its_error`
  regression test (from `ts570d`, guarding the error-propagation fix) are
  ported.
- `futures = "0.3.30"` added to root `Cargo.toml`'s
  `[workspace.dependencies]` (new to this repo — `cat-rigctl`'s `run()`
  needs `futures::future::select_all`, matching both apps' existing
  `server/Cargo.toml` dependency).
- `cargo build/test/clippy/fmt --workspace` all clean; `cargo tree -p
  cat-rigctl` confirmed to contain no radio-specific or unexpected
  dependencies.

**Not done in this session** (explicitly out of scope, per this task's
charter): migrating `ft991a`'s/`ts570d`'s own `server` crates onto
`cat-rigctl` and deleting their local copies — both repos are read-only
ground truth here. That migration is the next phase, in each app's own
working copy, by a different agent.

---

## Planning pass 2026-08-27: capability model, normalized signal, `cat-ui`, native MSVC (ADRs 0010–0012)

Governing documents, read in full before dispatching anything below:
[ADR 0010](../../docs/adr/0010-capability-model-and-normalized-signal-source.md)
(capability model, multi-endpoint transports, `SpectrumSource`, native
protocol with rigctl as a compatibility layer),
[ADR 0011](../../docs/adr/0011-cat-ui-base-widgets-radio-specific-layout.md)
(`cat-ui` base widgets; layout and features stay radio-specific), and
[ADR 0012](../../docs/adr/0012-native-msvc-windows-target.md)
(`x86_64-pc-windows-msvc` as the single Windows target).

**All three ADRs are `Proposed`. This queue does not open until they are
Accepted.** The `CLAUDE.md` amendments in `ts570d` and `ft991a` are
correspondingly marked "not yet in force."

**Scope note, carried over unchanged**: `ts570d`, `ft991a` and `ic7100` are
not touched by any task below — read-only reference only. The app-side
work (each `server` adopting the native protocol, deleting its
`rigctl_radio.rs`, and `ts570d`'s new `gui` crate per its own ADR 0008) is
a follow-on planning pass **in each app's own repository**, gated on this
queue. Not dropped; not here.

**Verification boundary, changed by ADR 0012**: `x86_64-pc-windows-gnu` is
retired. From Task 10 onward, a task's Windows "done when" is a green
`windows-latest` CI job running `cargo check` **and `cargo test`** — not a
Linux-hosted cross-compile type-check. Prior tasks' `-gnu` criteria in this
file are historical records of how they *were* verified and must not be
rewritten.

**Sequencing, settled with the user**: the library work lands first.
Nothing in the apps starts until this queue's gate (Task 13) has passed.

**Prerequisites owned by the user, not by any agent**: a calibrated
`trim_hz` value for the TS-570D CN4 tap (measured against a known carrier —
WWV 10 MHz), needed before Task 15 has a real `IfTapConfig`; and an IC-7100
manual to close ADR 0010's one remaining unverified bandscope claim.

**Runs in parallel, not in this queue**: the console design process
(`ts570d/.claude/agents/designer.md`). It has no dependency on any task
below — mockups are throwaway HTML and the information hierarchy does not
depend on wire framing. Starting it during this queue is what keeps the
GUI from waiting on design iterations later.

### Task 10 — `release_workflow` agent: migrate to `x86_64-pc-windows-msvc`, retire `-gnu`

**First, because it changes the "done when" bar for every task after it.**

Per ADR 0012: move the CI `windows-check` job from an Ubuntu runner
cross-compiling `-gnu` to a `windows-latest` runner, and upgrade it from
`cargo check` to `cargo check` **plus `cargo test`** across the
Windows-capable crates (`cat-transport-serial`, `cat-transport-tcp`,
`cat-transport-udp`, `cat-transport-core`, `cat-diagnostics`, `cat-server`,
`cat-rigctl`). Remove `-gnu` from `Makefile`, `CLAUDE.md` and `README.md`.
Add amendment notes — **not edits** — to ADRs 0004, 0006, 0007 and 0008
pointing at ADR 0012.

Evaluate `cargo-xwin` for a best-effort local Linux check and report both
ADR 0012 open caveats (whether it handles GPU-adjacent crates; whether its
Microsoft SDK/CRT downloads are licence-clean for CI). **If either is
blocking, report it — do not work around it.** No local check at all is an
acceptable outcome; ADR 0012 says so explicitly.

Done when: `windows-latest` CI is green on `check` + `test`; no `-gnu`
reference remains outside historical ADR/planning text; the `cargo-xwin`
findings are written to `planning/release_workflow/findings.md`.

**Expect Windows test failures here, and treat them as the point.** ADR
0006 §4 records that the Windows test modules have never actually executed.
This is the first time they run. Any failure is a real, pre-existing bug —
report it, do not paper over it to make the job green.

### Task 11 — `cat_framework` agent: `cat-framework::capabilities`

New `capabilities` module in `cat-framework`: `RadioCapabilities`,
`EndpointSet`/`EndpointDescriptor`/`EndpointRole`, `VfoCapability`,
`ModeDescriptor`, `FilterCapability`, `MeterSet`, `MemoryCapability`,
`MenuCapability` — exactly the shapes in ADR 0010 §1–2.

Pure data. No wire format, no protocol opinion, no `SpectrumSource`
dependency (`SignalCapability` is re-exported from `cat-signal` once Task 12
lands; until then, leave the field out rather than duplicating the type).

Done when: `cargo test -p cat-framework`, `clippy`, `fmt` clean; Task 10's
Windows job still green.

### Task 12 — `cat_signal` agent: create `cat-signal`

New workspace member. Types and one trait, per ADR 0010 §3–4: `SpectrumFrame`,
`SpectrumSource`, `SignalCapability`, `IfTapConfig`, `SpectrumSettings`,
`SettingDescriptor`, `SettingValue`, `SettingGroup`, `Access`, `Unit`.

**Zero dependencies beyond `async-trait`.** No driver, no FFT, no
`monoio`. `#[async_trait(?Send)]` per ADR 0002's binding.

Two invariants must be enforced by tests, not just documented:

1. `SpectrumFrame::bins` is **always low-frequency-first**. Inversion is
   corrected inside a source, never by a consumer.
2. `SignalCapability::AudioDerived` carries `max_bandwidth_hz` so a
   consumer can refuse to render it as a band panorama.

`NativeScope` is **defined but not implemented** — no radio in the fleet
exports a bandscope (ADR 0010, Context). Do not write speculative code for
it.

Ship an in-crate `FakeSpectrumSource` fixture (the `cat-server::test_fixtures`
pattern) so Tasks 16 and 19 have something to test against.

Done when: `cargo test -p cat-signal`, `clippy`, `fmt` clean; Windows job
green.

### Task 13 — `cat_framework` agent: describe two radios — **GATE**

**This is the gate. Nothing downstream is dispatched until it passes.**

ADR 0010 concedes `RadioCapabilities` "is a guess until three radios are
actually described by it." Describe **two** now, as test fixtures inside
`cat-framework` (the apps are read-only per the ground rules — read them,
do not modify them):

- **TS-570D** — one endpoint filling `Cat` + `Keying` together;
  `SignalCapability::IfTap` with `if_center_hz: 73_050_000`, `inverted:
  true`; no memory-scope; the meter set the radio actually reports.
- **FT-991A** — the stressing case, and the reason two is the minimum:
  **three** endpoints (Cat and Keying on separate CP210x virtual COM ports,
  plus a USB `Audio` codec), per `ft991a` ADR 0002. Its `control.rs` is 5.4×
  `ts570d`'s, so its menu and filter capability is where the model bends if
  it is going to.

Done when both fixtures compile and read naturally **without** adding an
escape hatch — no `Option<serde_json::Value>`, no `extra: HashMap`, no
`model: &str` that downstream code matches on. Any of those means the model
failed and Task 11/12 get revised **before** Tasks 16–19 build on it.

Report the outcome to the architect and user explicitly, including anything
that did not fit cleanly. A grudging fit is a failure, not a pass.

### Task 14 — `architect`: ADR for `cat-signal-rtlsdr`

An ADR, not code. ADR 0010 §5 deferred four things that must be decided
before implementation: the librtlsdr binding choice; whether the FFT runs on
a worker thread or in the frame pump; backpressure policy when a consumer
falls behind; and — per ADR 0012 §4 — the **Windows driver story**, which is
materially different (WinUSB via Zadig, libusb from a non-pkg-config source,
different linkage). That last one is a platform port, not a `cfg` detail, and
is the reason this is an ADR rather than a design note inside Task 15.

Done when: the ADR is written, numbered, added to `docs/adr/README.md`, and
reviewed by the user.

### Task 15 — `cat_signal` agent: implement `cat-signal-rtlsdr`

The `IfTap` source, per Task 14's ADR: read IQ, window, FFT, magnitude,
apply `IfTapConfig`, emit `SpectrumFrame`.

`retune(dial_hz)` does what `ts570d/if-panadapter-bridge.py` did with gqrx's
`LNB_LO`, except **no frequency ever reaches the SDR** — the dongle stays
parked on the IF and `center_hz` is computed as `dial_hz + trim_hz`. That is
why the trim is a constant and no ppm correction exists anywhere in this
crate. Read that script before starting; it is the working spike.

Done when: `cargo test -p cat-signal-rtlsdr` clean; a live capture against
the TS-570D CN4 tap produces frames whose `center_hz` tracks the dial and
whose bins are low-frequency-first (i.e. a signal above the dial appears to
the **right**, proving the inversion correction).

### Task 16 — `cat_server` agent: native protocol

Per ADR 0010 §6. `cat-server` serves the native typed protocol on its own
port: capability handshake on connect, typed JSON state, typed commands
validated against the capability set, and separately framed binary
`SpectrumFrame`s that a client which declines them never pays for.

Versioned in the handshake from the first commit.

Done when: `cargo test -p cat-server` clean, including a test that a client
declining the spectrum channel receives no frame traffic; Windows job green.

### Task 17 — `cat_rigctl` agent: reimplement `cat-rigctl` over `RadioCapabilities`

**The highest-risk task in this queue. Last, deliberately.**

Rigctl stays a compatibility layer on its own port with unchanged wire
behaviour. What changes is what it sits on: instead of each app hand-writing
a `RigctlRadio` impl (`ts570d` 269 lines, `ft991a` 221), `cat-rigctl` is
implemented **once** against `RadioCapabilities`, and `\dump_state`'s
capability tail is **generated** rather than hand-maintained.

Done when: the existing conformance behaviour passes unchanged, ADR 0005's
two interop fixes are present **as regression tests** (`\dump_state`'s
capability-tail field count; `F`'s `%f`-formatted float parsing), and the
bridge is verified against a live Hamlib client. **WSJT-X must not regress
— that is the acceptance bar, not a nice-to-have.**

### Task 18 — `cat_ui` agent: create `cat-ui` (renderer-agnostic)

Per ADR 0011. Presentation logic for radio concepts with **no renderer
dependency** — no `egui`, no `ratatui`, no `wgpu`: frequency and S-unit
formatting, meter scaling, band plans, the spectrum ring buffer, and the
two-rate discipline (spectrum frames push at ~60 fps; CAT state is
request/response and slow — separate lanes, so no renderer can block a
waterfall behind a menu read).

Done when: `cargo test -p cat-ui` clean. This crate is headlessly testable
by construction — meter scaling and frequency formatting get real unit tests
for the first time in this project.

### Task 19 — `cat_ui` agent: create `cat-ui-egui`

The GPU widget set. **Waterfall first** — it is the long pole, the most
reusable artifact in the whole plan, and the thing that justifies the
framework choice: a custom `wgpu` render pass via `egui_wgpu`'s callback,
owning a scrolling texture, `SpectrumFrame` bins going socket → texture
upload → shader with no per-pixel CPU work.

Then spectrum plot, analog S-meter, rotary tuning knob, bar meters driven by
`MeterSet`, VFO readout, band and mode grids, and **one** generic renderer
for ADR 0010 §4's `SettingDescriptor` lists.

Per ADR 0012 §4, gate `wgpu` backend selection (DX12 vs Vulkan/GL), DPI, and
window creation with explicit `#[cfg(target_os = "windows")]`.

**The seam rule, and it will be tested early**: a widget here takes
normalized data and draws it. It never decides whether it should be on
screen, or where. If a proposed parameter exists to express one radio's
preference, the widget belongs in that radio's crate instead — reject it
here.

> **Superseded 2026-08-27** by the amendment at the end of this file.
> This paragraph originally read that `cat-ui-ratatui` was not in this
> queue. ADR 0011 revision 4 and ADR 0013 put it in: see **Task 20**.

Done when: `cargo test -p cat-ui-egui` clean; the waterfall renders a
`FakeSpectrumSource` stream at 60 fps on Linux and on Windows CI.

## Summary / ordering (capability + signal + UI)

```
Task 10 (release_workflow: MSVC target, -gnu retired)   ← first: changes the bar
   │
   ├──▶ Task 11 (cat_framework: capabilities)  ─┐
   │                                            ├──▶ Task 13 (GATE: describe 2 radios)
   └──▶ Task 12 (cat_signal: types + trait)  ───┘         │
                                                          ▼
                              ┌───────────────────────────┼───────────────────┐
                              ▼                           ▼                   ▼
                    Task 14 (ADR: rtlsdr)        Task 16 (native protocol)  Task 18 (cat-ui)
                              │                           │                   │
                              ▼                           ▼                   ▼
                    Task 15 (cat-signal-rtlsdr)  Task 17 (cat-rigctl rewrite) Task 19 (cat-ui-egui)
```

Tasks 11 and 12 are independent and could be dispatched in either order;
this repo's one-task-per-dispatch ground rule applies regardless.

**Critical path**: 10 → 11 → 13 → 16 → 19 → (app-side `gui`, next pass).
Tasks 14/15 (RTL-SDR) and 17 (rigctl) sit off it and can move whenever a
slot is free.

**Not in this queue, and not dropped**: each app's migration onto the
native protocol and `cat-ui-egui`; deletion of `ts570d/server/src/rigctl_radio.rs`
and `ft991a`'s equivalent; `ts570d`'s `gui` crate (its ADR 0008);
`cat-ui-ratatui` plus the TUI divergence audit; audio streaming for
`EndpointRole::Audio`/`AudioDerived`; and `NativeScope`, when a radio that
actually has one enters the fleet.

## Amendment 2026-08-27 — renderer parity (ADR 0013, ADR 0011 rev 4)

User direction: *"we always want the tui and the gui to contain similar
features, within reason. TUI will not go away. We also want a generic set of
TUI components as well."*

Recorded as [ADR 0013](../../docs/adr/0013-renderer-parity-tui-and-gui.md)
(capability parity between renderers; the TUI is permanent) and ADR 0011
revision 4 (`cat-ui-ratatui` promoted from deferred to in-scope; the TUI
divergence audit becomes the extraction rather than a gate before it).

Two things above change:

- **Task 19's closing paragraph is superseded** — `cat-ui-ratatui` is now
  Task 20 below, not a future planning pass.
- **Task 18 (`cat-ui`) gains a named extraction target list.** It was
  already the renderer-agnostic crate; it is now the crate that settles most
  of the divergence audit, so the audit's first cases are called out
  explicitly rather than left to discovery.

Nothing else in the queue moves. Tasks 10–17 are unaffected.

### Task 18 — addendum: named extraction targets

Extract these first, in this order, because they are the audit's cheap cases
and they establish the pattern for the rest:

1. **`format_hz()`** — byte-identical in `ts570d/ui/src/layout.rs:29` and
   `ft991a/ui/src/layout.rs:69`. A straight lift; no decision to make.
2. **`mini_bar()`** — byte-identical in both (`ts570d` :72, `ft991a` :99).
   Same.
3. **`smeter_bar()`** — the first real decision, and the one that proves the
   seam. Identical glyph vocabulary (`▐ █ ░ ▌`) and structure; `ts570d`
   takes `u16` over 0–30 with width hardcoded to 20, `ft991a` takes `u8`
   over 0–255 with `width` a parameter. **The scale is `MeterSet` data, not
   a widget preference** — one implementation parameterized on the radio's
   reported meter range serves both. Getting this wrong in either direction
   (hardcoding a scale, or adding a per-radio taste knob) is the failure
   mode ADR 0011 names.
4. **`smeter_label()`** — exists in `ts570d` (:37) only; `ft991a` shows a
   bar with no S-unit called out. Extract it, and **report** whether the
   FT-991A's absence was a decision or an omission. Do not silently add it
   to the FT-991A TUI — that is an app-side change, and under ADR 0013 it
   needs a row in that app's `docs/renderer-parity.md` either way.

Also in `cat-ui`: meter scaling against `MeterSet` ranges, band plans, the
spectrum ring buffer, and the two-rate discipline, as already specified.

**Escalate, do not widen.** If a case turns out to be genuinely
irreconcilable between the two radios, report it — the answer is that it
belongs in an app's layout, not that the shared widget grows a parameter.

### Task 20 — `cat_ui` agent: create `cat-ui-ratatui`

Per ADR 0011 rev 4 and ADR 0013. The terminal widget set, covering the same
concept list as `cat-ui-egui`: coarse spectrum and half-block waterfall,
S-meter bar with S-unit label, meter rail, VFO readout, band and mode grids,
menu column builders, the connection/error/disconnected panels, and the same
generic renderer for ADR 0010 §4's `SettingDescriptor` lists.

**The spectrum is the point of this task, not an afterthought.** ADR 0013
§2(a) permits an exception on *fidelity*, never on *presence*: a terminal
gets a coarse, low-frame-rate spectrum and waterfall fed by the same
`SpectrumFrame`s, not a blank panel and not a "use the GUI" message. Build
it against `cat-signal`'s `FakeSpectrumSource` (Task 12).

Depends on Task 18 only. **Independent of Task 19** — the two renderer
crates can be dispatched in either order, or by two agents in parallel if
the one-task-per-dispatch rule is relaxed for them. If a slot forces a
choice, take this one first: it is lower risk than the wgpu pass and it
unblocks the app-side TUI migrations.

Done when: `cargo test -p cat-ui-ratatui` clean; the widget set renders a
`FakeSpectrumSource` stream in a terminal; every concept `cat-ui-egui`
renders has a counterpart here or a written justification under one of ADR
0013's three grounds.

### Ordering, amended

```
Task 18 (cat-ui: renderer-agnostic + audit's cheap cases)
   ├──▶ Task 19 (cat-ui-egui — waterfall first, GPU)
   └──▶ Task 20 (cat-ui-ratatui — coarse spectrum first)
```

Critical path is unchanged: **10 → 11 → 13 → 16 → 19 → app-side `gui`**.
Task 20 sits off it, but the app-side TUI migrations depend on it, so it
should not be left to last.

### Amended "not in this queue, and not dropped"

The earlier list stands, with these corrections:

- **`cat-ui-ratatui` is removed from it** — it is Task 20.
- **The TUI divergence audit is removed from it** — ADR 0011 rev 4 folds it
  into Tasks 18 and 20 rather than running it as a separate pass.
- **Added: migrating `ts570d/ui` and `ft991a/ui`** onto `cat-ui` +
  `cat-ui-ratatui`. App-side, per repo, per this queue's read-only ground
  rule. Acceptance bar is that the operator sees no change.
- **Added: each app's `docs/renderer-parity.md`**, the ADR 0013 exception
  table. Created during the app-side passes, seeded by whatever Tasks 18 and
  20 report.
