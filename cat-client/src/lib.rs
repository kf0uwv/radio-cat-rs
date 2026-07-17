//! Generic, radio-independent client-side CAT request/response mechanics.
//!
//! Genericized from `ts570d`'s `radio/src/client.rs` (commit `1585e1e`,
//! `refactor/generic-cat-framework` branch) — **not a pure move**. `ts570d`'s
//! `RadioClient<S: CatSession<Error = TransportError>>` hardcoded
//! `Ts570dCommandId`/`TS570D_COMMAND_TABLE` and returned the TS-570D-specific
//! `RadioError`. This crate's [`client::CatClient<C, S>`] parameterizes the
//! same method shape (`query`, `query_with_param`, `set`) over a
//! radio-supplied `C: CommandId` / `&'static CommandTable<C>` and a generic
//! [`client::ClientError<E>`] instead, per
//! `docs/adr/0001-scope-and-crate-boundaries.md` and
//! `planning/architect/findings.md` §5. See
//! `planning/cat_framework/task_plan.md`'s Task 3 section for the full
//! design proposal and rationale (naming, bounds, and the one variant added
//! beyond the architect's example list).
//!
//! `cat-client` never names a concrete radio type or a concrete transport
//! type — only [`cat_framework::CommandId`]/[`cat_framework::CommandTable`]
//! and [`cat_transport_core::CatSession`].

pub mod client;

pub use client::{CatClient, ClientError, ClientResult};
