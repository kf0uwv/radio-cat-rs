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

//! Windows analog of `crate::broker`'s `Job`/`BrokerWorker`/`BrokerHandle`/
//! `build`/`build_with_timeout` -- the single ordered worker's queue and
//! handle, rebuilt on genuine OS threads instead of cooperative `monoio`
//! tasks.
//!
//! Per `docs/adr/0006-windows-network-transport.md`: `crate::local_channel`
//! (what `crate::broker` uses) is `Rc`/`RefCell`-based -- sound only when
//! every user of a shared channel runs on the *same* OS thread, which
//! `monoio`'s thread-per-core, cooperative-task model guarantees. Windows
//! has no such cooperative-task model available (ADR 0002 forbids adding a
//! second general-purpose executor crate), so this repository's Windows
//! concurrency substrate for `cat-server` is genuine OS threads: one
//! dedicated thread per accepted TCP connection (`crate::tcp_windows`) or
//! per received UDP datagram (`crate::udp_windows`), all funneling `Job`s
//! into this module's worker over a real `Send` channel
//! (`std::sync::mpsc`), with replies delivered back via
//! `cat_transport_core::completion` (also genuinely `Send`-capable, unlike
//! `local_channel`'s oneshot).
//!
//! [`Broker<C, S>`](crate::broker::Broker) itself (validation, dispatch,
//! timeout-wrapping) is fully shared with the Linux backend -- nothing here
//! duplicates it. Only the *channel that feeds it* differs.
//!
//! `BrokerWorker::run` is a **plain blocking `fn`, not `async fn`**, unlike
//! the Linux backend: the single ordered worker's whole point is to process
//! jobs strictly one at a time, so there is no concurrency to preserve by
//! keeping it cooperative, and a plain blocking loop (using
//! `crate::block_on` to drive each `Broker::dispatch` call, which is itself
//! `async fn` because `CatSession` is) is simpler than forcing an `async
//! fn` shape that would need its own bespoke driving anyway. This is the
//! one place this backend's public shape is not letter-for-letter identical
//! to the Linux backend's `BrokerWorker::run` (`pub async fn` there) --
//! consistent with `docs/adr/0004-windows-serial-backend.md`'s own
//! precedent that the top-level "how do you start this" bootstrapping is
//! expected to differ per platform (each consuming application already
//! needs its own Windows entry point regardless, since `#[monoio::main]`
//! cannot exist there). [`BrokerHandle::submit`] -- the actual per-request
//! hot path a client-facing listener calls -- and [`BrokerHandle::submit_task`]
//! stay `pub async fn` with signatures **identical** to the Linux backend.
//!
//! # Test scope: Windows-only, unlike this crate's transport-level siblings
//!
//! This module's production code has no actual Windows-specific syscalls
//! (plain `std::sync::mpsc`/`std::sync::atomic` + the portable completion
//! primitive), which might suggest its tests could run on every platform,
//! mirroring `cat-transport-tcp::windows`/`cat-transport-udp::windows`.
//! They cannot: every test here drives `crate::broker::Broker::dispatch`,
//! whose internal timeout wrap is `monoio::time::timeout` on a Linux
//! build (a correctness requirement, not a style choice -- see
//! `crate::broker::with_request_timeout`'s doc comment for the full
//! explanation, discovered by this crate's own test suite hanging). Driving
//! that through this module's `crate::block_on` instead of a real `monoio`
//! runtime is exactly the unsafe combination that doc comment warns about,
//! so this module's tests are gated `#[cfg(all(test, target_os =
//! "windows"))]` -- type-checked via the Windows cross-compile check, not
//! executed, matching `docs/adr/0004-windows-serial-backend.md`'s original
//! precedent for genuinely platform-divergent code.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use cat_framework::{AsciiLineFormat, CatWireFormat, CommandId};
use cat_transport_core::{completion, CatSession};

use crate::block_on::block_on;
use crate::broker::{outcome_to_wire, run_task, Broker, TaskFn, Work, DEFAULT_REQUEST_TIMEOUT};
use crate::registry::ClientId;
use crate::release::{ReleaseLines, ReleaseLinesCloser};

/// One unit of work submitted to a [`BrokerWorker`]. Same fields/purpose as
/// `crate::broker::Job`; `reply` is a [`completion::CompletionTx`] instead
/// of `crate::local_channel::OneshotSender` since it must cross a genuine
/// OS thread boundary.
pub struct Job {
    /// Which registered client this request came from.
    pub client_id: ClientId,
    /// Monotonically-assigned id for this request, for tracking/logging.
    pub request_id: u64,
    /// What the worker should do with the session -- the same shared
    /// [`Work`] the Linux backend's `Job` carries.
    pub work: Work,
    reply: completion::CompletionTx<Vec<u8>>,
}

/// The single ordered worker. Owns a [`Broker`] exclusively and services
/// [`Job`]s strictly in the order they were submitted.
///
/// Callers run `worker.run()` on a dedicated `std::thread` (there is no
/// `monoio::spawn` on Windows) and distribute [`BrokerHandle`] clones to
/// every client-facing thread that needs to submit work.
pub struct BrokerWorker<C, S, F = AsciiLineFormat>
where
    C: CommandId,
    S: CatSession,
    S::Error: std::error::Error + 'static,
    F: CatWireFormat,
{
    broker: Broker<C, S, F>,
    receiver: mpsc::Receiver<Job>,
    /// Closes the release lines (ADR 0022) when the worker is dropped.
    _release: ReleaseLinesCloser,
}

impl<C, S, F> BrokerWorker<C, S, F>
where
    C: CommandId,
    S: CatSession,
    S::Error: std::error::Error + 'static,
    F: CatWireFormat + Clone,
{
    /// Run until every [`BrokerHandle`] (and clone) has been dropped and the
    /// job queue is drained. Blocking -- run this on a dedicated thread.
    pub fn run(mut self) {
        while let Ok(job) = self.receiver.recv() {
            let wire = match job.work {
                Work::Wire(payload) => outcome_to_wire(block_on(self.broker.dispatch(&payload))),
                // Runs here, on the worker thread, between whatever wire
                // jobs precede and follow it in the queue -- the one
                // service the broker sells a task. The modem-line borrow is
                // made from the session this thread owns, for the duration
                // of the call only (see `TaskFn`).
                Work::Task(run) => run_task(run, self.broker.session_modem_lines()),
            };
            // A disconnected requester (dropped its `CompletionRx` before
            // the reply was ready) is not an error here, mirroring
            // `crate::broker::BrokerWorker::run`'s identical contract.
            job.reply.send(wire);
        }
    }
}

/// Handle used by client-facing threads to submit work to a [`BrokerWorker`]
/// and await their own reply. Cheaply `Clone`-able and genuinely `Send` +
/// `Sync` (`std::sync::mpsc::Sender` + `Arc<AtomicU64>`) -- unlike
/// `crate::broker::BrokerHandle` (`Rc`-based, single-thread only), this one
/// is designed to be moved into a freshly spawned `std::thread` per
/// connection/datagram.
#[derive(Clone)]
pub struct BrokerHandle {
    sender: mpsc::Sender<Job>,
    next_request_id: Arc<AtomicU64>,
    release: ReleaseLines,
}

impl BrokerHandle {
    /// Submit one raw wire-format request from `client_id` and await the
    /// raw wire-format response bytes.
    ///
    /// `pub async fn` with an identical signature to
    /// `crate::broker::BrokerHandle::submit` -- callers written against
    /// "submit and `.await` the reply" work unchanged on either platform.
    /// Returns `None` only if the [`BrokerWorker`] itself has shut down.
    pub async fn submit(&self, client_id: ClientId, payload: Vec<u8>) -> Option<Vec<u8>> {
        self.enqueue(client_id, Work::Wire(payload)).await
    }

    /// Submit a caller-supplied task to run with exclusive access to the wire.
    ///
    /// Identical signature and semantics to
    /// `crate::broker::BrokerHandle::submit_task`. The closure is handed the
    /// session's modem-control lines (`None` if the transport has none) and
    /// runs on the worker thread, in queue order, so it cannot interleave
    /// with a CAT exchange. A failing task's reply uses the `b"ERR "`
    /// convention. Returns `None` only if the [`BrokerWorker`] has shut down.
    ///
    /// Use for **asserting** a PTT line, where ordering against CAT matters:
    /// do not key before the command that set the mode has landed. Do **not**
    /// route the *release* through here -- an un-key must not queue behind a
    /// CAT exchange that may take seconds, and a line change cannot corrupt a
    /// frame in flight because `EscapeCommFunction` never touches the byte
    /// stream.
    /// Release with [`Self::release_now_bypassing_queue`].
    pub async fn submit_task(&self, client_id: ClientId, run: TaskFn) -> Option<Vec<u8>> {
        self.enqueue(client_id, Work::Task(run)).await
    }

    /// **De-key now**, skipping the ordered queue. Identical signature and
    /// semantics to `crate::broker::BrokerHandle::release_now_bypassing_queue`
    /// (read its doc): runs `run` immediately on the calling thread against
    /// the broker's owned duplicate of the session's modem lines, even while
    /// the worker thread is mid-exchange. **Only de-keying / safe-state
    /// actions may use it.** With no release lines it falls back to the
    /// ordered queue and warns once. `None` only once the broker is gone.
    pub async fn release_now_bypassing_queue(
        &self,
        client_id: ClientId,
        run: TaskFn,
    ) -> Option<Vec<u8>> {
        match self.release.try_now(run) {
            Ok(reply) => Some(reply),
            Err(run) => self.submit_task(client_id, run).await,
        }
    }

    /// Tag `work` with a request id, put it on the single ordered queue, and
    /// await its reply.
    async fn enqueue(&self, client_id: ClientId, work: Work) -> Option<Vec<u8>> {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (reply, rx) = completion::channel();
        let job = Job {
            client_id,
            request_id,
            work,
            reply,
        };
        self.sender.send(job).ok()?;
        rx.await.ok()
    }
}

// The property `TaskFn: Send` exists to protect: a `BrokerHandle` carrying
// task jobs must still be movable into the per-connection OS threads
// `tcp_windows`/`udp_windows`/`cat-rigctl` spawn. Checked on every platform,
// since this module always compiles.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<BrokerHandle>();
};

/// Build a [`BrokerWorker`]/[`BrokerHandle`] pair around `session`, using
/// [`DEFAULT_REQUEST_TIMEOUT`](crate::broker::DEFAULT_REQUEST_TIMEOUT).
pub fn build<C, S, F>(
    session: S,
    table: &'static cat_framework::CommandTable<C, F>,
) -> (BrokerWorker<C, S, F>, BrokerHandle)
where
    C: CommandId,
    S: CatSession,
    S::Error: std::error::Error + 'static,
    F: CatWireFormat + Default + Clone,
{
    build_with_timeout(session, table, DEFAULT_REQUEST_TIMEOUT)
}

/// Like [`build`], with an explicit per-request timeout.
pub fn build_with_timeout<C, S, F>(
    session: S,
    table: &'static cat_framework::CommandTable<C, F>,
    request_timeout: Duration,
) -> (BrokerWorker<C, S, F>, BrokerHandle)
where
    C: CommandId,
    S: CatSession,
    S::Error: std::error::Error + 'static,
    F: CatWireFormat + Default + Clone,
{
    // Before the session moves into the broker: afterwards it is reachable
    // only through the worker thread's exclusive borrow (ADR 0022).
    let release = ReleaseLines::from_session(&session);
    let broker = Broker::with_timeout(session, table, request_timeout);
    let (sender, receiver) = mpsc::channel();
    let worker = BrokerWorker {
        broker,
        receiver,
        _release: release.closer(),
    };
    let handle = BrokerHandle {
        sender,
        next_request_id: Arc::new(AtomicU64::new(0)),
        release,
    };
    (worker, handle)
}

// Gated `target_os = "windows"` in addition to `test`, despite this file's
// own production code having no actual Windows-specific syscalls: every
// test below drives `crate::broker::Broker::dispatch` (via `build`/
// `BrokerWorker::run`), and `dispatch`'s internal per-request timeout wrap
// (`crate::broker::with_request_timeout`) is deliberately `monoio::time::
// timeout` on a Linux build (see that function's own doc comment for why
// this is a correctness requirement, not a style choice) -- which is not
// safe to drive through this module's `crate::block_on` (no `monoio`
// runtime at all). Running these tests on a Linux build would exercise
// exactly that unsafe combination. `cat-transport-tcp::windows`/
// `cat-transport-udp::windows` do NOT have this restriction (their tests
// run on every platform) because they never touch `Broker::dispatch` at
// all -- this module's restriction is specific to `cat-server`'s shared
// dispatch/timeout coupling, not a general property of "Windows-shaped
// backend modules" in this workspace.
#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use crate::test_fixtures::{FakeCommand, TABLE};
    use cat_transport_core::test_support::{Exchange, ScriptedCatSession};

    fn broker_with_script<I: IntoIterator<Item = Exchange>>(
        script: I,
    ) -> (BrokerWorker<FakeCommand, ScriptedCatSession>, BrokerHandle) {
        build(ScriptedCatSession::with_script(script), &TABLE)
    }

    /// As [`broker_with_script`], but the session matches requests by
    /// content instead of position -- for the tests that submit from
    /// several threads at once, where nothing orders arrival.
    fn broker_with_unordered_script<I: IntoIterator<Item = Exchange>>(
        script: I,
    ) -> (BrokerWorker<FakeCommand, ScriptedCatSession>, BrokerHandle) {
        build(ScriptedCatSession::with_unordered_script(script), &TABLE)
    }

    #[test]
    fn worker_happy_path_round_trip_through_handle() {
        let (worker, handle) = broker_with_script([Exchange::new("FA;", "FA00014250000;")]);
        std::thread::spawn(move || worker.run());

        let response = block_on(handle.submit(ClientId::from_raw(1), b"FA;".to_vec()));
        assert_eq!(response, Some(b"FA00014250000;".to_vec()));
    }

    // -----------------------------------------------------------------
    // Task jobs -- mirrors `crate::broker`'s four task-job tests.
    // -----------------------------------------------------------------

    #[test]
    fn a_task_job_runs_on_the_worker_and_returns_its_own_reply() {
        let (worker, handle) = broker_with_script([]);
        std::thread::spawn(move || worker.run());

        let out = block_on(handle.submit_task(
            ClientId::from_raw(7),
            Box::new(|_lines| Ok(b"TASK-RAN".to_vec())),
        ));
        assert_eq!(out.as_deref(), Some(&b"TASK-RAN"[..]));
    }

    #[test]
    fn a_task_runs_on_the_worker_thread_not_the_submitter() {
        // Windows-specific: the closure must cross to the worker's OS
        // thread, which is what makes it serialised against CAT traffic.
        let (worker, handle) = broker_with_script([]);
        let worker_thread = std::thread::spawn(move || worker.run());
        let worker_id = worker_thread.thread().id();

        let ran_on = std::sync::Arc::new(std::sync::Mutex::new(None));
        let slot = std::sync::Arc::clone(&ran_on);
        block_on(handle.submit_task(
            ClientId::from_raw(1),
            Box::new(move |_| {
                *slot.lock().unwrap() = Some(std::thread::current().id());
                Ok(Vec::new())
            }),
        ))
        .expect("worker alive");

        assert_eq!(*ran_on.lock().unwrap(), Some(worker_id));
        assert_ne!(Some(std::thread::current().id()), Some(worker_id));
    }

    #[test]
    fn a_task_is_serialised_between_wire_jobs() {
        // The one service the broker provides a task: it cannot interleave
        // with a CAT exchange on the same wire.
        let (worker, handle) = broker_with_script([
            Exchange::new("FA;", "FA00014250000;"),
            Exchange::new("FA;", "FA00014250000;"),
        ]);
        std::thread::spawn(move || worker.run());

        let first = block_on(handle.submit(ClientId::from_raw(1), b"FA;".to_vec()));
        let task =
            block_on(handle.submit_task(ClientId::from_raw(1), Box::new(|_| Ok(b"MID".to_vec()))));
        let second = block_on(handle.submit(ClientId::from_raw(1), b"FA;".to_vec()));

        assert_eq!(first.as_deref(), Some(&b"FA00014250000;"[..]));
        assert_eq!(task.as_deref(), Some(&b"MID"[..]));
        assert_eq!(second.as_deref(), Some(&b"FA00014250000;"[..]));
    }

    #[test]
    fn a_failing_task_uses_the_err_convention() {
        // A caller must not be able to misparse a task failure as a CAT frame.
        let (worker, handle) = broker_with_script([]);
        std::thread::spawn(move || worker.run());

        let out = block_on(handle.submit_task(
            ClientId::from_raw(1),
            Box::new(|_| Err("line control unavailable".to_string())),
        ))
        .expect("worker alive");
        assert!(out.starts_with(b"ERR "), "got {out:?}");
    }

    #[test]
    fn a_task_is_told_when_the_session_has_no_modem_lines() {
        // A socket session has no RTS/DTR. The task must be told `None`
        // rather than handed something that cannot work.
        let (worker, handle) = broker_with_script([]);
        std::thread::spawn(move || worker.run());

        let out = block_on(handle.submit_task(
            ClientId::from_raw(1),
            Box::new(|lines| {
                Ok(if lines.is_some() {
                    b"SOME".to_vec()
                } else {
                    b"NONE".to_vec()
                })
            }),
        ));
        assert_eq!(out.as_deref(), Some(&b"NONE"[..]));
    }

    #[test]
    fn a_task_submitted_after_shutdown_reports_none() {
        let (worker, handle) = broker_with_script([]);
        drop(worker);
        let out = block_on(handle.submit_task(ClientId::from_raw(1), Box::new(|_| Ok(Vec::new()))));
        assert_eq!(out, None);
    }

    #[test]
    fn worker_malformed_request_gets_error_wire_response() {
        let (worker, handle) = broker_with_script([]);
        std::thread::spawn(move || worker.run());

        let response = block_on(handle.submit(ClientId::from_raw(1), b"ZZ;".to_vec())).unwrap();
        assert!(response.starts_with(b"ERR "));
    }

    #[test]
    fn worker_serializes_requests_from_multiple_handles_correctly() {
        // Two client threads submit concurrently. This proves each
        // caller's response is routed back to that caller and not
        // cross-wired, and that the single ordered worker serializes
        // access to the physical session.
        //
        // The session matches by request content, NOT by position. An
        // ordered script here made the test a coin flip on thread
        // scheduling -- whichever thread reached the worker first was
        // compared against whichever exchange sat at the head of the
        // queue -- and it failed ~3 runs in 8 on Windows the first time
        // these tests were ever executed there (see `radio-cat-rs`
        // planning/release_workflow/findings.md section 7c). Serializing
        // access is what the worker guarantees; choosing an arrival order
        // for two independent client threads is not, and asserting on one
        // tested the scheduler rather than the code.
        let (worker, handle) = broker_with_unordered_script([
            Exchange::new("FA;", "FA00014250000;"),
            Exchange::new("IF;", "IF017;"),
        ]);
        std::thread::spawn(move || worker.run());

        let handle_a = handle.clone();
        let handle_b = handle.clone();

        let task_a = std::thread::spawn(move || {
            block_on(handle_a.submit(ClientId::from_raw(1), b"FA;".to_vec()))
        });
        let task_b = std::thread::spawn(move || {
            block_on(handle_b.submit(ClientId::from_raw(2), b"IF;".to_vec()))
        });

        let response_a = task_a.join().unwrap();
        let response_b = task_b.join().unwrap();

        assert_eq!(response_a, Some(b"FA00014250000;".to_vec()));
        assert_eq!(response_b, Some(b"IF017;".to_vec()));
    }

    #[test]
    fn disconnect_before_reply_does_not_wedge_the_worker() {
        let (worker, handle) = broker_with_script([
            Exchange::new("FA;", "FA00014250000;"),
            Exchange::new("IF;", "IF017;"),
        ]);
        std::thread::spawn(move || worker.run());

        // Simulate client 1 disconnecting mid-request: push a job directly
        // and drop its `CompletionRx` immediately.
        let (reply, rx) = completion::channel();
        handle
            .sender
            .send(Job {
                client_id: ClientId::from_raw(1),
                request_id: 0,
                work: Work::Wire(b"FA;".to_vec()),
                reply,
            })
            .expect("worker should still be alive");
        drop(rx);

        let response = block_on(handle.submit(ClientId::from_raw(2), b"IF;".to_vec()));
        assert_eq!(response, Some(b"IF017;".to_vec()));
    }

    #[test]
    fn worker_shuts_down_once_every_handle_is_dropped() {
        let (worker, handle) = broker_with_script([]);
        let worker_thread = std::thread::spawn(move || worker.run());

        drop(handle);
        // `run()` should return once the channel's sole sender is gone and
        // the queue is empty -- proving a clean shutdown, not a hang.
        worker_thread
            .join()
            .expect("worker thread should exit cleanly");
    }
}
