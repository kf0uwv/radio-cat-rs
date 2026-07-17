# Progress — architect

## 2026-07-16 — Planning pass: extraction + TCP/UDP/server authorized

Status: **planning complete, no code written** (per architect's constraints —
no Rust/Cargo, no `cargo init`, no edits to `ts570d`/`ft991a`).

Delivered:
- `docs/adr/0002-async-runtime-binding-for-transport-crates.md` — new ADR
  resolving the monoio open item ADR 0001 carried forward: retain `monoio`/
  `#[async_trait(?Send)]`, extend to all transport crates + `cat-server`,
  explicit revisit trigger recorded (Windows serial support, or a non-monoio
  runtime consumer).
- `docs/adr/0001-scope-and-crate-boundaries.md` — amended (not rewritten)
  with an "Amendments" section: monoio item resolved (pointer to ADR 0002);
  `cat-transport-core` dependency corrected (depends on `cat-framework`, not
  nothing, for `ResponseDisposition`/`ProtocolErrorKind` reuse);
  `cat-transport-serial` scope clarified to include `ts570d/serial/`'s
  concrete io_uring implementation; `framework::state_machine` scoped out
  entirely.
- `docs/adr/README.md` — ADR 0002 row added; repository status updated to
  reflect extraction being authorized and the dispatch queue existing.
- `planning/architect/findings.md` — full reasoning: monoio decision,
  workspace/Cargo-dependency-mechanism decision (git dependency, `origin`
  already configured, `file://` fallback noted), code-motion decision (clean
  copy-and-adapt, not git-subtree/filter-repo, with commit-hash citation as
  the provenance substitute), plus real findings from reading `ts570d`'s
  actual code that go beyond ADR 0001's paraphrase (§5 `cat-client`/
  `RadioClient` isn't generic yet; §6 `cat-transport-serial` needs
  `ts570d/serial/`; §7 the dependency correction; §8 `state_machine`
  exclusion; §9 `errors.rs` split; §10 wire-framing coordination between
  Task 4 and Task 5).
- `planning/architect/task_plan.md` — full dispatch queue: Task 1
  (`cat_framework`: `cat-framework`) → Task 2 (`cat_transport`:
  `cat-transport-core` + `cat-transport-serial`) → Task 3 (`cat_framework`:
  `cat-client`, flagged as design work not a pure move) → Task 4a
  (`cat_transport`: `cat-transport-tcp`) → Task 4b (`cat_transport`:
  `cat-transport-udp`) → Task 5 (`cat_server`: `cat-server`). Ordering
  rationale and cross-task dependencies recorded per task.

Not done, and explicitly not silently dropped:
- Migrating `ts570d` itself onto these crates once they exist — a separate,
  later planning pass (see `findings.md` §4). `ts570d`/`ft991a` were not
  touched.
- No task has been dispatched yet. Per the standing "one task at a time,
  review before the next" workflow, Task 1 is next, pending user review of
  this planning pass.

## Next action

Await review of ADR 0002, the ADR 0001 amendments, and the task_plan.md
dispatch queue. On approval, dispatch Task 1 to the `cat_framework` agent.
