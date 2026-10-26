# cat_ft8 — task plan

## Goal

A new workspace crate `cat-ft8`: FT8 implemented from the published
protocol (Franke, Somerville, Taylor, QEX July/August 2020). This step is
the encoder: message pack/unpack, CRC-14, LDPC (174,91), tone mapping and
GFSK synthesis. Decoding follows later in the same crate, after its own
plan is approved.

## Current Phase

Fix round and cleanup addendum complete (plan approved by the user
2026-10-25), uncommitted on branch `feat/cat-ft8`. Planning goes in its
own commit; ADR 0024 accepted 2026-10-26.

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

## Later (not in this round)

- Contiguous codeword layout and table CRC, as profiled optimisations that
  must leave results identical.
- Field Day and RTTY Roundup (need the section and state/province tables
  in protocol order). They may be recoverable black-box: pack each
  section/state with the oracle and read the index from its payload bits.
- A field-level payload API (bit offsets and widths, the `c28` of one
  call) for a-priori decoding.
- Decoder: needs its own approved plan first (memory per stage against a
  named microcontroller class, FFT source, API with caller-owned
  workspace, time budgets, provenance per stage, oracle tests,
  determinism of decodes).
