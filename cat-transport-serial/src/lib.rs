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

//! Serial CAT transport.
//!
//! Extracted from two `ts570d` sources (commit `1585e1e`,
//! `refactor/generic-cat-framework` branch), per
//! `docs/adr/0001-scope-and-crate-boundaries.md` Amendment 3 and
//! `planning/architect/findings.md` §6 — `framework/src/session.rs`'s
//! `SerialCatSession<T: Transport>` alone has no hardware behind it; the
//! concrete io_uring implementation lives in `ts570d`'s separate `serial`
//! crate:
//!
//! - `framework/src/session.rs` → [`session::SerialCatSession`], the
//!   generic read-until-`;` framing wrapper (moved to
//!   `cat-transport-core::CatSession` here).
//! - `serial/src/io_uring.rs` + `serial/src/lib.rs` → [`io_uring::SerialPort`]
//!   (+ [`SerialConfig`], [`Parity`], [`FlowControl`]), the concrete
//!   `Transport` implementation over a real serial port/PTY using monoio's
//!   io_uring driver and `nix`/`libc` termios plumbing.
//!
//! Per `docs/adr/0004-windows-serial-backend.md` §2, [`SerialConfig`],
//! [`Parity`], and [`FlowControl`] (pure data, no platform-specific code)
//! now live in the ungated [`config`] module instead of `io_uring.rs`, so a
//! future Windows backend can share them without duplication — same fields,
//! same `Default` impl, same re-export shape from this root. The private
//! `oneshot` module is a portable single-slot completion primitive, an
//! internal implementation detail for a future Windows worker-thread design
//! (ADR 0004 §1) — not part of this crate's public API. The private `baud`
//! and `timeouts` modules are likewise pure, ungated, platform-neutral
//! implementation details (the shared baud-rate-validation set and read
//! timeout, ADR 0004 §5/§3) consulted by both `io_uring.rs` and
//! `windows.rs`, not part of this crate's public API.
//!
//! Per ADR 0004 §2/§3 (Windows serial backend, dispatch queue Task 7): the
//! Linux `io_uring` module and its `SerialPort`/`Transport` re-export are
//! now explicitly `#[cfg(target_os = "linux")]`-gated (previously
//! implicitly Linux-only only because nothing else in the workspace built
//! for another target), and a mirror-image `#[cfg(target_os =
//! "windows")]`-gated `windows` module provides the Windows COM-port
//! backend's `SerialPort::open`/DCB-configuration/`SetCommTimeouts` path.
//! Per ADR 0004 §1/§4 (dispatch queue Task 8): `windows::SerialPort` now
//! also implements `Transport` (via a dedicated worker thread driving
//! blocking `ReadFile`/`WriteFile`, per the `oneshot.rs` completion
//! primitive) and `ModemControlLines` (direct `EscapeCommFunction`/
//! `GetCommModemStatus` calls), and is re-exported from this crate root
//! exactly like `io_uring::SerialPort` -- exactly one `SerialPort` type
//! compiles per target.
//!
//! This crate depends only on `cat-transport-core` in this workspace (never
//! on `cat-framework` directly — `ResponseDisposition`/`ProtocolErrorKind`
//! are reached via `cat-transport-core`'s re-export), per the dependency
//! rules in `.claude/agents/cat_transport.md`.

mod baud;
pub mod config;
#[cfg(target_os = "linux")]
pub mod io_uring;
mod oneshot;
pub mod session;
mod timeouts;
#[cfg(target_os = "windows")]
pub mod windows;

pub use config::{FlowControl, Parity, SerialConfig};
#[cfg(target_os = "linux")]
pub use io_uring::SerialPort;
pub use session::SerialCatSession;
#[cfg(target_os = "windows")]
pub use windows::SerialPort;

/// Serial communication errors (device open/configuration failures) —
/// distinct from [`cat_transport_core::TransportError`], which covers
/// runtime I/O failures during `Transport::{read,write,flush}`.
#[derive(Debug, thiserror::Error)]
pub enum SerialError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),
}

/// Result type for serial configuration/open operations.
pub type SerialResult<T> = Result<T, SerialError>;
