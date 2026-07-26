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

//! The diagnostics engine itself. See the crate root doc for the design
//! rationale; this module is the exact call shape.

use std::future::Future;
use std::time::{Duration, Instant};

use cat_client::CatClient;
use cat_framework::{CommandDefinition, CommandId, CommandOperation, CommandTable};
use cat_transport_core::CatSession;

/// Bound `fut` to `duration`, resolving to `Err(cat_transport_core::
/// timeout::Elapsed)` if it hasn't completed in time.
///
/// Two bodies, selected per platform -- see
/// `docs/adr/0006-windows-network-transport.md`'s §4 finding
/// (`cat-server::broker::with_request_timeout` is the original instance of
/// this exact split, for the identical reason): `monoio::time::timeout` on
/// Linux (this engine is meant to run directly inside a consuming app's own
/// `monoio`-based radio task there); the shared, portable
/// [`cat_transport_core::timeout::timeout`] combinator on Windows (no
/// `monoio` to be incompatible with). **Do not replace this with a single
/// call to the portable combinator** -- confirmed by this crate's own test
/// suite hanging under `#[monoio::test]` when a first draft did exactly
/// that (the portable combinator's cross-thread `Waker::wake()` is not
/// reliably observed by `monoio`'s executor).
#[cfg(target_os = "linux")]
async fn with_probe_timeout<F: Future>(
    duration: Duration,
    fut: F,
) -> Result<F::Output, cat_transport_core::timeout::Elapsed> {
    monoio::time::timeout(duration, fut)
        .await
        .map_err(|_elapsed| cat_transport_core::timeout::Elapsed)
}

/// See the Linux-side doc comment above.
#[cfg(target_os = "windows")]
async fn with_probe_timeout<F: Future>(
    duration: Duration,
    fut: F,
) -> Result<F::Output, cat_transport_core::timeout::Elapsed> {
    cat_transport_core::timeout::timeout(duration, fut).await
}

/// Default bound on a single command probe, used by [`run_diagnostics`].
/// Callers with different latency expectations should use
/// [`run_diagnostics_with`] with an explicit [`DiagnosticConfig`] instead.
pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(2);

/// Configuration for a diagnostic run.
#[derive(Debug, Clone)]
pub struct DiagnosticConfig {
    /// Maximum time to wait for a single command's response before
    /// recording [`CommandResult::Timeout`], regardless of whether the
    /// underlying [`CatSession`] enforces a timeout of its own (mirroring
    /// `cat-server::Broker::dispatch`'s identical reasoning for the same
    /// underlying transport property).
    pub per_command_timeout: Duration,
}

impl Default for DiagnosticConfig {
    fn default() -> Self {
        Self {
            per_command_timeout: DEFAULT_COMMAND_TIMEOUT,
        }
    }
}

/// The outcome of probing one command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandResult {
    /// The radio answered within the configured timeout, with no
    /// protocol/transport error. `response` is the raw wire response text
    /// (e.g. `"FA00014250000;"`), verbatim.
    Success { response: String },
    /// The radio (or the session/transport underneath it) reported an
    /// error for this exchange. `message` is that error's `Display` text.
    Failure { message: String },
    /// No response arrived within [`DiagnosticConfig::per_command_timeout`].
    Timeout,
    /// This command has no generically-safe read form to probe — either a
    /// write/action-only command (no query form, and no selector-read `Set`
    /// form), or an inconsistent table entry. Never a guess at a write
    /// value: see the crate root doc's "read-only, by construction" section.
    Skipped { reason: &'static str },
}

impl CommandResult {
    /// `true` for [`CommandResult::Success`] only.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }
}

/// The full result of probing one [`CommandDefinition`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome<C: CommandId> {
    /// The radio-owned command identifier.
    pub id: C,
    /// The wire command code (e.g. `"FA"`).
    pub code: &'static str,
    /// The command's human-readable name, from the table.
    pub name: &'static str,
    /// The raw wire request text sent (empty for [`CommandResult::Skipped`]).
    pub request: String,
    /// What happened.
    pub result: CommandResult,
    /// Wall-clock time spent waiting on this command (zero for
    /// [`CommandResult::Skipped`], since nothing was sent).
    pub latency: Duration,
}

/// The full report from one diagnostic run: one [`CommandOutcome`] per
/// command in the supplied [`CommandTable`], in table order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticReport<C: CommandId> {
    pub outcomes: Vec<CommandOutcome<C>>,
}

impl<C: CommandId> DiagnosticReport<C> {
    /// Total number of commands covered (tested or skipped).
    pub fn total(&self) -> usize {
        self.outcomes.len()
    }

    /// Number of commands that answered successfully.
    pub fn passed(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| o.result.is_success())
            .count()
    }

    /// Number of commands that failed or timed out (tested, but not
    /// successfully).
    pub fn failed(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| {
                matches!(
                    o.result,
                    CommandResult::Failure { .. } | CommandResult::Timeout
                )
            })
            .count()
    }

    /// Number of commands that had no generically-safe read form to probe.
    pub fn skipped(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o.result, CommandResult::Skipped { .. }))
            .count()
    }
}

/// Choose the generic, radio-agnostic read probe for `def`, if one exists:
/// its own zero-width [`CommandOperation::Query`] form if it has one, else
/// a selector-read `Set`-shaped form (see [`cat_framework::CommandForm::
/// is_selector_read`]) using an all-zero-digit selector as a conservative
/// probe value. Returns `None` (never a write) if neither applies.
fn choose_probe<C: CommandId>(def: &CommandDefinition<C>) -> Option<String> {
    let has_zero_width_query = def.query_forms.iter().any(|form| {
        form.operation == CommandOperation::Query && form.min_len == 0 && form.max_len == 0
    });
    if has_zero_width_query {
        return Some(String::new());
    }

    def.set_forms
        .iter()
        .find(|form| form.is_selector_read)
        .map(|form| "0".repeat(form.min_len))
}

/// Run the diagnostic engine against every command in `table`, using
/// [`DiagnosticConfig::default`] and no progress callback. See
/// [`run_diagnostics_with`] for a version with an explicit config and/or a
/// live-progress callback (e.g. for a UI screen that wants to render each
/// step as it completes, mirroring `ts570d`'s existing diagnostics screen).
pub async fn run_diagnostics<C, S>(
    client: &mut CatClient<C, S>,
    table: &'static CommandTable<C>,
) -> DiagnosticReport<C>
where
    C: CommandId,
    S: CatSession,
    S::Error: std::error::Error + 'static,
{
    run_diagnostics_with(client, table, &DiagnosticConfig::default(), |_| {}).await
}

/// Like [`run_diagnostics`], with an explicit [`DiagnosticConfig`] and a
/// callback invoked with each [`CommandOutcome`] as soon as it is known
/// (before moving on to the next command) — the hook a UI render loop uses
/// to show live progress.
pub async fn run_diagnostics_with<C, S, F>(
    client: &mut CatClient<C, S>,
    table: &'static CommandTable<C>,
    config: &DiagnosticConfig,
    mut on_progress: F,
) -> DiagnosticReport<C>
where
    C: CommandId,
    S: CatSession,
    S::Error: std::error::Error + 'static,
    F: FnMut(&CommandOutcome<C>),
{
    let mut outcomes = Vec::with_capacity(table.definitions().len());

    for def in table.definitions() {
        let outcome = match choose_probe(def) {
            None => CommandOutcome {
                id: def.id,
                code: def.code,
                name: def.name,
                request: String::new(),
                result: CommandResult::Skipped {
                    reason: "no generic-safe read form (write/action-only command)",
                },
                latency: Duration::ZERO,
            },
            Some(params) => {
                let request = format!("{}{};", def.code, params);
                let started = Instant::now();
                let result = match with_probe_timeout(
                    config.per_command_timeout,
                    client.query_with_param(def.code, &params),
                )
                .await
                {
                    Err(_elapsed) => CommandResult::Timeout,
                    Ok(Ok(response)) => CommandResult::Success { response },
                    Ok(Err(e)) => CommandResult::Failure {
                        message: e.to_string(),
                    },
                };
                let latency = started.elapsed();
                CommandOutcome {
                    id: def.id,
                    code: def.code,
                    name: def.name,
                    request,
                    result,
                    latency,
                }
            }
        };

        on_progress(&outcome);
        outcomes.push(outcome);
    }

    DiagnosticReport { outcomes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_framework::{CommandForm, CommandOperation};
    use cat_transport_core::test_support::{Exchange, ScriptedCatSession};
    use std::cell::RefCell;

    // -----------------------------------------------------------------
    // In-crate fake CommandId / CommandTable, mirroring the identical
    // convention already used by cat-client's, cat-server's own test
    // modules — never a real radio crate.
    // -----------------------------------------------------------------

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FakeCommand {
        Frequency,   // plain zero-width query
        SignalMeter, // selector-read (Set-shaped), width 1
        Transmit,    // action-only, no read form at all
        SetOnly,     // set-only (fixed-width write), no read form at all
        Information, // plain zero-width query, read-only
    }

    const QUERY: &[CommandForm] = &[CommandForm::fixed(CommandOperation::Query, 0)];
    const SET_11: &[CommandForm] = &[CommandForm::fixed(CommandOperation::Set, 11)];
    const ACTION: &[CommandForm] = &[CommandForm::fixed(CommandOperation::Action, 0)];
    const NONE: &[CommandForm] = &[];
    const SELECTOR_READ_1: &[CommandForm] = &[CommandForm::selector_read(1)];

    static DEFINITIONS: &[CommandDefinition<FakeCommand>] = &[
        CommandDefinition {
            id: FakeCommand::Frequency,
            code: "FA",
            name: "Frequency",
            description: "Test frequency",
            query_forms: QUERY,
            set_forms: SET_11,
            action_forms: NONE,
            response_forms: NONE,
            readable: true,
            writable: true,
        },
        CommandDefinition {
            id: FakeCommand::SignalMeter,
            code: "SM",
            name: "Signal meter",
            description: "Test selector-parameter read",
            query_forms: NONE,
            set_forms: SELECTOR_READ_1,
            action_forms: NONE,
            response_forms: NONE,
            readable: true,
            writable: false,
        },
        CommandDefinition {
            id: FakeCommand::Transmit,
            code: "TX",
            name: "Transmit",
            description: "Test parameterless action, no read form",
            query_forms: NONE,
            set_forms: NONE,
            action_forms: ACTION,
            response_forms: NONE,
            readable: false,
            writable: true,
        },
        CommandDefinition {
            id: FakeCommand::SetOnly,
            code: "SO",
            name: "Set-only",
            description: "Test write-only command, no read form",
            query_forms: NONE,
            set_forms: SET_11,
            action_forms: NONE,
            response_forms: NONE,
            readable: false,
            writable: true,
        },
        CommandDefinition {
            id: FakeCommand::Information,
            code: "IF",
            name: "Information",
            description: "Test read-only information",
            query_forms: QUERY,
            set_forms: NONE,
            action_forms: NONE,
            response_forms: NONE,
            readable: true,
            writable: false,
        },
    ];

    static TABLE: CommandTable<FakeCommand> = CommandTable::new(DEFINITIONS);

    fn client_with_script<I: IntoIterator<Item = Exchange>>(
        script: I,
    ) -> CatClient<FakeCommand, ScriptedCatSession> {
        CatClient::new(ScriptedCatSession::with_script(script), &TABLE)
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn queries_every_readable_command_and_skips_the_rest() {
        let mut client = client_with_script([
            Exchange::new("FA;", "FA00014250000;"),
            Exchange::new("SM0;", "SM0015;"),
            Exchange::new("IF;", "IF017;"),
        ]);

        let report = run_diagnostics(&mut client, &TABLE).await;

        assert_eq!(report.total(), 5);
        assert_eq!(report.passed(), 3);
        assert_eq!(report.skipped(), 2);
        assert_eq!(report.failed(), 0);

        let fa = &report.outcomes[0];
        assert_eq!(fa.code, "FA");
        assert_eq!(fa.request, "FA;");
        assert_eq!(
            fa.result,
            CommandResult::Success {
                response: "FA00014250000;".to_string()
            }
        );

        let sm = &report.outcomes[1];
        assert_eq!(sm.code, "SM");
        assert_eq!(
            sm.request, "SM0;",
            "selector-read probe uses an all-zero selector"
        );
        assert_eq!(
            sm.result,
            CommandResult::Success {
                response: "SM0015;".to_string()
            }
        );

        let tx = &report.outcomes[2];
        assert_eq!(tx.code, "TX");
        assert!(matches!(tx.result, CommandResult::Skipped { .. }));
        assert!(tx.request.is_empty());
        assert_eq!(tx.latency, Duration::ZERO);

        let so = &report.outcomes[3];
        assert_eq!(so.code, "SO");
        assert!(matches!(so.result, CommandResult::Skipped { .. }));

        let info = &report.outcomes[4];
        assert_eq!(info.code, "IF");
        assert_eq!(
            info.result,
            CommandResult::Success {
                response: "IF017;".to_string()
            }
        );
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn a_session_error_is_recorded_as_failure_not_a_panic() {
        // A dedicated single-command table: `simulate_disconnect` only
        // fails the *next* call, and this engine keeps probing every
        // remaining command regardless of an earlier failure -- a
        // multi-command table would panic ScriptedCatSession's exhausted-
        // script check on the second probe. One command is exactly what
        // this test needs to prove.
        static ONE_DEFINITION: &[CommandDefinition<FakeCommand>] = &[CommandDefinition {
            id: FakeCommand::Frequency,
            code: "FA",
            name: "Frequency",
            description: "Test frequency",
            query_forms: QUERY,
            set_forms: SET_11,
            action_forms: NONE,
            response_forms: NONE,
            readable: true,
            writable: true,
        }];
        static ONE_TABLE: CommandTable<FakeCommand> = CommandTable::new(ONE_DEFINITION);

        let mut session = ScriptedCatSession::new();
        session.simulate_disconnect();
        let mut client = CatClient::new(session, &ONE_TABLE);

        let report = run_diagnostics(&mut client, &ONE_TABLE).await;

        assert_eq!(report.total(), 1);
        let fa = &report.outcomes[0];
        assert_eq!(fa.code, "FA");
        assert!(matches!(fa.result, CommandResult::Failure { .. }));
    }

    /// A `CatSession` whose `execute()` never resolves, to exercise this
    /// engine's own per-command timeout independent of anything
    /// `ScriptedCatSession` can simulate (`simulate_timeout` returns an
    /// immediate `Err`, not a hang) -- mirrors `cat-server::broker::tests::
    /// NeverRespondingSession` exactly.
    #[derive(Default)]
    struct NeverRespondingSession;

    #[async_trait::async_trait(?Send)]
    impl CatSession for NeverRespondingSession {
        type Error = cat_transport_core::TransportError;

        async fn execute(
            &mut self,
            _request: &[u8],
            _response: &mut Vec<u8>,
        ) -> Result<cat_framework::ResponseDisposition, Self::Error> {
            std::future::pending::<()>().await;
            unreachable!("pending() never resolves");
        }
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn never_answered_command_times_out_instead_of_hanging_the_whole_run() {
        let mut client = CatClient::new(NeverRespondingSession, &TABLE);
        let config = DiagnosticConfig {
            per_command_timeout: Duration::from_millis(50),
        };

        let started = Instant::now();
        let report = run_diagnostics_with(&mut client, &TABLE, &config, |_| {}).await;
        let elapsed = started.elapsed();

        // Three probed commands (FA, SM, IF), each individually timing out
        // -- proving the engine moves on rather than getting stuck on the
        // first one.
        let timeouts = report
            .outcomes
            .iter()
            .filter(|o| o.result == CommandResult::Timeout)
            .count();
        assert_eq!(timeouts, 3);
        assert_eq!(report.skipped(), 2);

        assert!(
            elapsed < Duration::from_secs(2),
            "looked like a hang: {elapsed:?}"
        );
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn progress_callback_fires_once_per_outcome_in_table_order() {
        let mut client = client_with_script([
            Exchange::new("FA;", "FA00014250000;"),
            Exchange::new("SM0;", "SM0015;"),
            Exchange::new("IF;", "IF017;"),
        ]);

        let seen: RefCell<Vec<&'static str>> = RefCell::new(Vec::new());
        let report = run_diagnostics_with(&mut client, &TABLE, &DiagnosticConfig::default(), |o| {
            seen.borrow_mut().push(o.code);
        })
        .await;

        assert_eq!(*seen.borrow(), vec!["FA", "SM", "TX", "SO", "IF"]);
        assert_eq!(seen.borrow().len(), report.total());
    }
}
