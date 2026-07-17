# Progress — cat_framework

## Task 1 — create `cat-framework` (done, pending architect/user review)

Created:
- `Cargo.toml` (workspace root): `[workspace] members = ["cat-framework"]`,
  `resolver = "2"`; `[workspace.package]` (edition 2021, rust-version 1.75,
  Apache-2.0, authors/homepage/repository pointed at `radio-cat-rs`);
  `[workspace.dependencies]` recording `monoio = "0.2.3"`,
  `async-trait = "0.1"`, `thiserror = "1.0.61"`, `tracing = "0.1.40"`,
  `libc = "0.2.153"`, `nix = "0.27.1"`, per
  `planning/architect/findings.md` §2 (recorded for cross-crate consistency;
  `cat-framework` itself uses only `thiserror`).
- `cat-framework/Cargo.toml`: package metadata via `.workspace = true`, single
  dependency `thiserror = { workspace = true }`.
- `cat-framework/src/lib.rs`: crate doc citing the `ts570d` source commit,
  `pub mod cat;`, and a re-export of every public item `cat.rs` defines.
- `cat-framework/src/cat.rs`: verbatim copy of `ts570d`'s
  `framework/src/cat.rs` @ commit `1585e1e` — no logic changes, including the
  full `#[cfg(test)] mod tests` block (in-crate fake `TestCommand`/
  `CommandTable`, no radio import).

### Acceptance checks (all green)

```
$ cargo test -p cat-framework
running 8 tests
test cat::tests::command_lookup_finds_definition ... ok
test cat::tests::parses_action_form ... ok
test cat::tests::parses_query_form ... ok
test cat::tests::rejects_missing_terminator ... ok
test cat::tests::rejects_unknown_command ... ok
test cat::tests::rejects_wrong_width ... ok
test cat::tests::parses_set_form ... ok
test cat::tests::response_builder_preserves_leading_zeroes ... ok
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo tree -p cat-framework
cat-framework v0.1.0 (.../cat-framework)
└── thiserror v1.0.69
    └── thiserror-impl v1.0.69 (proc-macro)
        ├── proc-macro2 v1.0.106 └── unicode-ident v1.0.24
        ├── quote v1.0.46 └── proc-macro2 (*)
        └── syn v2.0.119 └── proc-macro2, quote, unicode-ident (*)
# No other local crate present, as required.

$ cargo clippy -p cat-framework -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) — no warnings.

$ cargo fmt --check
(clean — exit 0; one auto-fix applied to lib.rs's re-export list wrapping,
via `cargo fmt -p cat-framework`, before this final run)
```

### Notes for architect/user review
- `cargo`/crates.io resolved `thiserror` to `1.0.69` (the latest `1.x`
  compatible with the workspace's `"1.0.61"` constraint) — expected caret
  semver behavior, not a deviation.
- Did not commit, per this session's standing rule (commit only on explicit
  user request). Working tree has the new `Cargo.toml`, `Cargo.lock`, and
  `cat-framework/` staged for review, untracked.
- No discrepancies found between `ts570d`'s actual `framework/src/cat.rs`
  and the architect's Task 1 export list — all 18 named items present and
  moved; `state_machine.rs` and `errors.rs`'s `FrameworkError`/
  `FrameworkResult` left untouched in `ts570d`, as instructed.
- First commit (when the user is ready to commit) should cite: `ts570d`
  commit `1585e1e`, file `framework/src/cat.rs`.

## Task 3 — create `cat-client` (done, pending architect/user review)

Design proposal written to `planning/cat_framework/task_plan.md` (Task 3
section) before any code, per instruction — full `CatClient<C, S>`/
`ClientError<E>` signature, naming rationale, bounds rationale, and the one
discrepancy found vs. the architect's example variant list (see below).

Created:
- Root `Cargo.toml`: added `"cat-client"` to `[workspace] members`.
- `cat-client/Cargo.toml`: `cat-framework`, `cat-transport-core` path deps;
  `async-trait = { workspace = true }` (per ADR 0002's Consequences, even
  though nothing in this crate's own code invokes the `#[async_trait]`
  macro — `CatClient`'s methods are inherent `async fn`s, not trait methods;
  followed exactly per the "do not substitute" rule rather than silently
  dropped as apparently unused); `thiserror = { workspace = true }`; `monoio`
  as `[target.'cfg(target_os = "linux")'.dev-dependencies]` only, per ADR
  0002.
- `cat-client/src/lib.rs`: crate doc citing `ts570d` commit `1585e1e`, file
  `radio/src/client.rs`; `pub mod client;`; re-exports
  `CatClient`/`ClientError`/`ClientResult`.
- `cat-client/src/client.rs`: `CatClient<C: CommandId, S: CatSession>`
  (generic over the radio-supplied `C` and any `S: CatSession`, storing
  `session: S` and `table: &'static CommandTable<C>`), with `new`,
  `query`, `query_with_param`, `set` preserving `ts570d`'s exact method
  shape/logic; `ClientError<E: std::error::Error + 'static>` with
  `UnknownCommand`/`CommandNotReadable`/`CommandNotWritable`/
  `ProtocolError(ProtocolErrorKind)`/`Transport(#[from] E)`; `ClientResult<T,
  E>` alias. In-crate fake `FakeCommand`/`CommandTable` (mirrors
  `cat-framework`'s own `TestCommand` fake — 5 commands: `FA`
  read/write, `IF` read-only, `TX` write-only action, `SM`/`RM`
  selector-parameter reads for `query_with_param`). Reuses
  `cat_transport_core::test_support::{Exchange, ScriptedCatSession}` for the
  `CatSession` side rather than a second hand-rolled mock — that module is
  public test infrastructure built for exactly this. Added one small
  in-test-module fake `ProtocolErrorSession` (implements `CatSession`
  directly) to exercise `ClientError::ProtocolError`, since
  `ScriptedCatSession` never produces that disposition itself.
- 13 unit tests: 10 ported 1:1 from `ts570d`'s `radio/src/client.rs` test
  module (renamed `RadioError` → `ClientError`, `Ts570dCommandId`/
  `TS570D_COMMAND_TABLE` → the fake table), plus 3 new tests
  (`test_query_protocol_error_is_surfaced`,
  `test_query_transport_error_propagates`,
  `test_set_transport_error_propagates`) covering the `ProtocolError` and
  `Transport` variants that didn't have dedicated tests in the original
  suite.

### Discrepancy vs. the architect's task description (flagged, not silently resolved)

`planning/architect/task_plan.md`'s Task 3 gives an "e.g." (example, not
exhaustive) `ClientError<E>` variant list: `UnknownCommand`/
`CommandNotReadable`/`CommandNotWritable`/`Transport(E)`. Reading `ts570d`'s
actual `radio/src/client.rs::execute_query` shows it also handles
`ResponseDisposition::ProtocolError(kind)` via
`RadioError::InvalidProtocolString(format!("session reported a protocol
error: {:?}", kind))` — a case the example list omits. Preserving behavior
exactly (the task's own instruction) requires handling it, so I added a
fifth variant, `ProtocolError(ProtocolErrorKind)`, typed against
`cat_framework::ProtocolErrorKind` (already public and generic) instead of
reintroducing `RadioError`'s stringly-typed message. Recorded in the Task 3
design-proposal section of `task_plan.md` before writing the code, per the
"before writing code" instruction — not discovered after the fact.

No other discrepancy found: `RadioClient`'s method names/shapes (`query`,
`query_with_param`, `set`, private `validate_code`/`execute_query`) matched
the task description exactly.

### Naming decision

Chose `CatClient<C, S>` over `RadioClient` (the task explicitly allowed
proposing a better name). Rationale recorded in `task_plan.md`: matches the
crate name and this repository's `Cat`-prefixed generic-type convention
(`CatFramework`, `CatSession`, `CatRadio`); avoids the word "Radio" in a type
that is now fully radio-independent, which also avoids confusion with a
future radio crate's own wrapper (`ts570d::Ts570d<S>` already exists and
will wrap this type later).

### A real technical constraint surfaced during implementation (not a design change)

`ClientError<E>`'s `where E: std::error::Error + 'static` bound (needed so
`#[from]` can generate `Error::source()`) had to be repeated as a bound on
`CatClient`'s own `impl<C, S> ... where S: CatSession` block
(`S::Error: std::error::Error + 'static`), not just implied — Rust does not
propagate a `where`-clause from one type's definition into every place that
name is used as a generic argument elsewhere. This is a mechanical
consequence of the design already written down in the proposal, not a
change to it: the proposal's own "Bounds reasoning" section anticipated the
`E: std::error::Error + 'static` requirement; this is just where the
compiler needed it spelled out a second time.

### Acceptance checks (all green)

```
$ cargo test -p cat-client
running 13 tests
test client::tests::test_query_protocol_error_is_surfaced ... ok
test client::tests::test_query_fa_formats_correctly ... ok
test client::tests::test_query_transport_error_propagates ... ok
test client::tests::test_query_set_unknown_command_does_not_write ... ok
test client::tests::test_query_unknown_command_returns_error ... ok
test client::tests::test_query_with_param_rm1_formats_correctly ... ok
test client::tests::test_query_with_param_unknown_command_returns_error ... ok
test client::tests::test_query_with_param_sm0_formats_correctly ... ok
test client::tests::test_query_write_only_command_returns_error ... ok
test client::tests::test_set_does_not_read_response ... ok
test client::tests::test_set_fa_formats_correctly ... ok
test client::tests::test_set_read_only_command_returns_error ... ok
test client::tests::test_set_transport_error_propagates ... ok
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo tree -p cat-client
cat-client v0.1.0 (.../cat-client)
├── async-trait v0.1.89 (proc-macro)
├── cat-framework v0.1.0 (.../cat-framework)
│   └── thiserror v1.0.69
├── cat-transport-core v0.1.0 (.../cat-transport-core)
│   ├── async-trait v0.1.89 (proc-macro) (*)
│   ├── cat-framework v0.1.0 (.../cat-framework) (*)
│   ├── monoio v0.2.4 (...)
│   └── thiserror v1.0.69 (*)
└── thiserror v1.0.69 (*)
[dev-dependencies]
└── monoio v0.2.4 (*)
# Only cat-framework and cat-transport-core appear as local crates, as
# required. monoio appears only as: (a) a real dependency of
# cat-transport-core (per ADR 0002, unaffected by this task) and (b) our own
# dev-dependency for #[monoio::test] — never a cat-client production
# dependency.

$ cargo clippy -p cat-client --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) — no warnings.

$ cargo fmt --check   (whole workspace)
exit 0 — clean (one auto-fix applied via `cargo fmt -p cat-client` before
this final run: line-wrapping on validate_code's signature and a few
`matches!` macros in the test module).

$ cargo test --workspace
All crates green: cat-framework (8), cat-transport-core + cat-transport-serial
(14), cat-client (13). No regressions from adding cat-client.
```

### Notes for architect/user review
- Did not commit, per this session's standing rule. Working tree has
  `Cargo.toml` (root), `Cargo.lock`, and `cat-client/` new/modified,
  untracked/unstaged for review.
- First commit (when the user is ready) should cite `ts570d` commit
  `1585e1e`, file `radio/src/client.rs`, and note this was genericization
  (parameterizing over `C`/`CommandTable<C>`, introducing `ClientError<E>`),
  not a pure move — per `planning/architect/findings.md` §5.
- `ts570d` and `ft991a` were not touched, per the standing constraint.
- Did not create `cat-transport-tcp`, `cat-transport-udp`, or `cat-server` —
  out of scope for this task.
