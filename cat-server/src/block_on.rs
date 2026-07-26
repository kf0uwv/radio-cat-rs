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

//! A minimal, single-future, thread-parking executor.
//!
//! `docs/adr/0006-windows-network-transport.md`: Windows has no `monoio`
//! (banned/unavailable) and this repository does not adopt a general-purpose
//! async executor crate (per `docs/adr/0002`'s reasoning, extended here).
//! `crate::worker_windows`'s worker thread and `crate::tcp_windows`'s/
//! `crate::udp_windows`'s per-connection/per-datagram threads each need
//! *something* to drive a handful of `async fn` calls
//! (`Broker::dispatch`, `BrokerHandle::submit`) to completion, one at a
//! time, on a dedicated OS thread with nothing else to run concurrently --
//! exactly the shape `docs/adr/0004-windows-serial-backend.md` §1 sketches
//! for `ft991a`'s eventual Windows entry point ("a hand-rolled ~30-line
//! thread-parking `block_on`"). This is that primitive, generalized for
//! reuse inside this crate rather than left to each consuming application.
//!
//! Plain `std` -- no OS dependency -- so it is not `#[cfg(target_os =
//! "windows")]`-gated and gets real, executable tests on every platform,
//! same as `cat_transport_core::timeout`. Unlike that combinator, this one
//! has no `monoio`-compatibility caveat: `block_on` is never used to drive
//! anything that also runs under a real `monoio` runtime (it IS the
//! executor, on a dedicated OS thread with no `monoio` in the picture at
//! all) -- see `crate::broker::with_request_timeout`'s doc comment for the
//! specific incompatibility that constrains `cat_transport_core::timeout`'s
//! use inside this crate, which does not apply to `block_on` itself.

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

struct ThreadWaker(std::thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// Poll `fut` to completion on the calling thread, parking it between
/// `Pending` polls. Correct for any `Future` whose `Waker` implementation
/// follows the standard contract (see `cat_transport_core::completion`'s
/// module doc) -- in particular, it correctly drives futures built on that
/// completion primitive, woken from a different OS thread.
///
/// Not a general-purpose executor: it drives exactly one future at a time
/// on the thread that calls it, with no support for spawning additional
/// concurrent tasks. That is a deliberate scope limit (ADR 0002/0006), not
/// an oversight -- every caller in this crate uses it on a dedicated OS
/// thread whose entire job is running one sequential chain of async calls.
pub fn block_on<F: Future>(fut: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = Box::pin(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::park(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn resolves_an_immediately_ready_future() {
        assert_eq!(block_on(async { 1 + 1 }), 2);
    }

    #[test]
    fn drives_a_future_woken_from_a_different_thread() {
        let (tx, rx) = cat_transport_core::completion::channel::<u32>();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            tx.send(99);
        });

        assert_eq!(block_on(rx), Ok(99));
    }

    #[test]
    fn drives_a_multi_step_async_fn_body() {
        async fn two_steps() -> u32 {
            let (tx, rx) = cat_transport_core::completion::channel::<u32>();
            std::thread::spawn(move || tx.send(21));
            let first = rx.await.unwrap();
            first * 2
        }

        assert_eq!(block_on(two_steps()), 42);
    }
}
