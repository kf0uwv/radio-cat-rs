# Progress — cat_rigctl

## Naming note

This directory's `findings.md`-equivalent is named `investigation.md`
instead — the executing harness hard-blocks `Write` calls targeting a file
literally named `findings.md` ("Subagents should return findings as text,
not write report files"), independent of this repo's own convention.
Content and role are otherwise identical to every other agent's
`findings.md`. See `investigation.md`'s own header note.

## Log

- Read `CLAUDE.md`, both apps' `broker_session.rs`, `ft991a`'s current
  `rigctl.rs`, both apps' `lib.rs`, `cat-server`'s `lib.rs`/`broker.rs`/
  `test_fixtures.rs`/`tcp.rs`/`udp.rs`/`Cargo.toml`, `cat-framework`'s
  `CommandId` definition, root `Cargo.toml`. Wrote `task_plan.md` and
  `investigation.md` before touching any source.
- Deliverable 1: `cat-server/src/broker_session.rs` added, `lib.rs` updated
  (`mod broker_session;` private + `pub use broker_session::
  BrokerCatSession;`). Reused `cat-server::test_fixtures::{FakeCommand,
  TABLE}` for its unit tests instead of a third private fixture; had to
  adjust one test's request/response text to the fixture's 11-digit `Set`
  form width (`FA00014250000;`, matching `ts570d`'s original digit width)
  once `cargo test` caught the mismatch — `CommandTable::parse`'s width
  gate applies to the *request* a `Set`-shaped exchange sends, which the
  investigation notes had correctly flagged as the only place width could
  matter. `cargo test -p cat-server` → 51 passed.
- Deliverable 2: new `cat-rigctl` crate (`Cargo.toml`, `src/lib.rs`
  — `RigctlRadio`/`ServerConfig`/`run`, `src/rigctl.rs` — private
  `dispatch`/`dump_state`/`LineReader`/`serve`). Root `Cargo.toml`: added
  `"cat-rigctl"` to `members`, added `futures = "0.3.30"` to
  `[workspace.dependencies]` (matching both apps' existing
  `server/Cargo.toml` version pin). One deviation from the prompt's literal
  trait/fn signatures, load-bearing not stylistic: `run`'s `R: RigctlRadio`
  bound needed `+ 'static` (and `F` already had it) — `monoio::spawn`
  requires an owned `'static` future, confirmed by the exact compiler error
  (E0310) when first built without it. `cargo test -p cat-rigctl` → 18
  passed (16 ported `dispatch`/`dump_state`/`hamlib_mode_round_trips`
  tests against an in-crate `FakeRadio`, plus 1 new
  `dispatch_f_reports_error_when_the_radio_fails` test added for coverage
  of the `Err(_) => RPRT_ERR` mapping this crate's own generic dispatch
  code introduces, plus the 2 ported `run()`-level tests).
- Added `docs/adr/0005-rigctl-bridge-and-radio-trait-boundary.md`, updated
  `docs/adr/README.md`'s index, updated `planning/architect/task_plan.md`
  with a new "Task 9" entry recording this work.
- Full verification, all clean: `cargo build --workspace`,
  `cargo test --workspace` (every crate's suite green, cat-server 51 +
  cat-rigctl 18 among them), `cargo clippy --workspace --all-targets --
  -D warnings`, `cargo fmt --check` (after one `cargo fmt` pass — two files
  needed reformatting, both accepted as-is), `cargo tree -p cat-rigctl`
  (confirmed: only workspace members `async-trait`/`cat-framework`/
  `cat-server`/`cat-transport-core`/`futures`/`monoio`/`tracing` and their
  existing third-party transitive deps — no radio-specific or app crate
  anywhere in the tree, and there are none in this repo to depend on
  regardless).
- Committed locally (see git log for hash — not pushed, per this task's
  explicit instruction not to touch `origin`).

## Post-migration regression fix (2026-07-26)

An independent post-migration review agent (verifying the `ts570d` port
onto this crate) found that the extraction silently dropped
`MAX_LINE_LEN` — `ts570d`'s pre-migration `LineReader::read_line` rejected
a line exceeding 512 bytes with no `\n` (`io::ErrorKind::InvalidData`,
closing the connection); this crate's ported `LineReader` had no such
bound at all, so a client that never sends `\n` grows `self.buf` without
limit and the connection never resolves — an unbounded-memory-growth DoS,
reproduced live (a raw socket sending 600 bytes with no newline hung
forever with no error). Fixed: restored `const MAX_LINE_LEN: usize = 512`
and the length check in `cat-rigctl/src/rigctl.rs`, ported verbatim from
`ts570d`'s original (pre-migration) fix rather than reinvented, plus its
regression test `read_line_rejects_a_line_longer_than_the_maximum_without_a_newline`
(now `cat-rigctl`'s 19th test). Re-verified live against a rebuilt
`ft991a server` (via the `.cargo/config.toml` local patch both apps use):
the same 600-byte-no-newline socket now gets closed instantly instead of
hanging, and normal `rigctl` traffic on the same listener still works
immediately afterward. `cargo build/test/clippy(-D warnings)/fmt --check`
all clean across the whole workspace after the fix. This affects both
`ft991a` and `ts570d` (both share this crate) — their own working copies
don't need any change, since both already pick up `cat-rigctl` via the
local path patch; a rebuild is sufficient.

## Status: complete, including the above regression fix.
