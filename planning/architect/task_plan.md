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
