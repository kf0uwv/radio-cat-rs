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

//! Client session management: tracking which remote client sent what,
//! entirely separate from — and invisible to — [`crate::broker::Broker`]
//! and the physical radio session it owns.
//!
//! [`ClientId`] is bookkeeping only: no authentication, no per-client
//! behavior gating, nothing the physical `CatClient`/`CatRadio` layer ever
//! sees (a [`crate::broker::Job`] carries one for logging/observability;
//! `Broker::dispatch` never inspects it). This keeps the contract clean for
//! a future consuming application with a real `CatRadio` state machine
//! downstream — that state machine gains no client-id/broker concepts, per
//! this crate's charter.

use std::collections::HashSet;

/// Opaque identifier for one connected remote client (one TCP connection,
/// or one distinct UDP peer address).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(u64);

impl ClientId {
    /// Construct a `ClientId` from a raw value. Exposed for tests and for
    /// callers that already have their own id scheme (e.g. reusing a UDP
    /// peer address's hash) — [`ClientRegistry::register`] is the normal
    /// way to obtain one.
    pub fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// The raw value, for logging.
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// Tracks which [`ClientId`]s are currently connected.
///
/// Deliberately minimal: a monotonically-assigned id plus a set of
/// currently-active ids. No metadata (peer address, protocol, connect time)
/// is required by this crate's charter today; a listener that wants richer
/// bookkeeping can key its own side-table by `ClientId`.
#[derive(Debug, Default)]
pub struct ClientRegistry {
    next_id: u64,
    active: HashSet<ClientId>,
}

impl ClientRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Assign a fresh `ClientId` and mark it active.
    pub fn register(&mut self) -> ClientId {
        let id = ClientId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.active.insert(id);
        id
    }

    /// Mark `id` no longer active (e.g. a TCP connection closed).
    pub fn unregister(&mut self, id: ClientId) {
        self.active.remove(&id);
    }

    /// Whether `id` is currently marked active.
    pub fn is_active(&self, id: ClientId) -> bool {
        self.active.contains(&id)
    }

    /// Number of currently-active clients.
    pub fn active_count(&self) -> usize {
        self.active.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_assigns_distinct_ids_and_marks_active() {
        let mut registry = ClientRegistry::new();
        let a = registry.register();
        let b = registry.register();

        assert_ne!(a, b);
        assert!(registry.is_active(a));
        assert!(registry.is_active(b));
        assert_eq!(registry.active_count(), 2);
    }

    #[test]
    fn unregister_marks_inactive() {
        let mut registry = ClientRegistry::new();
        let a = registry.register();

        registry.unregister(a);

        assert!(!registry.is_active(a));
        assert_eq!(registry.active_count(), 0);
    }

    #[test]
    fn unregister_unknown_id_is_a_harmless_no_op() {
        let mut registry = ClientRegistry::new();
        registry.unregister(ClientId::from_raw(999));
        assert_eq!(registry.active_count(), 0);
    }
}
