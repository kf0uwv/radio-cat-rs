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

//! Optional capability trait for direct RS-232 modem control/status line
//! access, independent of byte-level CAT framing.

use async_trait::async_trait;

use crate::errors::TransportError;
use crate::session::CatSession;
use cat_framework::ResponseDisposition;

/// Optional capability for a serial-backed Transport/CatSession: direct
/// control of RS-232 modem control lines (RTS, DTR) and status lines
/// (CTS, DSR, DCD), independent of byte-level CAT framing.
///
/// Not every transport has physical modem control lines — TCP/UDP sessions
/// have none — so this is a separate, additively-implemented trait, never
/// folded into the base Transport/CatSession traits. A consumer bounds its
/// own methods on `S: CatSession + ModemControlLines` rather than requiring
/// it universally.
///
/// All methods are plain sync fns, not #[async_trait] — these are direct
/// ioctl(2) calls with no I/O wait, matching the precedent already set by
/// Transport::flush_rx/CatSession::flush_rx (both plain sync fns on
/// otherwise-async traits, for the same reason).
pub trait ModemControlLines {
    fn set_rts(&self, asserted: bool) -> Result<(), TransportError>;
    fn set_dtr(&self, asserted: bool) -> Result<(), TransportError>;
    fn read_cts(&self) -> Result<bool, TransportError>;
    fn read_dsr(&self) -> Result<bool, TransportError>;
    fn read_dcd(&self) -> Result<bool, TransportError>;
}

/// A generic, reusable `CatSession` + `ModemControlLines` adapter for
/// sessions with no physical modem-control lines at all (TCP, UDP, ...).
///
/// `docs/adr/0006-windows-network-transport.md` (Deliverable 4): both
/// `ft991a` and `ts570d` need a TCP-backed session to satisfy UI trait
/// bounds that require `S: CatSession + ModemControlLines` (e.g.
/// `ft991a`'s `CwKeying`, bounded on `+ ModemControlLines`) even though TCP
/// has no RTS/DTR concept at all — `ft991a`'s `src/main.rs` originally hand-
/// wrote this as a bespoke `TcpClientSession` with five near-identical
/// `Err(...)` method bodies. `NoModemControlLines<S>` generalizes exactly
/// that shape so neither app (nor a future third one) needs to repeat it.
///
/// - **`CatSession`**: transparent delegation to the wrapped `S` — same
///   `Error` type, same `execute`/`send`/`flush_rx` behavior, unchanged.
///   `NoModemControlLines<S>` adds no new failure mode to the session
///   itself; it only adds the `ModemControlLines` capability below. This
///   deliberately does **not** attempt to also solve "map `S::Error` to
///   some other error type" (e.g. `cat-transport-tcp::TcpSessionError` to
///   `TransportError`, which `ft991a`'s `Ft991a<S>` bound separately
///   requires) — that mapping has nothing to do with modem-control lines,
///   is orthogonal, and (per `From`/`Into`'s orphan rules) can only be
///   written by a crate that can see both concrete error types, which
///   `cat-transport-core` deliberately never does for any concrete
///   transport crate. An application composes its own such mapping first
///   (mirroring `cat-server::broker_session::BrokerCatSession`'s identical
///   pattern) and wraps the *result* in `NoModemControlLines`, e.g.
///   `NoModemControlLines::new(TcpClientSession::new(tcp_session))` where
///   `TcpClientSession` is the small, app-local `CatSession<Error =
///   TransportError>` adapter around `TcpCatSession` every consumer of a
///   network transport already needs regardless of modem-line concerns.
/// - **`ModemControlLines`**: every method returns
///   [`TransportError::Other`] with a clear, honest message — never a
///   panic, never a silent no-op success — mirroring `ft991a`'s original
///   convention exactly (the same "honest failure" idiom
///   `radio::CwKeying`'s own trait-level defaults and `cat-rigctl`'s
///   `RPRT_ERR` responses already use elsewhere in this workspace). This
///   impl needs no bound on `S` at all (it never touches the wrapped
///   session), so it is available even if `S` doesn't implement
///   `CatSession`.
///
/// # Example
///
/// ```ignore
/// // app wiring layer (e.g. ft991a's or ts570d's src/main.rs)
/// let tcp_session = cat_transport_tcp::TcpCatSession::connect(addr).await?;
/// let mapped = TcpClientSession::new(tcp_session); // app-local CatSession<Error = TransportError> adapter
/// let radio = Ft991a::new(NoModemControlLines::new(mapped));
/// // `radio: Ft991a<NoModemControlLines<TcpClientSession>>` now satisfies
/// // both `CatSession<Error = TransportError>` and `ModemControlLines`.
/// ```
pub struct NoModemControlLines<S> {
    /// The wrapped session. Public, mirroring
    /// `cat-transport-serial::SerialCatSession`'s identical `pub transport`
    /// field — this is a thin, transparent wrapper, not an opaque handle.
    pub session: S,
}

impl<S> NoModemControlLines<S> {
    /// Wrap `session`, adding an honest-error [`ModemControlLines`] impl.
    pub fn new(session: S) -> Self {
        Self { session }
    }
}

/// The shared error every [`ModemControlLines`] method on
/// [`NoModemControlLines`] returns.
fn unsupported(method: &str) -> TransportError {
    TransportError::Other(format!(
        "{method}: RTS/DTR/CTS/DSR/DCD modem control lines are not available on this transport"
    ))
}

#[async_trait(?Send)]
impl<S: CatSession> CatSession for NoModemControlLines<S> {
    type Error = S::Error;

    async fn execute(
        &mut self,
        request: &[u8],
        response: &mut Vec<u8>,
    ) -> Result<ResponseDisposition, Self::Error> {
        self.session.execute(request, response).await
    }

    async fn send(&mut self, request: &[u8]) -> Result<(), Self::Error> {
        self.session.send(request).await
    }

    fn flush_rx(&mut self) {
        self.session.flush_rx();
    }
}

/// Unconditional — this impl never touches `S`, so it needs no bound on it.
impl<S> ModemControlLines for NoModemControlLines<S> {
    fn set_rts(&self, _asserted: bool) -> Result<(), TransportError> {
        Err(unsupported("set_rts"))
    }

    fn set_dtr(&self, _asserted: bool) -> Result<(), TransportError> {
        Err(unsupported("set_dtr"))
    }

    fn read_cts(&self) -> Result<bool, TransportError> {
        Err(unsupported("read_cts"))
    }

    fn read_dsr(&self) -> Result<bool, TransportError> {
        Err(unsupported("read_dsr"))
    }

    fn read_dcd(&self) -> Result<bool, TransportError> {
        Err(unsupported("read_dcd"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Exchange, ScriptedCatSession};

    #[test]
    fn execute_delegates_to_wrapped_session() {
        futures::executor::block_on(async {
            let mut session =
                NoModemControlLines::new(ScriptedCatSession::with_script([Exchange::new(
                    "FA;",
                    "FA00014250000;",
                )]));

            let mut response = Vec::new();
            let disposition = session.execute(b"FA;", &mut response).await.unwrap();

            assert_eq!(disposition, ResponseDisposition::ResponseWritten);
            assert_eq!(response, b"FA00014250000;");
        });
    }

    #[test]
    fn send_delegates_to_wrapped_session() {
        futures::executor::block_on(async {
            let mut session =
                NoModemControlLines::new(ScriptedCatSession::with_script([Exchange::new(
                    "TX;", "",
                )]));

            session.send(b"TX;").await.unwrap();

            assert_eq!(session.session.written(), b"TX;");
        });
    }

    #[test]
    fn flush_rx_delegates_to_wrapped_session() {
        futures::executor::block_on(async {
            // ScriptedCatSession's flush_rx is the trait default (a no-op) --
            // this just proves NoModemControlLines::flush_rx doesn't panic and
            // genuinely calls through rather than silently doing nothing of
            // its own.
            let mut session = NoModemControlLines::new(ScriptedCatSession::new());
            session.flush_rx();
        });
    }

    #[test]
    fn every_modem_control_lines_method_returns_an_honest_error() {
        let session = NoModemControlLines::new(ScriptedCatSession::new());

        assert!(session.set_rts(true).is_err());
        assert!(session.set_dtr(false).is_err());
        assert!(session.read_cts().is_err());
        assert!(session.read_dsr().is_err());
        assert!(session.read_dcd().is_err());
    }

    #[test]
    fn error_message_names_the_failing_method() {
        let session = NoModemControlLines::new(ScriptedCatSession::new());

        let err = session.set_rts(true).unwrap_err();
        assert!(
            format!("{err}").contains("set_rts"),
            "error should name the method that failed: {err}"
        );
    }
}
