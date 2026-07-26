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

//! Pure, platform-neutral length-prefixed frame encode/decode logic.
//!
//! Extracted out of `session.rs` (`docs/adr/0006-windows-network-transport.md`)
//! so both the Linux `monoio`-based [`crate::session`] and the Windows
//! worker-thread-based [`crate::windows`] can share one definition of the
//! wire format instead of each re-deriving it — mirrors
//! `docs/adr/0004-windows-serial-backend.md` §2's extraction of
//! `SerialConfig`/`Parity`/`FlowControl` into `cat-transport-serial::config`
//! exactly: this module has no platform-specific code at all (no socket type
//! appears anywhere below), so it lives here ungated.
//!
//! See [`crate::session`]'s module doc for the full wire format writeup —
//! the length prefix is a 4-byte big-endian `u32` counting only the payload
//! bytes that follow it, bounded by [`MAX_FRAME_SIZE`].

use std::io;

use thiserror::Error;

/// Maximum payload length, in bytes, that a `TcpCatSession` will write or
/// accept in a single frame. See [`crate::session`]'s module doc for the
/// full sizing rationale (64 KiB judgment call, three orders of magnitude of
/// headroom over known CAT frame sizes).
pub const MAX_FRAME_SIZE: u32 = 64 * 1024;

/// Errors from frame I/O — shared by both platform backends.
#[derive(Debug, Error)]
pub enum TcpSessionError {
    /// The underlying TCP connection failed while reading or writing a
    /// frame -- includes a peer disconnecting mid-frame (surfaces as
    /// `io::ErrorKind::UnexpectedEof` on both platforms, since neither
    /// backend can complete a partial read when the connection closes
    /// before the declared number of bytes arrive).
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// A frame's declared length prefix exceeded [`MAX_FRAME_SIZE`]. The
    /// connection's byte stream is left mid-frame (the payload was never
    /// read) -- callers should treat this session as unusable and drop the
    /// connection rather than attempting further requests on it.
    #[error("frame length {len} exceeds max frame size {max} bytes")]
    FrameTooLarge {
        /// The length the peer declared.
        len: u32,
        /// The configured maximum ([`MAX_FRAME_SIZE`]).
        max: u32,
    },
}

/// Reject a declared frame length greater than [`MAX_FRAME_SIZE`], before a
/// caller attempts to read (or allocate a buffer for) the declared payload.
pub fn check_frame_len(len: u32) -> Result<(), TcpSessionError> {
    if len > MAX_FRAME_SIZE {
        return Err(TcpSessionError::FrameTooLarge {
            len,
            max: MAX_FRAME_SIZE,
        });
    }
    Ok(())
}

/// Encode one complete frame: a 4-byte big-endian length prefix (payload
/// bytes only) followed by `payload` verbatim. Rejects a payload wider than
/// [`MAX_FRAME_SIZE`] before allocating anything.
pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>, TcpSessionError> {
    let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    check_frame_len(len)?;

    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

/// Decode a 4-byte big-endian length prefix already read into `len_buf`,
/// checking it against [`MAX_FRAME_SIZE`].
pub fn decode_len_prefix(len_buf: [u8; 4]) -> Result<u32, TcpSessionError> {
    let len = u32::from_be_bytes(len_buf);
    check_frame_len(len)?;
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_frame_round_trips_through_decode_len_prefix() {
        let frame = encode_frame(b"FA;").unwrap();
        assert_eq!(frame.len(), 4 + 3);
        let len = decode_len_prefix(frame[0..4].try_into().unwrap()).unwrap();
        assert_eq!(len as usize, 3);
        assert_eq!(&frame[4..], b"FA;");
    }

    #[test]
    fn encode_frame_rejects_oversized_payload() {
        let oversized = vec![0u8; MAX_FRAME_SIZE as usize + 1];
        match encode_frame(&oversized) {
            Err(TcpSessionError::FrameTooLarge { len, max }) => {
                assert_eq!(len, MAX_FRAME_SIZE + 1);
                assert_eq!(max, MAX_FRAME_SIZE);
            }
            other => panic!("expected FrameTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn decode_len_prefix_rejects_oversized_declared_length() {
        let declared = (MAX_FRAME_SIZE + 1).to_be_bytes();
        match decode_len_prefix(declared) {
            Err(TcpSessionError::FrameTooLarge { len, max }) => {
                assert_eq!(len, MAX_FRAME_SIZE + 1);
                assert_eq!(max, MAX_FRAME_SIZE);
            }
            other => panic!("expected FrameTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn encode_frame_allows_zero_length_payload() {
        let frame = encode_frame(b"").unwrap();
        assert_eq!(frame, 0u32.to_be_bytes().to_vec());
    }
}
