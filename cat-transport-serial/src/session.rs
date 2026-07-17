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

//! [`SerialCatSession`]: the serial-specific [`CatSession`] implementation.
//!
//! Moved from `ts570d`'s `framework/src/session.rs` (commit `1585e1e`),
//! which defined this alongside the generic `CatSession` trait itself. The
//! trait now lives in `cat-transport-core`; this crate supplies the one
//! concrete implementation that reproduces the existing serial framing.

use async_trait::async_trait;

use cat_transport_core::{CatSession, ResponseDisposition, Transport, TransportError};

/// [`CatSession`] backed by a byte-level [`Transport`], reproducing today's
/// serial framing: write the request, then read bytes until a terminating
/// `';'` is seen (or the transport reports EOF).
///
/// This is a move of the framing logic that used to live directly in
/// `ts570d`'s `radio::RadioClient::read_response` — the wire bytes and
/// response boundary are unchanged, only the layer that owns them moved.
pub struct SerialCatSession<T: Transport> {
    /// The wrapped byte-level transport. Public so callers (including
    /// tests) that already hold a `Transport` implementation can still
    /// reach it directly — `SerialCatSession` is a thin framing layer, not
    /// an opaque handle.
    pub transport: T,
}

impl<T: Transport> SerialCatSession<T> {
    /// Wrap `transport` in a session that performs read-until-`;` framing.
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
}

#[async_trait(?Send)]
impl<T: Transport> CatSession for SerialCatSession<T> {
    type Error = TransportError;

    async fn execute(
        &mut self,
        request: &[u8],
        response: &mut Vec<u8>,
    ) -> Result<ResponseDisposition, TransportError> {
        self.transport.write(request).await?;
        self.transport.flush().await?;

        let mut buf = [0u8; 1];
        loop {
            let n = self.transport.read(&mut buf).await?;
            if n == 0 {
                // EOF — return whatever we have (may be empty).
                break;
            }
            response.push(buf[0]);
            if buf[0] == b';' {
                break;
            }
        }

        if response.is_empty() {
            Ok(ResponseDisposition::NoResponse)
        } else {
            Ok(ResponseDisposition::ResponseWritten)
        }
    }

    async fn send(&mut self, request: &[u8]) -> Result<(), TransportError> {
        // Deliberately does NOT read: set commands are fire-and-forget on
        // the real radio. Reading here would block for the transport's full
        // read timeout on every single set command in production.
        self.transport.write(request).await?;
        self.transport.flush().await?;
        Ok(())
    }

    fn flush_rx(&mut self) {
        self.transport.flush_rx();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A minimal in-memory `Transport` fake, local to this test module.
    struct FakeTransport {
        writes: Vec<u8>,
        reads: VecDeque<u8>,
        flush_rx_calls: usize,
    }

    impl FakeTransport {
        fn new() -> Self {
            Self {
                writes: Vec::new(),
                reads: VecDeque::new(),
                flush_rx_calls: 0,
            }
        }

        fn enqueue_response(&mut self, response: &str) {
            self.reads.extend(response.as_bytes());
        }
    }

    #[async_trait(?Send)]
    impl Transport for FakeTransport {
        async fn write(&mut self, data: &[u8]) -> Result<usize, TransportError> {
            self.writes.extend_from_slice(data);
            Ok(data.len())
        }

        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
            if let Some(byte) = self.reads.pop_front() {
                buf[0] = byte;
                Ok(1)
            } else {
                Ok(0)
            }
        }

        async fn flush(&mut self) -> Result<(), TransportError> {
            Ok(())
        }

        fn flush_rx(&mut self) {
            self.flush_rx_calls += 1;
        }
    }

    /// A `Transport` whose `read()` panics — used to prove `send()` never
    /// attempts a read, the exact regression a query-style `execute()`-based
    /// `send()` would reintroduce (see this module's doc comment).
    struct ReadPanicsTransport {
        writes: Vec<u8>,
    }

    impl ReadPanicsTransport {
        fn new() -> Self {
            Self { writes: Vec::new() }
        }
    }

    #[async_trait(?Send)]
    impl Transport for ReadPanicsTransport {
        async fn write(&mut self, data: &[u8]) -> Result<usize, TransportError> {
            self.writes.extend_from_slice(data);
            Ok(data.len())
        }

        async fn read(&mut self, _buf: &mut [u8]) -> Result<usize, TransportError> {
            panic!("send() must not read from the transport");
        }

        async fn flush(&mut self) -> Result<(), TransportError> {
            Ok(())
        }
    }

    #[monoio::test(driver = "legacy")]
    async fn execute_writes_request_and_reads_until_terminator() {
        let mut transport = FakeTransport::new();
        transport.enqueue_response("FA00014250000;");
        let mut session = SerialCatSession::new(transport);

        let mut response = Vec::new();
        let disposition = session.execute(b"FA;", &mut response).await.unwrap();

        assert_eq!(disposition, ResponseDisposition::ResponseWritten);
        assert_eq!(response, b"FA00014250000;");
        assert_eq!(session.transport.writes, b"FA;");
    }

    #[monoio::test(driver = "legacy")]
    async fn execute_stops_reading_after_first_terminator() {
        // Only bytes up to and including the first ';' belong to this
        // response — anything after is left unread, matching the original
        // `RadioClient::read_response` framing.
        let mut transport = FakeTransport::new();
        transport.enqueue_response("FA00014250000;IGNORED;");
        let mut session = SerialCatSession::new(transport);

        let mut response = Vec::new();
        session.execute(b"FA;", &mut response).await.unwrap();

        assert_eq!(response, b"FA00014250000;");
    }

    #[monoio::test(driver = "legacy")]
    async fn execute_returns_no_response_on_immediate_eof() {
        let transport = FakeTransport::new();
        let mut session = SerialCatSession::new(transport);

        let mut response = Vec::new();
        let disposition = session.execute(b"FA;", &mut response).await.unwrap();

        assert_eq!(disposition, ResponseDisposition::NoResponse);
        assert!(response.is_empty());
    }

    #[monoio::test(driver = "legacy")]
    async fn send_writes_without_reading() {
        let transport = ReadPanicsTransport::new();
        let mut session = SerialCatSession::new(transport);

        session.send(b"FA00014250000;").await.unwrap();

        assert_eq!(session.transport.writes, b"FA00014250000;");
    }

    #[monoio::test(driver = "legacy")]
    async fn flush_rx_delegates_to_transport() {
        let transport = FakeTransport::new();
        let mut session = SerialCatSession::new(transport);

        session.flush_rx();

        assert_eq!(session.transport.flush_rx_calls, 1);
    }

    #[monoio::test(driver = "legacy")]
    async fn propagates_transport_write_error() {
        struct FailingTransport;

        #[async_trait(?Send)]
        impl Transport for FailingTransport {
            async fn write(&mut self, _data: &[u8]) -> Result<usize, TransportError> {
                Err(TransportError::WriteTimeout)
            }

            async fn read(&mut self, _buf: &mut [u8]) -> Result<usize, TransportError> {
                Ok(0)
            }

            async fn flush(&mut self) -> Result<(), TransportError> {
                Ok(())
            }
        }

        let mut session = SerialCatSession::new(FailingTransport);
        let mut response = Vec::new();
        let result = session.execute(b"FA;", &mut response).await;

        assert!(matches!(result, Err(TransportError::WriteTimeout)));
    }
}
