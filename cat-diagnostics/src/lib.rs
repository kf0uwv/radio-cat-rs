// Copyright 2026 Matt Franklin
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! A generic, radio-independent diagnostics/self-test engine.
//!
//! Per `docs/adr/0007-shared-diagnostics-engine.md`: extracted from (but
//! **not** a port of) `ts570d`'s `ui::terminal::run_diagnostics_task` — that
//! implementation hand-codes 107 typed `radio::Radio`-trait method calls
//! (set-then-verify round trips, including ones that key the transmitter and
//! tune the antenna), with a snapshot/restore safety net, entirely specific
//! to the TS-570D's own domain types. None of that is portable: a generic
//! engine has no `radio::Frequency`/`radio::Mode` to construct and no way to
//! know a "safe" value to write to an arbitrary command.
//!
//! This crate takes a structurally different, deliberately narrower
//! approach that needs no radio-specific knowledge at all:
//!
//! - **Read-only, by construction.** [`run_diagnostics`] never calls
//!   [`cat_client::CatClient::set`] and never invents a parameter value to
//!   *write*. For each [`CommandDefinition`] in the supplied
//!   [`CommandTable`], it issues the command's own documented **read**
//!   form — either a genuine zero-width [`CommandOperation::Query`] form, or
//!   (per [`CommandForm::is_selector_read`]) a selector-read `Set`-shaped
//!   form using an all-zero-digit selector (`"0"` repeated to the form's
//!   width) as a conservative, generically-applicable probe value (every
//!   selector-read command in this workspace's existing radio tables is a
//!   numeric index — meter number, menu item, VFO/memory slot — where index
//!   `0` is a reasonable, low-risk default probe, though not a guarantee for
//!   every conceivable radio). A command with neither kind of read form
//!   (write/action-only, e.g. `TX`, `send_cw`, `start_antenna_tuning`) is
//!   recorded as [`CommandResult::Skipped`] rather than guessed at — every
//!   command in the table still appears in the report, either tested or
//!   explicitly skipped with a reason.
//! - **Liveness, not functional correctness.** Because there is no
//!   radio-specific expected value to compare against, "passing" means "the
//!   radio answered this documented read command within the configured
//!   timeout, with no protocol/transport error" — not "the value it
//!   returned matches physical reality." This is a strictly shallower check
//!   than `ts570d`'s current set/get value-comparison diagnostics; the
//!   trade is exactly what buys genericity and safety (no state mutation,
//!   no radio-specific probe values). A radio crate that wants deeper,
//!   radio-specific verification keeps that logic in its own `ui` crate,
//!   layered on top of (or instead of) this engine.
//! - **Bounded per-command**, regardless of whether the underlying
//!   [`CatSession`] enforces a timeout of its own (mirroring
//!   `cat-server::Broker::dispatch`'s identical reasoning for the same
//!   underlying transport property — `cat-transport-tcp::TcpCatSession` has
//!   no timeout of its own by design). Uses the shared, portable
//!   [`cat_transport_core::timeout::timeout`] combinator (`docs/adr/
//!   0006-windows-network-transport.md`), so this crate needs no `monoio`
//!   dependency of its own to get a working timeout on every platform.
//!
//! See [`run_diagnostics`]/[`run_diagnostics_with`] for the exact call
//! shape, and this crate's `tests` module for a full usage example against
//! [`cat_transport_core::test_support::ScriptedCatSession`] — the same
//! fake-transport test double already used throughout this workspace (no
//! second test-double mechanism is introduced here).

mod engine;

pub use engine::{
    run_diagnostics, run_diagnostics_with, CommandOutcome, CommandResult, DiagnosticConfig,
    DiagnosticReport, DEFAULT_COMMAND_TIMEOUT,
};
