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

//! A generic, `monoio`-free "run this future, or give up after a duration"
//! combinator.
//!
//! Originally introduced in `cat-server` (`docs/adr/0006-windows-network-transport.md`)
//! to replace `monoio::time::timeout` inside `Broker::dispatch`: that
//! function is shared, ungated code driven by two different executors
//! depending on which worker wraps it (a real `monoio` runtime on Linux, or
//! `cat-server`'s own minimal `block_on` on Windows/when its Windows-shaped
//! worker is exercised on Linux CI) -- `monoio::time::timeout` panics
//! outside an actual `monoio` reactor, so a single portable implementation
//! is a *correctness* requirement there, not just a style preference.
//!
//! Moved here (`docs/adr/0007-shared-diagnostics-engine.md`) and made a
//! shared, `pub` utility so `cat-diagnostics` can reuse the exact same
//! combinator to bound each per-command probe against a real radio
//! session -- a session that may have no timeout of its own by design
//! (e.g. `cat-transport-tcp::TcpCatSession`) -- instead of hand-rolling a
//! second copy. `cat-server` now imports this module instead of keeping its
//! own copy.
//!
//! This is plain `std` -- no OS dependency -- so it gets real, executable
//! tests here regardless of which crate's production code ends up calling
//! it (mirroring the identical precedent [`crate::completion`] documents
//! for itself).
//!
//! # How it works
//!
//! [`TimeoutFuture`] polls the wrapped future on every wake. If the future
//! isn't ready and the deadline hasn't passed yet, it lazily spawns one
//! dedicated `std::thread` that sleeps for the remaining duration and then
//! wakes the same `Waker` the driving executor gave this future -- causing
//! a re-poll that will find the deadline has passed (if the inner future
//! still hasn't resolved) and resolve to [`Elapsed`]. This composes with
//! any correct executor, per the same reasoning
//! `docs/adr/0004-windows-serial-backend.md` §1 gives for the completion
//! primitive's `Waker` contract.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

/// The wrapped future did not resolve before the configured duration
/// elapsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elapsed;

/// Future returned by [`timeout`]. See the module doc for how it works.
pub struct TimeoutFuture<F: Future> {
    inner: Pin<Box<F>>,
    deadline: Instant,
    timer_started: bool,
}

impl<F: Future> Future for TimeoutFuture<F> {
    type Output = Result<F::Output, Elapsed>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // `TimeoutFuture<F>`'s only field that could be `!Unpin` is `inner`,
        // and it is already `Pin<Box<F>>` -- a `Box` is `Unpin` regardless
        // of `F`, so `Pin<Box<F>>` is `Unpin`, and therefore so is the
        // whole struct (every other field is plain `Unpin` data). Safe to
        // get a plain `&mut Self` back out of the `Pin`.
        let this = self.get_mut();

        if let Poll::Ready(v) = this.inner.as_mut().poll(cx) {
            return Poll::Ready(Ok(v));
        }

        if Instant::now() >= this.deadline {
            return Poll::Ready(Err(Elapsed));
        }

        if !this.timer_started {
            this.timer_started = true;
            let waker = cx.waker().clone();
            let remaining = this.deadline.saturating_duration_since(Instant::now());
            std::thread::spawn(move || {
                std::thread::sleep(remaining);
                waker.wake();
            });
        }

        Poll::Pending
    }
}

/// Run `fut`, resolving to `Err(Elapsed)` if it has not completed by
/// `duration` after this function is called.
pub fn timeout<F: Future>(duration: Duration, fut: F) -> TimeoutFuture<F> {
    TimeoutFuture {
        inner: Box::pin(fut),
        deadline: Instant::now() + duration,
        timer_started: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::task::{Wake, Waker};

    struct ThreadWaker(std::thread::Thread);
    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    /// Minimal thread-parking block_on, local to this test module -- see
    /// `crate::completion`'s own test module for the identical shape.
    fn block_on<F: Future>(fut: F) -> F::Output {
        let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut fut = Box::pin(fut);
        loop {
            match fut.as_mut().poll(&mut cx) {
                Poll::Ready(out) => return out,
                Poll::Pending => std::thread::park(),
            }
        }
    }

    #[test]
    fn resolves_ready_when_inner_future_completes_before_the_deadline() {
        let result = block_on(timeout(Duration::from_secs(5), async { 42 }));
        assert_eq!(result, Ok(42));
    }

    #[test]
    fn resolves_elapsed_when_inner_future_never_completes() {
        let never = std::future::pending::<()>();
        let started = Instant::now();
        let result = block_on(timeout(Duration::from_millis(50), never));
        let elapsed = started.elapsed();

        assert_eq!(result, Err(Elapsed));
        assert!(
            elapsed >= Duration::from_millis(50),
            "returned before the configured duration elapsed: {:?}",
            elapsed
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "took far longer than the configured duration, looks like a near-hang: {:?}",
            elapsed
        );
    }

    #[test]
    fn does_not_spawn_a_timer_thread_when_the_inner_future_is_immediately_ready() {
        let started = Instant::now();
        let result = block_on(timeout(Duration::from_secs(30), async { "ready" }));
        assert_eq!(result, Ok("ready"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_slow_but_completing_future_still_resolves_ready_not_elapsed() {
        let count = Arc::new(AtomicU32::new(0));
        let count_clone = Arc::clone(&count);

        let (tx, rx) = crate::completion::channel::<u32>();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            count_clone.fetch_add(1, Ordering::SeqCst);
            tx.send(7);
        });

        let result = block_on(timeout(Duration::from_secs(5), rx));
        assert_eq!(result, Ok(Ok(7)));
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
