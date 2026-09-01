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

//! `cat-rigctl`: a generic Hamlib rigctld-compatible TCP bridge (for
//! WSJT-X's "Hamlib NET rigctl" rig type) plus the network-server
//! orchestration around it — one process owning a physical
//! [`cat_transport_core::CatSession`], shared by [`cat_server`]'s raw
//! TCP/UDP listeners and this crate's rigctld listener alike.
//!
//! Extracted from `ft991a`'s and `ts570d`'s independently hand-written,
//! ~90-100% duplicated `server` crates (see `planning/cat_rigctl/` in this
//! repo for the extraction record). The one genuinely radio-specific piece
//! — Hamlib mode-name mapping, plus a handful of typed get/set calls — is
//! isolated behind [`RigctlRadio`], a trait each consuming app implements
//! once for its own typed radio client (e.g. `ft991a`'s `radio::Ft991a<S>`,
//! `ts570d`'s `radio::Ts570d<S>`). Everything else — dispatch, `dump_state`,
//! line framing, listener orchestration, error propagation — lives here,
//! generic and radio-independent.
//!
//! ```text
//! rigctld TCP client (WSJT-X) ──▶ cat-rigctl::run/serve/dispatch ──▶ R: RigctlRadio
//!                                                                        │
//!                                                        (an app's own radio
//!                                                         crate's typed client,
//!                                                         wrapping cat_server::
//!                                                         BrokerCatSession)
//! ```
//!
//! - [`RigctlRadio`] — what the generic bridge needs from a radio's own
//!   typed client. Implemented once per app.
//! - [`ServerConfig`] — which listeners to bring up.
//! - [`run`] — brings up the broker plus every listener `config` requests,
//!   and runs until one fails. Two implementations, `#[cfg]`-selected —
//!   see "Platform support" below.
//! - [`protocol`] (private) — the rigctld wire protocol itself (command
//!   dispatch, `\dump_state`, line buffering), with no I/O of its own,
//!   shared by both platform backends.
//! - [`rigctl`] (private, Linux) / [`rigctl_windows`] (private, Windows) —
//!   the platform-specific accept loop around `protocol`, generic over
//!   `R: RigctlRadio`. Neither is part of the public API — only
//!   `RigctlRadio`/`ServerConfig`/`run` are; a consuming app's own wiring
//!   layer only ever calls `run`.
//!
//! # Platform support
//!
//! Both `run` implementations bring up the same three possible listeners
//! (raw TCP, raw UDP, rigctld) from the same [`ServerConfig`], and share
//! the exact same [`RigctlRadio`]/[`protocol::dispatch`] logic — a
//! consuming application calls one `cat_rigctl::run(...)` on either
//! platform, no branching required. Only the concurrency substrate differs
//! internally: Linux uses `monoio`'s cooperative tasks (`run` is `async
//! fn`, `#[monoio::main]`-compatible); Windows uses genuine OS threads
//! (`run` is a plain blocking `fn`, matching
//! `cat_server::worker_windows::BrokerWorker::run`'s own precedent that
//! top-level platform bootstrapping is expected to differ — see
//! `docs/adr/0006-windows-network-transport.md`'s follow-up note for the
//! full design record of this crate's Windows backend).

#[cfg(target_os = "linux")]
mod rigctl;
#[cfg(target_os = "windows")]
mod rigctl_windows;

pub mod native_bridge;
mod protocol;

use std::io;

use async_trait::async_trait;
use cat_framework::{CommandId, CommandTable};
use cat_server::BrokerCatSession;
use cat_transport_core::CatSession;
use tracing::{error, info};

/// What the generic rigctld bridge needs from a radio's own typed client
/// (e.g. `ft991a`'s `radio::Ft991a<S>`, `ts570d`'s `radio::Ts570d<S>`).
///
/// `Error` is deliberately unbounded (no `Display`/`std::error::Error`
/// requirement) — [`rigctl::dispatch`] only distinguishes `Ok`/`Err`, never
/// displays the message (rigctld's `RPRT -1` convention carries no error
/// text on the wire).
///
/// Mode-name mapping (`hamlib_mode_name`/`hamlib_mode_from_name`) is
/// deliberately *not* shared logic — it is genuinely per-radio (e.g. an
/// FT-991A implementation has C4FM/data-mode fallbacks with no exact
/// Hamlib counterpart; a TS-570D implementation is a clean 1:1 table over
/// a smaller mode set) — which is exactly why these are trait methods each
/// app implements itself, rather than a shared table this crate would have
/// to own.
#[async_trait(?Send)]
pub trait RigctlRadio {
    /// This radio's mode type (e.g. `radio::Mode`).
    type Mode: Copy;
    /// This radio's client error type. Never displayed by this crate — see
    /// the trait's own doc comment.
    type Error;

    /// Current VFO A frequency, in Hz.
    async fn get_vfo_a_hz(&mut self) -> Result<u64, Self::Error>;
    /// Set VFO A frequency, in Hz.
    async fn set_vfo_a_hz(&mut self, hz: u64) -> Result<(), Self::Error>;
    /// Current operating mode.
    async fn get_mode(&mut self) -> Result<Self::Mode, Self::Error>;
    /// Set operating mode.
    async fn set_mode(&mut self, mode: Self::Mode) -> Result<(), Self::Error>;
    /// Whether the radio is currently transmitting.
    async fn get_transmitting(&mut self) -> Result<bool, Self::Error>;
    /// Key the radio into transmit.
    async fn transmit(&mut self) -> Result<(), Self::Error>;
    /// Return the radio to receive.
    async fn receive(&mut self) -> Result<(), Self::Error>;

    /// Map `mode` to the Hamlib rig-mode name `m`/`M` exchange on the wire.
    fn hamlib_mode_name(mode: Self::Mode) -> &'static str;
    /// Map a Hamlib rig-mode name back to `Self::Mode`, if recognized.
    fn hamlib_mode_from_name(name: &str) -> Option<Self::Mode>;
    /// `(min_hz, max_hz)` for `\dump_state`'s RX/TX frequency range rows.
    fn freq_range_hz() -> (u64, u64);

    /// This radio's capabilities, if it publishes them.
    ///
    /// When present, `\dump_state`'s capability tail is **generated** from
    /// this instead of being hand-maintained (ADR 0010 §6). That matters
    /// because the hand-maintained version is exactly where ADR 0005's
    /// field-count bug came from: a reply that is short by one line makes
    /// Hamlib's `netrigctl_open()` block forever, and nothing about the
    /// symptom points at the cause.
    ///
    /// Defaults to `None` so existing implementations keep working
    /// unchanged, with the placeholder tail they have always sent. A radio
    /// gains real rigctl capability reporting by describing itself, not by
    /// editing this crate.
    fn capabilities() -> Option<&'static cat_framework::capabilities::RadioCapabilities> {
        None
    }
}

/// Which network listeners to bring up. Every field is optional — a `None`
/// port simply skips binding that listener, so a deployment can run with
/// only the pieces it needs (typically just `rigctl_port`, for WSJT-X).
#[derive(Debug, Clone, Default)]
pub struct ServerConfig {
    /// The typed console protocol (ADR 0010 §6), for a GUI or a TUI.
    pub native_port: Option<u16>,
    /// `cat-server`'s raw length-prefixed TCP protocol.
    pub raw_tcp_port: Option<u16>,
    /// `cat-server`'s raw enveloped UDP protocol.
    pub raw_udp_port: Option<u16>,
    /// The Hamlib rigctld-compatible TCP listener, for WSJT-X.
    pub rigctl_port: Option<u16>,
}

/// Bring up the broker (owning `session`, the one physical radio
/// connection, validated against `table`) plus every listener `config`
/// requests, and run until one of them fails.
///
/// `make_radio` constructs one app's typed radio wrapper from a
/// [`BrokerCatSession`] per rigctl connection (e.g. `|s| Ft991a::new(s)`) —
/// this is the one seam where a caller's concrete radio type plugs into
/// otherwise fully generic orchestration.
///
/// Ported from `ft991a`'s `server/src/lib.rs::run` for the overall shape,
/// with `ts570d`'s error-propagation fix applied: each listener task
/// itself returns `io::Result<()>`, so the `select_all` this function waits
/// on resolves with that real result as its first tuple element, and this
/// function returns it directly — rather than discarding each task's
/// result via `let _ = ...` and hardcoding `Ok(())` regardless of outcome.
#[cfg(target_os = "linux")]
pub async fn run<C, S, R, F>(
    session: S,
    table: &'static CommandTable<C>,
    config: ServerConfig,
    make_radio: F,
) -> io::Result<()>
where
    C: CommandId,
    S: CatSession + 'static,
    S::Error: std::error::Error + 'static,
    R: RigctlRadio + 'static,
    F: Fn(BrokerCatSession) -> R + Clone + 'static,
{
    run_with_native(
        session,
        table,
        config,
        make_radio,
        |_| native_bridge::NoNative,
        None,
    )
    .await
}

/// [`run`], also serving the typed console protocol.
///
/// `make_native` builds the thing that reads and drives the radio for
/// consoles, from its own broker session. It is a second seam rather than
/// a reuse of `make_radio` because the two want different shapes: rigctl
/// asks for one field at a time, a console asks for all of them at once
/// and expects meters with them.
///
/// Serving consoles needs no `native_port`; without one this is exactly
/// [`run`] plus an idle task.
#[cfg(target_os = "linux")]
pub async fn run_with_native<C, S, R, F, N, G>(
    session: S,
    table: &'static CommandTable<C>,
    config: ServerConfig,
    make_radio: F,
    make_native: G,
    native: Option<std::sync::Arc<native_bridge::NativeShared>>,
) -> io::Result<()>
where
    C: CommandId,
    S: CatSession + 'static,
    S::Error: std::error::Error + 'static,
    R: RigctlRadio + 'static,
    F: Fn(BrokerCatSession) -> R + Clone + 'static,
    N: native_bridge::NativeRadio + 'static,
    G: FnOnce(BrokerCatSession) -> N + 'static,
{
    use std::cell::RefCell;
    use std::rc::Rc;

    use cat_server::ClientRegistry;
    use monoio::net::{udp::UdpSocket, TcpListener};

    let (worker, handle) = cat_server::build(session, table);
    monoio::spawn(worker.run());

    // The caller owns this, not us: a radio with a spectrum source has to
    // publish frames into the same cache the listener reads from, and that
    // source is the app's business. `cat-rigctl` orchestrates listeners
    // and has no opinion about where a spectrum comes from.
    let native_shared = native.filter(|_| config.native_port.is_some());
    let serving_consoles = native_shared.is_some();

    let mut tasks = Vec::new();

    if let Some(port) = config.raw_tcp_port {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        info!("Raw CAT TCP listener bound on 0.0.0.0:{port}");
        let handle = handle.clone();
        let registry = Rc::new(RefCell::new(ClientRegistry::new()));
        tasks.push(monoio::spawn(async move {
            let result = cat_server::tcp::serve(listener, handle, registry).await;
            if let Err(e) = &result {
                error!("Raw CAT TCP listener on 0.0.0.0:{port} failed: {e}");
            }
            result
        }));
    }

    if let Some(port) = config.raw_udp_port {
        let socket = UdpSocket::bind(("0.0.0.0", port))?;
        info!("Raw CAT UDP listener bound on 0.0.0.0:{port}");
        let handle = handle.clone();
        let registry = Rc::new(RefCell::new(ClientRegistry::new()));
        tasks.push(monoio::spawn(async move {
            let result = cat_server::udp::serve(socket, handle, registry).await;
            if let Err(e) = &result {
                error!("Raw CAT UDP listener on 0.0.0.0:{port} failed: {e}");
            }
            result
        }));
    }

    if let Some(port) = config.rigctl_port {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        info!("Rigctld-compatible TCP listener bound on 0.0.0.0:{port} (for WSJT-X)");
        let handle = handle.clone();
        let make_radio = make_radio.clone();
        tasks.push(monoio::spawn(async move {
            let result = rigctl::serve(listener, handle, make_radio).await;
            if let Err(e) = &result {
                error!("Rigctld-compatible TCP listener on 0.0.0.0:{port} failed: {e}");
            }
            result
        }));
    }

    // Consoles. The listener is blocking and lives on its own threads --
    // `cat_native::serve` is `std::net` by design, because the GUI that
    // consumes it has no runtime either. What stays in here is the pump:
    // the only thing that touches the radio, inside the runtime that owns
    // it.
    if let Some(shared) = native_shared.clone() {
        let port = config.native_port.expect("shared implies a port");
        let listener = std::net::TcpListener::bind(("0.0.0.0", port))?;
        info!("Console protocol listener bound on 0.0.0.0:{port}");
        std::thread::spawn(move || {
            if let Err(e) = cat_native::serve(listener, shared) {
                error!("Console protocol listener on 0.0.0.0:{port} failed: {e}");
            }
        });
    }
    if let Some(shared) = native_shared {
        // A high, fixed id: the pump is one long-lived client, not one per
        // connection, and it must not collide with the rigctl listener's
        // sequence which starts at zero and counts up.
        let radio = make_native(BrokerCatSession::new(
            handle.clone(),
            cat_server::ClientId::from_raw(u64::MAX),
        ));
        monoio::spawn(native_bridge::pump(
            shared,
            radio,
            std::time::Duration::from_millis(200),
        ));
    }

    if tasks.is_empty() {
        // The console listener is a thread, not a task, so it never
        // appears in `tasks`. A server bound only to `--console-port` is a
        // perfectly ordinary server -- a GUI and nothing else -- and used
        // to be rejected here as if nothing had been asked for.
        if serving_consoles {
            std::future::pending::<()>().await;
        }
        return Err(io::Error::other(
            "server mode requires at least one of --raw-tcp-port/--raw-udp-port/--rigctl-port/--console-port",
        ));
    }

    // Every listener loop above only returns on a fatal accept()/bind-time
    // error (or never, on the happy path) -- wait for the first one to end,
    // and propagate whatever it returned (already logged above on the
    // `Err` path) instead of hardcoding `Ok(())` regardless of outcome.
    let (result, _index, _remaining) = futures::future::select_all(tasks).await;
    result
}

/// Windows implementation of [`run`] — same signature and behavior as the
/// Linux one from a caller's point of view (same [`ServerConfig`], same
/// three possible listeners, same [`RigctlRadio`] dispatch, including full
/// rigctld/WSJT-X support), but a plain blocking `fn` instead of `async
/// fn`: genuine OS threads instead of `monoio`'s cooperative tasks, since
/// `monoio` cannot compile on Windows at all. Mirrors
/// `cat_server::worker_windows::BrokerWorker::run`'s own precedent that
/// top-level "how do you start this" bootstrapping is expected to differ
/// per platform — see `docs/adr/0006-windows-network-transport.md`'s
/// follow-up note for the full design record.
///
/// Supersedes an earlier stopgap where consuming apps (`ft991a`, `ts570d`)
/// had to hand-roll their own Windows-only fallback that dropped
/// `--rigctl-port` support entirely, because this crate had no Windows
/// backend at all. `--rigctl-port` now works identically on both
/// platforms.
#[cfg(target_os = "windows")]
pub fn run<C, S, R, F>(
    session: S,
    table: &'static CommandTable<C>,
    config: ServerConfig,
    make_radio: F,
) -> io::Result<()>
where
    C: CommandId,
    S: CatSession + Send + 'static,
    S::Error: std::error::Error + 'static,
    R: RigctlRadio + 'static,
    F: Fn(BrokerCatSession) -> R + Clone + Send + 'static,
{
    run_with_native(
        session,
        table,
        config,
        make_radio,
        |_| native_bridge::NoNative,
        None,
    )
}

/// [`run`], also serving the typed console protocol. See the Linux
/// version's doc comment.
///
/// The pump runs on its own thread here rather than as a runtime task,
/// driven by `cat_server::block_on` — the same shape the rest of this
/// crate's Windows path uses, and the reason `NativeRadio` is `?Send`
/// async rather than requiring a full executor.
#[cfg(target_os = "windows")]
pub fn run_with_native<C, S, R, F, N, G>(
    session: S,
    table: &'static CommandTable<C>,
    config: ServerConfig,
    make_radio: F,
    make_native: G,
    native: Option<std::sync::Arc<native_bridge::NativeShared>>,
) -> io::Result<()>
where
    C: CommandId,
    S: CatSession + Send + 'static,
    S::Error: std::error::Error + 'static,
    R: RigctlRadio + 'static,
    F: Fn(BrokerCatSession) -> R + Clone + Send + 'static,
    N: native_bridge::NativeRadio + 'static,
    G: FnOnce(BrokerCatSession) -> N + Send + 'static,
{
    use std::net::{TcpListener, UdpSocket};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;

    use cat_server::ClientRegistry;

    let (worker, handle) = cat_server::build(session, table);
    thread::spawn(move || worker.run());

    // The caller owns this, not us: a radio with a spectrum source has to
    // publish frames into the same cache the listener reads from, and that
    // source is the app's business. `cat-rigctl` orchestrates listeners
    // and has no opinion about where a spectrum comes from.
    let native_shared = native.filter(|_| config.native_port.is_some());

    if let Some(shared) = native_shared.clone() {
        let handle = handle.clone();
        thread::spawn(move || {
            let radio = make_native(BrokerCatSession::new(
                handle,
                cat_server::ClientId::from_raw(u64::MAX),
            ));
            cat_server::block_on::block_on(native_bridge::pump(
                shared,
                radio,
                std::time::Duration::from_millis(200),
            ));
        });
    }

    let (done_tx, done_rx) = mpsc::channel::<io::Result<()>>();
    let mut listener_count = 0;

    if let Some(shared) = native_shared.clone() {
        let port = config.native_port.expect("shared implies a port");
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        info!("Console protocol listener bound on 0.0.0.0:{port}");
        let done_tx = done_tx.clone();
        listener_count += 1;
        thread::spawn(move || {
            let result = cat_native::serve(listener, shared);
            if let Err(e) = &result {
                error!("Console protocol listener on 0.0.0.0:{port} failed: {e}");
            }
            let _ = done_tx.send(result);
        });
    }

    if let Some(port) = config.raw_tcp_port {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        info!("Raw CAT TCP listener bound on 0.0.0.0:{port}");
        let handle = handle.clone();
        let registry = Arc::new(Mutex::new(ClientRegistry::new()));
        let done_tx = done_tx.clone();
        listener_count += 1;
        thread::spawn(move || {
            let result = cat_server::tcp_windows::serve(listener, handle, registry);
            if let Err(e) = &result {
                error!("Raw CAT TCP listener on 0.0.0.0:{port} failed: {e}");
            }
            let _ = done_tx.send(result);
        });
    }

    if let Some(port) = config.raw_udp_port {
        let socket = UdpSocket::bind(("0.0.0.0", port))?;
        info!("Raw CAT UDP listener bound on 0.0.0.0:{port}");
        let handle = handle.clone();
        let registry = Arc::new(Mutex::new(ClientRegistry::new()));
        let done_tx = done_tx.clone();
        listener_count += 1;
        thread::spawn(move || {
            let result = cat_server::udp_windows::serve(socket, handle, registry);
            if let Err(e) = &result {
                error!("Raw CAT UDP listener on 0.0.0.0:{port} failed: {e}");
            }
            let _ = done_tx.send(result);
        });
    }

    if let Some(port) = config.rigctl_port {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        info!("Rigctld-compatible TCP listener bound on 0.0.0.0:{port} (for WSJT-X)");
        let handle = handle.clone();
        let make_radio = make_radio.clone();
        let done_tx = done_tx.clone();
        listener_count += 1;
        thread::spawn(move || {
            let result = rigctl_windows::serve(listener, handle, make_radio);
            if let Err(e) = &result {
                error!("Rigctld-compatible TCP listener on 0.0.0.0:{port} failed: {e}");
            }
            let _ = done_tx.send(result);
        });
    }

    if listener_count == 0 {
        return Err(io::Error::other(
            "server mode requires at least one of --raw-tcp-port/--raw-udp-port/--rigctl-port",
        ));
    }
    drop(done_tx);

    // Wait for the first listener thread to end (accept()/bind-time
    // failure, or never on the happy path), and propagate its result -- the
    // `std` analog of the Linux path's `futures::future::select_all`.
    done_rx
        .recv()
        .unwrap_or(Err(io::Error::other("all listener threads exited")))
}

// Gated to Linux: exercises the `async fn run` implementation via
// `#[monoio::test]`. A Windows-shaped equivalent lives in
// `rigctl_windows`'s own `#[cfg(all(test, target_os = "windows"))]` tests
// (it tests the listener/accept-loop plumbing directly rather than through
// this crate's top-level `run`, since the Windows `run` itself is a thin,
// low-risk orchestration layer over already-tested pieces -- see that
// module's doc).
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use cat_framework::{CommandDefinition, CommandForm, CommandOperation};
    use cat_transport_core::test_support::ScriptedCatSession;

    // In-crate fake `CommandId`/`CommandTable`, mirroring `cat-server`'s
    // own `test_fixtures.rs` -- never a real radio crate's command table.
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

    /// A `RigctlRadio` impl never actually invoked by
    /// `run_with_no_listeners_configured_returns_an_error` (no
    /// `rigctl_port` is configured, so `make_radio` is never called) --
    /// exists purely so `run`'s `R`/`F` type parameters have something
    /// concrete to infer against.
    struct UnusedRadio;

    #[async_trait(?Send)]
    impl RigctlRadio for UnusedRadio {
        type Mode = ();
        type Error = std::convert::Infallible;

        async fn get_vfo_a_hz(&mut self) -> Result<u64, Self::Error> {
            unreachable!("UnusedRadio is never actually invoked")
        }
        async fn set_vfo_a_hz(&mut self, _hz: u64) -> Result<(), Self::Error> {
            unreachable!("UnusedRadio is never actually invoked")
        }
        async fn get_mode(&mut self) -> Result<Self::Mode, Self::Error> {
            unreachable!("UnusedRadio is never actually invoked")
        }
        async fn set_mode(&mut self, _mode: Self::Mode) -> Result<(), Self::Error> {
            unreachable!("UnusedRadio is never actually invoked")
        }
        async fn get_transmitting(&mut self) -> Result<bool, Self::Error> {
            unreachable!("UnusedRadio is never actually invoked")
        }
        async fn transmit(&mut self) -> Result<(), Self::Error> {
            unreachable!("UnusedRadio is never actually invoked")
        }
        async fn receive(&mut self) -> Result<(), Self::Error> {
            unreachable!("UnusedRadio is never actually invoked")
        }
        fn hamlib_mode_name(_mode: Self::Mode) -> &'static str {
            unreachable!("UnusedRadio is never actually invoked")
        }
        fn hamlib_mode_from_name(_name: &str) -> Option<Self::Mode> {
            unreachable!("UnusedRadio is never actually invoked")
        }
        fn freq_range_hz() -> (u64, u64) {
            unreachable!("UnusedRadio is never actually invoked")
        }
    }

    fn make_unused_radio(_session: BrokerCatSession) -> UnusedRadio {
        UnusedRadio
    }

    #[monoio::test(driver = "legacy")]
    async fn run_with_no_listeners_configured_returns_an_error() {
        let session = ScriptedCatSession::new();
        let result = run(session, &TABLE, ServerConfig::default(), make_unused_radio).await;
        assert!(result.is_err());
    }

    // Regression guard, ported from `ts570d`'s server crate (code review
    // 2026-07-25): `run()` used to discard each listener task's
    // `io::Result<()>` via `let _ = ...` and then unconditionally return
    // `Ok(())` from `select_all`, whose `Output` was `()` regardless of
    // *why* a task ended. `run()` now spawns each listener as a task that
    // itself returns `io::Result<()>`, so `select_all` resolves with that
    // real `Result` as its first tuple element and `run()` returns it
    // directly.
    //
    // A true end-to-end test of `run()` hitting this path would require a
    // real, already-bound listener's `accept()`/`recv_from()` to fail with
    // a genuine OS-level error post-bind (e.g. EMFILE, or closing the
    // listening socket's raw fd out from under it) -- confirmed by reading
    // `cat_server::tcp::serve`/`udp::serve` and `rigctl::serve` directly
    // that their `Err` return *only* comes from the top-level
    // `accept()`/`recv_from()` call, never from session/broker-level
    // failures (those are handled per-connection and never propagate out
    // of the accept loop). That is not reachable via
    // `ScriptedCatSession`/broker setup, and deliberately closing a raw fd
    // out from under a live `TcpListener` in this shared, multi-threaded
    // test binary risks a double-close hitting an unrelated fd reused by a
    // concurrently-running test -- not safely deterministic here. So this
    // test instead locks in the exact propagation mechanism `run()`
    // depends on (a `monoio::spawn`ed task returning `io::Result<()>`,
    // awaited through `futures::future::select_all`) using a single
    // synthetic failing task shaped like a real listener task,
    // deterministically (no race: only one task in the vec) -- a future
    // accidental reintroduction of `let _ = ...` around a listener task
    // would be caught here.
    #[monoio::test(driver = "legacy")]
    async fn select_all_over_a_failing_listener_task_propagates_its_error() {
        let failing_task: monoio::task::JoinHandle<io::Result<()>> =
            monoio::spawn(async { Err(io::Error::other("simulated post-bind listener failure")) });

        let (result, index, remaining) = futures::future::select_all(vec![failing_task]).await;

        let err = result.expect_err("failing listener task's Err must propagate, not be lost");
        assert_eq!(err.to_string(), "simulated post-bind listener failure");
        assert_eq!(index, 0);
        assert!(remaining.is_empty());
    }
}
