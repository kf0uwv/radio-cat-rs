# 6. Windows network transport (`cat-transport-tcp`/`cat-transport-udp`/`cat-server`), the shared RS-232 pin-test tool, and `NoModemControlLines`

Date: 2026-07-26

## Status

Accepted

## Context

[ADR 0002](0002-async-runtime-binding-for-transport-crates.md) named an
explicit second revisit trigger, distinct from the Windows-serial one ADR
0004 already resolved: **"a consuming application (a future `ft991a`
integration, or `ts570d` itself) needs an async runtime other than
`monoio`."** That trigger has now fired for real: both sibling applications
are adding features (server mode, TCP client mode, a shared diagnostics
screen) that require `cat-transport-tcp`, `cat-transport-udp`, and
`cat-server` to build and run on Windows, exactly the way ADR 0004 already
did for `cat-transport-serial`. This ADR is that second resolution.

### What was read before deciding

- `cat-transport-tcp/src/{lib,session}.rs` — `TcpCatSession` wraps
  `monoio::net::TcpStream` **directly** (not generic over `Transport`, unlike
  serial's `SerialCatSession<T: Transport>`) and implements its own
  length-prefixed framing (`write_frame`/`read_frame`/`read_frame_or_eof`,
  `MAX_FRAME_SIZE` = 64 KiB, `TcpSessionError`). No radio-specific knowledge.
- `cat-transport-udp/src/{lib,session}.rs` — `UdpCatSession` wraps
  `monoio::net::udp::UdpSocket` directly, with an envelope format
  (`session_id`/`request_id`, 8 bytes each BE, no length field), a
  client-side dedup cache (membership-only, `request_id`-keyed), and a
  response timeout via `monoio::time::timeout_at`. `encode_envelope`/
  `decode_envelope` were already pure functions with no `monoio` dependency.
- `cat-server/src/{lib,broker,broker_session,local_channel,registry,tcp,udp,
  test_fixtures}.rs` — the request broker. `Broker<C,S>::dispatch` is the
  *only* place in `broker.rs`/`broker_session.rs`/`local_channel.rs`/
  `registry.rs` that touches `monoio` (one call, wrapping the request in a
  timeout). `local_channel` (the hand-rolled `Rc`/`RefCell`-based
  many-producer/single-consumer queue and oneshot reply slot the broker's
  worker is built on) and `registry::ClientRegistry` are both already pure
  `std`, not `monoio`-dependent at all. `tcp.rs`/`udp.rs` (the server-side
  accept/dispatch loops) are the other genuinely `monoio`-dependent files:
  `monoio::net::{TcpListener, TcpStream}`/`monoio::net::udp::UdpSocket` +
  `monoio::spawn` per connection/datagram. `.claude/agents/cat_server.md`'s
  charter: one physical radio session, shared by many concurrent remote
  clients, through a single ordered worker — structurally different from
  serial/TCP/UDP's single point-to-point session, so this ADR does not copy
  ADR 0004's shape onto `cat-server` blindly (see Decision §3).
- `cat-transport-serial/src/{oneshot,windows,session,lib,config,timeouts}.rs`
  — ADR 0004's precedent in full: a dedicated background `std::thread` +
  `oneshot.rs`'s hand-rolled single-slot completion primitive bridging a
  blocking worker thread to an `async fn` caller; `SerialConfig`/`Parity`/
  `FlowControl` extracted into an ungated `config.rs` so Linux and Windows
  backends share one definition of the pure data instead of duplicating it.
- `ft991a/src/main.rs` (Deliverable 4) — `TcpClientSession`, a bespoke
  `CatSession` adapter around `TcpCatSession` with an honest-error
  `ModemControlLines` impl (five near-identical `Err(...)` bodies) so
  `Ft991a<TcpClientSession>` satisfies `ui::run`'s unconditional `CwKeying:
  ModemControlLines` bound despite TCP having no RTS/DTR concept.
- `ts570d/src/bin/pin_test.rs` (Deliverable 3) — a standalone RS-232
  null-modem cable pin tester, entirely hand-rolled against raw `libc`/
  `ioctl` calls on two file descriptors (`TIOCMBIS`/`TIOCMBIC`/`TIOCMGET`,
  raw `termios`) — predates this workspace's `SerialPort`/`ModemControlLines`
  abstractions and duplicates logic those types already generalize. Seven
  checks: TXD→RXD byte loopback both directions; RTS→CTS both directions
  (hard failure); DTR→DSR both directions and DTR→DCD one direction
  (warn-only, since the TS-570D's own cabling convention does not require
  those lines).

## Decision

### 1. TCP/UDP sockets need no `windows-sys`/FFI at all — unlike serial

`std::net::{TcpStream, TcpListener, UdpSocket}` are natively cross-platform.
Serial's problem (ADR 0004) was `monoio` itself being entirely absent from
Windows (io_uring is a Linux kernel interface); TCP/UDP's problem is
narrower — only the **executor** is missing, not the I/O primitive. So the
Windows backends below need no `windows-sys` dependency at all: plain
`std::net` + `std::thread` + the completion primitive.

### 2. The completion primitive moves to `cat-transport-core::completion`, shared by every Windows backend

`cat-transport-serial::oneshot` (private, single-slot `CompletionTx`/
`CompletionRx` pair) is exactly the primitive every Windows transport
backend below needs, unchanged. It moves to `cat_transport_core::completion`
(`pub`), and `cat-transport-serial` now does `use cat_transport_core::
completion as oneshot;` instead of keeping its own copy — no behavior
change, its existing test suite passes unchanged. This is the literal "put
the common thing in radio-cat-rs" consolidation this task named explicitly.

### 3. `cat-transport-tcp`/`cat-transport-udp`: extract pure codec logic, add a worker-thread Windows backend, same public types

Both crates gain the same three-module shape:

- **`codec.rs`** (new, ungated): the pure encode/decode/error/constant logic
  extracted from `session.rs` — `TcpSessionError`/`MAX_FRAME_SIZE`/
  `encode_frame`/`check_frame_len`/`decode_len_prefix` for TCP;
  `UdpSessionError`/`ENVELOPE_HEADER_LEN`/`MAX_PAYLOAD_SIZE`/
  `DEDUP_CACHE_CAPACITY`/`DEFAULT_RESPONSE_TIMEOUT`/`encode_envelope`/
  `decode_envelope`/the client-side `RequestIdCache` for UDP. Mirrors ADR
  0004 §2's extraction of `SerialConfig`/`Parity`/`FlowControl` into
  `config.rs` exactly: pure data/logic with no platform-specific code
  belongs in one ungated place, not duplicated per backend.
- **`session.rs`** (existing, now `#[cfg(target_os = "linux")]`-gated):
  unchanged `monoio`-based `TcpCatSession`/`UdpCatSession`, now built on top
  of `codec.rs` instead of owning the logic itself.
- **`windows.rs`** (new): the same public `TcpCatSession`/`UdpCatSession`
  type, built on a dedicated background `std::thread` doing blocking
  `std::net` I/O + `cat_transport_core::completion` for the `async fn`
  boundary — ADR 0004 §1's exact shape, extended from serial's `Transport`
  read/write primitives to TCP/UDP's own whole-exchange framing: since
  neither `TcpCatSession` nor `UdpCatSession` is generic over `Transport`
  (they own their framing directly), the Windows worker thread does one
  complete request/response exchange per message, not separate read/write
  round-trips through the channel. UDP's response-timeout wait (`monoio::
  time::timeout_at` on Linux) becomes `UdpSocket::set_read_timeout` plus a
  per-iteration remaining-time recomputation against one fixed deadline —
  preserving the Linux backend's "a noisy peer cannot extend the wait past
  `response_timeout`" property exactly, with no async timer machinery
  needed since the whole wait already runs inside a blocking thread.

**`windows.rs` is deliberately *not* `#[cfg(target_os = "windows")]`-gated.**
Everything in it is plain `std::net`/`std::thread`/`std::sync::mpsc` plus the
portable completion primitive — nothing is actually Windows-specific (unlike
serial's real Win32 FFI, which cannot run outside a Windows target at all).
So it compiles, and its tests run, on **every** platform this workspace
builds for, giving this backend real executable test coverage on ordinary
Linux CI rather than relying solely on `cargo check --target
x86_64-pc-windows-gnu`. Only `lib.rs`'s top-level `pub use` of
`TcpCatSession`/`UdpCatSession` is `cfg`-gated, to select `windows`'s type as
the crate's canonical one on Windows (where `session`'s `monoio`-based type
does not exist) while `session`'s type remains canonical on Linux. This is
a deliberate, load-bearing extension of ADR 0004's "same public name,
platform-gated internals" convention: it buys strictly more verification
than ADR 0004's original precedent could, for the parts of this repository
that happen not to need real platform FFI.

Verified: `cargo test -p cat-transport-tcp` 17 passed (was 12, all new
`windows` tests actually executed on Linux); `cargo test -p
cat-transport-udp` 28 passed (was 15, likewise). No new external
dependency for either crate.

### 4. `cat-server`: structurally different, so a different split — not a copy of §3

`cat-server`'s charter (one physical radio session shared by many concurrent
remote clients through a single ordered worker) is not "a session talking to
one peer," so ADR 0004's/§3's per-session worker-thread shape does not apply
directly. Decision, item by item:

- **`Broker<C,S>` (validation, dispatch, timeout-wrapping),
  `DispatchOutcome`, `DispatchError`, `outcome_to_wire`, `ClientRegistry`,
  and the newly-extracted `dedup::DedupCache`** (server-side, response-cache,
  moved out of `udp.rs` into its own ungated module for the same reason as
  §3's `codec.rs`) stay **fully shared, ungated** — no duplication at all.
- **The Job queue/reply channel and the listener concurrency substrate**
  are what actually differ, and are platform-specific:
  - **Linux** (`broker.rs`'s existing `Job`/`BrokerWorker`/`BrokerHandle`/
    `build`/`build_with_timeout`, `local_channel`, `tcp.rs`/`udp.rs`):
    unchanged. Cooperative `monoio::spawn`ed tasks, `Rc`/`RefCell` shared
    state — sound because every task sharing a `local_channel` runs on the
    same OS thread, which `monoio`'s thread-per-core model guarantees.
  - **Windows** (`worker_windows.rs`, `tcp_windows.rs`, `udp_windows.rs`,
    `block_on.rs`): genuine OS threads instead of cooperative tasks, since
    ADR 0002 forbids adding a second general-purpose async executor crate
    to get cooperative multitasking without `monoio`. `Job`/`BrokerWorker`/
    `BrokerHandle`/`build`/`build_with_timeout` are rebuilt on
    `std::sync::mpsc` (genuinely `Send`) + `cat_transport_core::completion`
    (also `Send`-capable, unlike `local_channel`'s `Rc`-based oneshot);
    `tcp_windows::serve`/`udp_windows::serve` spawn one dedicated
    `std::thread` per accepted connection / per received datagram (mirroring
    `tcp.rs`/`udp.rs`'s own "one task per connection/datagram" shape with
    threads standing in for cooperative tasks); `ClientRegistry`/
    `DedupCache` are shared via `Arc<Mutex<_>>` instead of `Rc<RefCell<_>>`
    (both types were already pure `std`, reused unmodified). `block_on.rs`
    is a minimal thread-parking executor (the same ~20-line shape ADR 0004
    §1 sketched for `ft991a`'s eventual Windows entry point, generalized for
    reuse inside this crate rather than left to each consuming application)
    driving `Broker::dispatch`/`BrokerHandle::submit` on each dedicated
    thread. `BrokerHandle::submit` keeps an **identical** `pub async fn`
    signature on both platforms — the one truly shared hot-path call a
    client-facing listener makes; `BrokerWorker::run` is `pub async fn` on
    Linux but a plain blocking `pub fn` on Windows (no concurrency to
    preserve cooperatively in a single-ordered-worker loop that already
    processes one job at a time), matching ADR 0004's own precedent that the
    top-level "how do you start this" bootstrapping is expected to differ
    per platform regardless (each consuming app already needs its own
    Windows entry point, since `#[monoio::main]` cannot exist there).

**A load-bearing correctness finding, not a style choice:** an earlier draft
of this ADR's implementation made `Broker::dispatch`'s internal timeout wrap
call a single portable combinator (`cat_transport_core::timeout`, see §5)
*unconditionally on every platform*, reasoning that a `Waker`-contract-correct
combinator is "correct under any conforming executor." **That reasoning is
wrong for `monoio` specifically.** `cat-server`'s own pre-existing
`#[monoio::test]`-based test suite (`broker::tests::
never_answered_request_times_out_instead_of_hanging` and
`worker_recovers_after_a_timeout_and_services_the_next_request`) hung
indefinitely once `with_request_timeout` stopped calling `monoio::time::
timeout` directly — `cat_transport_core::timeout`'s internal deadline timer
fires on a plain `std::thread::spawn`ed thread and calls the captured
`Waker` from that foreign OS thread, and `monoio`'s thread-per-core executor
does not reliably reschedule the polling task when woken this way. This was
caught by running this crate's own test suite, not by inspection. The fix:
`with_request_timeout` stays a `#[cfg(target_os = "linux")]`/`#[cfg(target_os
= "windows")]` split — `monoio::time::timeout` on Linux (exactly as before,
zero behavior change), the portable combinator on Windows — because the
*correct* timeout mechanism for `Broker::dispatch` is a property of which
executor is actually driving the calling task, and on real deployments of
either platform that always coincides with `target_os` (the Linux worker
always runs under a real `monoio` runtime; the Windows worker never does).
**Consequence:** `worker_windows`/`tcp_windows`/`udp_windows`'s test modules
— which drive `Broker::dispatch` via `block_on`, not `monoio` — are gated
`#[cfg(all(test, target_os = "windows"))]`, unlike their §3 transport-level
siblings (which never touch `Broker::dispatch` and so have no such
restriction, and are tested on every platform per §3). Two pre-existing
`cat-server` test modules (`local_channel`, `broker_session`) were also
gated `target_os = "linux"` for `cargo check --all-targets` consistency on
a Windows cross-compile — they already only ever used `#[monoio::test]`,
this ADR just makes that explicit the way `cat-transport-serial::session`'s
identical gate already does.

Verified: `cargo test -p cat-server` 57 passed (was 51). `cargo check
--target x86_64-pc-windows-gnu -p cat-server --all-targets`: clean.

### 5. `cat_transport_core::timeout`: a shared, portable timeout combinator

Promoted out of a first `cat-server`-local draft (see §4's finding above)
into `cat-transport-core::timeout` — a plain `std::future::Future` combinator
(`TimeoutFuture<F>`/`timeout(duration, fut)`/`Elapsed`) that races the
wrapped future against a lazily-spawned `std::thread::sleep` timer, waking
the same `Waker` either side provides. Pure `std`, no OS dependency, so it
gets real tests regardless of which crate's production code calls it
(mirroring `completion`'s identical precedent). Used by `cat-server`'s
Windows dispatch-timeout path (§4) and by `cat-diagnostics` (ADR 0007) to
bound each per-command probe against a session that may have no timeout of
its own by design (`cat-transport-tcp::TcpCatSession`). **Caveat, recorded
so it is never silently forgotten:** do not call this combinator from code
that is itself being polled by a real `monoio` runtime — see §4's finding.
It is correct under `cat-server::block_on` and under `std::thread`-based
executors generally; it is not correct under `monoio` specifically.

### 6. The RS-232 pin-test tool: a `[[bin]]` in `cat-transport-serial`, not `ts570d`

`ts570d/src/bin/pin_test.rs` has nothing to do with any radio's CAT protocol
— it only needs `SerialPort`/`Transport`/`ModemControlLines`, all of which
now live in `cat-transport-serial`/`cat-transport-core`. Rebuilt here as
`cat-transport-serial/src/bin/pin_test.rs`, same seven checks, generalized
from raw `libc`/`ioctl` calls onto the shared abstractions — with zero
platform-specific code in the file itself (`SerialPort`/`ModemControlLines`
are already identical on Linux/Windows per ADR 0004), so it is genuinely
cross-platform; only `main`'s entry point differs (`#[monoio::main]` on
Linux, a minimal thread-parking `block_on` on Windows, the same shape ADR
0004 §1 sketched and §4/`cat-server::block_on` above also uses).

**Shipped as a `[[bin]]` (`name = "pin-test"`), not a `[[example]]`.** Both
apps' packaging scripts want a real installable binary under
`target/release/` to `cp` (and rename, if desired — e.g. `rs232c-pintest`)
into a Debian/Windows package; an `[[example]]` is reachable only via
`cargo run --example` from within this repository's own checkout, which
does not serve a downstream consumer's packaging pipeline. Build it via
`cargo build --release -p cat-transport-serial --bin pin-test`, which works
against `cat-transport-serial` as a resolved git dependency, not only as a
workspace member — `cargo build -p <package>` selects by package ID across
the whole resolved dependency graph, not only workspace members.

Verified: `cargo build -p cat-transport-serial --bin pin-test` and `cargo
clippy` clean on Linux; `cargo check --target x86_64-pc-windows-gnu -p
cat-transport-serial --bins` clean.

### 7. `NoModemControlLines<S>`: a reusable adapter, in `cat-transport-core`

`ft991a/src/main.rs`'s `TcpClientSession` hand-writes an honest-error
`ModemControlLines` impl (five near-identical `Err(...)` bodies) so a
TCP-backed session satisfies a UI trait bound requiring `ModemControlLines`.
`ts570d` will need the identical shape once it adds its own TCP client mode.
Generalized as `cat_transport_core::NoModemControlLines<S>`
(`cat-transport-core/src/modem.rs`, alongside the trait it adapts):

```rust
pub struct NoModemControlLines<S> { pub session: S }

impl<S> NoModemControlLines<S> {
    pub fn new(session: S) -> Self;
}

// CatSession: transparent delegation, S::Error unchanged.
impl<S: CatSession> CatSession for NoModemControlLines<S> { type Error = S::Error; /* ... */ }

// ModemControlLines: every method returns Err(TransportError::Other(..)),
// naming which method failed. Unconditional — no bound on S at all.
impl<S> ModemControlLines for NoModemControlLines<S> { /* ... */ }
```

**Deliberately does not also solve "map `S::Error` to a different error
type"** (e.g. `cat-transport-tcp::TcpSessionError` to `TransportError`,
which `Ft991a<S>`'s own bound separately requires). That mapping is
orthogonal to modem-control lines, and — per `From`/`Into`'s orphan rules —
can only be written by a crate that can see both concrete error types,
which `cat-transport-core` deliberately never does for any transport crate.
An application composes its own small `CatSession<Error = TransportError>`
adapter first (mirroring `cat-server::broker_session::BrokerCatSession`'s
identical pattern, which `ft991a`'s `TcpClientSession` already does for
exactly this reason, independent of modem lines) and wraps the *result*:

```rust
// app wiring layer (ft991a's or ts570d's src/main.rs)
let tcp_session = cat_transport_tcp::TcpCatSession::connect(addr).await?;
let mapped = TcpClientSession::new(tcp_session); // existing app-local CatSession<Error = TransportError> adapter, unchanged
let radio = Ft991a::new(NoModemControlLines::new(mapped));
// radio: Ft991a<NoModemControlLines<TcpClientSession>> now satisfies both
// CatSession<Error = TransportError> and ModemControlLines.
```

This lets `ft991a` delete its own `impl ModemControlLines for
TcpClientSession` block and `tcp_modem_lines_unsupported` helper entirely,
keeping only the (unrelated, still-necessary) `CatSession` error-mapping
step. Verified with unit tests against `ScriptedCatSession` (delegation:
`execute`/`send`/`flush_rx`; every `ModemControlLines` method returns an
error naming itself) — `cargo test -p cat-transport-core`: 25 passed (was
20).

## Consequences

- No new external dependency anywhere in this ADR — `std::net`, `std::
  thread`, `std::sync::{mpsc, Arc, Mutex}`, and the moved `completion`
  primitive are sufficient for every Windows backend above. No
  `windows-sys` needed (unlike serial).
- `cat-transport-tcp`/`cat-transport-udp`/`cat-transport-serial`/
  `cat-server` all now build for `x86_64-pc-windows-gnu`
  (`cargo check --target x86_64-pc-windows-gnu -p cat-transport-tcp -p
  cat-transport-udp -p cat-server -p cat-transport-serial`, clean).
  `monoio` stays exactly as-is for Linux (`[target.'cfg(target_os =
  "linux")'.dependencies]`); no Linux behavior change; no regressions.
  Full workspace suite: `cargo test --workspace` — 187 passed (was 149
  before this ADR's work began), zero failures.
- `cat-transport-tcp`'s/`cat-transport-udp`'s Windows backends get real
  executable test coverage on Linux CI (§3); `cat-server`'s Windows backend
  is verified by `cargo check --target x86_64-pc-windows-gnu` type-checking
  only, matching ADR 0004's original precedent, for the specific,
  documented reason in §4 (the `monoio`/foreign-thread-wake incompatibility)
  — not a weaker verification standard chosen carelessly.
- `ft991a`/`ts570d` still need their own follow-on work (not authorized or
  dispatched by this ADR) to actually wire `NoModemControlLines`/the new
  Windows transports/`pin-test` into their own `src/main.rs`/packaging —
  this ADR builds and documents the reusable pieces only, per this task's
  own scope boundary.
- This ADR resolves [ADR 0002](0002-async-runtime-binding-for-transport-crates.md)'s
  second revisit trigger. `cat-framework`'s Windows story is unaffected (it
  has no async code and never depended on `monoio`).

## Amendment (2026-07-26): `cat-rigctl` closed the same gap

This ADR's original scope explicitly excluded `cat-rigctl` ("`cat-transport-
tcp`/`-udp`/`cat-server` are unaffected — network sockets are not the
trigger" did not extend to the newer `cat-rigctl` crate, which did not exist
when this ADR was first written and was built directly on `monoio::net`/
`monoio::spawn` with no Windows path). Both `ft991a` and `ts570d` discovered
this independently while wiring their own Windows server modes against this
ADR's work: `cat-rigctl` failed to compile at all for `x86_64-pc-windows-
gnu`, forcing `ts570d`'s `server` crate into a hand-rolled Windows fallback
that dropped `--rigctl-port` (the WSJT-X/Hamlib bridge) entirely on that
platform, while `ft991a` kept its whole headless server mode Linux-only
rather than ship an asymmetric feature set.

`cat-rigctl` now has a genuine Windows backend, following this ADR's own
pattern exactly: the radio-independent wire protocol (`dispatch`/
`dump_state`/line buffering) was extracted into a new, I/O-free
`cat-rigctl::protocol` module shared by both platforms; `cat-rigctl::rigctl`
(Linux, `monoio`-based, unchanged in behavior) and a new
`cat-rigctl::rigctl_windows` (`std::net` + genuine OS threads, reusing this
ADR's `cat_server::block_on` — made `pub` for exactly this reuse — and
`cat_server::worker_windows::BrokerHandle`) sit on top of it; `cat_rigctl::
run` is now `#[cfg]`-selected the same way `cat-server`'s `build`/
`BrokerHandle` already were, with one difference from every other backend
in this ADR: **`rigctl_windows` is genuinely `#[cfg(target_os =
"windows")]`-gated**, not left ungated like `tcp_windows`/`udp_windows`,
because it constructs a `cat_server::BrokerCatSession` — a type hardcoded to
`cat_server`'s ambient, platform-aliased `BrokerHandle` rather than generic
over which handle it wraps — so a version built against the explicit,
genuinely-`Send` `worker_windows::BrokerHandle` cannot also satisfy
`BrokerCatSession::new` on a Linux build, where the ambient alias resolves
to the different, `Rc`-based `broker::BrokerHandle` instead. This is a real
type-level constraint inherited from `cat-server`'s existing design, not a
new inconsistency introduced here; see `rigctl_windows`'s own module doc for
the full explanation.

Verified: `cargo check --target x86_64-pc-windows-gnu -p cat-rigctl
--all-targets` clean (both lib and tests — `rigctl_windows`'s tests, unlike
`tcp_windows`'s/`udp_windows`'s, only exist under this target at all, so
there is no Linux-executed test for them, matching `cat-server::
worker_windows`'s own precedent). Full workspace suite on Linux: 196 passed,
zero failures (was 191 before this amendment). `ft991a`'s and `ts570d`'s own
`server` crates can now call `cat_rigctl::run` uniformly on both platforms
and regain full `--rigctl-port`/WSJT-X support on Windows — that wiring is
each app's own follow-on, not part of this amendment.
