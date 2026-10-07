# cw_capability — progress

## 2026-10-07 — planning session 1

- Read CLAUDE.md, ADR 0010, 0021, 0022, ADR index.
- Traced the native server seam: `cat_native::{CapabilitiesWire, Command,
  RadioState, NativeSession}`, `cat_native::server::RadioHost`,
  `cat_rigctl::native_bridge::{NativeRadio, NativeShared, pump}`, and the
  three radios' `server/src/console.rs` impls.
- Checked CW commands in each sibling: ts570d command table + TS-570
  manual (KY/KS/PT/SD/PC, parameters 42-49); ft991a table + CAT manual
  (KM/KP/KR/KS/KY/BI/SD/PC, menus 056/060); ic7100 command.rs + CI-V text
  (14 09/0A/0C/0F, 16 47, 17, 17 FF).
- Found: ts570d `send_cw` writes `KY<msg>;` without the manual's byte-3
  space and 28-byte padding; its test pins the wrong form.
- Wrote `docs/adr/0023-cw-capability.md` (Proposed) and `task_plan.md`.
- `findings.md` could not be written (harness refused it as a report
  file); findings live in ADR 0023 §8-§9 and the hand-off report.
- No code, no commits, no push.

Next: architect/user review of ADR 0023 and the 12 open questions. T1 does
not start before that.

## 2026-10-07 — session 2

- Architect + user answered all 12 questions; recorded in ADR 0023
  ("Review decisions") and task_plan "Decisions". ADR → Accepted; README row.
- Worktree `../radio-cat-rs-cw` on `feat/cw-capability`; ADR + planning
  moved from the main checkout.
- Plan changes: T5 ID rule is server-appended `DE <CALL>`; T5 refuses
  non-CW mode; T6 refuses rigctl `T 1` while armed; new T7b
  (`connect_timeout`) and T7c (console CW display).
