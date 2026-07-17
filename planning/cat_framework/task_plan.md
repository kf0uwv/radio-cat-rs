# Task Plan — cat_framework

## Current task: Task 1 (architect's dispatch queue) — create `cat-framework`

Extraction is authorized (architect `planning/architect/task_plan.md`, dated
2026-07-16). This is the first code in the repository.

### Source

`ts570d` commit `1585e1e` (`refactor/generic-cat-framework`, confirmed current
HEAD of that branch), file `framework/src/cat.rs` (551 lines, read in full).
Content matches the architect's export list exactly — no discrepancy found:
`CommandId`, `CommandOperation`, `CommandForm`, `CommandDefinition<C>`,
`CommandTable<C>`, `CommandRequest`, `ParameterValues`,
`ParameterAccessError`, `ParseError`, `ProtocolErrorKind`,
`ResponseDisposition`, `CommandOutcome`, `ResponseBuildError`,
`ResponseBuilder`, `CatCommandCatalog`, `CatRadio`, `CatFrameworkError<E>`,
`CatFramework<R>`, plus the `#[cfg(test)] mod tests` block using an in-crate
fake `TestCommand`/`CommandTable`/`DEFINITIONS`/`TABLE` — no radio import.

Excluded per architect's findings.md §8–9 (not touched, not carried):
`framework/src/state_machine.rs`, `framework/src/errors.rs`'s
`FrameworkError`/`FrameworkResult`. `CatFrameworkError<E>` is self-contained
in `cat.rs` and needs nothing from `errors.rs`.

### Plan

1. Create root `Cargo.toml`: `[workspace]` with `members = ["cat-framework"]`,
   `resolver = "2"`; `[workspace.package]` (edition 2021, rust-version 1.75,
   license Apache-2.0, matching `ts570d`'s convention); `[workspace.dependencies]`
   recording `monoio = "0.2.3"`, `async-trait = "0.1"`, `thiserror = "1.0.61"`,
   `tracing = "0.1.40"`, `libc = "0.2.153"`, `nix = "0.27.1"` per
   `planning/architect/findings.md` §2 (recorded for eventual cross-crate
   consistency even though `cat-framework` itself only uses `thiserror`).
2. `cat-framework/Cargo.toml`: package metadata via `.workspace = true`,
   single dependency `thiserror = { workspace = true }`. No dev-dependencies
   needed (tests are plain `#[test]`, no `monoio::test`).
3. `cat-framework/src/lib.rs`: `pub mod cat;` plus re-export of everything
   `cat.rs` defines (mirroring `ts570d`'s `framework/src/lib.rs` re-export
   list, minus the excluded items and minus the `monoio`/state_machine/errors
   re-exports that don't apply here).
4. `cat-framework/src/cat.rs`: copy of `ts570d`'s `framework/src/cat.rs`
   verbatim (module doc comment, all types, all impls, the full `tests`
   module) — no logic changes. Adjust only the module doc line "delegate
   command semantics to a radio-specific `CatRadio` implementation" — kept
   as-is, still accurate.
5. Run `cargo test -p cat-framework`, `cargo tree -p cat-framework`,
   `cargo clippy -p cat-framework -- -D warnings`, `cargo fmt --check`.
6. Update `progress.md` with results. Do not commit (session standing rule:
   only commit when the user explicitly asks) — leave staged/unstaged for
   review.

### Judgment calls flagged for review
- Putting `cat.rs`'s contents in a submodule (`cat-framework/src/cat.rs`,
  re-exported from `lib.rs`) rather than inlining directly into `lib.rs`,
  matching `ts570d`'s own module structure (`pub mod cat;` + re-export) so
  the crate's internal layout stays recognizable against its source.
- Root `Cargo.toml` `[workspace.dependencies]` includes entries
  (`monoio`, `async-trait`, `libc`, `nix`) that `cat-framework` itself does
  not use yet, per the architect's explicit finding §2 that the workspace
  Cargo.toml should record all versions up front for cross-crate consistency
  as later crates land. Flagging in case this reads as scope creep — it's
  the architect's recorded decision, not my own addition.

## Current task: Task 3 (architect's dispatch queue) — create `cat-client`

Dated 2026-07-16 (continuing this session). Depends on Task 1
(`cat-framework`, done above) and Task 2 (`cat-transport-core` +
`cat-transport-serial`, done — confirmed present at
`cat-transport-core/src/{session,transport,errors,test_support}.rs` and
`cat-transport-serial/`, `cargo test -p cat-transport-core -p
cat-transport-serial` green: 14 tests passing before this task starts).

### Source and discrepancy check

`ts570d` commit `1585e1e` (`refactor/generic-cat-framework`, confirmed current
HEAD), file `radio/src/client.rs` (full file read). Matches
`planning/architect/findings.md` §5's characterization exactly:

- `RadioClient<S: CatSession<Error = TransportError>>` hardcodes
  `Ts570dCommandId`/`TS570D_COMMAND_TABLE` (via
  `crate::ts570d_radio::{Ts570dCommandId, TS570D_COMMAND_TABLE}`) and returns
  `radio::{RadioError, RadioResult}` (`radio/src/radio_trait.rs`).
- `RadioError` (full enum read) mixes generic variants — `UnknownCommand`,
  `CommandNotReadable`, `CommandNotWritable`, `Transport(#[from]
  TransportError)` — with TS-570D-specific ones — `InvalidMode`,
  `FrequencyOutOfRange`, `NotImplemented`, `Unsupported` — plus one more not
  named in the architect's `ClientError<E>` sketch: `InvalidProtocolString`,
  which `client.rs`'s `execute_query` uses for the
  `ResponseDisposition::ProtocolError(kind)` case
  (`RadioError::InvalidProtocolString(format!("session reported a protocol
  error: {:?}", kind))`).
- Methods present exactly as the architect's task description says: `query`,
  `query_with_param`, `set`, plus private helpers `validate_code` and
  `execute_query`. No discrepancy in method names/shapes — proceeding without
  stopping.
- **One real gap in the architect's `ClientError<E>` sketch, not a stop
  condition but recorded per instruction:** the sketch names four variants
  (`UnknownCommand`/`CommandNotReadable`/`CommandNotWritable`/`Transport(E)`)
  prefixed "e.g." (example, not exhaustive) but omits a case for
  `ResponseDisposition::ProtocolError`, which `execute_query`'s logic must
  still handle to preserve behavior exactly (task instruction: "Preserve the
  existing method shape ... and behavior exactly"). Resolved below as a fifth
  variant, `ProtocolError(ProtocolErrorKind)` — typed against
  `cat_framework::ProtocolErrorKind` (already a public generic type, reused
  rather than `RadioError`'s stringly-typed message) instead of
  `RadioError::InvalidProtocolString(String)`. This is a design choice
  flagged for review, not a silent deviation: the "e.g." qualifier and the
  instruction to keep behavior identical make adding this variant necessary,
  and typing it against the existing generic `ProtocolErrorKind` (rather than
  reintroducing a stringly-typed, radio-flavored message) fits this crate's
  own "radio-independent" mandate better than copying `RadioError`'s
  approach verbatim.

### Design proposal (write signature before code, per task instruction)

**Naming: `CatClient<C, S>`, not `RadioClient`.** The task explicitly invites
a better name than `RadioClient`/`CatClient` if found, with the instruction
not to leave it ambiguous. Choosing `CatClient`:
- Matches the crate name (`cat-client` → `CatClient`), exactly like this
  repository's other "Cat`-prefixed generic types (`CatFramework`,
  `CatSession`, `CatRadio`) — `RadioClient` reads as if it were still
  TS-570D-specific, which is precisely the property this task removes.
- Avoids a naming collision in intent with each radio crate's own future
  wrapper type (`ts570d::Ts570d<S>` already exists and will eventually wrap
  *this* type — keeping "Radio" out of the generic engine's name keeps that
  distinction clean).

**Full struct/impl/error definition:**

```rust
// cat-client/src/client.rs

use cat_framework::{CommandDefinition, CommandId, CommandTable, ProtocolErrorKind, ResponseDisposition};
use cat_transport_core::CatSession;
use thiserror::Error;

/// Generic, radio-independent CAT command sender.
///
/// Wraps any [`CatSession`] implementation and validates command codes
/// against a radio-supplied `&'static CommandTable<C>` before placing bytes
/// on the wire. `C` is the radio's own `CommandId` type (e.g. a future
/// `ts570d::Ts570dCommandId` or `ft991a`'s own); this crate never names a
/// concrete radio type or concrete transport type.
pub struct CatClient<C: CommandId, S: CatSession> {
    pub(crate) session: S,
    table: &'static CommandTable<C>,
}

impl<C, S> CatClient<C, S>
where
    C: CommandId,
    S: CatSession,
{
    /// Create a new `CatClient` wrapping `session`, validating commands
    /// against `table`.
    pub fn new(session: S, table: &'static CommandTable<C>) -> Self {
        Self { session, table }
    }

    /// Send a query command and return the radio's response string.
    ///
    /// Formats the wire bytes as `"<code>;"`.
    ///
    /// # Errors
    /// - [`ClientError::UnknownCommand`] — `code` is not in the command table
    /// - [`ClientError::CommandNotReadable`] — command does not support read
    /// - [`ClientError::Transport`] — I/O error on the underlying session
    pub async fn query(&mut self, code: &str) -> Result<String, ClientError<S::Error>> {
        let meta = self.validate_code(code)?;
        if !meta.is_readable() {
            return Err(ClientError::CommandNotReadable(code.to_string()));
        }
        let wire = format!("{};", code);
        self.execute_query(wire.as_bytes()).await
    }

    /// Send a query command with a parameter prefix, e.g. `"SM0;"`.
    ///
    /// Same error cases as [`query`](Self::query).
    pub async fn query_with_param(
        &mut self,
        code: &str,
        params: &str,
    ) -> Result<String, ClientError<S::Error>> {
        let meta = self.validate_code(code)?;
        if !meta.is_readable() {
            return Err(ClientError::CommandNotReadable(code.to_string()));
        }
        let wire = format!("{}{};", code, params);
        self.execute_query(wire.as_bytes()).await
    }

    /// Send a set command with parameters. Fire-and-forget: delegates to
    /// [`CatSession::send`] and does not wait for a response.
    ///
    /// # Errors
    /// - [`ClientError::UnknownCommand`] — `code` is not in the command table
    /// - [`ClientError::CommandNotWritable`] — command does not support write
    /// - [`ClientError::Transport`] — I/O error on the underlying session
    pub async fn set(&mut self, code: &str, params: &str) -> Result<(), ClientError<S::Error>> {
        let meta = self.validate_code(code)?;
        if !meta.is_writable() {
            return Err(ClientError::CommandNotWritable(code.to_string()));
        }
        let wire = format!("{}{};", code, params);
        self.session.send(wire.as_bytes()).await?;
        Ok(())
    }

    fn validate_code(&self, code: &str) -> Result<&'static CommandDefinition<C>, ClientError<S::Error>> {
        self.table
            .find(code)
            .ok_or_else(|| ClientError::UnknownCommand(code.to_string()))
    }

    async fn execute_query(&mut self, wire: &[u8]) -> Result<String, ClientError<S::Error>> {
        let mut response = Vec::new();
        let disposition = self.session.execute(wire, &mut response).await?;
        match disposition {
            ResponseDisposition::ProtocolError(kind) => Err(ClientError::ProtocolError(kind)),
            ResponseDisposition::ResponseWritten | ResponseDisposition::NoResponse => {
                Ok(String::from_utf8_lossy(&response).into_owned())
            }
        }
    }
}

/// Convenience alias, mirroring `ts570d::RadioResult`.
pub type ClientResult<T, E> = Result<T, ClientError<E>>;

/// Generic, radio-independent client-side errors.
///
/// Replaces `ts570d::RadioError` for this layer: keeps only the variants
/// that are actually generic (`UnknownCommand`/`CommandNotReadable`/
/// `CommandNotWritable`/`Transport`/`ProtocolError`) and drops the
/// radio-specific ones (`InvalidMode`, `FrequencyOutOfRange`,
/// `NotImplemented`, `Unsupported`) — those stay in each radio crate's own
/// error type, which will wrap or convert from `ClientError<E>` once
/// `ts570d` migrates onto this crate (a later, separate task; see
/// `planning/architect/findings.md` §4).
#[derive(Debug, Error)]
pub enum ClientError<E>
where
    E: std::error::Error + 'static,
{
    /// `code` is not present in the radio's command table.
    #[error("unknown command: {0}")]
    UnknownCommand(String),
    /// Command exists but does not support query (read).
    #[error("command {0} does not support read (query)")]
    CommandNotReadable(String),
    /// Command exists but does not support set (write).
    #[error("command {0} does not support write (set)")]
    CommandNotWritable(String),
    /// The session reported a protocol-level error for this exchange.
    #[error("session reported a protocol error: {0:?}")]
    ProtocolError(ProtocolErrorKind),
    /// I/O error from the underlying [`CatSession`].
    #[error("transport error: {0}")]
    Transport(#[from] E),
}
```

**Bounds reasoning:**
- `C: CommandId` on the struct (not just the impl) because the struct stores
  `&'static CommandTable<C>`, and `cat-framework`'s own `CommandTable<C:
  CommandId>` already requires it there — this isn't a new constraint, just
  surfacing an existing one.
- `S: CatSession` with no `Error = TransportError` pin (unlike `ts570d`'s
  `where S: CatSession<Error = TransportError>`) — this is the actual
  genericization: `ClientError<S::Error>` carries whatever error type the
  radio-supplied session uses, instead of hardcoding `TransportError`.
  `cat-transport-core`'s own `CatSession` trait declares `type Error;` with
  no supertrait bound, so `ClientError<E>`'s own `where E: std::error::Error
  + 'static` bound (needed for `#[from]`'s auto-generated `source()`) is
  attached to `ClientError` itself, not forced onto `CatSession::Error` —
  any concrete session whose error type implements `std::error::Error`
  (true of `TransportError` today, and expected of any real transport error)
  satisfies it; the bound only becomes a compile error at the point a
  concrete `S` is chosen with a non-conforming `Error` type, matching how
  `thiserror`'s `#[from]` normally composes with generics.
- `#[from]` on `Transport(E)` preserves the original code's exact `?`-based
  control flow (`self.session.execute(...).await?` and
  `self.session.send(...).await?` both still work via implicit `From`
  conversion), matching the instruction to change only "type parameters and
  error type," not the logic/shape of the method bodies.

### Plan

1. Add `"cat-client"` to the root `Cargo.toml`'s `[workspace] members`.
2. `cat-client/Cargo.toml`: `cat-framework` and `cat-transport-core` path
   deps; `async-trait = { workspace = true }` (per ADR 0002's Consequences
   section, which states this crate "depends on `async-trait` directly,"
   even though — checked directly — no method in the design above needs the
   macro itself: all `CatClient` methods are inherent `async fn`s, not trait
   methods, so nothing here requires `#[async_trait]`'s desugaring; recorded
   as a directive followed exactly per the "do not substitute" rule, not a
   silent omission); `thiserror = { workspace = true }` (for `ClientError`).
   `monoio` as a `[target.'cfg(target_os = "linux")'.dev-dependencies]`
   entry only, per ADR 0002 and the ground rules, for `#[monoio::test]`.
3. `cat-client/src/lib.rs`: crate doc citing `ts570d` commit `1585e1e`,
   file `radio/src/client.rs`; `pub mod client;`; re-export
   `CatClient`/`ClientError`/`ClientResult`.
4. `cat-client/src/client.rs`: the definition above, plus unit tests using an
   in-crate fake `CommandId` (mirroring `cat-framework`'s own
   `TestCommand`/`DEFINITIONS`/`TABLE`) and `cat_transport_core::test_support`'s
   already-existing `Exchange`/`ScriptedCatSession` (public test
   infrastructure built for exactly this reuse — not a second hand-rolled
   mock) for the `CatSession` side. Tests ported 1:1 from `ts570d`'s
   `radio/src/client.rs` test module (11 tests), adapted only for the new
   fake command table/error type/`#[monoio::test(driver = "legacy")]`
   dev-dependency.
5. Run `cargo test -p cat-client`, `cargo tree -p cat-client`, `cargo clippy
   -p cat-client -- -D warnings`, `cargo fmt --check`.
6. Update `progress.md`. Do not commit (standing rule). First commit message,
   when the user is ready, should cite `ts570d` commit `1585e1e`, file
   `radio/src/client.rs`, and note this was genericization, not a pure move.
