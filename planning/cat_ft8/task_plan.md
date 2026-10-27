# cat_ft8 — task plan

## Goal

A new workspace crate `cat-ft8`: FT8 implemented from the published
protocol (Franke, Somerville, Taylor, QEX July/August 2020). This step is
the encoder: message pack/unpack, CRC-14, LDPC (174,91), tone mapping and
GFSK synthesis. Decoding follows later in the same crate, after its own
plan is approved.

## Current Phase

Phase 1 (encoder) committed on `feat/cat-ft8`; ADR 0024 accepted
2026-10-26. Phase 2 (single-pass decoder): plan approved by the user
2026-10-26 (D0 done; ADR amendment accepted with estimated figures).
Next: D1–D3, stopping at the D3 checkpoint.

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

## Phase 2: decoder (single pass) — PLAN, revised after review (2026-10-26)

Design decisions are in ADR 0024, "Proposed amendment: single-pass
decoder" (to be accepted at D0 with its estimates, and updated with
measured values at D11). Sizing notes are in findings.md. One task at a
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
D8. [ ] WSJT-X-free end-to-end tests with the encoder and seeded noise:
        SNR sweep with pinned rate floors; noise-only slots (thousands,
        release build, separate CI job) including carrier-plus-noise
        slots, with zero decodes; non-overlapping and overlapping (10–30 Hz
        apart) signals; dt and frequency ranges; partial and late input;
        block-size and thread-count invariance; golden decode output hash
        on Linux and Windows CI.
D8b. [ ] CI: a new `ubuntu-24.04-arm` (aarch64) job running
         `cargo test -p cat-ft8` (golden waveform and decode hashes; the
         bulk noise-only test stays in its own job), so the determinism
         promise is checked on a second architecture.
D9. [ ] `cat-ft8-decode` (std): reads a 12 kHz mono 16-bit WAV; output as
        above; format documented as version 0; JSON written by hand with
        string escaping (`serde_json` stays a dev-dependency); tests parse
        the output with `serde_json`.
D10. [ ] Link check final form: decoder fed a synthesized slot in blocks
         from a static workspace, no allocator, no f64 helpers; flash,
         `.bss` and stack reported.
D11. [ ] Measurement against an external reference decoder on synthetic
         slots across SNR, time offset, frequency, crowding and fading
         (external harness); profile and memory; ADR amendment updated
         with measured values.
    ◆◆ Results to the user; then optimisation passes under the existing
       rules (results identical, at most five).

### Decisions by the user (2026-10-26, at approval)

1. **Microcontroller timing:** QEMU (an MPS2 Cortex-M33 model) for
   instruction counts now; no microcontroller speed claims until measured
   on a board, chosen later.
2. **Slot boundary on a 256 KiB part:** the small configuration plus a
   1 s i16 FIFO, with a 1 s microcontroller decode budget.

## Later (not in this round)

- Contiguous codeword layout and table CRC, as profiled optimisations that
  must leave results identical.
- Field Day and RTTY Roundup (need the section and state/province tables
  in protocol order). They may be recoverable black-box: pack each
  section/state with the oracle and read the index from its payload bits.
- A field-level payload API (bit offsets and widths, the `c28` of one
  call) for a-priori decoding.
