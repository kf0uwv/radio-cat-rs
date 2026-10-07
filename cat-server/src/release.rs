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

//! The broker's release lines: an owned duplicate of the session's modem
//! lines, used only to de-key without waiting for the queue (ADR 0022).
//!
//! One definition, shared by the Linux (`crate::broker`) and Windows
//! (`crate::worker_windows`) backends so the bypass cannot behave
//! differently on the two platforms.
//!
//! Why it exists: while a CAT exchange is in flight the worker holds an
//! exclusive borrow of the session until the read returns or times out
//! (2 s read, 5 s broker). Nothing can borrow the session's lines in that
//! window, so a PTT release routed through the worker waits that long while
//! the radio transmits. The duplicate is taken once, in `build`, before the
//! session moves into the broker, and is reachable without that borrow.
//!
//! Lifetime: the worker holds a [`ReleaseLinesCloser`]. When the worker is
//! dropped, the duplicate is dropped (closed), so it never outlives the
//! broker. A release holds the same lock while it runs, so a close cannot
//! overlap a release -- use-after-close is not representable.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use cat_transport_core::{CatSession, ModemControlLines};

use crate::broker::{run_task, TaskFn};

type Lines = Box<dyn ModemControlLines + Send + Sync>;

/// A handle's view of the broker's release lines. Cheap to clone; `Send +
/// Sync` so the Windows `BrokerHandle` stays movable across threads.
#[derive(Clone)]
pub(crate) struct ReleaseLines {
    inner: Arc<Inner>,
}

struct Inner {
    /// `None` once the broker has closed them, or if there never were any.
    lines: Mutex<Option<Lines>>,
    /// Whether the session offered release lines at all. A broker that
    /// had them and closed them at shutdown is not misconfigured, so only
    /// "never had any" earns the fallback warning.
    had_lines: bool,
    warned: AtomicBool,
}

impl ReleaseLines {
    /// Take the session's release lines, once, before it moves into the
    /// broker.
    pub(crate) fn from_session<S: CatSession>(session: &S) -> Self {
        let lines = session.release_lines();
        Self {
            inner: Arc::new(Inner {
                had_lines: lines.is_some(),
                lines: Mutex::new(lines),
                warned: AtomicBool::new(false),
            }),
        }
    }

    /// The closer the worker holds.
    pub(crate) fn closer(&self) -> ReleaseLinesCloser {
        ReleaseLinesCloser {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Run `run` now, on the calling thread, against the release lines.
    ///
    /// `Err(run)` hands the task back when there are no release lines --
    /// the caller then falls back to the ordered queue. The first such
    /// fallback on a broker that never had release lines logs a warning.
    pub(crate) fn try_now(&self, run: TaskFn) -> Result<Vec<u8>, TaskFn> {
        let guard = lock(&self.inner.lines);
        match guard.as_deref() {
            Some(lines) => Ok(run_task(run, Some(lines as &dyn ModemControlLines))),
            None => {
                drop(guard);
                if self.should_warn_of_fallback() {
                    tracing::warn!(
                        "PTT release: this session has no release lines (ADR 0022), so the \
                         release is going through the broker's ordered queue and may wait \
                         behind a CAT exchange"
                    );
                }
                Err(run)
            }
        }
    }

    /// True exactly once per broker, and only if it never had release
    /// lines.
    fn should_warn_of_fallback(&self) -> bool {
        !self.inner.had_lines && !self.inner.warned.swap(true, Ordering::SeqCst)
    }
}

/// Held by the worker; dropping it closes the duplicate.
pub(crate) struct ReleaseLinesCloser {
    inner: Arc<Inner>,
}

impl Drop for ReleaseLinesCloser {
    fn drop(&mut self) {
        // Takes the lock, so this waits for any release in progress.
        lock(&self.inner.lines).take();
    }
}

/// A task that panicked while holding the lock leaves nothing half-done in
/// the `Option` itself, so poisoning is not a reason to stop releasing.
fn lock(lines: &Mutex<Option<Lines>>) -> MutexGuard<'_, Option<Lines>> {
    lines
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::test_fixtures::{logged, StallingSession};

    #[test]
    fn a_release_runs_on_the_duplicate() {
        let session = StallingSession::new(true);
        let log = session.log();
        let release = ReleaseLines::from_session(&session);

        let out = release
            .try_now(Box::new(|lines| {
                lines.expect("release lines").set_dtr(false).unwrap();
                Ok(b"OFF".to_vec())
            }))
            .unwrap_or_else(|_| panic!("should run now"));
        assert_eq!(out, b"OFF");
        assert_eq!(logged(&log), vec!["release dtr=false"]);
    }

    #[test]
    fn without_release_lines_the_task_is_handed_back_and_warned_about_once() {
        let release = ReleaseLines::from_session(&StallingSession::new(false));
        assert!(release.try_now(Box::new(|_| Ok(Vec::new()))).is_err());
        assert!(release.inner.warned.load(Ordering::SeqCst));
        assert!(!release.should_warn_of_fallback(), "only once");
    }

    #[test]
    fn dropping_the_closer_closes_the_duplicate_and_does_not_warn() {
        let session = StallingSession::new(true);
        let drops = session.release_drops();
        let release = ReleaseLines::from_session(&session);
        let closer = release.closer();

        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(closer);
        assert_eq!(drops.load(Ordering::SeqCst), 1, "duplicate closed");
        assert!(release.try_now(Box::new(|_| Ok(Vec::new()))).is_err());
        assert!(
            !release.inner.warned.load(Ordering::SeqCst),
            "a closed broker is not a misconfigured one"
        );
    }

    #[test]
    fn a_failing_release_uses_the_err_convention() {
        let release = ReleaseLines::from_session(&StallingSession::new(true));
        let out = release
            .try_now(Box::new(|_| Err("line control unavailable".to_string())))
            .unwrap_or_else(|_| panic!("should run now"));
        assert!(out.starts_with(b"ERR "));
    }
}

/// The broker-level guarantees, on Linux's cooperative backend.
#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    use crate::broker::build_with_timeout;
    use crate::registry::ClientId;
    use crate::test_fixtures::{logged, EventLog, StallingSession, TABLE};

    const STALL: Duration = Duration::from_millis(400);
    const PROMPT: Duration = Duration::from_millis(100);

    async fn until_logged(log: &EventLog, entry: &str) {
        for _ in 0..1000 {
            if logged(log).iter().any(|e| e == entry) {
                return;
            }
            monoio::time::sleep(Duration::from_millis(1)).await;
        }
        panic!("never logged {entry:?}: {:?}", logged(log));
    }

    fn dtr(asserted: bool) -> crate::TaskFn {
        Box::new(move |lines| {
            lines
                .ok_or("no lines")?
                .set_dtr(asserted)
                .map_err(|e| e.to_string())?;
            Ok(if asserted {
                b"ON".to_vec()
            } else {
                b"OFF".to_vec()
            })
        })
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn a_release_does_not_wait_behind_a_stalled_exchange() {
        let session = StallingSession::new(true);
        let log = session.log();
        let (worker, handle) = build_with_timeout(session, &TABLE, STALL);
        monoio::spawn(worker.run());

        let stalled_done = Rc::new(Cell::new(false));
        let (h, done) = (handle.clone(), Rc::clone(&stalled_done));
        monoio::spawn(async move {
            let _ = h.submit(ClientId::from_raw(1), b"FA;".to_vec()).await;
            done.set(true);
        });
        until_logged(&log, "cat begin FA;").await;

        let started = Instant::now();
        let out = handle
            .release_now_bypassing_queue(ClientId::from_raw(2), dtr(false))
            .await;
        let took = started.elapsed();

        assert_eq!(out.as_deref(), Some(&b"OFF"[..]));
        assert!(took < PROMPT, "release took {took:?}");
        assert!(
            !stalled_done.get(),
            "the release must finish before the stalled exchange"
        );
        assert_eq!(
            logged(&log),
            vec!["cat begin FA;", "release dtr=false"],
            "released on the duplicate, mid-exchange"
        );

        // And the exchange really was stuck: it ends only at the timeout.
        while !stalled_done.get() {
            monoio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(
            logged(&log),
            vec!["cat begin FA;", "release dtr=false", "cat over FA;"]
        );
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn an_assert_still_waits_its_turn_behind_a_prior_exchange() {
        let session = StallingSession::new(true);
        let log = session.log();
        let (worker, handle) = build_with_timeout(session, &TABLE, STALL);
        monoio::spawn(worker.run());

        let stalled_done = Rc::new(Cell::new(false));
        let (h, done) = (handle.clone(), Rc::clone(&stalled_done));
        monoio::spawn(async move {
            let _ = h.submit(ClientId::from_raw(1), b"FA;".to_vec()).await;
            done.set(true);
        });
        until_logged(&log, "cat begin FA;").await;

        let out = handle.submit_task(ClientId::from_raw(2), dtr(true)).await;

        assert_eq!(out.as_deref(), Some(&b"ON"[..]));
        assert_eq!(
            logged(&log),
            vec!["cat begin FA;", "cat over FA;", "primary dtr=true"],
            "asserted on the session's own lines, after the prior exchange"
        );
        while !stalled_done.get() {
            monoio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn without_release_lines_a_release_falls_back_to_the_queue() {
        let session = StallingSession::new(false);
        let log = session.log();
        let (worker, handle) = build_with_timeout(session, &TABLE, STALL);
        monoio::spawn(worker.run());

        let out = handle
            .release_now_bypassing_queue(ClientId::from_raw(1), dtr(false))
            .await;
        assert_eq!(out.as_deref(), Some(&b"OFF"[..]));
        assert_eq!(logged(&log), vec!["primary dtr=false"]);
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn dropping_the_worker_closes_the_duplicate() {
        let session = StallingSession::new(true);
        let drops = session.release_drops();
        let (worker, handle) = build_with_timeout(session, &TABLE, STALL);

        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(worker);
        assert_eq!(drops.load(Ordering::SeqCst), 1, "no duplicate left open");

        // With the broker gone, a release reports so, like every submit.
        let out = handle
            .release_now_bypassing_queue(ClientId::from_raw(1), dtr(false))
            .await;
        assert_eq!(out, None);
    }
}

/// The same guarantees on the Windows backend: a real worker thread, with
/// the release made from another.
#[cfg(all(test, target_os = "windows"))]
mod windows_tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use crate::block_on::block_on;
    use crate::registry::ClientId;
    use crate::test_fixtures::{logged, EventLog, StallingSession, TABLE};
    use crate::worker_windows::build_with_timeout;

    const STALL: Duration = Duration::from_millis(400);
    const PROMPT: Duration = Duration::from_millis(100);

    fn until_logged(log: &EventLog, entry: &str) {
        for _ in 0..1000 {
            if logged(log).iter().any(|e| e == entry) {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("never logged {entry:?}: {:?}", logged(log));
    }

    fn dtr(asserted: bool) -> crate::TaskFn {
        Box::new(move |lines| {
            lines
                .ok_or("no lines")?
                .set_dtr(asserted)
                .map_err(|e| e.to_string())?;
            Ok(if asserted {
                b"ON".to_vec()
            } else {
                b"OFF".to_vec()
            })
        })
    }

    /// Submit the stalling `FA;` from its own thread; the flag flips when
    /// its reply (the timeout) arrives.
    fn stall(handle: &crate::worker_windows::BrokerHandle) -> Arc<AtomicBool> {
        let done = Arc::new(AtomicBool::new(false));
        let (h, d) = (handle.clone(), Arc::clone(&done));
        std::thread::spawn(move || {
            let _ = block_on(h.submit(ClientId::from_raw(1), b"FA;".to_vec()));
            d.store(true, Ordering::SeqCst);
        });
        done
    }

    #[test]
    fn a_release_does_not_wait_behind_a_stalled_exchange() {
        let session = StallingSession::new(true);
        let log = session.log();
        let (worker, handle) = build_with_timeout(session, &TABLE, STALL);
        std::thread::spawn(move || worker.run());

        let stalled_done = stall(&handle);
        until_logged(&log, "cat begin FA;");

        let started = Instant::now();
        let out = block_on(handle.release_now_bypassing_queue(ClientId::from_raw(2), dtr(false)));
        let took = started.elapsed();

        assert_eq!(out.as_deref(), Some(&b"OFF"[..]));
        assert!(took < PROMPT, "release took {took:?}");
        assert!(
            !stalled_done.load(Ordering::SeqCst),
            "the release must finish before the stalled exchange"
        );
        assert_eq!(logged(&log), vec!["cat begin FA;", "release dtr=false"]);

        while !stalled_done.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            logged(&log),
            vec!["cat begin FA;", "release dtr=false", "cat over FA;"]
        );
    }

    #[test]
    fn an_assert_still_waits_its_turn_behind_a_prior_exchange() {
        let session = StallingSession::new(true);
        let log = session.log();
        let (worker, handle) = build_with_timeout(session, &TABLE, STALL);
        std::thread::spawn(move || worker.run());

        let stalled_done = stall(&handle);
        until_logged(&log, "cat begin FA;");

        let out = block_on(handle.submit_task(ClientId::from_raw(2), dtr(true)));

        assert_eq!(out.as_deref(), Some(&b"ON"[..]));
        assert_eq!(
            logged(&log),
            vec!["cat begin FA;", "cat over FA;", "primary dtr=true"],
            "asserted on the session's own lines, after the prior exchange"
        );
        while !stalled_done.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn without_release_lines_a_release_falls_back_to_the_queue() {
        let session = StallingSession::new(false);
        let log = session.log();
        let (worker, handle) = build_with_timeout(session, &TABLE, STALL);
        std::thread::spawn(move || worker.run());

        let out = block_on(handle.release_now_bypassing_queue(ClientId::from_raw(1), dtr(false)));
        assert_eq!(out.as_deref(), Some(&b"OFF"[..]));
        assert_eq!(logged(&log), vec!["primary dtr=false"]);
    }

    #[test]
    fn dropping_the_worker_closes_the_duplicate() {
        let session = StallingSession::new(true);
        let drops = session.release_drops();
        let (worker, handle) = build_with_timeout(session, &TABLE, STALL);

        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(worker);
        assert_eq!(drops.load(Ordering::SeqCst), 1, "no duplicate left open");

        let out = block_on(handle.release_now_bypassing_queue(ClientId::from_raw(1), dtr(false)));
        assert_eq!(out, None);
    }
}
