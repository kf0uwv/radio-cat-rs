# Progress log: release_workflow
2026-07-26: Done. See findings.md for the key research result and docs/adr/0008 for the full record.


## 2026-08-27 — Task 10 (MSVC migration) — code/config complete, CI run outstanding

**Done.**

- `ts570d/.github/workflows/ci.yml` — `windows-check` moved from
  `ubuntu-latest` cross-compiling `-gnu` to `windows-latest`, upgraded from
  `cargo check` to `cargo check` + `cargo test`, scope
  `--workspace --exclude emulator`.
- `ft991a/.github/workflows/ci.yml` — same migration, plus scope widened
  from `-p ft991a` to `--workspace --exclude emulator` (see findings §4).
- `ts570d/Makefile` — `windows-check` target now `cargo xwin check
  --target x86_64-pc-windows-msvc`; help text updated.
- Operative `-gnu` references migrated in `radio-cat-rs` README + CLAUDE.md,
  `ts570d` README + CLAUDE.md, `ft991a` README + CLAUDE.md.
- Amendment notes (not edits) added to `radio-cat-rs` ADRs 0004, 0006, 0007,
  0008 and `ts570d` ADR 0006, pointing at ADR 0012. Historical `-gnu`
  verification records left intact throughout, per ADR 0012's Consequences.
- ADR 0012 §3 updated with the resolution of caveat 1 (affirmative),
  the sharpened caveat 2, and a new caveat 3.
- `radio-cat-rs` ADR index: 0010-0013 flipped to Accepted; `ts570d` ADR
  index: 0008 flipped to Accepted.
- `ts570d`/`ft991a` `CLAUDE.md` pending-amendment blocks put in force.

**Verified locally.** Both workspaces and a full `eframe`/`wgpu` probe
check *and* link to `x86_64-pc-windows-msvc` via `cargo-xwin`, emitting
PE32+ binaries. Details in `findings.md`.

**UPDATE (same day): the Windows run happened on real hardware.** The user
provided a Windows 11 machine (`radiombf`/"Victoria", build 22621) reachable
over SSH. Findings sections 7-8 record what it found: three classes of test
breakage and **one real production bug** (`cat-transport-udp`'s Windows
session timed out early, because it trusted `SO_RCVTIMEO`'s tick-granularity
clock over its own `Instant` deadline). All fixed; Windows now runs
123 passed / 0 failed, clean over 10 consecutive runs, and Linux is
unchanged at 197 / 0 over 3.

**Still outstanding.** No `windows-latest` CI run — these repos are not pushed from this environment. ADR 0012
predicts pre-existing failures the first time the Windows test modules
actually execute (ADR 0006 §4: they never have). Those failures are real
bugs and must not be silenced to make the job green.

**Next.** Task 11 (`cat-framework::capabilities`) is unblocked and does not
depend on the CI run landing.
