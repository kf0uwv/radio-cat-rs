# radio-cat-rs

A shared, radio-independent CAT (Computer Aided Transceiver) protocol library
for Rust: a generic command engine, a set of transport implementations
(serial, TCP, UDP), and a request broker for running a physical radio as a
network-shared server. It is designed to be consumed by more than one
radio-control application — starting with
[`ts570d`](https://github.com/kf0uwv/ts570d) (Kenwood TS-570D) and, later, a
second client for the Yaesu FT-991A (`ft991a`) — without either application
needing radio-specific code duplicated into it.

## Status: pre-extraction, scaffolding only

**This repository currently contains no Rust code.** `ts570d`'s `framework`
crate was designed from the start to be radio-independent and liftable into a
shared library "as-is" once its own refactor is complete and extraction is
explicitly warranted (see `ts570d` ADR 0004). That extraction **has not
happened yet**. What exists here today is planning and agent scaffolding —
ADRs recording the target scope and crate boundaries, and `.claude/agents/`
definitions for the specialist subagents that will do the extraction and
subsequent development — so that when extraction does happen, it is a
deliberate move guided by a pre-recorded design rather than an improvised one.

Do not expect a `Cargo.toml`, a workspace, or any crate source under this
repository yet.

## What this repository will eventually hold

Per `ts570d` ADR 0005 (and this repo's own [ADR 0001](docs/adr/0001-scope-and-crate-boundaries.md)),
the target crate layout is:

- `cat-framework` — the generic CAT command engine (command table, parsing,
  dispatch, response building), radio-independent.
- `cat-client` — the generic client-side request/response mechanics used by a
  radio's typed controller client.
- `cat-transport-core` — the `Transport` / `CatSession` abstractions shared by
  all transport implementations.
- `cat-transport-serial`, `cat-transport-tcp`, `cat-transport-udp` — concrete
  transport implementations (serial/io_uring today; TCP and UDP framing to
  come).
- `cat-server` — the request broker that lets one physical radio connection be
  shared by multiple remote clients.

## Source of truth

The target design for this repository is recorded primarily in `ts570d`, not
here, because it was written while extracting `ts570d`'s own `framework` crate
was still in progress:

- `ts570d` ADR [0001](../ts570d/docs/adr/0001-generic-cat-framework.md) — why
  the generic CAT engine is radio-independent in the first place.
- `ts570d` ADR [0004](../ts570d/docs/adr/0004-extraction-boundary.md) — the
  extraction boundary: what moves here, what stays, and that extraction itself
  is out of scope until `ts570d`'s refactor is mature.
- `ts570d` ADR [0005](../ts570d/docs/adr/0005-network-transport-readiness.md) —
  the network-transport/server-mode addendum that names this repository's
  target crate layout and the open design questions it must resolve.
- `ts570d`'s [`docs/architecture/network-readiness.md`](../ts570d/docs/architecture/network-readiness.md) —
  diagrams of how serial/TCP/UDP and control/server mode attach.

This repo's own [`docs/adr/0001-scope-and-crate-boundaries.md`](docs/adr/0001-scope-and-crate-boundaries.md)
reconstructs and records that target design in this repository's terms, so it
does not have to be re-derived from `ts570d` every time this repo is picked
back up.

## Contributing / working in this repo right now

See [`CLAUDE.md`](CLAUDE.md) for the planning-with-files convention and the
agent roster in `.claude/agents/`. Until extraction is explicitly approved,
work here is limited to planning documents — no Rust code, no `cargo init`,
no files moved out of `ts570d`.
