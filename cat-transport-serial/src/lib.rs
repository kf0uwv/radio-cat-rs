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
//! This crate depends only on `cat-transport-core` in this workspace (never
//! on `cat-framework` directly — `ResponseDisposition`/`ProtocolErrorKind`
//! are reached via `cat-transport-core`'s re-export), per the dependency
//! rules in `.claude/agents/cat_transport.md`.

pub mod io_uring;
pub mod session;

pub use io_uring::{FlowControl, Parity, SerialConfig, SerialPort};
pub use session::SerialCatSession;

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
