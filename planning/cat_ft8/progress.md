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

- Six reviews of the whole decoder consolidated into fix list sections
  A (required, results identical), B (simplicity, results identical)
  and C (optimisation passes, later). The user approved A and B followed
  by C on 2026-10-27. D12 plan written into the task plan before code;
  C waits for re-review.
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
- D12 re-reviewed (six approvals). D12a cleanup recorded in the plan;
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
- Pass reviews (QA, and a review for simplicity): both pass with
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
- E0 checkpoint reviewed (four reviews; open-source clean). E0a recorded
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
