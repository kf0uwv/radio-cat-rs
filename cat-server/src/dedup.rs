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

//! [`DedupCache`]: the server-side UDP deduplication cache.
//!
//! Extracted out of `udp.rs` (`docs/adr/0006-windows-network-transport.md`)
//! so both the Linux (`monoio`-based) [`crate::udp`] and the Windows
//! (OS-thread-based) [`crate::udp_windows`] listeners can share one
//! definition instead of each re-deriving it -- pure `std`
//! (`HashMap`/`VecDeque`/`SocketAddr`), no platform-specific code, so it
//! lives here ungated, mirroring `cat-transport-tcp`'s/
//! `cat-transport-udp`'s `codec.rs` extraction.
//!
//! **Not** the same mechanism as `cat-transport-udp::UdpCatSession`'s own
//! client-side cache (which only recognizes "have I already seen the
//! answer to this," never replays anything). This server serves *many*
//! client sessions on one bound socket, so a duplicate incoming request
//! must be answered from a cache **without re-executing it against the
//! physical radio**. Keyed by `(peer_addr, session_id, request_id)` -- see
//! [`crate::udp`]'s module doc for the full key-choice rationale.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;

/// Bounded FIFO capacity of the server-side dedup cache.
pub const DEDUP_CACHE_CAPACITY: usize = 256;

/// Server-side deduplication cache: caches the **actual response bytes**
/// for a `(peer_addr, session_id, request_id)` key, bounded FIFO.
#[derive(Default)]
pub struct DedupCache {
    order: VecDeque<(SocketAddr, u64, u64)>,
    responses: HashMap<(SocketAddr, u64, u64), Vec<u8>>,
}

impl DedupCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The cached response for this key, if this exact request has already
    /// been answered.
    pub fn get(&self, peer: SocketAddr, session_id: u64, request_id: u64) -> Option<&Vec<u8>> {
        self.responses.get(&(peer, session_id, request_id))
    }

    /// Record the response for this key, evicting the oldest entry first
    /// if already at [`DEDUP_CACHE_CAPACITY`].
    pub fn insert(
        &mut self,
        peer: SocketAddr,
        session_id: u64,
        request_id: u64,
        response: Vec<u8>,
    ) {
        let key = (peer, session_id, request_id);
        if !self.responses.contains_key(&key) {
            if self.order.len() >= DEDUP_CACHE_CAPACITY {
                if let Some(oldest) = self.order.pop_front() {
                    self.responses.remove(&oldest);
                }
            }
            self.order.push_back(key);
        }
        self.responses.insert(key, response);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_cache_recognizes_a_cached_response() {
        let peer: SocketAddr = "127.0.0.1:9999".parse().unwrap();
        let mut cache = DedupCache::new();
        assert!(cache.get(peer, 1, 1).is_none());

        cache.insert(peer, 1, 1, b"FA00014250000;".to_vec());
        assert_eq!(cache.get(peer, 1, 1), Some(&b"FA00014250000;".to_vec()));
    }

    #[test]
    fn dedup_cache_distinguishes_same_peer_addr_different_session_id() {
        let peer: SocketAddr = "127.0.0.1:9999".parse().unwrap();
        let mut cache = DedupCache::new();
        cache.insert(peer, /* session_id */ 1, 1, b"OLD".to_vec());

        assert!(cache.get(peer, /* session_id */ 2, 1).is_none());
    }

    #[test]
    fn dedup_cache_evicts_oldest_beyond_capacity() {
        let peer: SocketAddr = "127.0.0.1:9999".parse().unwrap();
        let mut cache = DedupCache::new();
        for id in 1..=DEDUP_CACHE_CAPACITY as u64 {
            cache.insert(peer, 1, id, vec![id as u8]);
        }
        assert!(cache.get(peer, 1, 1).is_some(), "oldest not yet evicted");

        cache.insert(peer, 1, DEDUP_CACHE_CAPACITY as u64 + 1, vec![0xFF]);
        assert!(
            cache.get(peer, 1, 1).is_none(),
            "oldest entry should have been evicted once capacity was exceeded"
        );
        assert!(
            cache.get(peer, 1, 2).is_some(),
            "second-oldest should survive"
        );
    }
}
