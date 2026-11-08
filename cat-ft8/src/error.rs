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

//! The crate's one error type. Hand-written `Display` rather than
//! `thiserror`, because the core is `no_std` and the workspace's
//! `thiserror` 1.x needs `std`.

use core::fmt;

use crate::message::MessageType;

/// Why packing, unpacking or synthesis failed.
///
/// Non-exhaustive: later releases may add variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The message text is empty.
    Empty,
    /// The text fits no message type this crate packs (for example free
    /// text longer than 13 characters, or a character outside the FT8
    /// alphabet).
    NotEncodable,
    /// A callsign that cannot be sent: too long, bad characters, or a
    /// combination (such as `/P` with `/R`) the protocol cannot carry.
    InvalidCallsign,
    /// A signal report (or, in type 5, a serial number) outside the range
    /// its message type can carry.
    InvalidReport,
    /// A defined message type this release does not pack or unpack yet
    /// (ARRL Field Day, ARRL RTTY Roundup).
    Unsupported(MessageType),
    /// A payload of an unassigned type, or whose fields hold values no
    /// message defines.
    InvalidPayload,
    /// A tone index above 7.
    InvalidSymbol,
    /// A base frequency that is not finite, not positive, or would put the
    /// top tone at or above the Nyquist frequency.
    FrequencyOutOfRange,
    /// A time offset or amplitude that is NaN or infinite.
    NonFinite,
    /// A decoder band that is not a whole number of 3.125 Hz bins, is
    /// empty, or reaches the Nyquist frequency.
    InvalidBand,
    /// A caller-provided buffer smaller than the configuration needs.
    BufferTooSmall,
    /// A decoder configuration out of range (see `DecodeConfig`).
    InvalidConfig,
    /// A candidate result pushed out of candidate-index order, or a
    /// decode pushed under another candidate's index.
    OutOfOrder,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Empty => f.write_str("empty message"),
            Error::NotEncodable => f.write_str("message fits no FT8 message type"),
            Error::InvalidCallsign => f.write_str("callsign cannot be encoded"),
            Error::InvalidReport => f.write_str("signal report out of range"),
            Error::Unsupported(t) => write!(f, "message type {t} is not supported yet"),
            Error::InvalidPayload => f.write_str("payload holds undefined field values"),
            Error::InvalidSymbol => f.write_str("tone index above 7"),
            Error::FrequencyOutOfRange => f.write_str("base frequency out of range"),
            Error::NonFinite => f.write_str("time offset or amplitude is not finite"),
            Error::InvalidBand => f.write_str("invalid decoder band"),
            Error::BufferTooSmall => f.write_str("buffer too small for the configuration"),
            Error::InvalidConfig => f.write_str("invalid decoder configuration"),
            Error::OutOfOrder => f.write_str("candidate result out of order"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}
