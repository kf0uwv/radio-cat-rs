# Task Plan — code_review

No extraction yet; awaiting ts570d refactor completion and an explicit
go-ahead.

There is no code in this repository yet to review. This file will track
review passes once implementation work begins under `cat_framework`,
`cat_transport`, or `cat_server`.

## 2026-10-06 — Review: `git diff main...feat/cat-morse`

Scope: new `cat-morse` crate; `cat-signal-audio` playback (sink.rs,
playback.rs, `host_lock`); `cat-signal` `DeviceKind::AudioOutput`;
ADR 0021; ADR 0017 §15–17; CI. Binding design: `planning/cat_morse/task_plan.md`.
Review mode only — no code changes, no commits, no branch switch.

- [x] Read diff, plan, ADRs
- [x] Run `cargo test -p cat-morse -p cat-signal-audio` and clippy (pass, clean)
- [x] Probe edge cases from a scratch crate outside the repo
- [x] Record findings (findings.md), hand back to architect
