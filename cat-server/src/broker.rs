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

//! The request broker: [`Broker`] owns one physical [`CatClient`] and
//! validates/executes raw wire-format requests against it; [`BrokerWorker`]
//! is the single ordered worker that owns a `Broker` exclusively and
//! services [`Job`]s pulled off [`crate::local_channel`]'s queue in
//! receive order; [`BrokerHandle`] is what client-facing accept/dispatch
//! loops (`crate::tcp`, `crate::udp`) hold to submit work and await their
//! own reply.
//!
//! # Why raw wire bytes, not `CatClient`'s `query`/`query_with_param`/`set`
//! # directly
//!
//! `cat-transport-tcp`/`cat-transport-udp`'s frames carry raw CAT
//! command/response bytes verbatim (e.g. `b"FA;"`, `b"FA00014250000;"`) —
//! the same shape a physical `CatSession::execute` exchanges with a radio.
//! A remote client of `cat-server` is conceptually another
//! `CatClient<C, TcpCatSession>`/`CatClient<C, UdpCatSession>` pointed at
//! this broker's socket instead of a radio's serial port. So the broker's
//! job per request is: parse and structurally validate one raw wire frame
//! against the same `&'static CommandTable<C>` the physical `CatClient`
//! was built with, then route it to the matching `CatClient` method.
//!
//! `CommandTable::parse` is the actual malformed-request gate — it checks
//! parameter width against `query_forms`/`set_forms`, which `CatClient`'s
//! own `query`/`query_with_param`/`set` do **not** do on their own (they
//! only check command existence and the `readable`/`writable` flags). A
//! request that fails `parse` never reaches `self.client`, and therefore
//! never reaches the physical session.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use cat_client::{CatClient, ClientError};
use cat_framework::{CommandId, CommandOperation, CommandTable, ParseError};
use cat_transport_core::CatSession;
use thiserror::Error;

use crate::local_channel::{self, Receiver, Sender};
use crate::registry::ClientId;

/// Default per-request timeout the broker enforces around every dispatch to
/// the physical radio session.
///
/// Load-bearing, not a nicety: some session implementations have no
/// timeout of their own by design (e.g. `cat-transport-tcp::TcpCatSession`,
/// which blocks reading a response frame forever if its peer goes silent —
/// its own docs defer "liveness enforcement against a misbehaving radio or
/// worker" explicitly to this layer). Without this wrap, a radio that never
/// answers would wedge the single ordered worker and starve every other
/// client indefinitely.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Outcome of one successfully-dispatched request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// A query executed; the physical radio's response text.
    Response(String),
    /// A set/action executed; the radio produced no response (fire-and-forget).
    NoResponse,
}

/// Everything that can go wrong dispatching one request, at any layer:
/// broker-boundary validation, the broker's own timeout, or the physical
/// session/radio itself.
#[derive(Debug, Error)]
pub enum DispatchError<E>
where
    E: std::error::Error + 'static,
{
    /// The request was not valid UTF-8 CAT wire text.
    #[error("request is not valid UTF-8")]
    InvalidEncoding,
    /// [`CommandTable::parse`] rejected the request before the physical
    /// session was touched — the malformed-request-rejection gate.
    #[error("malformed request: {0}")]
    Malformed(#[from] ParseError),
    /// The physical radio did not answer within the configured timeout. The
    /// in-flight call was cancelled; the worker is free to service the next
    /// request immediately.
    #[error("physical radio session did not respond within {0:?}")]
    Timeout(Duration),
    /// The physical `CatClient` reported an error executing an otherwise
    /// well-formed request (session I/O failure, or a radio-reported
    /// protocol error).
    #[error("physical radio session error: {0}")]
    Session(#[from] ClientError<E>),
}

/// Render a dispatch result as raw wire bytes for a client-facing listener
/// to frame and send back.
///
/// The frozen TCP/UDP frame formats (`planning/cat_transport/progress.md`)
/// only distinguish empty-vs-non-empty payload on the wire — there is no
/// wire-level "this was a protocol error" bit. Since there is no `CatRadio`
/// downstream to supply a radio-specific error convention (this crate's
/// charter: no radio state machine exists in this repo), every rejection
/// here is rendered as a non-empty `b"ERR <message>"` payload — no leading
/// two-letter code, no trailing `;`, so it can never collide with or be
/// mistaken for a real CAT frame (`<2 letters><data>;`) by a careful reader
/// — rather than silently answering with an empty (`NoResponse`-shaped)
/// frame that would hide the rejection entirely.
pub fn outcome_to_wire<E>(result: Result<DispatchOutcome, DispatchError<E>>) -> Vec<u8>
where
    E: std::error::Error + 'static,
{
    match result {
        Ok(DispatchOutcome::Response(text)) => text.into_bytes(),
        Ok(DispatchOutcome::NoResponse) => Vec::new(),
        Err(err) => format!("ERR {}", err).into_bytes(),
    }
}

/// Owns one physical radio session and validates/executes requests against
/// it. Never shared directly across tasks — see [`BrokerWorker`]/
/// [`BrokerHandle`] for how multiple clients serialize access through this
/// without introducing any interior concurrency around the session itself.
pub struct Broker<C, S>
where
    C: CommandId,
    S: CatSession,
    S::Error: std::error::Error + 'static,
{
    client: CatClient<C, S>,
    table: &'static CommandTable<C>,
    request_timeout: Duration,
}

impl<C, S> Broker<C, S>
where
    C: CommandId,
    S: CatSession,
    S::Error: std::error::Error + 'static,
{
    /// Build a broker around `session`, using [`DEFAULT_REQUEST_TIMEOUT`].
    pub fn new(session: S, table: &'static CommandTable<C>) -> Self {
        Self::with_timeout(session, table, DEFAULT_REQUEST_TIMEOUT)
    }

    /// Build a broker around `session`, with an explicit `request_timeout`.
    pub fn with_timeout(
        session: S,
        table: &'static CommandTable<C>,
        request_timeout: Duration,
    ) -> Self {
        Self {
            client: CatClient::new(session, table),
            table,
            request_timeout,
        }
    }

    /// Validate and execute one raw wire request against the physical
    /// session.
    ///
    /// Never forwards a request that fails [`CommandTable::parse`] to the
    /// session (malformed-request rejection at the broker boundary).
    /// Bounded by `request_timeout` regardless of whether the underlying
    /// session enforces one of its own.
    pub async fn dispatch(
        &mut self,
        request: &[u8],
    ) -> Result<DispatchOutcome, DispatchError<S::Error>> {
        let frame = std::str::from_utf8(request).map_err(|_| DispatchError::InvalidEncoding)?;
        let parsed = self.table.parse(frame)?;

        let code = parsed.code;
        let params = parsed.parameters.raw().to_string();
        let operation = parsed.operation;

        // `parse`'s structural operation label does not always match the
        // documented readable/writable *intent*: some commands document a
        // selector-parameterized *read* (e.g. a signal-meter read like
        // TS-570D's `SM`/`RM`, or the FT-991A's `MD`/`EX`) using a
        // `Set`-shaped form, because the generic query/set/action form
        // model has no other way to express "read with a required
        // parameter" — see `CommandDefinition::readable`'s own doc
        // comment, which names this exact case.
        //
        // A command that is *only* readable (never writable) — e.g.
        // TS-570D's `SM`/`RM` — is unambiguous from the coarse
        // `readable`/`writable` flags alone: never writable, so route by
        // `is_readable()`. But a command that is **both** readable and
        // writable via *different widths of the same `Set`-shaped form*
        // (e.g. the FT-991A's `MD0;` 1-byte selector read vs. `MD0<hex>;`
        // 2-byte write, or `EX047;` 3-byte read vs. `EX047<value>;` wider
        // write) cannot be routed correctly by those flags alone — both
        // widths are structurally `Set` and both flags are `true`
        // regardless of which width actually arrived, so a naive
        // "writable wins" rule would silently route every selector-read
        // request to `CatClient::set()` too, discarding the real read
        // response a fire-and-forget write never expects (found via live
        // testing against `ft991a`'s `MD`/`EX` commands — this crate's own
        // test suite only ever exercised the *read-only* case, which
        // can't surface this).
        //
        // So: check the *specific matched width*'s
        // [`CommandForm::is_selector_read`] tag first — set by a radio's
        // command table only on the exact form that is a read despite its
        // `Set` shape (see that field's own doc comment) — and fall back
        // to the coarse `readable`/`writable` flags only when the command
        // doesn't use this pattern at all (`is_selector_read` is `false`
        // for every form built via `CommandForm::fixed`/`variable`, so
        // this fallback is exact, not approximate, for every existing
        // command table).
        let definition = self
            .table
            .find(code)
            .expect("CommandTable::parse already resolved this code to a definition");

        let is_read = match operation {
            CommandOperation::Query => true,
            CommandOperation::Action => false,
            CommandOperation::Set => {
                if definition.is_selector_read(params.len()) {
                    true
                } else if definition.is_writable() {
                    false
                } else if definition.is_readable() {
                    true
                } else {
                    // Neither writable nor readable despite a matching Set
                    // form -- not expected from a well-formed command
                    // table, but rejected cleanly rather than guessing.
                    return Err(DispatchError::Malformed(ParseError::UnsupportedOperation(
                        code.to_string(),
                    )));
                }
            }
            // CommandTable::parse never actually produces this operation
            // (it is response-form metadata a controller never sends) —
            // handled defensively rather than with `unreachable!()`.
            CommandOperation::Response => {
                return Err(DispatchError::Session(ClientError::UnknownCommand(
                    code.to_string(),
                )));
            }
        };

        let timeout = self.request_timeout;
        let client = &mut self.client;
        let call = async move {
            if is_read {
                // Uniformly `query_with_param` — its formatting is
                // byte-identical to `query()`'s when `params` is empty
                // (both produce `"<code>;"`), so no separate code path is
                // needed for a parameterless query.
                client
                    .query_with_param(code, &params)
                    .await
                    .map(DispatchOutcome::Response)
            } else {
                client
                    .set(code, &params)
                    .await
                    .map(|_| DispatchOutcome::NoResponse)
            }
        };

        match monoio::time::timeout(timeout, call).await {
            Ok(result) => result.map_err(DispatchError::Session),
            Err(_elapsed) => Err(DispatchError::Timeout(timeout)),
        }
    }
}

/// One unit of work submitted to a [`BrokerWorker`].
///
/// `client_id`/`request_id` are carried for client-session
/// bookkeeping/observability; actual reply routing is structural (`reply`
/// is a single-use oneshot slot owned by exactly one waiting caller), not a
/// lookup-table scheme, so misrouting under interleaved concurrent clients
/// is not representable.
pub struct Job {
    /// Which registered client this request came from.
    pub client_id: ClientId,
    /// Monotonically-assigned id for this request, for tracking/logging.
    pub request_id: u64,
    /// Raw wire-format request bytes.
    pub payload: Vec<u8>,
    reply: local_channel::OneshotSender<Vec<u8>>,
}

/// The single ordered worker. Owns a [`Broker`] exclusively and services
/// [`Job`]s strictly in the order they were submitted — the only path by
/// which the physical session is ever touched.
///
/// Callers spawn `worker.run()` as one task (e.g. via `monoio::spawn`) and
/// distribute [`BrokerHandle`] clones to every client-facing task that
/// needs to submit work.
pub struct BrokerWorker<C, S>
where
    C: CommandId,
    S: CatSession,
    S::Error: std::error::Error + 'static,
{
    broker: Broker<C, S>,
    receiver: Receiver<Job>,
}

impl<C, S> BrokerWorker<C, S>
where
    C: CommandId,
    S: CatSession,
    S::Error: std::error::Error + 'static,
{
    /// Run until every [`BrokerHandle`] (and clone) has been dropped and the
    /// job queue is drained.
    pub async fn run(mut self) {
        while let Some(job) = self.receiver.recv().await {
            let result = self.broker.dispatch(&job.payload).await;
            let wire = outcome_to_wire(result);
            // A disconnected requester (dropped its `OneshotReceiver`
            // before the reply was ready) is not an error here — the
            // worker does not know or care; it simply moves on to the next
            // job immediately. This is exactly the "disconnect mid-request
            // must not wedge the worker" contract.
            let _ = job.reply.send(wire);
        }
    }
}

/// Handle used by client-facing tasks to submit work to a [`BrokerWorker`]
/// and await their own reply. Cheaply `Clone`-able (`Rc`-based) — every TCP
/// connection task / UDP datagram task holds one.
#[derive(Clone)]
pub struct BrokerHandle {
    sender: Sender<Job>,
    next_request_id: Rc<Cell<u64>>,
}

impl BrokerHandle {
    /// Submit one raw wire-format request from `client_id` and await the
    /// raw wire-format response bytes.
    ///
    /// Returns `None` only if the [`BrokerWorker`] itself has shut down
    /// (every handle dropped, or the worker task ended) — a listener
    /// should treat that as "the broker is gone" and stop serving new
    /// requests.
    pub async fn submit(&self, client_id: ClientId, payload: Vec<u8>) -> Option<Vec<u8>> {
        let request_id = self.next_request_id.get();
        self.next_request_id.set(request_id.wrapping_add(1));

        let (reply, mut reply_rx) = local_channel::oneshot();
        let job = Job {
            client_id,
            request_id,
            payload,
            reply,
        };
        self.sender.send(job).ok()?;
        reply_rx.recv().await
    }
}

/// Build a [`BrokerWorker`]/[`BrokerHandle`] pair around `session`, using
/// [`DEFAULT_REQUEST_TIMEOUT`].
pub fn build<C, S>(
    session: S,
    table: &'static CommandTable<C>,
) -> (BrokerWorker<C, S>, BrokerHandle)
where
    C: CommandId,
    S: CatSession,
    S::Error: std::error::Error + 'static,
{
    build_with_timeout(session, table, DEFAULT_REQUEST_TIMEOUT)
}

/// Like [`build`], with an explicit per-request timeout.
pub fn build_with_timeout<C, S>(
    session: S,
    table: &'static CommandTable<C>,
    request_timeout: Duration,
) -> (BrokerWorker<C, S>, BrokerHandle)
where
    C: CommandId,
    S: CatSession,
    S::Error: std::error::Error + 'static,
{
    let broker = Broker::with_timeout(session, table, request_timeout);
    let (sender, receiver) = local_channel::channel();
    let worker = BrokerWorker { broker, receiver };
    let handle = BrokerHandle {
        sender,
        next_request_id: Rc::new(Cell::new(0)),
    };
    (worker, handle)
}

#[cfg(test)]
mod tests {
    use std::future::pending;

    use async_trait::async_trait;
    use cat_framework::{CommandDefinition, CommandForm, ResponseDisposition};
    use cat_transport_core::test_support::{Exchange, ScriptedCatSession};
    use cat_transport_core::TransportError;

    use super::*;
    use crate::registry::ClientId;

    // -----------------------------------------------------------------
    // In-crate fake CommandId / CommandTable (never a real radio crate).
    // -----------------------------------------------------------------

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FakeCommand {
        Frequency,
        SignalMeter,
        Transmit,
        Information,
        Mode,
    }

    const QUERY: &[CommandForm] = &[CommandForm::fixed(CommandOperation::Query, 0)];
    const SET_11: &[CommandForm] = &[CommandForm::fixed(CommandOperation::Set, 11)];
    const ACTION: &[CommandForm] = &[CommandForm::fixed(CommandOperation::Action, 0)];
    const NONE: &[CommandForm] = &[];
    // A selector-parameterized *read* (e.g. a signal-meter query like
    // TS-570D's `SM0;`) is modeled with a `Set`-shaped form, per
    // `CommandDefinition::readable`'s own doc comment: `CommandTable::parse`
    // only ever classifies a parameterless frame as `Query` (its first
    // branch hardcodes width 0), so a width-1 read can only be reached via
    // the `Set` branch's width match. `readable: true, writable: false`
    // below is what tells `Broker::dispatch` to route it through
    // `query_with_param` despite the `Set` structural label.
    const SELECTOR_READ_AS_SET_1: &[CommandForm] = &[CommandForm::fixed(CommandOperation::Set, 1)];
    // A command that is **both** readable and writable via *different
    // widths of the same `Set`-shaped form — the FT-991A's `MD` (mode) is
    // the real-world case this reproduces: `MD0;` (1-byte selector read)
    // vs. `MD0<hex>;` (2-byte write). Unlike `SELECTOR_READ_AS_SET_1`
    // above (read-only, so the coarse `readable`/`writable` flags alone
    // already route it correctly), this is the shape that broke before
    // `CommandForm::selector_read`/`CommandDefinition::is_selector_read`
    // existed: both widths are structurally `Set` and both flags are
    // `true`, so a naive "writable wins" rule misrouted *even the read
    // width* to a fire-and-forget write, discarding the real response
    // (found via live testing against `ft991a`, not by this crate's own
    // prior test suite — see `Broker::dispatch`'s doc comment).
    const MODE_SET_FORMS: &[CommandForm] = &[
        CommandForm::selector_read(1),
        CommandForm::fixed(CommandOperation::Set, 2),
    ];

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
            set_forms: SELECTOR_READ_AS_SET_1,
            action_forms: NONE,
            response_forms: NONE,
            readable: true,
            writable: false,
        },
        CommandDefinition {
            id: FakeCommand::Transmit,
            code: "TX",
            name: "Transmit",
            description: "Test parameterless action",
            query_forms: NONE,
            set_forms: NONE,
            action_forms: ACTION,
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
        CommandDefinition {
            id: FakeCommand::Mode,
            code: "MD",
            name: "Mode",
            description: "Test readable-and-writable selector read (FT-991A MD shape)",
            query_forms: NONE,
            set_forms: MODE_SET_FORMS,
            action_forms: NONE,
            response_forms: NONE,
            readable: true,
            writable: true,
        },
    ];

    static TABLE: CommandTable<FakeCommand> = CommandTable::new(DEFINITIONS);

    fn broker_with_script<I: IntoIterator<Item = Exchange>>(
        script: I,
    ) -> Broker<FakeCommand, ScriptedCatSession> {
        Broker::new(ScriptedCatSession::with_script(script), &TABLE)
    }

    // -----------------------------------------------------------------
    // A CatSession double that never resolves `execute()` at all, to
    // exercise the broker's own timeout independent of anything
    // `ScriptedCatSession` can simulate (its `simulate_timeout` returns an
    // immediate `Err`, not a hang).
    // -----------------------------------------------------------------

    #[derive(Default)]
    struct NeverRespondingSession;

    #[async_trait(?Send)]
    impl CatSession for NeverRespondingSession {
        type Error = TransportError;

        async fn execute(
            &mut self,
            _request: &[u8],
            _response: &mut Vec<u8>,
        ) -> Result<ResponseDisposition, TransportError> {
            pending::<()>().await;
            unreachable!("pending() never resolves");
        }
    }

    // -------------------------------------------------------------------
    // Happy path
    // -------------------------------------------------------------------

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn happy_path_query_returns_response_text() {
        let mut broker = broker_with_script([Exchange::new("FA;", "FA00014250000;")]);

        let outcome = broker.dispatch(b"FA;").await.unwrap();

        assert_eq!(
            outcome,
            DispatchOutcome::Response("FA00014250000;".to_string())
        );
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn happy_path_query_with_selector_param() {
        let mut broker = broker_with_script([Exchange::new("SM0;", "SM0015;")]);

        let outcome = broker.dispatch(b"SM0;").await.unwrap();

        assert_eq!(outcome, DispatchOutcome::Response("SM0015;".to_string()));
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn selector_read_on_a_readable_and_writable_command_returns_the_response() {
        // The bug this reproduces: before `is_selector_read` existed, `MD`
        // being `writable: true` meant this 1-byte selector-read request
        // was routed to `client.set()` (fire-and-forget), silently
        // discarding the real response instead of returning it.
        let mut broker = broker_with_script([Exchange::new("MD0;", "MD02;")]);

        let outcome = broker.dispatch(b"MD0;").await.unwrap();

        assert_eq!(outcome, DispatchOutcome::Response("MD02;".to_string()));
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn wider_set_form_on_the_same_command_still_writes() {
        // The 2-byte write form on the *same* command must still route to
        // `client.set()` — `is_selector_read` is per-form, not per-command.
        let mut broker = broker_with_script([Exchange::new("MD02;", "")]);

        let outcome = broker.dispatch(b"MD02;").await.unwrap();

        assert_eq!(outcome, DispatchOutcome::NoResponse);
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn happy_path_set_yields_no_response() {
        let mut broker = broker_with_script([Exchange::new("FA00014250000;", "")]);

        let outcome = broker.dispatch(b"FA00014250000;").await.unwrap();

        assert_eq!(outcome, DispatchOutcome::NoResponse);
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn happy_path_action_yields_no_response() {
        let mut broker = broker_with_script([Exchange::new("TX;", "")]);

        let outcome = broker.dispatch(b"TX;").await.unwrap();

        assert_eq!(outcome, DispatchOutcome::NoResponse);
    }

    // -------------------------------------------------------------------
    // Malformed-request rejection (never touches the physical session)
    // -------------------------------------------------------------------

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn malformed_unknown_command_never_touches_session() {
        let mut broker = broker_with_script([]);

        let result = broker.dispatch(b"ZZ;").await;

        assert!(matches!(result, Err(DispatchError::Malformed(_))));
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn malformed_missing_terminator_never_touches_session() {
        let mut broker = broker_with_script([]);

        let result = broker.dispatch(b"FA").await;

        assert!(matches!(
            result,
            Err(DispatchError::Malformed(ParseError::MissingTerminator))
        ));
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn malformed_wrong_param_width_never_touches_session() {
        let mut broker = broker_with_script([]);

        // FA's only set form is width 11; this is width 3.
        let result = broker.dispatch(b"FA123;").await;

        assert!(matches!(
            result,
            Err(DispatchError::Malformed(
                ParseError::InvalidParameterWidth { .. }
            ))
        ));
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn malformed_non_utf8_never_touches_session() {
        let mut broker = broker_with_script([]);

        let result = broker.dispatch(&[0xFF, 0xFE, b';']).await;

        assert!(matches!(result, Err(DispatchError::InvalidEncoding)));
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn malformed_request_leaves_script_untouched_for_next_valid_request() {
        // The real proof that malformed rejection happens *before* the
        // physical session is touched: the script still has its one
        // exchange available afterward.
        let mut broker = broker_with_script([Exchange::new("FA;", "FA00014250000;")]);

        let malformed = broker.dispatch(b"ZZ;").await;
        assert!(matches!(malformed, Err(DispatchError::Malformed(_))));

        let happy = broker.dispatch(b"FA;").await.unwrap();
        assert_eq!(
            happy,
            DispatchOutcome::Response("FA00014250000;".to_string())
        );
    }

    // -------------------------------------------------------------------
    // Timeout: the physical radio never answers
    // -------------------------------------------------------------------

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn never_answered_request_times_out_instead_of_hanging() {
        let mut broker =
            Broker::with_timeout(NeverRespondingSession, &TABLE, Duration::from_millis(50));

        let started = std::time::Instant::now();
        let result = broker.dispatch(b"FA;").await;
        let elapsed = started.elapsed();

        assert!(matches!(result, Err(DispatchError::Timeout(_))));
        assert!(elapsed >= Duration::from_millis(50));
        assert!(
            elapsed < Duration::from_millis(500),
            "looked like a hang: {:?}",
            elapsed
        );
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn worker_recovers_after_a_timeout_and_services_the_next_request() {
        // Prove the timeout doesn't wedge the broker itself: build a
        // *separate* broker with a scripted session for the second
        // request (a real deployment would keep using the same physical
        // session across a timeout, but this demonstrates the important
        // property -- dispatch() returns control promptly and the broker
        // object remains fully usable for a subsequent call).
        let mut never =
            Broker::with_timeout(NeverRespondingSession, &TABLE, Duration::from_millis(30));
        let timed_out = never.dispatch(b"FA;").await;
        assert!(matches!(timed_out, Err(DispatchError::Timeout(_))));

        let mut recovered = broker_with_script([Exchange::new("FA;", "FA00014250000;")]);
        let outcome = recovered.dispatch(b"FA;").await.unwrap();
        assert_eq!(
            outcome,
            DispatchOutcome::Response("FA00014250000;".to_string())
        );
    }

    // -------------------------------------------------------------------
    // Physical session/transport error surfaces cleanly (not a panic/hang)
    // -------------------------------------------------------------------

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn physical_session_transport_error_surfaces_as_session_error() {
        let mut session = ScriptedCatSession::new();
        session.simulate_disconnect();
        let mut broker = Broker::new(session, &TABLE);

        let result = broker.dispatch(b"FA;").await;

        assert!(matches!(result, Err(DispatchError::Session(_))));
    }

    // -------------------------------------------------------------------
    // outcome_to_wire: the wire-level error-signaling convention
    // -------------------------------------------------------------------

    #[test]
    fn outcome_to_wire_response_is_the_response_text() {
        let result: Result<DispatchOutcome, DispatchError<TransportError>> =
            Ok(DispatchOutcome::Response("FA00014250000;".to_string()));
        assert_eq!(outcome_to_wire(result), b"FA00014250000;".to_vec());
    }

    #[test]
    fn outcome_to_wire_no_response_is_empty() {
        let result: Result<DispatchOutcome, DispatchError<TransportError>> =
            Ok(DispatchOutcome::NoResponse);
        assert!(outcome_to_wire(result).is_empty());
    }

    #[test]
    fn outcome_to_wire_error_is_non_empty_and_not_cat_shaped() {
        let result: Result<DispatchOutcome, DispatchError<TransportError>> =
            Err(DispatchError::Malformed(ParseError::MissingTerminator));
        let wire = outcome_to_wire(result);
        assert!(wire.starts_with(b"ERR "));
        assert!(!wire.ends_with(b";"));
    }

    // -------------------------------------------------------------------
    // BrokerWorker/BrokerHandle: single ordered worker, correlation,
    // disconnect handling.
    // -------------------------------------------------------------------

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn worker_happy_path_round_trip_through_handle() {
        let (worker, handle) = build(
            ScriptedCatSession::with_script([Exchange::new("FA;", "FA00014250000;")]),
            &TABLE,
        );
        monoio::spawn(worker.run());

        let response = handle.submit(ClientId::from_raw(1), b"FA;".to_vec()).await;
        assert_eq!(response, Some(b"FA00014250000;".to_vec()));
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn worker_malformed_request_gets_error_wire_response() {
        let (worker, handle) = build(ScriptedCatSession::with_script([]), &TABLE);
        monoio::spawn(worker.run());

        let response = handle
            .submit(ClientId::from_raw(1), b"ZZ;".to_vec())
            .await
            .unwrap();
        assert!(response.starts_with(b"ERR "));
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn worker_serializes_concurrent_requests_from_multiple_clients_correctly() {
        // Two "clients" submit concurrently; the scripted session only
        // accepts requests in a fixed order, so this also proves the
        // single ordered worker serializes access to the physical session
        // (a scripted session panics on an out-of-order/mismatched
        // request) *and* that each client's response is routed back to the
        // right caller, not cross-wired.
        let (worker, handle) = build(
            ScriptedCatSession::with_script([
                Exchange::new("FA;", "FA00014250000;"),
                Exchange::new("IF;", "IF017;"),
            ]),
            &TABLE,
        );
        monoio::spawn(worker.run());

        let handle_a = handle.clone();
        let handle_b = handle.clone();

        let task_a = monoio::spawn(async move {
            handle_a
                .submit(ClientId::from_raw(1), b"FA;".to_vec())
                .await
        });
        let task_b = monoio::spawn(async move {
            handle_b
                .submit(ClientId::from_raw(2), b"IF;".to_vec())
                .await
        });

        let response_a = task_a.await.unwrap();
        let response_b = task_b.await.unwrap();

        assert_eq!(response_a, b"FA00014250000;".to_vec());
        assert_eq!(response_b, b"IF017;".to_vec());
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn disconnect_before_reply_does_not_wedge_the_worker() {
        let (worker, handle) = build(
            ScriptedCatSession::with_script([
                Exchange::new("FA;", "FA00014250000;"),
                Exchange::new("IF;", "IF017;"),
            ]),
            &TABLE,
        );
        monoio::spawn(worker.run());

        // Simulate client 1 disconnecting mid-request: push a job directly
        // (bypassing `submit()`'s await) and drop its `OneshotReceiver`
        // immediately -- exactly what happens when a client-facing
        // connection task is dropped after sending a request but before
        // its reply arrives.
        let (reply_tx, reply_rx) = local_channel::oneshot();
        let send_result = handle.sender.send(Job {
            client_id: ClientId::from_raw(1),
            request_id: 0,
            payload: b"FA;".to_vec(),
            reply: reply_tx,
        });
        assert!(send_result.is_ok(), "worker should still be alive");
        drop(reply_rx);

        // The worker processes jobs strictly in order, so successfully
        // awaiting client 2's reply proves the abandoned job ahead of it
        // was serviced (or at least attempted) without wedging anything.
        let response = handle.submit(ClientId::from_raw(2), b"IF;".to_vec()).await;
        assert_eq!(response, Some(b"IF017;".to_vec()));
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn worker_shuts_down_once_every_handle_is_dropped() {
        let (worker, handle) = build(ScriptedCatSession::with_script([]), &TABLE);
        let worker_task = monoio::spawn(worker.run());

        drop(handle);
        // `run()` should return once the channel's sole sender is gone and
        // the queue is empty -- proving a clean shutdown, not a hang.
        worker_task.await;
    }
}
