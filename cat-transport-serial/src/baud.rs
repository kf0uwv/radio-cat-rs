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

//! Shared, platform-neutral baud-rate validation.
//!
//! Per `docs/adr/0004-windows-serial-backend.md` §5: Windows' `DCB.BaudRate`
//! accepts an arbitrary `u32` directly, more permissively than Linux's fixed
//! `nix::sys::termios::BaudRate` enum. This crate deliberately keeps both
//! backends behaviorally identical here -- reusing Linux's validated rate
//! set on Windows too, rather than letting Windows accept values Linux
//! would reject -- "a deliberate cross-platform *consistency* choice, not a
//! Win32 requirement, so `SerialConfig::default()` and every documented
//! supported rate behave identically on both platforms" (ADR 0004 §5).
//!
//! This module is the single source of truth for "which `u32` baud-rate
//! values this crate accepts." The Linux backend (`io_uring.rs`) validates
//! here, then maps the validated value to `nix::sys::termios::BaudRate`
//! (a Linux-only enum type that has no reason to live in this
//! platform-neutral module); the Windows backend (`windows.rs`) validates
//! here and writes the validated `u32` directly into `DCB.BaudRate`. Moved
//! out of `io_uring.rs` (was `baud_rate_from_u32`, returning
//! `nix::sys::termios::BaudRate`) rather than duplicated, mirroring the
//! `config.rs` extraction precedent from ADR 0004 dispatch queue Task 6.

use crate::SerialError;

/// Baud rates this crate accepts, identically on both platforms. Windows'
/// `DCB.BaudRate` would legally accept any `u32`; this crate restricts it to
/// exactly the set Linux's `nix::sys::termios::BaudRate` enum supports.
pub(crate) const SUPPORTED_BAUD_RATES: &[u32] =
    &[1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400];

/// Validate `baud` against [`SUPPORTED_BAUD_RATES`], returning it unchanged
/// on success so callers can chain this directly into whatever
/// platform-specific type they need next (an enum variant on Linux, a raw
/// `u32` written into `DCB.BaudRate` on Windows).
pub(crate) fn validate_baud_rate(baud: u32) -> Result<u32, SerialError> {
    if SUPPORTED_BAUD_RATES.contains(&baud) {
        Ok(baud)
    } else {
        Err(SerialError::InvalidConfig(format!(
            "Unsupported baud rate: {}",
            baud
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_every_supported_rate() {
        // `Result<u32, SerialError>` isn't `PartialEq` (`SerialError` only
        // derives `Debug`/`thiserror::Error`), so compare via `unwrap()`
        // rather than `assert_eq!(..., Ok(rate))`.
        for &rate in SUPPORTED_BAUD_RATES {
            assert_eq!(validate_baud_rate(rate).unwrap(), rate);
        }
    }

    #[test]
    fn rejects_unsupported_rate() {
        let err = validate_baud_rate(12345).expect_err("12345 is not a supported rate");
        assert!(matches!(err, SerialError::InvalidConfig(_)));
    }
}
