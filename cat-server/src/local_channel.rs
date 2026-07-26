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

//! Hand-rolled, single-threaded (`!Send`, `Rc`/`RefCell`-based) channel
//! primitives used to build [`crate::broker::BrokerWorker`]'s single ordered
//! worker.
//!
//! Not derived from any external channel crate — none is on this crate's
//! authorized dependency list (`cat-client`, `cat-transport-core`,
//! `async-trait`, `thiserror`, `monoio`). Built on `std` only:
//! `Rc<RefCell<_>>` for shared local state (sound here because monoio is a
//! thread-per-core, `!Send` executor — every task sharing one of these
//! channels runs on the same OS thread, never concurrently in the data-race
//! sense) plus `std::future::poll_fn` for the actual waiting.
//!
//! Two shapes:
//! - [`channel`] — a many-producer/single-consumer queue. `Sender<T>` is
//!   `Clone`, so every client-facing accept-loop/connection task can hold
//!   its own clone and push a [`crate::broker::Job`] onto the one worker's
//!   queue.
//! - [`oneshot`] — a single-use reply slot per request, so a request's
//!   response is delivered directly to the one task waiting on it, with no
//!   possibility of misrouting (unlike a request-id lookup table, which
//!   *could* misroute if implemented incorrectly).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::poll_fn;
use std::rc::Rc;
use std::task::{Poll, Waker};

// ---------------------------------------------------------------------------
// Many-producer/single-consumer queue
// ---------------------------------------------------------------------------

struct ChannelInner<T> {
    queue: VecDeque<T>,
    recv_waker: Option<Waker>,
    sender_count: usize,
    receiver_dropped: bool,
}

/// The paired [`Receiver`] is gone; `send` returns the value back to the
/// caller unchanged.
#[derive(Debug)]
pub struct SendError<T>(pub T);

/// Sending half of a [`channel`]. Cheaply `Clone`-able; every clone counts
/// toward keeping the channel open.
pub struct Sender<T> {
    inner: Rc<RefCell<ChannelInner<T>>>,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.inner.borrow_mut().sender_count += 1;
        Sender {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut inner = self.inner.borrow_mut();
        inner.sender_count -= 1;
        if inner.sender_count == 0 {
            if let Some(waker) = inner.recv_waker.take() {
                waker.wake();
            }
        }
    }
}

impl<T> Sender<T> {
    /// Push `value` onto the queue, waking the receiver if it is waiting.
    /// Never blocks. Fails only if the [`Receiver`] has already been
    /// dropped.
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        let mut inner = self.inner.borrow_mut();
        if inner.receiver_dropped {
            return Err(SendError(value));
        }
        inner.queue.push_back(value);
        if let Some(waker) = inner.recv_waker.take() {
            waker.wake();
        }
        Ok(())
    }
}

/// Receiving half of a [`channel`]. Not `Clone` — exactly one task owns
/// this, matching the "single ordered worker" contract.
pub struct Receiver<T> {
    inner: Rc<RefCell<ChannelInner<T>>>,
}

impl<T> Receiver<T> {
    /// Wait for the next queued value, in FIFO order. Returns `None` once
    /// every [`Sender`] has been dropped and the queue is empty — the
    /// channel is permanently closed at that point.
    pub async fn recv(&mut self) -> Option<T> {
        let inner = &self.inner;
        poll_fn(move |cx| {
            let mut guard = inner.borrow_mut();
            if let Some(value) = guard.queue.pop_front() {
                return Poll::Ready(Some(value));
            }
            if guard.sender_count == 0 {
                return Poll::Ready(None);
            }
            guard.recv_waker = Some(cx.waker().clone());
            Poll::Pending
        })
        .await
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.inner.borrow_mut().receiver_dropped = true;
    }
}

/// Create a many-producer/single-consumer local channel.
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let inner = Rc::new(RefCell::new(ChannelInner {
        queue: VecDeque::new(),
        recv_waker: None,
        sender_count: 1,
        receiver_dropped: false,
    }));
    (
        Sender {
            inner: Rc::clone(&inner),
        },
        Receiver { inner },
    )
}

// ---------------------------------------------------------------------------
// Oneshot reply slot
// ---------------------------------------------------------------------------

struct OneshotInner<T> {
    value: Option<T>,
    waker: Option<Waker>,
    sender_dropped: bool,
    receiver_dropped: bool,
}

/// Sending half of a [`oneshot`] reply slot.
pub struct OneshotSender<T> {
    inner: Rc<RefCell<OneshotInner<T>>>,
}

impl<T> OneshotSender<T> {
    /// Deliver `value` to the paired [`OneshotReceiver`]. Consumes `self` —
    /// a oneshot can only ever be used once, structurally.
    ///
    /// Returns `value` back on error if the receiver has already been
    /// dropped (e.g. the remote client that made this request disconnected
    /// before the physical radio answered) — this is a normal, expected
    /// outcome for the "disconnect mid-request" case, never a panic and
    /// never a block.
    pub fn send(self, value: T) -> Result<(), T> {
        let mut inner = self.inner.borrow_mut();
        if inner.receiver_dropped {
            return Err(value);
        }
        inner.value = Some(value);
        if let Some(waker) = inner.waker.take() {
            waker.wake();
        }
        Ok(())
    }
}

impl<T> Drop for OneshotSender<T> {
    fn drop(&mut self) {
        let mut inner = self.inner.borrow_mut();
        inner.sender_dropped = true;
        if let Some(waker) = inner.waker.take() {
            waker.wake();
        }
    }
}

/// Receiving half of a [`oneshot`] reply slot.
pub struct OneshotReceiver<T> {
    inner: Rc<RefCell<OneshotInner<T>>>,
}

impl<T> OneshotReceiver<T> {
    /// Wait for the value. Returns `None` if the [`OneshotSender`] is
    /// dropped without ever sending (e.g. the worker shut down mid-request).
    pub async fn recv(&mut self) -> Option<T> {
        let inner = &self.inner;
        poll_fn(move |cx| {
            let mut guard = inner.borrow_mut();
            if let Some(value) = guard.value.take() {
                return Poll::Ready(Some(value));
            }
            if guard.sender_dropped {
                return Poll::Ready(None);
            }
            guard.waker = Some(cx.waker().clone());
            Poll::Pending
        })
        .await
    }
}

impl<T> Drop for OneshotReceiver<T> {
    fn drop(&mut self) {
        self.inner.borrow_mut().receiver_dropped = true;
    }
}

/// Create a single-use oneshot reply slot.
pub fn oneshot<T>() -> (OneshotSender<T>, OneshotReceiver<T>) {
    let inner = Rc::new(RefCell::new(OneshotInner {
        value: None,
        waker: None,
        sender_dropped: false,
        receiver_dropped: false,
    }));
    (
        OneshotSender {
            inner: Rc::clone(&inner),
        },
        OneshotReceiver { inner },
    )
}

// Gated `target_os = "linux"` in addition to `test`: every test below uses
// `#[monoio::test(...)]`/`monoio::spawn`, and `monoio` is a Linux-only
// target-gated dependency -- mirrors `cat-server::broker`'s identical gate.
// `local_channel`'s own production code (this file's non-test items) is
// genuinely cross-platform (pure `std`) and needs no gating of its own --
// only its `monoio`-based test harness does.
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[monoio::test(driver = "legacy")]
    async fn channel_send_then_recv_in_order() {
        let (tx, mut rx) = channel::<u32>();
        tx.send(1).unwrap();
        tx.send(2).unwrap();
        assert_eq!(rx.recv().await, Some(1));
        assert_eq!(rx.recv().await, Some(2));
    }

    #[monoio::test(driver = "legacy")]
    async fn channel_recv_returns_none_once_all_senders_dropped() {
        let (tx, mut rx) = channel::<u32>();
        drop(tx);
        assert_eq!(rx.recv().await, None);
    }

    #[monoio::test(driver = "legacy")]
    async fn channel_send_fails_once_receiver_dropped() {
        let (tx, rx) = channel::<u32>();
        drop(rx);
        let result = tx.send(42);
        assert!(matches!(result, Err(SendError(42))));
    }

    #[monoio::test(driver = "legacy")]
    async fn channel_waits_for_a_value_sent_later() {
        let (tx, mut rx) = channel::<u32>();
        monoio::spawn(async move {
            tx.send(7).unwrap();
        });
        assert_eq!(rx.recv().await, Some(7));
    }

    #[monoio::test(driver = "legacy")]
    async fn oneshot_send_then_recv() {
        let (tx, mut rx) = oneshot::<u32>();
        tx.send(9).unwrap();
        assert_eq!(rx.recv().await, Some(9));
    }

    #[monoio::test(driver = "legacy")]
    async fn oneshot_send_fails_after_receiver_dropped() {
        let (tx, rx) = oneshot::<u32>();
        drop(rx);
        assert_eq!(tx.send(5), Err(5));
    }

    #[monoio::test(driver = "legacy")]
    async fn oneshot_recv_returns_none_if_sender_dropped_without_sending() {
        let (tx, mut rx) = oneshot::<u32>();
        drop(tx);
        assert_eq!(rx.recv().await, None);
    }
}
