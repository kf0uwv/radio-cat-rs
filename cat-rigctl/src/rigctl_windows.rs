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

//! Windows analog of `crate::rigctl`'s rigctld TCP accept loop — same wire
//! protocol ([`crate::protocol::dispatch`]/`\dump_state`/line framing, via
//! the shared [`crate::protocol::LineSplitter`]), rebuilt on `std::net` +
//! genuine OS threads instead of `monoio::net` + cooperative tasks.
//!
//! Mirrors `cat-server::tcp_windows`'s own shape exactly (see that module's
//! doc): one dedicated `std::thread` per accepted connection, each driving
//! its own blocking read/dispatch/write loop via [`cat_server::block_on`]
//! against [`cat_server::worker_windows::BrokerHandle`] (genuinely `Send`,
//! unlike `cat_server::broker::BrokerHandle`) — see
//! `docs/adr/0006-windows-network-transport.md`'s follow-up note on why
//! this crate needed its own Windows backend in addition to
//! `cat-transport-tcp`/`-udp`/`cat-server`'s.
//!
//! # Unlike `cat-server::tcp_windows`/`udp_windows`, this module IS
//! # `#[cfg(target_os = "windows")]`-gated
//!
//! Those two sibling modules stay ungated because they never construct a
//! [`cat_server::BrokerCatSession`] — they hand raw wire bytes straight to
//! the broker. This module does construct one (`make_radio` builds an
//! `R: RigctlRadio` from a `BrokerCatSession` per connection), and
//! `BrokerCatSession::new` is hardcoded to `cat_server`'s ambient,
//! `#[cfg]`-selected `BrokerHandle` alias (`broker::BrokerHandle` on
//! Linux, `worker_windows::BrokerHandle` on Windows) rather than being
//! generic over which handle type it wraps. That means a version of this
//! module written against `worker_windows::BrokerHandle` explicitly (the
//! genuinely `Send` one `std::thread::spawn` requires) cannot satisfy
//! `BrokerCatSession::new`'s parameter type on a Linux build, where the
//! ambient alias resolves to the different, `!Send`, `Rc`-based
//! `broker::BrokerHandle` instead — a real type mismatch, not a style
//! choice. Gating this module to Windows only is the honest fix, matching
//! `docs/adr/0004-windows-serial-backend.md`'s original precedent for
//! genuinely platform-divergent code: verified via `cargo check --target
//! x86_64-pc-windows-gnu`, not `cargo test`, on this Linux-only sandbox.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use cat_server::block_on::block_on;
use cat_server::worker_windows::BrokerHandle;
use cat_server::{BrokerCatSession, ClientId};

use crate::protocol::{self, LineSplitter};
use crate::RigctlRadio;

/// Accept loop. Binding is the caller's responsibility (pass an
/// already-bound `TcpListener`) so callers/tests can choose the address.
/// Spawns one dedicated `std::thread` per accepted connection; runs until
/// `accept()` itself fails.
pub(crate) fn serve<R, F>(
    listener: TcpListener,
    handle: BrokerHandle,
    make_radio: F,
) -> io::Result<()>
where
    R: RigctlRadio + 'static,
    F: Fn(BrokerCatSession) -> R + Clone + Send + 'static,
{
    let mut next_client_id: u64 = 0;
    loop {
        let (stream, _peer_addr) = listener.accept()?;
        let client_id = ClientId::from_raw(next_client_id);
        next_client_id = next_client_id.wrapping_add(1);
        let handle = handle.clone();
        let make_radio = make_radio.clone();
        thread::spawn(move || handle_connection(stream, handle, client_id, make_radio));
    }
}

/// Service one accepted connection until it disconnects, a line exceeds
/// [`protocol::MAX_LINE_LEN`], or the broker itself is gone. Blocking —
/// runs on its own dedicated thread. Structurally identical to
/// `crate::rigctl::handle_connection`, just blocking `std::net` I/O plus
/// [`block_on`] around each `async` dispatch call instead of `monoio`.
fn handle_connection<R, F>(
    mut stream: TcpStream,
    handle: BrokerHandle,
    client_id: ClientId,
    make_radio: F,
) where
    R: RigctlRadio,
    F: Fn(BrokerCatSession) -> R,
{
    let mut radio = make_radio(BrokerCatSession::new(handle, client_id));
    let mut splitter = LineSplitter::new();

    loop {
        let line = match read_line_blocking(&mut stream, &mut splitter) {
            Ok(Some(line)) => line,
            Ok(None) | Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.eq_ignore_ascii_case("q") {
            break;
        }

        let response = block_on(protocol::dispatch(&mut radio, trimmed));
        if stream.write_all(response.as_bytes()).is_err() {
            break;
        }
    }
}

/// Blocking analog of `crate::rigctl::read_line`, built on
/// `std::io::Read` instead of `monoio`'s owned-buffer
/// `AsyncReadRentExt::read` — same [`LineSplitter`], same contract.
fn read_line_blocking(
    stream: &mut TcpStream,
    splitter: &mut LineSplitter,
) -> io::Result<Option<String>> {
    loop {
        if let Some(line) = splitter.try_take_line() {
            return Ok(Some(line));
        }

        if splitter.is_over_limit() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "rigctl line exceeded maximum length of {} bytes without a newline",
                    protocol::MAX_LINE_LEN
                ),
            ));
        }

        let mut chunk = [0u8; 4096];
        match stream.read(&mut chunk) {
            Ok(0) => return Ok(splitter.take_final_partial_line()),
            Ok(n) => splitter.feed(&chunk[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_framework::{CommandDefinition, CommandForm, CommandOperation, CommandTable};
    use cat_server::worker_windows::build;
    use cat_transport_core::test_support::{Exchange, ScriptedCatSession};
    use cat_transport_core::CatSession;
    use std::net::SocketAddr;

    // Minimal fake command table, mirroring `crate::tests` (lib.rs)'s own
    // `FakeCommand`/`DEFINITIONS`/`TABLE` fixture — duplicated rather than
    // shared across a `#[cfg(test)]` module boundary, matching
    // `crate::protocol`'s own tests' precedent of a small local fixture per
    // test module.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FakeCommand {
        Frequency,
    }

    static DEFINITIONS: &[CommandDefinition<FakeCommand>] = &[CommandDefinition {
        id: FakeCommand::Frequency,
        code: "FA",
        name: "Frequency",
        description: "Test frequency",
        query_forms: &[CommandForm::fixed(CommandOperation::Query, 0)],
        set_forms: &[CommandForm::fixed(CommandOperation::Set, 9)],
        action_forms: &[],
        response_forms: &[],
        readable: true,
        writable: true,
    }];
    static TABLE: CommandTable<FakeCommand> = CommandTable::new(DEFINITIONS);

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FakeMode {
        Usb,
    }

    /// Backed directly by a real [`BrokerCatSession`] talking the fake `FA`
    /// command above — exercises the real accept-loop/thread/`block_on`/
    /// `BrokerHandle` plumbing end-to-end, not a hand-rolled shortcut.
    struct FakeRadio(BrokerCatSession);

    #[async_trait::async_trait(?Send)]
    impl RigctlRadio for FakeRadio {
        type Mode = FakeMode;
        type Error = cat_transport_core::TransportError;

        async fn get_vfo_a_hz(&mut self) -> Result<u64, Self::Error> {
            let mut response = Vec::new();
            self.0.execute(b"FA;", &mut response).await?;
            let s = String::from_utf8_lossy(&response);
            Ok(s.trim_start_matches("FA")
                .trim_end_matches(';')
                .parse()
                .unwrap_or(0))
        }
        async fn set_vfo_a_hz(&mut self, _hz: u64) -> Result<(), Self::Error> {
            unreachable!("not exercised by this module's own tests")
        }
        async fn get_mode(&mut self) -> Result<Self::Mode, Self::Error> {
            unreachable!("not exercised by this module's own tests")
        }
        async fn set_mode(&mut self, _mode: Self::Mode) -> Result<(), Self::Error> {
            unreachable!("not exercised by this module's own tests")
        }
        async fn get_transmitting(&mut self) -> Result<bool, Self::Error> {
            unreachable!("not exercised by this module's own tests")
        }
        async fn transmit(&mut self) -> Result<(), Self::Error> {
            unreachable!("not exercised by this module's own tests")
        }
        async fn receive(&mut self) -> Result<(), Self::Error> {
            unreachable!("not exercised by this module's own tests")
        }
        fn hamlib_mode_name(_mode: Self::Mode) -> &'static str {
            "USB"
        }
        fn hamlib_mode_from_name(_name: &str) -> Option<Self::Mode> {
            Some(FakeMode::Usb)
        }
        fn freq_range_hz() -> (u64, u64) {
            (30_000, 56_000_000)
        }
    }

    fn bind_loopback() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind failed");
        let addr = listener.local_addr().expect("local_addr failed");
        (listener, addr)
    }

    #[test]
    fn end_to_end_f_round_trips_over_real_loopback_tcp() {
        let (listener, addr) = bind_loopback();
        let (worker, handle) = build(
            ScriptedCatSession::with_script([Exchange::new("FA;", "FA00014250000;")]),
            &TABLE,
        );
        thread::spawn(move || worker.run());
        thread::spawn(move || serve(listener, handle, FakeRadio));

        let mut stream = TcpStream::connect(addr).expect("connect failed");
        stream.write_all(b"f\n").expect("write failed");
        let mut buf = [0u8; 64];
        let n = stream.read(&mut buf).expect("read failed");
        assert_eq!(&buf[..n], b"14250000\n");
    }
}
