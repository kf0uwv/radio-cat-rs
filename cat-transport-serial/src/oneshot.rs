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

//! A portable, single-slot completion primitive.
//!
//! Per `docs/adr/0004-windows-serial-backend.md` §1: a future Windows
//! `SerialPort` drives blocking `ReadFile`/`WriteFile` on a dedicated
//! background `std::thread`, with `Transport::write`/`read`'s `async fn`
//! bodies sending a request over an `std::sync::mpsc` channel and then
//! `.await`ing one half of this primitive for the result. This module is
//! **pure `std`** — no OS dependency, no executor, no scheduling logic — it
//! only lets one `Future` resolve once, safely, when signaled from a
//! different OS thread, resting entirely on `std::task::Waker`'s documented
//! contract that `Waker::wake()` is safely callable from any thread. It is
//! private (`mod oneshot;` in `lib.rs`, not `pub`): an internal
//! implementation detail of a future Windows worker-thread design, not a
//! [`crate::config::SerialConfig`]/`Transport`-facing type in this crate's
//! public API.
//!
//! Because it has zero OS dependency of its own, it gets real, executable
//! unit tests here that run on ordinary Linux CI, even though its only
//! future consumer is Windows-specific code (ADR 0004's "Consequences"
//! section).
//!
//! `#![allow(dead_code)]`: this task (ADR 0004 dispatch queue Task 6) adds
//! this primitive standing alone, with no production caller yet — the
//! Windows worker-thread `SerialPort`/`Transport` implementation that
//! actually sends requests through a `CompletionTx`/awaits a `CompletionRx`
//! is Task 7/8, explicitly out of scope here. Everything below is exercised
//! by this module's own tests (`cfg(test)`, unaffected by this allow), just
//! not yet by any non-test code, which `dead_code` would otherwise flag as
//! a hard `-D warnings` clippy failure on a Linux build where the only
//! consumer (Windows `mod windows`) doesn't exist to compile in yet.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

/// The error [`CompletionRx::poll`] resolves to when the corresponding
/// [`CompletionTx`] is dropped without ever calling [`CompletionTx::send`] —
/// e.g. the Windows worker thread exiting (`SerialPort::drop` dropping the
/// request sender, per ADR 0004 §1) while a request is still in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Canceled;

/// The single slot shared between a [`CompletionTx`]/[`CompletionRx`] pair.
///
/// Exactly one of three states over its lifetime: empty (no value sent
/// yet), a value (the sender called [`CompletionTx::send`]), or canceled
/// (the sender was dropped without sending). Once set to `Value` or
/// `Canceled` it never changes again — a single-slot primitive is used
/// exactly once.
enum Slot<T> {
    Empty,
    Value(T),
    Canceled,
}

/// Shared state behind a [`CompletionTx`]/[`CompletionRx`] pair: a
/// `Mutex<Option<T>>`-shaped slot (here `Mutex<Slot<T>>`, since the empty vs.
/// canceled distinction needs a third state beyond `Option`'s two) plus an
/// `Option<Waker>` registered by the receiver's most recent `Pending` poll.
struct Completion<T> {
    slot: Mutex<Slot<T>>,
    waker: Mutex<Option<Waker>>,
}

impl<T> Completion<T> {
    /// Wake whatever `Waker` is currently registered, if any, consuming it
    /// (a fresh one is registered on the next `Pending` poll if needed).
    fn wake_if_registered(&self) {
        if let Some(waker) = self.waker.lock().unwrap().take() {
            waker.wake();
        }
    }
}

/// The sending half of a single-slot completion channel. Created via
/// [`channel`]. Consumed by [`CompletionTx::send`]; if dropped without ever
/// calling `send`, the paired [`CompletionRx`] resolves to
/// `Err(`[`Canceled`]`)` instead of hanging forever.
pub struct CompletionTx<T> {
    shared: Arc<Completion<T>>,
}

impl<T> CompletionTx<T> {
    /// Store `value` in the slot and wake the receiver's registered waker,
    /// if any. Consumes `self` — a single-slot primitive is sent to exactly
    /// once.
    pub fn send(self, value: T) {
        {
            let mut slot = self.shared.slot.lock().unwrap();
            *slot = Slot::Value(value);
        }
        self.shared.wake_if_registered();
        // `self` still needs to be dropped without `Drop::drop` re-marking
        // the (already-`Value`-filled) slot as `Canceled`. `Drop::drop`
        // below checks for `Slot::Empty` specifically, so this is safe —
        // documented there.
    }
}

impl<T> Drop for CompletionTx<T> {
    /// If `send` was never called (the slot is still `Empty`), mark it
    /// `Canceled` and wake the receiver — this is what lets
    /// `CompletionRx::poll` resolve to `Err(Canceled)` instead of leaving a
    /// `.await`ing task parked forever with no sender left to ever wake it.
    fn drop(&mut self) {
        let mut slot = self.shared.slot.lock().unwrap();
        if matches!(*slot, Slot::Empty) {
            *slot = Slot::Canceled;
            drop(slot);
            self.shared.wake_if_registered();
        }
    }
}

/// The receiving half of a single-slot completion channel. Created via
/// [`channel`]. Implements [`Future`](std::future::Future) — `.await` it to
/// suspend the calling task (returning [`Poll::Pending`], not blocking the
/// thread) until the paired [`CompletionTx`] either sends a value or is
/// dropped.
pub struct CompletionRx<T> {
    shared: Arc<Completion<T>>,
}

impl<T> std::future::Future for CompletionRx<T> {
    type Output = Result<T, Canceled>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // `CompletionRx<T>` has no `!Unpin` fields (just an `Arc`), so
        // reading through the `Pin` here without projecting any field is
        // sound: nothing in this type relies on address stability.
        let this = &*self;
        let mut slot = this.shared.slot.lock().unwrap();
        match std::mem::replace(&mut *slot, Slot::Empty) {
            Slot::Empty => {
                // No value yet: register this poll's waker (replacing any
                // stale one from an earlier poll on a moved future) so the
                // sender's `send`/`drop` can wake us later, then suspend.
                drop(slot);
                *this.shared.waker.lock().unwrap() = Some(cx.waker().clone());
                Poll::Pending
            }
            Slot::Value(value) => Poll::Ready(Ok(value)),
            Slot::Canceled => Poll::Ready(Err(Canceled)),
        }
    }
}

/// Create a new single-slot completion channel: a `Mutex<Option<T>> +
/// Option<Waker>`-shaped pair (see [`Completion`]) where the sender resolves
/// the receiver's `Future` exactly once, safely, from any thread.
pub fn channel<T>() -> (CompletionTx<T>, CompletionRx<T>) {
    let shared = Arc::new(Completion {
        slot: Mutex::new(Slot::Empty),
        waker: Mutex::new(None),
    });
    (
        CompletionTx {
            shared: Arc::clone(&shared),
        },
        CompletionRx { shared },
    )
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    use std::time::{Duration, Instant};

    use super::{channel, Canceled};

    /// A `Waker` that just records whether it was ever woken — enough to
    /// verify "a `Pending` poll registers a waker that later fires,"
    /// without pulling in an executor or the `futures` crate.
    #[derive(Default)]
    struct RecordingWaker {
        woken: AtomicBool,
    }

    impl Wake for RecordingWaker {
        fn wake(self: Arc<Self>) {
            self.woken.store(true, Ordering::SeqCst);
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.woken.store(true, Ordering::SeqCst);
        }
    }

    /// A `Waker` that unparks a specific thread — used by
    /// [`block_on_with_timeout`] below, the same "park in a loop, `Waker`
    /// unparks" shape ADR 0004 §1 describes for `ft991a`'s eventual
    /// Windows `block_on`. Test-only infrastructure: driving a real
    /// executor is out of scope for this primitive and for this task.
    struct ThreadWaker(std::thread::Thread);

    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    /// Poll `fut` to completion, parking the current thread between
    /// `Pending` polls, bounded by `timeout`. Returns `None` on timeout
    /// instead of hanging forever — so a regression in the cancellation or
    /// cross-thread-wake path fails this test suite promptly rather than
    /// hanging it, per this task's explicit requirement.
    fn block_on_with_timeout<F: Future>(fut: F, timeout: Duration) -> Option<F::Output> {
        let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut fut = Box::pin(fut);
        let deadline = Instant::now() + timeout;
        loop {
            match fut.as_mut().poll(&mut cx) {
                Poll::Ready(output) => return Some(output),
                Poll::Pending => {
                    let now = Instant::now();
                    if now >= deadline {
                        return None;
                    }
                    std::thread::park_timeout(deadline - now);
                }
            }
        }
    }

    /// (a) Polling before a value is sent returns `Pending` and registers a
    /// waker — proven by then sending (from the same thread, synchronously)
    /// and observing the registered waker actually fires, then observing a
    /// follow-up poll resolves with the sent value.
    #[test]
    fn poll_before_send_returns_pending_and_registers_waker() {
        let (tx, mut rx) = channel::<u32>();
        let recording = Arc::new(RecordingWaker::default());
        let waker = Waker::from(Arc::clone(&recording));
        let mut cx = Context::from_waker(&waker);

        let first_poll = Pin::new(&mut rx).poll(&mut cx);
        assert!(
            matches!(first_poll, Poll::Pending),
            "expected Pending before any value is sent, got {first_poll:?}"
        );
        assert!(
            !recording.woken.load(Ordering::SeqCst),
            "waker must not fire before send"
        );

        tx.send(7);
        assert!(
            recording.woken.load(Ordering::SeqCst),
            "the waker registered by the Pending poll must fire on send"
        );

        match Pin::new(&mut rx).poll(&mut cx) {
            Poll::Ready(Ok(value)) => assert_eq!(value, 7),
            other => panic!("expected Ready(Ok(7)) after send, got {other:?}"),
        }
    }

    /// (b) A value sent from a separately-spawned `std::thread` (after a
    /// short delay, so the receiver is genuinely `Pending` and parked when
    /// the send happens — not resolved same-thread/immediately) correctly
    /// wakes and resolves the receiver with the sent value.
    #[test]
    fn cross_thread_send_after_delay_wakes_and_resolves_with_value() {
        let (tx, rx) = channel::<u32>();

        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            tx.send(42);
        });

        let result = block_on_with_timeout(rx, Duration::from_secs(5));
        handle.join().expect("sender thread panicked");

        assert_eq!(
            result,
            Some(Ok(42)),
            "cross-thread send must wake the parked receiver with the sent value, not time out"
        );
    }

    /// (c) Dropping the sender before ever sending resolves the receiver to
    /// `Err(Canceled)` rather than hanging forever. Exercised across a real
    /// thread boundary with a short delay (mirroring (b)'s cross-thread
    /// wake path) so this also proves the *cancellation* wake path fires
    /// correctly, not only the value-send wake path. Bounded by
    /// `block_on_with_timeout`'s timeout: a bug here (e.g. `Drop` failing
    /// to wake a registered waker) fails this test instead of hanging the
    /// suite.
    #[test]
    fn dropping_sender_before_send_resolves_to_canceled_not_hang() {
        let (tx, rx) = channel::<u32>();

        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            drop(tx); // never called `.send(..)`
        });

        let result = block_on_with_timeout(rx, Duration::from_secs(5));
        handle.join().expect("dropper thread panicked");

        assert_eq!(
            result,
            Some(Err(Canceled)),
            "dropping the sender without sending must resolve to Err(Canceled), not time out"
        );
    }

    /// Sending before the receiver is ever polled must also resolve
    /// correctly (the value sits in the slot until the first poll observes
    /// it) — no waker is registered yet in this ordering, so this exercises
    /// the immediate-`Ready` path distinctly from (a)'s pending-then-send
    /// path.
    #[test]
    fn send_before_first_poll_resolves_immediately() {
        let (tx, mut rx) = channel::<&'static str>();
        tx.send("ready");

        let recording = Arc::new(RecordingWaker::default());
        let waker = Waker::from(recording);
        let mut cx = Context::from_waker(&waker);

        match Pin::new(&mut rx).poll(&mut cx) {
            Poll::Ready(Ok(value)) => assert_eq!(value, "ready"),
            other => panic!("expected Ready(Ok(\"ready\")), got {other:?}"),
        }
    }
}
