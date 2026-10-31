# cat_ft8 — task plan

## Goal

A new workspace crate `cat-ft8`: FT8 implemented from the published
protocol (Franke, Somerville, Taylor, QEX July/August 2020). This step is
the encoder: message pack/unpack, CRC-14, LDPC (174,91), tone mapping and
GFSK synthesis. Decoding follows later in the same crate, after its own
plan is approved.

## Current Phase

Phase 1 (encoder) committed on `feat/cat-ft8`; ADR 0024 accepted
2026-10-26. Phase 2 (single-pass decoder): D1–D10 committed (D11, the
comparison with an external reference decoder, is not part of this
repository's CI). Review of the whole decoder done; fix round D12 (A and
B, results identical) approved by the user 2026-10-27, then up to five
optimisation passes (C). Phase 3 (single-pass changes that alter
results, E0–E4) done. Phase 4 (subtraction and further passes): plan
approved; S0 and S2 done (stopped at Checkpoint S2); S1 not needed (S2
measured the sub-sample timing residue at −53 dB at worst); S3 next,
not started.

## Hard constraints

- No WSJT-X source (GPLv3) is read or used. WSJT-X `ft8sim`/`jt9` are
  black-box test oracles only, never linked or needed to build or test.
- `ft8_lib` (MIT) may be studied; anything taken is credited in source and
  in `cat-ft8/NOTICE`.
- `#![no_std]` core: no allocation, no threads, `f32` only. Must build for
  `thumbv8m.main-none-eabihf` with `--no-default-features`.
- Deterministic output on every platform.
- No local crate dependencies; nothing radio-specific; no device audio.
- Apache-2.0 header on every file; package fields `.workspace = true`;
  MSRV 1.75.

## Encoder steps (done)

1. [x] Crate skeleton, features (`std` default, `profile`), workspace member.
2. [x] Message packing/unpacking: types 0.0, 0.1, 0.5, 1, 2, 4.
3. [x] CRC-14, LDPC (tables from ft8_lib), Gray/Costas tone mapping.
4. [x] GFSK waveform into a caller buffer; WAV writer; `cat-ft8-encode`.
5. [x] Tests: unit tests per stage; black-box vectors from `ft8sim`.
6. [x] `profile` feature: stage CPU time, memory account; bench example.
7. [x] ADR 0024, NOTICE, README/CLAUDE.md/ADR index, CI steps.

## Fix round (approved 2026-10-25)

Encode-stage algorithms (CRC, LDPC, tone mapping) stay as they are: their
results must not change. The waveform changes deliberately (sine table).

1. [x] Planning: this list, `findings.md` (black-box discoveries moved
       there), `progress.md` note that review was retroactive.
2. [x] `CallTable` recency: last-seen counter refreshed on re-hearing,
       evict the oldest; regression test (K1ABC, W9XYZ, K1ABC, G4ABC).
3. [x] Side-effect-free parsing: parse returns fields and the calls to
       remember; `pack()` remembers once, on success. Remove the double
       parses, the output-slicing in `write_c28`, the duplicate
       `parse_c28_call`, the `MAX_WORDS + 1` sentinel.
4. [x] API hygiene: `Error` `#[non_exhaustive]`; reserved types →
       `InvalidPayload`; `unpack` rejects a standard-call value `pack`
       would never produce (round-trip check).
5. [x] Type 5 (EU VHF contest): pack and unpack, verified black-box.
6. [x] Pulse table in flash: generated `static` half table (symmetry),
       test regenerates it bit-for-bit; remove `EMPTY`/`init`/`ready`/
       `NotInitialized`; no `Clone` of large values.
7. [x] Sine from the u32 phase: f32 quarter-wave table with
       integer-indexed linear interpolation (table regenerated in a test);
       ramp from the same routine. No `f64` anywhere in the core; `libm`
       becomes a dev-dependency.
8. [x] Streaming synthesizer: `Synth` holding phase and sample index,
       `fill`/`mix` into caller blocks, per-sample phase and envelope
       exposed for a later subtraction reference; `synthesize` built on
       it; symbol-outer loop; block-rendering doc example.
9. [x] Robustness: saturating start arithmetic; reject non-finite dt and
       amplitude; top-tone check at `7·Δf`; `wav.rs` checked arithmetic.
10. [x] Determinism pinned: golden FNV-1a hashes of a waveform and a full
        slot for several messages, asserted in tests (Linux and Windows
        CI); document the IEEE round-to-nearest assumption.
11. [x] Simplicity: profiling code moves from the library into the bench
        example (library keeps `pub const` sizes); AWGN moves from the
        binary into an example.
12. [x] Property tests: seeded round trips per supported type; random
        payload fuzz (`unpack` never panics; repack equals the payload
        except listed non-canonical cases). Random `ft8sim` vectors for
        types 4, 0.0, 0.5, 0.1, 2, 5 and type 1 with /R, /P, hashes, CQ
        modifiers and wide reports (generator kept outside this repo; data
        copied in).
13. [x] Embedded link check: a tiny `#![no_std] #![no_main]` binary with
        no allocator that encodes and synthesizes, built for thumbv8m in
        CI; CI fails if it references `__aeabi_d*`; flash size and stack
        sizes recorded. Drop the nested-cargo test.
14. [x] Docs and CI: intra-doc links valid with and without default
        features (`cargo doc -D warnings` in CI); clippy with all features
        and host `--no-default-features` tests in CI; ADR 0024 and NOTICE
        updated (scope rationale, provenance, `thiserror`, determinism,
        type 5, future API growth, decoder unpack order); README test
        count; CLAUDE.md planning list; test data mode 644.
15. [x] Verify: all tests, clippy variants, fmt, docs, thumbv8m release
        build; jt9 decode check on the new waveform; golden hashes.

## Cleanup addendum (within the approved fix-round plan, 2026-10-25)

No change to encode algorithms or waveform output: golden hashes must stay
the same. Optimisations stay for later results-identical passes.

C1. [x] Link check reaches every public waveform entry point
        (`Placement::slot`, `synthesize` at a negative start, `Synth::mix`,
        `skip`, `fill_phase`); negative test (f64 in `synthesize` must
        fail); ADR 0024 §3 states exactly what is covered; `check.sh` runs
        clippy and fmt --check on the check program.
C2. [x] Colliding-call lookup test uses a real 10-bit collision and
        asserts it, then newest-wins; dead code removed.
C3. [x] `tests/properties.rs`: specific reasons only (no blanket type-4
        CQ excuse), no empty `if`, public constants instead of numbers;
        fuzz repack check also for types 5, 0.1 and non-CQ 4 by seeding
        the table with calls whose hashes match.
C4. [x] Random vectors the oracle printed no text for: still assert bits
        and symbols.
C5. [x] Pin the `libm` dev-dependency exactly.
C6. [x] Wording: sine error below 4e-7; `Synth` about 100 bytes;
        determinism stated as IEEE-754 binary32 round-to-nearest in README
        and lib.rs; `fill` vs `mix` sign of zero.
C7. [x] `STATIC_TABLE_BYTES` from the actual statics (linked ones only).
C8. [x] Licence headers on `check.sh`, `link.x` and generated tables.
C9. [x] `Synth` derives `Clone`.
C10. [x] Bench reports pack with `NoHashes` and `CallTable::remember`
         (hit, miss with eviction) separately.
C11. [x] Plan "Later": field-level payload API; black-box recovery of the
         Field Day and RTTY Roundup tables.
C12. [x] Provenance wording: ft8_lib encoder consulted; hash multiplier
         taken from ft8_lib if the paper does not print it.
C13. [x] Dates corrected to 2026-10-25.
C14. [x] README line wrap and stale Windows sentence.
C15. [x] Black-box question: does `<K1ABC>` resolve after `K1ABC/R` or
         `K1ABC/P` was heard? Match WSJT-X or document the difference.
C16. [x] Remove the stray `cat-ft8/embedded-check/target/`.

## Optimisation passes (results identical; at most 5; 2026-10-25)

Each pass changes one thing for CPU or memory. A pass is kept only if
every vector test (bits and symbols), the property tests and the golden
waveform hashes are unchanged, and its target figure (median of 3
`encode_bench` runs, or table/stack/link-check bytes) improves by more
than 3% with nothing else worse by more than 3%. Otherwise it is reverted
and recorded as rejected. Stop after two passes in a row under 3%.
Every pass: tests, clippy, fmt, `embedded-check/check.sh`.

Baseline (release, median of 3): symbols 300 ns, pack 325 ns (NoHashes
139), remember 32 ns hit / 87 ns miss+evict, crc 130 ns, ldpc 131 ns,
waveform 0.765 ms/slot (0.739 in blocks); tables 17 055 B, working 400 B,
synth 104 B, call table<64> 1 552 B, heap 0. Self time: `symbols::map`
19.8%, `Synth::mix` 12.9%, `CallTable::remember` 12.6%, `Synth::fill`
12.5%, `crc::append_crc` 8.8%, `normalize` 5.9%.

T1. [x] Codeword as one contiguous MSB-first 174-bit word; tones read 3
        bits at a time; Costas copied with `copy_from_slice`; data
        positions from one source of truth.
T2. [x] CRC-14 from a `const` 256-entry `u16` table, a byte at a time.
T3. [x] (rejected) `CallTable::remember`: duplicate and oldest search in one pass;
        `seen` as `u32` (with a renormalising clock so order never wraps).
T4. [x] `PULSE_HALF` stored from its first non-zero sample; index
        clamped (less flash, fewer multiply-adds by zero).
T5. [x] (rejected) Best remaining hot spot after re-profiling: the call
        table's memory (`seen` as `u32`, two-pass search kept). Sine padding
        and the ramp division were judged under 3% from the profile.

## Follow-up to the optimisation passes (2026-10-26)

Output stays bit-identical (golden hashes, vectors, properties).

F1. [x] `Codeword::bits(start, 0)` returns 0 (was a 128-bit shift); test
        every length 0..=32 at every legal start against `bit()`;
        `bits` becomes `pub(crate)`; optional `[u64; 4]` padding word.
F2. [x] CRC table counted in `STATIC_TABLE_BYTES` (`pub(crate) static`),
        with a check that fails if a table is added uncounted.
F3. [x] `crc14_bits` test-only; `crc14` doc link fixed.
F4. [x] Const asserts on the frame layout (Costas at both ends, 3 × 58 =
        174).
F5. [x] Waveform loop as three fixed ranges per symbol (current +
        previous, current only, current + next), same addition order;
        const assert that at most two pulses overlap; doc fixed; measured
        once.
F6. [x] progress.md corrections (table bytes; the T4 ldpc swing).
F7. [x] (optional) `encode_bench` single-stage mode for instruction
        counts.

## Phase 2: decoder (single pass) — PLAN, revised (2026-10-26)

Design decisions are in ADR 0024, "Proposed amendment: single-pass
decoder" (accepted at D0 with its estimates; measured memory, flash,
stack, rate and cost figures replaced them on 2026-10-27). Sizing notes are in findings.md. One task at a
time, test first; each task ends with tests, clippy (four
configurations), fmt, docs, the bare-metal link check (from D2 on, grown
each step) and a progress entry. Architect checkpoints are marked ◆;
the user decides at ◆◆.

### Scope

In: slot framing and a block-fed spectrogram front end, Costas sync
search with local-maximum suppression, single-symbol soft bits, LDPC
belief propagation, CRC check, all-zero rejection, unpacking (or payloads
only), frequency / time / SNR estimates, a capacity-limited decode list in
a defined order, a JSON-lines decode command, and the link check.

Out (each needs its own plan later): signal subtraction and further
passes, multi-symbol block detection, ordered-statistics decoding,
a-priori decoding, decoding before the slot ends, Field Day and RTTY
Roundup unpacking.

Single-pass headroom, also later and each measured as its own change:
sum-product versus min-sum (whichever D6 does not choose), more message-
passing rounds and candidates, per-bin noise normalisation, window
choice, and sub-grid time and frequency refinement. The first measured
result is expected near the external reference decoder's single-pass
level, not above it.

### Decisions this plan rests on (details in the ADR draft)

- **Geometry.** Row k is the 3 840-sample Hann window over samples
  [240 + 960k, 240 + 960k + 3 840), k = 0..184 (184 rows). The 240-sample
  origin puts dt = 0 symbol centres exactly on the row grid (a zero origin
  leaves them a quarter hop off). Frequency bins are 3.125 Hz.
- **Time offsets.** Every symbol centre has a row for dt ∈
  [−0.40, +1.76] s (the sample extent alone would allow [−0.48, +1.84] s;
  the row centres are what count). The search covers dt ∈ [−2.5, +2.5] s: outside the
  full-fit range, symbols that fall outside the rows are absent — not
  scored in sync, and their bits are erasures (LLR 0). A candidate needs
  at least two of the three Costas arrays inside the rows. Scores are
  normalised exactly in integers: the summed score of the n arrays
  present (n = 2 or 3) is multiplied by 6 / n (× 3 or × 2), so partial and
  full candidates compare on one scale.
- **Band** = the base (tone 0) frequencies searched: default 100–3100 Hz
  (961 base bins). The stored spectrum covers the band plus the 43.75 Hz
  above the top base frequency and one guard bin: 976 bins.
- **Levels.** 8-bit log-magnitude (0.5 dB steps) by default. A 16-bit
  option is kept only if it measurably improves decoding at the D3
  checkpoint; otherwise there is one configuration. Level type is a
  sealed trait (`u8`, `u16`).
- **Workspace.** One caller-owned workspace (spectrogram slice and
  front-end buffers, all borrowed, const-constructible). Across a slot
  boundary the caller buffers the next slot's first samples in a short
  FIFO while `decode`/`finish` run, then feeds them. Host: 2 s FIFO,
  128 decodes. Small target: see open question 2.
- **Slot framing.** `start_slot()` resets the front end (rows filled = 0)
  and keeps nothing else: decodes have already been handed to the caller,
  and the call table is the caller's. `skip(n)` advances over a gap
  (missing samples are zeros, counted); `rows_filled()` reports progress;
  decoding a partial slot is allowed and treats missing rows as absent.
  Non-finite samples are replaced by 0.0 and counted in the report.
- **Text and call table.** Decode entries hold no text (payload, type,
  frequency, time offset, SNR, sync score, candidate index, hard-error
  count, message-passing rounds: about 40 B). Text is rendered by
  `finish` into caller storage through a callback, using the caller's
  `CallHashes`, which persists across slots and may be shared with
  transmitting. `finish_payloads` returns payloads without unpacking.
- **Later audio input** (subtraction, block detection) gets its own,
  additive entry point; `decode_candidate` keeps its signature.
- **Determinism contract.** Candidates are ordered by integer sync score
  (descending), then row, then base bin. `decode_candidate` fully
  re-initialises its scratch and includes refinement and SNR; it is a pure
  function of the spectrogram and the candidate. `finish` takes results
  indexed by candidate index and processes them in index order: all-zero
  rejection, duplicate removal (lowest index kept), truncation to
  capacity, then unpacking on one thread. `decode()` is defined as
  `find_candidates`, then `decode_candidate` for each candidate in order,
  then `finish`, and a test checks they are identical. Output is
  identical for the same samples, configuration and starting call-table
  state, on IEEE-754 binary32 round-to-nearest platforms without
  flush-to-zero.
- **False-decode guards.** The all-zero codeword (which passes CRC with
  zero initial value and every parity check) is rejected. Each decode
  records its hard-error count and message-passing rounds for later
  gating.
- **Command output** (`cat-ft8-decode`, format version 0, documented in
  the crate): one JSON object per decode, `{"freq": Hz, "dt": s, "snr":
  dB, "message": "..."}`, where `freq` is the tone 0 frequency, `dt` the
  start relative to the nominal 0.5 s and `snr` the 2500 Hz SNR. Only
  unpacked decodes by default. `--raw` adds payloads that did not unpack,
  as `"payload"` (hex), `"type"` and `"unpacked": false`. `--profile`
  adds one `{"profile": {...}}` line.
- **Time budget** = the profile's total from the first `feed` of a slot
  to the return of `finish`, excluding file I/O and one-time start-up,
  single-threaded; the host CPU is named in results.

### Steps (TDD, one at a time)

D0. [x] ◆◆ Plan and ADR amendment accepted (with estimates) by architect
    and user (2026-10-26).
D1. [x] Deterministic numeric helpers: `log2` from IEEE bits plus a fixed
        polynomial, with a power floor (no subnormal reaches it); tests
        against `libm` (tolerance) and pinned bit patterns.
D2. [x] FFT: 1 920-point complex mixed-radix, in place with a static
        digit-reversal table, plus the real split for 3 840 samples;
        static twiddle, real-split and window tables (generator test,
        bit-exact); accuracy against an f64 DFT in tests; no allocation.
        Bare-metal link check starts calling it.
D3. [x] Front end and slot framing: `start_slot`, `feed` (f32 and i16),
        `skip`, `rows_filled`, non-finite sanitising, log levels into the
        caller's spectrogram; tests: tone lands in the right bin, row
        origin (dt = 0 on grid), block-size invariance (1, 7, 960, 1 921,
        whole slot), gaps, slot end, partial slot, memory sizes; 8-bit
        versus 16-bit measured on synthetic slots.
    ◆ Checkpoint: measured memory table; level-width decision.
      (Reached 2026-10-26; reviewed: proceed after a fix-up.)
D3a. [x] D3 fix-up: non-finite samples counted in the first 240 too;
         finite input clamped to ±2¹⁶ (counted); `power_spectrum`,
         `FftBuffer` and `WINDOW` crate-private (the link check's `.bss`
         then shows the real configuration); `row()` returns `Option`,
         levels readable through `&self`; one public level width (8-bit),
         `u16` only for tests and the `profile` bench as the D6 baseline;
         input-level documentation (−127.5…0 dB relative to full scale,
         no enforced minimum) and counters for levels at the floor and at
         the top.
D4. [x] Sync search: integer Costas score over the half-symbol,
        half-bin grid; dt ±2.5 s with partial sync (≥ 2 Costas arrays
        present, score × 6 / n); 3 × 3 local-maximum suppression on a
        rolling three-row score buffer before top-K, with the tie rule:
        a point is kept if its score is ≥ every neighbour that comes
        later in (row, bin) order and > every neighbour that comes
        earlier, so a plateau of equal scores keeps its first point;
        neighbours outside the grid (before the first or after the last
        time position, below or above the band) impose no condition;
        deterministic order; minimum score; truncation count; tests:
        encoder-made signals at random offsets found at the exact grid
        point, edges of the dt range, a strong signal beside weak ones
        (every weak one still a candidate), plateaus (flat 1 × 2, 2 × 1,
        2 × 2 and L-shaped regions keep exactly their first point; a
        plateau at a grid edge).
D5. [x] Soft bits: single-symbol max-log metric (QEX §6, N = 1), Gray
        demapping, variance normalisation, erasures for absent symbols;
        tests: noise-free → every sign right; erased symbols give LLR 0.
        Review checklist (D5 and D6): every argmax or hard decision on
        8-bit levels splits ties without bias (no systematic first-index
        win), and soft bits are zero on ties; stack measured
        (`check.sh --stack`) at every checkpoint from D4 on.
D6. [x] LDPC belief propagation over the parity-check table, per-thread
        scratch, round limit, early stop on zero syndrome, CRC check,
        all-zero rejection, hard-error count; check-node rule chosen by
        measurement (sum-product with rational approximations versus
        normalised min-sum); tests: random codewords with noise,
        error-correction counts, no false CRC passes on random input, the
        all-zero codeword rejected.
    ◆ Checkpoint: D4–D6 on synthetic slots.
D6a. [x] D6 checkpoint corrections: a direct `hard_decide` tie test (all-zero input
         gives `hard[i] == i % 2`; −0.0 is a tie); `hard_errors` counts
         only bits whose soft value is not 0 (documented);
         `CheckRule::MinSum` fieldless with `const ALPHA: f32 = 0.75`;
         `PARITY_CHECKS` counted in a decoder-level `TABLE_BYTES`
         (`DECODE_COUNTED`); `exp2` coefficients E2–E7 pinned by bit
         pattern. Then all checks; stop for the D4–D6 commit.
D6b. [x] Delete the 16-bit level type in its own change (bench `levels`
         mode removed; sealed `Level` with `u8` only), after go-ahead.
D7a. [x] Decoder API and merge: borrowed workspace, split API, `finish`
         and `finish_payloads` keyed by candidate index, `DecodeReport`
         (candidates found / kept / decoded / rejected, truncation,
         sanitised samples), `decode()` ≡ the split calls (tested).
D7b. [x] Refinement and SNR: frequency and time refinement on the grid
         neighbours, SNR estimate in 2500 Hz calibrated on synthetic
         slots of known SNR.
    D7 design (2026-10-26, after the D6 checkpoint decisions):
    - `DecodeConfig { rule, max_rounds, llr_scale, min_score, candidates }`
      with `HOST` (sum-product) and `SMALL` (min-sum, provisional until
      instruction counts) presets; LLR scale 4, 30 rounds, 200
      candidates. `Error::InvalidConfig` for a bad configuration.
    - `Decoder<'a>`: borrows the front end, levels, sync scratch and
      candidate list (checked against the band and configuration);
      forwards `start_slot`/`feed`/`feed_i16`/`skip`/`rows_filled`.
      Split API: `find_candidates(&mut self)`, then
      `decode_candidate(&self, index, &mut CandidateScratch)` (pure, any
      thread), then `DecodeList::push(index, &result)` in index order
      (out of order is an error) or `merge(&results)`; `decode()` is
      exactly those calls. `DecodeList` borrows the caller's `[Decode]`:
      duplicate removal (lowest index kept), truncation counted.
      `finish` unpacks into a callback with the caller's `CallHashes`;
      `finish_payloads` returns the entries.
    - `Decode`: payload, message type, frequency, dt, SNR, sync score,
      candidate index, hard errors, rounds. `DecodeReport`: candidates
      found/kept/dropped, decoded, duplicates, all-zero rejected, CRC
      failures, not converged, decodes truncated, rows filled, sanitised
      / clamped / skipped samples, floored / top levels.
    - `CandidateScratch { llr, bp }`, const-constructible, one per worker.
    - `sync`, `soft` and `bp` become crate-private; public only with
      `profile` (benches). `Candidate`, `CheckRule` and `BpFailure` stay
      public through re-exports.
    - D7b: sub-grid offsets from the sync scores of the four grid
      neighbours (parabolic, clamped to ±½ step); soft bits from power
      interpolated at the refined position (measured against the grid
      soft bits; kept only if better); frequency and dt from the refined
      position. SNR: signal power on the decoded tones (re-encoded
      payload) against a local noise reference — the median level of the
      bins just outside the signal over its rows — in 2500 Hz;
      calibrated and checked by frequency on white and on shaped
      (non-white) noise.
    ◆ Checkpoint: API, merge and estimates.
      (Reached 2026-10-26; reviewed: proceed after D7c.)
D7c. [x] Before the API freeze: `push` rejects `Ok(d)` whose candidate
         index differs from `index`; `#[non_exhaustive]` on `DecodeReport`,
         `Decode` and `BpFailure`; `DecodeConfig` gains const `with_*`
         setters (decision: callers build from `HOST`/`SMALL` with
         setters, so later fields are not breaking; the fields stay public
         for reading); `const fn` `Decoder::sync_scratch_len`,
         `Band::base_bins` and `sync::scratch_len`, and the link check
         sizes its statics from them; document that feeding after
         `find_candidates` leaves the candidates stale until `start_slot`
         or a new search; exact counters (a repeat of a truncated payload
         is a duplicate; `decoded` = distinct decodes) if free in memory,
         otherwise `decoded` documented as an upper bound after
         truncation; SNR documented as against local noise (it can differ
         from figures against a band-wide noise reference near passband
         edges); ADR drift (merge does rejection, dedup and truncation;
         `decode()` excludes `finish`; capacities are consts; the
         level-to-power table in the rule-3 tables list); the misplaced
         Hann doc comment in `fft/tests.rs`; tests: `DecodeConfig::SMALL`
         end to end in the split/determinism test, multi-chunk `merge`.
D8 notes (2026-10-26, from the D7 review; not started):
    - SNR in busy slots: neighbours fill the reference bins, so crowded
      slots read 1.5–2.5 dB low with outliers far lower. Make the
      reference robust (a lower quantile corrected for the exponential
      distribution, a wider reference, or a band-wide low-percentile
      fallback when the reference is busy); test with a neighbour 30–60 Hz
      away.
    - SNR tests on a steep passband (e.g. 600–1 600 Hz at −6 dB, about
      44 dB down by 2 kHz).
    - Add to the D8 tests: overlaps 10–30 Hz apart; partial, late and
      early decode; workspace reuse; two alternating workspaces;
      repeatability with a seeded call table; the bulk noise-only and
      carrier-plus-noise jobs.
    - No further tuning against crowded multi-signal performance (that is
      subtraction work, later).
    D8 design (2026-10-26):
    - Busy-slot SNR: the noise reference becomes a low quantile of the
      reference levels, corrected for the exponential distribution
      (as built: each side's lower octile over 24 bins, 25–100 Hz from
      the signal, sides averaged; measured against pooled and in-band
      alternatives, see findings); checked on white and shaped noise; measured before
      and after on busy synthetic slots (bench mode `busy`); tests with a
      neighbour 30–60 Hz away and on a steep passband (one-pole sections
      giving about −6 dB at the band edges of 600–1 600 Hz and about 40 dB
      down by 2 kHz). SNR is computed after decoding, so decode rates
      must not change (checked: the sweep reproduces exactly).
    - `tests/decode.rs` (integration, public API, debug-build sized,
      slots spread over threads; inputs from the encoder and `libm`
      noise, which is the same on every platform): pinned rate floors
      for `HOST` and `SMALL`; golden decode hashes (FNV-1a over every
      output field's bits, the text and the report) for both
      configurations; overlaps 10–30 Hz apart; early and late signals;
      partial input and decoding before the slot ends; input that starts
      late (`skip`); workspace reuse and two alternating workspaces;
      repeatability with a seeded call table.
    - `tests/decode_bulk.rs` (`#[ignore]`, release, its own CI job):
      thousands of noise-only and carrier-plus-noise slots, zero decodes,
      for both configurations.
    - D8b: an `ubuntu-24.04-arm` job running `cargo test -p cat-ft8`
      (golden waveform and decode hashes). Linux and Windows already run
      the workspace tests, so the decode hashes run there too.
D8. [x] WSJT-X-free end-to-end tests with the encoder and seeded noise:
        SNR sweep with pinned rate floors; noise-only slots (thousands,
        release build, separate CI job) including carrier-plus-noise
        slots, with zero decodes; non-overlapping and overlapping (10–30 Hz
        apart) signals; dt and frequency ranges; partial and late input;
        block-size and thread-count invariance; golden decode output hash
        on Linux and Windows CI.
D8b. [x] CI: a new `ubuntu-24.04-arm` (aarch64) job running
         `cargo test -p cat-ft8` (golden waveform and decode hashes; the
         bulk noise-only test stays in its own job), so the determinism
         promise is checked on a second architecture.
D9. [x] `cat-ft8-decode` (std): reads a 12 kHz mono 16-bit WAV; output as
        above; format documented as version 0; JSON written by hand with
        string escaping (`serde_json` stays a dev-dependency); tests parse
        the output with `serde_json`.
D10. [x] Link check final form: decoder fed a synthesized slot in blocks
         from a static workspace, no allocator, no f64 helpers; flash,
         `.bss` and stack reported.
    D9/D10 design (2026-10-26):
    - `wav::read_wav` (std): RIFF/WAVE, PCM 16-bit (format 1 or
      extensible with the PCM subtype); returns rate, channels and
      samples; unknown chunks skipped (odd sizes padded); clear errors for
      anything else. The command requires 12 000 Hz mono and says so.
    - `cat-ft8-decode FILE.wav [--config host|small] [--raw] [--profile]`:
      feeds the first 180 000 samples in 1 920-sample blocks (a shorter
      file is a partial slot; samples past 15 s are ignored), decodes with
      the split API (to time each stage; identical to `decode()`), unpacks
      with a 64-call table. Output format version 0, one JSON object per
      line: `freq` (Hz, 0.1), `dt` (s, 0.01), `snr` (dB, integer),
      `message`, `candidate`, `hard_errors`, `rounds`, `score`; `--raw`
      adds lines for payloads that did not unpack (`payload` hex of the
      10 bytes, `type` "i3.n3", `error`, `"unpacked": false`); `--profile`
      adds one `{"profile": {...}}` line (`format`, `config`, per-stage
      `*_seconds`, `*_bytes` memory with `peak_bytes`, every report
      count). JSON by hand with escaping (unit-tested); the format is
      documented in the crate docs and the ADR.
    - D10: the link check builds two images — `small` (Band::SMALL,
      DecodeConfig::SMALL, 64 decodes; default) and `host` (Band::DEFAULT,
      DecodeConfig::HOST, 128 decodes) — the check-node rule is a run-time
      field, so both rules are linked in either image — and reports flash (`.text` + `.rodata`), `.bss` and stack for
      each.
D11. [x] Measurement against an external reference decoder on synthetic
         slots across SNR, time offset, frequency, crowding and fading
         (external harness); profile and memory; ADR amendment updated
         with measured values.
         Done 2026-10-27: run outside this repository (an external
         benchmark on synthetic slots); figures in the ADR.
    ◆◆ Results to the user; then optimisation passes under the existing
       rules (results identical, at most five).

### Decisions by the user (2026-10-26, at approval)

1. **Microcontroller timing:** QEMU (an MPS2 Cortex-M33 model) for
   instruction counts now; no microcontroller speed claims until measured
   on a board, chosen later.
2. **Slot boundary on a 256 KiB part:** the small configuration plus a
   1 s i16 FIFO, with a 1 s microcontroller decode budget.

## D12: fix round after the decoder review (approved 2026-10-27)

Every item leaves decode results identical: golden decode hashes, pinned
rate floors, the bulk test and the golden waveform hashes are checked
after each item. An item that would change results stops the round and
is reported instead (expected exception: stronger golden-slot asserts may
expose a false decode, which is reported).

A. Required:
A1. [x] MSRV: `frontend.rs` `x.abs()` (core `f32::abs` needs 1.85) →
        comparisons; CI step `cargo +1.75 build -p cat-ft8
        --no-default-features` (library on the MSRV).
A2. [x] Provenance: `NOTICE` and the crate docs name the decoder stages
        for which `ft8_lib` was consulted (spectrogram oversampling, sync
        scoring, soft-bit normalisation, belief-propagation decoder; no
        function translated); ADR provenance table: 2× oversampling
        consulted, a row for the `math` helpers (log2, exp2), soft bits
        "scaled by mean magnitude".
A3. [x] Crate docs state the decode determinism condition.
A4. [x] ADR memory/flash table with measured values; measured rate and
        cost evidence recorded generically (external benchmark on
        synthetic slots; reference-decoder comparison outside this
        repository; CPU named); the host time budget states its statistic
        and load, and that it has no margin today.
A5. [x] Docs drift: README, CLAUDE.md, Cargo `description` and feature
        comments cover the decoder; profile-exposed modules marked
        unstable; `Decode::snr_db` (25–100 Hz); `Error` doc; decode bench
        header; `merge` doc; α 0.75 tie-break stated; comment that
        `Payload::from_bits(..)` after the CRC cannot fail; plan wording
        (Current Phase, D10 rule wording, D11 not part of CI); a `Decoder`
        lives across the whole slot; BP stopping at the first CRC-failing
        codeword added to the later single-pass list.
A6. [x] Tests: golden slots assert exactly the transmitted decodes per
        configuration and none on the carrier slot; `feed_i16(x)` equals
        `feed(x / 32 768)`; bulk run adds strong carriers, clipped
        full-scale noise, very low-level noise and impulsive noise;
        `llr_scale` bounded (0 < s ≤ 1 000) with a test.
A7. [x] CLI: `--config small` uses `Band::SMALL` and `SMALL_DECODES` and
        the profile reports band and capacity; `--help` prints usage to
        stdout and exits 0; the WAV reader reads only the chunks it needs
        and at most the samples asked for.
B. Simplicity:
B8. [x] Delete the `Level` trait and generics (`u8` levels only).
B9. [x] One per-symbol soft-bit fill and a shared metric-and-scale tail;
        `DATA_SYMBOLS` (= 79 − 21) instead of the literal 21.
B10. [x] Belief propagation reads a const 0-based check table with
         degrees, derived at compile time from `PARITY_CHECKS`.
B11. [x] One `classify` helper for sample sanitising and clamping in
         `Spectrogram::take`.
B12. [x] `Debug` for `Spectrogram` and `Decoder` without the buffers; the
         decode bench counts rounds only for converged candidates.
B13. [x] Profile split: refinement, soft bits, belief propagation and SNR
         timed separately (a stage callback on the candidate decode, used
         by `cat-ft8-decode --profile`).
D12a. [x] Cleanup after the D12 re-review (results identical except a
          new pinned golden entry): `Stage` matched exhaustively with
          `Stage::ALL` and `name()` (no catch-all; documented as for
          timing); `CHECKS`/`DEGREES` static; a third golden entry for the
          real small setup (Band::SMALL, SMALL, 64 decodes) with exact
          count and hash, the existing "SMALL" entry renamed min-sum on the
          default band, golden doc lines fixed; ADR/plan (D11 done outside
          this repository, stale plan line, flash figure with toolchain,
          time-budget sentence per process with parallel load as an upper
          bound, long lines, D12 sizes in findings, bulk-seed note);
          `NOTICE` names ADR 0024 not a path; stale 25–50 Hz test comment;
          bench prints n/a when nothing converged. Then all checks and the
          bulk run; then local commits by topic on `feat/cat-ft8`
          (authorised), no push.
C (after re-review, not in this round): up to five optimisation passes
with identical results, starting with radix-specific FFT butterflies.

## C: decoder optimisation passes (approved 2026-10-27)

Up to five passes, one change each, results bit-identical. Per pass:
`cargo test -p cat-ft8` (golden waveform and decode hashes, floors), the
external identity check on the crowded and sweep synthetic sets against
the d47eb5f baseline runs (`--identical`) and the golden-slot check. A
pass is kept only if its targeted CPU (a stage time from `--profile` or
`decode_bench cost`) or memory improves by more than 3% and nothing else
worsens by more than 3%, judged by interleaved A/B runs with a
loop-aligned build, and per-stage instruction counts (`perf stat -e
instructions:u` on a single-stage bench mode) where placement noise is
in doubt. Stop after two passes in a row under 3%. Rejected passes are
reverted and recorded. Each kept pass is its own local commit. Early
stopping or round caps change results and are not passes.

Candidates in profile order, decided from the baseline profile:
P1. [x] Belief propagation inner loops (same operations in the same
        order): layout, bounds checks, hoisting, the static check tables.
        Kept: sum-product check updates over fixed eight lanes with
        branch-free `exp2`/`log2` selects, so they vectorise; BP −35.5%.
P2. [x] FFT: radix-specific butterflies with incremental twiddle
        indices instead of the generic small DFT with a modulo per term,
        only if the rounding is identical. Radix-specific butterflies
        would change rounding (the table's quarter-turn twiddles are not
        exactly ±1, ±i), so only the incremental indices were tried:
        rejected, feed −2.9% (instructions −4.3%).
P3. [x] Belief propagation: swap the two total buffers by reference
        instead of by content. Rejected, BP −0.5%.
P4–P5. [x] Not run: two passes in a row under 3% (stopping rule).
C1. [x] Close-out after the pass reviews (results identical): pinned
        `exp2_neg` bit patterns and a belief-propagation state hash for
        both rules (each shown by mutation to catch a one-ulp change);
        `LANES` constant with a compile-time bound and `q` built in eight
        lanes; comments (scalar-FPU cost of the lanes, SMALL's rule to be
        re-checked with instruction counts, `math.rs` wording,
        `tanh_half`); bench `stage` usage and N = 0; progress corrections;
        ADR frame and crowded-slot time.
Later single-pass ideas, each a new hypothesis needing approval (not
started): an early stop driven by the movement of the soft totals rather
than the unsatisfied-check count; an early stop that allows longer
plateaus once the unsatisfied-check count is already low (the E1
losses converged after plateaus of 15+ rounds).
Later single-pass work (changes results, each measured): early stopping of
failing candidates, then sum-product versus min-sum as the host default;
continuing past a CRC-failing codeword; normalised sync threshold with a
smaller K; multi-pass API additions.

## Phase 3: single-pass improvements that change results — APPROVED (2026-10-27)

Status: **approved by the user on 2026-10-27**, revised as the architects
asked. Order: E0 → E1 → E3; E2 and E4 are measurements only in this
phase. A checkpoint follows every step; E0 is reviewed before E1 starts.

### Goal

Faster single-pass decoding on hosts (and evidence for the small-target
choice) through changes that alter decode results, each its own measured
change behind paired gates. Nothing here adds a pass, subtraction or
a-priori decoding. Speed is headroom for later work, so a lost decode
weighs more than a saved millisecond.

### Ground rules for every step

- **One change per step**, its own commit(s), progress and findings
  entries, and a checkpoint. A step that fails a gate or misses its cost
  threshold is reverted and recorded.
- **Paired results, saved.** Old and new decoders run on the *same*
  seeded slots. Results are saved per slot and per transmitted message,
  keyed by set and seed, so any later commit pairs against a saved
  baseline and the files are the evidence for re-pins.
- **Measurement sets (encoder-made, seeded; in the decode bench, not in
  `cargo test`):**
  - sweep: one signal per slot, random offsets, −22…−15 dB in 0.5 dB
    steps, 1 000 slots per point;
  - crowded: 10, 20 and 40 distinct messages per slot (−16…+6 dB),
    200 slots each, compared **per transmitted message** (each message
    of a slot is one pair; overlapping signals are separate messages,
    and a message counts as decoded when its payload is in the output);
  - edges: early and late signals (±1.6…±2.4 s), band edges, partial
    slots;
  - empty: the bulk inputs (noise, weak and strong carriers, clipped,
    floored, impulsive);
  - a **fresh seed block** of sweep and crowded, used only to confirm a
    chosen value;
  - both configurations (`HOST`, `SMALL`) unless a step names one.
- **Decode gate ("no loss beyond noise")**, per set and never traded
  between sets: with b = pairs decoded by the old decoder only and c =
  pairs decoded by the new only, b − c ≤ 2·√(b + c) over the set, and
  b − c ≤ 3·√(b + c) at every point (every sweep SNR, every crowded size,
  every edges case). The sweep's 50% point may not move by more than
  about 0.05 dB (bootstrap confidence interval reported).
- **False-decode gate:** zero on the empty set and in the bulk run;
  everywhere else the new decoder never has more false decodes than the
  old at any point. Every false decode, old or new, gets a findings
  entry. E0 records the baseline counts.
- **Choosing a value (k, K):** the value chosen on the main seeds is
  confirmed on the fresh seed block; if it does not pass there, the
  next more conservative value is taken (and confirmed the same way).
- **Cost:** times are the median of at least five runs (interleaved A/B,
  loop-aligned builds), alongside deterministic counts — rounds per
  candidate, candidates through belief propagation, image sizes, stack
  frames. Thresholds are minimums, each measured against the previous
  accepted step; host figures are provisional until measured on the
  target class. No other cost (time per stage, memory, flash, stack)
  may worsen by more than 3%.
- **External acceptance per step**, outside this repository.
- **CLI compatibility:** output format version 0 unchanged (same keys,
  same meaning); `--config host|small`, `--raw` and `--profile` keep
  their behaviour; the profile line may only gain keys. No new CLI flag:
  new settings are reached through `DecodeConfig` and the bench.

### Pinning

- The existing golden decode hashes and counts stay, as pins of the
  decoder with the new features **off** (stall off, 200 candidates).
  Tests run that setup explicitly.
- Each step that changes a preset adds pins for the **new defaults**
  (hashes, counts, floors) in the same commit, each commented with the
  step and date.
- Findings record, per golden slot and setup, the messages gained or
  lost and, for each lost message, whether it is truly lost or still
  decoded through another candidate; and any field that moved
  (frequency, dt, SNR, rounds).
- Floors for a new default are set from its measured counts; a floor
  below the "off" floor needs the paired evidence quoted in the commit
  message.
- The belief-propagation state hash pins both stall off (as now) and
  stall on, with the round at which it stopped.
- A pin changed without a findings entry is a defect.

### API decisions

- `DecodeConfig` gains `stall_rounds: u8` (0 = off) and
  `const fn with_stall_rounds`, validated (at most `max_rounds`).
- The unsatisfied-check count is computed only when stall is on, so the
  "off" path costs nothing extra.
- E3 changes a preset's `candidates` value, not the `CANDIDATES`
  constant (which stays the buffer size recommendation).
- A preset change updates the `HOST`/`SMALL` docs and ADR 0024 in the
  same step.
- `SMALL` may get its own k, marked provisional until instruction counts
  on the small target exist; it is enabled only if it passes its own
  gates.
- `bp::decode_soft` (profile-exposed, unstable) may change signature.
- `CandidateScratch::last_rounds()` (and `BpScratch::rounds()`) are a
  deliberate public addition (E0): a read-only count of the rounds the
  last candidate ran, whatever its result, for callers that measure or
  budget decoding cost; it costs nothing on the decode path.

### Steps

E0. [x] **Paired measurement tooling and baseline** (results identical).
        Bench modes that run two configurations on the same seeded slots
        of each set, save per-slot and per-message results keyed by set
        and seed, and print the paired tables (b, c, the sign-test
        verdict per point and per set), false decodes, the 50% point with
        its bootstrap interval, and the deterministic cost counts; a
        crowded-slot generator with ground truth; the fresh seed block.
        Baseline for `HOST` and `SMALL` saved and summarised in findings
        (50% point, crowded rates per size, edges, false decodes with an
        entry each, rounds per candidate, time).
    ◆ Checkpoint E0 (results identical; reviewed before E1).
E0a. [x] E0 corrections after its checkpoint (decoder results identical):
         `compare` fails on a (set, block) present on one side only
         (MISSING) or when nothing was compared; a missing 50% point
         fails the shift gate and dropped resamples are reported; exit
         status 1 on FAIL and on usage errors; a per-slot hash of the
         input samples saved and required equal; `decodes_truncated` and
         `candidates_dropped` saved and reported; `cost` spreads its slots
         over the three crowded sizes; `run` records commit, rustc and
         CPU; slot construction reuses the integration tests' helpers
         (the slot bytes change, so the baseline is re-recorded); a typed
         slot record with its own JSON writer; the statistics explained in
         the module docs; percentile sorts once and handles empty input.
         Baseline re-recorded with file hashes, toolchain, CPU, flash,
         `.bss` and stack in findings.
E1. [x] **Early stop of failing candidates.** Rejected (no stall
        setting passes the gates with a 20% saving); reverted in 42e551e.
        - Hypothesis: a candidate that will not converge shows it early —
          its unsatisfied-check count stops improving — so stopping after
          k rounds without improvement saves most belief-propagation time
          at a loss within noise.
        - Change: `stall_rounds` as above; belief propagation tracks the
          best unsatisfied-check count and stops when it has not improved
          for k rounds (within `max_rounds`); the failure is still
          `NotConverged`.
        - Measurement: k ∈ {2, 3, 4, 5, 6, 8, 10, off}, and fixed caps of
          10, 15 and 20 rounds for comparison, for both rules; every
          gate; rounds at which successful decodes converge; chosen k
          confirmed on the fresh seeds.
        - Acceptance: every gate, and belief-propagation time at least 20%
          lower on crowded slots for `HOST` than the E0 baseline. `SMALL`
          gets its own k (provisional) if it passes its own gates.
        - Pinning as above; the state hash adds the stall-on case.
    ◆ Checkpoint E1.
E2. [x] **Host check-node rule — measurement only.** (Measured at
        the E3 defaults: min-sum on `HOST` fails the no-loss gate on both
        blocks, 50% point +0.034–0.038 dB, total −24% per crowded slot;
        no default change. Findings, "Phase 3 E2".) With the accepted
        early stop, sum-product versus min-sum for `HOST`, each with its
        own k, paired on every set, with costs. The default does not
        change in this phase; the numbers go to findings for a later
        decision (assumed, not measured: sum-product is the better input
        for later ordered-statistics decoding). Re-measured after E3. (E1 was
        rejected, so there is no early stop: E2 is measured with stall
        off, at the E3 defaults — `HOST` 100 candidates, `SMALL` 75.)
    ◆ Checkpoint E2.
E3. [x] **Fewer candidates.** (`HOST` 100, `SMALL` 75; paired gates and
        comparison with an external reference decoder passed at those
        values.)
        - Hypothesis: most of the 200 candidates are noise; a smaller
          preset `candidates`, or a threshold on a normalised sync score,
          removes them without losing decodes.
        - Measurement, in order: `candidates` ∈ {200, 150, 120, 100, 75}
          with the accepted early stop; then, only if that alone does not
          pass, a normalised threshold (each candidate's score relative to
          a robust spread of the slot's local-maximum scores, cut-off
          swept). Chosen value confirmed on the fresh seeds.
        - Acceptance: every gate, and total host time at least 10% lower
          than after E1. `SMALL` follows only if it passes its own gates.
        - Pinning as above. After E3, E2's measurement is repeated.
    ◆ Checkpoint E3.
E4. [x] **Continuing past a CRC-failing codeword — measurement only.**
        (Measured: about 1.5 CRC failures per 10 000 candidates; after a
        CRC failure belief propagation never reached another codeword in
        1 964 continued candidates; not recommended. Findings, "Phase 3
        E4".)
        Results identical: count CRC failures per candidate and per slot
        on every set; for messages not decoded, how often a candidate on
        them failed by CRC; and, by running further rounds in the bench
        only, how many extra codewords would be reached and how many of
        them would be wrong. Expected false-decode rate stated (a random
        codeword passes the 14-bit CRC about once in 16 384). Any
        implementation is a later, separate decision with a much larger
        empty-set run.
    ◆ Checkpoint E4; ◆◆ results to the user.

## Phase 4: subtraction and further passes — APPROVED (2026-10-27, revised)

Status: **S0, S2 (with its fix round) and S3 done; S1 not needed; at
Checkpoint S3** (2026-10-28). (Before S2: S0 done, results identical, and revised after
review; S1–S3 then awaited the user's approval, given 2026-10-28.) No subtraction code is in the decoder.
The ADR amendment draft is in ADR 0024 ("Amendment (draft): subtraction
and further passes"). Revision: the bench was split and its reports
fixed (findings, "Bench split, and no-candidate misses told apart"), a
wider crowded set was added and measured (findings, "Wider crowded set,
measured"), the API was simplified, the step order changed (S2 before
any sub-sample start) and the five open decisions were made.

### Goal

More decodes in crowded slots by subtracting each decoded signal from
the slot's audio and decoding the residual again. Pass 1 stays exactly
as it is today; further passes only add decodes. `HOST` gets a pass count
above one only after paired acceptance; `SMALL` stays at one pass (the
residual does not fit a 256 KiB part).

### What S0 measured (findings, "Subtraction ceiling", "Bench split", "Wider crowded set")

The oracle passes remove what earlier passes decoded with the true
waveforms (the slot rebuilt without them, same noise) and decode again
with the same configuration. They are a **same-configuration oracle
reference**, not a bound: a real further pass decodes a different input
(residue, but also a different candidate list), and could in principle
decode more.

- **Crowded set** (10, 20, 40 messages, −16…+6 dB): every message decodes
  alone in its slot (same noise), so the ceiling (decoded by the single
  pass, alone or an oracle pass) is 100% for both presets and blocks;
  every signal is at least 3 dB above the single-pass 50% point. One
  oracle pass takes the host from 69.1% to 91.0% (main) and 91.6%
  (fresh); a second to 96.5% and 97.0%. `SMALL` is within 0.2 points.
- **Wider crowded set** (10, 20, 30, 40 messages, −22…+5 dB, its own
  seeds): here the ceiling binds — the host decodes 62.5% (main) and
  62.3% (fresh) in one pass, 81.8% and 82.0% with one oracle pass, 86.2%
  and 86.4% with two; 89.3% and 89.8% are decoded alone or by some pass.
  `SMALL` is within 0.3 points.
- **Misses by cause** (host, crowded main): of pass 1's 4 328 misses,
  3 486 have a non-converging candidate on them, 354 have no sync peak
  on them, 486 have a peak that the candidate limit dropped, and 2 a
  CRC failure. One oracle pass recovers 77% of the not-converged misses,
  60% of those with no peak and 38% of those dropped by the limit. On
  the wider set (host main) the same causes are 5 299 / 729 / 1 467 / 2.

### Ground rules for every step

As Phase 3 (one change per step, its own commits, findings and
progress entries and a checkpoint; paired results saved per slot and
per message; the measurement sets including the fresh seed block; the
no-loss and false-decode gates; chosen values confirmed on the fresh
seeds; cost as medians of at least five interleaved runs of loop-aligned
builds, plus deterministic counts), with these additions:

- **Pass 1 is untouched.** With one pass, every output, pin and hash is
  bit-identical to today's; each step proves it (release binary,
  golden hashes, `paired_bench compare` against the single-pass runs
  with b = c = 0).
- **Oracle reference.** Every paired result on the crowded and the
  wider set is also reported as the share of the same-configuration
  oracle's gain it recovers (real gain over the oracle's +1 or +2 gain,
  for one or two further passes) and against the ceiling. A share above
  100% is possible (a real pass decodes different input) and is
  reported as measured, not capped. The oracle is re-run with `ceiling`
  at whatever per-pass configuration S3 adopts (its later-pass candidate
  count), so the share always compares like with like.
- **No CLI flag** (superseded 2026-10-28: a flag was requested,
  `--config host,passes=N`; output keys unchanged, profile keys added).
  The pass count is reached through `DecodeConfig` and
  the bench; output format version 0 keeps its keys; the pass number of
  a decode and every per-pass count appear only in profile and bench
  output.

### API (to settle at approval)

- **Pass 1 unchanged**: `start_slot`, `skip`, `feed`, `find_candidates`,
  `decode_candidate`, `DecodeList::push`/`merge` keep their signatures
  and behaviour.
- **The residual** is the caller's: always the **full slot from slot
  sample 0**, with the skipped region (the first `skip` samples) zero;
  the caller feeds `&audio[skip..]`. A buffer shorter than a slot is a
  slot cut short, as with `feed`.
- **Each later pass** uses the existing calls: `start_slot()`,
  `skip(skip)`, `feed(&residual[skip..])`, then `find_candidates` and
  `decode_candidate` as in pass 1, into the same `DecodeList`. There is
  no `start_pass`.
- **`decode::subtract(audio: &mut [f32], decode: &Decode, band: Band)`**
  (a free function, plus any setting S2 shows it needs) subtracts one
  decoded signal from the residual in place. It reads only `audio`, the
  decode and its arguments (no spectrogram), uses bounded stack storage
  only (no new workspace buffer) and is deterministic. It becomes a
  `Decoder` method only if S2 shows it needs decoder state.
- **`DecodeList::next_pass()`** only resets the candidate-index order
  check, so the new pass can push from index 0. The caller records the
  list's length before each pass to know where the pass's entries begin
  (bench and profile use only). Duplicate removal already compares
  against every entry, so a payload decoded in an earlier pass is a
  duplicate, never output twice.
- **Fixed subtraction order**: after pass p, the decodes of pass p are
  subtracted in list order, on one thread. So the split/threaded API
  stays bit-identical to one thread: candidates may be decoded in
  parallel, subtraction is sequential.
- **`DecodeConfig::passes: u8`**, validated 1..=3 (1 = today;
  `with_passes`), and one candidate count for every later pass
  (`with_later_candidates`, default to be chosen in S3; the candidate
  buffer needs the larger of the two counts). Presets keep one pass
  until S3's acceptance.
- **`Decoder::decode_passes(&mut self, audio: &mut [f32], skip: usize,
  scratch, list)`** runs the whole loop itself, pass 1 included: for
  every pass `start_slot`, `skip`, `feed(&audio[skip..])`,
  `find_candidates`, the decodes, and (before a further pass) the
  subtraction of that pass's decodes. It **overwrites the caller's
  buffer with the residual**: on return `audio` holds the slot minus
  every decode of every pass but the last. A pass that adds no new
  decode ends the loop early (deterministic). With one pass it is
  `start_slot`, `skip`, `feed`, `decode` and leaves `audio` unchanged.
- **`report()` after several passes.** `Decoder::report(&list)` keeps
  its meaning — the last search and the last input fed, with the list's
  counters — so after several passes it describes the last pass, not
  the slot. The multi-pass report is defined as pass 1's report (input
  counters, candidate counts, rejection and duplicate counts, all from
  pass 1, as before) with `decoded` and `decodes_truncated` taken over
  the whole list (the output); `decode_passes` returns exactly that. A
  caller driving the passes by hand takes `report()` after pass 1 and
  the list's length at the end. Per-pass candidate, decoded, rejection
  and cross-pass duplicate counts go to profile and bench output only.
- **Output format**: unchanged keys. `cat-ft8-decode` holds the residual
  (it already reads the WAV into memory) and uses the configuration's
  pass count; `--profile` gains per-pass keys (`pass<N>_*_seconds`,
  per-pass counts), as profile keys may only be added.

### Decisions (made 2026-10-27)

1. **Amplitude**: the reference is built at unit amplitude and scaled
   outside `Synth` by S2's estimated complex amplitude. `Synth` gets no
   amplitude parameter.
2. **Fading and drift**: a fading/drift set is added to the bench in S2
   (amplitude fading at stated rates, slow frequency drift, starts
   between samples). It is measured in S2 only; in S3 it is gated on
   false decodes and duplicates, not on no-loss.
3. **Cost**: one total budget per slot, not one per pass. A single pass
   keeps ADR 0024's mean of ≤ 100 ms per crowded slot. Multi-pass: the
   worst 40-signal slot ≤ 250 ms in total, the mean recorded. The
   statistic is the maximum, over every 40-signal slot of the crowded
   set's main block (all 200, not a subset), of that slot's median
   across at least five interleaved runs, one thread. Alongside it, the
   deterministic bound per pass: candidates × 30 rounds of belief
   propagation, plus one `feed` of the slot, one sync search and the
   subtraction of the previous pass's decodes.
4. **A wider crowded set**: added and measured (`wide`: −22…+5 dB, 10,
   20, 30 and 40 messages per slot, its own seeds; existing sets
   unchanged). It is measured only, with no gate of its own, and its
   measurement is a **precondition of S3's acceptance**.
5. **Passes**: validated 1..=3, with the deterministic early stop when
   a pass adds nothing. One candidate count applies to every later
   pass; it is swept and confirmed on the fresh seeds.

### Steps

Order: S2 first, with integer-sample starts; S1 only if S2 shows it is
needed; then S3.

S0. [x] **Clean-up and the subtraction ceiling** (results identical).
        E4's continuation code deleted (1870e8b reproduces it); misses by
        cause and CRC-based false-decode estimates in `run`/`compare`;
        `paired_bench ceiling` (alone decodes and oracle passes) on the
        crowded set, both presets, both blocks. After review: the bench
        split into modules, no-candidate misses told apart (no sync peak,
        peak dropped by the candidate limit), the ceiling column includes
        the oracle passes, and the wider crowded set measured. Findings:
        "Misses by cause", "Subtraction ceiling", "Bench split, and
        no-candidate misses told apart", "Wider crowded set, measured".
    ◆ Checkpoint S0: this plan to the user for approval.
S2. [x] **Subtracting one signal** (integer-sample starts).
        - Change: `decode::subtract` as above: from the decode's tones,
          frequency and time offset, a fine time estimate (rounded to a
          whole sample) and a fine frequency estimate on the residual,
          then a smoothed complex amplitude (the residual times the
          conjugate unit reference from `fill_phase`, averaged per
          segment and interpolated), then the subtraction of the scaled
          reference. Segment length, smoothing and estimator chosen by
          measurement. The fading/drift set is added to the bench.
        - Measurement: **residual suppression in dB** (residual energy
          over signal energy in the signal's band and time span) per
          condition — SNR (−20…+20 dB), dt (−2…+2.5 s, including
          sub-sample offsets the integer start cannot follow), frequency
          (band edges, fractional bins), fade rate and drift — reported
          as median and spread per condition, beside the true-waveform
          subtraction of S0; the timing part of the residue (the same
          estimate with the true start) reported separately. Then, in
          the bench only (no `DecodeConfig::passes` yet), one and two
          further passes on the crowded, wider and fading sets with the
          existing calls: decodes gained, false decodes, and **duplicates
          suppressed per pass** — the main signal of subtraction quality,
          since a later pass that re-decodes a subtracted payload means
          the subtraction failed. Cost per subtraction.
        - **No pass/fail bar on suppression.** S2 only reports: what
          matters is the paired gain in S3, and a suppression threshold
          set before measuring would be arbitrary. The one decision S2
          makes from its numbers is S1's: if the timing residue is
          above −30 dB in any condition, S1 is done; otherwise it is
          not. Hard requirement: no change to any pass-1 output.
        - Done (2026-10-28; findings, "Subtracting one signal (S2)"):
          `decode::subtract(audio, &decode, skip, &SubtractConfig)` with
          `SubtractConfig { segment, drift }` (default one symbol, drift
          on) and a `Subtraction` report with its own `left_db`
          estimate. The plan's `band` argument was not needed; `skip` was
          (the skipped region is neither read nor written). Fine time
          from adjacent symbols' phases per tone step, frequency and
          drift from the same, a least-squares amplitude per segment,
          linear between segment centres. The `fading` set and the
          `suppression`, `passes` and `subcost` modes added to the bench;
          `subtract_wav` example. Single signals: noise-limited residue
          (−27.7 dB at 0 dB SNR, −37.8 at +10), the same at every start
          offset, frequency and drift up to 0.2 Hz/s; fades up to 0.5 Hz
          followed. Further passes (bench loop, host main): share of the
          oracle's gain +1 / +2 crowded 96.6% / 94.8%, wide 93.0% /
          89.5%, fading 94.9% / 91.6%; duplicates 1 of ~10 000
          subtractions after one pass, 0–0.4% after two; no false
          decodes. Cost 3.3 ms per subtraction (one thread). Pass 1
          identical (records, golden pins, decode output of the release
          binary; its bytes moved, see findings).
    ◆ Checkpoint S2 (2026-10-28). **S1: not needed** — the sub-sample
      start's share of the residue is −57.7 dB at a half-sample start
      and −53.3 dB at worst in any trial, far below −30 dB. Open for S3:
      the segment length (1 920 vs 3 840: +2.4 / +3.4 points of oracle
      share on crowded for 4–36 duplicates) to sweep with the later-pass
      candidate count; the subtraction's cost against the 250 ms budget.
S1. [—] **Reference waveform: only what S2 needs** (conditional; not
        needed — S2 measured the timing residue far below −30 dB).
        - Rationale: half a sample of start error (τ = 1/24 000 s) is a
          phase error between tones of 2π·Δf·τ once the complex
          amplitude has absorbed the common part — at most 2π × 43.75 Hz
          × τ ≈ 0.011 rad across the eight tones, about −39 dB of
          residue, of the same order through each tone change. That is
          likely below what amplitude estimation in noise leaves, so
          integer starts are expected to suffice; S2 measures it.
        - Change, only if S2 measures timing residue above −30 dB:
          `Synth` gains a sub-sample start (a fraction of a sample in
          [0, 1)), additively. No amplitude parameter (decision 1).
        - Determinism: with a zero fraction every existing output is
          bit-identical (the waveform golden hashes and the `ft8sim`
          black-box vectors unchanged); non-zero fractions get new pins,
          checked on every CI target.
        - Tests: a fractional start equals the integer-start waveform
          resampled (within a stated tolerance), continuity across block
          sizes, phase at the start, `no_std` and the MSRV build.
    ◆ Checkpoint S1 (if done); S2's suppression re-measured with it.
S3. [x] **Further passes, with paired acceptance.**
        - Change: `DecodeConfig::passes` and the later-pass candidate
          count, `DecodeList::next_pass`, `decode_passes`, the CLI's use
          of the pass count, golden pins for multi-pass setups; `cost`
          gains the per-slot statistic of decision 3.
        - Measurement: passes ∈ {2, 3} on every set, both blocks (edges
          too), plus the wider and fading sets, against the single-pass
          runs; the later-pass candidate count swept, the chosen value
          confirmed on the fresh seeds; `ceiling` re-run at the adopted
          configuration for the oracle share.
        - HOST's default changes only after acceptance; SMALL stays at
          one pass.
        - Done (2026-10-28; findings, "Further passes (S3)"): the API as
          above plus `find_later_candidates`, `decode_passes_staged`
          with `PassReport`; the decode command's `passes=N`; the bench's
          `run`/`compare` per-pass records and gates, `slotcost`, `passes`
          on `decode_passes`. Sweep: segment 1 920–3 840 × later 20–200;
          chosen three passes, 50 later candidates, 3 840-sample segments
          (fresh seeds agree), adopted in `HOST`. Acceptance: every gate
          passes (crowded +3 767 / +3 853 with no loss, zero false decodes
          per pass on every set, expected false decodes within passes ×
          single, sweep interval unmoved); oracle share +1 / +2 98.9% /
          98.2% crowded main, 96.9% / 94.8% wide main; worst 40-signal
          slot 213.7 ms, mean 197.5 ms. Pass 1 identical.
    ◆ Checkpoint S3 (2026-10-28); ◆◆ results to the user.

### Acceptance bars (in this repository)

- **Target:** on the crowded set, paired gains − losses ≥ 2·√(gains +
  losses) on each block. The headline is the share of the
  same-configuration oracle's +1 and +2 gain recovered, on the crowded
  and on the wider set (reported, may exceed 100%), with the ceiling
  beside it.
- **False decodes:** zero on crowded, sweep and edges (both blocks), per
  pass, and on the fading set; still none on the empty set and in the
  bulk run (which alone cannot test later passes: with nothing decoded
  in pass 1 there is nothing to subtract and the early stop ends the
  loop). The expected false decodes from the CRC (CRC failures × 2⁻¹⁴
  per slot) at most passes × the single-pass rate, reported per pass. A
  duplicate in the output is a defect.
- **Other sets:** no loss beyond noise (the Phase 3 gate, per set and
  per point). Sweep: the 50% point's shift and its whole bootstrap 95%
  interval within ±0.05 dB (`compare`'s sweep gate is extended to the
  interval in S3).
- **Wider set:** measured and reported (gain, losses, share of the
  oracle's gain, false decodes, duplicates); no gate of its own, but S3
  is not accepted without it.
- **Cost:** decision 3 — single pass mean ≤ 100 ms per crowded slot
  (unchanged); multi-pass worst 40-signal slot ≤ 250 ms in total (the
  per-slot median statistic), mean recorded; per-pass stage costs (feed,
  sync, belief propagation, subtraction) and the deterministic bound
  recorded.
- **Memory:** the core's working memory and the caller's residual
  reported separately; flash growth on thumbv8m reported from the link
  check (a host image that calls `subtract`).
- **Determinism:** new multi-pass golden pins (hashes and counts); bit
  for bit identical on every CI target (Linux, Windows, aarch64); the
  split/threaded path equals one thread; the single-pass pins unchanged.

### Memory

- The residual is the caller's: 180 000 `f32` samples, 720 000 B on a
  host, beside the core's 234 680 B. The core gains no buffer;
  `subtract` uses a bounded stack frame (reported).
- **Multi-pass gives up block-by-block streaming.** Pass 1 can still be
  fed as the audio arrives, but every later pass needs the whole slot at
  once, so multi-pass decoding is a host feature. A 256 KiB part cannot
  hold the slot (an `i16` residual is still 360 000 B), so `SMALL` stays
  single-pass; the API is in the `no_std` core, and an image that never
  calls it links none of it.

### Risks

- **Fading and drift.** The crowded sets have constant amplitude, exact
  frequencies and integer-sample starts; real signals fade, drift and
  start between samples. Amplitude tracking (the smoothing length)
  trades noise against following a fade; a poor track leaves residue
  that can mask weaker signals or, at worst, decode as something. The
  fading/drift set (decision 2) measures it.
- **Speed ratio to small targets** is unmeasured: the per-pass cost is
  known only on this host. Nothing in this plan targets a
  microcontroller.
- **The oracle is a reference, not a promise.** It removes true
  waveforms; S2's estimates will leave residue, so S3's gain is expected
  below the oracle's, though a share above 100% is possible.
- **Order effects.** Overlapping signals bias each other's amplitude
  estimates; the fixed order keeps results deterministic, not optimal.

### Ideas for a later optimisation step (not in S1–S3)

- Recompute only the spectrogram bins a subtraction touched (the signal's
  band plus the window's spread, over its rows) instead of re-feeding the
  whole slot; this changes rounding, so it would be measured as a
  results-changing step.
- Subtracting during pass 1 as each decode arrives (not possible with
  the fixed-order rule as written).

## S3 fix round (2026-10-28, after the S3 checkpoint)

Results stay identical: every change is result-neutral, or opt-in with a
default that keeps today's output. Proof: pass-1 and multi-pass pins
(only report-semantics changes allowed, each explained); `paired_bench
run host` records equal to S3's on every set with timings removed (the
slot-level counts checked against the per-pass sums); the decode
command's output on the 800 WAV slots equal to cea0bc2's but for the
added `pass` field and the changed profile keys.

1. `Synth`: the phase increment through `i32`, not `i64` (a library call
   on Cortex-M); own commit; `__aeabi_f2lz` call sites back to d4fd2d3's;
   `subcost` of the cast alone.
2. One meaning for the multi-pass report: a new `PassesReport` whose
   `DecodeReport` holds whole-slot totals (search, list and rejection
   counts summed over passes; input counters from pass 1's feed), with
   `passes_run` and why the passes ended; per-pass detail stays in
   `PassReport`. Bench and `--profile` show the totals.
3. `"pass": N` on every decode line of `cat-ft8-decode` (format stays 0).
4. `candidate_capacity()` = max(candidates, later_candidates); no clamp in
   the search.
5. `DecodeList` counters as one `Copy` struct; the per-pass difference
   one method.
6. Count only successful subtractions (`debug_assert`), failures in
   `PassReport::subtract_failed`.
7. Decode command profile: fixed arrays, names only when printing.
8. `Decoder::decode_further_passes[_staged]`: passes 2.. after a pass 1
   run with `decode` (for example fed block by block with `feed_i16`);
   `decode_passes` = pass 1 + this. Test: `i16` blocks then further
   passes on `f32 = i16 / 32 768` equals `decode_passes` bit for bit.
9. Budget: the event callback returns `ControlFlow<()>`, checked between
   passes and between subtractions; `DecodeConfig::max_subtractions`
   (default none) stops before a pass whose subtractions would exceed
   it. The report says which.
10. Stop after a pass that leaves the list full; `max_decodes()`.
11. `Subtraction` non-exhaustive; `Stage`/`PassStep` documented as a
    profiling surface that grows.
12. `MAX_SEGMENTS` with `div_ceil`.
13. Tests: threaded path with nonzero skip comparing every report field;
    audio longer than a slot and skip past its end; nonzero earlier
    duplicates; the forward fill of a segment with no usable samples.
14. Full paired acceptance re-recorded at a clean commit.
15. Open-source wording in the planning files, NOTICE, crate docs, ADR.
16. `slotcost` again: worst 40-signal slot ≤ 250 ms.

Done 2026-10-28 (00c7cbe, 0d3c912, 3ed1aa0; findings, "S3 fix round").
`check.sh` keeps allowing `__aeabi_f2lz`: `subtract` and
`Placement::slot` need the `i64` range.

## O1 fix round (2026-10-29)

Results stay bit-identical to 36cc440 (pins, multi-pass pins, `run`
records with timings removed, the decode command on the 800 WAV slots);
the O1 speed is kept and re-measured.

1. Caller-provided subtraction scratch: `SubtractScratch` (`const fn
   new`) for `subtract` / `subtract_fixed`, `PassesScratch` (candidate +
   subtraction) for `decode_passes*` / `decode_further_passes*`; not
   folded into `CandidateScratch`; not cleared per call.
   `SUBTRACT_STACK_BYTES` back to 4 536 B. CLI, bench, examples, tests,
   embedded check, ADR.
2. Docs: memory and stack on `decode_passes`, `decode_further_passes`,
   `DecodeConfig::passes` and `with_passes`; ADR 0024 link-check figures.
3. Subtraction anchors in the repository: hashes recorded with 36cc440
   (segments 4 080, 7 680, the whole waveform and 3 840; skip 30 000;
   cut 90 000; drift on and off); the held/rerun test adds 4 080. Subcost
   at segments beyond two symbols, base against final.
4. FFT: the finite, bounded input precondition stated; the reference
   test adds subnormals, signed zeros and full-scale coherent input.
5. `Synth::increment`: `debug_assert!` below 2³¹, the margin tested.
6. AArch64 at run time: the release test suite and a `run` identity
   block on a Raspberry Pi 5, records compared with x86-64.
7. The record identity pass from the committed final tree.
8. `slotcost` and `subcost` on both hosts, base and final in one session.

Done 2026-10-29 (59bb2c7, 2dd54dc, c3f080f; findings, "O1 fix round").
Item 3's subcost found the second reference run (segments beyond two
symbols) about 12% slower than at 36cc440; c3f080f fixes it.

## O2: second decode-time round (2026-10-29)

Same ground rules as O1: results bit-identical to the O1 fix round (pins,
multi-pass pins, the 24 subtraction anchors, `run` records with timings
removed, the decode command on the 800 WAV slots), the same `f32`
operations in the same order, no new soft-float helpers on Cortex-M; up
to five steps, each timed on x86-64 and the Cortex-A76 against the one
before and kept only if it wins on one without losing on the other.

1. Profile first (`perf` on x86-64; stage timers and `subcost` on the
   A76) and pick targets from it.
2. Steps kept: the reference generator in runs per symbol part
   (08d5d8e); `exp2_neg`'s floor without a conversion on x86-64
   (fbc5857); sync scored a start row at a time (27bb0d5); the
   generator's increments without a conversion on x86-64 (d6bab4f).
   Rejected: the envelope-multiply skip with an `f32` offset counter
   (slower on both hosts); step 2's floor on AArch64 (no gain).
3. O1 follow-ups (b39ce66): per-image static RAM
   ceilings in `check.sh` (small and encoder-only 256 KiB, host 512
   KiB); `peak_bytes` documented as leaving out the subtraction scratch
   (key unchanged); `PassesScratch` steered to a `static`; the
   `Removal::refs` wording; the 7.4 KB and 14 KB frame figures told
   apart.
4. Identity at every step's clean commit; the release test suite and a
   `run` identity block on the A76 at the final commit; the release
   decode command's output on the 800 WAV slots.

Done 2026-10-29 (findings, "Decode time, optimisation round (O2)").

### O2 fix round (2026-10-29)

Results bit-identical to b465b75 on every target, as in O2.

1. One generator: `fill`, `mix` and `skip` through `fill_phase` /
   `skip_with` over a 64-sample stack chunk; the per-sample `run` and its
   stale timing comment deleted; `increment` and `increment_lanes` one
   helper with one cfg site. Encoder output proved equal (tests against
   the per-sample reference; 1 000 cases of `synthesize`, `fill`, `mix`
   and `skip` hashed against 0186703); encoder timed before and after;
   flash, frames and static RAM per image on thumbv8m and thumbv7em.
2. Docs and tidying: `costas_score`'s doc back in place; `present`
   counted in `costas_scores`' main loop; `floor_small` domain doc and
   `debug_assert!`; check.sh's RAM ceilings in one table.
3. Tests: "every tone triple" made true with de Bruijn B(8,3) windows;
   `increment` against the original `as i64` truncation; the exhaustive
   `_everywhere` tests in CI's release job on x86-64.
4. Identity and timing at the final code commit (69c5bd6): tests,
   clippy on four targets, `run` records, the 800-WAV output, x86
   `slotcost` against 0186703, the A76 test suite.

Done 2026-10-29 (findings, "O2 fix round").

### Open items

- **Cortex-M run time of `fill_phase` and `find_candidates`** (an O2
  follow-up before calling the round done for microcontrollers).
  Not attempted: no Cortex-M board was available.
  Steps 1 and 3 of O2 and the chunked encoder change the code shape
  there; only flash and frames are measured.

### Open API question (settled in O3)

`PassesScratch` held the candidate scratch and the subtraction scratch
together. Since 81db219 the decoder's subtractions use the spectrogram's
idle buffer, `decode_passes*` take the `CandidateScratch` alone, and
`subtract` takes any byte buffer: there is no combined type.

### Proposal for a gated round: `i16` residual

Measured in O3 (findings): the multi-pass residual held as 16-bit samples
halves the caller's 720 000 B; no measurable decode change on the run
sets or the 800 WAV slots. Changes results and the residual's type, so
it needs its own plan and acceptance round.
A proposed gate for it is in findings.md, "Proposed: `i16` residual,
gated" (O10; proposed, not yet run).

## O3: CPU and memory per decoded signal (2026-10-29)

Same ground rules as O1 and O2: every committed step bit-identical to
cc68e22 on every target (tests and pins, the 24 subtraction anchors, the
exhaustive tests, `run` records with timings removed, the decode
command's 800-WAV output). Targets now include memory as well as time:
peak working memory, static RAM per image, stack. Up to five steps, each
its own commit with before/after numbers on x86-64 and the Cortex-A76;
rejected attempts recorded with numbers.

1. Profile first: `slotcost`/`subcost` stage times on both hosts, `perf`
   on x86-64; memory from `check.sh --stack`, `.bss`/`.data` and the
   profile keys.
2. Candidate steps (from the profile): belief propagation over the 522
   edges rather than 83 × 8 padded lanes; the time search's rotations
   hoisted out of the groups; the subtraction's sine and cosine from one
   table read per value; the multi-pass subtraction's held reference in
   buffers idle between passes (no `SubtractScratch` in the decoder).
3. Changes that would alter results (a narrower residual, fewer
   spectrogram rebuilds, frequency-domain subtraction) are prototyped
   only (not committed) and written up as proposals for a separate gated
   round.

Done 2026-10-29 (findings, "CPU and memory per decoded signal (O3)"):
93fa40d (rotations once per trial) and 81db219 (the subtraction's
reference in the spectrogram's idle buffer; `subtract` takes any byte
buffer; `PassesScratch` and `SubtractScratch` removed). Rejected with
numbers: belief propagation over the 522 edges (x86-64 −14% BP, A76
+2.3%), a shared cosine/sine decode (slower on both), two other forms of
the hold's write loop. Proposal for a gated round: an `i16` residual
(720 000 → 360 000 B per stream; no measurable decode change in the
prototype).

## O3 fix round (2026-10-29)

From the O3 follow-ups; every step bit-identical to cc68e22 on every target
(the only visible change: no candidates after a stopped decode).

1. A stopped decode leaves the decoder as after `start_slot`; the level
   buffer taken once per pass's subtractions.
2. Tests: narrow band with an exact and a padded level buffer; the
   scratch-size test over the skip/cut/dt matrix.
3. RAM ceiling less a stated stack budget; `SUBTRACT_STACK_BYTES` with
   the rotations; docs (importable names, narrow-band threshold, check
   order).
4. One timing (`held` sliced once), kept only if no worse on the A76 and
   clearly better on x86-64.
5. Out of tree: a hotspot profile of a three-pass slot on the A76, and
   the share of passes 2–3 BP runs repeating a failure on identical LLRs.

Done 2026-10-29 (findings, "O3 fix round"): 37aa031, 13f88ed, b37df17;
the slice rejected (no gain on the A76). Profile: subtraction 57% of the
A76 slot (fit loop 25%, removal 13%, time search 13%), belief
propagation 25%, feed 14%. BP repeats on identical LLRs: 1.4% of rounds,
not worth a skip. Next CPU targets: the fit and removal loops and the
reference's cosine/sine (12% of the slot).

## O4: CPU per decoded signal, fourth round (2026-10-30)

Bit-identical to 91ee9fe; the ranked O3 targets, in order: a joint
cosine/sine, an `input()` fast path, the belief-propagation edge layout
(only in a materially different form from O3's), small items (the
envelope multiply in the time search's middles, an exact float counter
in the removal), and an O3 follow-up (one helper lending the level
buffer). A76 decision rule as in the O3 fix round; re-profile at the end.

Done 2026-10-30 (findings, "CPU per decoded signal, fourth round"):
872bd59 (middles skip the envelope), 657543e (BP edges grouped by
degree: BP −3% A76, −17% x86-64; frame −568 B), 66ba24b (helper).
Slot A76 203.4 → 201.2 ms (−1.1%), x86-64 114.3 → 110.3 (−3.5%).
Rejected: joint cosine/sine (+14.6% `subcost` on the A76), `input()`
fast path (+0.4%), float counter (+5.8%). Remaining bit-identical gains
judged ≤ 2% each.

## O5: CPU per decoded signal, fifth round (2026-10-30)

The last bit-identical round (identical to 46fdf4d) before the gated,
result-changing rounds; from the open items at the end of round O4, listed
below. A76 decision rule as before.

1. The decoder's subtractions without the report's sums (a
   crate-internal entry; the public `subtract` and its anchors
   unchanged); check whether the fit's `xx`/`n` are dead.
2. Belief-propagation messages kept in the degree-grouped edge layout,
   the round in place (no copy, clearing or scatter; one cursor).
3. Tests: a structural edge-map test and a bit-for-bit comparison with
   the round's first form over random states.
4. The time search's symbol cores named and asserted against the
   ramps; the waveform-length and generator-start assumptions stated.
5. Docs (ADR 0024 flash figures, the O4 notes' wording).
6. Cortex-M33 evidence for 657543e (static instruction counts).
7. Out of tree, for the gated rounds: cost and decodes per pass,
   distance of later-pass decodes from the previous pass's, and how
   often the time search repeats.

Done 2026-10-30 (findings, "CPU per decoded signal, fifth round"):
f0bb4ab (subtractions without the report: A76 slot −7.0%, x86-64
−5.8%; a `const` generic form was +4% on x86-64 and rejected; the
fit's `xx`/`n` were thought removed by the compiler: true only where
the fit closure is inlined, corrected in O6), 02ff55e (messages
in the edge layout: time unchanged, static RAM −236 B, round frame
2 176 → 88 B), 92970ba (tests), 1b47a55 (symbol cores), a5a2034 (docs).
Slot A76 201.2 → 187.5 ms (−6.8%), x86-64 110.3 → 104.2 (−5.5%).
M33: 657543e removes 21% of the transcendental evaluations and about
15% of the round's instructions (static). Measurements: pass 3 is 26%
of the A76's multi-pass time for 4.7% of the decodes (corrected in O6); 99.8% of pass-2 and
99.7% of pass-3 decodes lie within 50 Hz of a decode of the pass
before (a region of 45% / 16% of the band); the time search repeats in
0.14% of subtractions.

## O6: CPU per decoded signal, sixth round (2026-10-30)

From the open items at the end of round O5 (1. the fit's sums and
correction in locals; 2. a sine table padded by its end point; 3. doc
fixes; 4. out of tree, for the gated rounds: a frequency gate on the
later passes and a pass-3 trigger); bit-identical to 3f16549. Done (findings, "sixth
round"): 1d028a1 (fit sums in locals: time unchanged on A76/x86-64,
kept for the Cortex-M33), padded sine table rejected (A76 +2.7%
subtraction), 73ebf5d (docs). Gated-round measurements: the later-pass
cap is the lever (50 → 20: −10.4% A76 for 3 lost of 42 433); the
frequency gate saves nothing as specified; pass-3 trigger k = 2 adds
−4.6 ms for 27 lost.

## O7: release profile and the later-pass cap (2026-10-30)

From the open items at the end of round O6 (the release build profile;
a doc and a comment fix; a pin for the report's projection; out of
tree, the later-pass cap); bit-identical to c472100. Done (findings,
"seventh round"): release profiles measured (none wins by 2% on the A76;
defaults kept), 5704d32 (doc and comment), a437134 (projection pin),
notes corrected. Out of tree for the gated round: later-pass decodes'
ranks by load, the S3 sweep reconciled with O6 (d), fixed caps and the
proportional rule on three blocks. Recommended: later-pass cap 25.

## O8: later-pass cap 50 → 25, gated (2026-10-30)

The first result-changing round since S3. One code commit:
`HOST.later_candidates` 50 → 25, its doc (the measured cost: the loss
sits in 40-signal `fading`/`wide` slots, at the edge of the validated
40; callers raise it with `with_later_candidates`) and the pins that
move with it. `SMALL` runs one pass and must stay byte-identical.

1. Gate criteria written in findings.md before the acceptance run
   (below and findings, "eighth round").
2. Acceptance on the `fresh` block (crowded, wide, fading; never used
   for the cap measurements, which used `main`, `cap1` and `cap2`), the
   paired gates on every set of both blocks, the 800 WAV slots, the
   ceiling run and the A76 timing. If any criterion fails: no commit;
   report and re-measure, do not retune.
3. Text: O7 notes' corrections; the planning files' wording for
   material not in this repository; a boundary test for `Fit::det` if
   it changes no result.
4. After the cap: a fresh A76 profile of the three-pass slot (to choose
   between the `i16` residual and vector sine/cosine rounds), local CI
   and the Raspberry Pi 5 release suite.

Gate criteria (fixed in advance; every one must pass):
- Sets: `crowded`, `wide`, `fading` on the `fresh` block (used once for
  the verdict), the existing paired gates (`compare` against the cap-50
  and single-pass records), the 800 WAV slots.
- False decodes: 0, absolute, everywhere.
- Pass 1 byte-identical (`host,passes=1` and `small` records and WAV
  output; pass 1 of every `host` record); only passes 2 and 3 may differ.
- Lost: each a pass-2/3 decode ranked beyond 25 at cap 50, each logged;
  at most 8 on the `fresh` block (above 8: stop, re-measure, no retune).
- Gains and every 800-WAV difference listed and explained.
- Ceiling run (oracle passes) and the `wide` set's low-SNR messages
  (below −16 dB): loss rate at most the per-set rate on `main`/`cap1`/
  `cap2` (crowded 0, wide 1 of 50 996, fading 9 of 35 888).
- Speed: A76 three-pass slot at least 7% faster than ce09c5d, same
  hygiene (one source path, a target directory per commit, hashes
  checked, alternated rounds, throttle flags before and after).

Outcome (findings, "eighth round"): **not adopted**. Every criterion
passes but one: of the 3 messages lost on `fresh` (bound 8), one ranked
4 in pass 3 at cap 50: it sat 29 Hz from a pass-2 decode of rank 35
that the cap pushes into pass 3, so it was never searched without that
signal. A76 −8.70% per slot, no false decode, no gain, pass 1
unchanged. The candidate stays on a local branch (0f0341c); `HOST`
keeps 50. Whether a knock-on loss of this kind is acceptable is a
decision for the next round's criteria, not a retune of this one.
Text items and the `Fit::det` boundary test done (a4fe8e6, 1c1c8a8).

## O8b: re-gate of the later-pass cap on a new block (2026-10-30)

0f0341c unchanged, tested once on the unused block `accept2` (crowded,
wide, fading; ceiling mode) against ce09c5d, under a criterion that
names the knock-on loss (findings, "re-gate on a new block"; fixed
before any run). Pass: 0f0341c joins the branch with the doc naming
knock-on losses, then identity, local CI and the Raspberry Pi 5 suite.
Fail: the cap is dropped; no third block.

Outcome (findings, "re-gate on a new block"): **fail; the cap is
dropped.** Lost 5 on `accept2` (pooled 8), every loss explained (4 of
kind (a), 1 knock-on), no false decode or gain, pass 1 identical; but
the ceiling run's `fading` loss rate is 5 of 12 168, above O7's 9 of
35 888. 0f0341c stays unmerged; `HOST` keeps 50.

## O9: vector sine and cosine in the subtraction (2026-10-30, gated)

A result-changing CPU round, developed on the local branch `o9-vsin`
(not `feat/cat-ft8`) and gated later. The A76 profile at O6 puts 53.7%
of a three-pass slot in the subtraction, the fit loop and the time
search bound by two table-interpolated sine lookups per sample, which
the vector units cannot gather (the padded-table attempt of O6 lost to
general-to-vector register moves in half-vectorised loops).

1. The decoder's per-sample sine and cosine in the subtraction (the
   fit's reference, the removal's second run, the time search's
   correlations) from a minimax polynomial on the `u32` phase: integer
   reduction to the nearest quarter turn, both polynomials on the
   remainder, swap and signs by bit operations, so that a loop over
   samples vectorises with nothing scalar inside it. The degree chosen
   by measured accuracy against the table (largest error over every
   `u32` phase; the residual left by clean signals) and by A76 time.
2. Lane-split, fixed-order sums: the fit's five sums and the time
   search's two in four lanes keyed by the sample index (sample j in
   lane j % 4, each lane in sample order, the lanes added as
   `(l0 + l1) + (l2 + l3)`), so the order is the same on every target
   and does not depend on the scratch size or block splits. Loops are
   split so that the sine/cosine is a plain map (a loop the compiler
   vectorises) and the sums a separate pass.
3. Portable Rust only: no `unsafe`, no `std::arch`, `f32`, MSRV 1.75,
   `no_std`; no fused multiply-add (Rust never contracts). The encoder
   (`sin_turns`/`cos_turns`, `Synth`) keeps its table and stays
   byte-identical; only the decoder's subtraction changes. The time
   search's 15 rotations per trial keep the table (not per sample).
4. Steps measured one at a time on the A76 and x86-64 (same hygiene as
   O5–O8: one source path, a target directory per commit, hashes
   checked, rounds alternated, throttle flags before and after; under
   about 2% on the A76 is noise), with static Cortex-M33 instruction
   counts per sample (thumbv8m host image) and `check.sh --stack`.
5. Out of tree: the effect on `main`, `cap1`, `cap2` (never `fresh`,
   `accept2` or any other block): decodes lost and gained against
   bafed37 per set, false decodes, pass 1 unchanged, and the 800 WAV
   slots; the pins re-pinned with each change explained.
6. A proposed gate written in findings.md, naming an unused block for
   acceptance; **the acceptance run is not made** until the criteria
   are settled.

Status (findings, "ninth round"): developed and measured on `o9-vsin`
(a5c4a6c, a1119ed, ee7c0ce, d4e669e; ADR note 0c6f3bf). Degree 5/4.
A76 three-pass slot 187.0 → 151.1 ms (−19.2%), x86-64 −19.0%; on
main/cap1/cap2 1 lost (marginal), 0 gained, 0 false, pass 1 identical.
Gate fixed on block `accept3` (candidates 44d01ba, fallback ee7c0ce);
44d01ba passed and is merged (findings, "Acceptance run on `accept3`").

## O10: the spectrogram's transform, and small fixes (2026-10-30)

A bit-identical round on the local branch `o10-fft` from ddfd48d: every
output equal to ddfd48d's (records, CLI output, encoder), no more
memory, and the Cortex-M33 static cost of the transform not worse (it
runs in the small one-pass image too).

**A. The spectrogram's FFT** (`fft::power_bins`, 17.7% of the A76
three-pass slot at 55bbbff, in every configuration).

1. Measure first: an out-of-tree microbenchmark of one row (load and
   window, each of the six stages, the real split and powers, the
   per-bin `log2`) on x86-64 and the A76, and the AArch64 and x86-64
   code of the stages; cycles per transform.
2. Change no butterfly's arithmetic or operation order: independent
   butterflies side by side (adjacent `k` within a block), so a vector
   unit takes them together; the work buffer as separate real and
   imaginary arrays (same 15 360 bytes); twiddles laid out or read so
   lanes need no gather where possible; the real split and powers as
   plain loops that vectorise; fewer bounds checks. Identity by
   construction (each lane is one butterfly's own operations) and by
   tests: the transform against the reference bit for bit at every lane
   count, the powers against the form as first written bit for bit, the
   existing pins, the records and the 800 WAV slots.
3. Cortex-M33: flash, stack (`check.sh --stack`) and instructions per
   transform (the image run from `_start` in an instruction-set
   emulator, instructions counted inside `finish_row` and its callees)
   not worse than ddfd48d's. Where the target has no vector unit the lane
   count is one (the same results by construction).
4. Under about 2% of the A76 slot is noise: keep the simpler form.

**B. Small fixes from the O9 follow-ups** (all output-identical):
`Correction::skip` without the n < 2³² bound (halve the even factor,
then `wrapping_mul`); the ceiling mode's elapsed seconds to stderr (its
stdout then compares byte for byte; this removes that figure from
stdout, nothing else); `math.rs` comment and test notes (re-pin note,
the S5 coefficient's provenance, wrapping); findings corrections (BP the
largest single stage; 1f7d398's message reads the mutation backwards;
long lines).

**C. Plan only:** a proposed gated `i16` residual round (findings,
"Proposed: `i16` residual, gated"), not run.

Status (findings, "tenth round"): 4089b5b (FFT), e641a61, 5cb0e43,
7f89879. A76 three-pass slot 151.1 → 142.0 ms (−6.0%), one pass −9.3%,
x86-64 −4.0%; every record, WAV output and encoder output identical;
Cortex-M33 instructions per spectrogram row −12.4%, flash −132 B, stack
unchanged. The `i16` gate is proposed, not yet run.

Checks: local CI (fmt, clippy on the workspace, all features and the
thumbv8m library, tests with the ignored ones, MSRV 1.75, docs,
`check.sh --stack`, thumbv7em, no FMA and no `f64` helper in any
thumbv8m image); `paired_bench run` records of `main`, `cap1`, `cap2`
equal to ddfd48d's on x86-64 and the A76; the 800 WAV slots
byte-identical (but the moved ceiling figure, which the CLI does not
print); the encoder identical; A76 `slotcost` and a per-stage profile,
rounds alternated, one source path, a target directory per commit,
binary hashes compared.

**O10b, review follow-ups** (same branch, output-identical):

- `bin_powers` reduced to one form, if the M33 instruction count does
  not rise.
- The stage size `l` as a const generic.
- The lane checks derived from `STAGES` and evaluated per lane count,
  so `fft::<8>` fails to build (MSRV 1.75).
- The lengthened O9 lines reflowed, in their own commit.
- The bin-960 wording corrected.
- The small image's decode output compared in the emulator.
- The wording notes.
- A76 timing re-run with the live throttle bits sampled once a second.

Status (findings, "Review follow-ups"): 78d4f2f, 47739f1, 4af3304.
- M33 instructions per row −0.6% (1 027 705), flash +344 B.
- Every record (x86-64 and A76), WAV output and encoder output is
  identical to ddfd48d's.
- The emulated small image's decodes, spectrogram and decode list are
  byte-identical to ddfd48d's on seven slots.
- A76 three-pass slot 151.1 → 142.8 ms (−5.6%; 7f89879 142.0, the
  difference mostly BP placement). No live throttle bit in any sample.

## O12: belief propagation, bit-identical (2026-10-30)

A bit-identical round on the local branch `o12-bp` from 593b7c1: every
output equal to 593b7c1's (records, CLI output, encoder), no more
memory, and the Cortex-M33 cost of belief propagation not worse (it is
about 65% of the one-pass slot, the configuration of the small image).
Belief propagation (`sum_product_round` and `decode_soft` around it) is
32.7% of the A76 three-pass slot.

1. **Measure first.** On the A76 (user-space hardware counters, which
   the Raspberry Pi 5 allows without privileges) and x86-64 (`perf`): cycles,
   instructions, L1D misses and branch misses per edge of one round,
   in an out-of-tree microbenchmark over realistic states; the
   annotated AArch64 and x86-64 code of the hot loops (A76: a sampled
   PC histogram); the Cortex-M33 instructions per round and per
   one-pass slot of the small image in the instruction-set emulator,
   and the round's stack frame. Record cycles per edge and a floor:
   per edge, the loads, stores, floating-point operations by class and
   divides in the AArch64 code, each divided by the A76's throughput
   for that class (two FP/vector pipes, one divider, two load and one
   store per cycle); the floor is the largest of those quotients.
2. **Stop rule (fixed now, before any number).** Stop and record that
   belief propagation is at its floor, changing no code, if either
   - the measured round is within 1.3× of the floor of step 1, or
   - no candidate reaches 2% of the A76 three-pass slot or 3% of the
     M33 one-pass slot's instructions.
3. **Candidates** keep every floating-point operation and its order,
   so identity holds by construction: layout and indexing, bounds
   checks, hoisting, the loop shape of the check and variable passes,
   the per-round copies and the stopping test's bookkeeping, and the
   tanh/atanh path only where exactly the same operations remain.
   Identity also by tests (the round and the decode bit for bit against
   the current form over random states and edge values, saturation
   included), mutants that those tests catch, then the records and the
   800 WAV slots.
4. **Decision rule.** Under about 2% of the A76 three-pass slot is
   noise, unless the M33 one-pass slot gains 3% or more; otherwise the
   simpler form is kept. An M33 regression in belief propagation is not
   accepted.

Checks: local CI (fmt, clippy on the workspace, all features and the
thumbv8m library, tests with the ignored ones, MSRV 1.75, docs,
`check.sh --stack`, thumbv7em, no FMA and no `f64` helper in any
thumbv8m image); `paired_bench run` records of `main`, `cap1`, `cap2`
(`host`, `host,passes=1`, `small`) equal to 593b7c1's on x86-64 and
the A76; the 800 WAV slots × configurations × modes byte-identical; the
encoder identical; the small image's decode output in the emulator
byte-identical on the seven slots of O10b; A76 `slotcost` three and one
pass, rounds alternated, one source path, a target directory per
commit, binary hashes compared, the live throttle bits sampled once a
second (any live bit voids the run). No acceptance block is used.

Status (findings, "Belief propagation, twelfth round"): a04fc5a,
6fd70e8 (`tanh_half` computed once; the select kept where nothing
vectorises), 3120384 (24 values at a time in the tanh and atanh
passes), 7a5898d (min-sum in place), d1a9334 (totals swapped by
reference), 26bb83a (stopping test per check), d2667f1 (one exit),
636350d (the min-sum check's size as a type on the hosts). Stop rule
not met: the A76 round was 2.4× its floor (now 1.7×). A76 three-pass
slot 142.75 → 126.25 ms (−11.6%), one pass 29.6 → 21.7 (−26.7%), BP
−41.7%; the small configuration 16.1 → 15.5 ms; x86-64 three passes
−5.0%, one pass −11.4%; Cortex-M33 one-pass slot −9.6% (small image, min-sum) and −2.8%
(host image); flash −124 / −172 B, decoder frame −8 B. Every record,
WAV output and encoder output identical.

## O12b: belief propagation review follow-ups (2026-10-31)

On `o12-bp` after 1b1cc91; every output equal to 593b7c1's.

1. **Tests the review found missing**, each with the mutant it must kill:
   - a decode-level differential test of `decode_soft` against its
     593b7c1 form (both rules; `max_rounds` 0, 1, 2, 3, 7, 30, 255 and,
     per vector, the round it converges at and the one before), so a
     decode that converges exactly at the limit is pinned (mutant: the
     round limit tested before the checks);
   - stopping-test states in which every one of the 83 checks is the
     first to fail (mutant: the last check skipped);
   - the forms used where nothing vectorises (the select in
     `tanh_half`, the slice form of the min-sum check, one value at a
     time in the passes) selectable by a const parameter, so the host
     tests run both forms (mutant: the select's negative branch
     returning `x`); and the host image's sum-product path added to the
     emulated Cortex-M33 output comparison.
2. **Simplifications**: `VECTOR` by `target_arch` (x86-64, AArch64),
   not by `target_feature` (32-bit ARM with NEON keeps these loops
   scalar); the min-sum check over plain slices cut to 6 or 7, if the
   M33 code stays the same and x86-64 within noise.
3. **One candidate, the variable-side sums**: every variable has
   exactly three checks, so a compile-time `VAR_EDGES: [[u16; 3]; N]`
   (each variable's edges in check order) turns the per-check scatter
   into `next[i] = llr[i] + m[a] + m[b] + m[c]`, the same additions in
   the same order, one loop for both rules. Measured on the A76 (three
   passes and one), x86-64 and the M33 emulator (small one-pass slot,
   host image). **Kept if the A76 three-pass slot gains ≥ 2% or the
   M33 small image's one-pass slot ≥ 3%**; flash growth up to +2 KB is
   accepted for it (the maintainer's decision, recorded with the result).
   Otherwise rejected with its numbers, and belief propagation is
   recorded as done for bit-identical work.
4. **Findings corrections**: the d1a9334 + 26bb83a grouping recorded as
   a deviation decided after measuring; 3120384's M33 code "same count
   (small image), +1 instruction a round (host image)"; the work-branch
   commit hash dropped; the lane count's fragility in the toolchain-bump
   checks.

Checks: as in O12 (local CI with MSRV 1.75 and `check.sh --stack`,
thumbv7em, no FMA and no `f64` helper; records of `main`, `cap1`, `cap2`
× `host`, `host,passes=1`, `small` on x86-64 and the A76; the 800 WAV
slots × configurations × modes; the encoder; the emulated small image's
output on the seven slots, and the host image's; the Pi release suite).
Timing: rounds alternated, one source path, a target directory per
commit, binary hashes, the live throttle bits sampled once a second on
another core (any live bit voids a run).

Status (findings, "Belief propagation, review follow-ups"): 76151f8,
676d274, 49db9e3, c6e0e4e, 627d356, ee047b8; the four mutants the review
found surviving are killed. The variable-side sums: A76 three-pass
−1.21% (with flat inputs −1.94%), M33 small one-pass −1.62%, flash
+1 052 / +1 444 B; rejected under the fixed bar. Belief propagation is
done for bit-identical work. Every record, WAV output, encoder output
and emulated M33 output (small and host images) identical.

## O13: the subtraction, measure first (2026-10-31; measurement only)

The subtraction is about 57% of the A76 three-pass slot (72.4 of
126.2 ms, about 2.1 ms per subtraction). No code changes in this step:

1. A fresh sampled profile of the merged head on the A76 (the PC
   sampler out of tree) and, where user space allows, hardware counters
   on each stage: the fit loop, its waveform generator, the time
   search, the removal. Per stage: ms per slot, cycles and instructions
   per sample, IPC, and what bounds it (issue, a pipe, latency, memory),
   with the annotated AArch64 hot loops; the Cortex-M33 static size and
   emulated instructions of each stage in the host image.
2. Placement noise: the subtraction moved by about 0.66% between builds
   that did not change it. Propose how a gate accounts for it.
3. One to three ranked bit-identical candidates and a stop rule,
   written into findings before any of them is implemented.

Status (findings, "The subtraction, measured first"): 72.1 of the
125.8 ms three-pass slot, 32.0 cycles a sample, 2.0× an issue floor; by
stage the time search 18.2 ms, removal 16.9, reference 13.3
(latency-bound), generator 8.7, correction 7.4, fit 6.7. Ranked:
several four-lane chains in the reference and the time-search
products; the removal's ramp index as an exact `f32` per lane; the
fit's sums kept in registers. Stop rule and a placement-noise
allowance for gates recorded. Not implemented.

## O13: the subtraction, bit-identical round (2026-10-31)

On the local branch `o13-sub` from 67e2458. Every output equal to
67e2458's (records, CLI output, encoder, the emulated Cortex-M33 small
and host images' decodes); memory never larger. These rules are fixed
before any new measurement.

**Candidates, in this order**, one commit each, each judged against the
branch head before it:

1. **Several independent four-lane chains side by side** in the
   reference (cosine and sine) and the time search's products, with the
   count set by `VECTOR` (one chain where nothing vectorises, so the M33
   code stays as it is); and the seven polynomial constants the products
   loop re-makes (`dup`) every iteration hoisted out of it.
2. **The removal's ramp index as an exact `f32` per lane**: `j − from`
   is below 2²⁴, so an `f32` per lane (a 32-bit conversion, or a lane
   counter stepped by 4.0) is exactly `(j − from) as f32`, replacing the
   64-bit conversions and lane inserts. The exactness is proved in the
   code's comment and by test.
3. **The fit's five lane sums kept in registers and the sine loaded
   once.**
4. **The variable-side sums in belief propagation** (and the flat
   edge-variable inputs; the O12b candidate forms), entered again under
   this round's rule.

**Decision rule**, per candidate, with no regrouping after measuring:

- Timing: A76 `slotcost host,passes=1 host 200 5`, the candidate against
  its parent in two alternated rounds, one source path, a target
  directory per commit, binary hashes compared, the live throttle bits
  sampled once a second on another core (any live bit voids the run).
- The changed stages are the slot benchmark's stages that hold the
  changed code (`subtract` for 1–3, `belief_propagation` for 4). Their
  gain G is the parent's summed stage means less the candidate's, ms.
  The bar B is 2% of the parent's three-pass slot mean.
- The allowance A is the larger of 1.0 ms and the largest move of an
  untouched stage in the same A/B.
- **Keep** if G ≥ B + A, or the Cortex-M33 one-pass slot's emulated
  instructions fall by ≥ 3% (the small image; for 4 also the host
  image; 1–3 do not run in a one-pass slot, so for them only the A76
  bar can carry a candidate). **Reject** if G < B − A and the M33 bar
  is not met.
- **Inconclusive**, re-measured with a second build pair (both commits
  built at a second, different source path): G between B − A and
  B + A, or the untouched stages' net move above 1.0 ms. After the
  second pair the candidate is decided on the mean G of the two pairs
  against B, with no further pairs.
- **Bounds** (a candidate that breaks one is rejected): no Cortex-M33
  instruction regression in either image (the one-pass slots, and for
  1–3 the subtraction's instructions in the multi-pass measurement
  image); small-image flash unchanged, except +2 KB allowed for 4 (as
  recorded in O12b); host-image flash at most +512 B for 1–3 (+2 KB for
  4); no larger stack frames; static RAM unchanged.

**Stop rule**: stop and record the subtraction as done for bit-identical
work if the measured subtraction is within 1.3× of its issue floor
(counters, as in O13's measurement), or no candidate reaches the bar.

**Tests**, one commit with each change: the new form against the
current form bit for bit (random and edge inputs, both `VECTOR` forms
through a const parameter as in 49db9e3), and a mutant per new test
that it kills.

Checks: local CI (fmt; clippy on the workspace, all features and the
thumbv8m library; tests with the ignored ones; MSRV 1.75; docs;
`check.sh --stack`; thumbv7em; no FMA and no `f64` helper in any
thumbv8m image); the 73 800 `paired_bench run` records identical on
x86-64 and the A76; the 800 WAV slots × configurations × modes
byte-identical; the encoder identical; the emulated small and host
images' decodes on the seven WAV slots identical; the Pi release
suite. No acceptance block is used.

Status (findings, "The subtraction, thirteenth round"): all four kept.
- d486205: 16 samples at a time (subtract −5.07 ms).
- d116122: the ramp index in `i32` (−6.45 ms).
- 94ae1b8: the fit's sums out of line (−3.48 ms).
- 57cbc67: belief propagation's sums by variable and inputs by edge (BP
  −3.34 ms).
- e3f5444: test fix.

Candidate 3 passes by 0.20 ms; candidate 4's G is 3.34 ms against its
parent, about 2.5 ms against the same-code median (corrected in O13b;
kept). A76 three-pass slot 125.81
→ 107.28 ms (−14.7%), one pass 21.70 → 20.30, the subtraction 72.07 →
56.59 (−21.5%), 1.81× its floor (from 2.01×). Cortex-M33: the
subtraction −7.3%; the one-pass slots −1.6% (small) and −2.9% (host).
Every record, decode and residual is identical. The one changed output
is `static_table_bytes` in `--profile` (+1 566, the BP tables), and
flash is +1 444 B (small) and +600 B (host) net.

## O13b: review follow-ups (2026-10-31)

On `o13-sub` after 41f7368. Decodes, records, residuals and encoder
output equal to 67e2458's; the one output that may change is
`static_table_bytes` in `--profile` (the tables, below). These rules are
fixed before any new measurement.

1. **Belief propagation's graph stored once.** `check_edges()` takes
   each check's variables from `EDGE_VARS` (they are the same bytes, in
   the same order, as the padded check list's first `DEGREES[c]`), and
   the stopping test walks `check_edges()`: the same checks in the same
   order, so it stops at the same first failing check. The padded check
   list moves to the tests, which keep it as an independent derivation
   from `PARITY_CHECKS`. `TABLE_BYTES`, its doc, the table accounting
   test and ADR 0024's table row follow. A non-regression check, not a
   bar:
   - A76: its belief-propagation stage, in the three-pass and the
     one-pass slot, is compared with the median over this round's
     builds whose belief-propagation functions are byte-identical to
     the parent's (the standing rule below). It is a regression if
     either is above that median by more than A (the larger of 1.0 ms
     and the largest untouched stage's move against the same builds).
   - Cortex-M33 (emulated): the one-pass slot of the small and the host
     image not up by more than 0.1%; flash not larger in either image;
     no larger stack frame; static RAM unchanged.
   - A regression in either is recorded and the change not kept in
     that form.
2. **`scaled_cos_sin` without its type parameter**: the weights are
   always `f32`. The `if W > 1` guards stay (without them the thumbv8m
   library grows). Check: the AArch64 `paired_bench` and both M33
   images' functions identical instruction for instruction (names
   aside); no timing.
3. **Tests**: the removal at and before the centre with base −0.0, a
   positive slope, −0.0 audio and positive references, where the clamp's
   +0.0 is observable (mutant: the clamp returning −0.0).
4. **The removal's index bound as a release `assert!`**, once per run,
   outside the loop. Kept if it costs nothing measurable: the AArch64
   removal loops the same length (28 and 168 instructions), the A76
   subtract stage not above its same-code median by more than A, and
   the M33 subtraction's instructions (the multi-pass image on s0086)
   up by at most 0.1%. Otherwise a comment names the callers that
   guarantee the bound, and the `debug_assert!` stays.
5. **`paired_bench slotcost` prints each pass's new decodes** (list
   entries added) and its cost per new decode (the pass's summed stage
   means ÷ its new decodes per slot). Output of the bench tool only.
6. **Alignment trial for measurement builds.** The four O13 builds whose
   belief-propagation code is byte-identical (a003d6e, d486205,
   d116122, 94ae1b8; BP 23.83–24.64 ms in O13), each built as now and
   with `-C llvm-args=-align-loops=64 -C llvm-args=-align-all-functions=6`
   (64-byte loops and functions), all eight timed in one session (two
   alternated rounds). The measure is the spread (largest less
   smallest) over the four builds of each stage whose functions are
   byte-identical in all four. Adopted for measurement builds only if
   the aligned builds' largest such spread is at most half the plain
   builds' and the plain spread is at least 0.4 ms (otherwise the trial
   cannot show a gain); released builds unchanged either way.
7. **Findings corrections (O13)**: candidate 4's gain against the
   median of the same-code builds; the round described as "decodes,
   records, residuals and encoder identical; one profile metadata field
   changes"; the circular non-regression clause and its replacement;
   the second-pair rule's leniency (it ignores the allowance); that
   candidate 4 was entered again under a rule changed in advance.

Checks: as in O13 (local CI with MSRV 1.75 and `check.sh --stack`,
thumbv7em, no FMA and no `f64` helper; the 73 800 records on x86-64 and
the A76; the 800 WAV slots × configurations × modes identical except
`static_table_bytes`; the encoder; the emulated small and host images'
decodes and the multi-pass image's residuals; the Pi release suite).
Timing: rounds alternated, one source path, a target directory per
commit, binary hashes, the live throttle bits sampled once a second on
another core (any live bit voids a run).

### Standing rule for timed gates (from O13b)

The changed stage's baseline is the median over every build in the
round whose changed-stage functions are byte-identical (compared
function by function in the disassembly, addresses and names aside),
not the parent's measurement alone. A sequential gate otherwise hands
each candidate its parent's placement: a stage that one commit's
layout slowed becomes a free gain for the next commit that changes it.
The allowance for untouched stages (the larger of 1.0 ms and the
largest untouched move) stays.

### O13b result

Every check passes and every change is kept (findings: "Review
follow-ups (O13b)"). Against the same-code medians: c520517 subtract
+0.04 ms; 3daa258 BP −0.25 ms (one-pass −0.11); f8f2a7b BP +0.04 ms;
M33 one-pass +0.019% (small) and +0.038% (host); flash −572 B and
−500 B; static RAM and stack unchanged. Cost per new decode by pass:
0.84, 5.20 and 11.09 ms. Alignment trial: spread 0.82 → 0.41 ms,
adopted for measurement builds only (aligned and plain numbers are not
comparable).

## OSD sizing round (done, 2026-10-31; measurement only)

Measurement first; the decoder is unchanged (the default release
binaries byte-identical to the round's parent, checked). The question is
whether ordered-statistics decoding (Fossorier and Lin, 1995) of the
candidates belief propagation fails on would buy decoded signals more
cheaply than the third pass does. It is sized here, not implemented in
the decoder. The measurement code is a `paired_bench` subcommand and, for
the Cortex-M33, an image rewritten out of tree; as in E4 it is removed
afterwards (or kept in the bench only) and the commit to reproduce it is
recorded. Nothing from WSJT-X is read; the OSD and its acceptance test
come from the published literature.

**Sets: already-used blocks only.** `crowded`, `wide`, `fading` on
`main`, `cap1` and `cap2`; `sweep`, `edges` and `empty` on `main`;
`host`, `host,passes=1` and `small`. No unused block is touched: they
are kept for the gated round.

**What is measured, per pass, per set and by SNR (the ≤ −17 dB band
separately):**

1. **The pool.** Every candidate whose belief propagation fails (not
   converged, CRC failure, all-zero), split by whether it sits on a
   transmitted message (the existing rule: tone 0 within 4 Hz, start
   within 0.12 s) and whether that message was decoded anyway (by
   another candidate or pass). For each failed candidate on a message,
   against that message's codeword:
   - the Hamming distance of the channel's hard decisions and of belief
     propagation's last totals' hard decisions (174 bits);
   - the errors inside the most reliable basis: positions ordered by
     |soft bit|, Gaussian elimination over GF(2) picking the 91 most
     reliable independent positions, and the true codeword's
     disagreements with the hard decisions there. OSD of order i finds
     the true codeword in its list exactly when this count is at most
     i, so this gives OSD-0/1/2/3's reach directly. Both inputs (channel
     soft bits and belief propagation's last totals) are measured.
2. **What OSD-1 and OSD-2 would recover.** A measurement-grade OSD
   (bit-packed elimination, order-i reprocessing over the basis, the
   best codeword by correlation discrepancy, then the CRC-14 and the
   message's plausibility: it unpacks, and its fields are the kinds the
   decoder lists) on every failed candidate. Counted: recovered (the
   chosen codeword is the transmitted one), new decodes (its message
   not otherwise decoded in the slot, per pass and after all passes),
   duplicates, and wrong codewords, each as a function of a distance
   threshold on the chosen codeword (a trade-off curve, not a chosen
   threshold).
3. **The false-decode rate it would add.** Every CRC-passing,
   plausible codeword that is not a transmitted message is a false
   decode: on `empty` (every acceptance is false) and on the
   off-message candidates of every set. Two acceptance forms are
   measured: the best codeword, then the CRC (2⁻¹⁴ per noise candidate
   before any threshold, about 0.005 false decodes per slot over 75
   failures), and the best CRC-passing codeword of the list (OSD-2's
   list holds 1 + 91 + 4 095 codewords, so about one noise candidate in
   four would pass by chance). The distance threshold is what holds the
   rate down, and with it the expected count in these blocks is near
   zero, so counting alone is weak. The rate is also estimated from its
   factors: the measured share of noise candidates whose best codeword
   is under the threshold, × 2⁻¹⁴ for the CRC, × the measured
   plausibility pass rate of random payloads; the counted false decodes
   are the check, with their 95% upper bound (rule of three) stated.
4. **Cost per candidate.** A76 µs per failed candidate for OSD-1 and
   OSD-2 (elimination and reprocessing separately), timed under the
   standard protocol (alternated rounds, one source path, binary
   hashes, live throttle bits); Cortex-M33 emulated instructions per
   candidate, small and host images; scratch (the 91 × 174 bit matrix
   is about 2 KB, on the caller's scratch, not the stack), the stack
   frame, and flash. The per-slot cost follows from the failures per
   pass (about 75 in pass 1 of a crowded slot).
5. **The decision metric.** Per pass and in total: ms of OSD per slot
   ÷ new decodes it adds per slot, against the third pass's ms per new
   decode as `paired_bench slotcost` now prints it (A76 and M33). The
   report says which order, which passes and which candidates (all
   failures, or only those above a sync or soft-bit score) would beat
   the third pass and by how much, with the false-decode rate at each
   point. It recommends whether to open the gated round; it does not
   change a default.

**What the later gated round would need** (written before it runs):
- its criteria fixed in advance: the order, the candidates it runs on,
  the distance threshold and plausibility test, the gain bar (new
  decodes per slot on crowded, wide and fading, and in the ≤ −17 dB
  band), the no-loss bar, the CPU bound (ms per new decode below the
  third pass's; the M33 small image's budget in instructions, flash,
  RAM and stack);
- one block never used before for every set it judges, so the
  threshold chosen here is not tested on the data it was chosen from;
- a false-decode bound: at most a stated rate per slot at 95%
  confidence, from a run of empty slots on unused seed blocks large
  enough for that bound (with none observed, three divided by the
  number of slots), plus the factor estimate above;
- the 800 WAV slots against the reference decoder (black box), and
  every output identical with OSD off.

### Fixed before any measurement (branch `osd-size` from 511bf32)

Everything below is fixed in this commit, before any number is taken;
nothing is chosen after seeing data. The decoder is unchanged: the only
library change allowed is a `profile`-only accessor for the candidate's
soft bits and belief propagation's last totals (absent from the default
build, so the default release `cat-ft8-decode` stays byte-identical to
511bf32's; checked by hash). The OSD is written here from Fossorier and
Lin (1995); no other implementation is read or copied.

**Configurations.** `host` (three passes) and `small` (one pass).
`host,passes=1` is host's pass 1 (the same search and candidates), with
its new decodes counted against pass 1's list; it is derived from the
host run, not run again.

**Blocks.** `crowded`, `wide`, `fading` on `main`, `cap1`, `cap2`;
`sweep`, `edges`, `empty` on `main`; and, for false decodes only,
`empty` on `bulk01`…`bulk20` (46 000 slots already used in E4). No
other block, and no new seed block, is touched.

**Pool.** Per pass, every candidate whose `decode_candidate` fails (not
converged, CRC, all-zero). Per pass k of host, the decoder is run with
k passes and the last pass's candidates are decoded again (outside any
timing) for their soft bits; a slot whose decode ended before pass k
has no pass-k pool. On a message: the existing rule (refined tone 0
within 4 Hz, start within 0.12 s); on several, the one whose codeword
is nearest the channel's hard decisions. Decoded anyway: the message's
payload in the list after that pass, and after all passes.

**Inputs.** The channel soft bits (`llr`, positive favours 0) and belief
propagation's last totals. Hard decision 1 where the value is
negative. Reliability |value|.

**Most reliable basis (MRB).** Positions in order of reliability,
largest first, ties to the lower index; Gauss–Jordan elimination over
GF(2) on the generator's 91 rows takes each position in that order as a
pivot if it is independent of the pivots so far, until 91. The MRB
error count e is the number of pivot positions where the true codeword
differs from the hard decisions; OSD of order i lists the true codeword
exactly when e ≤ i (reported for i = 0…3).

**The OSD (orders 1 and 2).** The list: the codeword re-encoded from the
hard decisions on the basis, then every flip of one basis bit, then (order 2)
every pair, in pivot order. The discrepancy D of a codeword is the sum
of |value| over positions where it differs from the hard decisions.
Two acceptance forms:
- **A (best, then CRC):** the codeword of least D (ties: first listed),
  accepted if its 91-bit message part is non-zero, passes the CRC-14 and
  is plausible;
- **B (best CRC-passing):** the least-D codeword among those whose
  message part is non-zero and passes the CRC, accepted if plausible.

Plausible: `unpack` with `NoHashes` returns `Ok` (so Field Day and RTTY
Roundup, which it reports as unsupported, are not). The plausibility
rate of random payloads: 1 000 000 uniformly random 77-bit payloads
from a fixed seed.

**Metrics and the curve.** For the accepted codeword: dH, its Hamming
distance from the hard decisions, and Dn = D / Σ|value| (scale-free).
Acceptance at threshold T: metric ≤ T. Curves over Dn ∈ [0, 0.30] in
steps of 0.0025 and dH ∈ [0, 80]. An accepted codeword is **true** (the
candidate's own message), **other** (another message transmitted in the
slot) or **false** (none; every acceptance on `empty` is false).
Recovered messages are counted once per slot at their first recovery;
**new** means not in the decoder's list (per pass: after that pass;
overall: after every pass of the configuration). Reported per
configuration, pass, set and block, by SNR (the ≤ −17 dB band
separately), and by the candidate's rank in its pass (≤ 10, ≤ 25, all).

**False decodes.** Counted (both forms, every T) on `empty` and on every
off-message failed candidate, with the rule-of-three 95% bound when
none. Estimated from factors, per pass, from the empty slots' failed
candidates (n per slot): form A, n × qA(T) × 2⁻¹⁴ × π, with qA(T) the
share whose least-D codeword is within T (3/N when none); form B,
n × mB(T) × 2⁻¹⁴ × π, with mB(T) the mean number of list codewords
within T per candidate (every list codeword is measured); π the
plausibility rate. The current rate is the decoder's own, measured the
same way on the same empty slots: CRC failures per slot × 2⁻¹⁴ (E4).
**T\*** for each configuration, input, order, form and metric is the
largest grid T whose estimated added rate is at most the current
rate (so the total at most doubles). If the counted false decodes at T*
exceed the estimate's Poisson 95% upper limit, the estimate is rejected
and T* is lowered to the largest T with no counted false decode.

**Cost.** A76: a `paired_bench osdcost` subcommand times, on one thread,
the reliability sort with the elimination, OSD-1's and OSD-2's
reprocessing (forms A and B), per candidate, on every failed candidate
of host's three passes on the 200 40-signal `crowded` `main` slots
(the `slotcost` slots), median of 5 runs; a 64-byte-aligned
measurement build (the adopted rule), two runs in one session, the
throttle bits sampled once a second on another core (any live bit
voids a run), the binary's hash recorded. Its outputs (hashed) must
equal x86-64's. Cortex-M33: an out-of-tree image runs the same OSD
(`no_std`, no heap, `f32`, no FMA) on 16 failed candidates' inputs
taken by a fixed rule (every 50th pass-1 failure of the same slots) and
the emulator counts instructions per call; its results must equal
x86-64's. Scratch bytes, the largest stack frames (`-Z
emit-stack-sizes`) and the OSD's flash (its functions and tables) are
reported. Per slot: failures per pass × cost per candidate.

**The decision metric (fixed).** On the `slotcost` slots, A76: ms of OSD
per slot ÷ new decodes it adds per slot (new after all three passes),
at T*, per pass and for passes 1–2 and 1–3 together, against pass 3's
11.09 ms per new decode (O13b, these slots); also pooled over
`crowded`/`wide`/`fading` × `main`/`cap1`/`cap2` and on the M33 in
instructions. The input, order, form and metric used for the verdict
are those with the most new decodes at T*; the others are reported.
Verdicts:
- **replace pass 3:** OSD on passes 1–2 adds at least pass 3's new
  decodes per slot (3.0) for fewer ms than pass 3's 32.82 ms;
- **supplement:** some pass's OSD costs less per new decode than
  11.09 ms;
- **neither:** otherwise.
For `small`: new decodes per slot per extra A76 ms and per 10⁷ emulated
M33 instructions, beside its mean 0.64 ms per decode. **Promising**
(the later gated round's criteria are then drafted, not run): a
supplement or replace verdict. What this cannot measure, and the
verdict does not claim: the knock-on effect of subtracting OSD decodes
on later passes, and any block not used before.

### OSD sizing result

Findings, "OSD sizing". Commits b8ff866 (rules), 5ec0eed, af23f41,
4657bfe, 3f48a52, 48dd456, 2cbb245; the default binaries are
byte-identical to 511bf32's. Two deviations, both recorded there: the
false-decode estimate as fixed counts the all-zero codeword and gives
no usable threshold anywhere, so the estimate counts non-zero codewords
only; and the verdict is also given for each pass's 10 best-scoring
candidates.

- Pool: on crowded slots the missed messages sit a median 34–42 bits
  from the hard decisions; OSD-2 could list the truth for 13–18% of
  them.
- At a threshold matched to the current false-decode rate (9.0 × 10⁻⁷
  per slot, host), with all failed candidates: OSD-1 (channel soft
  bits, best codeword, Hamming distance ≤ 22) 0.070 new decodes per
  slot for 5.4 ms (77 ms each); OSD-2 0.084 for 21.8 ms (260 ms each).
  **Neither replaces nor supplements pass 3** (11.09 ms per new
  decode); replacing it would need 3.0 per slot, at most 0.11 is there.
- On each pass's 10 best candidates: OSD-1, distance ≤ 24, 0.104 new
  decodes per slot for 0.45 ms (4.3 ms each; ≤ 7.2 ms on the `slotcost`
  slots, 0.13 per slot there): **a supplement**, at about 0.5% of a
  crowded slot's decodes. No false decode counted (95% bound 4.2 × 10⁻⁵
  per slot); the estimate is 5.1 × 10⁻⁷ per empty slot.
- Cost: A76 31.0 µs per candidate (OSD-1), 124.9 (OSD-2); M33 0.35 M
  and 2.09 M emulated instructions; flash +6.7 KB (3.6 KB of it the
  core library's sort), scratch 9.2 KB plus 2.2 KB of generator rows.
- Small: top 25, distance ≤ 23, +0.079 decodes per slot for 0.31 ms and
  3.6 M M33 instructions per slot.

## OSD-1 on the best candidates: gated round (proposed; not yet run)

Drafted from the sizing round; nothing here is implemented or run. The
criteria are fixed in the commit that names the block, before any
record of it exists, and applied mechanically: if any fails, nothing
merges; no retune and no second block.

**The change** (a local branch, result-changing): after belief
propagation fails on one of a pass's 10 best-scoring candidates, OSD-1
on its channel soft bits (reliability order, elimination for the most
reliable basis, the order-0 codeword and every one-bit flip of the
basis, least discrepancy); the codeword is a decode when its message
part is non-zero, its CRC-14 passes, it unpacks with no call hashes, and
it is at most **24** bits from the hard decisions (host; fixed here, not
retuned). It enters the list like any decode (so it is subtracted
before the next pass), marked as an OSD decode in the profile only.
`small` is a separate candidate: top 25, at most 23 bits. Storage:
the candidate's belief-propagation scratch (idle once it fails) and a
generator table in flash; no new static RAM. With the option off,
every output byte-identical to the base.

**Blocks.** One block never used before for `crowded`, `wide`, `fading`,
`sweep` and `edges`, named only in the gate commit and checked absent
everywhere first (as for `accept3`); and, for the false-decode bound,
at least 20 unused seed blocks of `empty` (46 000 slots), named the same
way.

**Criteria, OSD on against OSD off, same commit:**
- **False decodes, counted:** 0 in every record, pass and set of the
  gate block, on the empty blocks, and in the WAV output (with the 95%
  bound 3 / slots stated).
- **False decodes, estimated:** on the unused empty slots, n × qU ×
  2⁻¹⁴ × πU at most the current rate measured on the same slots (CRC
  failures per slot × 2⁻¹⁴), with qU the 95% upper bound on the share
  of top-10 candidates whose best non-zero codeword is within 24 bits
  and πU = 0.46 (the largest plausible share of CRC-passing noise
  codewords measured here); n the top-10 failed candidates per slot of
  a three-pass crowded slot (14.5 here), not the empty slot's 10.
- **Gain:** new decodes per slot on `crowded`+`wide`+`fading` at least
  0.05 (half the sizing's 0.104), at or below −17 dB at least 0.015
  (half of 0.036); sweep 50% point not worse than the existing shift
  gate (0.05 dB); `edges` no loss.
- **Loss:** the existing `compare` no-loss gate (b − c ≤ 3√(b + c) per
  point, ≤ 2√(b + c) per set), and lost messages at most 10% of gained
  per set; every lost, gained and pass-moved message classified by the
  out-of-tree classifier, pinned by SHA-256 in the gate commit, with a
  class added for OSD decodes and their knock-on (a later-pass change
  within 50 Hz of an OSD decode's subtraction); one unexplained message
  fails.
- **800 WAV slots** against the reference decoder (black box): with the
  option off, every output byte-identical to the base; with it on, no
  slot loses a decode the base had, and every added decode is in the
  reference decoder's output (any other is false).
- **CPU (A76):** 64-byte-aligned measurement builds, the standing rule
  for baselines, the live throttle sampler (any live bit voids a run):
  `slotcost host` three-pass slot up by at most 1.0 ms; OSD's own time
  (timed as a stage) divided by its new decodes per slot below 11.09 ms
  on the `slotcost` slots of the gate block.
- **Cortex-M33 (emulated):** OSD-1 per candidate at most 0.4 M
  instructions; the small image's one-pass slot up by at most 2% if
  `small` adopts it (else unchanged); flash at most +7 KB per image;
  static RAM unchanged; no stack frame above the current largest; no
  FMA, no `f64` helper; `check.sh --stack` passes.
- **Pinned definitions:** on a message = refined tone 0 within 4 Hz and
  start within 0.12 s; false = CRC-passing, plausible and not
  transmitted; plausible = `unpack` with `NoHashes` returns `Ok`.
- Checks as in O13b (local CI with MSRV 1.75, thumbv7em, records on
  x86-64 and the A76, the encoder, the Pi release suite).

## Next: gated, result-changing rounds

Each changes results and needs its own acceptance (the run sets, the
800 WAV slots, a low-level set for anything that narrows the samples):
later passes searched near the previous pass's decodes (b above); a
pass-3 trigger or cap (a above); lane-split fixed-order accumulators in
the fit and removal; the `i16` residual; a phase-rotator reference.

## Later (not in this round)

- Contiguous codeword layout and table CRC, as profiled optimisations that
  must leave results identical.
- Field Day and RTTY Roundup (need the section and state/province tables
  in protocol order). They may be recoverable black-box: pack each
  section/state with the oracle and read the index from its payload bits.
- A field-level payload API (bit offsets and widths, the `c28` of one
  call) for a-priori decoding.
