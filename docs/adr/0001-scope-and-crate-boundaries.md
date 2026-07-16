# 1. Scope and crate boundaries for the shared CAT library

Date: 2026-07-15

## Status

Accepted

## Context

`ts570d` ADR [0004](https://github.com/kf0uwv/ts570d/blob/main/docs/adr/0004-extraction-boundary.md)
("Extraction boundary for a shared CAT library") records that `ts570d`'s
`framework` crate is designed to move into this repository **as-is** once
extraction is warranted, so that "a future extraction is a move rather than a
redesign." `ts570d` ADR [0005](https://github.com/kf0uwv/ts570d/blob/main/docs/adr/0005-network-transport-readiness.md)
("Network transport and server/control mode readiness") extends that boundary
to cover network transports and server mode, and names the crate layout this
repository is expected to grow into once it holds code.

**No code has been extracted into this repository yet**, and none should be,
prematurely. The network-transport-and-server-modes addendum quoted inside
`ts570d` ADR 0005 is explicit that the target crates should not all be built
out until code size in `ts570d` justifies the split; `ts570d` ADR 0004 is
equally explicit that extraction itself was out of scope of the refactor that
produced these decisions. This ADR exists purely to record, in this
repository's own terms, the scope and boundaries that `ts570d` has already
worked out — so that when extraction does happen, it starts from an agreed
target rather than being redesigned in the moment. This mirrors `ts570d` ADR
0004's own framing: the goal is that extraction is "a move rather than a
redesign."

## Decision

### Scope: this repository owns seven crates

The crates named in `ts570d` ADR 0005 section 4 are this repository's target
scope. None of them exist as code here yet; this records what each is
responsible for once it does.

```text
cat-framework         generic CAT command engine
cat-client            generic client-side request/response mechanics
cat-transport-core    Transport / CatSession trait abstractions
cat-transport-serial  serial transport (io_uring on Linux today)
cat-transport-tcp     TCP transport
cat-transport-udp     UDP transport
cat-server            request broker / server mode
```

Responsibilities, reconstructed from the addendum's "Recommended crate
boundaries" (quoted inside `ts570d` ADR 0005 and
`docs/architecture/network-readiness.md`):

- **`cat-framework`** — the radio-independent generic CAT engine that
  `ts570d`'s `framework/src/cat.rs` already implements: the command table
  model (`CommandTable<C>`, `CommandDefinition<C>`, `CommandForm`,
  `CommandOperation`), syntactic parsing and structural validation
  (`CommandRequest<C>`, `ParameterValues`), the dispatch lifecycle
  (`CatFramework<R>`), response construction (`ResponseBuilder`,
  `CommandOutcome`), the delegation traits (`CatCommandCatalog`, `CatRadio`),
  and generic errors (`FrameworkError`). It is generic over a radio-defined
  `CommandId` and never contains a radio-specific command id, mode, frequency,
  state, or handler — this is the one non-negotiable boundary rule inherited
  from `ts570d` ADR 0001, and it applies unchanged here.
- **`cat-client`** — the generic client-side mechanics for sending a command
  and awaiting a disposition: validating a request against a
  `CommandTable<C>`, formatting outgoing command bytes, and interpreting a
  `CatSession`'s `ResponseDisposition`. This is the radio-independent core of
  what `ts570d`'s `radio::RadioClient<S: CatSession>` does today, minus the
  TS-570D-specific typed get/set methods that stay behind in each radio crate
  (`ts570d::Ts570d<S>`, and eventually `ft991a`'s own client type).
- **`cat-transport-core`** — the `Transport` trait (byte-level `read`/
  `write`/`flush`) and the `CatSession` trait that sits above it
  (request/response framing, returning a `ResponseDisposition`), plus
  in-memory test doubles (`MockCatSession` / `ScriptedCatSession`) and any
  shared framing helpers. This is the trait surface every concrete transport
  crate implements; it assumes neither a Unix file descriptor nor a
  persistent connection, so a future UDP session can implement it honestly.
- **`cat-transport-serial`** — `SerialCatSession<T: Transport>`, reproducing
  `ts570d`'s existing read-until-`;` framing over a real serial port
  (io_uring on Linux today; Windows COM support is future work behind the
  same trait).
- **`cat-transport-tcp`** — `TcpCatSession`, using **length-prefixed frames**
  rather than inheriting the serial byte-loop's semicolon scanning.
- **`cat-transport-udp`** — `UdpCatSession`, using an **envelope format**
  (request/session IDs) plus a **deduplication cache**, since UDP has no
  delivery or ordering guarantee and a session is not connection-oriented.
- **`cat-server`** — the request broker: client session management and
  ownership of the physical radio session, sitting above a radio crate's
  client type and a `CatSession` implementation, serializing access through a
  single ordered worker. It depends on a radio crate and a transport crate,
  never the reverse, and it must not leak into a radio's state machine (see
  "Dependency direction," below).

### Extraction status

No code has been extracted. This ADR is written ahead of extraction, exactly
as `ts570d` ADR 0004 was written ahead of any second radio, so that a future
extraction is **a move rather than a redesign** — `ts570d` ADR 0004's own
phrase, and the same standard this ADR holds itself to.

### Dependency direction

Two independent call graphs, matching `ts570d`'s control-mode and
(future) server-mode diagrams:

**Control mode:**

```text
control-mode UI ──▶ controller service ──▶ CatClientTransport / CatSession
                                                    │
                                    ┌───────────────┼───────────────┬────────┐
                                    ▼               ▼               ▼        ▼
                                 serial            TCP             UDP     mock
```

**Server mode:**

```text
TCP/UDP server transport ──▶ request broker ──▶ physical radio controller ──▶ serial transport
```

In both graphs, the generic engine (`cat-framework`) and the transport
abstraction (`cat-transport-core`) sit below everything and depend on nothing
in this repository; `cat-server` sits above a radio's controller client and a
transport, never the reverse; and no UI-facing or server-facing code names a
concrete transport type — that choice is made once, at the wiring layer, in
each consuming application (`ts570d`'s `src/main.rs` today).

### Known open item (carried forward from `ts570d` ADR 0005 section 5)

`ts570d`'s `framework::transport::Transport` and the new `CatSession` trait
are `#[async_trait(?Send)]`, and `framework/src/lib.rs` re-exports `monoio`
directly — the generic engine currently names one async runtime. `ts570d` ADR
0005 records this as a deviation "tracked but not fixed" in that refactor,
and explicitly assigns resolving it to whichever extraction produces
`cat-transport-core`: either keep the `monoio`/`async_trait(?Send)` binding
(accepting that `cat-transport-core`, and therefore every transport crate
depending on it, requires `monoio`), or adopt a runtime-agnostic
associated-future design (the addendum's suggested
`AsyncCatClientTransport` with a GAT `SendFuture`) before Windows support
(which cannot use io_uring) is added to `cat-transport-serial`. This ADR does
not resolve that question — it records that `cat-transport-core`'s extraction
is the point at which it must be decided, not before.

## Consequences

- This ADR does not itself move any code, create any crate, or run `cargo
  init`. It is a design record only, so the actual extraction work (whenever
  it happens) has an agreed target instead of being designed ad hoc.
- The seven-crate scope above is the **target**, not a requirement to build
  out all seven crates immediately once extraction begins. Per the addendum's
  own guidance, crates should be split out as code size and need justify it —
  e.g. `cat-transport-tcp` and `cat-transport-udp` need not exist as separate
  crates before either transport has an implementation to put in them.
- `cat-framework` remains radio-independent per `ts570d` ADR 0001; a second
  radio (`ft991a`) supplies its own `CommandId`, command table, state
  machine, and `CatRadio` implementation, exactly as `ts570d` ADR 0004
  describes for a hypothetical second radio today.
- The `monoio` binding open item is inherited, not resolved, by this ADR.
  Whoever extracts `cat-transport-core` must make and record that call before
  Windows support is undertaken.
- This ADR will need revision if `ts570d`'s own refactor changes the shape of
  `CatSession`, `Transport`, or the addendum's recommended crate boundaries
  before extraction happens; `ts570d`'s ADRs remain the primary source of
  truth until this repository has its own code to diverge from them.
