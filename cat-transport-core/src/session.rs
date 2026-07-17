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

//! Request/response session abstraction above [`crate::transport::Transport`].
//!
//! `Transport` is the lowest-level byte I/O primitive: write/read/flush
//! against a connection-shaped or connectionless endpoint, with no opinion on
//! CAT framing. [`CatSession`] sits one layer above it: it turns "write this
//! request" into "here is the response, and here is what happened" — the
//! request/response boundary a future TCP/UDP transport needs to own with its
//! own framing (length-prefixed envelopes, datagram envelopes with request
//! IDs, …) instead of inheriting the serial byte-until-`;` loop.
//!
//! `SerialCatSession`, the serial-specific implementation of this trait,
//! lives in `cat-transport-serial`, not here — this crate defines only the
//! trait every transport crate implements with its own framing.
//!
//! See `ts570d` ADR 0005 (`docs/adr/0005-network-transport-readiness.md`) and
//! `docs/architecture/network-readiness.md` for the full rationale.

use async_trait::async_trait;

use cat_framework::ResponseDisposition;

/// Request/response abstraction above byte-level [`crate::transport::Transport`] I/O.
///
/// A `CatSession` turns one wire request into one wire response (or the
/// documented absence of one), without assuming a single `read()` call
/// returns exactly one response, and without assuming a persistent,
/// file-descriptor-backed connection. Both are true of
/// `cat-transport-serial::SerialCatSession` today; neither is required by
/// the trait, so a future `TcpCatSession` / `UdpCatSession` can implement it
/// with entirely different framing.
///
/// # monoio compatibility
/// Uses `#[async_trait(?Send)]` — no `Send` bounds, matching
/// [`crate::transport::Transport`]'s convention and compatible with
/// monoio's thread-per-core (`!Send`) futures.
#[async_trait(?Send)]
pub trait CatSession {
    /// Session-specific error type.
    type Error;

    /// Execute one query-shaped exchange: write `request`, then populate
    /// `response` with whatever bytes the session considers "the answer".
    ///
    /// Returns the [`ResponseDisposition`] describing what happened —
    /// reused from `cat-framework` rather than a parallel type, since a
    /// session answers exactly the same question a server-side `CatRadio`
    /// dispatch does: was a response written, was there deliberately none,
    /// or did a protocol error respond in its place.
    async fn execute(
        &mut self,
        request: &[u8],
        response: &mut Vec<u8>,
    ) -> Result<ResponseDisposition, Self::Error>;

    /// Send a set-shaped (fire-and-forget) request that the radio never
    /// answers.
    ///
    /// The default implementation forwards to [`execute`](Self::execute) and
    /// discards the response, so any implementor that only writes `execute`
    /// keeps working unchanged. **Implementations backed by a real
    /// connection should override this** to avoid waiting on a response that
    /// will never arrive: the TS-570D CAT protocol is silent on set commands
    /// unless AI (auto-information) mode is enabled, which this codebase
    /// never turns on. `SerialCatSession` overrides `send` for exactly this
    /// reason — see its doc comment in `cat-transport-serial`.
    async fn send(&mut self, request: &[u8]) -> Result<(), Self::Error> {
        let mut discard = Vec::new();
        self.execute(request, &mut discard).await?;
        Ok(())
    }

    /// Discard any unread/unsolicited bytes buffered by the session.
    /// Default implementation is a no-op (e.g. for in-memory test doubles).
    fn flush_rx(&mut self) {}
}
