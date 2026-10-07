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
- T1 done: CW capability types in `cat_framework::capabilities`, tests
  first (red: 58 compile errors; green: 86 cat-framework lib tests).
  Workspace: 1139 passed / 0 failed / 3 ignored; clippy -D warnings
  clean; fmt clean; `cargo xwin check --target x86_64-pc-windows-msvc
  --workspace --all-targets` clean.
- For T2: `BreakInKind`/`CwPacing`/`CwAbort`/`HzRange`/`WpmRange` are
  Serialize+Deserialize and reusable on the wire directly; only
  `CwSendText`/`TxGateCapability`/`CwCapability` (`&'static` fields) need
  owned mirrors. `requires_break_in` must be mirrored too.
- T1 accepted by architect (requires_break_in + PascalCase approved;
  0.9.0 bump goes in the last task before release).
- T2 done: wire types (tests first: 31 compile errors red; 10 `cw_wire`
  tests green). Workspace 1149 passed / 0 failed / 3 ignored; clippy,
  fmt, xwin msvc clean. PROTOCOL_VERSION still 2, existing pin unchanged.
- T3 done: per-connection CW rules + `RadioHost::serves_cw` gate (default
  false; Welcome hides cw, CW commands Unsupported, AbortCw acked but not
  applied). Red: 6 compile errors; one test had a miscounted callsign
  (VE3/N0CALL is 10, not 11) -- fixed the test. Green: 20 `cw_wire` + 2
  `cw_gate` socket tests; workspace 1161/0/3; clippy, fmt, xwin clean.
- T4 done: `ConsoleId`, `HostRefusal`, defaulted `apply_as` /
  `console_attached_as(id, peer)` / `console_detached_as` / `state_as`;
  `serve` routes every call through them. Red: 6 compile errors. Green:
  5 `console_identity` socket tests (20/20 repeat runs), workspace
  1166/0/3; clippy, fmt, xwin clean. No host overrides `serves_cw`.
- T5 done: `cat_rigctl::native_bridge::cw::CwSupervisor` (sans-IO), new
  dep cat-rigctl -> cat-morse, `ConsoleId::from_raw`, wire
  `CwState.restore: Option<RestoreStatus>`. Red: 41/41 failing against a
  todo!() skeleton. Green: 42 supervisor tests. Mutation check: 8
  mutants (no power verify, PTT gate only while armed, no retry wait, no
  clamp check, no stop, no watchdog, ID counted early, no restore on idle
  disarm) -- 7 killed at first, the ID one survived; added a test, now
  killed. Workspace 1208/0/3; clippy, fmt,
  xwin clean.
- T5 revision (architect rejected "never split a word"): operator text
  splits at the drain bound; `DE <CALL>` is one whole final chunk, never
  mixed; arm refuses an ID that cannot fit one chunk. Red: 6 failing.
  Green: 46 supervisor tests; workspace 1212/0/3; clippy, fmt, xwin
  clean. Worst-case TS-570D drain stated in ADR §4: 27.48 s.
