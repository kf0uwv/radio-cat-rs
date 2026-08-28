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

//! Test-only [`CatSession`] support, shared across workspace crates.
//!
//! **Test-only. Not for production use.**
//!
//! This module is deliberately *not* `#[cfg(test)]`-gated. Rust's
//! `#[cfg(test)]` items are private to the crate that defines them — they
//! are not visible across a crate boundary even from the dependent crate's
//! own test build. A future radio crate's client tests (and `cat-client`'s
//! own tests) need to construct [`ScriptedCatSession`] values, so this
//! module is a plain public module (mirroring how some crates expose
//! `proptest` strategies as public, clearly-documented test infrastructure)
//! rather than duplicating a second ad hoc mock in every dependent crate.
//!
//! Contents:
//! - [`Exchange`] / [`ScriptedCatSession`] — an in-memory [`CatSession`] that
//!   matches expected request bytes against a script of canned responses,
//!   with timeout, disconnect, and malformed-response simulation.
//! - [`conformance`] — reusable [`CatSession`] behavior checks, generic over
//!   any implementation, so a future `TcpCatSession` / `UdpCatSession` test
//!   suite can call the exact same functions used here against
//!   `ScriptedCatSession`.

use std::collections::VecDeque;

use async_trait::async_trait;
use cat_framework::ResponseDisposition;

use crate::errors::TransportError;
use crate::session::CatSession;

/// One scripted request/response pair for [`ScriptedCatSession`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchange {
    /// The exact wire bytes the session must receive next.
    pub expected_request: Vec<u8>,
    /// The wire bytes to hand back as the response. An empty response
    /// yields [`ResponseDisposition::NoResponse`] — use this for set-shaped
    /// exchanges the radio never answers.
    pub response: Vec<u8>,
}

impl Exchange {
    /// Build an exchange from anything byte-slice-shaped (`&str`, `&[u8]`,
    /// byte-string literals, ...).
    pub fn new(expected_request: impl AsRef<[u8]>, response: impl AsRef<[u8]>) -> Self {
        Self {
            expected_request: expected_request.as_ref().to_vec(),
            response: response.as_ref().to_vec(),
        }
    }
}

/// In-memory [`CatSession`] test double.
///
/// Matches each `execute()`/`send()` request against the next [`Exchange`]
/// in the script (panicking with a descriptive message on a mismatch or an
/// exhausted script — the same "fail loud, fail at the call site" behavior
/// as other hand-rolled test mocks in this workspace), and can be told to
/// fail the *next* call instead, to simulate:
/// - a read timeout ([`simulate_timeout`](Self::simulate_timeout)),
/// - a disconnect ([`simulate_disconnect`](Self::simulate_disconnect)).
///
/// Malformed-response simulation needs no dedicated API: a `CatSession`
/// does not validate payload shape (that is a radio-layer concern), so
/// "malformed" is just an [`Exchange`] whose `response` bytes fail typed
/// parsing one layer up — script it like any other canned response.
#[derive(Default)]
pub struct ScriptedCatSession {
    script: VecDeque<Exchange>,
    written: Vec<u8>,
    fail_next: Option<TransportError>,
    unordered: bool,
}

impl ScriptedCatSession {
    /// Create a session with an empty script.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a session pre-loaded with `script`, executed in order.
    pub fn with_script<I: IntoIterator<Item = Exchange>>(script: I) -> Self {
        Self {
            script: script.into_iter().collect(),
            written: Vec::new(),
            fail_next: None,
            unordered: false,
        }
    }

    /// Create a session pre-loaded with `script`, matched **by request
    /// content rather than by position**.
    ///
    /// For tests where several clients submit concurrently and nothing
    /// orders their arrival. The ordered constructor turns such a test into
    /// a coin flip on thread scheduling: whichever request reaches the
    /// session first is compared against whichever exchange happens to be
    /// at the head of the queue.
    ///
    /// This is deliberately a separate constructor rather than a relaxation
    /// of [`with_script`](Self::with_script) -- for a single-client test,
    /// request *order* is a real property worth asserting, and callers
    /// should have to say when it is not the property under test.
    pub fn with_unordered_script<I: IntoIterator<Item = Exchange>>(script: I) -> Self {
        Self {
            unordered: true,
            ..Self::with_script(script)
        }
    }

    /// Append one more exchange to the end of the script.
    pub fn push_exchange(&mut self, exchange: Exchange) {
        self.script.push_back(exchange);
    }

    /// Fail the *next* `execute`/`send` call with a read timeout, simulating
    /// a radio that never answers.
    pub fn simulate_timeout(&mut self) {
        self.fail_next = Some(TransportError::ReadTimeout);
    }

    /// Fail the *next* `execute`/`send` call with a generic transport error,
    /// simulating a dropped connection.
    pub fn simulate_disconnect(&mut self) {
        self.fail_next = Some(TransportError::Other("disconnected".to_string()));
    }

    /// All bytes seen by `execute`/`send` so far, concatenated in order.
    pub fn written(&self) -> &[u8] {
        &self.written
    }

    /// [`written`](Self::written) decoded as UTF-8 (panics on non-UTF-8 —
    /// every TS-570D CAT frame is ASCII).
    pub fn written_str(&self) -> &str {
        std::str::from_utf8(&self.written).expect("non-UTF-8 bytes recorded by ScriptedCatSession")
    }

    /// `true` once every scripted exchange has been consumed.
    pub fn is_exhausted(&self) -> bool {
        self.script.is_empty()
    }
}

#[async_trait(?Send)]
impl CatSession for ScriptedCatSession {
    type Error = TransportError;

    async fn execute(
        &mut self,
        request: &[u8],
        response: &mut Vec<u8>,
    ) -> Result<ResponseDisposition, TransportError> {
        if let Some(err) = self.fail_next.take() {
            return Err(err);
        }

        self.written.extend_from_slice(request);

        let exchange = if self.unordered {
            let at = self
                .script
                .iter()
                .position(|e| e.expected_request == request)
                .unwrap_or_else(|| {
                    panic!(
                        "ScriptedCatSession: no unconsumed exchange matches request {:?}",
                        String::from_utf8_lossy(request)
                    )
                });
            self.script
                .remove(at)
                .expect("position() returned a valid index")
        } else {
            let exchange = self.script.pop_front().unwrap_or_else(|| {
                panic!(
                    "ScriptedCatSession: no exchange scripted for request {:?}",
                    String::from_utf8_lossy(request)
                )
            });

            assert_eq!(
                exchange.expected_request,
                request,
                "ScriptedCatSession: request mismatch (expected {:?}, got {:?})",
                String::from_utf8_lossy(&exchange.expected_request),
                String::from_utf8_lossy(request),
            );

            exchange
        };

        response.extend_from_slice(&exchange.response);

        if exchange.response.is_empty() {
            Ok(ResponseDisposition::NoResponse)
        } else {
            Ok(ResponseDisposition::ResponseWritten)
        }
    }
}

/// Reusable [`CatSession`] conformance checks.
///
/// Exercised against [`ScriptedCatSession`] in this crate's own tests, but
/// generic over `S: CatSession` so a future `TcpCatSession` / `UdpCatSession`
/// test module can call these exact functions against its own session type
/// rather than re-deriving the same assertions.
pub mod conformance {
    use super::*;

    /// A query-shaped `execute()` returns the scripted response bytes and
    /// reports [`ResponseDisposition::ResponseWritten`].
    pub async fn query_round_trip<S>(session: &mut S, request: &[u8], expected_response: &[u8])
    where
        S: CatSession,
        S::Error: std::fmt::Debug,
    {
        let mut response = Vec::new();
        let disposition = session
            .execute(request, &mut response)
            .await
            .expect("execute() should succeed");

        assert_eq!(response, expected_response, "response bytes mismatch");
        assert_eq!(disposition, ResponseDisposition::ResponseWritten);
    }

    /// A set-shaped `send()` succeeds without surfacing any response bytes
    /// to the caller, even when the underlying exchange carries none.
    pub async fn set_is_fire_and_forget<S>(session: &mut S, request: &[u8])
    where
        S: CatSession,
        S::Error: std::fmt::Debug,
    {
        session
            .send(request)
            .await
            .expect("send() should succeed for a fire-and-forget exchange");
    }

    /// A session that fails the next call surfaces that failure as an
    /// `Err` from `execute()`, rather than panicking or silently swallowing
    /// it.
    pub async fn surfaces_transport_error<S>(session: &mut S, request: &[u8])
    where
        S: CatSession,
        S::Error: std::fmt::Debug,
    {
        let mut response = Vec::new();
        let result = session.execute(request, &mut response).await;
        assert!(result.is_err(), "expected an error, got {:?}", result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_expected_request_and_returns_response() {
        futures::executor::block_on(async {
            let mut session =
                ScriptedCatSession::with_script([Exchange::new("FA;", "FA00014250000;")]);

            let mut response = Vec::new();
            let disposition = session.execute(b"FA;", &mut response).await.unwrap();

            assert_eq!(disposition, ResponseDisposition::ResponseWritten);
            assert_eq!(response, b"FA00014250000;");
            assert_eq!(session.written(), b"FA;");
            assert!(session.is_exhausted());
        });
    }

    #[test]
    fn empty_scripted_response_yields_no_response() {
        futures::executor::block_on(async {
            let mut session = ScriptedCatSession::with_script([Exchange::new("TX;", "")]);

            let mut response = Vec::new();
            let disposition = session.execute(b"TX;", &mut response).await.unwrap();

            assert_eq!(disposition, ResponseDisposition::NoResponse);
            assert!(response.is_empty());
        });
    }

    #[test]
    #[should_panic(expected = "request mismatch")]
    fn panics_on_request_mismatch() {
        futures::executor::block_on(async {
            let mut session =
                ScriptedCatSession::with_script([Exchange::new("FA;", "FA00014250000;")]);
            let mut response = Vec::new();
            let _ = session.execute(b"FB;", &mut response).await;
        });
    }

    #[test]
    #[should_panic(expected = "no exchange scripted")]
    fn panics_on_exhausted_script() {
        futures::executor::block_on(async {
            let mut session = ScriptedCatSession::new();
            let mut response = Vec::new();
            let _ = session.execute(b"FA;", &mut response).await;
        });
    }

    #[test]
    fn simulates_timeout() {
        futures::executor::block_on(async {
            let mut session = ScriptedCatSession::new();
            session.simulate_timeout();

            let mut response = Vec::new();
            let result = session.execute(b"FA;", &mut response).await;

            assert!(matches!(result, Err(TransportError::ReadTimeout)));
        });
    }

    #[test]
    fn simulates_disconnect() {
        futures::executor::block_on(async {
            let mut session = ScriptedCatSession::new();
            session.simulate_disconnect();

            let mut response = Vec::new();
            let result = session.execute(b"FA;", &mut response).await;

            assert!(matches!(result, Err(TransportError::Other(_))));
        });
    }

    #[test]
    fn simulated_failure_only_affects_next_call() {
        futures::executor::block_on(async {
            let mut session =
                ScriptedCatSession::with_script([Exchange::new("FA;", "FA00014250000;")]);
            session.simulate_timeout();

            let mut response = Vec::new();
            let first = session.execute(b"FA;", &mut response).await;
            assert!(first.is_err());

            // The script is untouched by the failed call — the same exchange
            // is still there for the next attempt.
            let second = session.execute(b"FA;", &mut response).await.unwrap();
            assert_eq!(second, ResponseDisposition::ResponseWritten);
        });
    }

    #[test]
    fn supports_malformed_response_bytes() {
        futures::executor::block_on(async {
            // "Malformed" needs no dedicated API: it is just a scripted
            // response that fails typed parsing one layer up.
            let mut session = ScriptedCatSession::with_script([Exchange::new("IF;", "IFxxxxx;")]);

            let mut response = Vec::new();
            let disposition = session.execute(b"IF;", &mut response).await.unwrap();

            assert_eq!(disposition, ResponseDisposition::ResponseWritten);
            assert_eq!(response, b"IFxxxxx;");
        });
    }

    #[test]
    fn send_matches_script_via_default_execute() {
        futures::executor::block_on(async {
            let mut session = ScriptedCatSession::with_script([Exchange::new("TX;", "")]);
            session.send(b"TX;").await.unwrap();
            assert_eq!(session.written(), b"TX;");
        });
    }

    // -----------------------------------------------------------------------
    // conformance harness — exercised against ScriptedCatSession today;
    // a future TcpCatSession/UdpCatSession test module reuses these
    // functions unchanged.
    // -----------------------------------------------------------------------

    #[test]
    fn conformance_query_round_trip() {
        futures::executor::block_on(async {
            let mut session = ScriptedCatSession::with_script([Exchange::new("ID;", "ID017;")]);
            conformance::query_round_trip(&mut session, b"ID;", b"ID017;").await;
        });
    }

    #[test]
    fn conformance_set_is_fire_and_forget() {
        futures::executor::block_on(async {
            let mut session = ScriptedCatSession::with_script([Exchange::new("TX;", "")]);
            conformance::set_is_fire_and_forget(&mut session, b"TX;").await;
        });
    }

    #[test]
    fn conformance_surfaces_transport_error() {
        futures::executor::block_on(async {
            let mut session = ScriptedCatSession::new();
            session.simulate_disconnect();
            conformance::surfaces_transport_error(&mut session, b"FA;").await;
        });
    }
}
