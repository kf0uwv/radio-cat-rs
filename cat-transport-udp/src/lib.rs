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

//! UDP CAT transport.
//!
//! New code (`planning/architect/task_plan.md` Task 4b) -- no `ts570d`
//! source to move, since a UDP transport does not exist there. Implements
//! [`cat_transport_core::CatSession`] over `monoio::net::udp::UdpSocket` using an
//! **envelope format** (session id + request id) plus a **deduplication
//! cache** -- an independent design from `cat-transport-tcp`'s
//! length-prefixed framing, not derived from it: UDP guarantees neither
//! delivery nor ordering and is not connection-oriented, so this crate does
//! not pretend otherwise.
//!
//! See [`session`] module docs for the exact wire format (also written out
//! in full in `planning/cat_transport/progress.md` for a future
//! `cat-server` UDP listener to implement from the writeup alone).
//!
//! This crate depends only on `cat-transport-core` in this workspace, per
//! the dependency rules in `.claude/agents/cat_transport.md`.
//!
//! # Windows backend
//!
//! Per `docs/adr/0006-windows-network-transport.md`: [`codec`] holds the
//! pure, platform-neutral envelope encode/decode logic and the client-side
//! dedup cache; [`session`] (Linux, `monoio`-based) and [`windows`] (a
//! dedicated worker thread + the `cat-transport-core::completion`
//! primitive) each provide their own `UdpCatSession` built on top of it.
//! Both modules always compile and are always tested (`windows` has no
//! actual Windows-specific code -- see its own module doc) — only the
//! `UdpCatSession` re-exported below is `cfg`-gated per platform.

pub mod codec;
#[cfg(target_os = "linux")]
pub mod session;
pub mod windows;

pub use codec::{
    decode_envelope, encode_envelope, UdpSessionError, DEDUP_CACHE_CAPACITY,
    DEFAULT_RESPONSE_TIMEOUT, ENVELOPE_HEADER_LEN, MAX_PAYLOAD_SIZE,
};
#[cfg(target_os = "linux")]
pub use session::UdpCatSession;
#[cfg(target_os = "windows")]
pub use windows::UdpCatSession;
