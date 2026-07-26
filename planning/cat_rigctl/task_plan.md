# Task Plan — cat_rigctl

## Origin

Dispatched directly (not via `planning/architect/task_plan.md`'s existing
numbered queue at the time this task started — the architect entry for this
work is added retroactively as part of this task, see that file's new
"Task 12" entry). Source of truth for scope: the orchestrating agent's
prompt, which itself cites two real, already-fixed source files in the
sibling apps `ft991a` and `ts570d` as ground truth. Both sibling repos are
READ-ONLY for this task — nothing under `/home/mattfranklin/.../ft991a` or
`/home/mattfranklin/.../ts570d` is modified.

## Problem being solved

Both `ft991a` and `ts570d` independently hand-wrote:
1. `server/src/broker_session.rs` — a `CatSession` adapter over
   `cat_server::BrokerHandle`. 100% duplicated, zero radio-specific logic.
2. `server/src/rigctl.rs` — a Hamlib rigctld-compatible TCP bridge for
   WSJT-X's "Hamlib NET rigctl" rig type. ~90% duplicated; only the mode
   name table and the radio's typed client type differ. `ft991a`'s copy has
   two bugfixes (`\dump_state` capability tail width, `F`'s `%f`-formatted
   float parsing) that `ts570d`'s copy still lacks.
3. `server/src/lib.rs`'s `run()` orchestration — ~90% duplicated. `ts570d`'s
   copy has a bugfix (real error propagation through `select_all`) that
   `ft991a`'s copy lacks.

Goal: extract the generic 90-100% into this shared repo so both apps can
delete their local copies and consume the shared crate, each supplying only
the ~10% that is genuinely radio-specific (mode name mapping) via a new
trait, `RigctlRadio`.

## Deliverable 1 — `BrokerCatSession` moves into `cat-server`

- New module `cat-server/src/broker_session.rs`, ported verbatim from
  `ft991a`'s copy (byte-identical to `ts570d`'s copy modulo doc-comment
  radio names and the test fixture's frequency digit width — 9 vs. 11 —
  which does not affect behavior).
- Doc comments rewritten to be radio-generic: "a radio crate's typed
  client" instead of naming FT-991A/TS-570D, and no reference to
  `crate::rigctl` (that module no longer lives in either app's `server`
  crate — the generic version lives in the new `cat-rigctl` crate instead).
- Reuse `cat-server/src/test_fixtures.rs`'s existing `FakeCommand`/`TABLE`
  fixture for the unit tests instead of writing a third copy — confirmed
  during investigation that it fits: the three tests this module needs
  (`execute_returns_response_written_with_response_bytes`,
  `execute_maps_err_wire_convention_to_transport_error`,
  `execute_returns_no_response_for_empty_wire_payload`) only exercise
  `Query`-shaped 0-width requests (`"FA;"`, `"ZZ;"`) — the fixture's 11-digit
  `Set` form width is never touched by these tests, so the digit-width
  divergence between the two apps' original private fixtures is moot once
  shared.
- `mod broker_session;` stays private in `cat-server/src/lib.rs`;
  `pub use broker_session::BrokerCatSession;` added, matching the existing
  re-export style for `broker`/`registry`.

## Deliverable 2 — new `cat-rigctl` crate

New workspace member `cat-rigctl/`. Ported from `ft991a`'s **current**
(bugfixed, real-Hamlib-verified) `server/src/rigctl.rs` for all
dispatch/dump_state/framing logic, and from `ts570d`'s **current**
(bugfixed) `server/src/lib.rs` for the `run()` error-propagation shape.

### Public API (see `investigation.md` — this directory's `findings.md`
equivalent, renamed per a harness-level constraint noted at the top of that
file — for the exact trait text once finalized)

- `RigctlRadio` trait — what the generic bridge needs from a radio's own
  typed client. `async_trait(?Send)`, matching every other async trait in
  this workspace (`CatSession`, `ModemControlLines`, ...) per ADR 0002's
  runtime binding. `Error` is deliberately unbounded (dispatch only
  branches Ok/Err, never displays the message — rigctld's `RPRT -1`
  convention carries no error text on the wire, confirmed by rereading
  `ft991a::rigctl::dispatch`: every `Err(_) => RPRT_ERR.to_string()` arm
  discards the error value).
- `ServerConfig` — moved verbatim (already 100% generic in both apps).
- `run<C, S, R, F>(session, table, config, make_radio) -> io::Result<()>` —
  the orchestration, generic over `C: CommandId`, `S: CatSession + 'static`,
  `R: RigctlRadio`, `F: Fn(BrokerCatSession) -> R + Clone + 'static`.
- `dispatch`, `dump_state`, `RPRT_OK`/`RPRT_ERR`, `LineReader`, `serve`
  (the rigctld TCP accept loop) stay crate-private (`pub(crate)`/private) —
  only `RigctlRadio`/`ServerConfig`/`run` are part of the public contract,
  per the prompt's explicit instruction.

### Mode mapping is NOT generic

`hamlib_mode_name`/`hamlib_mode_from_name` are `RigctlRadio` trait methods,
implemented per-app. Confirmed by rereading `ft991a::rigctl`'s table (12
modes, with `C4fm`→`"USB"` and `AmN`→`"AM"` best-effort fallbacks with no
symmetric round-trip) versus what `ts570d`'s manual and its `Mode` domain
type describe (a clean 8-mode 1:1 table, per the prompt) — these are
genuinely different data, not a refactor opportunity.

### Tests to port

- `ft991a::rigctl`'s full `dispatch_*`/`dump_state`/`hamlib_mode_round_trips`
  suite, adapted to an in-crate `FakeRadio: RigctlRadio` (mirroring
  `cat-server::test_fixtures`'s `FakeCommand` pattern) — no dependency on a
  real `Ft991a`/`ScriptedCatSession`/`radio` crate.
- `ft991a::server`'s `run_with_no_listeners_configured_returns_an_error`.
- `ts570d::server`'s `select_all_over_a_failing_listener_task_propagates_its_error`
  regression test — the one that specifically guards the error-propagation
  fix. Must not be dropped.

### `Cargo.toml`

Path deps: `cat-server`, `cat-framework`, `cat-transport-core`.
Workspace deps: `async-trait`, `futures` (new to workspace.dependencies —
not present yet, add it), `tracing`. `monoio` under
`[target.'cfg(target_os = "linux")'.dependencies]`, matching `cat-server`'s
precedent exactly. Doc-comment-per-dependency, matching this repo's
existing convention (every crate's `Cargo.toml` here explains *why*).

## Design decisions flagged for ADR judgment

New shared crate + a new trait-based abstraction point between the generic
library and each app's radio-specific typed client is exactly the kind of
decision this repo's ADR density suggests recording. Plan: add
`docs/adr/0005-rigctl-bridge-and-radio-trait-boundary.md` and update
`docs/adr/README.md`'s index table, unless writing it surfaces that it's
purely mechanical extraction with no real judgment call — in which case a
progress.md note suffices instead. (Resolved in `progress.md` once decided.)

## Verification plan

1. `cargo build --workspace`
2. `cargo test --workspace`
3. `cargo clippy --workspace --all-targets -- -D warnings`
4. `cargo fmt --check`
5. `cargo tree -p cat-rigctl` — confirm no radio-specific / unexpected deps.

## Non-goals / explicitly out of scope

- Modifying `ft991a` or `ts570d` in any way (read-only ground truth).
- Pushing to `origin` (local commit only).
- Any new WSJT-X-facing behavior change — this is a pure extraction, wire
  behavior must be byte-identical to `ft991a`'s current, already-verified
  copy.
