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
