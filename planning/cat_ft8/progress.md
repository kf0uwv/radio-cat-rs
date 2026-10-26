# cat_ft8 — progress

## Session: 2026-10-25 (encoder, first pass)

- Read: CLAUDE.md, ADR 0021 (cat-morse) and its crate as the model,
  ci.yml, the QEX 2020 paper; studied ft8_lib (MIT) constants, CRC,
  encoder and message code.
- Implemented steps 1–7 of task_plan.md on `feat/cat-ft8`; not committed.
- Black-box vectors: 1 020 from `ft8sim` (1 017 match, 2 Field Day
  unsupported, 1 oracle "bad message"); 89 extra edge cases (76 match,
  12 documented deviations where ft8sim truncates or drops text, 1 input
  ft8sim printed nothing for).
- Black-box discoveries are recorded in `findings.md`.
- `jt9` decoded 50/50 clean generated slots and 18/20 at −15 dB; both
  misses reproduce with ft8sim's own audio (jt9 suppresses `/R` messages
  outside contest mode; `/P` at −15 dB decodes 8/10 for both generators).

## Session: 2026-10-25 (fix round)

- Process: the plan for the encoder was written after the code, so the
  first review of this change was retroactive. The user approved the
  fix-round plan (task_plan.md, "Fix round") on 2026-10-25 before any
  fix-round code was written.
- Step 1 done: task_plan.md fix-round steps, findings.md created.
- Steps 2–5 done: `CallTable` refreshes on re-hearing (u64 last-seen
  counter, evicts the oldest); parsing is side-effect free and `pack`
  remembers up to three calls once on success; `unpack` decodes every
  c28 field before writing or remembering; `Error` is `#[non_exhaustive]`;
  reserved types unpack as `InvalidPayload`; `std_call_text` only accepts
  values `std_call_number` produces; type 5 packs and unpacks. All
  existing vector results unchanged.
- Steps 6–11 done: pulse (half, by symmetry) and quarter-wave sine are
  generated static tables checked bit-for-bit against `libm` in a test;
  `Modulator`/`EMPTY`/`init`/`NotInitialized` removed; `libm` is a
  dev-dependency only (the library has no dependencies); `Synth` streams
  in blocks (`fill`, `mix`, `fill_phase`, `skip`) and `synthesize` is built
  on it; `Placement::slot` rejects non-finite dt; amplitude must be
  finite; top tone checked at 7·Δf; `wav.rs` header arithmetic checked;
  golden FNV-1a hashes pinned (identical in debug and release on x86_64
  Linux); profiling moved into `examples/encode_bench.rs`, AWGN into
  `examples/noisy_slot.rs`.
- Step 12 done: `tests/properties.rs` (seeded round trips, 16 000
  messages over every supported type; 200 000-payload fuzz: `unpack`
  never panics, every repack mismatch has a named non-canonical reason).
  Random `ft8sim` vectors (420; 405 match exactly, 1 documented
  deviation, 14 the oracle printed nothing for) in
  `tests/data/encoder-vectors-random.jsonl`. The round-trip test found a
  real bug, fixed: `unpack` remembered a decoded call before looking up
  the message's hashes, so a call could shadow the hash beside it; it now
  remembers only after every hash is resolved and the payload decoded.
  Black-box refinements recorded in findings.md.
- Step 13 done: `cat-ft8/embedded-check` (own workspace, not a member)
  links a `#![no_main]` thumbv8m program with no allocator; `check.sh`
  fails on unresolved symbols or any f64 helper and prints sizes
  (`--stack` adds per-function frames). `tests/no_std_build.rs` removed.
- Step 14 done: ADR 0024 rewritten (scope via ADR 0021, generic embedded
  rationale, provenance widened, `thiserror`, determinism by tables,
  type 5, future waveform API, decoder unpack order); NOTICE, README,
  CLAUDE.md, data README updated; CI: llvm-tools, link check, clippy
  all-features, host no-default-features tests, docs with `-D warnings`.
- Step 15 done: tests (47 default and profile, 45 without std, release
  too), clippy (4 configurations + link check), fmt, docs (both
  configurations), thumbv8m release build. Link check: `.text` 19 522 B,
  `.rodata` 18 002 B (15 624 B are the waveform tables), no `.data` or
  `.bss`; `_start` frame 3 048 B including the caller's 1 920 B block and
  a 16-entry call table. jt9: clean 50/50, −15 dB 18/20 (same two /R and
  /P misses as before, oracle-side); random non-type-1 vectors: clean
  64/70 (all 6 misses /R), −15 dB 27/28 (type 4 miss; paired 9/10 vs
  ft8sim 9/10). Bench (release, x86_64): pack 322 ns, crc 134, ldpc 141,
  symbols 300; waveform 0.79 ms/slot whole, 0.73 ms in 480-sample blocks;
  0 heap allocations.

## Session: 2026-10-25 (cleanup addendum)

- The user's approval of the fix-round plan covers this cleanup (follow-up
  review findings, all small). Addendum steps C1–C15 written in
  task_plan.md before any code.
- Dates: entries earlier written one day ahead corrected to 2026-10-25
  (the machine's local date).
- C1: the link-check program now calls every floating-point entry point
  (`Placement::slot`, `synthesize` at a negative start, `Synth::mix`,
  `skip`, `fill_phase`, `sin_turns`, `cos_turns`); injecting one f64
  operation into `synthesize` now fails the check (`__aeabi_d2f`,
  `__aeabi_dmul`, `__aeabi_f2d`); `check.sh` runs rustfmt and clippy on
  the check program and builds into the repository's `target/`.
- C2: K1AM/K1MB verified to share h10 = 348 with the crate's hash; the
  test asserts the collision, then newest-wins.
- C3: fuzz seeds the call table with the calls behind each hash field, so
  types 1 (hashed), 4, 5 and 0.1 reach the exact-repack check (≥ 7 600
  exact each); every difference has a specific reason; constants used.
- C4: vectors with no oracle text still check bits and symbols (14
  random, 1 extra).
- C5–C12, C14: libm pinned `=0.2.16`; wording; `STATIC_TABLE_BYTES`
  (17 055, linked tables only, tied to the items by a test); licence
  headers; `tables.rs` checked to equal the generator's output exactly;
  `Synth: Clone`; bench splits pack (NoHashes 137 ns) and remember (hit
  34 ns, miss+evict 88 ns); provenance wording; README.
- C15: black-box with jt9 (findings.md): WSJT-X remembers only the
  sending call, as written. `unpack` now does the same; `pack` still
  remembers every call typed.
- Verification: tests 50 (48 without std), clippy in four
  configurations, fmt, docs, thumbv8m release build; link check `.text`
  22 234 B, `.rodata` 18 002 B, `_start` frame 4 304 B (includes the
  check program's 1 920 + 1 024 + 512 B buffers). Golden hashes unchanged.

## Session: 2026-10-25 (profiled optimisation passes)

- Plan T1–T5 written in task_plan.md before code.
- Method: release build, `encode_bench` medians; a pass is judged by an
  interleaved A/B of the before and after binaries (7 runs each, 1 000
  repeats). Unchanged code moves by up to ±3–4% between runs here, so a
  change of that size in a stage the pass did not touch is noise.
- T1 kept — contiguous MSB-first codeword (`[u64; 3]`, `bits(start, len)`),
  Costas copied, data blocks derived from `COSTAS_POSITIONS`. symbols
  300 → 85 ns (−71.7%); ldpc +1.5%; crc +3.1% (code unchanged: noise);
  others within ±1.5%. Link check `.text` 22 234 → 22 222 B, `.rodata`
  18 002 → 18 010 B; working bytes 400 → 392. Results identical (all
  vectors, properties, golden hashes).
- T2 kept — CRC-14 from a compile-time 256-entry `u16` table over 11 bytes
  (6 leading zero bits added; they do not change a zero-start CRC); the
  bitwise form stays as the reference and a test compares them on 20 000
  payloads. crc 129 → 17 ns (−86.8%). In the default build, unchanged
  pack (NoHashes) and ldpc read +8.4% and +4.6% in an 11-run A/B; with both
  binaries built with loops aligned to 64 bytes (a measurement flag only)
  they are −0.7% and −0.7% and crc is −87.2%, so those readings are code
  placement, not the change. `.rodata` 18 010 → 18 522 B (+512, the
  table), `.text` 22 222 → 22 230 B. Results identical.
- A/B from here on: 9 runs each, default build and loops-aligned build.
- T3 rejected (reverted) — one-pass `remember` (duplicate and oldest
  search together) with `u32` stamps and a renumbering clock. Call
  table<64> 1 552 → 1 296 B (−16.5%) and miss+evict 83 → 75 ns (−9.6%,
  aligned), but a hit got slower: 33 → 44 ns aligned (+33%), 33 → 51 ns
  default (+55%), because the oldest-entry tracking now runs on every hit
  that used to stop early. Hits are the common case.
- T4 kept — the pulse table is stored from its first non-zero sample
  (1 215 leading samples are exactly +0.0; the regeneration test checks
  that), and the sample loop skips the previous/next symbol's terms where
  they are exactly zero (bit-identical: adding +0.0 to these non-negative
  sums changes nothing; golden hashes unchanged). Static tables 17 055 →
  12 707 B including the 512 B CRC table from T2 (−25.5%; the 12 195 B
  first reported here left the CRC table out, corrected 2026-10-26); link check `.rodata` 18 522 → 13 662 B (−26%),
  `.text` 22 230 → 22 290 B. Waveform −4.3% in both builds (default
  0.780 → 0.746 ms, aligned 0.769 → 0.736 ms; blocks −3.4%/−4.6%). The
  first version (zero-prefix check inside the lookup) cost +4.6–8.1%
  waveform CPU and was replaced within the pass. ldpc, whose code is
  unchanged, reads −3.6% in the default build and +4.2% in the aligned one
  (11 runs each): opposite signs in two builds of identical ldpc code,
  recorded as unexplained placement noise (no cause established).
- Re-profile after T4 (self time): `CallTable::remember` 18.1%,
  `Synth::mix` 16.1%, `Synth::fill` 15.6%, `normalize` 7.9%,
  `symbols::map` 7.3%. The synthesizer's time is in the store and the
  dependent sine-interpolation chain; the `idx >= 1024` sine branch
  compiles to a conditional move, so padding the sine table would gain
  under 3%. T5 therefore targeted the call table's memory.
- T5 rejected (reverted) — `seen` as `u32` with a renumbering clock,
  two-pass search kept (hit path unchanged). Call table<64> 1 552 →
  1 296 B (−16.5%), but `pack` with a call table got slower by +3.4%
  (default) and +3.5% (aligned) in 13-run A/Bs, consistent in both builds;
  remember hit/miss themselves within ±1%. Over the 3% limit.
- Five passes used (the maximum): T1, T2, T4 kept; T3, T5 rejected.

## Session: 2026-10-26 (follow-up to the optimisation passes)

- Addendum F1–F7 written in task_plan.md before code.
- F1: `Codeword::bits(start, 0)` returned garbage (a 128-bit shift); it
  now returns 0. A fourth, always-zero word removes the bounds fallback;
  `bit()` reads through `bits(i, 1)`; `bits` is `pub(crate)`. The test
  checks every length 0..=32 at every legal start (5 247 cases) against
  the words read bit by bit.
- F2: the CRC table is `pub(crate) static CRC_TABLE` and counted:
  `STATIC_TABLE_BYTES` 12 195 → 12 707. A test scans `src/` for array
  `static`/`const` items (skipping test modules) and fails unless each is
  counted or listed as not linked; checked by adding an uncounted table
  before and after a test module (both failed as intended).
- F3: `crc14_bits` is test-only; `crc14` doc no longer links to it.
- F4: const asserts: Costas at symbol 0 and ending at 79; 3 × 58 = 174.
- F5: the three-run loop (current + previous, current, current + next)
  was bit-identical but measured +3.9–6.4% waveform time in both builds
  (9-run A/B each), so the per-sample loop stays, with a comment saying
  why. Kept from F5: the const assert that at most two pulses overlap
  (strict `2·PULSE_FIRST > ns`; with equality one sample would need all
  three) and the module doc ("at most two pulses").
- F6: the two corrections above.
- F7: `encode_bench --stage NAME [REPEATS]` runs one stage only, for
  `perf stat -e instructions:u` (subtract REPEATS = 0 for setup): crc
  about 121 instructions per message, symbols about 802.

## Session: 2026-10-26 (decoder plan)

- Phase 1 committed; ADR 0024 accepted.
- Phase 2 (single-pass decoder) planned: task_plan.md "Phase 2" (steps
  D0–D11, checkpoints), ADR 0024 "Proposed amendment: single-pass
  decoder", sizing notes in findings.md. No decoder code written; waiting
  for architect and user approval (D0).
- Both architecture reviews: approve with changes. Plan, ADR amendment
  and findings revised: row geometry and dt range with partial sync, band
  definition, text rendered in `finish` with the caller's call table,
  exact determinism contract, scope rule 3 restated, D7 split with a
  checkpoint, link check from D2, all-zero rejection, local-maximum
  suppression, slot framing, payloads-only finish, command output
  conventions, aarch64 golden hash, time-budget definition. Open questions
  reduced to two.
- Confirmation (approve with two edits) applied: local-maximum tie rule
  and grid edges with a plateau test (D4); explicit `ubuntu-24.04-arm` CI
  task (D8b). Also: integer 6 / n normalisation of partial-sync scores,
  separate entry point for later audio input, hand-written JSON with
  escaping, no `sqrt`, sync operation count corrected, "every symbol
  centre has a row" wording.

## Session: 2026-10-26 (D0)

- The user approved the Phase 2 plan with the final edits (2026-10-26)
  and decided both open questions: QEMU (MPS2 Cortex-M33 model) now for
  instruction counts, no microcontroller speed claims until measured on a
  board chosen later; on a 256 KiB part, the small configuration plus a
  1 s i16 FIFO and a 1 s decode budget.
- ADR 0024 amendment accepted with estimated figures; measured values
  replace them at D11. D0 done; waiting for the go-ahead to start D1.

## Error Log
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-10-25 | Stage timings ~6 µs each | 1 | `Instant::now()` overhead on this host; time each stage as one batch |
