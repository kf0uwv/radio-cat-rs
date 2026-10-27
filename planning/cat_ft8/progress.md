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
- D8/D8b committed (dd51ea1). D9/D10 go-ahead (D11 runs elsewhere);
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

## Error Log
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-10-25 | Stage timings ~6 µs each | 1 | `Instant::now()` overhead on this host; time each stage as one batch |
