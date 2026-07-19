# Task Plan — cat_transport

## Task 2 (architect dispatch, `planning/architect/task_plan.md`) — DONE

Extraction is authorized (architect go-ahead dated 2026-07-16, superseding
the earlier "no extraction yet" placeholder that opened this file). Task 2:
create `cat-transport-core` and `cat-transport-serial`, sequenced internally
(core before serial, since serial builds on core's traits).

Governing documents (read in full before implementation, per this repo's
"plan before code" rule): `docs/adr/0001-scope-and-crate-boundaries.md` (as
amended), `docs/adr/0002-async-runtime-binding-for-transport-crates.md`,
`planning/architect/task_plan.md` Task 2, `planning/architect/findings.md`
§§6–7.

### `cat-transport-core` plan

Move from `ts570d` commit `1585e1e` (`refactor/generic-cat-framework`):
- `framework/src/transport.rs` → `src/transport.rs` (`Transport` trait,
  unchanged).
- `framework/src/session.rs` → `src/session.rs`, **`CatSession` trait
  only** (not `SerialCatSession`, which belongs to `cat-transport-serial`
  per the architect's dispatch).
- `framework/src/test_support.rs` → `src/test_support.rs` (`Exchange`,
  `ScriptedCatSession`, `conformance` module, unchanged).
- `framework/src/errors.rs`'s `TransportError` variant only → `src/errors.rs`
  (`FrameworkError`/`FrameworkResult` stay behind in `ts570d`, per Task 1's
  exclusion — not this crate's concern).
- `framework/src/lib.rs`'s `pub use monoio::{RuntimeBuilder,
  io::{AsyncReadRent, AsyncWriteRent}}` → this crate's `src/lib.rs` root,
  `cfg`-gated to `target_os = "linux"` to match the gated `monoio`
  dependency (ADR 0002).

Dependency: one-way on `cat-framework` (path dep) for
`ResponseDisposition`/`ProtocolErrorKind` reuse, per ADR 0001 Amendment 2 —
re-exported from this crate's root so downstream transport crates never need
their own `cat-framework` dependency.

### `cat-transport-serial` plan

Move from `ts570d` commit `1585e1e`:
- `framework/src/session.rs`'s `SerialCatSession<T: Transport>` (+ its
  local unit tests) → `src/session.rs`, now implementing
  `cat_transport_core::CatSession` instead of an in-crate trait.
- `ts570d`'s separate `serial` crate — `serial/src/io_uring.rs`
  (`SerialConfig`, `Parity`, `FlowControl`, `SerialPort`, `impl Transport
  for SerialPort`, termios/libc/nix plumbing, + its hardware-level PTY
  tests) → `src/io_uring.rs`. `serial/src/lib.rs`'s `SerialError`/
  `SerialResult` → this crate's `src/lib.rs`. Required per ADR 0001
  Amendment 3 / `findings.md` §6 — `session.rs` alone has no hardware
  behind it.

Dependency: `cat-transport-core` only in this workspace (never
`cat-framework` directly — `ResponseDisposition` reached via
`cat-transport-core`'s re-export). External: `monoio` (Linux target-gated),
`async-trait`, `thiserror`, `libc`, `nix` (`term` feature).

### Judgment call flagged before implementation (see `findings.md`)

`ts570d`'s `serial/src/io_uring.rs` test module depends on
`emulator::pty::PtyPair` (a separate `ts570d` crate wrapping
`serialport::TTYPort::pair()`). Neither `emulator` nor `serialport` is
authorized by the architect's dependency list for `cat-transport-serial`
(`cat-transport-core`, `monoio`, `async-trait`, `thiserror`, `libc`, `nix`).
Decision: build a minimal PTY-pair test helper directly on
`nix::pty::{posix_openpt, grantpt, unlockpt, ptsname_r}` (already available
via the `term` feature already required by the spec) instead of adding an
unauthorized dependency. This reproduces the exact same hardware-level test
behavior without changing any production code or the framing/`Transport`
design — recorded in `findings.md` rather than silently added.

## Status: implementation complete, awaiting architect/user review

Both crates created; root `Cargo.toml` workspace members updated to
`["cat-framework", "cat-transport-core", "cat-transport-serial"]`. All
acceptance checks green (see `progress.md`). Not committed, per this
session's standing rule. STOPPING here per the one-task-at-a-time
workflow — `cat-transport-tcp`/`cat-transport-udp` are separate later
tasks, not started.

## Task 4a (architect dispatch, `planning/architect/task_plan.md`) — new crate `cat-transport-tcp`

Authorized: architect dispatch queue Task 4a, depends only on Task 2
(`cat-transport-core`), which is done. New code, no `ts570d` source exists
for TCP.

Governing documents re-read before implementation: ADR 0002 (keep monoio,
Linux target-gated), `planning/architect/task_plan.md` Task 4a (verbatim
spec), `cat-transport-core/src/{transport,session,test_support}.rs` (current
`Transport`/`CatSession` trait shape + `conformance` module signatures, to be
reused unchanged), `cat-transport-serial/src/session.rs` (reference for code
shape/conventions only -- framing is NOT reused).

### Plan

`TcpCatSession` wraps a `monoio::net::TcpStream` directly (not generic over
`Transport<T>` -- unlike `SerialCatSession<T: Transport>`, since the task
names `monoio::net::TcpStream` specifically and TCP framing needs its own
read/write primitives, not the borrowed-slice `Transport::{read,write}`
shape). Implements `cat_transport_core::CatSession` with `type Error =
TcpSessionError`, a new crate-local `thiserror` enum (`Io(#[from]
std::io::Error)`, `FrameTooLarge { len: u32, max: u32 }`) -- a judgment call:
CatSession::Error is an associated type, nothing requires reusing
`TransportError` verbatim, and a dedicated "oversized frame" variant is
needed to reject cleanly rather than overload `TransportError::Other`.

Frame layout: 4-byte big-endian `u32` length prefix (payload bytes only, not
including itself), followed by exactly that many payload bytes, no
terminator. Zero-length frame = `ResponseDisposition::NoResponse`. Full
writeup goes in `progress.md` for Task 5 to consume verbatim.

`execute()`: write one request frame, then read exactly one response frame
in reply (1:1, connection-ordered -- no request/session IDs, that is UDP's
concern). `send()` NOT overridden (default forward-and-discard is safe here
*because* the wire protocol guarantees a response frame, possibly
zero-length, for every request -- this is a load-bearing requirement on
cat-server's listener, documented explicitly in `progress.md` for Task 5).

Max frame size: 65536 (64 KiB) payload bytes -- a judgment call, documented
with reasoning in `progress.md`.

Tests: reuse `cat_transport_core::conformance` functions unchanged against
`TcpCatSession` wrapping a real loopback `TcpStream`, with an in-test
"scripted peer" task on the other end. The peer encodes/decodes frames with
its own hand-rolled raw byte logic (not by calling `TcpCatSession`'s private
frame helpers) -- deliberate redundancy so the test suite independently
cross-checks the documented wire format against the production
implementation, rather than only checking the encoder against itself. Plus
dedicated tests: partial reads (prefix then payload in multiple chunks),
oversized frame (reject without attempting to read the declared payload),
disconnect mid-length-prefix, disconnect mid-payload.

Dependencies: `cat-transport-core`, `monoio` (Linux target-gated, per ADR
0002), `async-trait`, `thiserror`. First action: add `cat-transport-tcp` to
root `Cargo.toml`'s `[workspace] members`.

Done when: `cargo test -p cat-transport-tcp` green, `cargo clippy`/`cargo
fmt` clean, conformance tests passing unchanged.

## Task 4b (architect dispatch, `planning/architect/task_plan.md`) — new crate `cat-transport-udp`

Authorized: architect dispatch queue Task 4b, depends only on Task 2
(`cat-transport-core`), which is done. Independent of Task 4a; run after it
per the one-task-at-a-time workflow. New code, no `ts570d` source exists for
UDP.

Governing documents re-read before implementation: ADR 0002 (keep monoio,
Linux target-gated), `planning/architect/task_plan.md` Task 4b (verbatim
spec), `planning/cat_transport/progress.md`'s Task 4a section (TCP's frame
layout and its "every request gets exactly one response" flagged
requirement — read for terminology/style parity, NOT copied: UDP's envelope
is an independent design), `cat-transport-core/src/{transport,session,
test_support}.rs` (current trait shapes + `conformance` module, reused
unchanged), `cat-transport-tcp/src/session.rs` (reference for code shape only).

### Plan

`UdpCatSession` wraps a `monoio::net::UdpSocket` directly plus a fixed
`peer_addr: SocketAddr` (this session always talks to one specific remote
peer — a design choice, not a kernel-level `connect()`; see progress.md for
why `connect()` was deliberately avoided). Implements `CatSession` with
`type Error = UdpSessionError` (`Io`, `Timeout`, `PayloadTooLarge`).

Envelope: 16-byte header (8-byte big-endian `session_id` + 8-byte
big-endian `request_id`), followed by the payload — no length field needed
(a UDP datagram already preserves its own message boundary, unlike TCP's
byte stream). `session_id` is randomized once per `UdpCatSession` (via
`std::collections::hash_map::RandomState`, avoiding a `rand` dependency not
on the authorized list). `request_id` is a per-session monotonic counter.
Max payload 1024 bytes (judgment call, smaller than TCP's 64 KiB — chosen to
stay under typical path MTU and avoid IP fragmentation, since UDP has no
retransmission of fragments). Zero-length payload = `NoResponse`, matching
TCP's/`ScriptedCatSession`'s convention exactly.

Dedup cache: bounded FIFO (`VecDeque<u64>`, capacity 32) of request IDs this
session has already completed. `execute()` sends one request, then loops
`recv_from` (bounded by a single fixed deadline via `monoio::time::
timeout_at`, NOT a per-iteration reset — otherwise a flood of irrelevant
datagrams could extend the wait past the configured bound), discarding any
datagram that doesn't match `(peer_addr, session_id, the just-sent
request_id)` — including a datagram matching an ALREADY-COMPLETED
`request_id` (explicit dedup-cache hit) or one that's simply foreign/stale.
Because `request_id` is strictly monotonic and never reused, the mismatch
check alone is sufficient for correctness; the dedup cache is retained
anyway for defense-in-depth, explicit/testable classification, and because
the charter names it as a required design element — this is stated plainly
in `progress.md`, not overclaimed as load-bearing when it structurally isn't
today.

Timeout: unlike `TcpCatSession` (which deliberately has NO timeout of its
own, per Task 4a's writeup — TCP's peer disconnect gives an OS-level EOF
signal), `UdpCatSession` MUST have one, because UDP gives no signal at all
when a peer silently vanishes. `response_timeout: Duration` is a required
constructor parameter, applied as a single fixed deadline per `execute()`
call via `monoio::time::timeout_at`. This requires the consuming
application's monoio runtime to be built with `.enable_timer()` — documented
as a hard requirement, since `cat-transport-core`'s `RuntimeBuilder`
re-export doesn't do this by default.

Tests: reuse `cat_transport_core::conformance` unchanged (query round trip,
fire-and-forget, surfaces-transport-error — the latter via a peer that never
responds, timing out). Plus: duplicate delivery, out-of-order/stale
delivery, never-answered-with-bounded-wait (measuring elapsed wall time),
malformed/too-short datagram ignored as noise, foreign session-id ignored,
plus pure-logic unit tests on the dedup cache/encode/decode helpers directly
(no socket needed).

Dependencies: `cat-transport-core`, `monoio` (Linux target-gated, per ADR
0002), `async-trait`, `thiserror`. First action: add `cat-transport-udp` to
root `Cargo.toml`'s `[workspace] members`.

Done when: `cargo test -p cat-transport-udp` green, `cargo clippy`/`cargo
fmt` clean, conformance tests passing unchanged, dedup/out-of-order/timeout
tests passing.

## Task 6 (coordinating session dispatch, direct instruction, not via
## `planning/architect/`) — expose codec primitives as `pub` for `cat-server`

Authorized directly by the coordinating session's own code review of
`cat-server` (a separate agent's prior task): `cat-server`'s TCP/UDP
listeners had hand-rolled their own independent copies of the exact same
codec logic (frame/envelope encode-decode, size constants) instead of
importing this crate's, because the relevant functions were private. This
task makes the building blocks `pub` (visibility/API-surface change only --
no wire-visible behavior change, no redesign) so a follow-up `cat_server`
task can delete its duplicated codec and import these instead.

### Plan

`cat-transport-tcp/src/session.rs`:
- `write_frame` -> `pub async fn` (no behavior change; write side has no
  EOF-tolerance concern, symmetric for a request or a response frame).
- `read_frame` -> re-shaped into a thin `pub async fn` wrapper over a new
  `pub async fn read_frame_or_eof(&mut TcpStream) -> Result<Option<Vec<u8>>,
  TcpSessionError>`, per the task's option (a). `read_frame_or_eof` returns
  `Ok(None)` only when a clean disconnect is observed with *zero* bytes of
  a new frame already read (a boundary hangup, not an error); any I/O
  failure after at least one byte of the length prefix or payload has
  arrived is `Err` (mid-frame -- the connection state is unrecoverable).
  `read_frame` calls it and turns `None` into the same `Io(UnexpectedEof)`
  shape it always returned, so `TcpCatSession::execute` (client-shaped, no
  EOF-tolerance needed) is unaffected.
- New private helper `read_exact_or_eof` implements the byte-count
  bookkeeping this distinction needs: monoio's own `AsyncReadRentExt::
  read_exact` does not expose how many bytes were actually transferred
  before hitting EOF (only success/failure), so a clean boundary EOF and a
  mid-header EOF both surface identically from it -- `read_exact_or_eof`
  reimplements the same read loop shape but tracks `bytes read so far`
  itself so the two cases can be told apart. Kept crate-private (not
  `pub`) -- no caller needs an EOF-tolerant exact-length read for anything
  other than this crate's fixed 4-byte length prefix today.
- `MAX_FRAME_SIZE` already `pub`, unchanged. `lib.rs` now re-exports
  `write_frame`, `read_frame`, `read_frame_or_eof` alongside it.

`cat-transport-udp/src/session.rs`:
- `encode_envelope`/`decode_envelope` -> `pub fn`, signatures unchanged.
  `ENVELOPE_HEADER_LEN`/`MAX_PAYLOAD_SIZE` already `pub`, unchanged.
  `lib.rs` now re-exports `encode_envelope`/`decode_envelope` alongside
  them.
- `SESSION_ID_LEN`/`REQUEST_ID_LEN` visibility left untouched, per the
  task's explicit instruction -- confirmed by reading (not editing)
  `cat-server/src/udp.rs`: its own duplicated constants of the same names
  are only ever used internally by its own `decode_envelope` copy; nothing
  in that file needs `SESSION_ID_LEN`/`REQUEST_ID_LEN` split out separately
  from `ENVELOPE_HEADER_LEN` once it switches to importing this crate's
  `encode_envelope`/`decode_envelope`/`ENVELOPE_HEADER_LEN`/
  `MAX_PAYLOAD_SIZE` directly.

`cat-server/` was read only, never edited (out of scope per the task and
per `.claude/agents/cat_transport.md`'s crate boundary). `ts570d`/`ft991a`
untouched.

### Status: implementation complete, awaiting review

Not committed, per standing rule. Full details and acceptance-check output
in `progress.md`'s Task 6 section.

## Task 7 (coordinating session dispatch, direct instruction) — `ModemControlLines`
## trait for RTS/DTR/CTS/DSR/DCD, new capability for `ft991a`'s PC KEYING support

Authorized directly by the coordinating session: `ft991a` (sibling repo,
FT-991A radio control) needs direct control of RS-232 modem control lines
(RTS, DTR) and status lines (CTS, DSR, DCD) independent of byte-level CAT
framing, for the radio's Menu 060 "PC KEYING" (RTS/DTR hardware PTT/CW
keying as an alternative to CAT commands). Generic serial-transport
capability per `docs/adr/0001-scope-and-crate-boundaries.md`'s boundary
rules — belongs in `cat-transport-serial`/`cat-transport-core`, not
duplicated in a radio crate.

### Plan

New `ModemControlLines` trait in `cat-transport-core/src/modem.rs` (new
file, re-exported from `lib.rs` alongside `Transport`/`CatSession`), exact
shape dictated by the task (verbatim signatures). Plain sync fns (no
`#[async_trait]`) — direct ioctl(2) calls, no I/O wait, matching the
existing `Transport::flush_rx`/`CatSession::flush_rx` precedent (both
already plain sync fns on otherwise-async traits for the same reason). Not
folded into `Transport`/`CatSession` — TCP/UDP have no physical modem
lines, so this stays a separate, additively-implemented capability trait, a
consumer bounds on `S: CatSession + ModemControlLines` rather than
requiring it universally. `TransportError` (already has `Io(#[from]
std::io::Error)`) is reused verbatim for ioctl failures — `std::io::Error::
last_os_error()`/`from_raw_os_error()` after a `-1` return from
`libc::ioctl` fits this exactly, no new variant needed.

`cat-transport-serial`: `impl ModemControlLines for SerialPort` in
`io_uring.rs`, generalizing the existing constructor-time-only RTS+DTR
assert block (`TIOCMBIS`/local `TIOCM_RTS`/`TIOCM_DTR` consts) into runtime
`&self` methods, reusing `TIOCMBIS`/`TIOCMBIC` (set/clear via ioctl) for
`set_rts`/`set_dtr` and `TIOCMGET` (read status register, test bit) for
`read_cts`/`read_dsr`/`read_dcd`. New consts needed: `TIOCM_CTS = 0x020`,
`TIOCM_DSR = 0x100`, `TIOCM_CAR = 0x040` (DCD). `SerialPort::open`'s
existing inline assert block refactored to call the new `set_rts`/`set_dtr`
methods instead of duplicating the ioctl call, preserving the exact
ignore-ENOTTY-on-PTY behavior already documented there.

Blanket delegating `impl<T: Transport + ModemControlLines> ModemControlLines
for SerialCatSession<T>` in `session.rs`, copying the exact delegation shape
`CatSession::flush_rx`'s `self.transport.flush_rx()` already uses.

Judgment call (per the task's explicit "consider, not required" framing):
add `initial_rts: bool` / `initial_dtr: bool` to `SerialConfig`, default
`true` via `Default` (preserves today's unconditional-assert behavior
exactly). Confirmed low-risk before doing it: grepped every `SerialConfig`
construction site in this workspace AND in `ft991a`/`ts570d` (read-only,
not edited) — all use `SerialConfig { ..., ..SerialConfig::default() }`
functional-update syntax, never an exhaustive struct literal, so adding new
`Default`-backed fields cannot break any existing caller's compile.

### Known test-infrastructure limitation (flagged before writing tests, not
### silently discovered after)

Verified empirically (Python `fcntl.ioctl` against a fresh `pty.openpty()`
pair on this dev machine) that `TIOCMGET`/`TIOCMBIS`/`TIOCMBIC` all return
`ENOTTY` on BOTH master and slave sides of a Linux PTY — this is not new
information (the existing `SerialPort::open` RTS/DTR block already
documents this exact failure mode as "harmless" for its own use), but it
means the existing `TestPtyPair` helper (built on `nix::pty` per this
file's Task 2 section) CANNOT exercise the success path of any
`ModemControlLines` method — every call against a PTY-backed `SerialPort`
in the test suite deterministically returns
`Err(TransportError::Io(ENOTTY))`. Per this crate's "if it feels like scope
creep... STOP and report" guidance, and per the top-level task's explicit
"if the existing test PTY helper doesn't support ioctl the way you need...
STOP and report rather than improvising something that might be wrong":
NOT inventing new test infrastructure (e.g. mocking `libc::ioctl`, or a
fake character device) to fabricate a success path. Instead, tests use the
existing `TestPtyPair` to verify the REAL, production ioctl code path is
exercised correctly and fails predictably/non-panically
(`Err(TransportError::Io(_))` with `ENOTTY`), for all five methods — this
is genuine coverage of the error-propagation plumbing, not a fabricated
pass. The success path (bit actually toggles on real hardware) is not
verifiable in this environment and is reported as a gap, not silently
glossed over.

### Done when

`cargo test -p cat-transport-core -p cat-transport-serial` green, `cargo
clippy -p cat-transport-core -p cat-transport-serial --all-targets -- -D
warnings` clean, `cargo fmt --all -- --check` clean, `cargo test --workspace`
(all 7 crates) still green. `cat-framework`/`cat-client`/
`cat-transport-tcp`/`cat-transport-udp`/`cat-server` untouched. `ts570d`/
`ft991a` untouched. Not committed.

### Status: implementation complete, awaiting architect/user review

All acceptance checks green — full detail in `progress.md`'s Task 7
section, including the one pre-existing/unrelated clippy fix
(`write_to_master` dead code) and the PTY test-infrastructure limitation
(ENOTTY on all `TIOCM*` ioctls) flagged there rather than worked around.
Not committed, per standing rule. STOPPING here per the one-task-at-a-time
workflow.

## ADR 0004 dispatch queue, Task 6 (`planning/architect/task_plan.md`
## "### Task 6" — search that heading; NOT the same "Task 6" as this file's
## own section above, which was a separate, earlier, already-completed
## coordinator-direct task with a colliding label) — extract shared
## `SerialConfig`/`Parity`/`FlowControl` into `config.rs`; add the portable
## `oneshot.rs` completion primitive (2026-07-19)

Authorized by `docs/adr/0004-windows-serial-backend.md` (read in full) and
its dispatch queue in `planning/architect/task_plan.md`'s `### Task 6`
section (read in full, verbatim spec). Two pieces of groundwork for the
Windows COM-port backend that Tasks 7/8 will add later — not part of this
task: (1) a behavior-preserving refactor moving `SerialConfig`/`Parity`/
`FlowControl` out of `io_uring.rs` into a new ungated `config.rs`, so a
future Windows module can share them without duplication; (2) a new
private, pure-`std` `oneshot.rs` completion primitive (`Completion<T>` /
`channel<T>() -> (CompletionTx<T>, CompletionRx<T>)`) per ADR 0004 §1,
with real executable unit tests, since it has zero OS dependency even
though its only future consumer is Windows-specific code.

### Plan

`config.rs` (new, ungated — pure data, no `#[cfg(...)]` needed): move
`SerialConfig`, `Parity`, `FlowControl`, and `SerialConfig`'s `Default` impl
out of `io_uring.rs` verbatim — same fields, same doc comments (with doc
links re-qualified to resolve from the new location, e.g. `[`SerialConfig`]`
→ `crate::SerialPort::open` since `SerialPort` isn't in scope there; prose
text unchanged), same `Default` impl. `io_uring.rs` gets `use
crate::config::{FlowControl, Parity, SerialConfig};` in place of the old
definitions — no other change to that file. `lib.rs`: add `pub mod config;`,
change the re-export line to `pub use config::{FlowControl, Parity,
SerialConfig};` (still re-exported from the crate root exactly as before, so
nothing downstream that imports `cat_transport_serial::SerialConfig` etc.
needs to change), `pub use io_uring::SerialPort;` unchanged in effect.

`oneshot.rs` (new, `mod oneshot;` — **not** `pub mod` — in `lib.rs`; not
part of the crate's public API): `enum Slot<T> { Empty, Value(T), Canceled
}` inside a private `struct Completion<T> { slot: Mutex<Slot<T>>, waker:
Mutex<Option<Waker>> }` (an `Option<T>`-shaped slot needs a third state
here, since "no value yet" and "sender dropped without sending" must be
distinguishable — plain `Mutex<Option<T>>` can't tell those apart, hence
`Slot` instead of a bare `Option<T>`, still a `Mutex<Option<T>>`-equivalent
shape per the task's phrasing). `CompletionTx<T>` / `CompletionRx<T>` each
wrap `Arc<Completion<T>>`; `channel<T>() -> (CompletionTx<T>,
CompletionRx<T>)` constructs the shared state and both handles.
`CompletionTx::send(self, value: T)` stores the value then wakes any
registered waker, consuming `self`. `CompletionTx`'s `Drop` impl checks: if
the slot is still `Slot::Empty` (i.e. `send` was never called), marks it
`Slot::Canceled` and wakes the registered waker — this is what resolves an
in-flight `.await` to `Err(Canceled)` instead of hanging forever when a
future Windows worker thread exits mid-request (ADR 0004 §1's explicit
motivating case). `CompletionRx<T>: Future<Output = Result<T, Canceled>>`
(`Canceled` a small marker struct, `Debug + Clone + Copy + PartialEq + Eq`);
`poll` locks the slot, and on `Slot::Empty` stores `cx.waker().clone()` and
returns `Pending`, on `Slot::Value`/`Slot::Canceled` returns the matching
`Ready`.

Judgment call: added `#![allow(dead_code)]` at the top of `oneshot.rs`,
with an explanatory doc comment. Reasoning: this task adds the primitive
standing alone — no production caller exists yet, since the Windows
worker-thread `SerialPort`/`Transport` implementation that would actually
construct a `channel()` and send/await through it is Task 7/8, explicitly
out of scope here. Without the allow, `cargo clippy -p cat-transport-serial
--all-targets -- -D warnings` fails on a Linux build (the only target this
sandbox can build for) because every item in the module is genuinely
unreferenced by non-test code today; the module's own `#[cfg(test)]` tests
exercise every path regardless of the allow. This is the same shape as the
pre-existing `#[allow(dead_code)]` on `io_uring.rs`'s `write_to_master` test
helper (Task 7 section above) — not a new pattern for this codebase.

Tests (`oneshot::tests`, all pure-`std`, no new dependency): (a)
`poll_before_send_returns_pending_and_registers_waker` — poll before any
send returns `Pending` and the waker doesn't fire yet; then `send` from the
*same* thread and confirm the previously-registered waker fires and a
follow-up poll resolves with the sent value. (b)
`cross_thread_send_after_delay_wakes_and_resolves_with_value` — a separate
`std::thread::spawn`, after a genuine `sleep(50ms)`, calls `send`; the main
thread uses a small test-only `block_on_with_timeout` helper (thread-park
loop with a `ThreadWaker`, the same shape ADR 0004 §1 describes for
`ft991a`'s eventual Windows `block_on`) bounded by a 5s deadline, so this
exercises a real cross-thread wake, not a same-thread immediately-ready
resolution. (c)
`dropping_sender_before_send_resolves_to_canceled_not_hang` — same
cross-thread-with-delay shape as (b), but the spawned thread `drop`s the
sender instead of calling `send`; resolves to `Err(Canceled)` within the
same bounded `block_on_with_timeout`, so a regression in the
cancellation-wake path fails this test instead of hanging the suite (the
task's explicit requirement). Plus one extra test,
`send_before_first_poll_resolves_immediately`, covering the ordering the
other three don't (`send` before any poll at all) — cheap to add, not
explicitly required by the task but closes an obvious gap.

### Before/after test count (behavior-preservation check, per the task's
### explicit requirement)

Ran `cargo test -p cat-transport-serial` **before** touching any file:
**18 passed**, all the same test names later re-verified after the change
(see `progress.md` for the full list). Ran it again **after** both the
`config.rs` extraction and adding `oneshot.rs`: **22 passed** — the same 18
original tests, all still passing, unchanged names, unchanged behavior,
plus exactly the 4 new `oneshot::tests::*` tests. Zero pass/fail diff on
the pre-existing 18; the refactor alone (config.rs move) was independently
confirmed behavior-neutral by running the suite again right after that step
and before writing `oneshot.rs` — see `progress.md` for the intermediate
count.

`cargo test --workspace`: also re-run before/after. Grepped the whole
workspace (`grep -rn "SerialConfig\|FlowControl\|Parity\b"`) for any
consumer of these types outside `cat-transport-serial` itself first — none
found (`cat-client`/`cat-server`/`cat-transport-tcp`/`cat-transport-udp`/
`cat-framework` don't reference them at all), so the re-export-path change
was a zero-risk, single-crate-scoped move confirmed by that grep, not just
assumed. Workspace total: 119 → 123 (13 `cat-client` + 8 `cat-framework` +
46 `cat-server` + 12 `cat-transport-core` + 18→22 `cat-transport-serial` +
7 `cat-transport-tcp` + 15 `cat-transport-udp`), every other crate's count
identical before/after.

### Done when

`cargo test -p cat-transport-serial` green (18 pre-existing + 4 new = 22,
confirmed above); `cargo clippy -p cat-transport-serial --all-targets -- -D
warnings` clean; `cargo fmt --check` clean (crate and `--all`); `cargo test
--workspace` green with only `cat-transport-serial`'s count changing.
`Cargo.toml` untouched (crate-level and workspace-level — confirmed via
`git status`/`git diff --stat`, only `io_uring.rs`, `lib.rs` modified plus
`config.rs`/`oneshot.rs` new). No Windows-specific `SerialPort`/`Transport`/
`ModemControlLines` code written (that's Task 7/8). `cat-transport-core`,
`cat-transport-tcp`, `cat-transport-udp`, `cat-server`, `cat-framework`,
`cat-client` untouched. `ts570d`/`ft991a` (sibling repos, not in this
workspace) untouched.

### Status: implementation complete, awaiting architect/user review

All acceptance checks green — full detail in `progress.md`'s matching
section. Not committed, per standing rule. STOPPING here per the
one-task-at-a-time workflow — Tasks 7/8 (Windows `SerialPort::open`/
`Transport`/`ModemControlLines` impls) are separate, later, not started.

## Task 7 (ADR 0004 dispatch queue, `planning/architect/task_plan.md`'s
## `### Task 7` section) — Windows `SerialPort::open`/`configure_dcb`/
## `SetCommTimeouts` — DONE

Scope, read and confirmed before writing code: `windows-sys` dependency +
`cat-transport-serial/src/windows.rs`'s open/configure path only — **not**
`Transport`/`ModemControlLines`/the worker thread (Task 8). Read in full
first, in the order specified: `planning/architect/task_plan.md`'s Task 7
section; `docs/adr/0004-windows-serial-backend.md` §3/§5 (and, for full
context, §1/§2/§4/Consequences); `cat-transport-serial/src/config.rs`
(Task 6's extraction); `io_uring.rs` (`SerialPort::open`'s structure,
`READ_TIMEOUT`, `baud_rate_from_u32`, SAFETY-comment style); `lib.rs`'s
current module structure.

### Plan (matches what was actually built — no deviation during
### implementation beyond the two judgment calls logged below)

1. `Cargo.toml`: add a `[target.'cfg(target_os = "windows")'.dependencies]`
   section for `windows-sys`, mirroring the existing Linux `monoio` section
   exactly.
2. Move `READ_TIMEOUT` (from `io_uring.rs`) and the baud-rate-validated-set
   (from `io_uring.rs`'s `baud_rate_from_u32`) into new small, shared,
   ungated modules — `timeouts.rs` and `baud.rs` — mirroring the `config.rs`
   extraction precedent from Task 6, since both platform backends need
   them and Task 7's own dispatch text explicitly invites this ("similar to
   what Task 6 did for `SerialConfig`").
3. `lib.rs`: gate the Linux module/re-export, add the Windows module
   declaration.
4. Write `windows.rs`: `RawHandle` newtype + `unsafe impl Send`,
   `SerialPort::open`, `configure_dcb`, `set_comm_timeouts`,
   `SerialPort::path`.
5. Verify: `cargo check --target x86_64-pc-windows-gnu -p
   cat-transport-serial` (install the target first if needed); confirm
   Linux `cargo test -p cat-transport-serial` / `cargo build --workspace`
   are completely unaffected; `cargo clippy`/`cargo fmt` for whatever
   compiles on each target.

### Key finding that reshaped step 1-3 before any code was written: the
### module gating this task was told to "mirror" did not actually exist yet

The task brief (and ADR 0004 §2's own prose, "`cat-transport-serial` gains
a `#[cfg(target_os = "windows")] mod windows;` alongside the existing
`#[cfg(target_os = "linux")] mod io_uring;`") both describe `mod io_uring;`
as already being Linux-gated post-Task-6. Reading the actual post-Task-6
`lib.rs` showed this is not true: `pub mod io_uring;` and `pub use
io_uring::SerialPort;` were (and, until this task, remained) **unconditional
in the file text** — Task 6 left them "implicitly Linux-only" only because
nothing in this sandbox builds for another target, a fact Task 8's own
dispatch text later confirms in passing ("today only the module is
implicitly Linux-only"). Confirmed empirically, not just by reading: ran
`cargo check --target x86_64-pc-windows-gnu -p cat-transport-serial`
*before writing any of my own code* (right after installing the target) —
it failed with 19 errors, all from `io_uring.rs` referencing Linux-only
`libc` symbols (`TIOCMBIS`, `O_NONBLOCK`, `tcflush`, ...) that don't exist
in the `libc` crate's Windows surface, and would additionally have failed
on `monoio` not resolving at all (target-gated out already). This is a
blocking prerequisite for Task 7's own literal "Done when" bar — no amount
of correct code in a new `windows.rs` makes `cargo check --target
x86_64-pc-windows-gnu -p cat-transport-serial` succeed while `io_uring.rs`
is unconditionally compiled into every target's build.

Judgment call, not a design change: added `#[cfg(target_os = "linux")]` to
`pub mod io_uring;` and to `pub use io_uring::SerialPort;` in `lib.rs`,
making explicit what the ADR's own prose already assumed to be true. This
is squarely Task 7's own "mirror-image Windows gating" instruction, not a
reach into Task 8's scope — Task 8's stated lib.rs work is specifically
*adding the Windows-side* `#[cfg(target_os = "windows")] pub mod windows;
pub use windows::SerialPort;` pair once `Transport`/`ModemControlLines` are
implemented; it says nothing about gating the *Linux* re-export, and
correctly assumes (per its own "today only the module is implicitly
Linux-only" phrasing) that this gating either already exists or is trivial
housekeeping, not new design. Not gating it would leave Task 7's own
Done-when bar structurally unreachable regardless of `windows.rs`'s
content — flagged here per this crate's "if the task prompt contradicts
the planning files, surface the conflict" rule, then resolved as a
mechanical fix (not routed around, not silently skipped) since re-reading
confirmed no actual design decision hinges on which task's diff physically
contains this one `#[cfg]` line.

### `windows-sys` version — confirmed against the real crates.io index, not
### merely used on faith

`cargo info windows-sys` (works in this sandbox — network access to
crates.io's index confirmed available) showed latest `0.61.2`; `0.59` was
not separately confirmed to exist by that query alone. Definitive
confirmation came from actually adding `windows-sys = "0.59"` to
`Cargo.toml` and running the real acceptance command: `cargo check
--target x86_64-pc-windows-gnu -p cat-transport-serial` resolved and
downloaded `windows-sys v0.59.0` from crates.io without error ("Adding
windows-sys v0.59.0 (available: v0.61.2)") — so `0.59` is a real, current,
resolvable version, empirically confirmed, not assumed.

### `windows-sys` API investigation — used locally-vendored 0.52.0 source
### as ground truth before writing FFI code, then caught a real 0.52→0.59
### breaking change via the actual compiler

Before writing `windows.rs`, grepped the locally-cached `windows-sys`
0.52.0 and 0.48.0 sources
(`~/.cargo/registry/src/.../windows-sys-0.52.0/src/Windows/Win32/...`) for
the exact generated signatures of every Win32 function/type this task
needs (`CreateFileW`, `GetCommState`/`SetCommState`, `SetCommTimeouts`,
`DCB`, `COMMTIMEOUTS`, the `DCB_PARITY`/`DCB_STOP_BITS`/
`ESCAPE_COMM_FUNCTION`/`PURGE_COMM_FLAGS`/`MODEM_STATUS_FLAGS` constants)
rather than writing FFI calls from memory/guesswork. Two findings from
that investigation materially shaped the implementation:

1. **`windows-sys` exposes `DCB`'s packed boolean/2-bit sub-fields
   (`fBinary`, `fParity`, `fOutxCtsFlow`, `fRtsControl`, `fDtrControl`,
   ...) as one opaque `_bitfield: u32`, with no generated named
   accessors** (confirmed by grepping the whole crate for `fParity`/
   `set_fParity`-style methods — none exist anywhere, unlike some other
   Windows structs which do get bitfield accessor methods generated).
   `windows.rs`'s private `dcb_bits` submodule hand-rolls the documented
   `winbase.h` bit layout (bit positions/widths, `DTR_CONTROL_ENABLE`/
   `RTS_CONTROL_ENABLE`/`RTS_CONTROL_HANDSHAKE` values) since these aren't
   real linkable API surface, only documented bit *meanings* — not a
   deviation from anything specified, just a necessary implementation
   detail the ADR's field-mapping table (correctly) doesn't need to spell
   out at the bit-layout level.
2. **`CreateFileW`'s own generated binding requires the `Win32_Security`
   feature**, not just `Win32_Foundation`/`Win32_Storage_FileSystem` as
   literally listed in the task brief — confirmed twice: (a) by grepping
   the vendored source directly (`#[cfg(all(feature = "Win32_Foundation",
   feature = "Win32_Security"))]` immediately precedes the generated
   `CreateFileW` binding, because its `lpSecurityAttributes` parameter's
   type, `SECURITY_ATTRIBUTES`, is itself defined under `Win32_Security`);
   (b) empirically, by temporarily removing `Win32_Security` from
   `Cargo.toml` after the implementation was otherwise complete and
   re-running `cargo check --target x86_64-pc-windows-gnu -p
   cat-transport-serial` — it failed with `error[E0432]: unresolved import
   ... no CreateFileW in Win32::Storage::FileSystem`, then adding the
   feature back made it pass again. **Judgment call, not a design
   change**: added `Win32_Security` to the feature list. Without it,
   `CreateFileW` does not exist as a callable item *at all*, regardless of
   how it's invoked — the task's literal three-feature list is
   insufficient for the exact function the task also explicitly names
   (`CreateFileW`) to compile. This is windows-sys's own binding
   structure, unrelated in every way to the deliberately-excluded
   `Win32_System_IO`/`OVERLAPPED` feature (which is about a genuinely
   different, correctly-avoided design axis — overlapped I/O — not about
   `CreateFileW` existing at all).

### A real 0.52→0.59 breaking change, caught by the compiler, not missed

`windows-sys`'s `HANDLE` type changed between the two locally-vendored
versions and 0.59 (the version actually resolved): `0.52.0`/`0.48.0`
define `pub type HANDLE = isize;`; `0.59.0` (confirmed by re-grepping
after `cargo check` downloaded and vendored it into the local registry
cache) defines `pub type HANDLE = *mut core::ffi::c_void;`. This was
**caught by the compiler, not missed**: the first `cargo check --target
x86_64-pc-windows-gnu -p cat-transport-serial` run against the actually
-implemented `windows.rs` failed with `error[E0308]: mismatched types ...
expected *mut c_void, found usize` on `CreateFileW`'s `hTemplateFile`
argument (a literal `0`), pointing directly at the one call site relying on
`HANDLE` being an integer type. Fixed by passing `std::ptr::null_mut()`
instead. This incidentally makes `RawHandle`'s `unsafe impl Send`
*genuinely* load-bearing for the compiler on the version actually in use —
`*mut c_void` is `!Send` by default (unlike `isize`, which was already
`Send` on its own), so the manual `unsafe impl Send` this task's brief
asked for is not just documentation here, it's the only thing that will
let a Task-8-era worker thread actually take ownership of a `SerialPort`.

### Baud-rate validation / `READ_TIMEOUT` sharing — resolved by extraction,
### not duplication

Per the task's explicit invitation to use judgment here: created
`cat-transport-serial/src/baud.rs` (the shared, platform-neutral
`SUPPORTED_BAUD_RATES` list + `validate_baud_rate`, the single source of
truth for "which `u32` values this crate accepts") and
`cat-transport-serial/src/timeouts.rs` (the shared `READ_TIMEOUT`, moved
verbatim — same value, same `#[cfg(test)]`/`#[cfg(not(test))]` split, same
doc comment content — out of `io_uring.rs`). `io_uring.rs`'s
`baud_rate_from_u32` now calls `crate::baud::validate_baud_rate` first,
then maps the *already-validated* value to `nix::sys::termios::BaudRate`
(the Linux-only enum type that has no reason to exist in a platform-neutral
module) via the same explicit match it always had — so the "which values
are accepted" list has exactly one copy, not two that could drift, while
each platform's own value-mapping stays local to that platform's file.
`windows.rs`'s `configure_dcb` calls the same `validate_baud_rate` and
writes the validated `u32` straight into `DCB.BaudRate`. This is the same
shape as Task 6's `config.rs` extraction, applied to two smaller,
single-purpose pieces rather than one combined module — kept separate
(`baud.rs` vs. `timeouts.rs`) rather than merged into one "misc shared
stuff" file, matching this crate's existing one-concern-per-file
convention (`config.rs` alone, `oneshot.rs` alone).

### `DCB` field-by-field mapping — confirmed against ADR 0004 §5 exactly,
### plus two fields the table is silent on

Every row of ADR 0004 §5's table is implemented in `configure_dcb` exactly
as specified — see `progress.md`'s Task 7 section for the full field-by-
field confirmation. Two `DCB` bitfield members the table doesn't name are
set as a judgment call, documented inline in `windows.rs` and here:

- `fBinary = 1`, unconditionally: the Windows analog of `configure_termios`'s
  unconditional `cfmakeraw()` call. Not a `SerialConfig` field on either
  platform (Linux's raw-mode setup isn't driven by config fields either),
  and Win32 documentation states non-binary mode isn't supported for
  serial communication at all.
- `fDtrControl = DTR_CONTROL_ENABLE`, unconditionally: ADR 0004 §5's table
  has no dedicated fDtrControl row (its `initial_dtr` row is about Task 8's
  post-open `EscapeCommFunction` calls, not configure-time DCB state).
  Set to manual/software control so those later `EscapeCommFunction(
  SETDTR/CLRDTR)` calls actually take effect (`DISABLE` would keep the line
  low regardless; `HANDSHAKE` would let the driver override manual calls)
  — mirroring Linux, where DTR is never touched by `configure_termios`/
  `CRTSCTS` at all and is purely `ioctl`-controlled post-open.

Also set (all `0`/off, no `SerialConfig` field or Linux-termios equivalent
drives them, conservative/no-special-processing values matching
`cfmakeraw`'s general intent): `fOutxDsrFlow`, `fDsrSensitivity`,
`fTXContinueOnXoff`, `fErrorChar`, `fNull`, `fAbortOnError`.

### `WriteTotalTimeoutConstant` choice

5000ms (5s), matching ADR 0004 §3's own suggested starting point verbatim
("a generous bound (e.g. 5s) ... not a hard requirement"). No real Windows
hardware is reachable from this sandbox to empirically tune this value; 5s
is generous relative to typical CAT command-response latency (< 100ms,
same figure `crate::timeouts`' own doc comment already uses for
`READ_TIMEOUT`'s justification) while still failing a genuinely
hung/removed device in bounded time.

### Verification — the unusual bar this task specified, executed exactly

`rustup target list --installed` showed only `x86_64-unknown-linux-gnu`.
`rustup target add x86_64-pc-windows-gnu` succeeded (network-available
sandbox). `cargo check --target x86_64-pc-windows-gnu -p
cat-transport-serial` — after both real bugs above were fixed (the
`io_uring.rs` gating prerequisite and the `HANDLE`-type/`Win32_Security`
issues) — **compiles cleanly**: `Finished \`dev\` profile [unoptimized +
debuginfo] target(s) in 0.10s`. This is a genuine `cargo check`-only
cross-compile type-check, no Windows toolchain/linker was needed or
invoked (confirmed: `cargo check` does not reach the link step). Full
before/after Linux verification and the one known residual `--all-targets`
caveat are in `progress.md`'s Task 7 section.

### Status: implementation complete, all acceptance checks green, awaiting
### architect/user review

Not committed, per standing rule. STOPPING here per the one-task-at-a-time
workflow — Task 8 (Windows `Transport`/`ModemControlLines`/worker thread)
is a separate, later task, not started, not authorized by this task.

## Task 8 (ADR 0004 dispatch queue, `planning/architect/task_plan.md`'s
## `### Task 8` section) — Windows `Transport`/`ModemControlLines` + worker
## thread (2026-07-19)

Authorized by the coordinating session, dispatching the architect's Task 8
verbatim spec, depends on Task 6 (`oneshot.rs`) and Task 7 (`SerialPort::
open`/`configure_dcb`/`set_comm_timeouts`), both done. Completes
`cat-transport-serial/src/windows.rs`'s `SerialPort`: the worker-thread
request/reply design ADR 0004 §1 specifies, `impl Transport for SerialPort`,
`impl ModemControlLines for SerialPort`, and the `lib.rs` re-export gating.
Also fixes a flagged residual issue from Task 7: `session.rs`'s
`#[monoio::test]`-based test module is ungated and breaks
`--target x86_64-pc-windows-gnu --all-targets` (monoio isn't a dependency
on that target at all).

Read in full first, in the order the dispatch specified: ADR 0004 §1/§3/§4;
this file's own Task 7/8 sections (verbatim spec); `oneshot.rs` (Task 6's
`CompletionTx<T>`/`CompletionRx<T>`/`channel<T>()` API); `windows.rs` (Task
7's current `SerialPort`/`RawHandle`/`configure_dcb`/`set_comm_timeouts`);
`cat-transport-core/src/modem.rs` and `.../transport.rs` (exact trait
signatures); `io_uring.rs`'s `impl Transport`/`impl ModemControlLines` (the
behavioral contract to match, not the mechanism).

### Plan

1. **Worker-thread request enum**, per ADR §1 and the architect's Task 8
   text verbatim:
   ```rust
   enum WorkerRequest {
       Read {
           len: usize,
           reply: oneshot::CompletionTx<Result<Vec<u8>, TransportError>>,
       },
       Write {
           data: Vec<u8>,
           reply: oneshot::CompletionTx<Result<usize, TransportError>>,
       },
   }
   ```
   `SerialPort::open` spawns one `std::thread::spawn` after `configure_dcb`/
   `set_comm_timeouts` succeed, moving the `RawHandle` and an
   `mpsc::Receiver<WorkerRequest>` into it; loops `recv()`, performs
   blocking `ReadFile`/`WriteFile` via `windows-sys`, replies via
   `reply.send(...)`, exits its loop (ending the thread) when `recv()`
   returns `Err` (sender dropped). `SerialPort` gains `request_tx:
   mpsc::Sender<WorkerRequest>` and `worker: Option<JoinHandle<()>>` fields.
2. **`impl Transport for SerialPort`**: `write`/`read` build a
   `channel::<...>()` pair, send a `WorkerRequest` down `request_tx`,
   `.await` the `CompletionRx`, mapping `Err(oneshot::Canceled)` (worker
   thread gone/panicked) to `Err(TransportError::Io(...))` (an
   `ErrorKind::BrokenPipe`-shaped synthetic `io::Error`, since
   `TransportError` has no dedicated "worker gone" variant and `Io` is the
   established catch-all for "something went wrong at the OS/transport
   boundary" per `SerialError`'s own precedent). Worker's `ReadFile`
   returning `Ok(0)` maps to `Err(TransportError::ReadTimeout)` *inside the
   worker thread*, before it ever replies — so `Transport::read`'s contract
   (`Ok(n)` always `> 0`) holds either way, matching `io_uring.rs` exactly.
   `flush_rx`: direct synchronous `PurgeComm(handle, PURGE_RXCLEAR)`, not
   via the worker. `flush`: direct synchronous `FlushFileBuffers(handle)`
   inside the `async fn` body, mirroring `io_uring.rs`'s `tcdrain`
   exception precisely (same justifying comment shape).
3. **`impl ModemControlLines for SerialPort`**: direct synchronous
   `EscapeCommFunction`/`GetCommModemStatus` on the calling thread against
   the same `HANDLE`, never touching the worker thread or a
   `CompletionTx`/`CompletionRx` — mirrors `io_uring.rs`'s ioctl-based impl
   exactly, same method-to-primitive mapping shape.
4. **`Drop for SerialPort`** (Windows): drop `request_tx` (by replacing the
   field's `Sender` is not possible without `Option`, so `SerialPort` holds
   `request_tx: mpsc::Sender<WorkerRequest>` directly and `Drop` is
   implemented manually, using `std::mem::take`/field extraction as needed
   to move the sender and `JoinHandle` out of `&mut self` inside `drop`),
   join the worker thread, then `CloseHandle(handle)`.
5. `SerialPort::open` calls `self.set_rts(true)`/`self.set_dtr(true)`
   post-open when `config.initial_rts`/`config.initial_dtr`, identical
   sequencing/error-ignoring to Linux.
6. `lib.rs`: add `#[cfg(target_os = "windows")] pub use windows::
   SerialPort;` alongside the existing (now both explicitly gated) Linux
   line.
7. **`session.rs` test-gating fix**: gate the entire `#[cfg(test)] mod
   tests` block with an additional `#[cfg(target_os = "linux")]` — every
   async test in that module uses `#[monoio::test(driver = "legacy")]`, and
   `monoio` is a Linux-only *target-gated* dependency (not present at all
   for a Windows target), so `--target x86_64-pc-windows-gnu --all-targets`
   fails to resolve the `monoio::test` macro regardless of `--lib`-only
   passing. `io_uring.rs` has no analogous in-file gate to mirror because
   the whole file is already `#[cfg(target_os = "linux")]`-gated at the
   `lib.rs` module-declaration level — `session.rs` is different because it
   is genuinely cross-platform (`SerialCatSession<T: Transport>` compiles
   and is useful on both platforms), so only its *test module*, not the
   file itself, needs gating.

### Judgment calls flagged before/while implementing (updated after
### writing the code — see below for what was actually done vs. planned)

- `Canceled` → `TransportError` mapping: no existing `TransportError`
  variant means "the worker thread is gone." Reusing `Io` with a synthetic
  `io::Error` (kind `BrokenPipe`, no raw OS error) was chosen over adding a
  new `TransportError` variant, since ADR 0004 explicitly says "No new
  `SerialError` variant is expected to be needed" for the open/configure
  path and the same minimal-surface-change spirit applies here — a `Canceled`
  really is an unexpected connection-lost condition, which `Io` already
  models for a std::io::Error underneath.

### Verification plan

Same unusual bar as Task 7: `cargo check --target x86_64-pc-windows-gnu -p
cat-transport-serial --all-targets` (this time `--all-targets`, the bar
Task 7's own flagged residual finding didn't clear) must succeed; `cargo
clippy --target x86_64-pc-windows-gnu -p cat-transport-serial --all-targets
-- -D warnings` clean if reasonably achievable. Linux: `cargo test -p
cat-transport-serial` — re-measured directly before touching any file:
**24 passed** (Task 6 left it at 22; Task 7 added no new Linux tests to
`io_uring.rs`/`session.rs`, so 22 does not match — re-ran to confirm rather
than trusting the stale planning-file figure, and 24 exactly matches the
coordinating session's dispatch text, which cited 24 correctly). `cargo
test --workspace` re-measured the same way: **125** (13 cat-client + 8
cat-framework + 46 cat-server + 12 cat-transport-core + 24
cat-transport-serial + 7 cat-transport-tcp + 15 cat-transport-udp),
matching the dispatch's cited 125 baseline exactly. `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`. No
`cargo test` for Windows-specific code in this sandbox — type-check only,
per ADR 0004's Consequences section and this crate's standing verification
bar for Windows work.

### Status: see `progress.md` for what was actually implemented and the
### full verification transcript.
