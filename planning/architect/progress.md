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

## Next action (superseded by the entry below for the Windows planning pass;
kept for history)

Await review of ADR 0002, the ADR 0001 amendments, and the task_plan.md
dispatch queue. On approval, dispatch Task 1 to the `cat_framework` agent.

## 2026-07-19 — Planning pass: Windows serial backend (ADR 0002's revisit trigger fired)

Status: **planning complete, no code written** (same constraints as every
prior pass — no Rust/Cargo, no edits to `ts570d`/`ft991a`).

Trigger: the user wants real Windows COM-port control of a physical FT-991A
from a native Windows `ft991a` build (and eventually `ts570d`), with
explicit direction to keep `monoio`/io_uring unchanged for Linux and add a
genuinely separate Windows backend inside `cat-transport-serial` — not a
runtime-agnostic redesign touching the Linux path.

Read before deciding (not touched, reference only): `ft991a/ui/src/
terminal.rs`, `ft991a/src/main.rs` (single sequential loop, no
`monoio::spawn`), `ts570d/ui/src/terminal.rs` (genuine concurrent two-task
design via `monoio::spawn` + channels, for UI responsiveness during slow
polls), plus this repo's own `cat-transport-serial/src/{io_uring.rs,lib.rs,
session.rs}`, `cat-transport-core/src/{transport.rs,modem.rs,errors.rs}`,
and ADR 0002/0003 in full.

Delivered:
- `docs/adr/0004-windows-serial-backend.md` — new ADR. Async-execution
  decision: a dedicated background OS thread doing blocking Win32
  `ReadFile`/`WriteFile`, paired with a small hand-rolled single-slot
  completion primitive (not blocking-in-async-fn as the general mechanism,
  not a third async-runtime crate) — reasoning tied directly to `ts570d`'s
  concurrent two-task architecture, which a naive blocking implementation
  would silently break on a future Windows port even though it would be
  harmless for `ft991a`'s simpler single-loop shape today. Crate/module
  structure: same `cat-transport-serial` crate, same public type names
  (`SerialPort`/`SerialConfig`/`Parity`/`FlowControl`), `#[cfg(target_os =
  "windows")]`-gated internals in a new `windows.rs` alongside the existing
  `io_uring.rs`, with `SerialConfig`/`Parity`/`FlowControl` extracted into a
  new shared, ungated `config.rs` (behavior-preserving move, not a
  duplication) — not a new crate. Full `SerialConfig` ↔ `DCB` field mapping
  table (every field maps cleanly; one deliberate cross-platform
  consistency choice flagged — reusing Linux's validated baud-rate set on
  Windows even though `DCB.BaudRate` itself is more permissive). Win32
  dependency: `windows-sys`, target-gated exactly like the existing
  Linux-gated `monoio` entry. `ModemControlLines` maps to direct,
  synchronous `EscapeCommFunction`/`GetCommModemStatus` calls, mirroring
  ADR 0003's "no I/O wait" precedent exactly.
- `docs/adr/0002-...md` — small appended "Amendment" section (not a
  rewrite) pointing at ADR 0004 as the resolution of the revisit trigger
  ADR 0002 itself named.
- `docs/adr/README.md` — ADR 0004 row added; a short paragraph added
  pointing at the new dispatch queue.
- `planning/architect/findings.md` §11 — the supporting research: what was
  read in both consuming repos and why the asymmetry between `ft991a`'s and
  `ts570d`'s UI architectures drove the async-execution decision; why
  option 3 (a third runtime) was rejected in concrete terms; explicit note
  that `monoio` is never in the picture on the Windows side at all, which
  is what makes the hand-rolled completion primitive's correctness rest on
  `std::task::Waker`'s ordinary contract rather than on `monoio`
  internals.
- `planning/architect/task_plan.md` — Tasks 6, 7, 8 appended (`cat_transport`
  agent, sequential): Task 6 extracts `config.rs` and adds the portable
  `oneshot.rs` completion primitive (real `cargo test` coverage, since it
  has no OS dependency); Task 7 implements Windows `SerialPort::open`/`DCB`
  configuration/`SetCommTimeouts` only; Task 8 implements
  `Transport`/`ModemControlLines` plus the worker thread. Verification
  boundary stated explicitly and differently from every prior task in this
  file: this sandbox is Linux-only and cannot execute Windows binaries, so
  Tasks 7–8's "done when" is `cargo check --target x86_64-pc-windows-gnu`
  compiling cleanly, not `cargo test` — matching the user's own statement
  that they will validate against real hardware.

Not done, and explicitly not silently dropped:
- No task dispatched yet — same one-task-at-a-time, review-before-next
  workflow as every prior pass. Task 6 is next, pending review of ADR 0004
  and this task_plan.md addition.
- `ft991a`'s and `ts570d`'s own Windows entry-point work (replacing
  `#[monoio::main]`, since `monoio` cannot compile on Windows at all) is
  recorded as a needed follow-on in ADR 0004 §1 but is explicitly out of
  scope for this repository's dispatch queue — a future planning pass in
  each of those repos, gated on Task 8 landing here and on those repos'
  own architects picking it up, not something this session touched or
  authorized.

## Next action

Await review of ADR 0004 and the Task 6–8 additions to `task_plan.md`. On
approval, dispatch Task 6 to the `cat_transport` agent.

## 2026-08-27 — ADRs 0010-0013 Accepted; Task 10 executed

User sign-off ("accept and start"). Status flipped to **Accepted** on
`radio-cat-rs` ADRs 0010 (capability model + normalized signal), 0011 rev 4
(`cat-ui` for both renderers), 0012 (native MSVC), 0013 (renderer parity),
and `ts570d` ADR 0008 (GPU `gui` crate). Both ADR indexes updated. The
`ts570d` and `ft991a` `CLAUDE.md` pending-amendment blocks are now in force
as direction — with the explicit caveat, written into both, that Rules 1-7
still describe and govern the current code because none of the migration
has been written; what acceptance forbids is *new* code entrenching the
superseded framing.

**Task 10 (`release_workflow`) executed** — see
`planning/release_workflow/{findings,progress}.md`. Config and docs
complete; both `windows-check` jobs now run `cargo check` + `cargo test` on
`windows-latest`. ADR 0012's caveat 1 (GPU crates under `cargo-xwin`)
closed affirmative by measurement, caveat 2 (Microsoft licence) narrowed to
developer machines and left for the user, caveat 3 (the local check cannot
run tests) newly recorded. One stale-scope bug found and fixed: `ft991a`'s
CI had excluded `server` from Windows verification for a month after the
upstream gap that justified it had closed.

**Outstanding on Task 10:** no `windows-latest` run has happened — these
repos are not pushed from here. That run is where ADR 0006 §4's
never-executed Windows tests finally execute.

**Next:** Task 11 (`cat-framework::capabilities`), unblocked.
