# cat_morse — progress

## Session: 2026-10-05 (planning phase)

- Status: plan written, awaiting architect approval. No crate code written,
  no Cargo.toml touched.
- Read: radio-cat-rs CLAUDE.md, ADR 0001, cat-signal-audio/Cargo.toml and
  lib.rs header, planning/cat_signal/task_plan.md (format reference),
  .github/workflows/ci.yml feature/test steps, docs/adr index.
- Researched: current POTA reference format (ISO prefixes since 2024).
- Created: planning/cat_morse/{task_plan,progress}.md (findings.md was
  refused by the Write tool for subagents; see Error Log)

## Error Log
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-10-05 | Write tool refused findings.md ("subagents should return findings as text") | 1 | Reported to architect; findings content included in hand-off |

## Session: 2026-10-05 (implementation, approved)

- Branch `feat/cat-morse` off main.
- Step 1 done: crate scaffold, `rng` (xoshiro256**, pinned to reference
  output 0x99EC5F36CB75F2B4 for seed 0), `codec`. 15 tests. Commit 55742be.
  Note: `codec::table()` returns an iterator (built once via `OnceLock`)
  rather than a `&'static [..]` slice.
- Step 2 done: `timing` (Speed, Farnsworth, Fist presets, Keying, key,
  key_text). `key` returns `Result` (validates the Fist); `TimingError`
  gained a `Codec` variant so `key_text` has one error type. 26 tests total.
- Step 3 done: `synth` + internal `dsp` (Goertzel, RBJ band-pass biquad).
  `render_tone` output is `total + rise` long (keying offset by rise/2 so
  ramps centred on edges are whole) — plan said `round(total·fs)`; changed
  because centred ramps need the half-rise lead/tail. Channel SNR is
  referenced to a 0 dB-level sine (`REFERENCE_AMPLITUDE` 0.25), so a
  signal's own SNR = snr_db + level_db. Channel got a `padding` field.
  Test fixes during TDD: key-click test needed a dense (2 Hz) sideband
  sweep; discontinuity test uses 713 Hz because 700 Hz edges at 30 wpm all
  fall on zero crossings. 40 tests.
