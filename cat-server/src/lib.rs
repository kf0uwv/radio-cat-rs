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

//! `cat-server`: the request broker that lets one physical radio session be
//! shared by multiple remote clients over TCP and/or UDP.
//!
//! New code — no `ts570d` analog (`planning/architect/task_plan.md` Task 5,
//! `.claude/agents/cat_server.md`). This crate sits **above** a radio's
//! generic client ([`cat_client::CatClient<C, S>`]) and a
//! [`cat_transport_core::CatSession`] implementation, exactly the way
//! direct control mode uses that client today, and serializes access to it
//! through one worker:
//!
//! ```text
//! TCP/UDP server transport ──▶ request broker ──▶ physical radio controller
//! ```
//!
//! - [`broker`] — [`broker::Broker`] (validates/executes one raw wire
//!   request against the physical `CatClient`), [`broker::BrokerWorker`]
//!   (the single ordered worker), [`broker::BrokerHandle`] (what
//!   client-facing tasks hold to submit work).
//! - [`local_channel`] — the hand-rolled, `!Send`, single-threaded channel
//!   primitives the worker is built on (no channel crate is on this
//!   crate's dependency list).
//! - [`registry`] — [`registry::ClientId`]/[`registry::ClientRegistry`],
//!   client session bookkeeping kept entirely separate from the broker and
//!   the physical session.
//! - [`tcp`] / [`udp`] — server-side accept/dispatch loops, wire-compatible
//!   with `cat-transport-tcp`/`cat-transport-udp`'s client-side framing
//!   (re-implemented directly against `monoio::net`, not by depending on
//!   those crates — see `planning/cat_server/task_plan.md`'s "Server-side
//!   framing" section for why).
//!
//! This crate never leaks into a radio's `CatRadio` state machine — there is
//! no radio state machine in this repository to touch, but the contract is
//! kept clean for one downstream: `Broker` wraps and serializes calls to a
//! generic `CatClient<C, S>` from the outside only, and never names a
//! concrete radio's command-id type.

pub mod broker;
pub mod local_channel;
pub mod registry;
pub mod tcp;
pub mod udp;

#[cfg(test)]
mod test_fixtures;

pub use broker::{
    build, build_with_timeout, Broker, BrokerHandle, BrokerWorker, DispatchError, DispatchOutcome,
    Job,
};
pub use registry::{ClientId, ClientRegistry};
