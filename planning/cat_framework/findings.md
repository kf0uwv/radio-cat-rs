# Findings — cat_framework

## Task 1 — `cat-framework` extraction

- `ts570d`'s `framework/src/cat.rs` @ commit `1585e1e` matches the architect's
  Task 1 export list exactly — no gap, no extra items found. All 18 named
  types/traits present: `CommandId`, `CommandOperation`, `CommandForm`,
  `CommandDefinition<C>`, `CommandTable<C>`, `CommandRequest`,
  `ParameterValues`, `ParameterAccessError`, `ParseError`,
  `ProtocolErrorKind`, `ResponseDisposition`, `CommandOutcome`,
  `ResponseBuildError`, `ResponseBuilder`, `CatCommandCatalog`, `CatRadio`,
  `CatFrameworkError<E>`, `CatFramework<R>`.
- The file's own unit tests already use an in-crate fake `TestCommand`
  enum/`CommandTable` — confirmed no `radio` import anywhere in `cat.rs`, so
  the tests moved as-is with zero adaptation needed.
- `cat.rs` has no `async` code and no dependency beyond `thiserror` (for the
  `#[derive(Error)]` enums) — confirms ADR 0002's claim that `cat-framework`
  is unaffected by the monoio binding decision.
- `ts570d/framework/src/lib.rs` re-exports `cat.rs`'s items alongside
  `errors::{FrameworkError, FrameworkResult, TransportError}`,
  `session::{CatSession, SerialCatSession}`,
  `state_machine::{ApplicationStateMachine, State}`, `transport::Transport`,
  and `monoio`/`Arc`/`Pin` convenience re-exports — none of that belongs in
  `cat-framework`'s `lib.rs`; only the `cat` module's own items were
  re-exported here, confirming the exclusions in the architect's findings.md
  §8–9 by direct inspection of the file, not just by taking the ADR's word
  for it.
