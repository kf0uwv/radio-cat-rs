# Findings: cat-diagnostics (Deliverable 2)

## What ts570d's existing diagnostics screen actually does (read in full:
ts570d/ui/src/{terminal.rs,diag.rs,layout.rs})

- `run_diagnostics_task` (terminal.rs) hand-codes 107 LABELS, each a direct
  call to a typed `radio::Radio`/`Ft991aExtras`-style method
  (`radio.set_vfo_a(Frequency::new(14_195_000)).await`, etc.), repeated
  DIAG_ROUNDS=3 times, with per-step set-then-verify-get comparisons against
  hardcoded target values. This DOES mutate radio state (frequency, mode,
  RIT/XIT, memory channel 0, transmits via `send_cw`/antenna tuning) --
  guarded by a `snapshot_state`/`restore_state` pair that reads everything
  readable before the run and best-effort restores it after. Aborreadable
  via [Esc]. Entirely TS-570D-specific (radio::Frequency, radio::Mode, ...).
- None of this is portable: a generic engine has no domain types to
  construct and no idea what a "safe" value is for an arbitrary command.
- `DiagResult { label, round, passed, detail }` (diag.rs) has no latency
  field. `layout.rs::draw_diag_panel` renders Idle/Running/Done states with
  live "Now testing: <label> [round N/3]" progress and a pass/fail summary.

## Design decision: read-only, generic, liveness-only

cat-diagnostics does NOT port ts570d's approach. It is deliberately narrower
and structurally different -- see cat-diagnostics/src/lib.rs's crate doc for
the full rationale. Summary:
- Iterates `CommandTable::definitions()` directly (not radio-specific
  labels). For each `CommandDefinition<C>`: if it has a genuine zero-width
  `Query` form, probe with `CatClient::query_with_param(code, "")`; else if
  it has a selector-read `Set`-shaped form (`CommandForm::is_selector_read`),
  probe with an all-zero-digit selector of the form's width (a reasonable,
  low-risk default for every existing selector-read command in this
  workspace's tables -- SM/RM meter index, MD mode index, EX menu item --
  all numeric indices where 0 is a plausible valid selector, though not a
  guarantee for every conceivable radio); else record `Skipped` with an
  explicit reason -- NEVER guesses a write value.
- "Passing" means "answered within timeout, no protocol/transport error" --
  liveness, not functional correctness (no expected-value comparison,
  since a generic engine has no idea what physical reality should be).
- Never calls `CatClient::set`. Read-only by construction.
- Bounded per-command via a timeout wrap, regardless of whether the
  underlying CatSession has one of its own (TcpCatSession has none by
  design) -- same reasoning as cat-server::Broker::dispatch.

## Important pitfall found and fixed: monoio + portable timeout

Reused the newly-promoted `cat_transport_core::timeout` (see
docs/adr/0006's §4/§5) directly at first -- this hung under this crate's
own `#[monoio::test]`-based tests for the exact reason recorded in ADR
0006 (monoio's Waker does not reliably support a foreign-OS-thread wake).
Fixed with the same cfg(target_os) split cat-server's
`with_request_timeout` already established: `monoio::time::timeout` on
Linux (this engine is meant to be called directly from a consuming app's
own monoio-based radio task, not just from tests), the portable combinator
on Windows. `monoio` is therefore a real (non-dev), Linux-target-gated
production dependency of this crate, matching cat-server/cat-transport-tcp/
cat-transport-udp's identical convention.

## Public API (see docs/adr/0007-shared-diagnostics-engine.md for the
authoritative, exact signatures)

- `DiagnosticConfig { per_command_timeout: Duration }` (+ `Default`)
- `DEFAULT_COMMAND_TIMEOUT: Duration`
- `CommandResult` enum: `Success { response }`, `Failure { message }`,
  `Timeout`, `Skipped { reason }` (+ `is_success()`)
- `CommandOutcome<C: CommandId> { id, code, name, request, result, latency }`
- `DiagnosticReport<C: CommandId> { outcomes: Vec<CommandOutcome<C>> }`
  (+ `total()`/`passed()`/`failed()`/`skipped()`)
- `run_diagnostics(client: &mut CatClient<C, S>, table: &'static CommandTable<C>) -> DiagnosticReport<C>`
- `run_diagnostics_with(client, table, config: &DiagnosticConfig, on_progress: impl FnMut(&CommandOutcome<C>)) -> DiagnosticReport<C>`
  -- the progress-callback hook a UI render loop uses for live updates,
  matching ts570d's existing "Now testing: ..." UX without needing this
  crate to know anything about UI/channels itself.

Tests: 4 unit tests against `ScriptedCatSession`/`Exchange` (no second test
double invented) plus a local `NeverRespondingSession` fake (mirrors
cat-server::broker::tests's identical pattern) to exercise the per-command
timeout without relying on ScriptedCatSession's simulate_timeout (which
returns an immediate Err, not a hang).
