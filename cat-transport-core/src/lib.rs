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

//! Transport-level and session-level trait abstractions for CAT protocol
//! communication.
//!
//! Extracted from `ts570d`'s `framework/src/transport.rs` (the `Transport`
//! trait), `framework/src/session.rs` (the `CatSession` trait only —
//! `SerialCatSession` moved to `cat-transport-serial`),
//! `framework/src/test_support.rs` (`Exchange`/`ScriptedCatSession`/the
//! `conformance` module), and the `TransportError` variant of
//! `framework/src/errors.rs` (commit `1585e1e`,
//! `refactor/generic-cat-framework` branch), per
//! `docs/adr/0001-scope-and-crate-boundaries.md` (as amended) and
//! `docs/adr/0002-async-runtime-binding-for-transport-crates.md`.
//!
//! This crate provides:
//! - [`Transport`] — byte-level `read`/`write`/`flush` I/O primitive,
//!   framing-agnostic. Assumes neither a Unix file descriptor nor a
//!   persistent connection.
//! - [`CatSession`] — request/response abstraction above `Transport`. Every
//!   concrete transport crate (`cat-transport-serial`, and future
//!   `cat-transport-tcp`/`cat-transport-udp`) implements this with its own
//!   framing — never inherited from another transport's strategy.
//! - [`TransportError`] — errors from `Transport` implementations.
//! - [`ModemControlLines`] — optional capability for direct RS-232 modem
//!   control/status line access (RTS/DTR/CTS/DSR/DCD), independent of
//!   byte-level CAT framing. Not every transport has physical modem lines
//!   (TCP/UDP do not), so this is a separate, additively-implemented trait
//!   — never folded into `Transport`/`CatSession`.
//! - [`test_support`] — [`Exchange`]/[`ScriptedCatSession`], an in-memory
//!   `CatSession` test double, plus the [`conformance`] module of reusable
//!   `CatSession` behavior checks that any transport crate's own test suite
//!   can call unchanged.
//! - [`completion`] — a portable, single-slot completion primitive (moved
//!   here from `cat-transport-serial::oneshot` per
//!   `docs/adr/0006-windows-network-transport.md`) shared by every Windows
//!   transport backend (`cat-transport-serial`, `cat-transport-tcp`,
//!   `cat-transport-udp`) that drives a dedicated background `std::thread` +
//!   `async fn` completion boundary, instead of each crate hand-rolling its
//!   own copy.
//!
//! # `cat-framework` dependency
//!
//! Per ADR 0001 Amendment 2, this crate takes a one-way dependency on
//! `cat-framework` to reuse [`ResponseDisposition`]/[`ProtocolErrorKind`]
//! (re-exported below) rather than duplicating a parallel type —
//! `CatSession::execute` answers exactly the same question a server-side
//! `CatRadio` dispatch does. `cat-framework` itself still depends on nothing
//! in this workspace, so the dependency DAG's essential property (the
//! generic engine at the bottom) survives.
//!
//! # `monoio` runtime binding
//!
//! Per ADR 0002, the `monoio`/`#[async_trait(?Send)]` binding is retained,
//! and `cat-transport-core` is now the crate that owns it: the `monoio`
//! convenience re-export that used to live in `ts570d`'s
//! `framework/src/lib.rs` moves to this crate's root. `monoio` is a
//! Linux-only dependency (see `Cargo.toml`), so the re-export is
//! `cfg`-gated to match.

pub mod completion;
pub mod errors;
pub mod modem;
pub mod session;
pub mod test_support;
pub mod timeout;
pub mod transport;

pub use cat_framework::{ProtocolErrorKind, ResponseDisposition};
pub use errors::TransportError;
pub use modem::ModemControlLines;
pub use session::CatSession;
pub use test_support::{conformance, Exchange, ScriptedCatSession};
pub use transport::Transport;

// Re-export monoio runtime bits for downstream crates' convenience, mirroring
// `ts570d`'s `framework/src/lib.rs` re-export (see ADR 0002's Consequences).
// Linux-only, matching the `monoio` dependency's own target gating.
#[cfg(target_os = "linux")]
pub use monoio::io::{AsyncReadRent, AsyncWriteRent};
#[cfg(target_os = "linux")]
pub use monoio::RuntimeBuilder;
