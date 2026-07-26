# Task plan: cat_diagnostics

1. [x] Read ts570d's ui/src/{terminal.rs,diag.rs,layout.rs} in full.
2. [x] Read cat-framework's CommandTable/CommandDefinition/CommandForm and
       cat-client's CatClient API to design the generic probe strategy.
3. [x] Design + implement cat-diagnostics (Cargo.toml, lib.rs, engine.rs).
4. [x] Unit tests against ScriptedCatSession + a local NeverRespondingSession.
5. [x] Fix monoio/portable-timeout incompatibility (found via own test hang).
6. [x] cargo fmt/clippy/test -p cat-diagnostics; cargo check --target
       x86_64-pc-windows-gnu -p cat-diagnostics.
7. [ ] Write docs/adr/0007-shared-diagnostics-engine.md with the exact API
       and a usage example; update docs/adr/README.md.
8. [ ] Update root README.md's crate list.
