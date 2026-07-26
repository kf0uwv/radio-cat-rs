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

//! [`BrokerCatSession`]: a [`cat_transport_core::CatSession`] implementation
//! that submits raw wire-format requests through a [`crate::BrokerHandle`]
//! instead of talking to a transport directly.
//!
//! This is what lets a generic rigctld-style bridge (see the sibling
//! `cat-rigctl` crate) build a real `radio::SomeRadio<BrokerCatSession>`
//! (a radio crate's typed client, wrapping its own command table) and call
//! its already-correct, already-tested typed methods
//! (`get_vfo_a`/`set_mode`/`transmit`/...) instead of hand-rolling that
//! radio's wire frames a second time. No radio crate ever sees this type —
//! it lives here, alongside the broker it submits through, so that a
//! radio's own client crate never needs to depend on a transport crate
//! directly.
//!
//! Its `Error` type is deliberately [`TransportError`] (not a new local
//! error type) specifically because a radio crate's generic client (e.g.
//! `radio::SomeRadio<S>`) is typically bounded on
//! `S: CatSession<Error = TransportError>` — matching that bound exactly is
//! what makes this reuse possible at all, without widening the radio
//! crate's own generic bounds.

use async_trait::async_trait;
use cat_framework::ResponseDisposition;
use cat_transport_core::{CatSession, TransportError};

use crate::{BrokerHandle, ClientId};

/// One logical remote client's session against a [`BrokerHandle`] — the
/// physical radio session is shared (via the broker's single ordered
/// worker), but each [`BrokerCatSession`] is tagged with the [`ClientId`]
/// its raw TCP/rigctl connection was registered under, purely for the
/// broker's own logging/observability.
pub struct BrokerCatSession {
    handle: BrokerHandle,
    client_id: ClientId,
}

impl BrokerCatSession {
    pub fn new(handle: BrokerHandle, client_id: ClientId) -> Self {
        Self { handle, client_id }
    }
}

#[async_trait(?Send)]
impl CatSession for BrokerCatSession {
    type Error = TransportError;

    async fn execute(
        &mut self,
        request: &[u8],
        response: &mut Vec<u8>,
    ) -> Result<ResponseDisposition, Self::Error> {
        let wire = self
            .handle
            .submit(self.client_id, request.to_vec())
            .await
            .ok_or_else(|| TransportError::Other("broker worker has shut down".to_string()))?;

        // `crate::broker::outcome_to_wire`'s convention: empty payload = no
        // response; a `b"ERR <message>"` payload (no leading 2-letter
        // command code, no trailing `;`, so it can never collide with a
        // real CAT frame) = a dispatch-time failure at any layer (malformed
        // request, broker timeout, or the physical session/radio itself);
        // anything else = the radio's real response text.
        if wire.is_empty() {
            return Ok(ResponseDisposition::NoResponse);
        }
        if let Some(message) = wire.strip_prefix(b"ERR ") {
            return Err(TransportError::Other(
                String::from_utf8_lossy(message).into_owned(),
            ));
        }
        response.extend_from_slice(&wire);
        Ok(ResponseDisposition::ResponseWritten)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{FakeCommand, TABLE};
    use cat_transport_core::test_support::{Exchange, ScriptedCatSession};

    fn build_broker(
        exchanges: Vec<Exchange>,
    ) -> (
        crate::BrokerWorker<FakeCommand, ScriptedCatSession>,
        BrokerHandle,
    ) {
        let session = ScriptedCatSession::with_script(exchanges);
        crate::build(session, &TABLE)
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn execute_returns_response_written_with_response_bytes() {
        let (worker, handle) = build_broker(vec![Exchange::new("FA;", "FA014250000;")]);
        monoio::spawn(worker.run());

        let mut session = BrokerCatSession::new(handle, ClientId::from_raw(0));
        let mut response = Vec::new();
        let disposition = session.execute(b"FA;", &mut response).await.unwrap();

        assert_eq!(disposition, ResponseDisposition::ResponseWritten);
        assert_eq!(response, b"FA014250000;");
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn execute_maps_err_wire_convention_to_transport_error() {
        let (worker, handle) = build_broker(vec![]);
        monoio::spawn(worker.run());

        let mut session = BrokerCatSession::new(handle, ClientId::from_raw(0));
        let mut response = Vec::new();
        let err = session
            .execute(b"ZZ;", &mut response)
            .await
            .expect_err("unknown command must fail");

        assert!(matches!(err, TransportError::Other(_)));
        assert!(response.is_empty());
    }

    #[monoio::test(driver = "legacy", timer_enabled = true)]
    async fn execute_returns_no_response_for_empty_wire_payload() {
        // `TABLE`'s `FakeCommand::Frequency` set form is 11 digits wide
        // (`cat-server::test_fixtures`'s own fixed width, matching
        // `CommandTable::parse`'s structural gate) — unlike
        // `execute_returns_response_written_with_response_bytes` above,
        // this test sends a `Set`-shaped request, so its width must match.
        let (worker, handle) = build_broker(vec![Exchange::new("FA00014250000;", "")]);
        monoio::spawn(worker.run());

        let mut session = BrokerCatSession::new(handle, ClientId::from_raw(0));
        let mut response = Vec::new();
        let disposition = session
            .execute(b"FA00014250000;", &mut response)
            .await
            .unwrap();

        assert_eq!(disposition, ResponseDisposition::NoResponse);
        assert!(response.is_empty());
    }
}
