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
- T6 done: `NativeCw` + `NativeRadio::cw()` opt-in, supervisor in
  `NativeShared`, `apply_as`/`state_as`/detach wiring, `pump_once` with
  CW-first ordering and live-CW fast tick, `PttGate` on both platforms.
  Red: 15 compile errors. Green: 16 `cw_host_tests`; mutation check 8
  mutants, 1 survivor (CW between queued commands) -> added a test,
  killed. Workspace 1228/0/3; clippy, fmt, xwin clean. No radio in this
  repo opts in; radio servers must call `shutdown_cw()` on exit.
- Fix 1 (architect): ID chunked at the 3 s bound too, never mixed with
  operator text, leads the next send if an abort cut it; arm refusal for
  a long ID removed. Red: 6 failing; green: 63 cw tests (supervisor +
  host). Mutants (no ID-first, unsplit ID) killed. ADR §4 worst case: 3 s.
- Fix 2 (architect): `run_with_native` settles CW on exit on both
  platforms (`settle_cw` async on Linux, `settle_cw_blocking` on
  Windows; 10 s bound; error log if unsettled). Red: 4 compile errors;
  green: 20 host tests; workspace 1233/0/3; clippy, fmt, xwin clean.
  Signal-driven exit is not owned by any library code -- reported.
- T7 done: `Connection::{cw_capability, cw_state, set_keyer_speed,
  arm_cw, send_cw, abort_cw, disarm_cw}`, `Client::cw_capability`,
  `ClientError::Refused`. Never probes a server without CW; `Malformed`
  -> `Refused(Unsupported)`. Red: 18 compile errors; green: 3 socket + 2
  unit tests; workspace 1238/0/3; clippy, fmt, xwin clean.
- Stop handle (architect): `ServerConfig::stop: StopHandle`, polled by a
  task (Linux) / thread (Windows) that ends the listener wait, so the
  restore path runs. Red: 3 compile errors; the end-to-end test (arm,
  then stop, restore observed) then exposed the pump sleeping up to 10 s
  idle before acting -> CW requests now kick the pump (red: 5.0 s; green
  < 2 s). Workspace 1240/0/3; clippy, fmt, xwin clean.
- T7b done: `Connection::connect_timeout` / `Client::connect_timeout`:
  one deadline over connect + handshake, each read armed with the time
  remaining, expiry is `Io(TimedOut)`, timeouts cleared afterwards. Red: 6
  compile errors. The drip test first passed trivially (zeros decode as a
  bad frame) -- rewritten to drip a valid frame; a per-read-timeout
  mutant now fails it (capped so it fails, not hangs). 5 tests; workspace
  1245/0/3; clippy, fmt, xwin clean.
