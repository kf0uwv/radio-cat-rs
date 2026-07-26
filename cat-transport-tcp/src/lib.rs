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

//! TCP CAT transport.
//!
//! New code (`planning/architect/task_plan.md` Task 4a) -- no `ts570d`
//! source to move, since a TCP transport does not exist there. Implements
//! [`cat_transport_core::CatSession`] over `monoio::net::TcpStream` using
//! **length-prefixed frames**, deliberately not reusing
//! `cat-transport-serial::SerialCatSession`'s read-until-`;` framing: TCP
//! framing is this crate's own concern, per
//! `docs/adr/0002-async-runtime-binding-for-transport-crates.md` and
//! `.claude/agents/cat_transport.md`.
//!
//! See [`session`] module docs for the exact wire format (also written out
//! in full in `planning/cat_transport/progress.md` for a future
//! `cat-server` TCP listener to implement from the writeup alone).
//!
//! This crate depends only on `cat-transport-core` in this workspace, per
//! the dependency rules in `.claude/agents/cat_transport.md`.
//!
//! # Windows backend
//!
//! Per `docs/adr/0006-windows-network-transport.md`: [`codec`] holds the
//! pure, platform-neutral frame encode/decode logic; [`session`] (Linux,
//! `monoio`-based) and [`windows`] (a dedicated worker thread + the
//! `cat-transport-core::completion` primitive, needed since `monoio` does
//! not exist on Windows) each provide their own `TcpCatSession` built on top
//! of it. Both modules always compile and are always tested (`windows` has
//! no actual Windows-specific code -- see its own module doc) — only the
//! `TcpCatSession` re-exported below is `cfg`-gated per platform, so
//! application code never branches on which one it's using.

pub mod codec;
#[cfg(target_os = "linux")]
pub mod session;
pub mod windows;

pub use codec::{TcpSessionError, MAX_FRAME_SIZE};
#[cfg(target_os = "linux")]
pub use session::{read_frame, read_frame_or_eof, write_frame, TcpCatSession};
#[cfg(target_os = "windows")]
pub use windows::TcpCatSession;
