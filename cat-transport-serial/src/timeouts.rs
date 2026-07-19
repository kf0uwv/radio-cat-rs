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

//! Shared serial read timeout, used by both platform backends.
//!
//! Moved out of `io_uring.rs` (same value, same `#[cfg(test)]`/
//! `#[cfg(not(test))]` split, same doc comment) per
//! `docs/adr/0004-windows-serial-backend.md` §3's Windows `SetCommTimeouts`
//! mapping: `ReadTotalTimeoutConstant = READ_TIMEOUT.as_millis()`, "reusing
//! the existing 2s-production/100ms-test split already in `io_uring.rs`" --
//! `planning/architect/task_plan.md`'s Task 7 dispatch text is explicit that
//! this must not become a second, parallel constant. Mirrors the `config.rs`
//! extraction precedent from ADR 0004 dispatch queue Task 6.

/// Timeout for `Transport::read` retries on EAGAIN (Linux) / the maximum
/// wait of a single blocking `ReadFile` call (Windows, via
/// `COMMTIMEOUTS::ReadTotalTimeoutConstant`).
///
/// 2 seconds is generous for TS-570D/FT-991A command-response latency
/// (typically < 100 ms).
///
/// In tests, a shorter timeout is used to keep the test suite fast.
#[cfg(not(test))]
pub(crate) const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Short read timeout used in tests to avoid multi-second waits in the test suite.
#[cfg(test)]
pub(crate) const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);
