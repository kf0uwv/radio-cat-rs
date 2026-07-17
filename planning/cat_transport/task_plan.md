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
