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

//! Pure, platform-neutral envelope encode/decode logic and the client-side
//! deduplication cache.
//!
//! Extracted out of `session.rs` (`docs/adr/0006-windows-network-transport.md`)
//! so both the Linux `monoio`-based [`crate::session`] and the Windows
//! worker-thread-based [`crate::windows`] can share one definition of the
//! wire format and dedup policy instead of each re-deriving it -- mirrors
//! `docs/adr/0004-windows-serial-backend.md` §2's extraction of
//! `SerialConfig`/`Parity`/`FlowControl` into `cat-transport-serial::config`.
//! `encode_envelope`/`decode_envelope` were already pure functions with no
//! `monoio` dependency before this extraction; this module just gives them
//! (and the constants/error type/dedup cache they travel with) a home that
//! is not itself gated to `session.rs`'s Linux-only module.
//!
//! See [`crate::session`]'s module doc for the full wire format writeup.

use std::collections::VecDeque;
use std::hash::{BuildHasher, Hasher};
use std::net::SocketAddr;
use std::time::Duration;

use thiserror::Error;

/// Width, in bytes, of the `session_id` field.
pub const SESSION_ID_LEN: usize = 8;
/// Width, in bytes, of the `request_id` field.
pub const REQUEST_ID_LEN: usize = 8;
/// Total envelope header width in bytes (`session_id` + `request_id`).
pub const ENVELOPE_HEADER_LEN: usize = SESSION_ID_LEN + REQUEST_ID_LEN;

/// Maximum payload length, in bytes, that a `UdpCatSession` will send or
/// accept in a single envelope. See [`crate::session`]'s module doc for the
/// full sizing rationale (MTU/fragmentation avoidance).
pub const MAX_PAYLOAD_SIZE: usize = 1024;

/// Total datagram size (header + max payload) a session allocates for each
/// `recv_from` call, on either platform.
pub const MAX_DATAGRAM_SIZE: usize = ENVELOPE_HEADER_LEN + MAX_PAYLOAD_SIZE;

/// Maximum number of completed `request_id`s the client-side deduplication
/// cache retains before evicting the oldest (FIFO). See [`crate::session`]'s
/// module doc's "Deduplication cache" section for the reasoning.
pub const DEDUP_CACHE_CAPACITY: usize = 32;

/// Default `response_timeout` for `UdpCatSession::bind_to`.
pub const DEFAULT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);

/// Errors from a `UdpCatSession`'s envelope I/O -- shared by both platform
/// backends.
#[derive(Debug, Error)]
pub enum UdpSessionError {
    /// The underlying UDP socket failed to send or receive.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// No response envelope matching the request just sent arrived from
    /// `peer` within `timeout`.
    #[error("no response received from {peer} within {timeout:?}")]
    Timeout {
        /// The peer address this session was waiting on a response from.
        peer: SocketAddr,
        /// The configured response timeout that elapsed.
        timeout: Duration,
    },

    /// A request payload's length exceeded [`MAX_PAYLOAD_SIZE`]. Nothing
    /// was sent.
    #[error("payload length {len} exceeds max payload size {max} bytes")]
    PayloadTooLarge {
        /// The length of the payload that was rejected.
        len: usize,
        /// The configured maximum ([`MAX_PAYLOAD_SIZE`]).
        max: usize,
    },
}

/// Generate a randomized session id without an external `rand` dependency
/// (not on this crate's authorized dependency list). `RandomState` seeds a
/// `SipHash` instance from OS-provided entropy on construction; calling
/// `finish()` on a freshly built, unwritten hasher yields a value derived
/// from that random seed. This is not cryptographic-quality randomness and
/// is not used as one -- it only needs to make collisions between
/// concurrently-alive sessions on the same host implausible.
pub fn random_session_id() -> u64 {
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}

/// Encode one envelope: `session_id` (8 bytes BE) + `request_id` (8 bytes
/// BE) + `payload` verbatim. Rejects a payload longer than
/// [`MAX_PAYLOAD_SIZE`] before allocating or sending anything.
pub fn encode_envelope(
    session_id: u64,
    request_id: u64,
    payload: &[u8],
) -> Result<Vec<u8>, UdpSessionError> {
    if payload.len() > MAX_PAYLOAD_SIZE {
        return Err(UdpSessionError::PayloadTooLarge {
            len: payload.len(),
            max: MAX_PAYLOAD_SIZE,
        });
    }

    let mut datagram = Vec::with_capacity(ENVELOPE_HEADER_LEN + payload.len());
    datagram.extend_from_slice(&session_id.to_be_bytes());
    datagram.extend_from_slice(&request_id.to_be_bytes());
    datagram.extend_from_slice(payload);
    Ok(datagram)
}

/// Decode one envelope from a received datagram's bytes. Returns `None` if
/// `datagram` is too short to contain a full header -- treated as noise by
/// callers, not an error.
pub fn decode_envelope(datagram: &[u8]) -> Option<(u64, u64, &[u8])> {
    if datagram.len() < ENVELOPE_HEADER_LEN {
        return None;
    }

    let session_id = u64::from_be_bytes(
        datagram[0..SESSION_ID_LEN]
            .try_into()
            .expect("slice is exactly SESSION_ID_LEN bytes"),
    );
    let request_id = u64::from_be_bytes(
        datagram[SESSION_ID_LEN..ENVELOPE_HEADER_LEN]
            .try_into()
            .expect("slice is exactly REQUEST_ID_LEN bytes"),
    );
    let payload = &datagram[ENVELOPE_HEADER_LEN..];
    Some((session_id, request_id, payload))
}

/// The client-side deduplication cache: a bounded FIFO of `request_id`s a
/// `UdpCatSession` has already completed. See [`crate::session`]'s module
/// doc's "Deduplication cache" section for the full reasoning (this cache
/// does not change accept/reject behavior on its own -- `request_id`
/// mismatch is already sufficient -- it exists to make the discard explicit
/// and bound memory growth). Shared by both platform backends' internal
/// session state.
#[derive(Debug, Default)]
pub(crate) struct RequestIdCache {
    entries: VecDeque<u64>,
}

impl RequestIdCache {
    pub(crate) fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(DEDUP_CACHE_CAPACITY),
        }
    }

    /// Record `request_id` as completed, evicting the oldest entry first if
    /// already at [`DEDUP_CACHE_CAPACITY`].
    pub(crate) fn remember_completed(&mut self, request_id: u64) {
        if self.entries.len() >= DEDUP_CACHE_CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(request_id);
    }

    /// `true` if `request_id` is one this cache has already recorded.
    pub(crate) fn is_known_duplicate(&self, request_id: u64) -> bool {
        self.entries.contains(&request_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip() {
        let datagram = encode_envelope(0x0102_0304_0506_0708, 42, b"FA;").unwrap();
        assert_eq!(datagram.len(), ENVELOPE_HEADER_LEN + 3);

        let (session_id, request_id, payload) = decode_envelope(&datagram).unwrap();
        assert_eq!(session_id, 0x0102_0304_0506_0708);
        assert_eq!(request_id, 42);
        assert_eq!(payload, b"FA;");
    }

    #[test]
    fn encode_rejects_oversized_payload() {
        let oversized = vec![0u8; MAX_PAYLOAD_SIZE + 1];
        let result = encode_envelope(1, 1, &oversized);
        match result {
            Err(UdpSessionError::PayloadTooLarge { len, max }) => {
                assert_eq!(len, MAX_PAYLOAD_SIZE + 1);
                assert_eq!(max, MAX_PAYLOAD_SIZE);
            }
            other => panic!("expected PayloadTooLarge, got {:?}", other),
        }
    }

    #[test]
    fn decode_rejects_datagram_shorter_than_header() {
        assert!(decode_envelope(&[0u8; ENVELOPE_HEADER_LEN - 1]).is_none());
        assert!(decode_envelope(&[]).is_none());
    }

    #[test]
    fn decode_accepts_header_only_datagram_as_empty_payload() {
        let datagram = encode_envelope(7, 8, b"").unwrap();
        let (session_id, request_id, payload) = decode_envelope(&datagram).unwrap();
        assert_eq!(session_id, 7);
        assert_eq!(request_id, 8);
        assert!(payload.is_empty());
    }

    #[test]
    fn dedup_cache_recognizes_a_completed_request_id() {
        let mut cache = RequestIdCache::new();
        assert!(!cache.is_known_duplicate(5));
        cache.remember_completed(5);
        assert!(cache.is_known_duplicate(5));
        assert!(!cache.is_known_duplicate(6));
    }

    #[test]
    fn dedup_cache_evicts_oldest_beyond_capacity() {
        let mut cache = RequestIdCache::new();
        for id in 1..=DEDUP_CACHE_CAPACITY as u64 {
            cache.remember_completed(id);
        }
        assert!(cache.is_known_duplicate(1), "oldest entry not yet evicted");

        cache.remember_completed(DEDUP_CACHE_CAPACITY as u64 + 1);
        assert!(
            !cache.is_known_duplicate(1),
            "oldest entry should have been evicted once capacity was exceeded"
        );
        assert!(cache.is_known_duplicate(2), "second-oldest should survive");
        assert!(cache.is_known_duplicate(DEDUP_CACHE_CAPACITY as u64 + 1));
    }
}
