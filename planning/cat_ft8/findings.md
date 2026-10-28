# cat_ft8 — findings

## Sources

- QEX July/August 2020 paper: message types (Table 1), field tags
  (Table 2), c28 ranges (Table 7), CRC (§3), Gray map (Table 3), Costas
  array and GFSK pulse (§4–5, eq. 3), T/8 raised-cosine ramp (§5).
- The paper's reference [14] (generator/parity matrices, section and
  state lists) could not be downloaded; the LDPC tables come from ft8_lib.
- ft8_lib (MIT, 9fec6ca398) consulted for: LDPC tables (copied), CRC
  bit order, c28 offsets, hash multiplier and alphabets, waveform edge
  handling (first/last tone extension). Rust written independently.

## Black-box behaviour of WSJT-X 3.0.0-rc1 (`ft8sim`) where it differs from the paper or ft8_lib

Matched by this crate:

- `RR73` after two calls is sent as the grid locator RR73 (g15 = 32 373),
  not the reserved value MAXGRID4 + 3. Both unpack to `RR73`.
- Free text shorter than 13 characters is right-aligned (leading spaces).
  ft8_lib left-aligns.
- Any single word of 1–18 hex digits is telemetry (0.5): `ABC`, `73`,
  `A1` alone. Telemetry 0 unpacks as an empty string in WSJT-X (this crate
  shows `0`).
- Type 1/2 reports: −30…+50 as `dd + 35`; −50…−31 as `dd + 136`;
  unpack shows `dd − 101` for anything above +50 (`+99` → `−02`).
- Type 4 with `CQ`: h12 holds the hash of the call itself.
- `3DA0xxx` and `3XA0xx` calls are not standard calls (ft8_lib maps them
  to `3D0xxx`/`QA0xx`; WSJT-X 3.0 does not).
- Bracketed calls: `<call>` beside a standard call without suffix, or two
  bracketed calls neither of which contains `/` (special-event calls such
  as `<YW51MMZ> <JA9VHY>` included), is type 1. Beside a nonstandard or
  /R /P call, or two bracketed calls of which one is a compound `/` call,
  it is type 4 (first hashed, second sent in full).
- Type 5 (EU VHF contest): both calls bracketed; fields h12 (call 1),
  h22 (call 2), R1, r3 = report − 52 (52…59), s11 serial 0…2047, g25 =
  6-character locator as mixed radix 18·18·10·10·24·24 (subsquare letters
  A–X). Text `<A> <B> [R] RRSSSS LLNNLL` with a 4-digit serial.

Refused by this crate (WSJT-X loses information):

- Free text over 13 characters (WSJT-X truncates).
- A report in a type 4 message (dropped).
- An unbracketed compound call beside a standard one (`PJ4/K1ABC W9XYZ` is
  sent as `K1ABC W9XYZ`).
- Reports beyond ±50 (wrap on unpack).
- `CQ <standard call>` (WSJT-X packs it as type 1 and cannot unpack it).
- Type 5 serial above 2047 (WSJT-X clamps to 2047).

- `+1` (a sign then hex digits) is sent as telemetry `1`, dropping the
  `+`. This crate sends it as free text `+1` (lossless).

Oracle quirks:

- ft8sim prints nothing for type 4 messages whose full call is a
  special-event call without `/` beside a hashed call
  (`<OH3IE> TM16VMNE RR73`, `YW42DJ <G8YET> RRR`), though `CQ YW18FIFA`
  works.

- `K1ABC W9XYZ R 570007 JO22DB` (unbracketed calls) packs as type 3 and
  unpacks as `*** bad message ***` with all-zero symbols.
- `<KA1ABC> YW18FIFA RRR` prints no decode.
- `<G4ABC> <PA9XYZ> 520000 AA00AA` is sent as truncated free text, while
  `520001 AA00AA` is type 5.

## Hash collisions

- 10- and 12-bit hashes collide often enough to see in tests (about 1 in
  1 000 random type 5 / DXpedition messages): the protocol cannot tell
  the two calls apart and a hashed field may show the other one. ft8sim
  does the same (`<PA3XYZ> <G4ABC/P> 590123 IO91NP` unpacks with
  `<G4ABC/P>` twice). The round-trip test skips and counts such messages.

## What jt9 remembers for hashes (WSJT-X 3.0, black box)

Decoding several files in one `jt9` run, with fresh state per run (note:
`K1ABC` and `W9XYZ` resolve with nothing heard — jt9's default calls — so
the probe used `N7QQ`, `G4XYZ`, `W1AW`):

- Type 1/2: only the second (sending) call is remembered; `N7QQ G4XYZ
  EN37` does not make `<N7QQ>` resolve, `G4XYZ N7QQ EN37` and `CQ N7QQ
  EN37` do.
- Calls are remembered as written: after `G4XYZ N7QQ/P R-10`, `<N7QQ/P>`
  resolves and `<N7QQ>` does not. `/R` could not be tested (jt9 does not
  decode `/R` messages in normal mode); assumed the same.
- Type 4: the full call is remembered when it is the sender
  (`<G4XYZ> PJ4/N7QQ RRR`, `CQ PJ4/N7QQ`), not in `PJ4/N7QQ <G4XYZ> RRR`.
- DXpedition: neither call is remembered.

`unpack` matches this; `pack` remembers every call the operator typed.

## jt9 behaviour

- jt9 (normal mode, `-d 2`) does not decode `/R` messages even from
  ft8sim's own audio.
- `/P` (type 2) at −15 dB decodes 8/10 for both ft8sim's audio and ours.

## Measurement notes

- `Instant::now()` costs about 3 µs on the development host, so stage
  times are measured as batches.

## Decoder sizing (planning, 2026-10-26, revised after review; estimates until measured)

Geometry (12 kHz, 15 s slot = 180 000 samples):
- Window 3 840 samples (3.125 Hz bins), hop 960 (half a symbol). With
  the row origin at sample 240, row k covers [240 + 960k, +3 840):
  ⌊(180 000 − 240 − 3 840) / 960⌋ + 1 = 184 rows. A signal starting at
  the nominal 0.5 s (sample 6 000) has its first symbol centre at 6 960 =
  240 + 960·5 + 1 920, the centre of row 5; with a zero origin it would
  fall a quarter hop (240 samples) off the grid.
- Every symbol centre has a row for dt ∈ [−0.40, +1.76] s; the sample
  extent alone (a symbol's 1 920 samples overlapping some row) would
  allow [−0.48, +1.84] s.
- Row centres run from 2 160 to 177 840. The 79-symbol signal (151 680
  samples; last symbol centre 149 760 after the first) fits entirely for
  dt ∈ [−0.40, +1.76] s. At dt = −2.5 s about 13 leading symbols fall
  before row 0 (the first Costas array and six data symbols); at +2.5 s
  about 5 trailing symbols fall after the last row (most of the last
  Costas array). Hence partial sync: at least two of three Costas arrays
  scored, and erasures for absent data symbols.
- Band 100–3100 Hz: 961 base bins; stored spectrum adds 43.75 Hz (14
  bins) and a guard bin: 976 bins. Band 200–2900 Hz: 865 base, 880 stored.
- Local-maximum ties: integer scores make plateaus likely. Rule: keep a
  point if ≥ every later neighbour in (row, bin) order and > every
  earlier one; a plateau keeps its first point; outside-grid neighbours
  impose nothing. Partial-sync scores × 6 / n (n arrays present) keep one
  integer scale.
- Spectrogram: 184 × 976 × 1 B = 179 584 B (8-bit); × 2 = 359 168 B
  (16-bit); small band 8-bit 161 920 B.
- Sync scores for local-maximum suppression: a rolling buffer of three
  rows of base bins, i32: 3 × 961 × 4 = 11 532 B (small: 10 380 B). A full
  score grid would be 961 × 63 time positions × 4 B ≈ 242 KB.
- FFT in place with a digit-reversal table (1 920 u16 = 3 840 B flash);
  Stockham would add 15 360 B RAM.
- Decode entries without text ≈ 40 B (payload 16, frequency, time offset,
  SNR, score, candidate index, hard errors, rounds, flags). `MessageText`
  is 65 B; rendering text in `finish` avoids 64–128 of them.
- Totals: default (100–3100 Hz, 8-bit, 128 decodes) ≈ 233 812 B; small
  (200–2900 Hz, 8-bit, 64 decodes) ≈ 212 436 B; default with 16-bit
  levels ≈ 413 396 B. Caller-owned and not counted: call table (1 552 B
  for 64), audio, slot-boundary FIFO (i16: 24 000 B per second).
- Flash tables ≈ 35 KB (twiddles 15 360, real-split twiddles 7 680, half
  Hann window 7 684, digit reversal 3 840).

All-zero codeword: with a zero initial value the CRC of the all-zero
payload is zero, and the all-zero word satisfies every parity check, so
a carrier near a candidate can "decode" as an all-zero payload. It must
be rejected explicitly.

`ft8_lib` (consulted) uses the same two-fold oversampling in time and
frequency, keeps up to 140 candidates, runs 25 message-passing rounds and
drops candidates under a sync score of 10; it stores 8-bit levels.

Operation-count estimates for a Cortex-M33-class part at about 150 MHz:
- 1 920-point complex FFT ≈ 1 920 × log2(1 920) × 5 ≈ 105 000 flops, a
  few hundred thousand cycles: about 3 ms per 80 ms hop (≈ 4% of real
  time); log2 of about 976 bins per hop adds well under 1 ms.
- Sync search: about 961 base bins (already on the half-bin grid) × 63
  time positions × 21 Costas symbols × about 4 operations ≈ 5 million
  operations: well under 0.1 s. (An earlier note multiplied by 2
  sub-bins again, overcounting.)
- Belief propagation: 25 rounds × 522 edges × 2 directions × ~20 cycles
  ≈ 0.5 M cycles per candidate; 140 candidates ≈ 0.5 s.

No QEMU is installed on the development host (the user will install it;
work does not wait for it). QEMU gives instruction counts, not timing;
timing needs a development board, chosen later. QEMU counts are needed
from D2 (FFT per hop) and D3 (front end per 80 ms hop), and again at D4–D6
(sync search, belief propagation per candidate) and D10–D11 (whole
decode); until then those figures stay estimates.

## Front end, measured at the D3 checkpoint (2026-10-26)

- Sizes (`size_of`): `FrontEnd` 30 720 B; spectrogram default (100–3100
  Hz) 8-bit 179 584 B, 16-bit 359 168 B; small (200–2900 Hz) 8-bit
  161 920 B. Static tables of the front end (flash): 34 572 B.
- Link check (thumbv8m, small configuration in statics): `.bss` 223 360 B
  = 161 920 + 30 720 + 30 720 (the last only for the stand-alone
  `power_spectrum` call); `.text` 27 042 B; `.rodata` 48 466 B. The
  library's own frames stay small (the FFT's `spectrum` 152 B); `_start`
  is 4 216 B including the check program's two 1 920 B sample blocks.
- FFT accuracy against an f64 DFT: complex 1.3e-7 of peak; windowed real
  power 2.7e-7 of peak, ≤ 2.9e-5 dB within 60 dB of peak.
- `feed`: 77–80 µs per 80 ms hop on an AMD Threadripper 3970X (≈ 0.1% of
  real time), dominated by the FFT, which uses a generic small-DFT kernel
  with an index modulo per term (a candidate for a later optimisation
  pass: radix-specific kernels, incremental twiddle indices).
- 8-bit versus 16-bit: for hard decisions, 8-bit loses only through ties
  (2–5% of symbols at −26…−20 dB); split fairly, the difference is
  +0.05 points (within noise). Soft bits work on level differences, where
  ties give zero rather than wrong. To recheck with soft bits and belief
  propagation at the D6 checkpoint.
- Clipping: the −128…0 dB range (both widths) floors input noise below
  about −80 dBFS RMS; 16 bits do not change that. Inputs that quiet are
  rare (16-bit audio's own dither is near −101 dBFS) but possible.
- QEMU instruction counts are still to come (the user is installing it):
  first for `feed` per hop (now), then D4–D6 and D10–D11.

## D3 checkpoint decisions and follow-ups (2026-10-26)

- One public level width: 8-bit. The 16-bit form exists only for tests
  and the `profile` bench, as the baseline for D6; delete it at D6 unless
  belief propagation shows a gain beyond noise.
- The level range stays −127.5…0 dB relative to full scale (a full-scale
  sine centred in a bin). Shifting it 20 dB lower would clip strong
  signals. Levels start to floor when per-bin noise is below about
  −105 dB (about −80 dBFS RMS for white noise); in practice the audio
  codec's noise floor is the real lower limit. Typical operating range:
  −40…−15 dBFS RMS, peaks at or below −6 dBFS. No minimum is enforced;
  the spectrogram counts levels at the floor and at the top (and the
  decode report will).
- D5/D6 review checklist: every argmax or hard decision on 8-bit levels
  splits ties without bias (no systematic first-index win); soft bits are
  zero on ties.
- Before D6: the QEMU instruction count for `feed` per hop. The generic
  FFT kernel does about 42 000 complex multiply-adds per hop with an
  integer modulo per term; one estimate puts it at 10–15% of real time at
  150 MHz, against 4% in the ADR (budget 25%). If above about 10%,
  schedule radix-specific kernels. Measure stack (`check.sh --stack`) at
  every checkpoint from D4 on.

## D6 decision: 8-bit levels only (2026-10-26)

- Recheck with soft bits and belief propagation at the D6 checkpoint
  (`examples/decode_bench.rs -- levels 2000` at commit 2546586; first pass
  on the coarse grid, one signal per slot at a random off-grid frequency
  and time offset, seeded white noise, LLR scale 4, 30 rounds, K = 200,
  band 100–3100 Hz; the same 2 000 slots for both widths at each SNR):

  | Rule | SNR dB | 8-bit | 16-bit |
  |---|---|---|---|
  | sum-product | −20 | 80 | 86 |
  | sum-product | −19 | 615 | 622 |
  | sum-product | −18 | 1 423 | 1 428 |
  | sum-product | −17 | 1 849 | 1 845 |
  | min-sum 0.75 | −20 | 72 | 77 |
  | min-sum 0.75 | −19 | 571 | 581 |
  | min-sum 0.75 | −18 | 1 394 | 1 401 |
  | min-sum 0.75 | −17 | 1 840 | 1 837 |

  Totals over 8 000 slots: sum-product 3 967 against 3 981, min-sum
  3 877 against 3 896, about +0.2 points for 16-bit, within noise (no
  false decodes either way). This agrees with the D3 hard-decision
  result (+0.05 points with ties split).
- Decision: delete the 16-bit level type (it doubled the largest buffer
  for no measurable gain). `Level` stays a sealed trait, implemented for
  `u8` only, so a wider form could return later without an API change;
  the `levels` bench mode and the front-end bench's level-width section
  are removed with it. The measurement remains reproducible at
  2546586.

## D7 measurements (2026-10-26)

Decode bench (`examples/decode_bench.rs`, `profile`), one signal per slot
from this crate's encoder, seeded noise, band 100–3100 Hz, 300 slots per
point unless stated, host AMD Threadripper 3970X.

- **Grid offset costs** (grid soft bits, decodes of 300 at −20 / −19 /
  −18 dB): on the grid 61 / 223 / 292; half a bin off 3 / 57 / 211; half
  a row off 1 / 19 / 132; both 0 / 1 / 15. The 50% point on the grid is
  about −19.4 dB against about −18.6 dB for random offsets.
- **Refinement** (interpolated soft bits at the estimated offsets): half
  a bin 52 / 203 / 293; half a row 24 / 137 / 275; both 12 / 104 / 251;
  random offsets 29 / 179 / 285 (grid: 17 / 101 / 215). True offsets
  (oracle) gave 32 / 174 / 259 for random offsets — no better, since the
  oracle refines only candidates within half a step of the signal.
- **Decode rate, sum-product, before → after refinement** (decodes of
  300): −21 dB 0 → 1, −20 dB 17 → 29, −19.5 dB 50 → 98, −19 dB 101 → 179,
  −18.5 dB 158 → 247, −18 dB 215 → 285, −17 dB 277 → 299, −16 dB 292 →
  300, −14 dB 300 → 300. 50% point about −18.6 → −19.2 dB. Min-sum after:
  0 / 27 / 90 / 174 / 245 / 284 / 299 / 300 / 300 at −21…−14 dB. False
  decodes: 0 at every point; 0 in 3 000 noise-only slots; 0 in 3 000
  carrier-plus-noise slots (17 601 all-zero codewords rejected there).
- **SNR estimate** after calibration (200 slots per SNR, mean error by
  500 Hz band 200–2 800 Hz): white noise −0.3…+0.1 dB from −18 to +20 dB
  (RMS about 0.5 dB); shaped noise (2nd-order high-pass at 400 Hz,
  4th-order low-pass at 1 800 Hz: −12 dB at 200 Hz, −20 dB at 2 800 Hz)
  −0.3…+0.5 dB, the 200–700 Hz band reading +0.3…+0.5 dB on the steepest
  slope. At −20 dB only the decodes that succeed are counted, so the mean
  reads +0.1…+1.2 dB high (selection). A 2-bin gap before the reference
  compressed the estimate (−4.2 dB at +20 dB, uncalibrated) through the
  signal's own skirts; the 8-bin gap keeps it linear.
- **Frequency and dt** (same runs): RMS 0.22–0.47 Hz (max 1.22 Hz),
  RMS 5–8 ms (max 27 ms).
- **Cost** (one thread): refined soft bits 9.4 µs per candidate (grid
  1.2 µs; 26.6 µs before the level-to-power table); sum-product BP 375 µs
  and min-sum 130 µs per candidate at 30 rounds; whole decoder (feed +
  `decode()`, 200 candidates) 94 ms per slot with `HOST`, 45 ms with
  `SMALL`.
- **Memory**: `Decode` 48 B, `CandidateScratch` 5 100 B, `Candidate` 8 B;
  link check (small band, 64 decodes, all buffers static) `.bss`
  212 284 B: 262 144 − 212 284 ≈ 49.9 KB left on a 256 KiB part before
  the stack (≈ 4.2 KB measured for `_start`) and the 1 s i16 FIFO
  (24 000 B), so about 21 KB spare.

## D8 measurements (2026-10-26)

- **SNR in busy slots** (decode bench mode `busy`, 200 slots per row,
  estimate − truth over the decoded signals, same seeds before and after):

  | Slot | D7 reference (pooled median, 8 bins/side, 25–50 Hz) | D8 reference (per-side lower octile, 24 bins/side, 25–100 Hz, sides averaged) |
  |---|---|---|
  | one signal at −10 dB, a 10 dB stronger one 30–60 Hz above | mean −2.99, RMS 3.13, min −5.0; 58 of 109 below −3 dB | mean −0.75, RMS 0.97, min −2.0; none below −3 dB |
  | 10 signals (−16…+6 dB) | mean −0.79, RMS 1.77, min −14.6 | mean −0.73, RMS 1.34, min −8.2 |
  | 20 signals | mean −1.75, RMS 3.17, min −21.5 | mean −1.47, RMS 2.14, min −10.1 |
  | 40 signals | mean −3.80, RMS 5.59, min −23.6 | mean −2.96, RMS 3.74, min −20.3 |

  With 40 signals in 2 600 Hz (one per 65 Hz) the whole reference is
  raised and the estimate still reads low: that is interference, which a
  local reference cannot separate from noise.
- **Rejected alternatives** (same runs): a pooled lower quartile over 16
  bins was better in crowds (40 signals −2.65) but read 13 dB high on a
  steep passband edge, where the quieter side decides; an in-band
  reference (unsent tones of each symbol) compressed badly (−15 dB at
  +20 dB) from the signal's own leakage, and was worse in crowds.
- **Steep passband** (noise through a 600–1 600 Hz windowed-sinc
  band-pass, −6 dB at the edges, −43 dB 100 Hz outside, over a floor 45 dB
  down; signals 10 dB below the local noise; three seeds): in the
  passband (900, 1 250 Hz) within ±0.6 dB; at the edges 650 Hz +2.3…+2.8,
  1 500 Hz +2.0…+3.3, 1 560 Hz −1.9…−2.5 dB (D7 reference: 650 Hz
  +1.8…+2.7, 1 500 Hz −0.6…−1.6, 1 560 Hz +3.5…+4.4).
- **Calibration** unchanged (+2.3 dB): white noise −0.3…+0.1 dB in every
  500 Hz band from −18 to +20 dB, shaped noise −0.6…+0.1 dB.
- **Decode rates unchanged** (SNR is computed after decoding): the sweep
  reproduces D7 exactly for both rules, as do the carrier all-zero counts.
- **Pinned floors** (`tests/decode.rs`, 20 slots per SNR): HOST 6 / 16 /
  20 and SMALL 6 / 15 / 20 at −19.5 / −18.5 / −17 dB, no false decodes.
- **Golden decode hashes** (busy slot, carrier slot, busy slot cut at
  13.5 s): HOST `0x5eea347773cfdf7d`, SMALL `0x1c826afc8074f291`
  (Linux x86-64; Windows and aarch64 to be confirmed by CI).
- **Overlaps** 10, 20 and 30 Hz apart, same or 0.5 s different start:
  equal signals at −8 dB both decode; with 12 dB between them only the
  strong one decodes (the weak one needs subtraction), with no false
  decode.
- **Bulk** (release, all threads of the 3970X, 32 s): 2 000 noise-only and
  1 000 carrier-plus-noise slots per configuration, 0 decodes; 5 162
  (HOST) and 5 173 (SMALL) all-zero codewords rejected in the carrier
  slots.

## D10 sizes (2026-10-26, thumbv8m.main-none-eabihf, opt-level s, LTO)

| Image | `.text` | `.rodata` | flash | `.bss` (static RAM) | largest frames |
|---|---|---|---|---|---|
| small (200–2900 Hz, min-sum, 64 decodes) | 33 334 | 50 102 | 83 436 | 212 280 | `_start` 4 096, decoder path 2 512, `unpack` 464, `finish_row` 232, `encode_payload` 192 |
| host (100–3100 Hz, sum-product, 128 decodes) | 33 330 | 50 102 | 83 432 | 233 656 | the same |
| encoder alone | 22 186 | 13 666 | 35 852 | 0 | `_start` 4 216 |

- The decoder adds 47.6 KB of flash (36.2 KB of it static tables). Both
  check-node rules are in either image (the rule is a run-time field).
- The decoder path's 2 512 B frame includes the check program's 1 920 B
  feed block, which a real device would hold in its FIFO instead.
- On a 256 KiB part the small image leaves 49.9 KB of RAM; less the 1 s
  i16 FIFO (24 000 B) and a few KB of stack, about 21 KB is spare. The
  host configuration's 233.7 KB does not leave room for the FIFO.
- Host command (`cat-ft8-decode --profile`, one slot, one signal):
  `peak_bytes` 236 232 (spectrogram 179 584, front end 30 720, sync
  scratch 11 532, decode list 6 144, candidate scratch 5 100, candidates
  1 600, call table 1 552); total 94 ms on the 3970X with `host`.

## D12 sizes (2026-10-27, thumbv8m.main-none-eabihf, Rust 1.93, opt-level s, LTO)

| Image | flash | `.bss` | largest frames |
|---|---|---|---|
| small (200–2900 Hz, min-sum, 64 decodes) | 83 412 B (83 404 B in the `--stack` build; before `CHECKS`/`DEGREES` became statics, 83 404 B and 83 408 B) | 212 280 B | `_start` 4 096, decoder path 2 496, `unpack` 464, `finish_row` 232 |
| host (100–3100 Hz, sum-product, 128 decodes) | 83 412 B (83 404 B with `--stack`) | 233 656 B | the same |
| encoder alone | 35 848 B | 0 | `_start` 4 216 |

`decode::TABLE_BYTES` is 36 260 B (the parity-check list replaced by
`CHECKS` and `DEGREES`). Flash moves by a few bytes with each change;
the ADR gives it rounded.

## Phase 3 E0 baseline (2026-10-27, re-recorded at E0a)

**Provenance.** `examples/paired_bench.rs` as of the E0a commit, decoder
library identical to 255876b (each `config.txt` says
`255876b…+dirty` because the bench changes were not yet committed);
rustc 1.93.0 (254b59607 2026-01-19); AMD Ryzen Threadripper 3970X; 16
threads for `run`. Commands: `run host|small DIR sweep,crowded,edges,empty
main` and `run host|small DIR sweep,crowded fresh`. Files (outside the
repository, regenerated bit-for-bit by the same commands) in
`~/.cache/cat-ft8-paired/e0a/{host,small}/`, sha256:

| file | host | small |
|---|---|---|
| sweep-main | 07751db2983edc4d786c159980d170d1ff1e8d8784234847f054f31cd4eb3ecc | 98f8e1c1e160e31228ac78ed1e30236ebb1536d926fcd84b23daec2a3b7c1d8e |
| sweep-fresh | e4afa7c01ba7a25be1da052d0412546070b3b0cad45a79f96a0fd5bb7252ce76 | 7947cb3bbc913a48ffc64f7dab3bd49f173ffb6806c60cc77ffdbfd5316f932f |
| crowded-main | c875d8215d0fc6944c748bbeb041045a6e56238229a78ee6391e67e648f46b78 | 015f31a08f4e955eed735003861f4aa8338a49e5f7cb883147d9ec205b15602e |
| crowded-fresh | 1a1a7a762db78b60c58eb062b81ff876680a3483072772ba078229619690cdbe | 294138c179c503d7c74bce4b1872a312c7016df1754d311e35d032d7cc8027ca |
| edges-main | 8ca4d6bef5ae98db332f7e4125cc62364540108fa28bdc56620ab68dd47df3c0 | 1b311324821f2d223e9076d2cb222333bcb032e9b583c6b5bdc62030ce5a284d |
| empty-main | 4c0a8404e1fc8722888efa5968d5abb4cf2907bfd9d08960b214cb0865d2facd | 4a696c70619a5af57df5935b5c4a421e323ae7564d1be3b267f7711f6ed5f295 |

(The stage times inside the files vary run to run, so a re-run has other
sha256 values; the decode content does not change — a re-run of host
pairs with zero discordant messages on every set.) The slots differ from
the first E0 recording because slot construction now uses the
integration tests' shared helpers (their uniform draw differs from the
bench's former copy); each slot's sample hash now pins its input.

Configurations: `host` = 100–3100 Hz, sum-product, 128 decodes; `small`
= 200–2900 Hz, min-sum, 64 decodes; both 30 rounds, LLR scale 4, 200
candidates.

**Sweep** (1 000 slots per point; messages decoded, host / small):

| SNR dB | −22.0 | −21.5 | −21.0 | −20.5 | −20.0 | −19.5 | −19.0 | −18.5 | −18.0 | −17.5 | ≥ −17.0 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| main | 0 / 0 | 1 / 1 | 8 / 6 | 41 / 34 | 124 / 109 | 292 / 280 | 567 / 544 | 795 / 788 | 949 / 949 | 988 / 987 | 1000 / 1000 |
| fresh | 1 / 0 | 0 / 0 | 12 / 10 | 35 / 30 | 114 / 102 | 322 / 305 | 533 / 519 | 835 / 823 | 941 / 934 | 981 / 979 | 998–1000 / 998–1000 |

50% point (paired bootstrap 95% interval, no resample dropped): host main
−19.122 dB [−19.163, −19.072], fresh −19.078 dB [−19.133, −19.010];
small main −19.083 dB [−19.128, −19.024], fresh −19.044 dB [−19.101,
−18.981].

**Crowded** (200 slots per size; transmitted messages decoded, host /
small): main 10 → 1755 / 1753 of 2 000, 20 → 3102 / 3099 of 4 000, 40 →
4815 / 4804 of 8 000; fresh 1758 / 1756, 3082 / 3080, 4829 / 4812.

**Edges** (−18 dB, 100 slots per case; host / small): dt −2.4 s 94 / 92,
−2.0 s 85 / 84, −1.6 s 96 / 96, +1.6 s 98 / 97, +2.0 s 97 / 97, +2.4 s
98 / 98; tone 0 at 100 Hz 90 / 0 and 3 090 Hz 93 / 0 (outside the small
band), 200 Hz 90 / 91, 2 890 Hz 92 / 90; cut at 13.5 s 99 / 98; 1 s late
97 / 97; dt 0 99 / 98.

**False decodes:** none in any set, block or configuration (sweep,
crowded, edges, and 2 300 empty slots). No decodes truncated.

**Deterministic cost:** 200 candidates through belief propagation per
slot; rounds per candidate 29.92 (sweep), 27.61 (crowded host; 27.62
small), 29.87 (edges), 29.59 (empty).

**Time** (`paired_bench cost host small 42 5`, loop-aligned build
(`-C llvm-args=-align-loops=64`), median of five interleaved
single-thread runs over 42 crowded slots — 14 each of the 10-, 20- and
40-message sizes, indices 0–13, 200–213 and 400–413 — ms per slot): host
total 67.1 (belief propagation 44.7, feed 14.8, soft bits 3.2, sync 2.9,
refine 1.4, SNR 0.2); small total 45.3 (belief propagation 23.6). This is
the reference for E1's belief-propagation target.

**Sizes** (thumbv8m.main-none-eabihf, Rust 1.93, opt-level s, LTO): small
image flash 83 504 B (83 496 B in the `--stack` build), `.bss`
212 280 B; host image 83 504 B, 233 656 B; frames `_start` 4 096 B,
decoder path 2 544 B, `unpack` 464 B, `finish_row` 232 B.

**Gate demonstration:** host against small flags small's real losses —
sweep b − c 67 (main) and 72 (fresh) against limits 22 and 21, crowded
over a set, the band-edge cases — with its 50% point 0.034–0.038 dB
worse. Failure cases exit with status 1: a set missing from one run, an
empty directory, one slot's sample hash changed.

## Phase 3 E1: early-stop sweep (2026-10-27)

Each configuration ran all four main sets and the fresh block
(sweep and crowded) with `paired_bench` at 3c7bde9 and was compared with
the E0a baseline of its preset (`host` or `small`). Columns: b − c
against the set limit for sweep (main / fresh) and crowded (main /
fresh), points failing the per-point limit (L), the sweep's 50% point
shift (main / fresh), and rounds per candidate on the sweep (baseline
29.92). Empty: zero decodes in every configuration; no false decode
anywhere, in any set or configuration.

| config | sweep b − c / limit | crowded b − c / limit | edges | 50% shift dB | rounds |
|---|---|---|---|---|---|
| host stall 2 | 593/49 L8, 557/47 L7 | 145/24, 126/22 | 107/21 L7 | +0.307, +0.245 | 6.49 |
| host stall 3 | 388/39, 354/38 | 86/18, 76/17 | 59/15 | +0.212, +0.169 | 8.73 |
| host stall 4 | 283/34, 258/32 | 62/16, 53/15 | 38/12 | +0.153, +0.135 | 10.63 |
| host stall 5 | 212/29, 185/27 | 46/14, 36/12 | 32/11 | +0.112, +0.101 | 12.30 |
| host stall 6 | 168/26, 142/24 | 40/13, 30/11 | 24/10 | +0.094, +0.080 | 13.81 |
| host stall 8 | 118/22, 99/20 | 26/10, 16/8 | 15/8 | +0.063, +0.054 | 16.51 |
| host stall 10 | 90/19, 60/16 | 18/8, 13/7 | 13/7 | +0.046, +0.031 | 18.92 |
| host stall 12 | 67/16, 45/13 | 10/6, 10/6 | 8/6 | +0.035, +0.020 | 21.10 |
| host stall 15 | 36/12, 26/10 | 3/4, 3/4 | 6/5 | +0.017, +0.012 | 23.98 |
| host stall 20 | 11/7, 9/6 | 0/0, 2/3 | 1/2 | +0.009, +0.002 | 27.77 |
| host cap 10 rounds | 302/35, 280/34 | 65/16, 62/16 | 42/13 | +0.175, +0.144 | 9.98 |
| host cap 15 rounds | 148/24, 118/22 | 25/10, 30/11 | 17/8 | +0.077, +0.076 | 14.96 |
| host cap 20 rounds | 69/17, 57/15 | 17/8, 11/7 | 10/6 | +0.031, +0.035 | 19.95 |
| small stall 2 | 595/49, 561/47 | 136/23, 121/22 | 93/19 | +0.315, +0.231 | 6.01 |
| small stall 3 | 380/39, 335/37 | 89/19, 62/16 | 49/14 | +0.199, +0.146 | 8.33 |
| small stall 4 | 262/32, 251/32 | 54/15, 42/13 | 27/10 | +0.137, +0.124 | 10.50 |
| small stall 5 | 194/28, 169/26 | 43/13, 33/12 | 20/9 | +0.097, +0.086 | 12.52 |
| small stall 6 | 152/25, 119/22 | 33/12, 24/10 | 17/8 | +0.072, +0.062 | 14.45 |
| small stall 8 | 91/19, 64/16 | 21/9, 14/8 | 9/6 | +0.050, +0.037 | 18.01 |
| small stall 10 | 54/15, 36/12 | 9/6, 9/6 | 7/5 | +0.027, +0.023 | 21.12 |
| small stall 12 | 37/12, 23/10 | 6/5, 7/5 | 7/5 | +0.016, +0.012 | 23.69 |
| small stall 15 | 16/8, 10/6 | 3/4, 3/4 | 4/4 | +0.008, +0.004 | 26.49 |
| small stall 20 | 4/4, 2/3 | 1/2, 1/2 | 1/2 | +0.002, +0.002 | 29.01 |

Every host configuration fails the no-loss gate on the sweep (main and
fresh); only small with stall 20 passes every gate, saving 3% of the
rounds. Cost (`paired_bench cost`, loop-aligned, median of five runs over
the 42 spread crowded slots): host stall 10 cuts belief propagation by
34.4% (total −23.2%), stall 15 by 18.3% (total −12.7%) — so no setting
that comes near passing reaches the 20% target.

Why: successful decodes near threshold converge slowly. In the host
baseline sweep, 302 of 8 765 decodes needed more than 10 rounds, 148
more than 15 and 69 more than 20 (crowded: 65, 25 and 17 of 9 672). The
messages host stall 15 loses on the main sweep all needed 20–30 rounds
in the baseline; their unsatisfied-check count plateaus for 15 rounds or
more before they converge. None of them is decoded through another
candidate (each is lost from the output), and the stall rule gains no
message.

Conclusion: early stop on a stalled unsatisfied-check count, as
specified, cannot deliver a 20% belief-propagation saving without a loss
the paired gates detect. No default change; nothing re-pinned. **E1
rejected; the mechanism (3c7bde9) reverted in 42e551e.**

Raw runs (outside the repository) are in `~/.cache/cat-ft8-paired/e1/`,
one directory per configuration (`host_stall_K`, `host_rounds_N`,
`small_stall_K`) with each comparison's output beside it
(`<config>.compare.txt`); the baseline is `~/.cache/cat-ft8-paired/e0a/`.
They were made at 3c7bde9 with, for each configuration CONFIG:

```text
paired_bench run CONFIG DIR sweep,crowded,edges,empty main
paired_bench run CONFIG DIR sweep,crowded fresh
paired_bench compare ~/.cache/cat-ft8-paired/e0a/<host|small> DIR
paired_bench cost host host,stall=K 42 5      (loop-aligned build)
```

where CONFIG is `host,stall=K`, `host,rounds=N` or `small,stall=K`
(the `stall` key existed only at 3c7bde9).

## Phase 3 E3: candidate-count sweep (2026-10-27)

Stall off; baseline E0a (200 candidates). Each configuration ran all
four main sets and the fresh block with `paired_bench` at 53e4a7d
(`run <host|small>,candidates=K DIR …`, then `compare` against
`~/.cache/cat-ft8-paired/e0a/<host|small>`); raw runs in
`~/.cache/cat-ft8-paired/e3/` with each comparison beside them.

| config | sweep b − c / limit (main, fresh) | crowded | edges | empty | 50% shift dB (main, fresh) | candidates through BP (crowded main) |
|---|---|---|---|---|---|---|
| host 150 | 1/2, 0/0 | 0/0, 0/0 | 0/0 | 0 | +0.000, +0.000 | 90 000 |
| host 120 | 1/2, 0/0 | 0/0, 0/0 | 0/0 | 0 | +0.000, +0.000 | 72 000 |
| host 100 | 1/2, 0/0 | 0/0, 0/0 | 0/0 | 0 | +0.000, +0.000 | 60 000 |
| host 75 | 1/2, 1/2 | 0/0, 0/0 | 0/0 | 0 | +0.000, +0.002 | 45 000 |
| small 150 | 1/2, 0/0 | 0/0, 0/0 | 0/0 | 0 | +0.000, +0.000 | 90 000 |
| small 120 | 1/2, 0/0 | 0/0, 0/0 | 0/0 | 0 | +0.000, +0.000 | 72 000 |
| small 100 | 1/2, 0/0 | 0/0, 0/0 | 0/0 | 0 | +0.000, +0.000 | 60 000 |
| small 75 | 1/2, 0/0 | 0/0, 0/0 | 0/0 | 0 | +0.000, +0.000 | 45 000 |

(baseline: 120 000.) Every configuration passes every gate on both
blocks; no false decode anywhere. The one message lost on the main sweep
at every K (slot 4436, −20 dB) was decoded in the baseline by candidate
171 — beyond any of these cut-offs; host 75 also loses one on the fresh
sweep. Nothing is gained.

Cost (`paired_bench cost`, loop-aligned, median of five runs over the
42 spread crowded slots, ms per slot): host 200 → 75: total 66.8 → 33.6
(−49.8%), belief propagation 44.8 → 14.4 (−67.8%), soft bits and refine
−63%, feed and sync unchanged; host 100: total −39.1%; small 200 → 75:
total 45.3 → 26.9 (−40.7%).

Every message gained or lost at 75 against 200, all sets and blocks:
host main sweep, slot 4436 (−20 dB, "P5D B2Y RR73"), lost — found by
candidate 171 in the baseline; host fresh sweep, slot 6389 (−19 dB,
"CQ K2GPK BG95"), lost — candidate 78; small main sweep, slot 4436, lost
— candidate 151. Each is lost from the output (no other candidate finds
it); nothing is gained; crowded, edges and empty are identical in
decoded messages.

The external reference comparison of a 75-candidate host binary passed,
but with one message of 2 000 lost on its 40-signal crowded slots, a
margin of one signal over its crowded bar.

**Decision: `HOST` 100 candidates, `SMALL` 75** (`CANDIDATES`, the
buffer recommendation, stays 200). 75 was the lowest value swept, so it
has no measured margin below it; on the host it lost the fresh-block
message at candidate rank 78, and externally it had a one-signal margin
on 40-signal slots. 100 loses nothing on the fresh block and costs
little more. The small preset keeps 75: it passes both blocks, and its
budget matters more.

Confirmation at the chosen defaults (`paired_bench run host|small DIR
sweep,crowded,edges,empty main`, then `… sweep,crowded fresh`, then
`compare` against `~/.cache/cat-ft8-paired/e0a/<host|small>`; raw runs
and comparisons in `~/.cache/cat-ft8-paired/e3-final/`): both pass every
gate. Host 100: lost 1 (main sweep, slot 4436, candidate 171), gained 0,
false 0 on every set and block; small 75: lost 1 (the same slot,
candidate 151), gained 0, false 0. Both runs are identical in decodes to
the sweep's `host,candidates=100` and `small,candidates=75` runs (b = c
= 0 on every set). 50% point and its 95% paired-bootstrap interval
(1 000 resamples, from `compare`):

| preset | block | old (200) | new | shift |
|---|---|---|---|---|
| host 100 | main | −19.122 [−19.163, −19.072] | −19.122 [−19.163, −19.072] | +0.000 [+0.000, +0.000] |
| host 100 | fresh | −19.078 [−19.133, −19.010] | −19.078 [−19.133, −19.010] | +0.000 [+0.000, +0.000] |
| small 75 | main | −19.083 [−19.128, −19.024] | −19.083 [−19.128, −19.024] | +0.000 [+0.000, +0.000] |
| small 75 | fresh | −19.044 [−19.101, −18.981] | −19.044 [−19.101, −18.981] | +0.000 [+0.000, +0.000] |

Cost at the chosen defaults (`paired_bench cost host,candidates=200 host
42 5` and the same for small, loop-aligned build, median of five
interleaved runs over 42 crowded slots, one thread, ms per slot): host
200 → 100: total 66.4 → 40.4 (−39.1%), belief propagation 44.0 → 20.3
(−53.9%); small 200 → 75: total 44.8 → 26.3 (−41.4%), belief
propagation 23.5 → 7.7 (−67.3%). Feed and sync are unchanged.

External reference comparison of the 100-candidate host binary (built
from this tree with `cargo build --release -p cat-ft8 --bin
cat-ft8-decode`), against the last accepted decoder: every synthetic set
and the live session gained 0, lost 0, no false decode; crowded 1 262
against a bar of 1 260; live 69 found against 68; the golden check 60
of 60 slots; CPU per slot about half.

Golden slots: on every golden setup the defaults decode exactly the same
decodes as 200 candidates — payload, frequency, time offset, SNR and
score bit for bit, compared field by field — and the carrier slot still
has its all-zero rejection (1 with the defaults, 3 with 200; now
asserted). The hashes of the default setups differ from the 200 ones
only because the hash also covers the report counts (candidates kept
and dropped, not converged, all-zero rejected).

Pins:

- golden "HOST" (default band, `DecodeConfig::HOST`, 128 decodes): new
  pin `0xb989_1439_4016_eb1f` for 100 candidates (it replaces
  `0x8fb5_e2a8_bab1_a8c3`, the pin taken for 75 before the decision
  and never committed);
- golden "min-sum on the default band" (`SMALL`, 128 decodes):
  `0x9fd1_ce9b_bfce_8d9f`, new for 75 candidates;
- golden "small setup" (`Band::SMALL`, `SMALL`, 64 decodes):
  `0x4356_eee9_42f3_6a23`, new for 75 candidates;
- the three 200-candidate pins (`0x5eea_3477_73cf_df7d`,
  `0x1c82_6afc_8074_f291`, `0xdd6e_4bf0_8d7d_91f9`) are unchanged, now
  as explicit `with_candidates(200)` setups; golden counts unchanged
  (8/0/8, 7/0/7, 6/0/6 for each pair);
- rate floors: the 200-candidate configurations added to the floors'
  configurations; all four give the pinned counts (6/16/20 host,
  6/15/20 min-sum) and the floors are unchanged;
- the bulk run stays at zero decodes (fewer all-zero rejections: fewer
  candidates reach belief propagation).

## Phase 3 E2: host check-node rule, measured (2026-10-27)

Measurement only; no default changes. Stall off, at the E3 defaults
(`HOST` 100 candidates, `SMALL` 75). `paired_bench` at 8c410e0 (clean
tree): `run host,rule=ms DIR sweep,crowded,edges,empty main`, then `…
sweep,crowded fresh`, and the same for `small,rule=sp`; each compared
with the default run of its preset in `~/.cache/cat-ft8-paired/e3-final/`
(old = the default). Raw runs and comparisons in
`~/.cache/cat-ft8-paired/e2/`.

**`HOST`, sum-product (default) → min-sum as configured (normalised,
the current `ALPHA` = 0.75, 30 rounds).** Lost = b (decoded by
sum-product only), gained = c (by min-sum only):

| set (block) | lost | gained | b − c / limit | points failing | false (old, new) |
|---|---|---|---|---|---|
| sweep (main) | 94 | 28 | 66 / 22.1 | −20.0 dB (16/1), −19.0 dB (32/9) | 0, 0 |
| sweep (fresh) | 91 | 21 | 70 / 21.2 | −19.5 dB (20/3) | 0, 0 |
| crowded (main) | 23 | 7 | 16 / 11.0 | — (40: 17/6) | 0, 0 |
| crowded (fresh) | 24 | 3 | 21 / 10.4 | 40 messages (19/2) | 0, 0 |
| edges (main) | 11 | 1 | 10 / 6.9 | — (spread over 9 cases) | 0, 0 |
| empty (main) | 0 | 0 | 0 / 0 | — | 0, 0 |

50% point (paired bootstrap, 1 000 resamples, none dropped): main
−19.122 → −19.083 dB, shift +0.038 dB [+0.020, +0.061]; fresh −19.078
→ −19.044 dB, shift +0.034 dB [+0.012, +0.057]. Rounds per candidate
unchanged (29.84 sweep, 25.25 → 25.26 crowded). Min-sum fails the
no-loss gate on every set with signals, on both blocks; the shift is
inside the 0.05 dB limit but its interval excludes zero. Only this
min-sum was measured: other α values or round limits were not part of
E2.

**`SMALL`, min-sum (default) → sum-product** (75 candidates): the mirror
image, with the same counts — lost / gained: sweep main 28 / 94, fresh
21 / 91; crowded main 7 / 23, fresh 3 / 24; edges 1 / 11; empty 0 / 0;
no false decode. 50% shift −0.038 dB [−0.061, −0.020] (main), −0.034 dB
[−0.057, −0.012] (fresh). 302 of the 303 discordant messages are the
host's, with the sides swapped: the rule, not the band or the
candidate count, decides them.

**Cost** (`paired_bench cost host host,rule=ms 42 5` and `cost small
small,rule=sp 42 5`, loop-aligned build of 8c410e0, median of five
interleaved single-thread runs over the 42 spread crowded slots, ms per
slot):

| preset | rule | total | belief propagation | feed | sync | soft bits | refine |
|---|---|---|---|---|---|---|---|
| host | sum-product (default) | 40.56 | 20.58 | 14.64 | 2.87 | 1.61 | 0.68 |
| host | min-sum | 30.82 (−24.0%) | 10.90 (−47.1%) | 14.59 | 2.86 | 1.59 | 0.67 |
| small | min-sum (default) | 26.59 | 7.74 | 14.34 | 2.60 | 1.22 | 0.51 |
| small | sum-product | 33.28 (+25.2%) | 14.53 (+87.8%) | 14.27 | 2.60 | 1.23 | 0.52 |

**Reading.** Sum-product is worth about 0.035 dB at the 50% point and
about 65–70 sweep messages in 15 000 (0.5%), plus 15–20 crowded
messages in 14 000, for about 10 ms per crowded slot on this host
(belief propagation twice as long). For `HOST` the sum-product default
stands: min-sum as configured (normalised, the current `ALPHA`, 30
rounds) fails the no-loss gate on both blocks. *Assumption, not
measured:* sum-product is the better input for ordered-statistics
decoding. For `SMALL` the same numbers are the price of its budget:
sum-product would pass every gate there (it is the gain side) but costs
+88% belief propagation on the host. *Assumption, not measured:* it
costs more on a scalar FPU (transcendentals per edge); whether it fits
is a question for instruction counts on the small target, not this
host. No default change; nothing re-pinned.

## Phase 3 E4: continuing past a CRC-failing codeword, measured (2026-10-27)

Measurement only; the decoder is unchanged (default release binary
byte-identical to 8c410e0's). The measurement code was removed
afterwards (the bench keeps the CRC counts and the misses by cause, see
"Misses by cause" below); to reproduce E4, check out 1870e8b. Method:
`paired_bench crc CONFIG SETS BLOCK` decodes each slot as `run` does and, for every candidate whose
belief propagation stopped at a parity-satisfying codeword that fails
the CRC (or is all zero), recomputes its soft bits and runs belief
propagation again without stopping, to the 30-round limit
(`bp::decode_soft_observed`, `profile` builds only: a copy of
`decode_soft`'s loop that reports every round whose hard decisions are a
codeword; a test checks it stops where `decode_soft` does, bit for bit,
and the bench asserts the same on every candidate re-run). A candidate
is "on" a transmitted message when tone 0 is within 4 Hz and the start
within 0.12 s (every one of the decodes matched its own message this
way). Presets at the E3 defaults; all main sets, the fresh block of
sweep, crowded and empty, and 20 further seed blocks of the empty set
(`bulk01`…`bulk20`, 46 000 slots; the same six kinds of input as the
bulk test). Output in `~/.cache/cat-ft8-paired/e4/`.

**CRC failures** (candidates through belief propagation that end at a
codeword failing the CRC):

| set (block) | host: failures / candidates | per slot (max) | small: failures / candidates | per slot (max) |
|---|---|---|---|---|
| sweep (main) | 227 / 1 500 000 | 0.015 (2) | 132 / 1 125 000 | 0.009 (1) |
| sweep (fresh) | 221 / 1 500 000 | 0.015 (2) | 114 / 1 125 000 | 0.008 (1) |
| crowded (main) | 12 / 60 000 | 0.020 (2) | 7 / 45 000 | 0.012 (1) |
| crowded (fresh) | 14 / 60 000 | 0.023 (1) | 8 / 45 000 | 0.013 (1) |
| edges (main) | 15 / 130 000 | 0.012 (1) | 10 / 97 500 | 0.008 (1) |
| empty (main) | 37 / 230 000 | 0.016 (1) | 20 / 172 500 | 0.009 (1) |
| empty (fresh) | 52 / 230 000 | 0.023 (1) | 34 / 172 500 | 0.015 (1) |
| empty (20 more blocks) | 678 / 4 600 000 | 0.015 (2) | 383 / 3 450 000 | 0.008 (2) |

About 1.5 per 10 000 candidates on the host (min-sum about 1.1), the
same with and without signals, at every SNR: they are noise reaching
some codeword, not near misses. Their mean distance from the nearest
transmitted codeword is 86–87 bits of 174 on the sweep (a random
codeword's is about 87) and 72–76 bits on crowded slots (nearest of 10–40
messages); one per host sweep block, and one in the small main sweep,
is within 20 bits.

**Messages not decoded with a CRC-failing candidate on them:** host
sweep 2 of 6 236 missing (main) and 2 of 6 228 (fresh), crowded 2 of
4 328 and 0 of 4 331, edges 0 of 72; small sweep 1 of 6 303 and 1 of
6 300, crowded 2 of 4 344 and 0 of 4 352, edges 0 of 262. So at most
about one missing message in 2 000 even had a candidate that reached a
wrong codeword.

**Continuing past the failure.** 1 964 CRC-failing candidates continued
(1 256 host, 708 small) ran 25 421 further rounds (under 0.01% of all
rounds); in 25 104 of them (98.8%) the hard decisions were still a
codeword, and in every case the same one. **No continued candidate
reached a second codeword** — none correct, none wrong, none passing or
failing the CRC — on any set, block or preset. The same holds past an
all-zero rejection (79 474 host and 59 692 small candidates, nearly all
on carriers and clipped input): never another codeword. Belief
propagation continued past its first codeword never reached a different
codeword; in 1.2% of the extra rounds it briefly left the codeword and
returned.

**False-decode rate.** A codeword that is not the transmitted one passes
the 14-bit CRC with probability 1/16 384. The current decoder already
reaches such codewords at the rates above, so its expected false-decode
rate from belief propagation is about 0.015/16 384 ≈ 9 × 10⁻⁷ per slot on
the host (about one per 1.1 million slots; small 5 × 10⁻⁷), consistent
with no false decode in any set (50 600 empty slots per preset).
Continuing would add the wrong codewords it reaches, times 1/16 384:
none were reached, so the measured addition is zero; with 1 964
continued candidates the 95% upper bound (rule of three) is 0.0015 new
codewords per continued candidate, i.e. at most about 2 × 10⁻⁵ more
wrong codewords per slot and 1.4 × 10⁻⁹ more expected false decodes per
slot. The gain is bounded the same way: at most the 0–2 missing
messages per set that had a CRC-failing candidate on them, and none was
recovered.

**Recommendation: do not implement continuation past a CRC failure.**
It reaches nothing (the gain and the risk are both zero as measured),
and the cases it could address are at most one missing message in
2 000. CRC failures are already rare and cheap (a failing candidate
stops early; continuing would cost under 0.01% more rounds, so cost is
not the reason either). Recovering those few would need a search for a
*different* codeword near the soft bits (ordered-statistics decoding or
bit flipping on the least reliable bits), which is a separate step with
its own false-decode budget: every extra codeword tried adds 1/16 384,
so it needs the much larger empty-set run the plan names. Nothing
re-pinned; no default change.

Put plainly: the codewords that fail the CRC are noise, not near misses
(they sit as far from the transmitted codewords as random ones, at the
same rate with and without signals). A later ordered-statistics step is
therefore sized by the candidates that did not converge while sitting on
a transmitted message, not by the CRC failures: on the crowded main set
that is 3 486 of the host's 4 328 missed messages (840 had no candidate
on them, 2 a CRC failure), and 6 165 of 6 236 on the main sweep
(`compare`'s misses by cause, "Misses by cause" below).

## Misses by cause, and the E2/E4 clean-up (2026-10-27)

Results identical. The E4 measurement code is gone (`bp::decode_soft_observed`
and its tests; `paired_bench crc` and its continued runs; 1870e8b
reproduces E4). Two of its capabilities moved into `run` and `compare`:
`compare` prints each side's CRC failures per slot and the expected
false decodes they imply (CRC failures per slot × 2⁻¹⁴), and `run`
saves, per transmitted message, its outcome — decoded, or why not, by
position (the E4 tolerances: refined tone 0 within 4 Hz, start within
0.12 s; candidates that decoded some payload are left out; the furthest
any candidate on it got decides: CRC failure, then all-zero, then not
converged; otherwise no candidate on it). Runs saved before this record
no outcomes, and `compare` says "not recorded" for them.

Identity: the default release `cat-ft8-decode` is byte-identical
(sha256 b3ead07bca4a245ebeefeaae85f36ecb519fa794664d7c47266f7a318c8dea93,
the same as at 1870e8b and 8c410e0); the decoder sources equal 8c410e0's;
golden hashes and floors unchanged (`cargo test`); and `run host|small`
on every main set and the fresh block, compared with
`~/.cache/cat-ft8-paired/e3-final/`, gives b = c = 0 everywhere, the
same 50% points, and every slot record (seed, sample hash, messages,
decodes, report, rounds) equal field by field. Runs in
`~/.cache/cat-ft8-paired/s0/`.

**Missed messages by cause** (E3 defaults; no candidate / not
converged / CRC failure / all-zero, of the messages missed):

| set (block) | host | small |
|---|---|---|
| sweep (main) | 69 / 6 165 / 2 / 0 of 6 236 | 86 / 6 216 / 1 / 0 of 6 303 |
| sweep (fresh) | 61 / 6 165 / 2 / 0 of 6 228 | 72 / 6 227 / 1 / 0 of 6 300 |
| crowded (main) | 840 / 3 486 / 2 / 0 of 4 328 | 938 / 3 404 / 2 / 0 of 4 344 |
| crowded (fresh) | 853 / 3 478 / 0 / 0 of 4 331 | 947 / 3 405 / 0 / 0 of 4 352 |
| edges (main) | 0 / 72 / 0 / 0 of 72 | 200 / 62 / 0 / 0 of 262 |

(small's 200 edges without a candidate are the two cases outside its
band.) The CRC column matches E4's "missing with a CRC-failing candidate
on them". CRC failures per slot, from `compare`, as in E4: host 0.015
(sweep), 0.020–0.023 (crowded), 0.012 (edges), 0.016 (empty); expected
false decodes about 1 × 10⁻⁶ per slot; small about half.

Nearly every miss has a candidate on it that did not converge; on
crowded slots about a fifth have no candidate on them at all (not
broken down further; a stronger neighbour's sync peak suppressing
theirs, or the candidate limit, are the likely reasons). How many of either kind subtraction could recover is
measured next ("Subtraction ceiling").

## Subtraction ceiling (2026-10-27)

Measurement only; the decoder is unchanged. `paired_bench ceiling
host|small main|fresh` (bench as committed with this section; outputs
in `~/.cache/cat-ft8-paired/s0/ceiling-<preset>-<block>.txt`) decodes
every crowded slot of the block three ways, at the E3 defaults:

- **single pass**: the slot as `run` decodes it (the counts equal
  `e3-final` exactly);
- **alone**: each transmitted message by itself in a slot with the same
  noise realisation and the other signals left out — decoded alone or in
  the single pass is the **ceiling** of perfect subtraction;
- **oracle +1, +2**: the slot rebuilt without every message decoded so
  far (true waveforms, same noise), decoded once more, then again —
  what one or two further passes could gain with perfect reference
  waveforms.

The SNR bins are relative to the single-pass 50% point of the same
block's sweep (host −19.122 dB main, −19.078 fresh; small −19.083,
−19.044), computed by the same command.

**Decoded, % of transmitted** (single pass / oracle +1 / oracle +2 /
ceiling):

| preset, block | 10 messages (2 000) | 20 messages (4 000) | 40 messages (8 000) | all (14 000) |
|---|---|---|---|---|
| host, main | 87.8 / 99.0 / 99.7 / 100 | 77.5 / 95.7 / 98.8 / 100 | 60.2 / 86.7 / 94.5 / 100 | 69.1 / 91.0 / 96.5 / 100 |
| host, fresh | 87.9 / 99.0 / 99.7 / 100 | 77.0 / 95.9 / 98.8 / 100 | 60.4 / 87.6 / 95.4 / 100 | 69.1 / 91.6 / 97.0 / 100 |
| small, main | 87.7 / 99.0 / 99.7 / 100 | 77.5 / 95.7 / 98.7 / 100 | 60.0 / 86.5 / 94.4 / 100 | 69.0 / 90.9 / 96.4 / 100 |
| small, fresh | 87.8 / 99.0 / 99.8 / 100 | 77.0 / 96.0 / 98.8 / 100 | 60.1 / 87.4 / 95.3 / 100 | 68.9 / 91.5 / 96.9 / 100 |

Counts: host main 9 672 / 12 746 / 13 508 / 14 000; host fresh 9 669 /
12 826 / 13 575 / 14 000; small main 9 656 / 12 726 / 13 491 / 14 000;
small fresh 9 648 / 12 813 / 13 570 / 14 000. Every message decodes
alone: on this set the ceiling is 100% by construction, since every
crowded signal (−16…+6 dB) is at least 3 dB above the 50% point, where
the sweep decodes 998–1 000 of 1 000 alone. No false decode and no
duplicate in any single-pass, alone or oracle decode.

**Single-pass misses by cause, and how many one oracle pass recovers**
(no candidate on it / not converged; CRC failures 0–2 per run, all-zero
none; every miss decodes alone):

| preset, block | 10 messages | 20 messages | 40 messages | all |
|---|---|---|---|---|
| host, main | 19 → 16 / 226 → 210 | 146 → 100 / 752 → 626 | 675 → 283 / 2 508 → 1 837 | 840 → 399 / 3 486 → 2 673 |
| host, fresh | 15 → 12 / 227 → 209 | 113 → 74 / 805 → 681 | 725 → 339 / 2 446 → 1 842 | 853 → 425 / 3 478 → 2 732 |
| small, main | 20 → 16 / 227 → 211 | 155 → 106 / 746 → 621 | 763 → 321 / 2 431 → 1 793 | 938 → 443 / 3 404 → 2 625 |
| small, fresh | 16 → 13 / 228 → 212 | 125 → 82 / 795 → 676 | 806 → 380 / 2 382 → 1 802 | 947 → 475 / 3 405 → 2 690 |

One oracle pass recovers 77–79% of the not-converged misses and 47–50%
of those with no candidate; on 40-message slots the no-candidate misses
are the slowest to recover (42–47% after one pass, 78–81% after two).
The CRC-failure misses (two each in the host and small main runs) are
all recovered by the first oracle pass.

**By SNR relative to the 50% point** (host, main; all sizes; sent /
single pass / oracle +1, and the single-pass misses as no candidate /
not converged):

| relative SNR | sent | single | oracle +1 | misses |
|---|---|---|---|---|
| +3 to +6 dB | 1 742 | 750 (43%) | 1 375 (79%) | 318 / 673 |
| +6 to +9 dB | 1 941 | 966 (50%) | 1 606 (83%) | 251 / 723 |
| +9 to +12 dB | 1 934 | 1 147 (59%) | 1 718 (89%) | 155 / 632 |
| +12 to +15 dB | 1 904 | 1 253 (66%) | 1 739 (91%) | 67 / 584 |
| +15 to +18 dB | 1 908 | 1 404 (74%) | 1 810 (95%) | 36 / 468 |
| +18 to +21 dB | 1 934 | 1 641 (85%) | 1 894 (98%) | 10 / 283 |
| +21 to +24 dB | 1 886 | 1 771 (94%) | 1 859 (99%) | 3 / 112 |
| +24 to +27 dB | 751 | 740 (99%) | 745 (99%) | 0 / 11 |

(The other three runs differ by a point or two per bin.) Crowding costs
decodes at every level, most within 9 dB of the 50% point, where more
than half are missed in one pass; the no-candidate misses sit mostly
there too.

**Reading.** On this set subtraction alone is not limited by the
ceiling (100%); it is limited by how well the waveform is estimated
and by the number of passes. With perfect waveforms one further pass
reaches 91.0% (main) and 91.6% (fresh) on the host, and 90.9% and 91.5%
on the small preset; two reach about 96.5–97%. These oracle figures are
a same-configuration reference, not a bound and not a target: a real
estimate leaves residue, the oracle has advantages a real decoder does
not (constant amplitude, exact frequency, starts on a sample), and a
real pass decodes a different input. The two-pass oracle shows the
margin a second further pass could add.
Cost: each oracle pass is another full single-pass decode (the bench
does not time it); `ceiling` took about 58 s per block for the host on
16 threads, mostly the 14 000 alone decodes.

## Bench split, and no-candidate misses told apart (2026-10-27)

Results identical: `paired_bench` is split into
`examples/paired_bench/{main,sets,record,compare,cost,ceiling}.rs`
(the per-point verdict, the sweep bootstrap and the per-set comparison
are functions now). `run` against `e3-final` with `compare` (both
presets, every main set and the fresh sweep and crowded): b = c = 0
everywhere, every 50% point and interval unchanged, PASS; the release
`cat-ft8-decode` is unchanged (sha256 b3ead07b…dea93). Outputs in
`~/.cache/cat-ft8-paired/s0b/`.

Three report changes, each deliberate:

- **No candidate, split.** A miss with no candidate on it is now one of
  two causes: **no sync peak** on the message at all, or a **peak
  dropped by the candidate limit** (the sync search is repeated without
  the limit, outside the timed calls, and its peaks beyond the kept list
  are checked with the same 4 Hz / 0.12 s tolerance). A
  belief-propagation failure of a kind the bench does not know (the
  failure type is non-exhaustive) is counted as "other failure" instead
  of "not converged"; there were none.
- **Not recorded.** A run saved before outcomes were recorded now says
  "not recorded" for the `empty` set too (it printed zero misses, since
  that set has no messages).
- **Ceiling column.** `ceiling`'s "ceiling" column is now decoded by the
  single pass, alone or an oracle pass (it was single pass or alone).
  Since every crowded message decodes alone, its numbers do not change.

`ceiling host main` again: every count equals the S0 run except the
split rows. Host, main, the no-candidate misses split as (no sync peak
/ dropped by the limit, with one and two oracle passes recovering):

| size | no sync peak | dropped by the limit |
|---|---|---|
| 10 | 12 → 12 / 0 | 7 → 4 / 2 |
| 20 | 86 → 67 / 15 | 60 → 33 / 24 |
| 40 | 256 → 134 / 76 | 419 → 149 / 168 |
| all | 354 → 213 / 91 | 486 → 186 / 194 |

The same split over the `compare` runs (host): crowded main 354 / 486,
fresh 351 / 502; sweep main 1 / 68, fresh 0 / 61 (the sweep's
no-candidate misses are at its lowest points, where noise peaks outrank
the signal's). By SNR (host main), the dropped peaks sit near the 50%
point (205 of 318 no-candidate misses at +3…+6 dB, none above +18 dB);
the misses with no peak spread further up (10 at +18…+21 dB, 3 at
+21…+24 dB), presumably where a neighbouring signal masks the peak
(not measured).

## Wider crowded set, measured (2026-10-27)

Measurement only; the decoder and every existing set are unchanged
(`crowded` main re-run with this bench: b = c = 0 against `e3-final`).
`wide` is a new bench set with its own seeds: 10, 20, 30 and 40
distinct messages per slot, SNRs uniform in −22…+5 dB, the same offsets
as `crowded` (200–2 800 Hz, −0.3…+0.7 s, overlaps allowed), 200 slots
per size (20 000 messages per block). Its lower end is below the
single-signal 50% point (about −19.1 dB), so not every message decodes
even alone and the ceiling is informative. It is not in `run`'s default
sets. `paired_bench ceiling host|small main|fresh wide` at the E3
defaults; outputs in `~/.cache/cat-ft8-paired/wide/`.

**Decoded, % of transmitted** (single pass / oracle +1 / oracle +2 /
alone / ceiling — decoded by the single pass, alone or an oracle pass):

| preset, block | 10 | 20 | 30 | 40 | all |
|---|---|---|---|---|---|
| host, main | 78.9 / 89.0 / 89.3 / 90.2 / 90.2 | 69.8 / 85.1 / 87.6 / 89.5 / 89.5 | 62.0 / 82.0 / 86.3 / 88.7 / 88.8 | 55.2 / 78.2 / 84.7 / 89.3 / 89.3 | 62.5 / 81.8 / 86.2 / 89.3 / 89.3 |
| host, fresh | 78.8 / 88.7 / 89.3 / 89.7 / 89.7 | 68.7 / 85.0 / 87.4 / 89.2 / 89.2 | 62.3 / 82.8 / 86.9 / 89.8 / 89.8 | 55.0 / 78.3 / 84.9 / 90.0 / 90.0 | 62.3 / 82.0 / 86.4 / 89.7 / 89.8 |
| small, main | 78.7 / 88.8 / 89.2 / 90.0 / 90.0 | 69.5 / 84.7 / 87.2 / 89.2 / 89.2 | 61.7 / 81.7 / 86.0 / 88.5 / 88.6 | 55.0 / 77.9 / 84.3 / 89.1 / 89.1 | 62.3 / 81.5 / 85.9 / 89.1 / 89.1 |
| small, fresh | 78.8 / 88.7 / 89.4 / 89.7 / 89.7 | 68.6 / 84.8 / 87.2 / 89.1 / 89.1 | 62.1 / 82.6 / 86.7 / 89.6 / 89.6 | 55.0 / 78.2 / 84.8 / 89.9 / 89.9 | 62.2 / 81.9 / 86.3 / 89.6 / 89.7 |

Counts, all sizes (single / +1 / +2 / alone / ceiling): host main
12 503 / 16 366 / 17 248 / 17 851 / 17 856; host fresh 12 464 / 16 409
/ 17 287 / 17 946 / 17 951; small main 12 457 / 16 299 / 17 174 / 17 811
/ 17 813; small fresh 12 444 / 16 379 / 17 259 / 17 929 / 17 930. No
false decode and no oracle duplicate in any single-pass, alone or oracle
decode.

**Single-pass misses by cause** (all sizes; missed → decoded alone →
recovered by oracle +1 / +2):

| preset, block | no sync peak | dropped by the limit | not converged | CRC |
|---|---|---|---|---|
| host, main | 729 → 464 → 290 / 114 | 1 467 → 719 → 379 / 238 | 5 299 → 4 167 → 3 193 / 529 | 2 → 2 → 1 / 1 |
| host, fresh | 717 → 480 → 282 / 136 | 1 419 → 692 → 345 / 215 | 5 399 → 4 311 → 3 318 / 526 | 0 |
| small, main | 729 → 462 → 286 / 112 | 1 750 → 863 → 458 / 272 | 5 064 → 4 031 → 3 098 / 491 | 0 |
| small, fresh | 717 → 478 → 280 / 137 | 1 664 → 824 → 414 / 253 | 5 174 → 4 182 → 3 241 / 489 | 0 |

Missed even alone (host main): 2 130 not converged and 18 with their
peak dropped by the candidate limit (a single weak signal whose sync
peak is outranked by 100 noise peaks); the other runs are alike.

**By SNR** (host, main, relative to the −19.12 dB 50% point; sent /
single / oracle +1 / alone): −3…0 dB 2 147 / 69 / 148 / 204; 0…+3 dB
2 208 / 829 / 1 510 / 2 002; +3…+6 dB 2 089 / 1 005 / 1 748 / 2 089;
above +6 dB every message decodes alone and one oracle pass reaches
90–99% per 3 dB bin.

**Reading.** On this set the ceiling binds: 10–11% of the messages do
not decode even alone, all of them below +3 dB relative to the 50%
point (host main: 1 943 at −3…0 dB, 206 at 0…+3 dB). Of the gap between the single pass and the ceiling
(host main, 5 353 messages), one oracle pass covers 3 863 (72%) and two
4 745 (89%). The oracle duplicates and false decodes are zero by
construction, so they say nothing about a real subtraction.

## Subtracting one signal (S2, 2026-10-28)

`decode::subtract(audio, &decode, skip, &SubtractConfig)` is in the
core (`src/decode/subtract.rs`); nothing calls it in pass 1. Bench:
`paired_bench suppression|passes|subcost` and the `fading` set; outputs
in `~/.cache/cat-ft8-paired/s2/`. Host preset unless stated.

**Method** (module docs): the unit reference from `Synth::fill_phase` at
a whole-sample start; per-symbol correlations over each symbol's middle
half; adjacent symbols' products summed by tone step in six 2 s groups
and aligned over ±½ symbol (the phase of a start error grows with the
tone step: 1/1920 turn per sample per step), groups added without their
phases; one more timing pass only if the start moved by more than ⅛
symbol; frequency offset and linear drift from the groups' aligned
phases (weighted line); per segment a two-parameter least-squares
amplitude on the corrected reference's cosine and sine; amplitude
interpolated linearly between segment centres; subtraction in place.
`f32` and integer phase only (the drift correction is a quadratic
phase in 64-bit integers), bounded stack (`SUBTRACT_STACK_BYTES` 6 836 B
of arrays), no allocation, deterministic (three golden pins of residual
bits and estimates). It also returns `left_db`, its own estimate of the
signal left (the residual's projection on the reference plus the noise's
expected share in the amplitude fit).

**A second generator.** Starts between samples, fades and drift need a
waveform `Synth` does not make, so the bench has its own `f64` GFSK
generator (`channel.rs`, same definition, phase accumulated at the
continuous waveform time). At whole-sample starts it matches `Synth` to
6.6 × 10⁻⁴ of full scale.

**Suppression of single signals** (`suppression host "" 100`: 100 seeded
slots per condition, each decoded by the single pass first and that
decode subtracted; signal left = residual minus the noise actually
added, over the signal's energy; median [10%, 90%]; conditions other
than SNR at +10 dB; segment 1 920, drift on):

| condition | decoded | left, dB | true start imposed | noiseless | sub-sample share (median / largest) |
|---|---|---|---|---|---|
| −20 dB | 12 | −7.7 [−8.2, −7.1] | −7.7 | −68.8 | −68.8 / −59.6 |
| −16 dB | 100 | −11.8 [−12.4, −11.1] | −11.8 | −62.1 | −62.1 / −58.0 |
| −12 dB | 100 | −15.7 [−16.3, −15.1] | −15.7 | −63.0 | −63.0 / −57.4 |
| −8 dB | 100 | −19.8 [−20.5, −19.2] | −19.8 | −65.1 | −65.1 / −56.6 |
| −4 dB | 100 | −23.8 [−24.3, −23.2] | −23.8 | −63.3 | −63.3 / −56.9 |
| 0 dB | 100 | −27.7 [−28.4, −27.1] | −27.7 | −62.7 | −62.7 / −57.2 |
| +5 dB | 100 | −32.6 [−33.3, −32.1] | −32.7 | −63.6 | −63.6 / −57.4 |
| +10 dB | 100 | −37.8 [−38.5, −37.1] | −37.8 | −63.8 | −63.8 / −57.0 |
| +20 dB | 100 | −47.5 [−48.2, −46.7] | −47.5 | −63.6 | −63.6 / −57.5 |
| dt −2.0 / −1.0 / 0 / +1.0 / +2.0 / +2.4 / +2.5 s | 100 each | −37.6 … −37.8 | −37.7 … −37.8 | −63.4 … −65.8 | ≤ −63.4 / ≤ −56.5 |
| start +0 / +0.25 / +0.5 sample | 100 each | −37.7 / −37.7 / −37.7 | −37.7 | −104.2 / −63.8 / −57.7 | none / −63.8 / −57.7 (largest −56.4) |
| tone 0 at 100, 101.56, 200, 1 500, 1 500.78, 1 501.56, 2 850, 3 050 Hz | 100 each | −37.7 … −37.8 | −37.7 … −37.8 | −63.1 … −64.8 | ≤ −63.1 / ≤ −57.1 |
| fade 0.05 / 0.1 / 0.2 / 0.5 / 1 / 2 Hz (depth 0.9) | 100 each | −34.0 / −33.9 / −33.7 / −31.3 / −23.4 / −13.3 | the same | −60.7 / −55.1 / −48.3 / −35.0 / −23.9 / −13.3 | ≤ −63.2 / largest −53.3 (2 Hz) |
| drift 0.02 / 0.05 / 0.1 / 0.2 Hz/s | 100 each | −37.6 / −37.6 / −37.7 / −37.5 | −37.6 … −37.8 | −64.0 / −62.5 / −59.2 / −52.2 | ≤ −63.5 / ≤ −57.0 |
| drift 0.5 Hz/s | 100 | −36.4 [−37.7, −9.8] | −37.6 | −42.1 | −62.8 / −57.8 |

- In noise the residue is the amplitude estimate's noise: it falls 1 dB
  per dB of SNR (−27.7 dB at 0 dB, −37.8 at +10), the same at every start
  offset (including the slot's edges, where part of the waveform is
  outside the slot), every frequency (band edges and fractions of a bin)
  and drifts up to 0.2 Hz/s. With the true start imposed nothing changes
  (≤ 0.1 dB): the time estimate costs nothing measurable.
- Noiseless, everything estimated, the floor is −62…−69 dB; with a start
  on a sample it is −104 dB.
- Fades: a one-symbol segment follows fades up to 0.5 Hz (−31 dB at
  +10 dB SNR); at 1 Hz it leaves −23 dB and at 2 Hz −13 dB.
- Drift 0.5 Hz/s (6.3 Hz over the transmission, beyond the fading set's
  ±0.2 Hz/s) mostly works (median −36.4 dB) but fails in about one trial
  in ten (90% quantile −9.8 dB: the frequency estimate misses, |error| 90%
  0.84 Hz).

**Fine estimates against the truth** (|error| median [90%]; the
decoder's own in brackets): start 0.05 [0.13] samples at +20 dB, 0.21
[0.51] at +10, 0.69 [1.40] at 0 dB, 4.0 [9.4] at −16 dB (decoder 45–75
[90–137] samples); the whole-sample start is exactly right in 90% of
trials at +20 dB, 81% at +10 dB, 51% at a half-sample start. Frequency
below 0.001 Hz at +5 dB and up, 0.013 [0.026] Hz at −16 dB, 0.022
[0.064] at −20 (the decoder's medians up to 0.51 Hz, 90% up to 0.73). Drift error 0.004
[0.008] Hz/s at −16 dB, below 0.001 above 0 dB.

**`left_db` against the measurement:** +10 dB −35.6 vs −37.8; −16 dB −9.9
vs −11.8; fade 1 Hz −24.0 vs −23.4; fade 2 Hz −15.7 vs −13.3. It is
within about 2.5 dB: high where noise dominates, low for fast fades.

**Segment length** (signal left, median dB): at +10 dB 480 → −31.8, 960
→ −34.8, 1 920 → −37.8, 3 840 → −40.6, 7 680 → −43.3, the whole waveform
→ −42.1 (3 dB per doubling, the noise averaged). Fades reverse it: at
1 Hz 480 → −27.7, 960 → −29.4, 1 920 → −23.4, 3 840 → −13.3; at 0.5 Hz
3 840 leaves −23.7 against −31.3. Drift needs the drift estimate once
segments are long: at 0.2 Hz/s without it, 1 920 leaves −20.6 dB and
3 840 −10.7 (−37.5 and −39.6 with it).

**S1 (a sub-sample start in `Synth`): not needed.** The residue a start
between samples leaves — the noiseless subtraction with the true start
rounded to a sample and the true frequency and drift imposed, less the
same for the same signal started on a sample — is −57.7 dB at a
half-sample start (the worst case), −63.8 dB at a quarter sample, and
at most −53.3 dB in any trial of any condition (a 2 Hz fade). Every
condition is more than 23 dB below the −30 dB threshold, and 15 dB or
more below what noise leaves even at +20 dB SNR. This agrees with the
plan's estimate (about −39 dB as an upper bound from the phase error
between tones; the amplitude fit absorbs most of it).

**Further passes with the bench's own loop** (`passes`; each pass's new
decodes subtracted in list order, then the residual decoded again with
the same configuration; beside the same-configuration oracle passes run
in the same command; decoded, % of transmitted):

| set, block | single | +1 real / oracle | +2 real / oracle | share of the oracle's gain +1 / +2 | duplicates p2 / p3 (of decodes subtracted) | false p2 / p3 |
|---|---|---|---|---|---|---|
| crowded, main | 69.1 | 90.3 / 91.0 | 95.1 / 96.5 | 96.6% / 94.8% | 1 of 9 672 / 11 of 2 969 | 0 / 0 |
| crowded, fresh | 69.1 | 91.0 / 91.6 | 95.7 / 97.0 | 97.3% / 95.5% | 0 of 9 669 / 10 of 3 071 | 0 / 0 |
| wide, main | 62.5 | 80.5 / 81.8 | 83.8 / 86.2 | 93.0% / 89.5% | 1 of 12 503 / 6 of 3 593 | 0 / 0 |
| wide, fresh | 62.3 | 80.5 / 82.0 | 83.9 / 86.4 | 92.4% / 89.5% | 0 of 12 464 / 4 of 3 644 | 0 / 0 |
| fading, main | 63.3 | 81.5 / 82.5 | 84.8 / 86.8 | 94.9% / 91.6% | 0 of 8 858 / 0 of 2 551 | 0 / 0 |
| fading, fresh | 62.7 | 81.2 / 82.3 | 84.6 / 86.7 | 94.1% / 91.0% | 0 of 8 782 / 3 of 2 583 | 0 / 0 |
| crowded, main, small preset | 69.0 | 90.2 / 90.9 | 95.0 / 96.4 | 96.7% / 94.8% | 0 of 9 656 / 9 of 2 970 | 0 / 0 |

Per size (crowded, main): 10 messages 87.8 → 99.0 / 99.7 (oracle 99.0 /
99.7), 20 messages 77.5 → 95.3 / 98.0 (95.7 / 98.8), 40 messages 60.2 →
85.6 / 92.4 (86.7 / 94.5). Counts (crowded, main): single 9 672, real +1
12 641, +2 13 309; oracle 12 746 and 13 508 (as S0 measured). A real
pass decoded 3 messages the oracle pass did not (fading main: 18 and
16; wide main: 9 and 6).

- **Duplicates** — a payload of an earlier pass decoded again, so a
  subtraction that left a decodable signal — are 1 in about 10 000
  subtractions after the first further pass and 0.1–0.4% after the
  second; on crowded main the duplicated messages were at 0–10 dB SNR
  (not weak signals: overlapping strong neighbours bias each other's
  estimates). The list's duplicate rule keeps them out of the output.
- **False decodes:** none in any further pass on any set (the expected
  rate from the CRC was not computed per pass here; S3's bar).
- **The fading set** decodes fewer in the single pass (63.3% against
  crowded's 69.1%) and keeps nearly the same share of the oracle's gain
  (94.9% against 96.6%); drift estimation on or off makes no
  difference to the pass gain (94.9% / 91.6% on, 94.9% / 91.4% off).
  One-symbol segments do not fully follow drift on their own (single
  signals at +10 dB without the drift estimate: −30.0 dB at 0.1 Hz/s and
  −20.6 dB at 0.2 Hz/s, against −37.7 and −37.5 with it). A likely cause,
  not measured separately: the set's drifts are uniform in ±0.2 Hz/s and
  its signals at −16…+6 dB, where the amplitude estimate's noise alone
  leaves −12…−34 dB, so the extra residue matters only for the few
  strong signals that drift fast.

**Segment length on the passes** (host, main; share +1 / +2, duplicates
p2 / p3):

| segment | crowded | wide | fading |
|---|---|---|---|
| 960 | 91.8% / 86.3%, 0 / 0 | 83.2% / 77.6%, 0 / 0 | 85.4% / 79.4%, 0 / 0 |
| 1 920 (default) | 96.6% / 94.8%, 1 / 11 | 93.0% / 89.5%, 1 / 6 | 94.9% / 91.6%, 0 / 0 |
| 2 880 | 98.2% / 97.4%, 8 / 42 | 96.1% / 93.4%, 6 / 36 | 97.2% / 94.8%, 0 / 10 |
| 3 840 | 99.0% / 98.2%, 4 / 36 | 96.9% / 94.8%, 9 / 37 | 97.4% / 95.1%, 27 / 52 |
| 1 920, no drift | 96.6% / 94.8%, 1 / 12 | 93.0% / 89.5%, 1 / 6 | 94.9% / 91.4%, 0 / 0 |

Fresh seeds, 3 840: crowded 99.0% / 98.6% (4 / 41), wide 96.7% / 94.8%
(2 / 24), fading 96.9% / 94.9% (21 / 49). Longer segments recover more
(less noise in the amplitude) and leave more duplicates (fades and
neighbours followed less well); the default stays at one symbol, the
segment with almost no duplicates on every set. Since duplicates are
dropped by the list, S3 should sweep 1 920–3 840 together with the
later-pass candidate count, on the paired gain.

**Cost** (`subcost host "" 20 5`: one thread, the single-pass decodes of
20 crowded 40-message slots, 485 calls, median of five runs): 3.3 ms per
call (3.28–3.32 in two runs; 3.23–3.35 per slot), about 80 ms for a
40-message slot's ~24 pass-1 decodes. A first version took 6.4 ms;
skipping the confirming timing pass when the start moved by ⅛ symbol or
less, an integer quadratic phase for the drift and whole-block loops
halved it without changing the suppression (≤ 0.1 dB). Each call runs
the reference generator three to five times (one to three timing steps,
each over the symbols' middle halves only, then amplitude and
subtraction over the whole waveform; one more timing run in the rare
case that a repeated step finds nothing), about 0.7 ms per full run. In `passes` (all threads
busy) a further pass's decode took 51 ms median. Against the multi-pass
budget (worst 40-signal slot ≤ 250 ms in total) this is tight; S3
measures it, and the subtraction cost is the first candidate for
optimisation (one generator run for amplitude and subtraction would
need a buffer of about 1.5 segments).

**Memory.** `subtract`'s arrays: 6 836 B (`SUBTRACT_STACK_BYTES`); its
frame on thumbv8m (`check.sh --stack`, host image, residual in a
static): 7 432 B. Flash: the host image grows from 83 504 B to 90 296 B
(+6 792 B); the small image (which does not call it) and the encoder
image are unchanged (83 504 B, 35 848 B); no `f64` helper linked. The
host image's static RAM grows only by the check program's 8 192 B
residual buffer.

**Pass 1 unchanged.** Every golden pin passes; `paired_bench run` of every
default set (both presets, the main block and the fresh sweep and
crowded sets) is record-for-record identical to `e3-final` apart from
timings (`compare`: b = c = 0, PASS). The release `cat-ft8-decode` does
not link `subtract` (no symbol of it or of the new arctangent), but its
bytes changed (sha256 ce6a2549…fe8f at this commit, was
b3ead07b…dea93; an intermediate state gave a02cff01…9eac): adding the
module moved code between codegen units (`soft_bits_at` is 4 bytes
longer; two error helpers changed size). Its output is identical: 800
seeded WAV slots (0, 1, 5, 10, 20 and 40 signals; some cut short),
both configurations, `--raw` and `--profile` (timings removed): 35 840
output lines, 31 040 of them decodes, byte-identical between the two
binaries. With only the new arithmetic helpers (no new module) the
binary is byte-identical to b3ead07b.

**Not measured:** subtraction of signals from another FT8 generator
(`subtract_wav` exists for that); real off-air audio; the cost on any
other host; fades deeper than 0.9 or not sinusoidal (Rayleigh); drift
that is not linear; the expected false decodes per further pass from
the CRC.

## S2 fix round (2026-10-28)

Outputs in `~/.cache/cat-ft8-paired/s2/fix/` (this round) and
`~/.cache/cat-ft8-paired/s2/logs/` (the S2 logs: stack and flash report,
WAV-identity outputs, `subcost`).

- **Shortest segment 960 samples** (was 480, which measured worse in
  every condition). The per-segment `valid` flags are gone: a segment
  without an estimate takes the previous one's as the amplitude pass
  goes, those before the first estimate are filled afterwards. The time
  search keeps one set of step sums (a repeated step that finds nothing
  takes the sums again where the estimate came from; rare). Results
  identical: with the old drift weight the default and whole-waveform
  pins were unchanged.
- **Stack.** `SUBTRACT_STACK_BYTES` 4 536 B (was 6 836). On thumbv8m
  (`check.sh --stack`, host image): `subtract`'s frame 4 504 B (was
  7 432 B, everything inlined); the time search's sums are now an
  out-of-line closure of 2 800 B and the amplitude/subtraction block loop
  one of 2 232 B, so the deepest chain is about 7 300 B. A version with
  one call site for the sums kept the chain at about 6 900 B but was 10%
  slower (interleaved `subcost`, 3.7 against 3.4 ms), so it was not kept.
  Flash: host image 90 820 B (S2 90 296 B); small and encoder images
  unchanged (83 504 B, 35 848 B).
- **Input as the front end takes it.** `subtract` now reads a non-finite
  sample as 0.0 and clamps a finite one to ±2¹⁶, and writes a changed
  sample as that value less the estimate. Identical for ordinary input
  (pins unchanged by it); costs about 4% (`subcost`, interleaved with
  d4fd2d3: 3.29–3.41 against 3.45–3.49 ms per call).
- **Drift weight** |z|² instead of |re| + |im| (the latter depends on the
  groups' phase). Suppression (`fix/suppression-host-100.txt`, 100
  trials per condition) is unchanged to 0.1 dB in every condition except
  drift 0.5 Hz/s's 90% quantile (−9.1 dB, was −9.8). Passes (host main,
  share of the oracle's gain +1 / +2 and duplicates p2 / p3): crowded
  1 920 96.6% / 94.8%, 1 / 11 (unchanged), 3 840 98.9% / 98.2%, 5 / 36
  (was 99.0% / 98.2%, 4 / 36); wide 1 920 93.0% / 89.6% (+2 was
  89.5%); fading 3 840 97.5% / 95.2%, 27 / 51 (was 97.4% / 95.1%,
  27 / 52); every other entry of the S2 table unchanged; fresh seeds:
  crowded 1 920 97.3% / 95.5%, 3 840 99.1% / 98.6%; wide 92.3% / 89.5%
  and 96.7% / 94.8%; fading 94.1% / 91.0% and 96.9% / 94.9%. No false
  decodes.
- **Pins** (`subtract::tests::deterministic_and_pinned`): default
  0x0c03f1faaa51307a → 0xf67781b25882c626 (drift weight); 960 samples,
  drift off 0xfc2a9cb5fdf776ce (new setting, replaces 480 samples, drift
  off, 0x2e0bf0eb311b0c28); whole waveform, drift on 0xfa0729dc78e8af7d
  → 0xd6d805c967815885 (drift weight).
- **Tests added:** a skipped region holding NaN, ∞, 1e30 or −3e4 gives a
  result bit-identical to the zero-filled run and is left as it was; tone
  0 at 25 and 60 Hz (noiseless, below −40 dB); NaN and −∞ samples (in a
  symbol's middle half and at a boundary) give the same result as zeros
  there, 1e30 the same as 2¹⁶; a strong signal clipped at 60% of its peak
  in noise loses 7.3 dB of the slot's energy.
- **`suppression`** gained input conditions: cut at 13.5 s and starting
  1 s late, at +10 and 0 dB: −37.7 / −37.8 dB and −27.8 / −27.6 dB, the
  same as whole slots. It now fails if the bench generator differs from
  `Synth` by more than 10⁻³ (measured 6.58 × 10⁻⁴); `drift=` takes only 0
  or 1; the segment sweeps of `suppression` and `passes` are one GRID
  argument (`seg=1920|3840,drift=1|0`; grids joined with `;`).
- **Another generator.** An independent check with another FT8
  generator's waveforms found subtraction as good as on this crate's:
  identical residue from −16 to +10 dB SNR, and a −68 dB noiseless floor
  from a 1–2-sample offset of the ramps. `left_db` cannot see that kind
  of residue (it measures only along the reference waveform); its docs and
  `subtract_wav`'s now say so.
- **`atan2_turns`**: the coefficients are the six-term odd polynomial
  F. Mazzoli gives (2021), attributed by him to C. Hastings Jr.,
  *Approximations for Digital Computers* (1955); checked against
  Abramowitz and Stegun (page 81): not their 4.4.47 (Hastings' five
  terms, 1e-5) nor 4.4.49 (degree 17, Carlson and Goldstein). Measured
  error 1.7 × 10⁻⁶ rad; the test bound is now 5 × 10⁻⁷ turn (was 2 × 10⁻⁶).
- **Build paths.** Binary hashes depend on the build path. Built here
  from this repository's path with the same target directory, 23d8336
  and d4fd2d3 give different bytes (b3ead07b… and ce6a2549…); built from
  a second path, they differ too (7a722d9c… and 42db0537…). So the S2
  byte change is the source change (the new module moving code between
  codegen units), not the path; the two binaries' decode output is
  identical (`s2/logs/wav-identity-*.txt`, byte-identical). Compare
  binaries only when both are built from the same path.
- **Pass 1 unchanged:** release `cat-ft8-decode` ce6a2549… as at
  d4fd2d3; `paired_bench run` of every default set, both presets,
  record-for-record identical to `e3-final` (and to `s2/identity`, with
  outcomes); golden pins pass.

## Further passes (S3, 2026-10-28)

`DecodeConfig::{passes, later_candidates, subtract}`,
`Decoder::{decode_passes, decode_passes_staged, find_later_candidates}`,
`DecodeList::next_pass` (and `len`), `PassReport`/`PassEvent`/`PassStep`;
`cat-ft8-decode --config host|small[,passes=N]` (the user asked for a
flag; profile keys added only). Outputs in `~/.cache/cat-ft8-paired/s3/`
(`sweep/`, `accept*`, `ceiling/`, `cost/`, `identity/`, `wav/`, `ci/`).

**Sweep** (`paired_bench passes`, `decode_passes` with three passes, main
blocks; share of the same-configuration oracle's +1 / +2 gain, the oracle
re-run at each later count; duplicates p2 / p3; no false decode and no
list truncation anywhere):

| segment, later | crowded | wide | fading |
|---|---|---|---|
| 1 920, 100 / 150 / 200 | 96.6% / 94.8%, 1 / 11 (all three) | 93.0% / 89.6%, 1 / 6 | 94.9–94.8% / 91.6%, 0 / 0 |
| 2 880, 50 / 75 / 100 / 150 / 200 | 98.2% / 97.4%, 8 / 42 | 96.1% / 93.4%, 6 / 36 | 97.1–97.2% / 94.8–94.9%, 0 / 10 |
| 3 840, 100 / 150 / 200 | 98.9–99.0% / 98.2%, 5 / 36 | 96.9% / 94.8%, 9 / 37 | 97.4–97.5% / 95.2%, 27 / 49 |
| 3 840, 50 / 75 | 98.9% / 98.2%, 4 / 36 | 96.9% / 94.8%, 9 / 36 | 97.4% / 95.2–95.3%, 26–27 / 49 |
| 3 840, 20 / 30 / 40 | 98.8–98.9% / 98.2%, 4 / 34–36 | 96.8–96.9% / 94.8%, 8–9 / 36 | 97.4% / 95.3%, 26 / 46–49 |

Decoded counts at 3 840 (crowded main, +1 / +2): later 100–200 12 713–
12 714 / 13 440, 50 and 75 12 713 / 13 439, 30–40 12 712 / 13 439, 20
12 710 / 13 439; wide main 16 246 / 16 999 from 40 up (20: 16 240);
fading main 11 476–11 480 / 11 995–11 998 from 40 up (20: 11 464 /
11 992). The later passes' candidate count hardly matters from 40 up:
once pass 1's signals are gone, the decodable rest ranks near the top.
It costs belief propagation (pass 2 decode, all threads busy: 26 ms at
20, 38 at 50, 56 at 100, 90 at 200). Fresh seeds (`sweep/passes-host-
fresh-*.txt`, segments 1 920 / 2 880 / 3 840 × later 50 / 100) rank the
same: 3 840 best (crowded 99.1% / 98.6%, wide 96.7% / 94.8%, fading
97.0% / 95.0% at later 50; 50 = 100 within 0.1 point).

**Chosen and adopted in `HOST`:** three passes, 50 later candidates,
3 840-sample (two-symbol) segments, drift on. `SMALL` stays at one pass.
Three passes over two: +5.7 points on crowded for about 60 ms more per
40-signal slot (two passes, 40 slots: worst 154 ms, mean 138 ms).

**Paired acceptance** (`accept.compare.txt`: `run host` against the
single pass — `e3-final` plus `base/` for edges fresh, wide and fading,
recorded with `host,passes=1`, record-identical to the single pass):

| set, block | single | three passes | b / c | target c − b ≥ 2√(b + c) | false per pass | expected false (CRC), per slot: three passes / single |
|---|---|---|---|---|---|---|
| crowded main | 9 672 | 13 439 | 0 / 3 767 | met | 0 / 0 / 0 | 2.03e-6 / 1.22e-6 |
| crowded fresh | 9 669 | 13 522 | 0 / 3 853 | met | 0 / 0 / 0 | 2.75e-6 / 1.42e-6 |
| wide main | 12 503 | 16 999 | 0 / 4 496 | met | 0 / 0 / 0 | 2.14e-6 / 9.92e-7 |
| wide fresh | 12 464 | 17 036 | 0 / 4 572 | met | 0 / 0 / 0 | 1.98e-6 / 8.39e-7 |
| fading main | 8 858 | 11 995 | 0 / 3 137 | met | 0 / 0 / 0 | 2.24e-6 / 9.16e-7 |
| fading fresh | 8 782 | 11 971 | 0 / 3 189 | met | 0 / 0 / 0 | 1.63e-6 / 1.12e-6 |
| sweep main / fresh | 8 764 / 8 772 | the same | 0 / 0 | — | 0 / 0 | 1.20e-6 / 9.24e-7; 1.17e-6 / 8.99e-7 |
| edges main / fresh | 1 228 / 1 220 | the same | 0 / 0 | — | 0 / 0 | 1.13e-6 / 7.04e-7; 1.60e-6 / 1.17e-6 |
| empty main | 0 | 0 | — | — | 0 | 9.82e-7 (one pass: nothing to subtract) |

Every gate passes: no loss (b = 0 everywhere); zero false decodes in
every pass of every set; the expected false decodes at most the passes
run × the single pass's rate (largest ratio 2.4, fading main, limit 3); no
payload listed twice; the sweep's 50% point unmoved with its whole
bootstrap interval within ±0.05 dB (−19.122 and −19.078 dB, shift
+0.000 [+0.000, +0.000]). Per pass, crowded main: new decodes 9 672 /
3 041 / 726, duplicates of earlier passes 0 / 4 / 36, CRC failures 12 / 7
/ 1, pass 3 run in 550 of 600 slots. Sweep and edges: pass 2 runs where
pass 1 decoded and adds nothing. `compare` now checks the per-pass gates
and the sweep interval (both blocks).

**Headline (share of the oracle's gain, the oracle re-run with `ceiling`
at 50 later candidates — the same counts as at 100):** crowded 98.9% /
98.2% (main), 99.1% / 98.6% (fresh) of the +1 / +2 gain, ceiling 100%;
wide 96.9% / 94.8% and 96.7% / 94.8%, decoded 85.0% and 85.2% against a
ceiling of 89.3% and 89.8%; fading 97.4% / 95.3% and 97.0% / 95.0%.

**Cost** (`paired_bench slotcost host,passes=1 host 200 5`, one thread,
every 40-message slot of crowded main, per slot the median of five
interleaved runs; `cost/slotcost-final.txt`): **worst 213.7 ms** (budget
250), **mean 197.5 ms**; single pass worst 40.9, mean 38.9 ms (budget:
mean ≤ 100). Per pass (mean of per-slot medians): pass 1 39 ms (feed
14.4, sync 2.8, belief propagation 18.7); pass 2 98 ms (subtraction of
24.1 decodes 69.7, feed 14.5, sync 2.9, belief propagation 9.7); pass 3
60 ms (subtraction of 10.4 decodes 30.1, feed 14.4, sync 2.9, belief
propagation 11.4). Deterministic bound per pass: pass 1 100 candidates
× 30 rounds, later passes 50 × 30, each one feed and one sync search,
plus the previous pass's subtractions (measured: 2 297 / 1 195 / 1 413
rounds per slot on average). Measured on a host with a busy desktop
session (about two cores of background load).

**Speedups** (results identical; `cost/ablation.txt`, interleaved, 40
slots and `subcost`): in `Synth::run` the envelope is computed only in
the first and last symbols (elsewhere it is exactly 1.0) and the phase
increment converted through `i64` (the same value: it is in [0, 2³¹)):
`subtract` 3.70 → 3.19 ms per call, the 40 slots' mean 215–219 → 200–
201 ms and worst 229–233 → 210–214 ms; waveform golden hashes and every
other pin unchanged. Two others measured no gain and were dropped:
skipping `subtract`'s residue statistics in the passes (198.5–198.9 ms
without against 200.3–201.2 with — within noise) and a closed-form
`Correction::skip` (it runs only where a waveform leaves the buffer). A
constant-increment path for the middle of each symbol was slower (more
instructions, 3.46–3.50 against 3.38–3.44 ms) and was not kept; nor was a
fused amplitude-and-subtraction pass (it needs a buffer of up to 1.5
segments of reference, unbounded for long segments).

**Determinism.** New pins (`tests/decode.rs`,
`multi_pass_golden_hashes`, over the three golden slots and a crowded
30-message slot whole, cut at 13.5 s and starting 1 s late; output,
report and residual bits): HOST settings with one-symbol segments and
100 later candidates, two passes 0x0f939b4c46530a23 (decodes 8 / 0 / 8 /
26 / 26 / 26), three passes 0x5b2023001e1b9607 (8 / 0 / 8 / 28 / 28 /
28); min-sum on the small band, two passes 0x47cb465646e18e77 (6 / 0 / 6
/ 26 / 26 / 25); the adopted `HOST` 0x23c98790b69a90b8 (8 / 0 / 8 / 28 /
28 / 28). The single-pass pins are unchanged (`Decoder::decode` runs one
pass whatever the configuration). The split API driven by hand — later
searches, candidates on 1, 3 and 8 threads in reverse order, merged in
order, subtraction in list order — equals `decode_passes` bit for bit
(`passes_by_hand_on_threads_match_decode_passes`, run by `cargo test` on
every CI target; the decode command itself is single-threaded, so no
further CI step).

**Pass 1 unchanged.** `paired_bench run host,passes=1` and `small` of
every default set and the fresh sweep and crowded sets: record-for-record
identical to `e3-final` and to `s2/identity` (b = c = 0, PASS). The
release `cat-ft8-decode` changed bytes (new code, and `Synth`); on the
800-slot WAV corpus, `--config host,passes=1` and `small`, plain,
`--raw` and `--profile` (timings removed), its output is byte-identical
to d4fd2d3's (ce6a2549…) with `--config host` and `small`
(`wav/base.txt`, `wav/s3-p1.txt`).

**Memory.** The core gains no buffer; `Decoder` grows by its
configuration to 200 B (was 176 B); the caller's residual is 720 000 B (the decode
command's profile reports it as `residual_bytes`). Flash (`check.sh`,
thumbv8m): small image 83 976 B (was 83 504), encoder-only 36 304 B (was
35 848), host image 91 264 B (was 90 820): the `Synth` change (a
per-symbol envelope test) adds about 450 B to every image that
synthesises.

**Not measured:** off-air audio; other hosts' speed; the cost of slots
beyond 40 signals; the external acceptance (another decoder's corpus)
of the binary built here.
