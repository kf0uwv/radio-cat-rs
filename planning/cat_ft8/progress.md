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
  first check of this change came after the code. The user approved the
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
- Architecture checks of the plan: approved with changes. Plan, ADR amendment
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

## Session: 2026-10-26 (D1–D3)

- Plan and ADR committed (264ee4c); go-ahead for D1–D3, stopping at the
  D3 checkpoint.
- D1 done: `decode::math::log2` from the IEEE exponent and the atanh
  series on [√½, √2] (z⁹ term), `log2_power` with a 2⁻¹⁰⁰ floor and an
  `f32::MAX` cap (NaN maps to the floor). Tests: within a few ulps of
  `libm` over 2⁻¹⁰⁰…10³⁰ (scaled tolerance: the result's ulp grows with
  |log2|), exact at powers of two, four pinned bit patterns, floor/cap.
- D2 done: `decode::fft` — 1 920-point complex mixed-radix FFT (stages
  4, 4, 4, 2, 3, 5; in place; compile-time digit-reversal table checked
  against the recursive definition), real split for 3 840 samples, Hann
  window; tables (twiddles, real-split twiddles, half window) generated
  and checked bit-for-bit; public `power_spectrum` and `FftBuffer`.
  Accuracy against an f64 DFT (tests only): complex transform max error
  1.3e-7 of peak; real windowed power 2.7e-7 of peak, ≤ 2.9e-5 dB in bins
  within 60 dB of peak; a full-scale sine centred in a bin reads 1.0.
  Link check calls `power_spectrum` from statics: `.rodata` 13 662 →
  48 338 B (+34 572 B of FFT tables), `.bss` 30 720 B, no f64 helpers.
- D3 done: `decode::frontend` — `Band` (base bins; stored = base + 15),
  sealed `Level` (u8 0.5 dB steps, u16 1/512 dB steps; 0 dB = full-scale
  sine in a bin), `FrontEnd` (30 720 B, const), `Spectrogram` borrowing
  it and the caller's level slice: `start_slot`, `feed`, `feed_i16`,
  `skip`, `rows_filled`, sanitised/skipped counts, `row`. Rows from sample
  240, hop 960, 184 rows. Tests: tone in its bin; nominal-start Costas
  symbols peak on rows 5 + 2j; identical rows for block sizes 1, 7, 960,
  1 921, 4 096 and whole slot; `skip` and non-finite input equal zeros and
  are counted; i16 equals scaled f32; partial slot, slot end, start_slot;
  band validation; buffer size check. New errors `InvalidBand`,
  `BufferTooSmall`. Link check feeds a synthesized slot (with `skip`)
  into a small-configuration spectrogram in statics: `.text` 27 042 B,
  `.rodata` 48 466 B, `.bss` 223 360 B (levels 161 920 + front end
  30 720 + 30 720 for the separate `power_spectrum` call), no f64.
- D3 checkpoint measurements (`examples/frontend_bench.rs`, release,
  AMD Threadripper 3970X): `feed` 77–80 µs per 80 ms hop (≈ 0.1% of real
  time), same for 8- and 16-bit and for 480-sample blocks. Level width,
  100 slots per SNR, data-symbol hard decisions on the known rows: 16-bit
  equals unquantised f32; 8-bit is 1.8 points lower if ties count as
  wrong, but +0.05 points behind with ties split (within noise; at
  −26 dB 8-bit is ahead). Clipping: both widths cover −128…0 dB; 8-bit
  levels start to floor for input noise below about −80 dBFS RMS (1.2% at
  −80, 11% at −90, 70% at −100). Golden hashes unchanged. Stopped at the
  D3 ◆ checkpoint.
- D3 checkpoint reviewed: proceed after a small fix-up (D3a in the plan).
  Decisions: one public 8-bit configuration (`u16` kept only as the D6
  baseline, deleted at D6 unless belief propagation shows a gain beyond
  noise); the level range is not shifted (a shift would clip strong
  signals); input levels documented with no enforced minimum.
- D3a done: non-finite samples before row 0 counted; finite samples
  clamped to ±2¹⁶ and counted (`clamped_samples`); `power_spectrum`,
  `FftBuffer`, `WINDOW` and the `fft` module crate-private (`decode::
  TABLE_BYTES` re-exported for the memory account); `row()` returns
  `Option` and works through `&self`; `u16` levels only under `cfg(test)`
  or `profile` (hidden); input-level section in the module docs; counters
  `floored_levels` and `top_levels`. New tests: shared-reference reads,
  early counting and clamping, floor/top counters. Tests 75 (73 without
  std); link check `.text` 26 766 B, `.rodata` 48 466 B, `.bss` 192 640 B
  (levels 161 920 + front end 30 720: the real small configuration),
  largest library frame `Spectrogram::finish_row` 232 B. Golden hashes
  unchanged. Waiting for the commit of D1–D3 and the go-ahead for D4.

## Session: 2026-10-26 (D4–D6)

- D1–D3 and D3a committed (8eebc3b); go-ahead for D4–D6, stopping at the
  D6 checkpoint.
- D4 done: `decode::sync` — integer Costas score (8·L[expected] − Σ L
  over the eight tones per symbol), arrays counted only when all seven rows
  are filled, ≥ 2 arrays, × 6 / n; start rows −26…36 (±2.48 s); generic
  `select` with 3 × 3 local-maximum suppression (≥ later neighbours, >
  earlier ones; outside-grid or unscored neighbours impose nothing) on a
  rolling three-row buffer, then top-K by (score desc, row, bin) with
  found/kept/dropped counts. Tests: plateaus 1 × 2, 2 × 1, 2 × 2, two
  L-shapes, both grid edges keep exactly their first point; order, ties
  and truncation; encoder-made signals found at their exact grid point;
  both ends of the time range via partial sync; weak signals 20 bins
  beside a 0 dB signal at −15 dB still candidates; partial slot scoring.
  Link check: `.bss` 204 620 B (+10 380 scratch, +1 600 candidates).
- D5 done: `decode::soft::soft_bits` — max-log per bit over the Gray
  masks on integer levels (ties give exactly 0, no tone or bit value
  favoured), erasures (exactly 0) for symbols without a filled row,
  scaling to a target mean magnitude over bits not erased (no square
  root). Tests: Gray masks; a clean signal gives every sign right and the
  target mean; a start row of −20 erases exactly the 3 data symbols before
  row 0 and the rest are right; silence (all ties) gives all zeros. Link
  check `.text` 28 362 B, `.bss` unchanged.
- D6 done: `decode::bp::decode_soft` — flooding message passing over the
  parity-check table, fixed summation order, early stop when every check
  holds, round limit, CRC, all-zero rejection, round and hard-error
  counts; `CheckRule::SumProduct` (tanh through the in-crate `exp2`, a
  7-term polynomial; atanh through the in-crate `log2`) and
  `CheckRule::MinSum { alpha }`. Exact ties (0) in hard decisions go to the
  bit's index parity, so erasures do not pull towards the all-zero word.
  Statics: `BpScratch` (3 892 B). Tests: clean codeword in 0 rounds;
  all-zero codeword rejected; an all-erased input never decodes; noisy
  codewords (mean 2, sigma 1.4) 91/100 sum-product, 95/100 min-sum 0.75;
  erasures recovered; 300 pure-noise inputs never pass; reused scratch
  gives the same result; tanh/atanh helpers; `exp2` within 3e-6 relative.
- D6 measurements (`examples/decode_bench.rs`, first pass on the coarse
  grid only, single signal, seeded noise, 300 slots per point,
  sum-product, LLR scale 4, 30 rounds, K = 200): decoded 17 / 50 / 101 /
  158 / 215 / 277 / 292 / 300 of 300 at −20 / −19.5 / −19 / −18.5 / −18 /
  −17 / −16 / −14 dB, 0 at −21 dB and below; 50% near −18.6 dB. False
  decodes 0 in every signal slot, 0 in 3 000 noise-only slots, 0 in 3 000
  slots with three carriers 10 dB above the noise (16 460 all-zero
  codewords rejected there, none in noise). Min-sum 0.75: 15 / 38 / 98 /
  152 / 213 / 275 / 293 / 300, the same false-decode counts. Over 8 000
  paired slots (−20…−17 dB) sum-product decodes 1.1 points more than
  min-sum 0.75 (3 967 against 3 877); min-sum 0.875 ties 0.75, 0.625 is
  worse; sum-product scale 3–6 is flat, 2 loses, 1.5 collapses. 8- against
  16-bit levels on the same 8 000 slots: 3 967 against 3 981
  (sum-product), 3 877 against 3 896 (min-sum), +0.2 points, within
  noise; decision: delete the 16-bit level type. Host cost (one thread,
  Threadripper 3970X, all 200 candidates failing, 30 rounds each): soft
  bits + BP 375 µs per candidate sum-product, 131 µs min-sum; feed 14.6 ms
  per slot, sync search 4.6 ms. Link check: `.text` 30 658 B, `.rodata`
  49 058 B, `.bss` 208 512 B; largest frames `_start` 4 200 B (BP inlined),
  library `finish_row` 232 B, `hard_decide` 96 B. Golden hashes unchanged.
- D6 checkpoint reviewed: proceed. Before the D4–D6 commit, D6a
  (checkpoint corrections) is recorded in the plan; D6b (delete 16-bit levels) waits for a
  go-ahead, then D7a/D7b with the review decisions.
- D6a done: `hard_decide` tie test (all 0.0 and all −0.0 give
  `hard[i] == i % 2`; signs decide elsewhere); `hard_errors` counts only
  bits with a non-zero soft value (documented; test: 24 erased bits give
  0, one flipped bit gives 1); `CheckRule::MinSum` fieldless with
  `bp::ALPHA = 0.75` (the bench's α comparison removed); `decode::TABLE_BYTES`
  now adds `PARITY_CHECKS` (581 B; `DECODE_COUNTED`, checked against the
  tables by a test); `exp2` coefficients E1–E7 pinned by bit pattern and
  checked against (ln 2)^k / k!. Tests 89 (87 without std); link check
  `.text` 30 666 B, `.bss` 208 512 B, `_start` frame 4 184 B.
- D4–D6 and D6a committed (2546586). D6b: the 16-bit level type
  deleted (decision and evidence in findings, "D6 decision: 8-bit levels
  only"); `Level` sealed with `u8` only; the decode bench's `levels` mode
  and the front-end bench's level-width section and 16-bit timing
  removed; tests that compared or used 16-bit levels now use 8-bit.
- D6b committed (24e9070). D7a/D7b go-ahead with the D6 checkpoint
  decisions; D7 design recorded in the plan.
- D7a done: `decode::decoder` — `DecodeConfig` (`HOST` sum-product,
  `SMALL` min-sum; `Error::InvalidConfig`), `Decoder` over the caller's
  buffers with the split API (`find_candidates`, `decode_candidate(&self,
  index, &mut CandidateScratch)`, `DecodeList::push`/`merge` in index
  order, `Error::OutOfOrder`), `decode()` as exactly those calls,
  `DecodeList::finish` (callback, caller's `CallHashes`, unpack errors
  reported) and `finish_payloads`, `DecodeReport` (with all-zero
  rejections, CRC failures, not converged, duplicates, truncation and
  the front-end counters). `sync`/`soft`/`bp` crate-private, public with
  `profile` only. Tests (9): configuration and buffer checks; three
  signals with their fields; split API identical to `decode()` for 1, 3,
  4 and 7 threads, either order, feed blocks 1/777/4 096/whole; push
  order enforced; duplicates keep the lowest index, truncation counted;
  a carrier gives all-zero rejections and no decode; `finish` reports a
  payload it cannot unpack; the caller's hash table resolves a hashed
  call; a reused decoder matches a fresh one, and a 14 s partial slot
  decodes. The bench sweep through `Decoder` reproduces the D6 figures
  exactly. Link check on the `Decoder` API: `.text` 31 806 B, `.bss`
  211 772 B, `_start` 4 096 B, `unpack` 464 B.
- D7b done: `sync::refine` (parabola through the sync scores of the
  grid neighbours, clamped to ±½ step), `soft::soft_bits_at` (power
  interpolated bilinearly at the refined position, from a 256-entry
  level-to-power table generated with `libm` and checked bit-for-bit;
  zero-weight rows ignored), refined frequency and dt in `Decode`, and
  `snr::estimate` (decoded tones against the median of 8 reference bins
  each side, 25–50 Hz from the signal; +2.3 dB calibration; floor −30
  dB). The grid soft bits remain for tests and `profile` only. Tests
  (+5): refinement moves towards sub-grid offsets; interpolated bits at
  the grid point follow the integer bits (signs, zeros, erasures); SNR
  with local noise on a one-pole-shaped noise floor (two signals −10 dB
  locally, read within 1.5 dB); SNR tracks the level −16…+20 dB within
  1 dB; memory per item. Field tolerances in the three-signal test
  tightened (0.8 Hz, 20 ms, SNR ±1 dB). Decoder doctest added. Results in
  findings ("D7 measurements").
- ◆ D7 checkpoint reached (2026-10-26): tests 103 (101 without std),
  2 ignored, integration 2 + 5, doctests 3; clippy (four
  configurations), fmt, docs, thumbv8m build; link check `.text`
  33 734 B, `.rodata` 50 118 B, `.bss` 212 284 B, `_start` 4 216 B;
  golden hashes unchanged.
- D7 checkpoint reviewed: proceed after D7c (recorded in the plan with
  the D8 notes).
- D7c done: `push` rejects a decode carrying another index (nothing
  changed); `#[non_exhaustive]` on `Decode`, `DecodeReport`, `BpFailure`
  and `DecodeConfig`, with const `with_rule`/`with_max_rounds`/
  `with_llr_scale`/`with_min_score`/`with_candidates` setters (decision:
  callers start from `HOST`/`SMALL`; fields stay public for reading;
  doctest); `const fn` `Band::first_bin`/`base_bins`/`stored_bins`,
  `sync::scratch_len`, `Decoder::sync_scratch_len`, and the link check's
  sync scratch is sized from it; stale-candidate note on `feed`; counters:
  exact counting would need the truncated payloads (up to the candidate
  capacity), so `decoded` is documented as exact until truncation and an
  upper bound after; `snr_db` documented as against local noise; ADR
  drift fixed (merge does rejection, dedup and truncation; `decode()`
  excludes `finish`; capacity constants; level-to-power table in the
  rule-3 tables); Hann doc comment moved back to its generator. Tests:
  `SMALL` end to end in the split/determinism test, chunked `merge`
  equals one merge, mismatched candidate index rejected, the setters.
  Checks: tests 103 (101 without std), 2 ignored, integration 2 + 5,
  doctests 4; clippy (four configurations), fmt, docs, thumbv8m build;
  link check `.text` 33 706 B, `.rodata` 50 118 B, `.bss` 212 284 B,
  `_start` 4 216 B; golden hashes unchanged.
- D7 committed (a3adafb). D8/D8b go-ahead; D8 design recorded in the
  plan.
- D8 done: SNR noise reference per side (24 bins, 25–100 Hz, lower
  octile corrected for the exponential distribution, sides averaged);
  bench mode `busy`; `tests/common` (libm noise, buffers, parallel map,
  output hash), `tests/decode.rs` (10 tests: pinned floors; golden
  hashes for HOST and SMALL with a block-size check; overlaps 10–30 Hz;
  early/late/band-edge; partial input and decoding before the slot ends;
  input starting late; reusable and alternating workspaces; seeded call
  table; SNR on a steep passband; SNR beside a strong neighbour) and
  `tests/decode_bulk.rs` (ignored; release job). Two library SNR
  tolerances widened from 1.0 to 1.5 dB (single-slot, about 3 σ).
  Results in findings ("D8 measurements").
- D8b done: CI jobs `ft8-bulk` (release, the bulk run) and `ft8-aarch64`
  (`ubuntu-24.04-arm`, `cargo test -p cat-ft8` with and without std);
  the Linux and Windows jobs run the decode tests through the workspace
  tests. Bulk run checked locally; aarch64 and Windows need a real CI
  run (no local runtime for either).
- D8/D8b committed (dd51ea1). D9/D10 go-ahead (D11 is not part of this repository's CI);
  design recorded in the plan.
- D9 done: `wav::read_wav` (16-bit PCM, plain or extensible, any rate
  and channels, unknown chunks skipped, over-long data chunk read to the
  end; 4 tests); `cat-ft8-decode` (split API with stage timings, 64-call
  table, JSON by hand with escaping; 2 unit tests); output format version
  0 documented in the crate docs and the ADR; `tests/decode_cli.rs` (5
  tests, `serde_json`): lines and keys for both configurations, `--raw`
  with an unsupported (0.3) payload, `--profile` (format, timings, memory
  with `peak_bytes` = the sum, every report count), 14 s and 30 s files,
  and errors (48 kHz, stereo, not a WAV, missing file, bad options).
- D10 done: the link check builds three images (small, host,
  encoder-only), each checked for unresolved symbols and f64 helpers and
  sized; the decoder path is a separate, non-inlined function so its
  stack frame is reported. Results in findings ("D10 sizes").

## Session: 2026-10-27 (D12 fix round)

- Checks of the whole decoder consolidated into fix list sections
  A (required, results identical), B (simplicity, results identical)
  and C (optimisation passes, later). The user approved A and B followed
  by C on 2026-10-27. D12 plan written into the task plan before code;
  C waits for a re-check.
- D12 done, one item at a time; after each, every test (golden waveform
  and decode hashes, pinned floors) and the release bulk run were checked
  (bulk counts compared line by line with a baseline taken after A6
  extended the run). Decode results identical throughout.
  - A1: the one `f32::abs` in the core replaced (a range check); the
    library builds on 1.75 with and without std; CI step added.
  - A2: decoder provenance in `NOTICE`, the crate docs and the ADR table
    (oversampling, numeric helpers row, mean-magnitude scaling).
  - A3: the decode determinism condition (no flush-to-zero) in the crate
    docs.
  - A4: ADR memory table measured (host 234 680 B, 236 232 B with the call
    table; small `.bss` 212 280 B; tables 36 260 B), link-check sizes and
    frames, the external benchmark's figures stated generically, the host
    budget's statistic and load, and its lack of margin.
  - A5: README (decoder, about 1 470 workspace tests), CLAUDE.md, Cargo
    description and feature comments (profile modules unstable), `snr_db`
    doc, `Error` doc, bench header, `merge` doc, α tie-break, the
    cannot-fail comment, `Decoder` lifetime note, plan wording.
  - A6: golden slots assert only transmitted decodes, exact counts (HOST
    8/0/8, SMALL 7/0/7 of 8 sent; no false decode exposed); `feed_i16`
    test; bulk run extended (strong carriers, clipped, floored,
    impulsive: 0 decodes); `llr_scale` ≤ 1 000 with a test.
  - A7: `--config small` = 200–2900 Hz, min-sum, 64 decodes, reported in
    the profile; `--help` to stdout, exit 0; `wav::read_wav_prefix` reads
    only the needed chunks and frames (test).
  - B8: `Level` trait and generics removed (`u8` levels, `level_of`).
  - B9: shared `fill_and_scale` for both soft-bit forms; the grid form's
    bench counts reproduce D7 exactly.
  - B10: compile-time 0-based `CHECKS` and `DEGREES` (test against the
    matrix); `TABLE_BYTES` 36 260.
  - B11: `classify` helper in the front end.
  - B12: `Debug` without buffers (test); bench rounds for converged
    candidates only.
  - B13: `Stage` and `decode_candidate_staged` (same result, test); the
    command's profile adds `refine`, `soft_bits`, `belief_propagation`
    and `snr` seconds.
- D12 re-checked and approved. D12a cleanup recorded in the plan;
  the coordinator authorised local commits by topic (no push).
- Note: A6 changed the bulk run's seeds, so its zero decodes from then on
  are a new sample of noise and carriers, not a re-run of the earlier
  slots.
- D12a done: `Stage` exhaustive with `ALL`, `index()` and `name()` (the
  command builds its keys from them; no catch-all); `CHECKS`/`DEGREES`
  static; golden setups now HOST, min-sum on the default band (renamed
  from "SMALL") and the real small setup (Band::SMALL, SMALL, 64
  decodes: 6 / 0 / 6 of 8 sent, hash 0xdd6e4bf08d7d91f9, new); ADR flash
  rounded with the toolchain named, time-budget sentence per process
  under parallel load as an upper bound, D11 marked done (outside this
  repository), two long lines wrapped, D12 sizes in findings; `NOTICE`
  names ADR 0024; stale test comment; bench prints n/a. Existing golden
  hashes, floors and bulk counts unchanged.

## Session: 2026-10-27 (C: optimisation passes)

- D12/D12a committed as eight commits (8f4876d..deff523). The user
  approved section C (up to five optimisation passes, results
  identical); plan written before any change.
- Baseline (deff523, loop-aligned build, 40 crowded slots × 2,
  interleaved, one process at a time): total 94.4 ms per slot — belief
  propagation 70.7, feed 14.8, soft bits 3.2, sync 2.9, refine 1.3, SNR
  0.1; 7.127 × 10⁹ instructions over 10 crowded slots. External identity
  check of deff523 against the d47eb5f runs: identical on both sets;
  golden 60/60. (The harness's parallel CPU means moved +11% / +3% with
  no code change: too noisy to judge a pass.)
- Pass 1 (kept): sum-product check updates computed over fixed eight
  lanes (padding lanes 0.0, discarded) with the suffix products as an
  array in the same order, and `exp2_neg`/`log2` written with selects
  instead of branches; every used lane's operations are unchanged. A/B:
  belief propagation 70.73 → 45.65 ms (−35.5%), total 94.45 → 69.23 ms
  (−26.7%), other stages within ±1.2%; instructions −15.6%. Identity:
  cargo tests, bulk run, crowded and sweep `--identical` (0 slots
  differ; harness CPU −24% / −25%), golden 60/60. Link check: flash
  +112 B (+0.13%), decoder-path frame 2 496 → 2 568 B (+2.9%).
- Pass 2 (rejected, reverted): FFT twiddle index stepped by d and
  wrapped instead of a modulo per term (same twiddles, same order;
  radix-specific butterflies were ruled out because the table's
  quarter-turn twiddles are not exactly ±1, ±i, so specialised kernels
  would round differently). Results identical (cargo tests). Feed-only
  bench, ten interleaved rounds of 100 feeds: 14.51 → 14.09 ms (−2.9%,
  sd 0.13 ms); per-feed instructions 212.2 → 203.0 M (−4.3%), taken as
  the difference of N = 40 and N = 20 runs divided by 20, so the set-up
  is subtracted. The CPU gain is real but under 3%, so rejected. (An
  earlier note here said the modulo is a hardware divide on a
  microcontroller: wrong. N = 1 920 is a constant, so `% N` compiles to a
  multiply-high on thumbv8m as on the host, and the gain there would be
  about the host's.)
- Pass 3 (rejected, reverted): belief propagation swaps its two total
  buffers by reference instead of copying their contents each round
  (the memmove seen in the profile). Results identical (cargo tests);
  A/B: belief propagation −0.5%, total −0.2%.
- Two passes in a row under 3%: stopped (passes 4–5 not run). Kept: pass
  1 only. Tooling kept: `decode_bench stage N feed|sync|bp` (the
  argument order the code takes; commit a0b5961's message gives it
  reversed) for single-stage instruction counts.
- Pass checks (correctness, and simplicity): both pass with
  small changes; close-out C1 recorded in the plan.
- C1 done (results identical): `exp2_neg` pinned at 19 inputs (edges
  included) plus a pinned hash over a 10 001-point sweep of −126…0; a
  belief-propagation test runs five rounds on a fixed non-converging
  vector and pins a hash of the final totals and messages for each rule.
  Mutations, each reverted: E7 × 1.001 fails both new tests; `exp2_neg`
  result × (1 + 2⁻²³) fails both; one ulp on each `twice_atanh` message
  fails the state hash; α one ulp up fails the state hash. (E7's last
  digit 4 → 6 rounds to the same f32, so it is no mutation.) `LANES` and
  `MAX_DEGREE` with a compile-time bound, `q` built in `LANES` lanes, the
  scalar-FPU cost and SMALL's rule noted at the lanes; `math.rs` comment
  and `tanh_half` explained; bench `stage` usage documented (set-up
  included in counts, `bp` covers refine and soft bits, failure path
  only) and "n/a" for N = 0; ADR frame (2 544 B now that `q` is built
  in place; 2 568 B after pass 1) and post-pass cost.

## Session: 2026-10-27 (Phase 3 plan)

- Phase 2 closed and pushed (tip 724e89b). Phase 3 (single-pass changes
  that alter results: early stop, host rule, fewer candidates, CRC
  continuation as a measurement first) drafted in the plan; awaiting
  approval, no code.

- CRLF fix for the two generated-table tests committed (d768627) after
  the Windows CI run failed on them.
- Phase 3 plan approved by the user (2026-10-27), revised as the
  architects asked: order E0 → E1 → E3 with E2 and E4 measurement-only;
  a checkpoint after every step (E0 reviewed before E1); paired results
  saved per slot and per message; crowded compared per message;
  false-decode gate (zero on empty and bulk, new ≤ old elsewhere, an entry
  per false decode); chosen values confirmed on fresh seeds; 50% point
  within about 0.05 dB; cost as medians of five runs plus deterministic
  counts, thresholds against the previous step; "off" pins kept, new pins
  for new defaults; `stall_rounds: u8` API; no CLI flag. E0 not started.
- E0 done (results identical): `examples/paired_bench.rs` (`run`,
  `compare`, `cost`): sets sweep, crowded (with ground truth), edges,
  empty and a fresh seed block; per-slot JSON lines keyed by set, block,
  index and seed with the transmitted messages, decodes, report counts,
  rounds and stage times; paired tables per point and per set with the
  noise gate, the false-decode gate and the 50% point with bootstrap
  intervals; deterministic counts; interleaved medians for cost. For
  exact rounds per candidate, `BpScratch::rounds()` and
  `CandidateScratch::last_rounds()` record the rounds a decode ran,
  whatever its result (test added). Edges set at −18 dB, so a loss can
  show. Baselines for host and small in findings ("Phase 3 E0
  baseline"); no false decodes anywhere. Checks: tests 115 (108 without
  std), golden hashes and floors unchanged, bulk identical, clippy, fmt,
  docs, thumbv8m, 1.75, link check (flash 83 504 B, `.bss` unchanged).
- E0 checkpoint checked. E0a recorded
  in the plan: gate defects, traceable baseline, shared helpers, typed
  records; `last_rounds()` kept public as a deliberate addition.
- E0a done (decoder library untouched): `compare` fails on MISSING sets,
  on nothing compared, on missing main sets (unless `--partial`), on a
  missing 50% point and on any input difference (seed, sample hash,
  messages); reports dropped resamples, truncation and dropped
  candidates; exits 1 on FAIL and 2 on usage or I/O errors. Slots reuse
  `tests/common` (so the slots changed and the baseline was re-recorded);
  typed `SlotResult` with its own JSON writer; statistics documented;
  `cost` spreads 42 slots over the three sizes; `run` records commit,
  rustc and CPU. Baseline re-recorded (findings, "Phase 3 E0 baseline"),
  re-run of host pairs zero-discordant, PASS; failure cases exit 1.
  `last_rounds()` recorded as a deliberate public addition in the plan
  and ADR 0024.
- E0 checkpoint passed. E1 started: the stall mechanism committed first
  with stall off as the default (results identical): `DecodeConfig::
  stall_rounds` (0 = off, at most `max_rounds`, `with_stall_rounds`),
  `bp::decode_soft_stall` (`decode_soft` is the stall-off case, now
  tests and benches only); the unsatisfied-check count is computed only
  when stall is on. Tests: config bound; stall-on state pin with its stop
  round (both rules stop after round 5 at stall 3, in the same state as
  five stall-off rounds); stall changes no decode that converges. Golden
  hashes, floors, stall-off state pin and bulk unchanged; off-path A/B
  total −0.1%; flash +200 B, stack unchanged. Bench config key `stall=K`.
- E1 sweep done (findings, "Phase 3 E1: early-stop sweep"): host stall
  2–20 and caps 10/15/20, small stall 2–20, each on all main sets and the
  fresh block against the E0a baseline. No host configuration passes;
  small passes only at stall 20 (3% fewer rounds). Near-passing settings
  save 18% of belief-propagation time at most (host stall 15). E1 fails
  its gate as specified; no default change, nothing re-pinned, no
  candidate binary. Stopped at the E1 checkpoint.
- E1 rejected at its checkpoint; per the plan the mechanism (3c7bde9)
  is reverted in 42e551e. Alternative stopping signals recorded as later
  ideas. Next: E3 with stall off, against the E0a baseline.
- E3 sweep done (findings, "Phase 3 E3: candidate-count sweep"): host
  and small with 150/120/100/75 candidates, all main sets and the fresh
  block against E0a: every value passes every gate; 75 chosen (passes
  on both blocks); cost host total −49.8%, small −40.7%. Candidate binary
  built; default change saved as a patch, not committed; stopped at the
  E3 checkpoint.
- E3 comparison with an external reference decoder passed. Default
  applied, uncommitted for the checkpoint reviews: the presets' `candidates` = 75 (`CANDIDATES`
  unchanged, documented as the buffer size); golden setups now six — the
  existing three as explicit 200-candidate pins (hashes unchanged) and
  three new pins for the default (hashes 0x8fb5e2a8bab1a8c3,
  0x9fd1ce9bbfce8d9f, 0x4356eee942f36a23; counts unchanged); floors
  unchanged (same counts); the carrier all-zero test runs with 200
  candidates (with 75 its candidates rank below the cut-off); HOST/SMALL
  docs and ADR 0024 updated with the measured cost.
- E3 decision: `HOST` 100, `SMALL` 75 (75 was the lowest value swept,
  lost the host fresh-block message at candidate rank 78 and had a
  one-signal external margin on 40-signal slots). The 75-candidate HOST
  pin 0x8fb5e2a8bab1a8c3 is replaced by 0xb98914394016eb1f (100); the
  two SMALL pins and the three 200-candidate pins are unchanged. The
  200-candidate configurations join the rate floors (same counts); the
  golden carrier slot asserts an all-zero rejection. Re-run at the
  defaults against E0a: both pass every gate on both blocks (each loses
  the main-sweep slot 4436 message, gains none, no false decode; 50%
  shift +0.000 dB, bootstrap interval [+0.000, +0.000]). Cost: host
  66.4 → 40.4 ms (−39.1%), small 44.8 → 26.3 ms (−41.4%) per crowded
  slot. External check of the 100-candidate binary passed; nothing lost.
  HOST/SMALL rustdoc trimmed to what each preset is; the figures moved
  to ADR 0024, which now says `CANDIDATES` sizes the buffer for the
  largest preset or `with_candidates` value. E2 is next, with stall off
  at the new defaults.
- E2 measured (findings, "Phase 3 E2: host check-node rule, measured"),
  stall off at the E3 defaults, against the default runs in
  `e3-final`: `HOST` with min-sum loses 94/91 sweep messages and gains
  28/21 (main/fresh), crowded 23/24 against 7/3, edges 11 against 1; no
  false decode; 50% point +0.038/+0.034 dB; it fails the no-loss gate on
  every set with signals. Cost: total 40.6 → 30.8 ms (−24%), belief
  propagation −47%. `SMALL` with sum-product is the mirror image (the
  same messages, sides swapped) at +25% total, +88% belief propagation.
  No default change; nothing re-pinned. Stopped at the E2 checkpoint.
- E4 measured (findings, "Phase 3 E4: continuing past a CRC-failing
  codeword, measured"). Bench only: `paired_bench crc` and, in `profile`
  builds, `bp::decode_soft_observed` (a copy of `decode_soft`'s loop
  that reports every codeword round and can run on; tests pin it to
  `decode_soft`'s stop, result and state). CRC failures about 0.015 per
  slot (1.5 per 10 000 candidates host, 1.1 small), the same on empty
  input; at most 2 missing messages per set had a CRC-failing candidate
  on them; continuing to the round limit reached no second codeword in
  1 964 candidates (nor past 139 166 all-zero rejections). Expected
  false-decode rate as decoded ≈ 9 × 10⁻⁷ per slot (host); continuing
  adds none measured. Recommendation: do not implement. Decoder
  unchanged: default release binary byte-identical to 8c410e0's, golden
  hashes unchanged, default runs record-for-record identical to
  `e3-final` (b = c = 0 on every set and block). Stopped at the E4
  checkpoint.

## Session: 2026-10-27 (subtraction and further passes, S0)

- S0 started: clean-up after the E2/E4 reviews, the subtraction ceiling
  (bench only), then a plan for approval. Results stay identical; no
  subtraction code in the decoder.
- Clean-up done (results identical): `bp::decode_soft_observed` and its
  two tests deleted (the decoder sources now equal 8c410e0's); the
  continuation half of `paired_bench crc` deleted (1870e8b reproduces
  E4). `run` saves each transmitted message's outcome (decoded, or no
  candidate on it / not converged / CRC failure / all-zero, by
  position), and `compare` prints the misses by cause and the CRC
  failures per slot with the expected false decodes for both sides.
  One decode helper serves `run`, `cost` and the new measurement; the
  slot rendering is shared. Findings wording (E2: min-sum as
  configured, two statements marked as assumptions; E4: no fixed-point
  claim, CRC failures are noise, ordered statistics sized by
  not-converged candidates); ADR 0024's scope marks continuing past a
  CRC failure as measured, not pursued. Identity: release binary sha256
  b3ead07b…dea93 unchanged, golden hashes unchanged, every slot record
  equal to `e3-final` (b = c = 0 on every set and block).
- Subtraction ceiling measured (findings, "Subtraction ceiling"):
  `paired_bench ceiling` decodes each crowded message alone (same noise)
  and runs oracle passes that remove the decoded signals' true
  waveforms. Every message decodes alone (ceiling 100%: the crowded set
  starts 3 dB above the 50% point). Host 69.1% single pass, 91.0% (main)
  / 91.6% (fresh) after one oracle pass, 96.5% / 97.0% after two; small
  within 0.2 points; no false decode or duplicate anywhere. Misses: about
  80% not converged, 20% no candidate; one oracle pass recovers 77–79%
  and 47–50% of them. Decoder unchanged.
- Plan for S1–S3 written (task plan, "Phase 4"), with an ADR 0024
  amendment draft (proposed, not accepted): reference waveform with a
  sub-sample start (S1), subtracting one signal measured as residual
  suppression (S2), a pass count with paired acceptance (S3); the
  additive API (`subtract`, `start_pass`, `DecodeList::next_pass`,
  `DecodeConfig::passes`), fixed subtraction order, acceptance bars,
  memory, risks and five decisions for the user. Stopped at the S0
  checkpoint for approval.
- S0 review fixes, bench (results identical): `paired_bench` split into
  `examples/paired_bench/`; per-point verdict, sweep bootstrap and
  per-set comparison as functions; `ceiling` tracks the first decoding
  pass per message; "other failure" for unknown belief-propagation
  failures; "not recorded" for old runs of `empty`; no-candidate misses
  split into no sync peak / peak dropped by the candidate limit; the
  ceiling column includes the oracle passes. `compare` against
  `e3-final`: b = c = 0, PASS, both presets; release binary unchanged.
  `ceiling host main` matches S0 except the split rows (findings, "Bench
  split, and no-candidate misses told apart").
- Wider crowded set (`wide`, its own seeds, −22…+5 dB, 10/20/30/40
  messages per slot) added to the bench and measured with `ceiling`, both
  presets and blocks; existing sets unchanged (`crowded` main b = c = 0).
  Host main: single 62.5%, oracle +1 81.8%, +2 86.2%, ceiling 89.3%
  (findings, "Wider crowded set, measured").
- Phase 4 plan and ADR 0024 amendment draft revised after review: no
  `start_pass` (a later pass is `start_slot`, `skip`, `feed` of the
  residual, which is always the full slot from sample 0); `subtract` a
  free function; `DecodeList::next_pass` only resets the order check;
  `decode_passes` runs every pass and overwrites the caller's buffer;
  the multi-pass report defined; the oracle named a same-configuration
  reference, not a bound; S2 (integer starts) before any sub-sample
  start, with residual suppression reported per condition and no
  pass/fail bar; the five decisions made (unit reference, fading/drift
  set, one total cost budget, the wider set as a precondition of S3,
  1..=3 passes with one later-pass candidate count); acceptance bars
  tightened. Stopped at the S0 checkpoint for approval.

## Session: 2026-10-28 (subtraction and further passes, S2)

- Plan approved by the user; S2 started (subtracting one signal,
  integer-sample starts). Pass 1 untouched.
- `decode::subtract` added to the core (`src/decode/subtract.rs`, with an
  in-crate `atan2_turns` in `math.rs`): fine time from adjacent symbols'
  phases summed by tone step in six groups (scored without the groups'
  phases, so drift and fading need only hold within ~2 s), frequency and
  drift from the groups' phases, least-squares amplitude per segment,
  linear interpolation, subtraction in place; `skip` region neither read
  nor written. `no_std`, `f32` and integer phase only, 6 836 B of stack
  arrays, no allocation. Unit tests: noiseless suppression below −40 dB
  (measured −96…−102 dB), a slot cut short, a skipped region stays zero,
  determinism with three golden pins, settings and inputs checked, the
  stack bound.
- Bench: a second (`f64`) generator for starts between samples, fades
  and drift (matches `Synth` to 6.6 × 10⁻⁴ at whole-sample starts); the
  `fading` set (10/20/40 messages, fades 0.05–1 Hz, drift ±0.2 Hz/s,
  fractional starts; own seeds; not in the default sets; `ceiling`
  accepts it); modes `suppression`, `passes`, `subcost`. Existing sets'
  records unchanged.
- First measurement showed drift above ~0.2 Hz/s broke the time search
  (one coherent sum over the transmission): fixed by scoring six groups
  without their phases and fitting drift through the groups. Cost 6.4 ms
  per call, then 3.3 ms after skipping the confirming timing pass for
  small moves and integer phase stepping (suppression unchanged).
- Results (findings, "Subtracting one signal (S2)"): noise-limited
  residue on single signals (−37.8 dB at +10 dB SNR) at every start
  offset and frequency; fades to 0.5 Hz followed; drift to 0.2 Hz/s
  fine, 0.5 Hz/s fails in ~10% of trials. Sub-sample timing residue
  ≤ −53.3 dB everywhere: **S1 not needed**. Further passes (bench loop):
  oracle-gain share +1 / +2 96.6% / 94.8% crowded, 93.0% / 89.5% wide,
  94.9% / 91.6% fading (main; fresh within a point); duplicates 1 in
  ~10 000 subtractions after one pass; no false decodes. Segment 3 840
  recovers more (99.0% / 98.2% crowded) with more duplicates; default
  kept at 1 920, S3 to sweep.
- `subtract_wav` example: decodes a WAV slot, subtracts every decode,
  writes the residual and one JSON line per decode with the estimates
  and `left_db`.
- Identity: golden pins pass; `run` of every default set, both presets,
  record-for-record equal to `e3-final` (b = c = 0, PASS). The release
  `cat-ft8-decode` does not link `subtract` but its bytes changed
  (codegen-unit partitioning; sha256 ce6a2549…fe8f); its output on 800
  seeded WAV slots, both configurations, `--raw` and `--profile` counts,
  is byte-identical to b3ead07b's.
- Sizes: host link-check image +6 792 B flash (90 296 B), `subtract`
  frame 7 432 B on thumbv8m; small and encoder images unchanged.
- Checks: fmt; clippy (workspace, cat-ft8 all features, thumbv8m);
  `embedded-check/check.sh` and `--stack`; release workspace tests and
  the `--ignored` bulk run; `--features profile`; `--no-default-features`;
  docs with `-D warnings` (default, no-std, profile); MSRV 1.75 library
  build. Stopped at Checkpoint S2.

## Session: 2026-10-28 (S2 fix round)

- Shortest segment 960; `valid` flags dropped (forward fill, leading
  segments back-filled); one set of step sums in the time search;
  `SUBTRACT_STACK_BYTES` 4 536 B, frame chain ~7.3 KB on thumbv8m.
- Drift weight |z|²; suppression and passes re-run (unchanged to 0.1 dB
  and a tenth of a point); pins re-pinned (findings, "S2 fix round").
- `subtract` reads input as the front end does (non-finite → 0, clamp
  ±2¹⁶); tests for a junk-filled skipped region, low tone 0, NaN/∞/huge
  samples, clipped input; `suppression` input conditions (cut, late).
- Bench: generator check fails above 1e-3; `drift=` validated; one GRID
  argument for `suppression` and `passes` sweeps.
- Docs: time-search stop rule, `left_db` limits, generator figure, cost
  text, `atan2_turns` source (checked against A&S p. 81), NOTICE line.
- Build-path check: same-path builds of 23d8336 and d4fd2d3 differ here
  (b3ead07b vs ce6a2549), as from a second path; WAV output identical.
- Pass 1 identical (release binary ce6a2549 unchanged, run records equal
  to e3-final). CI-equivalent checks pass.

## Session: 2026-10-28 (S3, further passes)

- API: `DecodeConfig::{passes, later_candidates, subtract}`,
  `candidate_capacity`; `Decoder::{decode_passes, decode_passes_staged,
  find_later_candidates}`; `DecodeList::{next_pass, len, is_empty}`;
  `PassReport`, `PassEvent`, `PassStep`. `decode` stays one pass.
- `cat-ft8-decode --config host|small[,passes=N]`; profile per-pass keys
  only with more than one pass. One code path (`decode_passes_staged`).
- Bench: multi-pass `run` records (per-pass counts), `compare` per-pass
  gates and the sweep interval gate, `slotcost`, `passes` on the
  library, `ceiling` oracle at the later count.
- Sweep (segments × later 20–200 × 2–3 passes) and fresh confirmation;
  chosen 3 passes / later 50 / 3 840 segments, adopted in `HOST`.
- Acceptance PASS on every set; cost worst 213.7 ms, mean 197.5 ms.
- Speedup kept: `Synth` envelope only in the end symbols, `i64`
  conversion (subtract −14%); dropped (no gain): residue-statistics skip,
  closed-form `Correction::skip`, constant middle increment.
- Pins: four multi-pass golden hashes; threaded split = `decode_passes`.
- Pass 1 identical: run records = e3-final (host,passes=1 and small);
  WAV output of the new binary with passes=1 = d4fd2d3's.

## Session: 2026-10-28 (S3 fix round)

- 00c7cbe: `Synth` phase increment through `i32` (no `__aeabi_f2lz` per
  sample on Cortex-M; call sites back to d4fd2d3's). x86 `subtract` about
  6% slower; worst 40-signal slot 224.7 ms (budget 250).
- 0d3c912: `PassesReport` (whole-slot totals, passes run, why they
  ended); `decode_further_passes` (pass 1 streamed, i16 identity tested);
  `ControlFlow` stop and `max_subtractions`; stop on a full list,
  `max_decodes()`; `candidate_capacity()` = max; one counts struct; only
  successful subtractions counted; `"pass"` on decode lines; profile
  timings in arrays; `Subtraction` non-exhaustive; `MAX_SEGMENTS`
  `div_ceil`; tests for the review's gaps. Multi-pass pins re-pinned
  (report only; old pins reproduced from the pass reports).
- 3ed1aa0: NOTICE subtraction note at crate level; planning wording.
- Identity: `run host` every set, both blocks, at clean 3ed1aa0 = S3's
  records (reports = per-pass sums); pass 1 = e3-final; compare PASS.
  Decode command on the 800 WAV slots = cea0bc2's but for `"pass"` and
  the summed profile counts.
- Release `cat-ft8-decode` built from this repository's path at the
  final commit of this round: sha256
  e762e43f3d4abd0f26c3cc89ca6ec73a0c9ae222b12f9bf8db660bc30e80ca6a.
- CI-equivalent checks pass before each code commit (fmt, clippy
  workspace and all features, workspace and release tests, thumbv8m
  build and clippy, `check.sh` and `--stack`, MSRV 1.75, profile and
  no-std tests, docs, bulk).

## Error Log
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-10-25 | Stage timings ~6 µs each | 1 | `Instant::now()` overhead on this host; time each stage as one batch |

## Session: 2026-10-28 (O1, decode time)

- 4752586: subtraction behind the amplitude fit (reference pairs held,
  one generator run fewer); 46 KB more stack in `subtract`.
- 5f53377: FFT stages by radix (const generic), spectrum levels in
  chunks of 64 (vectorised loop); feed −65% (x86 and Cortex-A76).
- c554d6a: `Synth` increment through `i64` on 64-bit (x86 −5%, A76 none).
- 4fe722c: sum-product round in phases, not inlined (BP −18% / −15%).
- 40-signal slot, three passes, worst / mean: x86 221.0 / 203.8 →
  142.1 / 132.5 ms; Raspberry Pi 5 395.8 / 367.2 → 262.2 / 241.3 ms.
- Identity after every step: pins, 960-case subtraction grid, run
  records of host / host,passes=1 / small = s3b (0 differing); final
  decode binary's 800-WAV output = 3ed1aa0's. Rejected variants with
  numbers in findings ("Decode time, optimisation round").

## Session: 2026-10-29 (O1 fix round)

- 59bb2c7: `SubtractScratch` / `PassesScratch` from the caller;
  `SUBTRACT_STACK_BYTES` 50 616 -> 4 536 B (thumbv8m `subtract` frame
  50 632 -> 4 560 B); docs on the multi-pass calls; ADR figures;
  subtraction anchors recorded with 36cc440 (4 080, 7 680, whole
  waveform, skip 30 000, cut 90 000, drift on/off).
- 2dd54dc: FFT finite-input precondition and wider reference test;
  `Synth` increment `debug_assert!` with the margin tested (2³¹ − 256).
- c3f080f: the second reference run (segments beyond two symbols) a
  block at a time through the scratch; it was about 12% slower than
  36cc440 since O1, now about 7–9% faster (x86 2.79–2.86 against
  3.02–3.11 ms per call; Pi 5.74 against 6.00).
- Identity at clean c3f080f: run records = s3b (host 38 900, passes=1
  and small 34 800 each, 0 differing); 1 080-case subtraction grid =
  36cc440; 800-WAV output = 11a9221's and 3ed1aa0's.
- Raspberry Pi 5: release test suite passes; `run host` / `small`
  (sweep and crowded, main) records = x86's.
- Slot cost kept: 40-signal slots, three passes, worst / mean x86
  143.8 / 132.4 ms (O1 142.4 / 132.3), Pi 262.2 / 241.3 (O1 262.3 /
  241.3).
- CI-equivalent checks before each code commit (fmt, clippy workspace
  and all features and thumbv8m, workspace tests, thumbv8m build,
  `check.sh` and `--stack`, MSRV 1.75, profile and no-std tests, docs,
  bulk).
- Release `cat-ft8-decode` built from this repository's path at the
  final commit of this round (the planning commit changes no input of
  the build; same bytes as at c3f080f): sha256
  56f3786f1744977263f771c7e1178d7b9bd828ed28bbdbcb1b6018fa502704e2.

## Session: 2026-10-29 (O2, decode time)

- Profile: `perf` on x86-64 (three passes and `subcost`), stage timers on
  the Cortex-A76. Targets: the reference generator's per-sample
  branches, saturating `f32` → integer conversions done a lane at a time
  on x86-64 (SSE2), the sync search's per-point lookups.
- 08d5d8e: `fill_phase` / `skip_with` form increments in runs per symbol
  part (vectorised), then add them in order. Subtract per call x86
  2.05–2.09 → 1.92–1.95 ms, A76 4.263 → 3.646.
- fbc5857: `exp2_neg` floor via 1.5·2²³ on x86-64 only. BP per slot
  32.0–32.3 → 25.8–26.0 ms (x86); A76 unchanged (kept its conversion).
- 27bb0d5: sync scores a start row at a time. Sync per slot 8.53 → 3.55
  ms (x86), 18.4 → 4.67 (A76).
- d6bab4f: generator increments via the exact floor on x86-64. Subtract
  per call 1.92–1.95 → 1.85–1.89 ms; A76 unchanged.
- Rejected: envelope-multiply skip with an `f32` offset counter (x86
  +13%, A76 +6%); step 2's floor on AArch64 (no gain); `as i64` in the
  lane loop (no gain).
- b39ce66: `check.sh` static RAM ceilings per image (tested failing at a
  lowered limit); `peak_bytes`, `PassesScratch`, `Removal::refs` and
  frame-figure docs. `PassesScratch` shape left as an open API question
  (task_plan.md).
- 40-signal slot, three passes, worst / mean, base → final, same
  session: x86 144.3 / 133.6, 145.1 / 132.7 → 123.8 / 114.6, 126.1 /
  114.5 ms; Raspberry Pi 5 262.0 / 241.1, 261.9 / 241.0 → 223.6 / 206.1
  (both rounds).
- Identity at each step's clean commit: run records = S3 fix round's
  (host 38 900, passes=1 and small 34 800 each, 0 differing); final
  decode command's 800-WAV output byte-identical to b465b75's.
- Raspberry Pi 5 at b39ce66: release test suite passes (ignored tests
  included); `run host` / `small` (sweep and crowded, main) records =
  x86's (under-voltage flag seen during that all-core run; records
  equal).
- Sizes (thumbv8m): small 85 680 → 86 492 B, host 94 060 → 94 896,
  encoder-only 36 116 → 36 548; static RAM and the subtraction frames
  unchanged; soft-float helper call sites unchanged (thumbv8m and
  thumbv7em).
- Stale Pi binaries built from extracted source archives into one target
  directory (archive mtimes older than the last build) found by hash; those runs discarded, every timed
  binary rebuilt in its own target directory.
- CI-equivalent checks before each code commit (fmt, clippy workspace,
  all features and thumbv8m, workspace and release tests, thumbv8m
  build, `check.sh` and `--stack`, MSRV 1.75, profile and no-std tests,
  docs, bulk).
- Release `cat-ft8-decode` built from this repository's path at b39ce66
  (the planning commit changes no input of the build): sha256
  2e689a03bddd57840dbb027176ee683fcc791a425b58f2b828625645bb550059.

## Session: 2026-10-29 (O2 fix round)

- 0cd4248: `fill`/`mix`/`skip` through `fill_phase`/`skip_with` over a
  64-sample stack chunk; per-sample `run` deleted (kept in tests as the
  reference); one `increment` helper, one cfg site; de Bruijn B(8,3)
  windows in the generator tests; `increment` tested against `as i64`.
  Encoder x86 `synthesize` 409–411 → 368–373 µs per waveform, `fill`
  609–611 → 565–570, `skip` 233–235 → 217–220. Flash −1.0 to −1.2 KB per
  image (thumbv8m and thumbv7em); `fill`/`mix` frames 160 → 584 B.
  1 000 encoder cases hashed: identical to 0186703.
- 7694d25: `costas_score` doc in place; `present` counted in the main
  loop; `floor_small` domain doc and `debug_assert!`.
- 69c5bd6: `check.sh` ceilings in one table, one awk; CI runs the
  `_everywhere` tests in release on x86-64.
- An apparent 0.8% decode slowdown came from building the base at
  another path (code placement); rebuilt at the same path, equal.
  A revert of the `present` change made on that evidence was undone.
- Identity at 69c5bd6: tests (debug, no-std, release, ignored in
  release), clippy four targets and workspace, `check.sh --stack`, MSRV;
  run records = S3 fix round's (0 differing); 800-WAV output = b465b75's;
  Raspberry Pi 5 release test suite with ignored tests passes (0x50000
  before and after).
- Release `cat-ft8-decode` built from this repository's path at 69c5bd6:
  sha256 96339c7a67b12366b6a864a0860978b813b8004f3a89360ff3e4f0d72a3fb9a9.
- Not done: Cortex-M run time (microcontroller unavailable; open item).

## Session: 2026-10-29 (O3, CPU and memory per decoded signal)

- Profile at cc68e22: three-pass 40-signal slot x86-64 / A76 subtraction
  65.5 / 126.4 ms, belief propagation 26.1 / 41.5, feed 15.5 / 31.3;
  `perf` (x86-64) sum-product 25%, fit loop 19%, FFT 11%, removal 11%,
  increments 11%, correlations 8%, `score` 2.1%. Static RAM small 212 280
  B (spectrogram 161 920), host 287 928 B (subtraction scratch 46 080).
- 93fa40d: the time search's 15 rotations once per trial. Subtract per
  call A76 3.661 → 3.577 ms; slot A76 206.4 → 203.5 mean, x86-64 116.0–117.1
  → 114.0–114.6.
- 81db219: the multi-pass subtractions hold their reference in the
  spectrogram's idle buffer; `subtract` takes `&mut [u8]` (any idle
  buffer, ≥ 1 920 B, 46 080 B for one run); `PassesScratch` and
  `SubtractScratch` removed; spectrogram empty after a stop following a
  subtraction (tested). Host static RAM 287 928 → 241 848 B, every
  image's ceiling 256 KiB; working memory per stream −46 080 B. Slot A76
  mean 203.5 → 202.6–202.7; x86-64 114.0–114.6 → 116.6–116.8 (the write
  loop's codegen; alternatives measured and rejected). CLI max RSS
  3 860–3 908 → 3 752–3 784 KB.
- Rejected: belief propagation over the 522 edges (x86-64 BP −14%, A76
  +2.3%; padded and row-of-7 forms no better on the A76); shared
  cosine/sine quadrant decode (slower on both); the hold written in ring
  segments (x86-64 faster, A76 +1.4%) or through a chained iterator
  (slower on both); the CLI freeing its 16-bit samples early (max RSS
  −56 KB only).
- Proposal: `i16` residual (prototyped, not committed): `run host` every set
  PASS, 22 messages gained and 9 lost of the crowded, fading and wide
  sets, no false decodes; 800 WAV slots 10 696 decodes either way.
- Identity at 93fa40d and 81db219: run records equal to the S3 fix
  round's (host 38 900, passes=1 and small 34 800 each, 0 differing);
  800-WAV output of the release decode command at 81db219 byte-identical
  to b465b75's.
- Raspberry Pi 5 at 81db219: release test suite with ignored tests (lib
  153, then 2 + 12 + 1 bulk + 7 + 2 + 5) passes; 0x50000 before and after.
- Release `cat-ft8-decode` built from this repository's path at 81db219
  (the planning commit changes no input of the build): sha256
  52c04a9e2098cce5034a62129d1d2a4ab9b5686e7de3e597fc3db69373522fc1.

## Session: 2026-10-29 (O3 fix round)

- 37aa031: `further_passes` calls `Decoder::start_slot` once before a
  pass's subtractions and takes the level buffer once; a stopped decode
  leaves no stale candidates (test: `candidates()` empty,
  `decode_candidate(0)` `None`; failed before the change).
  `PassesReport` unchanged.
- 13f88ed: narrow-band decoder test (1000–1500 Hz, exact vs padded level
  buffer, whole and skip/cut); scratch-size test over the held-and-rerun
  test's starts, skips, cuts and segments.
- b37df17: RAM ceiling 256 KiB less a 16 KiB stack budget in `check.sh`
  (fails at a 20 KiB budget); `SUBTRACT_STACK_BYTES` 4 536 → 4 656 (the
  rotations); ADR 0024 `subtract` frame 4 600 B; rustdoc shows the
  `SUBTRACT_*` names; narrow-band threshold (235 base bins) and padding
  documented; config checked before scratch size documented.
- Rejected: `held` sliced to the ring once (A76 slot 203.4 → 203.2,
  subcost unchanged; x86-64 subcost −0.9%, within the spread).
- A76 profile (out-of-tree sampler, three-pass slot): subtraction 56.7%
  (fit loop 25.5%, removal 13.3%, time search 13.4% with its generator,
  fit-run generator 4.5%, frequency estimate ~0), belief propagation
  24.7%, feed 14.5%, sync 2.4%.
- BP repeats on identical LLRs in passes 2–3: 463 of 20 000 candidates,
  13 871 of 521 581 rounds (1.4% of all rounds, about 0.6 ms of 203 on the
  A76): not worth an exact skip.
- O3 findings wording: hold at the end of the buffer if row skipping is
  pursued; 256 KiB is the link-check image (real M33 stream ~956 KB);
  "prototyped, not committed".
- Identity at 37aa031 and b37df17: run records equal to the S3 fix
  round's (0 differing); 800-WAV output equal to b465b75's. Local CI
  checks, release and exhaustive tests, clippy on four targets pass;
  Raspberry Pi 5 release suite with ignored tests (lib 154, then 2 + 12 +
  1 bulk + 7 + 2 + 5) passes; 0x50000 before and after.
- Release `cat-ft8-decode` built from this repository's path at b37df17
  (the planning commit changes no input of the build): sha256
  411b9c38eac68c16dceea39e0b5f48eaeba45db6944bf023f39b07d98ea4328c.


## Session: 2026-10-30 (O4, CPU per decoded signal)

- 872bd59: the time search's symbol middles skip the envelope (exactly
  1.0 there; compile-time assert, debug assert). `subcost` A76
  3.570–3.583 → 3.540–3.541 ms; x86-64 unchanged; slot A76 −0.4%.
- 657543e: sum-product round over the 522 edges, grouped by degree (59
  checks of 6, then 24 of 7), fixed-size products; check order of
  messages and sums kept; no new table. BP per slot A76 41.4–41.6 →
  40.3 ms, x86-64 25.8 → 21.4–21.5; round frame 2 744 → 2 176 B; flash
  +212 B. Slot gain on the A76 −0.5% (under the 2% rule; kept for the
  stage gain, x86-64 −3.2% and the frame; own commit).
- 66ba24b: `Decoder::idle_levels` (one reset before the subtractions).
- Slot, base → 66ba24b, two alternated rounds: A76 220.2–220.4 /
  203.3–203.5 → 218.0 / 201.2 ms (single pass 32.8–32.9 → 32.2); x86-64
  123.6–128.1 / 114.1–114.4 → 119.1–120.5 / 110.3 (single pass 21.0 →
  18.9–19.0).
- Rejected with numbers (findings): joint `cos_sin_turns` (A76 `subcost`
  +14.6%; proved equal at all 2³² phases on x86-64 first), `input()`
  fast path (+0.4% `subcost`, +1.3% subtraction in a slot), exact `f32`
  counter in the removal (+5.8%).
- A76 profile at 66ba24b (out-of-tree sampler): subtraction 56.9% (fit
  48.7 ms, removal 30.4, time search 26.3 with its generator), BP 24.3%,
  feed 14.6%.
- Identity at each code commit: run records = S3 fix round's (0
  differing); 800-WAV output = b465b75's. Local CI checks at 66ba24b;
  `check.sh --stack` per step (static RAM unchanged: 212 280 / 241 848 /
  0 B). Raspberry Pi 5 release suite with ignored tests passes at
  66ba24b (0x50000 before and after).
- Release `cat-ft8-decode` built from this repository's path at 66ba24b
  (the planning commit changes no input of the build): sha256
  1f68772b3e24ba15773b0a8bd8714440d18541b928a5726f88fad242787cd1bc.

## Session: 2026-10-30 (O5, CPU per decoded signal, last bit-identical round)

- f0bb4ab: the decoder's subtractions skip the report's sums
  (`subtract_unreported`; flag in `Removal`). Subtraction per slot A76
  122.5 → 107.5 ms, x86-64 64.5–65.3 → 58.6–58.8; slot A76 −7.0%,
  x86-64 −5.8% (paired base 109.7–110.9 → 103.7–104.2 ms). Host flash
  +220 B. Rejected first form: `const REPORT` on `run` (A76 −0.7%, x86-64 +4%: `blocks` out of line). The fit's
  `xx`/`n` are already dead in the AArch64 loop (only where the fit
  closure is inlined into `run`; corrected in O6).
- 02ff55e: BP messages kept in the degree-grouped edge layout, round
  in place. Time unchanged (A76 slot +0.3%, BP 40.9 → 40.7 ms); kept
  for memory: `BpScratch` 3 892 → 3 656 B, `CandidateScratch` 5 100 →
  4 864 B, static RAM −236 B (host headroom 3 912 → 4 148 B), round
  frame 2 176 → 88 B. Pins unchanged; `--profile` memory keys −236.
- 92970ba: edge-layout test and the first-form round compared bit for
  bit over 20 000 random states; two layout mutants caught by both.
- 1b47a55: `CORE_START`/`CORE_END`, ramp assert at both ends,
  waveform-length assert and generator-start debug assert.
- a5a2034: ADR 0024 flash figures (85 508 / 94 100 / 35 536 B); O4
  wording.
- Slot, base → 1b47a55, two alternated rounds: A76 201.1–201.2 → 187.5
  ms; x86-64 110.2–110.3 → 103.9–104.5.
- M33 (static): 657543e cuts tanh/atanh evaluations and `vdiv`s by 21%
  and the round's instructions by about 15%; 02ff55e a further 8.3%
  (dynamic estimate, against 657543e).
- A76 profile at 1b47a55: subtraction 53.7% (fit 51.8 ms, time search
  26.2 with its generator, removal 14.7), BP 26.2%, feed 15.6%.
- Out of tree: per pass (A76, 2 000 crowded/wide/fading slots) 34.5 /
  72.5 / 37.6 ms for 15.52 / 4.70 / 1.00 decodes per slot (pass 3:
  40.7 ms and 1.08 per slot where it ran; corrected in O6); later-pass decodes
  within 50 Hz of the previous pass's: 99.8% / 99.7%; time-search
  repeat in 56 of 40 435 subtractions.
- Identity at each code commit: run records = S3 fix round's (0
  differing); 800-WAV output = b465b75's (from 02ff55e except the two
  memory keys, −236). Local CI checks at 1b47a55; `check.sh --stack`
  per step. Raspberry Pi 5 release suite with ignored tests passes at
  1b47a55 (lib 157; 0x50000 before and after).
- Release `cat-ft8-decode` built from this repository's path at 1b47a55
  (later commits change no input of the build): sha256
  36ce320895815758c0b978bd563eaefbef4e0eeefaf9d400366b01126c6c3987.

## Session: 2026-10-30 (O6, CPU per decoded signal)

- 1d028a1: the fit's sums (`Fit`, five fields), correction and ring
  size in locals per block. A76 and x86-64 unchanged (the timed build
  already inlined them; an earlier listing was built with one codegen unit);
  kept for the Cortex-M33 image, where `Sums::add` was a call per
  sample (now inlined; `subtract` frame 4 600 → 4 584 B, host flash
  +108 B).
- Rejected: sine table padded by its end point (bit-identical at all
  2³² phases on both hosts) — A76 subtraction +2.7%, slot +1.3%
  (NEON pairing of the two lookups).
- 73ebf5d: `run_as` and `decode_passes` doc fixes; O5 notes corrected
  (pass 3 is 37.6 ms / 1.00 decodes per slot, 26%; M33 and x86-64 step
  bases; the `xx`/`n` claim).
- Identity at 73ebf5d: run records = S3 fix round's (0 differing);
  800-WAV output = O5's (1b47a55). Local CI including `--ignored`,
  `check.sh --stack`, MSRV 1.75, docs: pass.
- Out of tree: the later-pass cap (50) binds in every search (≈5 000
  sync points); a 50 Hz gate without refill saves nothing and loses 39
  decodes; pass-3 trigger k = 2 / 3 saves 4.6 / 8.7 ms (A76) for 27 /
  85 decodes; cap 20 saves 15.0 ms (−10.4%) for 3 of 42 433.
- Release `cat-ft8-decode` built from this repository's path at
  73ebf5d: sha256
  dea2368c4f18fd1bf89eed7ef841dad96ae04fe2bb0894b5eaf82954c0774377.

## Session: 2026-10-30 (O7, release profile and the later-pass cap)

- Release profile: default, `codegen-units = 1`, fat LTO and both,
  each bit-identical (run records and 800-WAV output). A76 three-pass
  slot 187.6 / 187.6 / 185.7 / 187.8 ms, x86-64 104.0 / ~103 / ~105 /
  ~103: nothing wins by 2%, so the workspace keeps cargo's defaults
  (fat LTO more than doubles the decoder's rebuild). Embedded image
  unaffected (its own workspace and profile).
- 5704d32: `HOST` doc (50 later candidates decode all but 4 of 42 437
  that 200 do; to re-measure with the count) and the fit-locals comment
  (the reason, not the codegen outcome).
- a437134: `projection_pinned` (the report's projection on random sums;
  the cross-term re-association no subtraction pin saw now fails it).
- O5/O6 notes: wording of their sources; x86-64 20.9;
  measurement commit 23742aa; ~41% at 25 Hz; sync-search cost backed;
  O6 profile table and Pi suite filled in; `SMALL` and memory notes on
  the cap.
- Out of tree: rank histogram, S3 reconciliation (its −21 at 20 was the
  two-pass column; three passes lose 3, as O6), caps 20/25/30 and the
  proportional rule on the main block and two new blocks (`cap1`,
  `cap2`). Cap 25 recommended: −8.8% A76 on the main block (8.7% over
  the three) for 10 of 127 336; no
  proportional setting beats it by 2% at equal loss.
- Local CI at a437134 (fmt, clippy incl. thumbv8m, tests incl.
  `--include-ignored`, MSRV 1.75, docs, `check.sh --stack`): pass.
  Raspberry Pi 5 release suite with the ignored tests at a437134: pass
  (lib 158; 0x50000 before and after). 800-WAV output of the shipped
  CLI = O5's.
- Release `cat-ft8-decode` built from this repository's path at
  a437134 (default profile): sha256
  9bf5630b4cda44c2f09e829c37e50a46b10cb33e68f942d5d6498c49e4bfa7df.

## Session: 2026-10-30 (O8, later-pass cap 50 → 25, gated)

- a4fe8e6: O7 notes corrected (one-pass cu1 32.5–32.6; x86-64 fat LTO
  +1.25%; deepest rank at 30 signals 36; 25 halves the loss on the
  fresh blocks; −8.8% is the main block, 8.7% over three); sources
  named plainly in the planning files; O8 plan and gate criteria,
  committed before the results.
- 1c1c8a8: `fit_conditions_at_their_boundaries` (the determinant at
  exactly `(0.1·e)·e`, the next `f32` up, energy at segment/8); the
  `>=`, `0.1·(e·e)` and energy `>` mutants each fail it. Test only.
- Candidate 0f0341c (`HOST.later_candidates` 25, doc, `max_decodes`
  150, the adopted-`HOST` multi-pass pin; decodes, texts and residuals
  of the pinned slots equal to 50's): **not adopted**. On `fresh`: 3
  lost (bound 8), 0 gained, 0 false; one lost message ranked 4 at cap
  50 (a knock-on of a rank-35 pass-2 decode moving to pass 3), which
  fails "each lost ranked beyond 25". Pass 1 identical (34 800 + 34 800
  records; pass 1 of 38 900); paired gates PASS on every set; ceiling
  fading 2 / 12 140, crowded and wide 0; `wide` below −16 dB 0 lost;
  800 WAV decode lines identical (621 profile lines differ in passes
  2–3 work only). A76 three-pass slot 144.26 → 131.71 ms (−8.70%);
  `slotcost` 187.0 → 173.9 ms (−7.0%). Branch `gate-cap25` kept,
  local only.
- Local CI at 1c1c8a8 (fmt, clippy incl. thumbv8m, workspace tests,
  profile and no-std tests, release tests with `--include-ignored`
  (lib 159), bulk, MSRV 1.75, docs, `check.sh --stack`: flash 85 508 /
  94 208 / 35 536 B, static RAM 212 044 / 241 612 / 0 B, unchanged).
- Release `cat-ft8-decode` built from this repository's path at
  1c1c8a8: bit-identical to O7's (sha256
  9bf5630b4cda44c2f09e829c37e50a46b10cb33e68f942d5d6498c49e4bfa7df).

## Session: 2026-10-30 (O8b, later-pass cap re-gate on a new block)

- a616780: re-gate criteria (block `accept2` checked unused; loss kinds
  (a) ranking and (b) knock-on; lost ≤ 8, pooled ≤ 13; ceiling and
  `wide` low-SNR rates ≤ O7's), with the classifier's check on `fresh`
  (2 (a), 1 (b)), committed before the run.
- One run of `accept2` (ce09c5d vs 0f0341c, ceiling at both counts):
  **fail** on the ceiling's `fading` rate (5 of 12 168 against O7's
  9 of 35 888); every other criterion passes (lost 5, pooled 8, all
  explained; 0 false; 0 gained; pass 1 identical; `wide` < −16 dB 0
  of 1 814). The cap is dropped; 0f0341c stays on `gate-cap25`.
- O7's 10 losses reclassified: all ranking losses (a), no knock-on.

## Session: 2026-10-30 (O9, vector sine and cosine in the subtraction; branch `o9-vsin`)

- 853961f: O9 plan (task_plan.md), committed before the code.
- a5c4a6c: `Correction::skip` in closed form; identical results (all
  pins hold). A76 −0.5%, x86-64 +2% (placement; kept, steps 2–3 use it).
- a1119ed: the fit's and the removal's reference by polynomial
  (`cos_sin_turns`, degree 7/6 then), split into vectorising loops
  (correction in four `u64` lanes in place, reference map, fit sums in
  four lanes keyed by j % 4, fixed order); ring a multiple of 4 pairs.
  Pins re-pinned. A76 slot −12.4%.
- ee7c0ce: the time search's correlations the same way (products in the
  scratch, lane sums). Pins re-pinned. A76 −4.6% more.
- 0c6f3bf: ADR 0024 note (degree text updated in d4e669e).
- d4e669e: degree 5/4 (error 1.0e-5; −100 dB left of a clean signal
  against the table's −103; 2.8% faster on the A76 than 7/6). Pins
  re-pinned. Release binaries byte-identical to the measurement branch
  `o9-d54` (ec72aa0), whose measurements are therefore d4e669e's.
- Slot (A76, `slotcost`, two alternated rounds): 187.0 → 151.1 ms
  (−19.2%), subtraction 107.5 → 72.1; x86-64 104.8–105.0 → 85.0
  (−19.0%). Host flash +3 740 B, `subtract` frame +272 B; small and
  encoder images unchanged; no FMA, no `f64` helper.
- M33 (static): fit ≈ 127 instructions per sample against 110 (lane
  loops not unrolled at `opt-level = "s"`); time search equal; one path
  kept (a Cortex-M table under `cfg` would break cross-target identity).
- Effect (main, cap1, cap2; 127 336 messages): degree 7/6 0 lost 0
  gained; degree 5/4 1 lost (marginal: 29 BP rounds at bafed37) 0
  gained; 0 false; pass 1 identical; `compare` PASS everywhere. 800 WAV:
  same messages in the same passes in every run; 605 lines differ in
  later-pass parameters and profile counters. Encoder output identical.
- Proposed gate in findings.md (block `accept3`, unused); **acceptance
  not run**; proposed, not yet run.
- Gate fixed (findings.md, "Gate"), before any `accept3` record:
  candidates 44d01ba (degree 5/4; release binaries byte-identical to
  d4e669e's) and ee7c0ce (degree 7/6, fallback) against bafed37, one
  attempt; classifier pinned by SHA-256 and checked on main/cap1/cap2
  (the one loss: marginal). `accept3` checked unused on the host and
  the Pi the same day.
- Acceptance on `accept3`: **44d01ba passes every criterion** and
  merges (fallback not needed). 0 lost, 1 gained (k), 1 moved (m), 0
  false; pass 1 and the ceiling identical; 800 WAV slots the same
  messages in the same passes; A76 `slotcost` −19.2%, A76 records
  −14.8%, x86-64 −19.1%; A76 records equal x86-64's; A76 release suite
  passes. (ee7c0ce would also have passed: nothing changed, −16.9% /
  −12.9%.)
- Merged: feat/cat-ft8 fast-forwarded to 55bbbff (ADR note on the M33
  static cost). Release binaries = 44d01ba's on both targets; local CI
  and the Pi release suite pass. A76 profile: subtraction 44.1% (fit
  18.5%, time search 11.1%, removal 8.6%, fit-path generator 5.8%), BP
  31.7%, feed 18.9%.

## Session: 2026-10-30 (O10, the spectrogram transform; branch `o10-fft`)

- 6ed0020: O10 plan (task_plan.md), committed before the code.
- Measured first (out-of-tree microbenchmark of one row, both hosts;
  AArch64 and x86-64 code of the stages): A76 row 53.2 µs, of it the
  transform 39.7 (stages 2–6 scalar, stages 2 and 3 reloading spilled
  twiddles per butterfly), split and powers 6.6, `log2` and level 2.4.
- 4089b5b: butterflies side by side (four lanes on SSE2/NEON, one
  elsewhere), real/imaginary arrays, split and powers as vectorising
  loops. Bit-identical by construction and by test (transform at 1, 2, 4
  lanes against the reference; powers against the form as first
  written, every bin). Row: A76 53.2 → 37.3 µs, x86-64 26.2 → 20.9.
  M33 (emulated, small image): instructions per row −12.4%, flash
  −132 B, stack unchanged.
- e641a61: `Correction::skip` exact for any count (no bound, no
  assertion; test to `u64::MAX`).
- 5cb0e43: ceiling elapsed time to stderr (stdout loses only that).
- 7f89879: coefficient provenance (LP grid fit, not Remez; S5 not
  f32-tuned) and the re-pin note.
- Slot: A76 three passes 151.1 → 142.0 ms (−6.0%), one pass 32.3 →
  29.3 (−9.3%), feed 31.2 → 22.0; x86-64 85.5 → 82.1 (−4.0%). A76
  profile: `power_bins` 17.7% → 13.0% of the slot.
- Identity: 63 000 records equal on x86-64 and on the A76 (against
  x86-64's ddfd48d); 800 WAV slots byte-identical; ceiling stdout equal
  but for the moved figure; encoder outputs identical. Local CI, the
  embedded checks (no FMA, no `f64` helper) and the Pi release suite
  pass.
- Proposed, not run: the gated `i16` residual round (findings).
- O10b (review follow-ups, output-identical):
  - 78d4f2f: stage size a const generic; `Lanes::<L>::OK` from
    `STAGES` (`fft::<8>` fails to build).
  - 47739f1: `bin_powers` one form (M33 −0.6% instructions, flash
    +344 B).
  - 4af3304: reflow.
  - The emulated small image's output is byte-identical to ddfd48d's
    on seven slots.
  - A76 timing with live throttle bits sampled once a second: none set;
    151.1 → 142.8 ms; 7f89879 142.0.
  - Records, WAV output and encoder identical; local CI and the Pi suite
    pass.

## Session: 2026-10-30 (O12, belief propagation; branch `o12-bp`)

- ecfabf2: O12 plan with the stop rule, committed before any number.
- Measured first: A76 round 36.7 cycles and 58.1 instructions per edge,
  backend-bound (no cache or branch misses); the tanh loop computed its
  `exp2_neg` and division twice (for x and −x); floor 15.4 cycles per
  edge (2.4×), so the stop rule did not apply. M33: the small image runs
  min-sum (31% of its one-pass slot), the host image sum-product (46%).
- a04fc5a + 6fd70e8: |x| by the sign bit where the loops vectorise, the
  select elsewhere (M33 code unchanged). A76 BP −11.5%.
- 3120384: tanh and atanh 24 values at a time (one on the M33). A76
  slot 138.7 → 126.6 ms.
- 7a5898d, d1a9334, 26bb83a: min-sum in place, totals swapped by
  reference, stopping test per check. M33 small slot −6.8%, −1.1%,
  −2.0%.
- d2667f1: one exit in `decode_soft`; flash and frame below 593b7c1's.
- 636350d: the min-sum check's size as a type where the loops
  vectorise (7a5898d had made the small configuration's BP on the A76
  +29%); M33 code unchanged. A76 small slot 18.0 → 15.5 ms (593b7c1
  16.1).
- Result: A76 142.75 → 126.25 ms (−11.6%), one pass −26.7%; x86-64
  −5%; M33 small slot −9.6% (seven WAV slots −8.1%), host −2.8%.
- Rejected with numbers: inputs and sums by degree (A76 −1.8%, M33
  +0.45%, flash +156 B), min-sum by degree, bounded min-sum loops,
  branch-free minima, fused min-sum loops (M33 −2.6%, x86-64 min-sum
  +31%), 8/16/32 lanes, the min-sum check by degree on the M33 too.
- Identity: records (73 800 on x86-64 at every commit, the A76's equal
  to x86-64's), 800 WAV slots × 3 configurations × 3 modes, 45 encoder
  outputs, the emulated small image's output on seven slots. Local CI
  and the embedded checks pass.

## Session: 2026-10-31 (O12b review follow-ups and O13 measurement; branch `o12-bp`)

- bc54e8e: O12b and O13 plan, the candidate's rule fixed before any
  number (A76 three-pass ≥ 2% or M33 small one-pass ≥ 3%; flash up to
  +2 KB accepted).
- 76151f8: `VECTOR` by `target_arch`. 676d274: the min-sum check over
  plain slices (M33 code identical). 49db9e3: the scalar forms under test
  on hosts (const parameters). c6e0e4e: every check as the first
  failure. 627d356: `decode_soft` against its first form at every round
  limit. ee047b8: clippy (a type alias). 8b9a50d: O12 findings
  corrected.
- Mutants that survived O12's suite and are now killed: the round limit
  before the checks, the last check skipped, the select's negative
  branch, the scalar min-sum arm.
- Variable-side sums (and flat inputs): A76 −1.21% / −1.94%, M33 small
  −1.62%; rejected under the fixed bar. Belief propagation is done for
  bit-identical work.
- Identity at 627d356 (ee047b8's x86-64 and AArch64 binaries are
  byte-identical to it): records x86-64 and A76, 800 WAV slots, encoder,
  the emulated small and host images. Local CI and the Pi suite pass.
- O13 measured: the subtraction 72.1 ms of the 125.8 ms three-pass slot,
  32.0 cycles a sample, 2.0× an issue floor; reference 13.3 ms
  (latency-bound), removal 16.9, time search 18.2, fit 6.7, generator
  8.7, correction 7.4. Three ranked candidates and a stop rule recorded;
  none implemented.

## Session: 2026-10-31 (OSD sizing; branch `osd-size`, measurement only)

- b8ff866: the round's questions, definitions and decision metric fixed
  before any number.
- 5ec0eed, 4657bfe: `profile`-only accessors (the default binaries
  byte-identical to 511bf32's after moving them to the files' ends).
- af23f41, 3f48a52, 48dd456, 2cbb245: `paired_bench osd`, `osdcost`,
  `osdplaus`, `osdcheck`, `osddump` and the measurement-grade OSD.
- Measured: pool, recovery, false decodes, cost (A76, M33), on
  main/cap1/cap2 and E4's empty blocks. All failed candidates: neither
  replace nor supplement. Each pass's top 10, OSD-1: supplement, 4.3 ms
  per new decode, 0.104 per slot. Gated round drafted, not run.

## Session: 2026-10-31 (lean OSD-1 and calibration; branch `osd1` from c3b7dd7)

- 043ee8a: `DecodeConfig::osd` (off by default) and `decode::osd`, the
  lean OSD-1 form A; storage shared with belief propagation's through a
  safe enum (+4 B static RAM on the M33); differential test against the
  sizing OSD. e7e7f48: `paired_bench osdlib`, `osdcal`, `osd=C:T`; the
  sizing OSD's renames and notes.
- Option off: records and the 800 WAV slots' decodes identical to
  511bf32's (profile metadata only: scratch +8 B, table bytes +2 184).
- Lean cost: A76 23.2 µs per candidate (sizing 31.0), M33 0.204 M
  instructions (0.352 M), flash +7.4 KB per image.
- 69fcfad: calibration fixed before running; run on qv0000–qv2999
  (6.9 M empty slots): T = 21 for host and small.
- Gated round revised (task plan); proposed, not run.


## Session: 2026-10-31 (OSD-1 fix round; branch `osd1`)

- 923c393: `unpack` behind a generic shim over one `unpack_dyn`: flash
  small 93 888 → 90 540, host 105 588 → 102 232, encoder alone +148 B;
  `osd_decode` 488 → 264 B; `_start` +8 B. x86-64 decode instructions
  ±0.0001%, `slotcost` and the encoder within noise.
- 05fe78f: the round count in `Work::Osd(OsdWork { bp_rounds, osd })`
  (a two-field variant put every `Work` value on the stack: 6 096 B
  frame); `hard_errors` documented; `TABLE_BYTES` by `size_of`; `N <
  256` asserted; check images at (10, 21) and (25, 21); two tests; the
  API changes noted.
- 624c4de, a7a9f6a: bench records mark OSD-1 decodes and count its runs;
  `compare` by origin; `osdpath`, `osdinput`.
- OSD-1 per run through the decoder: x86-64 13.59 µs mean (1 669 runs on
  the slotcost slots' pass inputs); M33 199 731 instructions mean,
  226 010 max (15 runs, six pass inputs); A76 (2026-11-01) 22.79 µs
  mean, 26.69 µs max (`osdlib` 23.16 µs).
- A76 shim placement: `slotcost host` 2980198 vs 923c393 (same decode-path
  code), −0.06 ms (−0.05%) on 107.11 ms. A76 records equal x86-64's.
- Option off: records and 800 WAV slots identical to 511bf32's.
- Findings additions (crowded vs empty q_T by pass, assumed parts, n as
  a mean, the χ² grouping, shared host/small events); the gated round
  revised for host only (task plan), still proposed, not run.

## Session: 2026-11-01 (OSD-1 pre-gate fixes; branch `osd1`)

- ef2470e: `cat-ft8-decode --config ...,osd=C:T`. 8332aa5: `paired_bench
  gains` (the gain bars' tool, ≤ −17 dB inclusive), `BLOCK` for
  `slotcost` and `osdpath`, any new false decode fails with OSD on, the
  origin join asserted.
- Out of tree, hashed in the plan: `classify_osd.py` with (o) and (s)
  classes and tests; the M33 harness manifest.
- Gate text: WAV matching rules, R with OSD-triggered subtraction
  (1.64 ms each on the A76), the slotcost command and "up by" pinned.
- Smoke on `main` (used): pooled 0.0615 new decodes per slot (bar
  0.026), 0.0170 at or below −17 dB (bar 0.010); no loss, no new false
  decode; classifier 0 unexplained. Option off identical to 511bf32.

## Session: 2026-11-01 (OSD-1 gated round named; branch `osd1`)

- 4b64991: `gains` checks its inputs (seed, samples, messages; missing
  set or block). Out of tree, hashed in the plan: `classify_osd_v2.py`
  (wider (s)), the WAV matcher and the reference runner, the gate
  scripts, the M33 manifest (with its `Cargo.toml`, `cargo +1.93.0`).
- The reference decoder run once on the 800 WAV slots, its outputs
  hashed; no option-on WAV output before it.
- Gate commit: gain block `gosd1`, empty blocks `gosd1e01`–`gosd1e20`;
  status running.
- Gated round run once on `gosd1` (x86-64 records and WAV, A76, M33):
  every numeric criterion passes; `compare` fails two of 25 sets
  (crowded: its multi-pass CRC-expectation check, unchanged by OSD-1;
  sweep: its two-sided shift check, the 50% point 0.052 dB better).
  Applied mechanically: FAIL. Nothing merged, nothing deleted.
- 7ccbd48: `compare`'s multi-pass CRC check against the old run's own
  passes (3·√ margin; absolute limit reported), `--gain` for a one-sided
  sweep shift. On gosd1: off vs off now PASS (was FAIL); off vs on with
  `--gain` PASS. 56 replayed earlier comparisons and 280 self-comparisons
  unchanged. Second gated round drafted (proposed, not run).
- 689a361: CRC gate paired per slot (2·√), false checks relative to the
  old run, pass counts printed, gate unit tests. gosd1: off/off and on/on
  PASS; off/on PASS with `--gain` (exploratory). 56 earlier comparisons,
  re-paired by text, unchanged; 280 self-comparisons PASS.
- Second round draft: the user's decision recorded (one retry; the OSD
  code deleted if it fails), new hashed scripts and inputs (corpus copied
  into the cache), the A76 baseline now 0x0 (any bit voids a run).
- Gate 2 closure fixes (scripts out of tree, hashed in the plan): the
  A76 0x0 pre-check, a voided run = no result (one repeat after a reboot
  by the user; a second void = no verdict), `CODE` in the hashed
  `gate2.env`, the base binary pinned, the WAV count checked, `env.sh`
  hashed, every script logs its and the manifests' sha256; the paired
  CRC gate's residual risk disclosed; the replay table corrected.
- Second gated round, named at 037ed44 (`hosd2`, `hosd2e01`–`hosd2e20`,
  CODE 689a361), run once each (x86-64, A76 at 0x0, M33): every
  criterion passes; pooled gain 0.0645 per slot, 0.0190 at or below
  −17 dB; 0 new false decodes; 0 losses; R 6.02 ms. Verdict PASS.
- 748f34d: OSD-1 on in HOST (10, 21), SMALL off; docs, README, ADR 0024
  amendment, embedded-check. HOST records = the gates' OSD-on records;
  SMALL and `host,osd=0:21` records and WAV output = 511bf32's. Release
  decoder handed over for external acceptance (sha256 d7bba820…).
- 8303c71: enable follow-ups (tools force OSD off, `osd=off`, embedded
  names, docs); ADR evidence called synthetic, the WAV slots described,
  sparse CPU (A76 +0.39 ms per slot, valid at 0x0). `host,osd=off`
  identical to 511bf32's host on records and WAV output.

## Session: 2026-11-01 (O14, the subtraction's generator; branch `o14` from b93516e)

- A variant not on the branch: the generator's running phase
  in four lanes (sliding sums of four). A76 subtraction −0.11 ms (0.2%),
  slot unchanged, within the spread; x86-64 cycles +0.54%,
  instructions +2.8%; microbenchmark A76 −1.7%, x86-64 +4%. Not faster:
  kept out. The generator's cost is `increments` (x86-64 17.1% of the
  slot's cycles against `fill_phase`'s 3.6%), not the running sum.
- 68634cc: ⌊r · 2³² / d⌋ in 32-bit arithmetic for the envelope and the
  step phases; the M33's 64-bit library division gone. M33 subtraction
  per call 36.378 → 36.007 M (−1.02%), host 40-message slot −0.56%,
  flash −910 to −930 B per image, static RAM and frames unchanged
  (encoder-only `fill_phase` 112 → 80 B). A76 and x86-64 neutral.
- Identity at 68634cc: x86-64 records (73 800) and 800 WAV slots ×
  3 × 3, encoder, the emulated small and host images on seven slots,
  the multi-pass residuals on s0086 and s0407. Local CI passes.
- Every timed A76 run valid (24 runs, every sample 0x0). The A76
  record run was interrupted when the Raspberry Pi 5 went off the
  network; after it came back, rerun from scratch: records equal to
  x86-64's (73 800, 0 differ), the 800 WAV slots × 3 × 3 identical, the
  release library tests with the ignored ones pass (189).

## Session: 2026-11-01 (O15, the generator's per-sample step; branch `o15` from `o14`)

- Measured first (out-of-tree microbenchmark, A76 and x86-64): on the
  A76 the plain running sum is half the generator (1.9 cycles a
  sample), which corrects O14's reading; `increments`' per-call cost is
  about 9% of it (ceiling about 0.8 ms a crowded slot, not pursued); on
  x86-64 `increments`' two-floor conversion dominates.
- b18ded6: x86-64 increment with one rounding and one floor (exact for
  [0, 2³¹), every `f32` tested). x86-64 cycles −3.4%, `increments`
  17.2% → 13.6% of the slot; AArch64 and M33 code unchanged.
- 81cb159: the running sum in groups of four (scalar, paired loads and
  stores) on x86-64 and AArch64. A76 three-pass slot 107.50 → 104.40 ms
  (−2.9%), subtraction 56.78 → 53.55 ms (−5.7%), 13× the spread;
  x86-64 neutral; M33 unchanged (plain loop kept).
- Identity at 81cb159: x86-64 and A76 records (73 800 each, 0 differ),
  800 WAV slots × 3 × 3 on both, encoder, the emulated small and host
  images on seven slots and the residuals. Local CI and the A76 test
  suite pass. Every timed A76 run valid (12 `slotcost`, 11
  microbenchmark runs, every sample 0x0). Sizes and frames unchanged.

## Session: 2026-11-01 (O15b review follow-ups; branch `o15b` from `o15`)

- Three running-sum forms measured on every target (variants not on the
  branch): the grouped form is the only one faster on the A76 (b and c
  0.33–0.35 ms slower on the subtraction, about 0.2 ms (0.2%) on the
  slot, at the edge of placement noise);
  it costs the M33 +4.3% per subtraction and +108 B, x86-64 neutral.
- 641c599: the grouped sum on AArch64 only (`GROUPED_SUM`), its comment
  on the emitted code; `round_small` shared by `floor_small` and the
  x86-64 increment; `phase_of_ratio` in `decode::math`; docs re-wrapped.
  A76 within spread (104.40 against 104.35 ms); M33 subtraction
  unchanged (36.007 M per call) and `check.sh` sizes unchanged, the host
  slot +1.29 M (+0.05%, `Decoder::pass` feed, layout and inlining);
  x86-64 within spread.
- Findings fixes from review (logged per-pass medians, the
  microbenchmarks' low-clock samples, local-only hashes, wording).
- Identity: records x86-64 and A76, 800 WAV slots × 3 × 3 on both,
  encoder, M33 dump images and residuals. Local CI and the A76 suite
  pass. All 20 timed A76 runs valid (2 900 samples, all 0x0).

## Session: 2026-11-01 (O15c closure fixes; branch `o15c` from `o15b`)

- Records and a test only: the M33 claim narrowed (subtraction and
  sizes unchanged; host slot +1.29 M from inlining in `Decoder::pass`,
  confirmed from the emulator's per-function counts), O15's carried-add
  wording, the A76 form result restated, rustc version on the codegen
  claim, `round_small` pinned at its edges. Log files now carry the
  round in their names.

## Session: 2026-11-01 (O16, the front end outside the butterflies)

- Measured first (out-of-tree microbenchmark of the front end per row,
  A76 and x86-64, ten variants checked bit for bit against the crate's
  `feed`): the window fused into the first FFT stage is the larger part,
  the block-level input checks the smaller; four blocks side by side
  with one permutation entry per four is best on the A76 and ties the
  natural-order form, which would have changed `TABLE_BYTES`.
- 78b1755: the Hann window applied as the first radix-4 stage loads its
  inputs (the same values and products in the same order; the table
  layout checked at compile time), and `feed` checks each block of
  samples as a whole before copying it (`feed_i16` needs no check).
  A76 three-pass slot 104.40 → 101.40 ms (−2.9%), feed 22.23 → 19.13 ms
  (−13.9%), 31× the largest untouched stage's shift; one pass 20.30 →
  19.30 ms. M33 feed −3.9 to −4.0%, small slot −2.6%, host slot −0.95%;
  flash +96 to +100 B, RAM and frames unchanged. x86-64 cycles −2.4%
  (instructions −3.4%).
- An out-of-line M33 measurement build split the feed: butterflies 80%
  (159.7 M a pass), window and gather 10.6 M, input checks 4.9 M,
  powers and levels 16.1 M. Not shipped (no better than the inlined
  build).
- Identity: x86-64 records (73 800) and 800 WAV slots × 3 × 3, encoder,
  M33 dump images on seven slots and the residuals; A76 records
  (73 800, 0 differ) and 800 WAV slots × 3 × 3. Local CI and the A76
  test suite (193) pass. Every timed A76 run valid (8 `slotcost`, 8
  microbenchmark runs, every sample 0x0).

## Session: 2026-11-02 (O16b review follow-ups)

- dc9fc22: the radix-4 sum shared by the butterflies and the first stage
  as a macro. A function, even inlined, changed the transform's code on
  every target; the macro leaves every binary's `.text` byte-identical.
- a145178: the first stage's group size is its own parameter with its
  own assertion (it divides 4), and the docs no longer call the group
  lanes. Offsets are named constants from N; the stated bounds are
  asserted; n₀ is now base(b). `.text` byte-identical on x86-64,
  AArch64 and thumbv8m. Measured and not kept: per-block window slices
  (A76 `feed` +0.65%, M33 +0.09 M) and a group of four on the M33 (feed
  +2.5%, +212 B flash).
- 4150813: tests check that `skip` and `feed_i16` leave the counters at
  0 (the (0, 1) `skip` mutant now fails); the feed test's comment names
  its anchors.
- O16 findings fixed: one statistic for the crate-level table and the
  noisy x86-64 base run, 199.94 M and 2.43 M, the microbenchmarks'
  warm-up clocks, and 78b1755's two optimisations noted as one commit.
- Local CI and `check.sh --stack` pass at 4150813. The 8 A76 runs are
  valid (one warm-up sample at 1.5 GHz, disclosed); a first set of 8
  was void (a quoting error) and discarded.

## Session: 2026-11-02 (O16c closure follow-ups)

- 43079a8 (twiddles as arrays, `Same` removed), 79e8d84 (comments and
  an assertion message), 9dabad0 (the first-stage test says group).
- `.text` and `.rodata` byte-identical to 78b1755's on all eight
  binaries; the comparison script now builds from clean directories and
  records its provenance. Local CI and `check.sh --stack` pass.

## Session: 2026-11-02 (O17 belief propagation)

- Measured first: an out-of-tree microbenchmark of the sum-product
  round, its parts and `exp2_neg` / `log2`. On the A76 the tanh and atanh
  passes are 68% of the round. Inputs, products and sums are 36% and were
  issue- or latency-bound. No memory or branch effect.
- 6bc283f: on x86-64 and AArch64 the messages are laid out in groups of
  four checks of a degree (`place`), and the sum-product products run
  four checks in the lanes of each step. The M33 keeps groups of one
  (its previous layout and code). Every walker takes the group size as
  a const parameter, so the tests run both layouts on any host.
- 63d8dd6: the round's inputs in blocks of eight on AArch64 only; the
  x86-64 and M33 binaries are byte-identical to 6bc283f's.
- 37ec493: min-sum over four checks at a time (selects for the two
  smallest magnitudes). 6bc283f had copied each check out and back,
  costing the small configuration on hosts +5% (x86-64) and +8% (A76).
- A76 host three-pass slot 101.30 → 99.65 ms (BP −9.3%), one pass
  19.30 → 18.40; small 14.40 → 12.90 ms (BP −28%). x86-64 host −0.7%
  cycles, small −22%. M33 counts at 37ec493 identical to the base's in
  both images; flash, RAM and frames unchanged.
- Identity: x86-64 and A76 records (73 800, 0 differ) and 800 WAV slots
  × 3 × 3 on both; encoder; M33 dumps on seven slots and the residuals.
  Local CI and the A76 test suite (194) pass. Every timed A76 run valid.
- Not done: the A76 records of 6bc283f; per-pipe counters for the tanh
  and atanh passes; `exp2_neg` and `log2` unchanged (no same-operation
  form found).

## Session: 2026-11-02 (O17b review follow-ups)

- a034ecd, 4813087, 508fc10: comments only (`place`'s doc says which
  code relies on its shape; two comments keep the reason; the blocked
  round is tested only on AArch64).
- eb9f723: `group_products`' scratch has D rows. The same change in
  `group_min_sum` and the plain loop moved into `inputs` changed code,
  and were not kept.
- 3fe54a0: the min-sum round test carries NaN, ±∞, −0 and ±MAX (a
  NaN-only mutant now fails); the layout test renamed.
- d9dadbd: `cat-ft8/codegen-check.sh`, an AArch64 assembly floor on the
  rounds' `.4s` instructions, in the aarch64 CI job; shown to fail for
  groups of one and for 4, 8, 16 and 32 pass values.
- `Layout<G>` associated constants tried: fine on 1.75 and byte-identical
  on hosts, but the M33 images moved, so not kept.
- Every commit's eight binaries are byte-identical to aec6a07's. The nine
  O17 mutants were rerun with logs (all caught); local CI passes; the A76
  library tests were rerun at the head with the source commit recorded.

## Session: 2026-11-02 (O17c closure follow-ups)

- The code-generation check: cleans the library before building (a
  second run failed), counts in awk (no `grep -P`), and has a ceiling of
  25 single-lane moves in `sum_product_round` (15 at the head; 46 with
  the group products in place, which the A76 timed at BP +0.99 ms; 38
  with groups of one). The thin margin under passes of 4 values (452 for
  a floor of 500) is noted. An intermediate products form still passes.
- A test comment on comparing NaN bits; the NaN-only mutant in the
  mutant script, with logs.
- Records corrected: the toolchain re-check points at the check, two
  O17b rejections become follow-ups, and the mutant-count note is
  reworded.
- `.text` and `.rodata` byte-identical to aec6a07's on all eight
  binaries; local CI passes.

## Session: 2026-11-02 (O18, the subtraction)

- Profiled first on the A76 (O13's microbenchmark brought up to date,
  counters and PC samples): 24.5 cycles a waveform sample, 1.72× its
  issue floor; the polynomial passes near their floor, the removal's
  clamp 11 of its loop's 23 vector instructions, and the generator
  computing a constant increment over 26% of each symbol and an envelope
  of 1.0 nearly everywhere.
- a17baa6: the input clamp without a branch in the removal and the time
  search's products (`input_select`; exhaustive test, ignored).
- 7d747e3: one phase increment per symbol where its pulse is alone and
  exactly 1.0 (tested against the table).
- 513fd17: no envelope, no multiplication by it, for the blocks clear of
  the ramps (`Synth::fill_phase_only`).
- A76 crowded three-pass slot 99.65 → 94.00 ms (−5.7%), subtraction
  53.63 → 48.12 ms (−10.3%), untouched stages within 0.11 ms; x86-64
  cycles −4.6%; M33 subtraction 36.01 → 32.43 M instructions a call
  (−9.9%), host slot −5.5%, host flash +380 B (the second
  `scaled_cos_sin` instance), stack high water lower.
- Not kept: the reference in two planes (the loop vectoriser interleaved
  the fit, +33%).
- Identity: records on x86-64 and the A76 at all three commits, WAVs × 3 ×
  3 on both, encoder, M33 dumps and residuals; local CI and the A76
  library tests (199) pass; four mutants caught.

## Session: 2026-11-02 (O18b review follow-ups)

- 8dc7a85: the branch-free clamp only where `max`/`min` are instructions
  (x86-64, AArch64, `--cfg cat_ft8_minmaxnm` for thumbv8m.main with its
  FPU, set in the embedded check); thumbv7em's subtraction loses its 34
  `fmaxf`/`fminf` calls. A build script was tried and moved code on every
  target.
- a6feb13: one `input`, chosen per target; `input_branch` is the
  reference and the fit's edge samples' form.
- 75729ab: comment on the reference's unit weights (a `()` slice changed
  code). 6f389c4: `blocks` asserts the generator starts at 0.
- 85ba626: the code-generation check counts `fmaxnm`/`fminnm` in the
  removal and the time search and selects in the removal; shown to fail
  with the branch form.
- Every commit's eight binaries byte-identical to 513fd17's; x86-64
  record comparison fixed in the script and rerun; findings corrected
  (x86-64 records log, the M33 +632 B).

## Session: 2026-11-02 (O18c closure follow-ups)

- 1d53ad8: the code-generation check fails on a missing function or an
  unknown metric.
- 795bb7e: it also checks the clamp on thumbv8m (with the cfg,
  `vmaxnm`/`vminnm` floor) and thumbv7em (no `fmaxf`/`fminf` calls);
  both shown to fire; the aarch64 CI job installs the targets.
- 0d7f951: the crate docs document `--cfg cat_ft8_minmaxnm` with a
  target-scoped config entry. c4fa1ae, eb0cbfc: the embedded check's
  stack headings and clippy with the cfg. 38aec91: wording.
- `.text`/`.rodata` byte-identical to 513fd17's at the `.rs` commits;
  local CI passes. Follow-ups recorded: the fit's edge samples, an i686
  run for the branch form, `Synth::envelope_is_one`.

## Session: 2026-11-02 (O19 subtraction)

- 0286ee8: the generator's phase in closed form where the pulse is
  alone (`Synth::phases`): the run's first phase plus k increments,
  exact in `u32`. A76 subtraction 48.10 → 47.26 ms (reproduced in two
  sequences); M33 32.428 → 32.344 M instructions a call; flash +592 to
  +740 B (the generator inlined into `skip` and `fill_phase`). A
  runtime-argument form that kept one copy was slower on the hosts: not
  kept.
- 7e388d3: the subtraction's reference in groups of four lanes (`GROUP`;
  one on targets without a vector unit). No `ld2`/`st2` left; the fit's
  loop 27 → 20 AArch64 instructions per four samples. A76 subtraction
  47.26 → 43.77 ms, slot 93.0 → 89.7 ms; M33 32.198 M a call. The
  code-generation check covers interleaving loads and stores, the
  reference block's shuffles and `whole_lane_sums`; each shown to fire.
- In all: A76 subtraction −9.0%, slot 94.0 → 89.7 ms; x86-64 cycles −2.34%; M33
  −0.71% a call.
- Identity: records on x86-64 and the A76, WAVs × 3 × 3, encoder, M33
  dumps and residuals, at both commits; local CI and the A76 library
  tests pass; eight mutants caught.
- With `GROUP` forced to 1 on x86-64 (not shipped), one release test
  case failed; found in O19b to be LLVM's loop vectoriser miscompiling
  the test's own expected-value loop (corrected in the findings).

## Session: 2026-11-02 (O19b review follow-ups)

- The forced group-of-one failure was LLVM's loop vectoriser (LLVM 20
  and 21) miscompiling the test's own `acc[(i0 + k) % LANES] +=` loop,
  not `add_all`. 580938c, ffba08e: the tests sum lane by lane; the
  forced run passes (199).
- 954e5b0: `lane-index-check.sh` in CI fails on arrays indexed by a
  lane-wrapping expression (textual; limits stated).
- 78aab1f: `--cfg cat_ft8_test_group_one` runs the M33's layout in the
  release library tests on x86-64 (CI and local CI). Deleting the
  groups-of-one branches cost the M33 +20%: kept.
- 4eb4fd0 ring assertion; 00ef632 docs; ff3b924 compile-time assertion;
  35aa6da floors beside the code-generation ceilings (both shown to
  fire).
- Every `.rs` commit's eight binaries byte-identical to 7e388d3's. The
  `Slots` view changed code on every target: a follow-up, with the one
  split helper.
- Records corrected: O19's group-of-one note, the closed-form note, the
  encoder-only +720 B; logs annotated with their real trees.

## Session: 2026-11-02 (O19c closure follow-ups)

- 8c81172: the lane-index source check removed (it missed the
  let-bound form, swallowed grep errors, and invited that form); the
  lane-by-lane release tests with and without the groups-of-one cfg are
  the protection. No production code has the pattern.
- 19cc572 what the groups-of-one config covers; b8a3566 the ring's
  tautological assertion dropped; 2f1d160, 742a710 the codegen check's
  header. A compile-time check of every `W` (an associated constant)
  changed host code: not kept.
- `.text`/`.rodata` byte-identical to 7e388d3's at every `.rs` commit;
  O19b records corrected (the 296.09 M closure, the filtered test).
