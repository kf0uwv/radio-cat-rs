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

//! Transport error types.
//!
//! Moved out of `ts570d`'s `framework/src/errors.rs`, which also defined
//! `FrameworkError`/`FrameworkResult` (state-machine-only errors that stay
//! behind in `ts570d`, per `planning/architect/findings.md` §9 — they are
//! not part of this crate's scope).

use thiserror::Error;

/// Errors from [`crate::transport::Transport`] implementations (serial port
/// I/O today; future network transports reuse this type too).
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Port not open")]
    NotOpen,
    #[error("Write timeout")]
    WriteTimeout,
    #[error("Read timeout")]
    ReadTimeout,
    #[error("Transport error: {0}")]
    Other(String),
}
