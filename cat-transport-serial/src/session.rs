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
use cat_framework::wire_format::{AsciiLineFormat, FrameScanner};

use crate::timeouts::READ_TIMEOUT;
use cat_transport_core::{
    CatSession, ModemControlLines, ResponseDisposition, Transport, TransportError,
};

/// [`CatSession`] backed by a byte-level [`Transport`], reproducing today's
/// serial framing: write the request, then read bytes until a terminating
/// `';'` is seen (or the transport reports EOF).
///
/// This is a move of the framing logic that used to live directly in
/// `ts570d`'s `radio::RadioClient::read_response` — the wire bytes and
/// response boundary are unchanged, only the layer that owns them moved.
pub struct SerialCatSession<T: Transport, F: FrameScanner = AsciiLineFormat> {
    /// The wrapped byte-level transport. Public so callers (including
    /// tests) that already hold a `Transport` implementation can still
    /// reach it directly — `SerialCatSession` is a thin framing layer, not
    /// an opaque handle.
    pub transport: T,
    /// How a frame ends.
    ///
    /// Defaulted to `AsciiLineFormat`, so every existing caller compiles
    /// unchanged and keeps reading until `;`. A binary protocol supplies
    /// its own: CI-V frames end with `FD` and contain no semicolon at all,
    /// so a session that waited for one would wait forever — which is
    /// exactly what happened the first time an IC-7100 was pointed at
    /// this.
    format: F,
}

impl<T: Transport> SerialCatSession<T, AsciiLineFormat> {
    /// Wrap `transport` in a session that performs read-until-`;` framing.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            format: AsciiLineFormat,
        }
    }
}

/// Upper bound on a single response frame, in bytes.
///
/// `execute` reads until the wire format says the frame is complete. If a
/// terminator never arrives — a desynchronised stream, a radio that stopped
/// mid-answer — that loop has no natural end and the response buffer grows
/// without limit. The longest legitimate TS-570D response is `IF` at 38
/// bytes, so 64 leaves real headroom while still bounding the failure.
const MAX_FRAME_LEN: usize = 64;

impl<T: Transport, F: FrameScanner> SerialCatSession<T, F> {
    /// Wrap `transport`, framing with `format`.
    pub fn with_format(transport: T, format: F) -> Self {
        Self { transport, format }
    }
}

#[async_trait(?Send)]
impl<T: Transport, F: FrameScanner> CatSession for SerialCatSession<T, F> {
    type Error = TransportError;

    async fn execute(
        &mut self,
        request: &[u8],
        response: &mut Vec<u8>,
    ) -> Result<ResponseDisposition, TransportError> {
        self.transport.write(request).await?;
        self.transport.flush().await?;

        // Whole-frame deadline. `Transport::read`'s budget is per *call*, and
        // this loop calls it once per byte, so without this a single response
        // can legitimately outlive the broker's own per-request timeout
        // (`cat-server`'s DEFAULT_REQUEST_TIMEOUT, 5s). When that happens the
        // broker drops this future mid-exchange and starts the next job, and
        // two requests end up in flight on one serial port: the abandoned
        // response is then read by whoever asks next. Measured on a TS-570D
        // as `MD;` answering `SM0000;`.
        //
        // Bounding the whole frame here keeps this layer's failure inside its
        // own error path -- which flushes -- instead of being cancelled from
        // above, where nothing can clean up.
        let frame_deadline = std::time::Instant::now() + READ_TIMEOUT;

        let mut buf = [0u8; 1];
        loop {
            if std::time::Instant::now() >= frame_deadline {
                self.transport.flush_rx();
                response.clear();
                return Err(TransportError::ReadTimeout);
            }
            let n = match self.transport.read(&mut buf).await {
                Ok(n) => n,
                Err(e) => {
                    // A mid-frame failure is what poisons the stream. The
                    // bytes already read have been consumed from the kernel
                    // buffer and are about to be discarded with this error,
                    // while the rest of the frame is still arriving — so the
                    // *next* execute would read that tail as a fresh frame and
                    // every request after it would be offset by one frame
                    // boundary, permanently. Discard the remainder before
                    // returning so the damage stops here.
                    self.transport.flush_rx();
                    response.clear();
                    return Err(e);
                }
            };
            if n == 0 {
                // EOF — return whatever we have (may be empty).
                break;
            }
            response.push(buf[0]);
            // The format decides where a frame ends. `AsciiLineFormat`
            // says at `;`; CI-V says at `FD`.
            if self.format.frame_complete(response) {
                break;
            }
            if response.len() >= MAX_FRAME_LEN {
                // No terminator inside a plausible frame. Without this the
                // loop is unbounded and `response` grows without limit. The
                // longest legitimate TS-570D response is `IF` at 38 bytes;
                // the bound is deliberately well above that so a longer
                // command in future fails loudly rather than silently.
                self.transport.flush_rx();
                response.clear();
                return Err(TransportError::Other(format!(
                    "response exceeded {MAX_FRAME_LEN} bytes with no terminator"
                )));
            }
        }

        if response.is_empty() {
            Ok(ResponseDisposition::NoResponse)
        } else {
            Ok(ResponseDisposition::ResponseWritten)
        }
    }

    async fn send(&mut self, request: &[u8]) -> Result<(), TransportError> {
        // Deliberately does NOT read a *response*: set commands are
        // fire-and-forget on the real radio, and reading would cost the full
        // read timeout on every one.
        self.transport.write(request).await?;
        self.transport.flush().await?;
        // But it must consume anything the radio volunteers, here, before
        // returning.
        //
        // A rejected set sometimes answers `?;` and sometimes does not
        // ("Occasionally this message may not appear due to microprocessor
        // transients" — TS-570D manual). Such an answer is an orphan by
        // construction: `send` has no reader waiting. Draining at the start
        // of the *next* exchange is too late — the answer takes tens of
        // milliseconds to arrive and lands after that drain has run and
        // written, so the next read consumes it and every exchange after is
        // off by one. Measured on hardware at 39 crossings in 40
        // set-then-read cycles, `IF;` answering `SM0000;`.
        //
        // `drain` waits for a quiet line rather than blind-flushing, because
        // a `tcflush` here cuts a frame mid-arrival and leaves its tail.
        self.transport.drain().await;
        Ok(())
    }

    fn flush_rx(&mut self) {
        self.transport.flush_rx();
    }

    fn modem_lines(&self) -> Option<&dyn ModemControlLines> {
        // Delegate rather than `Some(self)`: this impl is bounded on
        // `T: Transport`, not `T: ModemControlLines`, so the session cannot
        // claim lines it may not have. The transport knows.
        self.transport.modem_lines()
    }
}

/// Blanket delegation: whenever the wrapped `Transport` also implements
/// [`ModemControlLines`] (e.g. `cat-transport-serial::SerialPort`),
/// `SerialCatSession<T>` forwards the capability unchanged, mirroring
/// `CatSession::flush_rx`'s delegation to `self.transport.flush_rx()` above.
/// A `SerialCatSession<T>` wrapping a transport WITHOUT modem lines (e.g. a
/// test fake) simply does not get this impl — nothing to opt out of.
impl<T: Transport + ModemControlLines, F: FrameScanner> ModemControlLines
    for SerialCatSession<T, F>
{
    fn set_rts(&self, asserted: bool) -> Result<(), TransportError> {
        self.transport.set_rts(asserted)
    }

    fn set_dtr(&self, asserted: bool) -> Result<(), TransportError> {
        self.transport.set_dtr(asserted)
    }

    fn read_cts(&self) -> Result<bool, TransportError> {
        self.transport.read_cts()
    }

    fn read_dsr(&self) -> Result<bool, TransportError> {
        self.transport.read_dsr()
    }

    fn read_dcd(&self) -> Result<bool, TransportError> {
        self.transport.read_dcd()
    }
}

// Gated `target_os = "linux"` in addition to `test`: every async test below
// uses `#[monoio::test(driver = "legacy")]`, and `monoio` is a Linux-only
// *target-gated* dependency (`[target.'cfg(target_os = "linux")'.
// dependencies]` in `Cargo.toml`) -- not present at all when building for
// `x86_64-pc-windows-gnu`. Without this gate, `cargo check --target
// x86_64-pc-windows-gnu -p cat-transport-serial --all-targets` fails to
// resolve the `monoio::test` attribute macro even though `SerialCatSession`
// itself (this file's non-test code) is genuinely cross-platform and needs
// no gating of its own. `io_uring.rs` has no analogous in-file gate to
// mirror -- that whole file is already `#[cfg(target_os = "linux")]`-gated
// at the `lib.rs` module-declaration level, so its test module inherits the
// gate for free; `session.rs` is different because the file itself compiles
// on both platforms and only its *tests* (which happen to all lean on
// `monoio`, including the one plain `#[test]` fn that lives in the same
// module) need this.
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::VecDeque;

    /// A minimal in-memory `Transport` fake, local to this test module.
    ///
    /// Also implements `ModemControlLines` (via `Cell`s for interior
    /// mutability, since the trait's methods take `&self`) so
    /// `SerialCatSession<T>`'s blanket delegating impl can be exercised
    /// without any real ioctl/hardware — unlike `cat-transport-serial`'s
    /// PTY-backed `SerialPort` tests, which can only reach the ENOTTY error
    /// path, this fake proves the delegation itself (values pass through
    /// unchanged) on its success path.
    struct FakeTransport {
        writes: Vec<u8>,
        reads: VecDeque<u8>,
        flush_rx_calls: usize,
        last_set_rts: Cell<Option<bool>>,
        last_set_dtr: Cell<Option<bool>>,
        cts: Cell<bool>,
        dsr: Cell<bool>,
        dcd: Cell<bool>,
    }

    impl FakeTransport {
        fn new() -> Self {
            Self {
                writes: Vec::new(),
                reads: VecDeque::new(),
                flush_rx_calls: 0,
                last_set_rts: Cell::new(None),
                last_set_dtr: Cell::new(None),
                cts: Cell::new(false),
                dsr: Cell::new(false),
                dcd: Cell::new(false),
            }
        }

        fn enqueue_response(&mut self, response: &str) {
            self.reads.extend(response.as_bytes());
        }
    }

    impl ModemControlLines for FakeTransport {
        fn set_rts(&self, asserted: bool) -> Result<(), TransportError> {
            self.last_set_rts.set(Some(asserted));
            Ok(())
        }

        fn set_dtr(&self, asserted: bool) -> Result<(), TransportError> {
            self.last_set_dtr.set(Some(asserted));
            Ok(())
        }

        fn read_cts(&self) -> Result<bool, TransportError> {
            Ok(self.cts.get())
        }

        fn read_dsr(&self) -> Result<bool, TransportError> {
            Ok(self.dsr.get())
        }

        fn read_dcd(&self) -> Result<bool, TransportError> {
            Ok(self.dcd.get())
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
            // Actually discard. `tcflush(TCIFLUSH)` drops what is queued, and
            // a fake that only counted the call would let a test "pass" while
            // the bytes it was supposed to discard were still delivered.
            self.reads.clear();
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

    /// `ModemControlLines` methods on `SerialCatSession<T>` must delegate
    /// unchanged to the wrapped transport, exactly mirroring
    /// `flush_rx_delegates_to_transport` above.

    #[test]
    fn modem_control_lines_delegate_to_transport() {
        let transport = FakeTransport::new();
        transport.cts.set(true);
        transport.dsr.set(false);
        transport.dcd.set(true);
        let session = SerialCatSession::new(transport);

        session.set_rts(true).expect("set_rts must succeed");
        session.set_dtr(false).expect("set_dtr must succeed");

        assert_eq!(session.transport.last_set_rts.get(), Some(true));
        assert_eq!(session.transport.last_set_dtr.get(), Some(false));
        // TransportError has no PartialEq impl, so compare the unwrapped
        // bools rather than the Results directly.
        assert!(session.read_cts().expect("read_cts must succeed"));
        assert!(!session.read_dsr().expect("read_dsr must succeed"));
        assert!(session.read_dcd().expect("read_dcd must succeed"));
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

    /// A transport that answers *when asked*, like a radio: the response to
    /// a request is queued by `write`, not pre-loaded. That distinction is
    /// the whole point here — an orphan is already in the buffer when the
    /// next request is written, whereas that request's own answer arrives
    /// afterwards and must survive the flush.
    struct AnsweringTransport {
        pending: VecDeque<u8>,
        script: Vec<(Vec<u8>, Vec<u8>)>,
        flush_rx_calls: usize,
        drain_calls: usize,
    }

    impl AnsweringTransport {
        fn new(script: Vec<(&str, &str)>) -> Self {
            Self {
                pending: VecDeque::new(),
                script: script
                    .into_iter()
                    .map(|(q, a)| (q.as_bytes().to_vec(), a.as_bytes().to_vec()))
                    .collect(),
                flush_rx_calls: 0,
                drain_calls: 0,
            }
        }
    }

    #[async_trait(?Send)]
    impl Transport for AnsweringTransport {
        async fn write(&mut self, data: &[u8]) -> Result<usize, TransportError> {
            if let Some((_, answer)) = self.script.iter().find(|(q, _)| q == data) {
                let answer = answer.clone();
                self.pending.extend(answer);
            }
            Ok(data.len())
        }
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
            match self.pending.pop_front() {
                Some(b) => {
                    buf[0] = b;
                    Ok(1)
                }
                None => Ok(0),
            }
        }
        async fn flush(&mut self) -> Result<(), TransportError> {
            Ok(())
        }
        fn flush_rx(&mut self) {
            self.flush_rx_calls += 1;
            self.pending.clear();
        }

        async fn drain(&mut self) {
            self.drain_calls += 1;
            self.pending.clear();
        }
    }

    #[monoio::test(driver = "legacy")]
    async fn a_set_consumes_its_own_orphaned_answer() {
        // The radio answered a rejected set. `send` must consume that answer
        // before returning, or the next request reads it as its own.
        let t = AnsweringTransport::new(vec![("TX;", "?;"), ("FA;", "FA00014250000;")]);
        let mut session = SerialCatSession::new(t);

        session.send(b"TX;").await.unwrap();
        assert_eq!(
            session.transport.drain_calls, 1,
            "send must drain the set's possible answer"
        );

        let mut response = Vec::new();
        session.execute(b"FA;", &mut response).await.unwrap();
        assert_eq!(
            response, b"FA00014250000;",
            "execute received the set's orphaned answer instead of its own"
        );
    }

    #[monoio::test(driver = "legacy")]
    async fn execute_without_a_preceding_send_does_not_flush() {
        // The discipline must cost nothing on the ordinary read path.
        let t = AnsweringTransport::new(vec![("FA;", "FA00014250000;")]);
        let mut session = SerialCatSession::new(t);
        let mut response = Vec::new();
        session.execute(b"FA;", &mut response).await.unwrap();
        assert_eq!(response, b"FA00014250000;");
        assert_eq!(
            session.transport.flush_rx_calls, 0,
            "a clean read path must not flush"
        );
    }

    #[monoio::test(driver = "legacy")]
    async fn a_silent_set_still_leaves_the_next_read_intact() {
        // The common case: the radio says nothing at all to a set. The drain
        // must cost the caller nothing beyond a quiet window, and must not
        // eat the next request's answer.
        let t = AnsweringTransport::new(vec![("MD;", "MD2;")]);
        let mut session = SerialCatSession::new(t);
        session.send(b"FA00014250000;").await.unwrap();

        let mut response = Vec::new();
        session.execute(b"MD;", &mut response).await.unwrap();
        assert_eq!(response, b"MD2;");
    }

    // ------------------------------------------------------------------
    // Frame-boundary integrity (Phase 0 — response crossing)
    //
    // `execute` read one byte at a time and propagated errors with `?`. On a
    // mid-frame failure the bytes already read were consumed and discarded
    // with the error while the frame's tail stayed in the kernel buffer, so
    // the *next* execute read that tail as a fresh frame and every request
    // afterwards was offset by one frame boundary, permanently.
    //
    // Observed on a physical TS-570D: an `IF;` answered with
    // `'0      000000 0002000008 ;'` — an IF frame's tail with its head gone
    // — and later `IF;` answered `TN08;` and `CT0;`.
    // ------------------------------------------------------------------

    /// A transport that fails mid-frame once, then serves the tail — exactly
    /// the shape that poisons the stream today.
    struct MidFrameFailTransport {
        reads: VecDeque<u8>,
        fail_after: usize,
        served: usize,
        failed: bool,
        flush_rx_calls: usize,
    }

    #[async_trait(?Send)]
    impl Transport for MidFrameFailTransport {
        async fn write(&mut self, data: &[u8]) -> Result<usize, TransportError> {
            Ok(data.len())
        }
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
            if !self.failed && self.served >= self.fail_after {
                self.failed = true;
                return Err(TransportError::ReadTimeout);
            }
            match self.reads.pop_front() {
                Some(b) => {
                    buf[0] = b;
                    self.served += 1;
                    Ok(1)
                }
                None => Ok(0),
            }
        }
        async fn flush(&mut self) -> Result<(), TransportError> {
            Ok(())
        }
        fn flush_rx(&mut self) {
            self.flush_rx_calls += 1;
            self.reads.clear();
        }
    }

    #[monoio::test(driver = "legacy")]
    async fn mid_frame_failure_does_not_poison_the_next_request() {
        // 10 bytes of an IF frame arrive, then the read fails. The remaining
        // bytes are still queued. A following request must NOT receive them.
        let mut t = MidFrameFailTransport {
            reads: "IF00014235340      000000 0002000008 ;".bytes().collect(),
            fail_after: 10,
            served: 0,
            failed: false,
            flush_rx_calls: 0,
        };
        t.reads.extend(b"FA00014235340;");
        let mut session = SerialCatSession::new(t);

        let mut first = Vec::new();
        let r = session.execute(b"IF;", &mut first).await;
        assert!(r.is_err(), "mid-frame timeout must surface as an error");

        let mut second = Vec::new();
        let _ = session.execute(b"FA;", &mut second).await;
        let got = String::from_utf8_lossy(&second).to_string();
        assert!(
            !got.starts_with('0') && !got.contains("000000 0002"),
            "second request received the first frame's tail: {got:?}"
        );
    }

    #[monoio::test(driver = "legacy")]
    async fn error_path_flushes_the_receive_buffer() {
        // The concrete mechanism: after any failed exchange the stale tail
        // must be discarded, not left for the next caller.
        let mut t = MidFrameFailTransport {
            reads: "IF00014235340      000000 0002000008 ;".bytes().collect(),
            fail_after: 10,
            served: 0,
            failed: false,
            flush_rx_calls: 0,
        };
        t.reads.extend(b"FA00014235340;");
        let mut session = SerialCatSession::new(t);
        let mut out = Vec::new();
        let _ = session.execute(b"IF;", &mut out).await;
        assert!(
            session.transport.flush_rx_calls > 0,
            "a failed execute must flush the receive buffer before returning"
        );
    }

    #[monoio::test(driver = "legacy", timer = true)]
    async fn a_frame_that_never_terminates_is_bounded() {
        // No `;` ever arrives. Without a length bound `response` grows without
        // limit and the loop never exits.
        struct EndlessTransport;
        #[async_trait(?Send)]
        impl Transport for EndlessTransport {
            async fn write(&mut self, d: &[u8]) -> Result<usize, TransportError> {
                Ok(d.len())
            }
            async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
                buf[0] = b'X';
                Ok(1)
            }
            async fn flush(&mut self) -> Result<(), TransportError> {
                Ok(())
            }
            fn flush_rx(&mut self) {}
        }
        let mut session = SerialCatSession::new(EndlessTransport);
        let mut out = Vec::new();
        // Wrapped in a timeout deliberately: without a frame bound this loops
        // forever, and an un-timeouted guard would *hang* rather than fail —
        // the same trap `ts570d/tests/rfc2217_under_monoio.rs` documents.
        let r = monoio::time::timeout(
            std::time::Duration::from_secs(2),
            session.execute(b"IF;", &mut out),
        )
        .await;
        match r {
            Err(_) => panic!("execute never returned: an unterminated frame is unbounded"),
            Ok(inner) => {
                assert!(
                    inner.is_err(),
                    "an unterminated frame must be bounded, not looped on"
                );
            }
        }
    }
}
