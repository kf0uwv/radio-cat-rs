# 2. Async runtime binding for cat-transport-core and dependents

Date: 2026-07-16

## Status

Accepted

## Context

[ADR 0001](0001-scope-and-crate-boundaries.md) recorded, as a known open item
carried forward from `ts570d` ADR 0005 section 5, that `ts570d`'s
`framework::transport::Transport` and `framework::session::CatSession` are
`#[async_trait(?Send)]` and that `framework/src/lib.rs` re-exports `monoio`
directly — and explicitly assigned resolving this to "whichever extraction
produces `cat-transport-core`," i.e. now, rather than resolving it in the
abstract ahead of time.

Reading `ts570d`'s actual current code (not just the ADR's paraphrase of it)
confirms the binding is real and pervasive, not incidental:

- `framework/src/transport.rs`'s `Transport` trait and
  `framework/src/session.rs`'s `CatSession` trait are both
  `#[async_trait(?Send)]`.
- Every test in `session.rs` and `test_support.rs` uses
  `#[monoio::test(driver = "legacy")]` — there is no runtime-agnostic test
  harness today.
- `framework/src/lib.rs` re-exports `monoio::RuntimeBuilder` and
  `monoio::io::{AsyncReadRent, AsyncWriteRent}` as part of the crate's public
  API, for the convenience of downstream crates (`serial`, `radio`, `ui`,
  `emulator`, `src/main.rs`) that would otherwise need their own direct
  `monoio` dependency declaration for common bits.
- The addendum quoted in `ts570d` ADR 0005 suggested an alternative:
  `AsyncCatClientTransport` with a GAT `SendFuture`, so the trait shape does
  not name any executor.

This ADR is the point ADR 0001 named for making that call. Two tasks are now
authorized in this repository: (A) extracting `ts570d`'s generic engine into
real crates, and (B) implementing TCP/UDP transports and a `cat-server`
request broker. Both are affected by this decision, since `cat-transport-tcp`,
`cat-transport-udp`, and `cat-server` do not exist as code yet and can be
built to either shape.

## Decision

**Retain the `monoio` / `#[async_trait(?Send)]` binding**, and extend it
uniformly to every crate that needs an async runtime in this repository:
`cat-transport-core`, `cat-transport-serial`, `cat-transport-tcp`,
`cat-transport-udp`, and `cat-server`. Do **not** adopt the runtime-agnostic
associated-future (`AsyncCatClientTransport` + GAT `SendFuture`) design at
this time.

`cat-framework` and `cat-client` are unaffected either way: `cat-framework`
has no `async` code at all today (`CatFramework::process_frame` is
synchronous), so it takes no runtime dependency regardless of this decision.
`cat-client` inherits `#[async_trait(?Send)]`'s shape transitively because its
methods call through `CatSession`, but does not need a direct `monoio`
dependency for production code — only `async-trait` — plus a `monoio`
dev-dependency for its own `#[monoio::test]`-based unit tests, mirroring
`ts570d`'s `radio` crate today.

### Why keep it, and why now is not the trigger point

1. **The stated trigger for revisiting hasn't fired.** `ts570d` ADR 0005
   framed this as a question to resolve "before Windows support is undertaken"
   for `cat-transport-serial`. Windows COM support is not part of either task
   authorized now (extraction, or TCP/UDP/server). Nothing currently on this
   repository's plate needs it.
2. **TCP and UDP are not escaping monoio's model.** `monoio` supports network
   sockets over io_uring on Linux (`monoio::net::{TcpStream, UdpSocket}`)
   exactly as it supports serial I/O. The premise that TCP/UDP "don't need
   io_uring the way serial does" is true in the sense that they *could* run
   under any executor, but it does not follow that they must escape monoio to
   be implemented well — there is a real, working, in-tree option that keeps
   one runtime end-to-end across control mode and server mode.
3. **Tokio remains banned project-wide** (`CLAUDE.md`, both repositories)
   unless a future ADR changes that. The main practical audience for a
   runtime-agnostic trait shape — a tokio-based consumer — does not exist and
   is not wanted. Building the abstraction now serves a hypothetical consumer,
   not a real one.
4. **The redesign cost is not small, and this repository has zero lines of
   code today.** A GAT/associated-future redesign touches `Transport`,
   `CatSession`, `SerialCatSession`, `ScriptedCatSession`, and every future
   `TcpCatSession`/`UdpCatSession`/broker-worker signature simultaneously.
   Paying that cost now, before a single concrete implementation exists, is
   speculative complexity — it inverts `ts570d` ADR 0004's own standard that
   "a future extraction is a move rather than a redesign."
5. **`cat-server`'s single-ordered-worker requirement (its own charter, see
   `.claude/agents/cat_server.md`) is naturally expressed with monoio's
   thread-per-core, `!Send`-future executor.** Introducing a second
   abstraction layer between the broker and its worker buys nothing until a
   second runtime consumer actually exists.
6. **Reversal later is bounded, not foreclosed.** `Transport` and `CatSession`
   are two small traits with few methods. If Windows serial support is ever
   scheduled, or a consuming application needs a non-monoio runtime, revisiting
   this decision touches the same trait surface it would touch today — just
   with `cat-transport-serial`/`-tcp`/`-udp`/`cat-server` as real, migratable
   implementations instead of zero. That is an acceptable place to pay this
   cost: when a concrete need exists, not unconditionally today.

### Explicit revisit trigger (so this is a decision, not a silent default)

This decision must be revisited — not silently carried forward again — if
either becomes true:

- Windows serial support (no io_uring) enters scope for `cat-transport-serial`.
- A consuming application (a future `ft991a` integration, or `ts570d` itself)
  needs an async runtime other than `monoio`.

Until then, this is settled.

## Consequences

- `cat-transport-core`, `cat-transport-serial`, `cat-transport-tcp`,
  `cat-transport-udp`, and `cat-server` all take a direct `monoio` dependency
  and use `#[async_trait(?Send)]` for their trait definitions, matching
  `ts570d`'s existing convention exactly (no wire- or API-visible behavior
  changes from what `ts570d` already does).
- **`monoio` is compiled in for Linux specifically**, matching `ts570d`'s own
  constraint (`serial/`'s io_uring implementation is already Linux-only).
  Each of the five crates above declares `monoio` as a
  `[target.'cfg(target_os = "linux")'.dependencies]` entry in its
  `Cargo.toml`, not a plain unconditional `[dependencies]` entry, so that
  `cargo metadata`/`cargo check` on a non-Linux host fail fast and legibly
  (missing target-gated dependency) rather than compiling `monoio` and then
  failing deep inside its io_uring internals. This is a build-graph
  clarification of the decision above, not a new decision: the project has
  never claimed non-Linux support, so this makes the existing constraint
  explicit in `Cargo.toml` rather than leaving it implicit in "works on my
  Linux machine."
- The `pub use monoio::{RuntimeBuilder, io::{AsyncReadRent, AsyncWriteRent}}`
  convenience re-export that lived in `ts570d`'s `framework/src/lib.rs` moves
  to `cat-transport-core`'s crate root in this repository, not
  `cat-framework` — `cat-framework` has no async code and should not
  re-export an executor it doesn't use. `cat-transport-core` is now the crate
  that owns the runtime binding, so it is the natural home for that
  convenience.
- `cat-client` depends on `async-trait` directly and `monoio` as a
  dev-dependency only (for test execution), never as a production dependency.
- This ADR resolves the open item [ADR 0001](0001-scope-and-crate-boundaries.md)
  recorded under "Known open item"; see the amendment note added there
  pointing at this ADR.
- Nothing in this ADR moves code or writes a `Cargo.toml` — it is a design
  record, same as ADR 0001. The dispatch queue in
  `planning/architect/task_plan.md` applies it.

## Amendment (2026-07-19): the revisit trigger fired

The first bullet under "Explicit revisit trigger" — "Windows serial support
(no io_uring) enters scope for `cat-transport-serial`" — has fired: the user
wants real Windows COM-port control from a native Windows build. Per this
ADR's own §6 ("Reversal later is bounded, not foreclosed ... an acceptable
place to pay this cost: when a concrete need exists, not unconditionally
today"), that cost is now paid, recorded as its own new decision rather than
a rewrite of this one: see
[ADR 0004](0004-windows-serial-backend.md). The decision made there is
consistent with this ADR's Linux/`monoio` binding, not a reversal of it:
`monoio`/io_uring stays exactly as decided above for Linux; Windows gets a
genuinely separate backend inside the same crate, gated by
`#[cfg(target_os = "windows")]`, not a runtime-agnostic redesign of
`Transport`/`CatSession` touching the Linux path.
