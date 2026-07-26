# 7. `cat-diagnostics`: a shared, radio-generic diagnostics/self-test engine

Date: 2026-07-26

## Status

Accepted

## Context

`ts570d` ships a diagnostics feature (the `[D]` screen its README describes
as "runs 99 CAT command round-trips" — 107 in the current source, see
below) that both apps would benefit from sharing instead of each
maintaining their own copy. This ADR records what was read, what was
deliberately **not** ported, and the exact API the new `cat-diagnostics`
crate exposes.

### What was read before designing

- `ts570d/ui/src/terminal.rs`'s `run_diagnostics_task` (and its
  `RadioCmd::StartDiagnostics` driver) and `ts570d/ui/src/diag.rs`'s
  `DiagResult`/`DiagState`/`DIAG_ROUNDS` data model, in full. Findings:
  - **107 hand-coded steps** (`LABELS`), each a direct call to a typed
    `radio::Radio`/`Ft991aExtras`-shaped method
    (`radio.set_vfo_a(Frequency::new(14_195_000)).await`, `radio.
    get_smeter().await`, `radio.send_cw("TEST").await`, ...), repeated
    `DIAG_ROUNDS` = 3 times each.
  - **It mutates radio state**, including keying the transmitter
    (`send_cw`) and tuning the antenna (`start_antenna_tuning`) — guarded by
    a `snapshot_state`/`restore_state` pair (reads everything readable
    before the run, best-effort writes it all back after, PTT cleared
    first, `power_on` restored last).
  - **Pass/fail is a set-then-verify-get value comparison** against
    hardcoded target values (e.g. set VFO A to 14,195,000 Hz, then assert
    the read-back equals it exactly) — genuine functional verification, not
    mere liveness.
  - **No latency tracking** on `DiagResult` at all.
  - **Entirely TS-570D-specific**: `radio::Frequency`, `radio::Mode`, and
    every target value are FT-991A/TS-570D-domain concepts with no generic
    equivalent. A radio-agnostic engine has no such types to construct and
    no way to know what a "safe" value is for an arbitrary command.
  - `ts570d/ui/src/layout.rs`'s `draw_diag_panel` renders three states
    (`Idle`/`Running`/`Done`) with live "Now testing: `<label>` [round
    N/3]" progress and a final pass/fail summary.
- `cat-framework/src/cat.rs` — `CommandTable::definitions()` (returns
  `&'static [CommandDefinition<C>]`), `CommandDefinition::{is_readable,
  is_writable, is_selector_read}`, `CommandForm::{operation, min_len,
  max_len, is_selector_read}`.
- `cat-client/src/client.rs` — `CatClient::{query, query_with_param, set}`,
  `ClientError<E>`.
- `cat-transport-core::test_support::{Exchange, ScriptedCatSession}` and
  `cat-server::broker::tests::NeverRespondingSession`'s pattern (a
  `CatSession` whose `execute()` never resolves, to exercise a real timeout
  independent of `ScriptedCatSession::simulate_timeout`'s immediate-`Err`
  shape).

## Decision

### 1. Structurally different from `ts570d`'s implementation, not a port of it

`cat-diagnostics` does not carry forward `ts570d`'s set/get value-comparison
approach — that requires exactly the radio-specific domain knowledge a
generic crate cannot have. Instead:

- **Read-only, by construction.** [`run_diagnostics`]/[`run_diagnostics_with`]
  never call [`cat_client::CatClient::set`] and never invent a value to
  *write*. For each [`cat_framework::CommandDefinition`] in the supplied
  table, they issue the command's own documented **read** form:
  - a genuine zero-width [`cat_framework::CommandOperation::Query`] form, if
    one exists; else
  - a selector-read `Set`-shaped form (per [`cat_framework::CommandForm::
    is_selector_read`]), probed with an all-zero-digit selector (`"0"`
    repeated to the form's width) — every selector-read command across this
    workspace's existing radio tables (meter number, menu item, VFO/memory
    slot) is a numeric index where `0` is a reasonable, low-risk default,
    though not a guarantee for every conceivable radio's convention; else
  - neither exists: recorded as [`CommandResult::Skipped`] with an explicit
    reason. **Every command in the table appears in the report** — tested
    or explicitly skipped, never silently omitted.
- **Liveness, not functional correctness.** "Passing" means "the radio
  answered this documented read command within the configured timeout, with
  no protocol/transport error" — not "the value returned matches physical
  reality." This is a deliberately shallower check than `ts570d`'s
  set/get comparison; the trade buys genericity and safety (no radio-
  specific probe values, no state mutation) directly. A radio crate that
  wants deeper, radio-specific verification keeps that logic in its own
  `ui` crate, layered on top of (or instead of) this engine — out of this
  ADR's scope to prescribe.
- **Bounded per command**, regardless of whether the underlying
  [`cat_transport_core::CatSession`] enforces a timeout of its own —
  mirroring `cat-server::Broker::dispatch`'s identical reasoning for the
  same underlying transport property (`cat-transport-tcp::TcpCatSession`
  has none by design).

### 2. Timeout mechanism: the same `target_os` split as `cat-server`, for the same load-bearing reason

[ADR 0006](0006-windows-network-transport.md) §4 found that
`cat_transport_core::timeout` (a portable, `std::thread`-spawning
timeout combinator) is not safe to drive from code being polled by a real
`monoio` runtime — its cross-thread `Waker::wake()` call is not reliably
observed by `monoio`'s executor, discovered by a real test hang, not by
inspection. `cat-diagnostics` hit the identical failure the first time its
own `#[monoio::test]`-based suite ran against a version that called the
portable combinator unconditionally, for the identical underlying reason:
**this engine is meant to be called directly from a consuming app's own
`monoio`-based radio task on Linux** (`ts570d`'s/`ft991a`'s control-mode UI
today runs everything, including a future diagnostics call, under
`monoio`), not only from a test harness. Fixed with the same split:
`monoio::time::timeout` on Linux (a real, non-dev, target-gated production
dependency of this crate — not merely a test convenience), the portable
[`cat_transport_core::timeout::timeout`] combinator on Windows. Anyone
extending this crate's timeout logic must preserve this split; do not
collapse it back to a single implementation.

### 3. The exact public API

```rust
// cat-diagnostics = { git = "https://github.com/kf0uwv/radio-cat-rs" }

pub const DEFAULT_COMMAND_TIMEOUT: std::time::Duration; // 2 seconds

pub struct DiagnosticConfig {
    pub per_command_timeout: std::time::Duration,
}
impl Default for DiagnosticConfig { /* per_command_timeout: DEFAULT_COMMAND_TIMEOUT */ }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandResult {
    Success { response: String },   // raw wire response text, verbatim
    Failure { message: String },    // the underlying error's Display text
    Timeout,
    Skipped { reason: &'static str },
}
impl CommandResult {
    pub fn is_success(&self) -> bool; // true for Success only
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome<C: cat_framework::CommandId> {
    pub id: C,
    pub code: &'static str,
    pub name: &'static str,
    pub request: String,            // raw wire request text sent; empty if Skipped
    pub result: CommandResult,
    pub latency: std::time::Duration, // zero if Skipped
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticReport<C: cat_framework::CommandId> {
    pub outcomes: Vec<CommandOutcome<C>>, // one per table.definitions(), in table order
}
impl<C: cat_framework::CommandId> DiagnosticReport<C> {
    pub fn total(&self) -> usize;
    pub fn passed(&self) -> usize;
    pub fn failed(&self) -> usize;   // Failure + Timeout
    pub fn skipped(&self) -> usize;
}

/// Runs every command in `table` against `client`, using
/// DiagnosticConfig::default() and no progress callback.
pub async fn run_diagnostics<C, S>(
    client: &mut cat_client::CatClient<C, S>,
    table: &'static cat_framework::CommandTable<C>,
) -> DiagnosticReport<C>
where
    C: cat_framework::CommandId,
    S: cat_transport_core::CatSession,
    S::Error: std::error::Error + 'static;

/// Like `run_diagnostics`, with an explicit config and a callback invoked
/// with each `CommandOutcome` as soon as it is known (before moving on to
/// the next command) -- the hook a UI render loop uses for live progress.
pub async fn run_diagnostics_with<C, S, F>(
    client: &mut cat_client::CatClient<C, S>,
    table: &'static cat_framework::CommandTable<C>,
    config: &DiagnosticConfig,
    on_progress: F,
) -> DiagnosticReport<C>
where
    C: cat_framework::CommandId,
    S: cat_transport_core::CatSession,
    S::Error: std::error::Error + 'static,
    F: FnMut(&CommandOutcome<C>);
```

### 4. Usage example (the call shape a consuming `ui` crate needs)

```rust
use cat_diagnostics::{run_diagnostics_with, DiagnosticConfig, CommandOutcome};

// `client: &mut CatClient<Ts570dCommandId, S>` (or Ft991aCommandId),
// `TABLE: &'static CommandTable<..>` -- the SAME table/client the app's
// normal control-mode code already builds; no new wiring needed.
let mut on_progress = |outcome: &CommandOutcome<_>| {
    // e.g. push a UI update onto the existing radio->UI channel, mirroring
    // ts570d's own RadioUpdate::DiagProgress today:
    send_update(RadioUpdate::DiagProgress {
        label: outcome.name,
        passed: outcome.result.is_success(),
        detail: format!("{:?}", outcome.result),
    });
};

let report = run_diagnostics_with(client, TABLE, &DiagnosticConfig::default(), &mut on_progress).await;

println!(
    "{}/{} passed, {} failed, {} skipped",
    report.passed(), report.total(), report.failed(), report.skipped()
);
```

No `[D]`-key/render-loop wiring is prescribed here — each app's `ui` crate
keeps that (its existing `DiagState`/`draw_diag_panel`-shaped code, or a new
equivalent), driven by `on_progress`/the final `DiagnosticReport` instead of
by 107 hand-coded steps.

## Consequences

- New workspace crate `cat-diagnostics`, depending only on `cat-framework`,
  `cat-client`, and `cat-transport-core` (+ Linux-target-gated `monoio`,
  §2) — never a concrete radio crate, never a concrete transport crate,
  matching this repo's existing dependency rules for `cat-client`.
- `ts570d`/`ft991a` are **not modified** by this ADR — wiring this crate
  into either app's `ui` (replacing, augmenting, or leaving alone their
  existing diagnostics screens) is explicitly out of scope here, left to
  each app's own follow-on work, per this task's own scope boundary.
- This engine's "passing" is a strictly weaker guarantee than `ts570d`'s
  current diagnostics (liveness, not value correctness) — this is a known,
  deliberate, and documented trade, not an oversight. An app that wants
  `ts570d`-style deep verification keeps that logic itself.
- Tested with `cat_transport_core::test_support::{Exchange,
  ScriptedCatSession}` (no second test-double mechanism invented) plus one
  local `NeverRespondingSession` fake mirroring `cat-server::broker::
  tests`'s identical pattern, for the one case `ScriptedCatSession` itself
  cannot simulate (a genuine hang, as opposed to an immediate `Err`).
  `cargo test -p cat-diagnostics`: 4 passed. `cargo check --target
  x86_64-pc-windows-gnu -p cat-diagnostics`: clean.
