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
- Step 4 done: `decoder` + tests/roundtrip.rs (10 integration tests).
  TDD iterations (all logged here, none changed the approved design):
  - noise collapsed decoding: floor tracker biased up by mark tails, and
    squelch too close -> mean-of-key-up floor (running mean first 25
    windows, then spike-rejecting), thresholds never below squelch margin,
    default squelch 13 dB.
  - first-mark and start-up garbage -> marks re-timed at half their own
    peak with interpolation; 60 ms warm-up; blips < 0.3 dot dropped;
    dit clamped to 5-60 wpm.
  - Farnsworth letter gaps read as word gaps -> gap threshold computed
    with the gap being judged included (lone first gap = letter gap).
  - QRM leakage in digital silence -> Hann-windowed Goertzel bin and a
    `min_level_dbfs` floor (-70 dBFS). Added a second QRM test on a noisy
    band.
  - running-gap timer used the previous mark's end -> ordering bug fixed.
  Divergences from plan: `window` is `Option<Duration>` (None = adaptive,
  half a dot, 8-40 ms) instead of a fixed 12 ms default; new config field
  `min_level_dbfs`; auto-pitch lock holds until `reset()` (no automatic
  re-acquire). Human-fist test bounds mean CER <= 1 % and worst <= 5 %
  over 10 seeds (measured mean 0.52 % over 200 seeds; single seeds reach
  ~3 % from genuinely ambiguous timing), instead of per-seed <= 2 %.
  Noise test (+6 dB/500 Hz, CER <= 2 %) passed 100/100 seeds in a stress
  run. 57 tests total.
- Step 5 done: `scenario` (mod/callsign/data/qso). API additions vs plan:
  `Persona` (call, name, qth, region, zone, rig, power, antenna,
  reference) shared by `Station` and `Script::me`; `Difficulty` presets
  `beginner`/`intermediate`/`advanced`; `Difficulty::wpm` is whole wpm
  `(u32, u32)`; `ScenarioKind::Contest { exchange, cut_numbers }`.
  Pileup: you run the DX; callers re-call each round with `WithPrevious`
  overlap. SOTA activators in W associations get the matching state.
  POTA refs ISO form (`US-0477`, 5 digits for some US parks); DX parks
  only for entities with a known ISO prefix (DE FR IT ES JP AU CA).
  Unknown operator prefix -> region "DX", random zone. 16 scenario tests
  incl. 8 kinds x 1000 seeds all encode. 73 tests total.
- Step 6 done: `score` + `serde` feature. Divergence: `Unit` is a type
  alias for `codec::Token` (WordSpace = space; unknown copied chars are
  `Token::Char(c)`) rather than a separate enum. Added `tokenize_copy`
  (lenient), `align_units`, `Score::add`, `CharTally::accuracy`. BTreeMaps
  serialize as lists of pairs (Token is not a JSON key). Roundtrip tests
  now use the crate scorer for CER. 74 unit tests with `--features serde`
  (72 without) + 10 integration.
- Step 7 done: ADR 0021 + index row; CLAUDE.md (crate count 19, diagram,
  architecture list); README bullet; CI steps `cargo test -p cat-morse
  --features serde` on Linux and windows-latest; crate-level doctest
  showing the copy-practice loop.
- Verification (2026-10-06):
  - cargo test -p cat-morse: 72 unit + 10 integration + 1 doctest, pass
  - cargo test -p cat-morse --features serde: 74 + 10 + 1, pass
  - cargo test --workspace: 1059 passed, 0 failed
  - cargo clippy -p cat-morse --all-targets --all-features -D warnings: clean
  - cargo clippy --workspace --all-targets -D warnings: clean
  - cargo fmt --all -- --check: clean
  - cargo xwin check --target x86_64-pc-windows-msvc -p cat-morse
    --all-features --all-targets: clean
  - cargo tree -p cat-morse -e normal --all-features: serde + thiserror
    only; no local crates, no monoio/tokio/rand
  - No 1.75 toolchain locally; clippy `incompatible_msrv` (honours
    rust-version 1.75) is clean.
- Noted, not fixed (out of scope): README.md says "Nine crates" and "191
  tests", both stale before this change.

## Session: 2026-10-06 (follow-up: band, unordered scoring, playback)

- Part A done: `scenario::band` — `BandConditions` (+ `QsbOdds`,
  `QrnOdds`, `QrmOdds`), `Difficulty::band` on all presets (values from
  the app plan's table as small ranges), `band_channel` returning
  `(Channel, BandReport)`, `Keying::truncated`. QRN uses chance 1.0 on
  intermediate/advanced (the table gives a rate, not a probability).
  QRM is skipped for overs shorter than 1 s. 8 new tests (82 with serde).
- Part B done: `score::align_unordered` -> `UnorderedAlignment { items,
  matched, copied_words, extras }`, Hungarian assignment over integer
  edit costs on an (n+m)² matrix with miss/extra dummies; pairings with
  errors > half the expected item's length are scored as miss + extra.
  `CharStats::record_unordered`. 5 new tests (85 / 87 with serde).
- Part C done: `cat-signal-audio` playback. Ungated `sink.rs`
  (PlaybackConfig/Format/Error, PlaybackQueue, OutputOffer +
  choose_output; 10 tests); gated `playback.rs` (AudioPlayback, output
  enumeration; 3 tests); `output_devices()` with a feature-off stub
  (+1 test); `examples/playback_smoke.rs` (required-features device).
  - Divergence: added `cat_signal::DeviceKind::AudioOutput` (heading
    "AUDIO OUTPUT", flag `--audio-out`) — additive, enum is
    `#[non_exhaustive]`.
  - Divergence: `PlaybackConfig/Format/Error` are ungated (only
    `AudioPlayback` is gated); `PlaybackError` derives Clone/PartialEq/Eq.
  - Found: capture + playback device tests hung when run in parallel
    (ALSA enumeration on two threads deadlocks; each passed alone; the
    pre-existing suite passed in parallel). Fix: process-wide
    `device::host_lock()` around every enumerate/open in capture.rs and
    playback.rs (held only while opening).
  - Underrun rule: counted only when a callback runs short after a full
    one; the natural end of written audio counts once unless it falls on a
    callback boundary (documented).
  - Smoke test: opened `default` (PipeWire) 48 kHz 2 ch f32; 48 000
    samples drained in ~1.2 s wall; underruns 1 (the end). Audibility not
    verified.
  - ADR 0017 amendment §15-17; README row updated. CI: Linux job installs
    libasound2-dev and `cargo check -p cat-signal-audio --features device
    --all-targets`; Windows job checks the same (ADR 0017 §10.2 claimed
    this already existed; it did not).
- Checks: cat-signal-audio 53+15+3+0 default, 61+15+7+3 device; cat-morse
  85+10+1 (87 with serde); cat-signal 45; workspace 1084 passed 0 failed;
  clippy -D warnings clean (workspace; cat-morse all features;
  cat-signal-audio ± device); fmt clean; xwin msvc clean for
  cat-signal-audio --features device and cat-morse --all-features.

## Session: 2026-10-06 (review fixes, worktree ../radio-cat-rs-morsefix)

- Decoder group (findings 1, 2, 3, 6, 7-decoder, 9), tests in
  `cat-morse/tests/review_regressions.rs` — written first and seen
  failing for 1, 2, 3, 7, 9 (all 7 initial tests failed); 6's test
  was written alongside the fix (re-acquire is new behaviour).
  - Floor: 10th percentile of recent key-up windows / 0.366 (Rayleigh
    p10→mean), 500-window history; 300 ms warm-up learns floor+peak from
    every window and is replayed. Default squelch back to 10 dB: with an
    unbiased floor, 13 dB failed 6/100 seeds at +6 dB; at 10 dB 0/100
    fail (mean CER 0.01 %). At +3 dB mean CER is 12 % (the pre-fix
    decoder: 73 %) — the advanced band is hard by design.
  - Reference peak = min(instant peak, median of last 5 mark peaks);
    short marks > 3x that median are dropped as crashes; marks longer
    than 3 dahs end the word and stay out of the estimates (3 in a row
    are believed as a speed change).
  - Input clamped to ±4 (non-finite → 0).
  - Auto pitch: bank extends 100 Hz beyond the range; lock vs the band
    median outside ±3 bins of the best; peak-hold decays 0.7/block;
    re-acquire a tone ≥ 30 Hz away after 2 s with nothing heard,
    replaying the last 2 s. Added `auto_pitch_does_not_lock_on_band_noise`
    after a 25th-percentile reference false-locked on noise.
  - `PITCH_MARGIN_HZ` = 100 Hz for Fixed and Auto bounds.
  - Architect accepted the API drift (TimingError::InvalidWpm,
    Difficulty::wpm as (u32, u32)); recorded here as instructed.
- Findings 5, 7 (band), 8: tests `first_sample_is_silent`,
  `band_channel_qrm_starts_from_silence`,
  `band_channel_keeps_qrm_inside_the_passband`,
  `speeds_below_the_floor_are_rejected` (failed first: compile error on
  `MIN_WPM`, first sample 0.124). Fix: `Signal::effective_start()` =
  max(start, rise/2) used by `end()` and the renderer (same half-rise lead
  as `render_tone`); band_channel pitch must be ≥100 Hz from 0/Nyquist,
  QRM side chosen to fit or QRM dropped; `timing::MIN_WPM = 3`,
  `MAX_WPM = 100`.
- Findings 10, 11: `callsign_deserializes_through_parse` (failed first:
  "kf0uwv" came back lower-case, invalid calls accepted) → serde
  `try_from = "String"` via `parse`. `align_unordered_handles_a_huge_copy`
  (40 calls + 240 words): documented O((n+m)³); past
  `UNORDERED_OPTIMAL_MAX` = 128 a deterministic greedy assignment is used
  (the test already passed with Hungarian at 280 — it now exercises the
  fallback). Pinned tests `keying_is_pinned_across_platforms` and
  `script_is_pinned_across_platforms` (FNV-1a over segment nanos / script
  fields incl. f32/f64 bit patterns), recorded on Linux x86_64.
- Finding 4 (+ the `playing` nit): `render_never_waits_on_a_held_lock`
  (holds the lock on one thread, renders on another with a 2 s timeout)
  and `an_empty_callback_does_not_reset_playing`. `render` now uses
  `try_lock` (busy → silence, nothing consumed, `busy_callbacks()`
  counted; also on `AudioPlayback`), a zero-frame buffer is a no-op,
  `write` clamps into a local buffer before locking. ADR 0017 §16
  corrected.
- Review-fix checks (worktree): cat-morse 85+17+10+1 (serde 87+18+10+1);
  cat-signal-audio 55+15+0+3 (device 63+15+7+3); workspace 1103 passed,
  0 failed; clippy -D warnings clean (workspace; cat-morse all features;
  cat-signal-audio device); fmt clean; xwin msvc clean for cat-morse
  --all-features and cat-signal-audio --features device.
