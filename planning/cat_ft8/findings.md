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

## Decoder sizing (planning, 2026-10-26, revised; estimates until measured)

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
- D5/D6 checklist: every argmax or hard decision on 8-bit levels
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
`cat-ft8-decode --config host|small[,passes=N]` (a flag, on request;
profile keys added only). Outputs in `~/.cache/cat-ft8-paired/s3/`
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
set of 800 WAV slots, `--config host,passes=1` and `small`, plain,
`--raw` and `--profile` (timings removed), its output is byte-identical
to d4fd2d3's (ce6a2549…) with `--config host` and `small`
(`wav/base.txt`, `wav/s3-p1.txt`). The binary built from cea0bc2 in this
repository's path, sha256 b07afd894e432a81634d3b8e5d06eb59c19a90a4a9ba2a832ae437102746b989,
gives the same identity (`wav/cea0bc2-p1.txt`); with its defaults (host
three passes) the set's host decodes go from 7 767 to 10 696, the
single pass's lines first and unchanged in every slot, no message twice,
`small` unchanged (`wav/cea0bc2-default.txt`).

**Memory.** The core gains no buffer; `Decoder` grows by its
configuration to 200 B (was 176 B); the caller's residual is 720 000 B (the decode
command's profile reports it as `residual_bytes`). Flash (`check.sh`,
thumbv8m): small image 83 976 B (was 83 504), encoder-only 36 304 B (was
35 848), host image 91 264 B (was 90 820): the `Synth` change (a
per-symbol envelope test) adds about 450 B to every image that
synthesises.

**Not measured:** off-air audio; other hosts' speed; slots beyond 40
signals.

## S3 fix round (2026-10-28)

Fixes after the S3 checkpoint; decode results unchanged. Code at
00c7cbe (the `Synth` conversion) and 0d3c912 (the rest); outputs in
`~/.cache/cat-ft8-paired/s3b/`.

- **`Synth` phase increment through `i32`, not `i64`.** On thumbv8m an
  `f32 as i64` is a call to `__aeabi_f2lz`, which S3 had put on the
  per-sample path of every image that synthesises. Exact for the
  increment's range [0, 2³¹); waveforms and pins unchanged. `__aeabi_f2lz`
  call sites back to d4fd2d3's (small 1, host 5, encoder-only 1); the ones
  left are per-estimate conversions in `subtract` and `Placement::slot`,
  which need the `i64` range, so `check.sh` cannot forbid the helper.
  Flash: small 83 784 B, host 91 140 B, encoder-only 36 116 B (S3:
  83 976, 91 264, 36 304). On this host the `i64` form was the faster:
  `subcost` (interleaved, three rounds, `s3b/subcost-cast2.txt`) `i64`
  2.94–3.03 ms per call, `i32` 3.11–3.25, `u32` 3.12–3.24; per 40-signal
  slot (`s3b/cost/ablation.txt`, 40 slots, two rounds) mean 198–199 →
  205–207 ms, worst 211–213 → 217–221 ms. The API changes below cost
  nothing measurable (the same runs with them: mean 206 ms).
- **One meaning for the multi-pass report.** `decode_passes` returns a
  `PassesReport`: its `DecodeReport` is the whole slot (search, list and
  rejection counts summed over the passes run; input counters from pass
  1's feed), beside `passes_run`, subtractions made and failed, and why
  no further pass ran (`PassesEnd`: all passes, nothing new, list full,
  subtraction cap, stopped). The bench's records and the decode command's
  profile carry the totals; the bench's CRC line is now summed (crowded
  main 20, was 12 for pass 1 only), while its per-pass gate still uses
  the base's single-pass rate.
- **Further passes after a streamed pass 1:**
  `Decoder::decode_further_passes[_staged]`; `decode_passes` is pass 1
  then it. Pass 1 fed as `i16` blocks with `feed_i16`, then further
  passes on `f32 = i16 / 32 768`, is bit-identical to `decode_passes`
  (unit test, skip 0 and 12 000).
- **Budgets.** The staged callback returns `ControlFlow`; a `Break` stops
  after the current candidate pass or subtraction (`PassesEnd::Stopped`).
  `DecodeConfig::max_subtractions` (default `None`) starts a further pass
  only if its subtractions fit: the output is a prefix of the uncapped
  one (tested with caps 0, pass 1's count and the slot's total).
- **List overflow.** A pass that leaves the list full ends the passes
  (truncated decodes are never subtracted, so a further pass would decode
  them again and count them again). `DecodeConfig::max_decodes()`: 200
  for `HOST`, above `HOST_DECODES` = 128 (validated to 40 signals; the
  bench's sets never truncate).
- **Smaller fixes:** `candidate_capacity()` = max(candidates, later)
  always, the search's clamp removed; the list's counters one `Copy`
  struct with one difference method; only successful subtractions
  counted (`debug_assert`), failures in `PassReport::subtract_failed`;
  one `PassStep::Subtract` event per decode subtracted (so a stop can
  come between subtractions); `Subtraction` non-exhaustive; `Stage` and
  `PassStep` documented as a profiling surface that grows;
  `MAX_SEGMENTS` with `div_ceil` and every allowed segment checked; the
  decode command's timings in fixed arrays; `"pass"` on every decode
  line.
- **Tests added:** the threaded split with a missing start (skip 12 000)
  comparing the whole report; audio longer than a slot, and a skip at or
  past the audio's end; an earlier-pass duplicate (one message sent on
  two frequencies: pass 1 subtracts only the listed copy); the list-full
  stop (lists of 6, of exactly pass 1's count, and one more); the cap;
  stopping during pass 1 and after the first subtraction; `i16`
  streaming; a segment with too few samples taking the previous
  estimate (without the fill its last 100 samples keep −5.8 dB of the
  signal; with it, below −30 dB).
- **Pins.** Single-pass pins unchanged. The four multi-pass golden hashes
  change only through the report (0x0f93…→0xdb5e…, 0x5b20…→0x3ff6…,
  0x47cb…→0x4771…, 0x23c9…→0x7e11…): with the S3 report rebuilt from the
  pass reports (pass 1's search counts, the list's decoded and
  truncated) the test reproduces the S3 pins, so decodes, texts and
  residuals are bit-identical.
- **Paired acceptance re-recorded at commit 3ed1aa0** (clean tree; the
  code is 0d3c912's; `s3b/accept`, `s3b/accept.compare.txt`): every set,
  both blocks, PASS; against S3's `accept.compare.txt` only timings and
  the now-summed CRC failures and dropped candidates differ. Records
  equal to S3's `accept` with timings removed, each slot's report equal to
  the sums over its unchanged pass records (38 900 records,
  `s3b/accept.recdiff.txt`); the single-pass baselines equal S3's (4 100
  records); pass 1 (`host,passes=1` and `small`, every default set and the
  fresh sweep and crowded sets) record-for-record equal to `e3-final` and
  `s2/identity` (34 800 records each). (S3's stored records came from a
  tree with the S3 changes not yet committed; these replace them.)
- **Cost** (`paired_bench slotcost host,passes=1 host 200 5`,
  `s3b/cost/slotcost-final.txt`): worst 40-signal slot **224.7 ms**
  (budget 250), mean 207.8 ms (S3: 213.7 / 197.5); single pass worst
  41.3, mean 39.1 ms. Pass 2's subtraction 76.2 ms (was 69.7), pass 3's
  33.0 (30.1): the conversion above.
- **Decode command.** Release `cat-ft8-decode` built from this
  repository's path, sha256 given in the progress log. On the set of 800
  WAV slots (`host` and `small` defaults, plain, `--raw` and `--profile`,
  timings removed) its output equals cea0bc2's but for the added
  `"pass"` field (46 560 / 7 206 / 1 581 lines from passes 1 / 2 / 3, in
  pass order) and the profile's slot counts, which equal the sums of the
  unchanged per-pass keys, and the added `passes_end`; with
  `host,passes=1` and `small` it equals cea0bc2's but for `"pass": 1`.

## Decode time, optimisation round (O1, 2026-10-28)

Four changes to the multi-pass decode's cost, each bit-identical; code at
4752586, 5f53377, c554d6a and 4fe722c. Outputs kept locally (not
committed). Hosts: the x86-64 desktop of the earlier
measurements, and a Raspberry Pi 5 (Cortex-A76, 2.4 GHz; generic
`aarch64-unknown-linux-gnu` build, one benchmark at a time, its throttle
flags read before and after every run: none current in any run).

**Where the time went** (`slotcost host,passes=1 host 200 5`, 40-signal
slots, three passes, one thread; per slot, mean of the per-slot medians):
subtraction 107 / 207 ms (x86 / Pi), feed 43 / 89 ms (three passes),
belief propagation 40 / 50 ms, sync 8.5 / 18 ms. In `perf` (x86): the
amplitude-and-subtraction loops 24%, belief propagation 22%, the FFT
and spectrum levels 19%, the reference generator (`fill_phase`) 16%.

| step | x86 worst / mean, ms | Pi worst / mean, ms | the stage it targets |
|---|---|---|---|
| base (36cc440) | 219.9 / 204.4 | 396.2 / 367.7 | |
| 1. one run of the reference for fit and subtraction | 187.5 / 173.1 | 328.2 / 307.4 | subtract 107.5 → 76.8 (x86), 207.4 → 147.3 (Pi) |
| 2. FFT stages by radix, levels in chunks | 157.1 / 145.5 | 270.4 / 249.6 | feed 43.1 → 15.5 (x86), 88.7 → 31.3 (Pi) |
| 3. `Synth` increment through `i64` on 64-bit | (subcost −5%) | (no change) | subtract per call 2.21–2.25 → 2.08–2.12 ms (x86), 4.266 → 4.266 (Pi) |
| 4. sum-product round in phases | 151.2 / 141.8 → 144.4 / 136.0 (40 slots) | 263.9 / 248.5 → 256.8 / 241.3 (40 slots; inlined variant) | BP 39.5–40.0 → 32.5–32.8 (x86), 50.2 → 42.6 (Pi) |
| **final (4fe722c)** | **142.1 / 132.5** (base again 221.0 / 203.8) | **262.2 / 241.3** (base again 395.8 / 367.2) | subtract 71.5 / 147.5, feed 15.3 / 31.3, BP 32.0 / 41.4, sync 8.7 / 18.4 |

Single pass (`host,passes=1`): x86 worst 40.9 → 27.1 ms, mean 38.6 →
25.4; Pi 62.2 → 38.8, 60.6 → 37.4. Steps 1, 2 and the final rows are
200 slots × 5 runs interleaved by configuration, the binaries run one
after another; steps 3 and 4 were timed alternately in rounds (3 rounds
of `subcost host seg=3840 20 5`, or 2 rounds of 40-slot `slotcost`).

1. **Subtraction behind the fit.** The amplitude fit already computes
   each sample's corrected reference (envelope times cosine and sine of
   the phase); with segments up to two symbols it now holds those pairs
   (at most a segment and a half, 5 760 samples) and subtracts each
   sample as soon as the amplitudes of the segments on both sides of its
   interval are final. A second run of the generator, the frequency
   correction and the sine table is gone. Longer segments, or a first
   estimate that comes too late to hold everything since the first
   usable sample, run the reference a second time as before (a test
   forces holds of 0, 1 000 and 2 500 samples, overflowing part-way both
   before and after the first estimate: identical results). The
   subtraction loop runs between events (segment end, interval end, wrap
   of the held buffer) with its state in locals. `subcost host seg=3840`:
   3.09–3.18 → 2.20–2.24 ms per call (x86), 6.00 → 4.27 ms (Pi).
   **Stack:** `SUBTRACT_STACK_BYTES` 4 536 → 50 616 B (the held pairs are
   46 080 B); on thumbv8m `subtract`'s frame (host image) is 50 632 B,
   was about 7.3 KB of chained frames. Holding only the phase (23 040 B)
   and recomputing the cosine and sine measured 2.64–2.68 ms per call
   (x86), against 2.20 for the pairs, so the pairs were kept: the host
   image is the only one that links `subtract`, and the residual it
   needs (720 000 B) dwarfs it. Host image flash 91 140 → 92 296 B.
2. **FFT.** Each of the six stages is a const-generic radix-R pass with
   the twiddles read once per `k` and the blocks inside, replacing one
   loop whose radix, inner loops and twiddle index (`% N` per product)
   were all at run time. The products are the same and summed in the
   same order; the `m = 0` twiddle is exactly [1, −0], so that product is
   the value itself (only the sign of a zero can differ, which no later
   `+ − ×` or the power can see). The old transform stays as a test
   reference: equal bit for bit, zeros aside, on noise at four scales,
   silence, an impulse and a constant. The real-split powers then go to
   the front end 64 at a time, so `log2` and the level quantisation run
   as a plain loop (it vectorises; the floor and top counts are taken
   from the finished row). `frontend_bench` per 80 ms row: 80 → 42 µs
   (stages) → 28 µs (chunks) on x86; 162 → 57 µs on the Pi.
   **Small targets:** small image flash 83 776 → 85 476 B (+1 700 B, the
   unrolled stages), `finish_row` frame 232 → 464 B (the chunk), no new
   soft-float helper call (thumbv8m and thumbv7em: `__aeabi_f2lz`,
   `__aeabi_l2f`, `__aeabi_uldivmod` one call site each, as before). The
   stages should also run faster there (no per-product index arithmetic
   or run-time radix loops), but no Cortex-M was measured.
3. **`Synth` increment.** `as i64 as u32` on 64-bit targets, `as i32 as
   u32` elsewhere (same value on [0, 2³¹)); 32-bit images unchanged in
   helper calls. A clear win on x86 only: AArch64's conversions saturate
   in hardware at either width.
4. **Sum-product in phases.** One round forms all 83 checks' inputs,
   runs tanh over all, the prefix and suffix products per check, atanh
   over all, then adds the messages to the variable sums check by check
   in the original order; each value's operations are unchanged. Not
   inlined, so its 2 656-byte array stays off the min-sum path: the
   small image's decode frame went 2 560 → 2 496 B, the round's own frame
   is 2 744 B (on the call chain only with sum-product); small flash
   +204 B. Inlined it was as fast but put the array in every decode's
   frame (5 056 B on the small image).

**Rejected** (results identical in each; x86 unless stated):

- Holding only the phase in step 1: 2.64–2.68 against 2.20–2.24 ms per
  call (half the stack).
- Copying the fit's sums and correction into locals per block: 2.20–2.21
  against 2.20–2.22 ms per call, no gain.
- `Removal::sample` left to the inliner (not inlined): 2.65 ms per call
  against 2.43 forced inline, and 2.22 with the event-run loop kept.
- FFT stages `#[inline(never)]` (one copy per radix): small flash −300 B
  but feed 35.5 against 28 µs per row.
- FFT twiddle indices advanced incrementally instead of `% N`: 36.4–36.8
  against 28.0–30.0 µs per row.
- The `i64` increment on the Pi: no gain (kept for x86, see 3).

**Identity.** After every step: all unit and integration tests and pins
(including the multi-pass golden hashes and the subtraction pins); a
grid of 960 subtractions (four decodes, eight segment lengths from 960
to the whole waveform, drift on and off, five skips, three cuts) with
residual and estimate hashes equal to the base's (steps 1 and 2);
`paired_bench run` records with timings removed equal to
`s3b/accept` for `host` (38 900 records, every set and block), and to
`s3b/identity` for `host,passes=1` and `small` (34 800 each): 0
differing in each, after each of the four steps. Release `cat-ft8-decode`
built at 4fe722c from this repository's path, sha256
aea5d6bf91c6d94c1a3371028ec508c23fe6b63608121ae6413d4904eacd68ba: on the
800 WAV slots (`host` and `small`, `host,passes=1` and `small`; plain,
`--raw` and `--profile` with timings removed) its output is
byte-identical to 3ed1aa0's (the code of 36cc440).

**CI-equivalent checks** before each commit: fmt, clippy (workspace and
cat-ft8 all features, thumbv8m), workspace tests, thumbv8m build,
`check.sh` (and `--stack` for the sizes above), MSRV 1.75, profile and
no-std tests, docs, the bulk test.

**Not measured:** any Cortex-M (flash and frames only); the Pi with a
CPU-specific build (earlier: no gain from `target-cpu=cortex-a76`).
Still open: the reference generator (`fill_phase` and `skip` in the time
search, about 14% of the x86 profile after these steps) and the time
search's correlations (6%).

## O1 fix round (2026-10-29)

Changes after the O1 checkpoint, each with results bit-identical to 36cc440;
code at 59bb2c7, 2dd54dc and c3f080f. Outputs kept locally (not
committed). Hosts as in O1 (x86-64 desktop; Raspberry Pi 5, Cortex-A76,
2.4 GHz, generic `aarch64-unknown-linux-gnu` build, one benchmark at a
time; throttle flags 0x50000 before and after every run: the sticky
"has occurred" bits only, nothing current).

1. **Caller-provided subtraction scratch** (59bb2c7). The 46 080-byte
   held reference moved off `subtract`'s stack into `SubtractScratch`
   (`const fn new`, so it can be a `static`), passed to `subtract` and
   `subtract_fixed`. `decode_passes*` and `decode_further_passes*` take
   a `PassesScratch` (a `CandidateScratch` and a `SubtractScratch`;
   public fields, so a pass 1 run separately uses its `candidate`) rather
   than a second argument: one object for the caller to place, and
   single-pass users keep the 5.1 KB `CandidateScratch`. The scratch is
   not cleared per call: a call reads only held samples it wrote, and the
   tests run with a NaN-filled scratch that keeps earlier calls'
   contents. `SUBTRACT_STACK_BYTES` 50 616 → 4 536 B. Docs on
   `decode_passes`, `decode_further_passes`, `DecodeConfig::passes` and
   `with_passes` now state the scratch and the stack the passes need.
   V0 API change, no version bump.
2. **Subtraction anchors** (59bb2c7). `anchored_to_the_unheld_subtraction`
   pins 24 residual-and-estimate hashes recorded with 36cc440's
   subtraction (one run of the reference per step): segments 3 840,
   4 080 (just above the fused limit, full hold), 7 680 and the whole
   waveform, drift on and off, skip 30 000 (the first estimate comes
   late), cut 90 000, a start inside the slot and one before it. The
   held/rerun equality test adds segment 4 080. A 1 080-case grid (four
   decodes, segments 960 to the whole waveform with 4 080, drift on and
   off, five skips, three cuts) equals 36cc440's at the final code.
3. **The rerun path was slower** (found by item 2's timing, fixed in
   c3f080f). With segments longer than two symbols the reference is run
   a second time for the subtraction; since O1 that path entered the
   interval logic and a one-sample run per sample, about 12% slower than
   36cc440 (x86 3.44–3.57 against 3.04–3.11 ms per call; Pi 6.76
   against 6.00). Now each block's pairs go into the start of the
   scratch (unused on this path) and are subtracted in runs, with the
   frequency correction in a local over the block (`perf` showed its
   state stored and reloaded every sample). `subcost host seg=N 20 5`,
   per call, three rounds x86 / two Pi, binaries alternated in each:

   | segment | 36cc440 | O1 (11a9221) | final (c3f080f) |
   |---|---|---|---|
   | 3 840 (fused), x86 | 3.06–3.15 | 2.08–2.10 | 2.06–2.09 |
   | 7 680, x86 | 3.07–3.11 | 3.49–3.57 | 2.79–2.86 |
   | whole waveform, x86 | 3.02–3.11 | 3.44–3.49 | 2.80–2.84 |
   | 3 840 (fused), Pi | 6.004 | 4.270 | 4.262 |
   | 7 680, Pi | 6.001–6.005 | 6.757–6.777 | 5.736–5.739 |
   | whole waveform, Pi | 6.001–6.004 | 6.757–6.761 | 5.738 |

4. **FFT precondition** (2dd54dc). `stage` and `power_bins` state that
   skipping the m = 0 product is exact for finite input only (`inf · −0`
   is NaN), which the front end's clamp guarantees, with every output
   below about 1.8·10⁸. The reference test adds subnormal noise, the
   smallest subnormal alternating in sign, signed zeros (alone and with
   a few nonzero values), and full-scale coherent input (±2¹⁶ at bin 0,
   a bin on the grid, one between, and a constant).
5. **`Synth` increment** (2dd54dc). `debug_assert!(x < 2³¹)` in
   `increment`. A test forms every deviation `run` can (each tone triple,
   each sample) at the largest base `Synth::new` accepts (5 956.2495 Hz):
   the largest increment is 2 147 483 392 (2³¹ − 256, two `f32` steps
   below the limit), and the generator runs there in debug builds.

**Stack and flash** (thumbv8m.main-none-eabihf, `check.sh --stack`):

| | 36cc440 | O1 (11a9221) | final (c3f080f) |
|---|---|---|---|
| small image flash | 83 776 B | 85 680 B | 85 680 B |
| host image flash | 91 132 B | 94 128 B | 94 060 B |
| host image static RAM | 241 848 B | 241 848 B | 287 928 B (scratch in `.bss`) |
| `subtract` frame (host image) | 4 504 B | 50 632 B | 4 552 B |
| its time-search closure | 2 800 B | 2 800 B | 2 800 B |
| decoder path frame (small) | 2 560 B | 2 496 B | 2 496 B |
| `finish_row` | 232 B | 464 B | 464 B |
| `SUBTRACT_STACK_BYTES` | 4 536 | 50 616 | 4 536 |

The link check's model RAM went from 256 to 512 KiB: the host image's
statics no longer fit 256 KiB once the scratch is a static (the small
image's 212 280 B still do). Soft-float helper call sites unchanged on
thumbv8m and thumbv7em in all three images (small and encoder-only one
each of `__aeabi_f2lz`, `__aeabi_l2f`, `__aeabi_uldivmod`; host 5, 2, 2).
The `--profile` memory keys of the decode command are unchanged, so
`peak_bytes` still leaves out the subtraction scratch (it left out the
same bytes on the stack before); adding it would change the output.

**Decode time** (`slotcost host,passes=1 host 200 5`, the 200 40-signal
crowded slots, per-slot median of 5 interleaved runs; base, O1 and final
one after another, two rounds, same session):

| | x86 worst / mean, ms | Pi worst / mean, ms |
|---|---|---|
| 36cc440, 3 passes | 224.7 / 202.2, 216.4 / 200.8 | 396.1 / 367.6, 395.6 / 367.1 |
| 11a9221, 3 passes | 142.4 / 132.3, 142.5 / 132.4 | 262.3 / 241.3, 262.2 / 241.4 |
| c3f080f, 3 passes | 143.8 / 132.4, 143.2 / 132.1 | 262.2 / 241.3, 261.5 / 240.7 |
| 36cc440, 1 pass | 39.5 / 38.0, 39.7 / 38.3 | 62.5 / 60.7, 62.3 / 60.5 |
| 11a9221, 1 pass | 26.2 / 25.2, 26.8 / 25.3 | 38.8 / 37.4, 38.9 / 37.4 |
| c3f080f, 1 pass | 27.6 / 25.4, 26.6 / 25.5 | 38.9 / 37.5, 38.7 / 37.3 |

The O1 speed is kept (x86 subtract 71.4–71.6 ms per slot against
71.8–71.9 at O1, 106 at 36cc440; work counts equal).

**Identity**, all at clean commits (`config.txt` without `+dirty`):

- `paired_bench run` at c3f080f (and before at 2dd54dc), timings
  removed: `host` equal to `s3b/accept` (38 900 records, every set and
  block), `host,passes=1` and `small` equal to `s3b/identity` (34 800
  each); 0 differing in each.
- Raspberry Pi 5: the release test suite of c3f080f (all eight test
  executables, ignored tests included: 143 + 12 + 7 + 5 + 2 + 2 + 1
  bulk) passes, so every pin, the multi-pass pins and the subtraction
  anchors hold on AArch64 at run time. `paired_bench run host` and
  `small` on sweep and crowded (main) there: records equal to x86's and
  to `s3b` (15 600 each, 0 differing; the comparison flags 567 of 600
  slots when host is compared with small).
- Release `cat-ft8-decode` built from this repository's path at the
  final commit (sha256 in progress.md): its output on the 800 WAV slots
  (`host` and `small`, `host,passes=1` and `small`; plain, `--raw` and
  `--profile` with timings removed) is byte-identical to 11a9221's and
  3ed1aa0's.

**Not measured:** any Cortex-M at run time (frames and flash only);
whether a real Cortex-M target's stack layout matches the link check's
per-function frames (the chain figures are sums of those).

## Decode time, optimisation round (O2, 2026-10-29)

Four changes, each bit-identical to the O1 fix round's code (c3f080f,
b465b75): 08d5d8e, fbc5857, 27bb0d5 and d6bab4f; then b39ce66 (O1
follow-ups, no change to results). Outputs kept locally (not committed).
Hosts as before: the x86-64 desktop, and a Raspberry Pi 5 (Cortex-A76,
2.4 GHz; generic `aarch64-unknown-linux-gnu` build, one benchmark at a
time; throttle flags 0x50000 before and after every timing run: sticky
bits only).

**Where the time went** at the start (`perf` on x86-64, three passes):
belief propagation 26%, the subtraction's amplitude fit 16%, the
reference generator (`fill_phase` 12%, `skip` 3%), the FFT 10%, the
removal loop 9%, the time search's correlations 7%, the sync search 6%.
In the subtraction alone (`subcost`): fit 33%, generator 30%, removal
18%, correlations 14%. On the Pi (stage timers, per 40-signal slot):
subtraction 147 ms, belief propagation 41, feed 31, sync 18.4. The
annotated hot loops showed two things the O1 profile had not: the
generator's per-sample branches and bounds checks, and, in every
vectorised loop that converts `f32` to an integer, the saturating `as`
conversion done a lane at a time on x86-64 (SSE2 has no saturating
conversion; AArch64's is one instruction and saturates).

`slotcost host,passes=1 host 200 5` (200 crowded 40-signal slots, per
slot the median of 5 interleaved runs; worst / mean ms), each step
against the one before, two rounds, binaries alternated:

| step | x86-64 | Cortex-A76 | stage it targets (per slot) |
|---|---|---|---|
| base (c3f080f code) | 142.8 / 132.1, 142.6 / 132.4 | 262.1 / 241.2, 261.8 / 240.9 | |
| 1. generator in runs (08d5d8e) | 136.9 / 126.8, 138.2 / 127.5 | 237.1 / 219.5, 237.1 / 219.5 | subtract 71.3–71.6 → 66.4–66.8 (x86), 147.2 → 125.9 (A76) |
| 2. `exp2_neg` floor (fbc5857), against 08d5d8e | 138.4 / 129.2, 139.8 / 128.0 → 132.8 / 122.7, 131.0 / 121.5 | 237.8 / 220.2 → 237.2 / 219.7, 237.5 / 220.0 (same code there) | BP 32.0–32.3 → 25.8–26.0 (x86) |
| 3. sync by rows (27bb0d5), against fbc5857 | 134.4 / 122.1, 131.3 / 121.2 → 126.8 / 116.2, 127.8 / 116.4 | 237.2 / 219.7, 237.5 / 220.0 → 223.5 / 206.0, 223.7 / 206.2 | sync 8.53 → 3.55 (x86), 18.4–18.8 → 4.67 (A76) |
| 4. increments without a conversion (d6bab4f) | → 125.8 / 114.7, 123.3 / 114.7 | → 223.6 / 206.2, 223.7 / 206.2 (same code there) | subtract 66.6–66.8 → 64.5 (x86) |
| **final (b39ce66) against base, same session** | **144.3 / 133.6, 145.1 / 132.7 → 123.8 / 114.6, 126.1 / 114.5** | **262.0 / 241.1, 261.9 / 241.0 → 223.6 / 206.1, 223.6 / 206.1** | |

Final stages (x86-64 / Cortex-A76, ms per slot, base → final):
subtract 72.0 → 64.4 / 147.2 → 126.1; belief propagation 32.1 → 26.2 /
41.6 → 41.4; sync 8.55 → 3.6 / 18.35 → 4.67; feed 15.4 / 31.3
unchanged. Single pass (`host,passes=1`): x86 worst 26.7–26.8 → 21.7–22.2
ms, mean 25.5 → 21.0–21.2; A76 38.8–38.9 → 34.2, 37.4 → 32.8.
`subcost host seg=3840 20 5` per call, base → final: x86 2.06–2.12 →
1.85–1.88 ms (three rounds); A76 4.261–4.262 → 3.651 ms (two rounds).

1. **Reference generator in runs** (08d5d8e). `Synth::fill_phase` and the
   time search's skips (`Synth::skip_with`, crate-internal) form the
   phase increments a symbol part at a time: where the previous symbol's
   pulse and the current one's rising half overlap, the rising half
   alone, the falling half alone, the falling half with the next
   symbol's pulse. The products and sums are `run`'s, in the same order;
   the increments are then added in order (integer arithmetic, so the
   skip may sum them in any order). No per-sample branch or bounds
   check: the float part vectorises (on x86-64 the conversion still went
   a lane at a time, see 4). Tested equal, phase and envelope bit for
   bit, to the per-sample generator for five tone sets (two encoded
   messages, two constant tones and a pseudo-random walk, which covers
   some but not all tone triples; the fix round adds windows of a de
   Bruijn sequence that cover every one) and bases up to the highest accepted, blocks of 1 to 4 097
   samples around every run boundary, with skips between. `fill`, `mix`
   and `skip` (the encoder's) are unchanged. `subcost`: x86 2.05–2.09 →
   1.92–1.95 ms per call, A76 4.263 → 3.646.
2. **`exp2_neg` floor without a conversion on x86-64** (fbc5857). The
   sum-product round's tanh loop vectorised but converted `y as i32` a
   lane at a time. On x86-64 ⌊y⌋ now comes from y + 1.5·2²³ (spacing 1
   there, so the sum is y rounded, held in the low mantissa bits), one
   less if that rounded up: the same integer, every float operation
   unchanged (tested at every `f32` in −126…0). Other targets keep the
   conversion.
3. **Sync by rows** (27bb0d5). The candidate search scored each (start
   row, base bin) on its own: 21 spectrogram row lookups and eight levels
   summed per point. It now fills a start row of the score grid at once,
   a spectrogram row at a time, the inner loop along the bins over
   slices. Integer sums of the same terms in another order: equal
   (tested at every bin of 69 start rows, three bands, full and partial
   slots against `costas_score`, which stays for the refinement).
4. **Generator increments without a conversion on x86-64** (d6bab4f). The
   increment, in [0, 2³¹), from the same exact floor: the top 15 bits
   ⌊x / 2¹⁶⌋, the low 16 bits ⌊x − 2¹⁶·⌊x / 2¹⁶⌋⌋ (the scaling exact, the
   difference a multiple of x's spacing below 2¹⁶): the same integer as
   `as i32` (tested at every `f32` in [0, 2³¹)). Small: `subcost` x86
   1.92–1.95 → 1.85–1.89 ms per call (three rounds each, no overlap),
   slot mean −1.5 ms; other targets unchanged.

**Rejected** (results identical in each):

- Step 2's floor on AArch64 as well: 237.8 / 220.2 and 237.9 / 220.4
  against 237.2 / 219.7 and 237.5 / 220.0 (BP 41.2–41.4 either way); its
  one-instruction conversion is kept, and Cortex-M's.
- `as i64` in step 1's lane loop on x86-64 (as `increment` does): 1.93–1.95
  against 1.93–1.99 ms per call, no gain.
- Skipping the envelope multiply in the blocks where the envelope is 1.0
  (all but the first and last; `1.0 · v` is `v`) in the fit, the second
  reference run and the correlations, with the removal's offset counted
  in `f32` instead of converted per sample: x86 1.85–1.90 → 2.11–2.14 ms
  per call, the envelope part alone 2.09–2.13; A76 3.673 → 3.888.

**Identity.** After each step, at its clean commit, built in this
repository's path: all unit and integration tests and pins (multi-pass
pins and the 24 subtraction anchors included); `paired_bench run` with
timings removed equal to the S3 fix round's records for `host` (38 900,
every set and block) and to its single-pass records for `host,passes=1`
and `small` (34 800 each): 0 differing, at 08d5d8e, fbc5857, 27bb0d5,
d6bab4f and b39ce66. The release `cat-ft8-decode` built at b39ce66 from
this repository's path (sha256 in progress.md): its output on the 800
WAV slots (`host` and `small`, `host,passes=1` and `small`; plain,
`--raw` and `--profile` with timings removed) is byte-identical to
b465b75's. Raspberry Pi 5, b39ce66: the release test suite (all eight
test executables, ignored tests included: 149 + 12 + 7 + 5 + 2 + 2 + 1
bulk) passes (throttle flags 0x50000 before and after); `paired_bench
run host` and `small` on sweep and crowded (main) there: records equal to
x86-64's and to the references (15 600 each, 0 differing). That run uses
every core, and the under-voltage flag was current at the start of the
`host` part and the end of the `small` part (0x50005): not a timing run,
and its records are equal, but the board's supply was at its limit.

**Sizes** (`check.sh --stack`, thumbv8m.main-none-eabihf; base → final):

| | base | final |
|---|---|---|
| small image flash | 85 680 B | 86 492 B (+472 `fill_phase`, +340 sync) |
| host image flash | 94 060 B | 94 896 B |
| encoder-only flash | 36 116 B | 36 548 B (+432 `fill_phase`) |
| static RAM (small / host / encoder) | 212 280 / 287 928 / 0 B | unchanged |
| `subtract` / its closure / `blocks` frames (host) | 4 552 / 2 800 / 2 192 B | unchanged |
| `fill_phase` frame (host) | 176 B | 72 B (+ `increments` 88 B, `skip_with` 32 B) |
| sync row closure | (none) | 96 B |

The check program calls `fill_phase` in every image (it exercises each
public floating-point entry point), so all three carry the new one; an
encoder that does not call it is unchanged. thumbv7em-none-eabihf: small
86 006 → 86 782 B, host 94 482 → 95 242, encoder-only 36 158 → 36 586.
Soft-float helper call sites unchanged in all three images on both
targets (small and encoder-only one each of `__aeabi_f2lz`,
`__aeabi_l2f`, `__aeabi_uldivmod`; host 5, 2, 2). On Cortex-M the
conversions of steps 2 and 4 are not used; step 1 removes per-sample
branches there too, step 3 per-point lookups, but no Cortex-M was run.

**Measurement note.** Binaries for the Pi built from extracted source
archives of different commits into one target directory came out stale:
the archives' files carry the commit time, older than the directory's
last build, and cargo's unit hashes do not include the source path, so
cargo kept the previous binary (two "different" builds were the same
file). Caught by their hashes and by a stage time that could not
change; the affected Pi runs were discarded and every timed binary was
rebuilt in its own target directory. The numbers above are from those.

**Not measured:** any Cortex-M at run time.

## O2 fix round (2026-10-29)

Three code commits after the O2 checkpoint, results bit-identical to
b465b75: 0cd4248 (one generator), 7694d25 (sync and `floor_small`
docs), 69c5bd6 (`check.sh` ceilings, CI).

1. **One generator** (0cd4248). `fill`, `mix` and `skip` take their
   phase and envelope from `fill_phase` and `skip_with` over a 64-sample
   stack chunk; the per-sample `run` loop and its comment (the "4–6%
   slower" that 08d5d8e disproved) are gone, so an image that encodes and
   decodes links one generator. `increment` and `increment_lanes` are one
   helper with one cfg site: the exact floor on x86-64, `as i32`
   elsewhere (equal to the old `as i64` on [0, 2³¹), so AArch64's scalar
   path changes its conversion but not its values).
   - Equality: the per-sample generator stays in the tests as the
     reference (`as i64` truncation, as originally); `fill_phase` with
     `skip`/`skip_with` between, and `fill`, `mix` (onto a non-zero
     buffer) and `skip` interleaved, are compared bit for bit with it at
     block sizes across the chunk and run boundaries. The tone sets now
     include a de Bruijn sequence B(8, 3) cut into 79-symbol windows that
     overlap by two, so every tone triple is covered (checked by a test).
     `increment` is compared with `x as i64 as u32` at sampled `f32`
     values and, in the ignored test, at every one in [0, 2³¹) (passes on
     x86-64 and on the Cortex-A76). An out-of-tree program hashed every
     output of `synthesize` (zeroed and non-zero buffers, starts from
     −30 000 to 170 000), `fill`/`mix`/`skip` interleaved at 14 block
     sizes and a whole-waveform `fill`, for 1 000 cases (encoded messages
     and random symbol sets; bases including the smallest positive, 0.4
     and the highest accepted; amplitudes 1, −0.25, 0 and random):
     identical to 0186703's.
   - Encoder time, x86-64 (that program, best of 20, per waveform, three
     alternated rounds, both builds from this repository's path):
     `synthesize` 408.8–411.3 → 368.1–372.9 µs, `fill` in 480-sample
     blocks 608.8–610.8 → 564.7–569.7, `skip` of the whole waveform
     233.1–235.0 → 216.6–220.1. A 32-sample chunk was slower (`fill`
     609–612, `skip` 273–276 µs, against 64's 568–572 and 218–220):
     not kept.
   - Sizes (`check.sh --stack`, 0186703 → 69c5bd6), flash:

     | image | thumbv8m.main | thumbv7em |
     |---|---|---|
     | small | 86 492 → 85 332 B | 86 774 → 85 618 B |
     | host | 94 896 → 93 656 B | 95 234 → 94 002 B |
     | encoder-only | 36 548 → 35 536 B | 36 586 → 35 578 B |

     Static RAM unchanged (212 280 / 287 928 / 0 B). Frames on both
     targets: `fill` and `mix` 160 → 584 B (the chunk's phase and
     envelope, 512 B), `skip` 56 → 296 B (280 in the host image), with
     `fill_phase` (112 B) and `increments` (88 B) below them, so the
     encoder's deepest path grows by about 620 B, to about 780 B. The
     largest frames (the check program's 4.1–4.2 KB, `subtract`'s 4 552 /
     4 560 B and its 2 800 B) are unchanged. Soft-float helper call sites
     unchanged in all six images; five more `__aeabi_memclr4` calls (the
     chunk buffers are zeroed per call).
2. **Docs and tidying** (7694d25). `costas_score`'s doc comment is back
   above it. `costas_scores` counts the Costas arrays present in its main
   loop instead of a pre-pass (a start row with fewer than two still gets
   no score; such rows now do their sums before discarding them; sync
   time unchanged, 3.55–3.57 ms per three-pass slot). `floor_small`
   documents its domain, [−2²², 2²²] (the sum's bits stay exact up to
   2²⁴), and checks it with `debug_assert!`.
3. **Ceilings and CI** (69c5bd6). `check.sh`'s per-image RAM ceilings
   are set in one table that the header and `link.x` refer to; one awk
   pass gives flash and RAM, printed with the ceiling (tested failing at
   a lowered limit). CI's release job runs the ignored `_everywhere`
   tests (both exhaustive floors, about 4 s in release).

**Decode time.** `slotcost host,passes=1 host 200 5`, alternated, x86-64,
worst / mean ms, both builds from this repository's path: 0186703 124.4 /
115.0, 124.3 / 114.5; 69c5bd6 124.2 / 115.1, 124.3 / 114.7 (single pass
20.9–21.1 either way; subtraction 64.4–65.2, sync 3.54–3.56 either way):
no change, as expected (the decoder's code is unchanged but for the sync
loop and addresses).

**Measurement note: the build path moves code.** Built from a worktree
at another path, 0186703 measured 0.8% faster than 69c5bd6 built here,
over eight alternated rounds (three-pass mean 113.4–114.8 against
114.6–116.5 ms, the subtraction 0.4–1.3 ms of it), with the subtraction's
machine code identical but for addresses. The same commit built at the
two paths places `subtract::run` 160 bytes apart, and a variant
without the sync change, built here, gave 69c5bd6's address and 114.4 ms
(the sync change was briefly reverted on the first evidence, then kept). Rebuilt here, 0186703 matched 69c5bd6 (above). Timed comparisons
need both binaries built from the same path, as well as in separate
target directories.

**Identity** (69c5bd6, built in its own target directories): unit,
integration and doc tests (debug with all features, no-std, release)
and the ignored tests in release (the two exhaustive floors, the bulk
run); clippy `-D warnings` on x86-64 and AArch64 (all targets, all
features), thumbv7em and thumbv8m (`--lib --no-default-features`), and
the workspace; `check.sh` and `--stack`; MSRV 1.75 library build;
`paired_bench run` with timings removed equal to the S3 fix round's
records: `host` 38 900 (every set, both blocks), `host,passes=1` and
`small` 34 800 each, 0 differing. The release `cat-ft8-decode` (sha256
in progress.md): its output on the 800 WAV slots (`host` and `small`,
`host,passes=1` and `small`; plain, `--raw` and `--profile`, timings
removed) byte-identical to b465b75's. Raspberry Pi 5: the release test
suite with the ignored tests (lib 149 with the two table printers
skipped, then 2 + 12 + 1 bulk + 7 + 2 + 5, including `increment` and `floor_small` at every value) passes;
throttle flags 0x50000 before and after.

**Not done:** a Cortex-M run time of `fill_phase` and `find_candidates`
(an O2 follow-up); no Cortex-M board was available. Open
item in task_plan.md. Encoder time on the Cortex-A76 not measured.

## CPU and memory per decoded signal (O3, 2026-10-29)

Two code commits, each bit-identical to cc68e22 on every target:
93fa40d (the time search's rotations) and 81db219 (the subtraction's
reference held in idle memory). Outputs kept locally (not committed).
Hosts as before: the x86-64 desktop, and a Raspberry Pi 5 (Cortex-A76,
2.4 GHz; generic `aarch64-unknown-linux-gnu` build, one benchmark at a
time, throttle flags 0x50000 before and after every run: sticky bits
only). Every timed pair was built from this repository's path into its
own target directory, binaries compared by hash, and run alternately.

**Where the time and memory went** at cc68e22 (`slotcost host,passes=1
host 200 5`, 40-signal slots, three passes, ms per slot, x86-64 /
Cortex-A76): subtraction 65.5 / 126.4 (34.5 calls, 1.89 / 3.66 ms per
call), belief propagation 26.1 / 41.5, feed 15.5 / 31.3, sync 3.6 / 4.7,
soft bits 3.2 / 2.4. `perf` (x86-64, three passes): sum-product round
25%, the subtraction's fit loop (`run`) 19%, the FFT 11%, the removal
loop 11%, the generator's increments 11%, symbol correlations 8%,
`fill_phase` 2.4%, the time search's `score` 2.1%. Memory (thumbv8m link
check, static RAM): small 212 280 B, of which the spectrogram is 161 920
(76%) and the front end 30 720; host 287 928 B, of which the
subtraction's `SubtractScratch` 46 080; the multi-pass caller's residual
720 000 B more. The spectrogram, front end and residual cannot shrink
without changing results (below); the subtraction scratch could.

`slotcost host,passes=1 host 200 5`, worst / mean ms, three passes, two
alternated rounds each (both hosts in one session, base and both steps
in each round):

| | x86-64 | Cortex-A76 | subtraction per slot, x86-64 / A76 |
|---|---|---|---|
| base (cc68e22) | 128.0 / 116.0, 127.4 / 117.1 | 223.9 / 206.4, 224.0 / 206.4 | 65.7–66.3 / 126.4 |
| 1. rotations once per trial (93fa40d) | 124.1 / 114.0, 124.1 / 114.6 | 220.5 / 203.4, 220.6 / 203.5 | 63.5–63.9 / 123.5 |
| 2. reference in idle memory (81db219) | 127.7 / 116.8, 125.6 / 116.6 | 219.4 / 202.6, 219.6 / 202.7 | 65.7–65.9 / 122.7 |

Belief propagation, feed and sync unchanged by both (x86-64 26.2–26.6,
15.5–15.6, 3.6; A76 41.3–41.4, 31.3, 4.7). Single pass unchanged (the
steps touch only the subtraction). `subcost host seg=3840 20 5` per call:
x86-64 1.906–1.941 → 1.846–1.902 (step 1, four rounds) and the A76 3.661
→ 3.577 (step 1) → 3.555 (step 2).

1. **Rotations once per trial** (93fa40d). The time search's score of a
   trial start error turned each of the six groups' 15 tone-step sums
   by the step's phase, taking the cosine and sine of the same 15 phases
   six times (and a 64-bit division for each phase). They are now taken
   once per trial and shared by the groups, and likewise in the
   frequency estimate; the products and sums are unchanged. `score`'s
   frame on thumbv8m 72 → 184 B (the 15 rotations), on the time search's
   path below `subtract`'s own frame.
2. **The subtraction's reference in idle memory** (81db219). The
   multi-pass decoder's subtractions held their reference (46 080 B) in a
   caller-provided `SubtractScratch` inside `PassesScratch`. Between one
   pass's search and the next pass's feed the spectrogram's level buffer
   (161 920 B or more for the standard bands) is idle, and the next feed
   rebuilds every row, so the subtractions now hold the reference there.
   - API (V0, no version bump): `subtract(audio, decode, skip, config,
     scratch: &mut [u8])` takes any byte buffer idle during the call: at
     least `SUBTRACT_MIN_SCRATCH_BYTES` (1 920, one block of the
     reference), best `SUBTRACT_SCRATCH_BYTES` (46 080) or more; with
     less, the reference is run a second time where a segment and a half
     does not fit (the existing rerun path; slower, same result); below
     the minimum, `BufferTooSmall`. `decode_passes*` and
     `decode_further_passes*` take the `CandidateScratch` of `decode`;
     `PassesScratch` and `SubtractScratch` are gone. This settles the
     open `PassesScratch` question: there is no combined type to split,
     and a caller can lend `subtract` whatever buffer it has idle.
   - The reference is stored as native-order bytes and read back bit for
     bit. A test runs `subtract` with scratch from 1 920 bytes through
     odd sizes and rings that hold a segment and a half of the shorter
     segments but not the longer, to more than needed, NaN-filled and
     reused: identical residuals and estimates; one byte short is
     `BufferTooSmall` and leaves the audio as it was.
   - The spectrogram after `decode_passes`: the last pass's rows, as
     before (tested equal to a fresh spectrogram of the residual that
     pass decoded), except after a decode stopped by its callback after
     a subtraction, when it is empty (`rows_filled` 0), as after
     `start_slot`. Documented on `decode_passes`.
   - Time: three forms of the hold's write loop were measured, and they
     disagree between the two hosts. The kept one (a ring index per
     sample) is the fastest on the A76 (`subcost` 3.555 against 3.577
     ms per call for step 1; slot mean 202.6–202.7 against 203.4–203.5)
     but gives back step 1's gain on x86-64 (slot mean 116.6–116.8
     against 114.0–114.6; subtraction 65.7–65.9 against 63.5–63.9). The
     alternatives are under "Rejected".
   - `cat-ft8-decode`'s maximum resident size on a 40-signal WAV, three
     passes (`/usr/bin/time`, three runs each): 3 860–3 908 → 3 752–3 784
     KB (its `Box<PassesScratch>` is now a `Box<CandidateScratch>`; the
     rest of the drop is the allocator's).

**Memory, before → after** (thumbv8m.main-none-eabihf link check, `check.sh
--stack`; the host image runs a decode and a subtraction):

| | cc68e22 | 81db219 |
|---|---|---|
| static RAM, small image | 212 280 B | 212 280 B |
| static RAM, host image | 287 928 B | **241 848 B** (−46 080) |
| static RAM, encoder only | 0 | 0 |
| RAM ceiling (check.sh), link map RAM | 256 KiB small/encoder, 512 KiB host; 512 KiB | 256 KiB every image; 256 KiB |
| flash, small / host / encoder | 85 332 / 93 656 / 35 536 B | 85 332 / 93 764 / 35 536 B |
| multi-pass working memory per stream (host config, residual included) | 956 232 B (`peak_bytes`) + 46 080 B subtraction scratch = 1 002 312 B | **956 232 B** (`peak_bytes`, unchanged key, now the whole of it) |
| `subtract` frame (host image, with its caller) / time-search closure | 4 552 / 2 800 B | 4 600 / 2 800 B |
| second-run `blocks` frame (host image) | 2 192 B | inlined (no separate frame; the caller grew 48 B) |
| `SUBTRACT_STACK_BYTES` | 4 536 | 4 536 |
| sum-product round frame | 2 744 B | 2 744 B |
| encoder's deepest path (`fill`/`mix` 584 B) | unchanged | unchanged |

The host image's statics fit 256 KiB again, so every image now has the
same ceiling and the link map is back to 256 KiB. Soft-float helper call
sites unchanged (`check.sh`: no `f64` helper in any image). (That figure
is the link-check image, which runs one decode and one subtraction on a
short residual. A real multi-pass Cortex-M33 stream needs about 956 KB,
three quarters of it the 720 000 B residual, so this step saved about 5%
of the real per-stream memory.)

**Rejected** (results identical in each):

- **Belief propagation over the 522 edges** (the sum-product round's
  tanh and atanh passes over the graph's edges instead of 83 checks × 8
  lanes, 21% fewer values; the round's array 2 656 → 2 088 B). x86-64:
  belief propagation 26.1–26.4 → 22.5 ms per three-pass slot (−14%),
  slot mean 115.9 / 118.9 → 113.4 / 112.1; single pass 12.3–12.8 →
  10.5–10.7. A76: single pass 19.35 → 19.8 (+2.3%), three passes 41.3 →
  42.3. Padding the edges to 528 (whole vectors): the same on the A76
  (19.84). A row of seven per check (581 values): x86-64 11.6–11.7, A76
  21.4. No layout won on both; not kept. (The min-sum path's time moved
  6.39 → 6.15 ms on the A76 with the unrelated code change: layout.)
- **Cosine and sine from one quadrant decode** in the subtraction's loops
  (`(cos, sin)` share the quadrant and read the quarter wave at x and
  2³⁰ − x; tested equal at every phase): `subcost` x86-64 1.868–1.869 →
  1.892, A76 3.650 → 3.680 ms per call. Slower on both.
- **The hold written in ring segments** (chunks of the ring, no per-sample
  index): x86-64 `subcost` 1.76–1.84 against 1.82–1.86 for step 1, slot
  mean 112.7–112.8 (subtraction 61.8–61.9); A76 3.629 against 3.577,
  slot mean 205.5 against 203.5. Faster on x86-64, slower on the A76.
  **As a chained iterator** (the ring from the write position, then from
  its start): x86-64 1.98–2.04, A76 3.66. Slower on both.
- **The CLI freeing its 16-bit samples** before allocating the decoder's
  buffers: maximum resident size 3 716 → 3 660 KB on a 40-signal WAV (the
  WAV read sets the peak); not worth a change.

**Proposal for a separate gated round: an `i16` residual** (result-
changing; prototyped, not committed). The residual a
multi-pass caller keeps is 720 000 B of `f32` per stream, three
quarters of the host configuration's working memory. Held as 16-bit
samples (each subtraction's output rounded to the nearest 1/32 768,
clamped, as an `i16` buffer would store it) it is 360 000 B. Measured
with the prototype (each subtraction followed by that rounding of the
buffer), `paired_bench run host` against the S3 fix round's accepted
records, every set and both blocks (38 900 slots): verdict PASS on every
set; messages lost / gained (b / c) crowded 0 / 2 (fresh) and 0 / 0
(main), fading 0 / 3 and 3 / 4, wide 1 / 9 and 5 / 4, sweep, edges and
empty 0 / 0 (22 gained, 9 lost in all); no false decodes in either. On
the 800 WAV slots (`host`): 10 696 decodes
either way. So rounding the residual to 16 bits costs nothing measurable
here. It would change results (so it needs its own acceptance round),
the residual's type in `decode_passes` / `decode_further_passes` /
`subtract` (an `&mut [i16]`, or a choice of either), and the
subtraction's write (round and clamp, one more operation per sample);
its speed was not measured (the prototype rounds the whole buffer after
each subtraction).

**Other memory, not changed:**

- The spectrogram (161 920 / 179 584 B) is the decoder's: every row
  and bin of it is read by the search. Only fewer rows or bins (coarser
  time or frequency steps) would shrink it, and that changes results.
- The front end's window (15 360 B) is redundant in passes 2 and on,
  whose input is the caller's whole buffer; a whole-slot feed path that
  reads rows straight from the buffer would let a multi-pass-only caller
  drop it (bit-identical, an API addition), but pass 1 fed as audio
  arrives needs it. Not attempted.
- Fewer spectrogram rebuilds: the rows whose 3 840 samples no
  subtraction touched are equal to the previous pass's, so a later pass
  could skip them bit-identically; with 40 signals spread over ±1 s
  few rows qualify (the slot's first and last few), worth about 10% of
  feed (a few ms of 206 on the A76). Not attempted. Since 81db219 the
  subtractions' held reference overwrites the level buffer's first rows
  (46 080 B is rows 0–47 of the host band's 976-byte rows, 0–52 of the
  small band's 880-byte rows), which are among
  the few such rows; to pursue this, hold the reference at the end of the
  buffer instead.

**Identity.** At 93fa40d and 81db219 (each a clean tree, built in this
repository's path): unit, integration and doc tests, the 24 subtraction
anchors and the exhaustive `_everywhere` tests (release); `paired_bench
run` with timings removed equal to the S3 fix round's records for `host`
(38 900, every set and block) and its single-pass records for
`host,passes=1` and `small` (34 800 each): 0 differing at each. The
rejected belief-propagation commit (later dropped) was equal too. The
release `cat-ft8-decode` at 81db219 (sha256 in progress.md): its output
on the 800 WAV slots (`host` and `small`, `host,passes=1` and `small`;
plain, `--raw` and `--profile` with timings removed) is byte-identical to
b465b75's, profile memory keys included (`peak_bytes` already left the
scratch out). Raspberry Pi 5 at 81db219: the release test suite with the
ignored tests (lib 153, then 2 + 12 + 1 bulk + 7 + 2 + 5) passes;
throttle flags 0x50000 before and after.

**Not measured:** any Cortex-M at run time (flash, frames and static RAM
only).

## O3 fix round (2026-10-29)

Three code commits after the O3 checkpoint, results bit-identical to
cc68e22 on every target: 37aa031 (no stale candidates after a stop),
13f88ed (size-independence tests where the rerun path runs), b37df17
(stack budget under the RAM ceiling; subtraction docs). Hosts and timing
hygiene as in O3 (every timed pair built from this repository's path into
its own target directory, hashes compared, rounds alternated, throttle
flags 0x50000 before and after every Raspberry Pi run).

**Decision rule** (set in the O3 fix round): a difference under about
2% on the Cortex-A76 target is noise (code placement alone moves a slot
by 0.4–0.8%), so between two forms within it the simpler one is kept.

1. **A stop leaves the decoder as after `start_slot`** (37aa031). The
   subtractions' level buffer was taken through the spectrogram once per
   subtraction (about 35 times a slot), emptying the spectrogram but not
   the search's candidates, so a decode stopped after a subtraction left
   100 candidates over no rows. `further_passes` now calls
   `Decoder::start_slot` once before a pass's subtractions and takes the
   buffer once for all of them. `PassesReport` is unchanged (its counts
   were taken before). The stop test checks `candidates()` is empty and
   `decode_candidate(0)` is `None` (it failed before the change), and
   that the candidates are kept when no stop is asked for.
2. **Tests where the rerun path runs** (13f88ed). A decoder test on a
   1000–1500 Hz band (level buffer 32 384 B, under the 46 080 B of one
   run's held reference) compares an exact-size level buffer with one
   padded to `SUBTRACT_SCRATCH_BYTES`: the same decodes, report and
   residual, whole and with a skip of 12 000 and a cut at 150 000 (two
   passes, about 5 s in a debug build). The scratch-size test of
   `subtract` now runs its eight sizes over both starts, both skip/cut
   pairs and three segment lengths of the held-and-rerun test, one buffer
   per size kept across the cases (about 4.7 s in debug).
3. **Ceiling, stack and docs** (b37df17).
   - `check.sh`: every image's static RAM ceiling is the link map's
     256 KiB less a stated 16 KiB stack budget (245 760 B); the deepest
     path measured is about 14 KB (the host image's subtraction with the
     check program's frames). Set in `check.sh` rather than as a linker
     `ASSERT`: it is already the one place ceilings live, and its failure
     names the image and both figures. With a 20 KiB budget (ceiling
     241 664 B) the host image (241 848 B) fails, as intended; with 16 KiB
     all three pass (headroom 33 480 / 3 912 / 245 760 B).
   - `SUBTRACT_STACK_BYTES` counts the time search's 15 rotations
     (120 B): 4 536 → 4 656. ADR 0024 gives `subtract`'s frame (with its
     caller, host image) as 4 600 B. Frames and sizes otherwise as at
     81db219 (flash 85 332 / 93 764 / 35 536 B, static 212 280 /
     241 848 / 0 B).
   - Rustdoc link text names the importable `SUBTRACT_*` constants.
     `decode_passes` states when a level buffer is too small for one run
     (`Band::spectrogram_len` under 46 080 B: 235 base bins or fewer, a
     band up to 731.25 Hz wide) and that padding it restores the single
     run; `subtract` documents that the settings are checked before the
     scratch size and that every error leaves the audio unchanged.

**Rejected: `held` sliced to the ring once** (`&mut held[..ring * PAIR]`
before the loops, for the per-sample index's bounds check). The bounds
check stays (the function grows 14 B on x86-64 and 28 B on AArch64).
`subcost host seg=3840 20 5`, four alternated rounds: x86-64 1.858–1.920
(mean 1.881) → 1.836–1.895 (1.865) ms per call, within the rounds'
spread; A76 3.566–3.578 → 3.566–3.568. `slotcost` on the A76, two
alternated rounds: 220.3 / 203.4 both → 220.2 / 203.2 both (−0.1%). No
gain on the target and none clear on x86-64: not kept.

**Time at the final commit.** A76 `slotcost host,passes=1 host 200 5`:
220.3 / 203.4 ms (subtraction 123.2, belief propagation 41.5, feed 31.3,
sync 4.7), against 219.4–219.6 / 202.6–202.7 at 81db219 (+0.4%: the step
1 change is outside every loop; `subcost`, which calls `subtract`
directly and whose code is unchanged, also moved 3.555 → 3.566–3.578, so
it is placement). Under the decision rule.

### Where a three-pass slot's time goes on the Cortex-A76

`perf` is not usable on the Pi without system packages (no `perf`
binary; `perf_event_paranoid` 2 would allow user-space sampling of the
user's own processes). Instead, a SIGPROF-style wall-clock sampler (not
included) in a copy of `paired_bench`: a 487 µs interval timer signal,
the interrupted PC from the signal context and the unwound stack, inlined
frames resolved from line tables (the decoder's hot functions have the
same code sizes as in the timed binary; only the decoder's per-candidate
entry points differ, by the hook for the count below). Samples were
taken only during the three-pass side of `slotcost host,passes=1 host
200 5` (1 000 slot decodes, 455 062 samples, every one unwound through
the interrupted frame; the run's three-pass mean was 208.0 ms against
203.4 unsampled, +2.3%). Shares of the slot, and ms at 203.4:

| | share | ms per slot |
|---|---|---|
| **subtraction** | **56.7%** | **115.4** |
| fit loop (reference cos/sin, least-squares sums, holding the reference) | 25.5% | 51.8 |
| removal (`Removal`: amplitude interpolation, subtraction, residue sums) | 13.3% | 27.1 |
| time search: symbol correlations and scores | 10.1% | 20.6 |
| generator, fit run (`fill_phase`, increments) | 4.5% | 9.2 |
| time search: its generator runs | 3.3% | 6.7 |
| frequency estimate | < 0.01% | 0.0 |
| setup (encode, `Synth::new`) | 0.04% | 0.1 |
| **belief propagation** | **24.7%** | **50.2** |
| **feed (spectrogram, FFT)** | **14.5%** | **29.4** |
| sync | 2.4% | 4.8 |
| soft bits | 1.4% | 2.8 |
| refine, SNR, other | 0.4% | 0.8 |

Leaves (share of the slot): the reference's cosine and sine (`sin_turns`)
8.0% in the fit loop and 4.1% in the time search, 12% in all; the fit
loop's own body 11.9%; the removal's sums (`Sums::add`) 4.9% and its loop
4.3%; the input conversion (`input`) 2.2 + 1.9 + 1.9% in the fit loop,
removal and time search; the sum-product round 9.0% with `exp2_neg` 6.2%
and `log2` 4.3%; the FFT stages 5.6% and complex multiplies 3.2%. The
generator is 7.8% in all, half of it the time search's. The same sampler
on x86-64 (289 949 samples, 126.4 ms mean): subtraction 50.6% (fit loop
20.1%, removal 10.3%, time search 13.9%, generator 6.2%), belief
propagation 24.9%, feed 12.1%, consistent with the O3 `perf` profile;
there the bench's own stage clock reads took 6.7% of the samples (under
0.01% on the A76), so x86-64 stage times carry that overhead.

So on the target the subtraction's fit and removal loops (39% of the
slot) and the reference's trigonometry are the next CPU candidates; the
frequency estimate and setup are nothing.

### Belief propagation repeated on identical LLRs

Counted out of tree (a hook in a copy of the decoder, not committed):
every candidate's LLRs (bit patterns) and outcome, per pass, on the same
200 slots; a candidate of pass 2 or 3 counts as a repeat when a candidate
of an earlier pass at the same (start_row, base) failed with LLRs equal
bit for bit (hash, then exact compare).

| | candidates | BP rounds |
|---|---|---|
| passes 2–3, all | 20 000 | 521 581 |
| repeating an earlier failure, identical LLRs | 463 (2.3%) | 13 871 (2.7%) |
| same position, LLRs differ | 4 904 | 90 126 |
| repeating an earlier success | 0 | 0 |

The repeats are 23 in pass 2 and 440 in pass 3, almost all at the round
limit (13 871 / 463), and every one ended as before (0 outcome
mismatches, as BP is deterministic). They are 1.4% of the slot's 980 977
rounds; at the A76's 41.5 ms of belief propagation per slot that is
about 0.6 ms of 203 (0.3%), about 0.4 ms of 126 on x86-64. An exact skip
would also need the stored LLRs (696 B per failed candidate) or a
collision-proof key, and the same failure accounting. Not worth it.

**Identity.** At 37aa031 and b37df17 (clean trees, built in this
repository's path): `paired_bench run` with timings removed equal to the
S3 fix round's records (`host` 38 900, every set and block;
`host,passes=1` and `small` 34 800 each; 0 differing); the release
`cat-ft8-decode`'s output on the 800 WAV slots (`host` and `small`,
`host,passes=1` and `small`; plain, `--raw` and `--profile`) byte-identical
to b465b75's. At b37df17: fmt, clippy `-D warnings` (workspace; cat-ft8
all targets and features on x86-64 and AArch64; thumbv8m and thumbv7em
library), workspace tests, profile and no-std tests, release lib tests
(150 + 4 ignored) with the 24 subtraction anchors, the exhaustive
`_everywhere` tests in release, the bulk run, `check.sh` and `--stack`,
MSRV 1.75, docs. Raspberry Pi 5 at b37df17: the release test suite with
the ignored tests (lib 154, then 2 + 12 + 1 bulk + 7 + 2 + 5) passes;
throttle flags 0x50000 before and after. The only visible change is the
empty candidate list after a stopped decode.

**Not measured:** any Cortex-M at run time.


## CPU per decoded signal, fourth round (O4, 2026-10-30)

Three code commits, each bit-identical to 91ee9fe on every target:
872bd59 (the time search's symbol middles skip the envelope), 657543e
(the sum-product round over the edges, grouped by check degree) and
66ba24b (one decoder helper lends the level buffer). Hosts and timing
hygiene as in O3: every timed binary built from this repository's path
into its own target directory, hashes compared, rounds alternated, one
benchmark at a time, throttle flags 0x50000 before and after every
Raspberry Pi run (sticky bits only). Decision rule as in the O3 fix
round (under about 2% on the Cortex-A76 is noise).

**Slot time per commit** (`slotcost host,passes=1 host 200 5`, all four
binaries in each of two alternated rounds, both hosts; worst / mean ms of
the three-pass 40-signal slot; single-pass mean; stage means):

| | Cortex-A76 three passes | single pass | subtraction / BP | x86-64 three passes | single pass | subtraction / BP |
|---|---|---|---|---|---|---|
| base (91ee9fe) | 220.2 / 203.3, 220.4 / 203.5 | 32.8–32.9 | 123.2 / 41.4–41.6 | 128.1 / 114.1, 123.6 / 114.4 | 20.9–21.0 | 64.4–64.7 / 25.8 |
| 1. middles skip the envelope (872bd59) | 219.1 / 202.4, 219.1 / 202.5 | 32.8–33.0 | 122.3–122.4 / 41.4–41.5 | 125.4 / 115.0, 122.5 / 113.6 | 21.0–21.2 | 64.2–65.0 / 25.6–26.0 |
| 2. BP edges by degree (657543e) | 218.1 / 201.3, 218.5 / 201.3 | 32.3 | 122.4 / 40.3 | 120.4 / 110.3, 120.8 / 110.3 | 18.8–19.0 | 64.8–64.9 / 21.4–21.5 |
| 3. level-buffer helper (66ba24b) | 218.0 / 201.2, 218.0 / 201.2 | 32.2 | 122.5 / 40.0 | 120.5 / 110.3, 119.1 / 110.3 | 18.9–19.0 | 64.8–64.9 / 21.4–21.5 |

In all, the A76 three-pass slot 203.4 → 201.2 ms (−1.1%), single pass
32.85 → 32.2 (−2.0%); x86-64 114.3 → 110.3 (−3.5%), single pass 21.0 →
18.95 (−10%). Feed and sync unchanged (A76 31.3 / 4.7, x86-64 15.4 /
3.6).

1. **The time search's middles skip the envelope** (872bd59). The symbol
   correlations read only the middle half of each symbol (blocks
   starting T/4 … 3T/4 of 1 920 samples), which lies clear of the
   waveform's 240-sample ramps (`RAMP_SAMPLES <= T / 4`, asserted at
   compile time; a debug assertion checks every envelope value read
   there is 1.0), so the multiply by the envelope (exactly 1.0) is
   dropped. `subcost host seg=3840 20 5`, two alternated rounds: A76
   3.570–3.583 → 3.540–3.541 ms per call (−0.9%); x86-64 1.833–1.874 →
   1.841–1.860 (three rounds; unchanged). Slot: A76 −0.4%, x86-64
   within the rounds' spread. Kept as free (one multiply and one
   argument fewer; the gain is under the 2% rule).
2. **Sum-product round over the edges, grouped by check degree**
   (657543e). The round ran its tanh and atanh over 83 checks × 8 lanes
   (664 values) for 522 edges. The edges are now laid out by degree: the
   59 degree-6 checks in order (354 values), then the 24 degree-7 ones
   (168); each check finds its place from a running offset per degree
   while the inputs are gathered and the messages scattered in check
   order, so `msg`, the sums into `next` and their order are unchanged.
   The tanh and atanh loops run over 522 values and the products over
   whole checks of a constant size (`others_products::<6>` and `<7>`,
   unrolled). No new table (`TABLE_BYTES` and the decode command's
   `static_table_bytes` unchanged). Belief propagation per three-pass
   slot: A76 41.4–41.6 → 40.3 ms (−3.0%), x86-64 25.8 → 21.4–21.5
   (−17%); slot A76 −0.5%, x86-64 −3.5%. This is the "materially
   different design" asked for: O3's 522-edge form (variable-degree
   products over a flat array) was +2.3% on the A76; with fixed-size
   checks it is −3.0%. **Under the 2% rule at slot level on the A76
   (−0.5%), kept for the repeatable stage gain (both rounds 40.29–40.31
   against 41.39–41.63), the x86-64 gain and the smaller frame;** it is
   its own commit, so it can be dropped alone.
3. **One decoder helper lends the level buffer** (66ba24b; an O3
   follow-up). `Decoder::idle_levels` empties the candidate list and returns
   `Spectrogram::idle_levels()`, so `further_passes` no longer resets the
   spectrogram twice. The stop test (empty candidates after a stopped
   decode) still passes. No timing effect (A76 201.2–201.3 either side).

**Memory and flash** (`check.sh --stack`, thumbv8m.main-none-eabihf):

| | 91ee9fe | 872bd59 | 657543e | 66ba24b |
|---|---|---|---|---|
| flash small / host / encoder | 85 332 / 93 764 / 35 536 B | 85 332 / 93 716 / 35 536 | 85 544 / 93 928 / 35 536 | 85 544 / 93 928 / 35 536 |
| static RAM small / host / encoder | 212 280 / 241 848 / 0 B | same | same | same |
| sum-product round frame | 2 744 B | 2 744 | **2 176** | 2 176 |
| `subtract` frame (with caller) / time-search closure | 4 600 / 2 800 B | same | same | same |

Static headroom unchanged (host 3 912 B). The deepest sum-product
decoder path is about 568 B shallower (ADR 0024: about 8.7 KB, was 9.3).
Flash +212 B for the unrolled products. No `f64` helpers in any image.

**Rejected** (results identical in each; numbers per candidate build
against the same base, alternated):

- **Joint `cos_sin_turns`** (the first O3 target): one function sharing the
  quadrant, mask and mirror, the two signs applied by XOR of the sign
  bit, the last table interval clamped (`min`) instead of a branch
  (proved equal to `(cos_turns, sin_turns)` at all 2³² phases on
  x86-64: 0 mismatches, 1.6 s threaded). `subcost`: A76 3.566–3.567 →
  4.087–4.088 ms (+14.6%), x86-64 1.840–1.919 → 1.921–1.931. The
  AArch64 disassembly shows why: at the base, LLVM already shares the
  decode (one `and`, one `sub`, both mirrors selected from one `tst`);
  the joint form was SLP-vectorised into two-lane vectors with lane
  inserts, `zip` and `dup` moves. O3's simpler joint form was +0.8%.
  Not retried further; no exhaustive Pi run, as nothing was kept.
- **`input()` fast path** (the second O3 target): `abs <= INPUT_LIMIT` returns x,
  otherwise a `#[cold]`, not-inlined function. The fused fit loop went
  79 → 70 AArch64 instructions per sample, yet `subcost` A76 3.566–3.567
  → 3.581–3.583 ms (+0.4%), x86-64 1.840–1.919 → 1.978–1.989; in a
  slot run (built with step 2's BP change) subtraction 123.4–123.5 →
  125.1–125.2 ms (+1.3%). The base `input` is already a branch on the
  bit pattern for non-finite values plus two selects.
- **Exact `f32` counter in the removal** (a small O3 target; `t = if d > 0.0 { d }
  else { 0.0 }`, d counted from `j0 − from`, exact below 2²⁴):
  `subcost` A76 3.570–3.583 → 3.785–3.786 (+5.8%), x86-64 1.833–1.874 →
  1.904–1.928 (+3%). The integer index and conversion it replaces are
  cheaper than the float add's loop-carried chain.
- (Step 2's first build also carried the `input()` change: BP 41.6 →
  40.3 there too, subtraction +1.3%; the BP change alone is step 2.)

**Not done:** feed row skipping (deferred at O3); the BP repeat
skip, caching the time-search reference and reordering sums (dropped
at O3).

### Where a three-pass slot's time goes on the Cortex-A76, at 66ba24b

The O3 fix round's sampler (a SIGPROF-style sampler, not included), ported onto
66ba24b, same method: a 487 µs interval timer during the three-pass side
of `slotcost host,passes=1 host 200 5` (1 000 slot decodes, 450 810
samples, every one unwound through the interrupted frame; that run's
three-pass mean 205.9 ms against 201.2 unsampled, +2.3%). Shares of the
slot, and ms at 201.2 (O3 fix round at 203.4 in brackets):

| | share | ms per slot |
|---|---|---|
| **subtraction** | **56.9%** | **114.5** (115.4) |
| fit loop (reference cos/sin, least-squares sums, holding the reference) | 24.2% | 48.7 (51.8) |
| removal (amplitude interpolation, subtraction, residue sums) | 15.1% | 30.4 (27.1) |
| time search: symbol correlations and scores | 9.6% | 19.4 (20.6) |
| generator, fit run (`fill_phase`, increments) | 4.5% | 9.0 (9.2) |
| time search: its generator runs | 3.4% | 6.9 (6.7) |
| frequency estimate, setup | < 0.1% | 0.1 |
| **belief propagation** | **24.3%** | **48.9** (50.2) |
| **feed (spectrogram, FFT)** | **14.6%** | **29.4** (29.4) |
| sync | 2.4% | 4.8 |
| soft bits | 1.4% | 2.8 |
| refine, SNR, other | 0.4% | 0.8 |

Leaves (share of the slot): the reference's cosine and sine
(`sin_turns`) 7.6% in the fit loop and 3.9% in the time search; the fit
loop's own body 11.3%; the removal's sums (`Sums::add`) 5.6% and its
loop 4.8%; `input` 2.1 + 2.2 + 1.7% (fit, removal, time search); belief
propagation's round 7.3% with `exp2_neg` 6.2%, `log2` 5.0%, the products
1.1%; the FFT stages 5.8% and complex multiplies 3.2%; the generator
7.9% in all. The fit loop's and removal's split moved between the two
profiles (inlining attributes the shared `Sums::add` and `input` by call
site); together 78.9 → 79.1 ms, unchanged.

**Is the bit-identical CPU work near its floor?** For the subtraction,
probably yes. Its two big loops (fit and removal, 39% of the
slot) are each one pass over the samples whose arithmetic is fixed by
the results: six serial accumulators in `Sums::add` (order fixed), the
interpolated table lookups for cosine and sine (no NEON gather; LLVM
already shares their quadrant decode), and the input check. This round
tried every per-sample trim on the O3 target list; each either did nothing
or lost on the A76 (−9 instructions per sample in the fit loop made it
slower), so the loop runs at its dependency and port limits, not its
instruction count. What is left there is the reference generator (7.9%)
and the time search's correlations (9.6%), neither with an identified
bit-identical gain. Belief propagation still has headroom only in
`exp2_neg`/`log2` (11% of the slot), whose arithmetic is pinned bit for
bit; a hand-vectorised round (NEON intrinsics, same per-lane
operations) might recover a little but adds unsafe, target-specific
code for a few percent. The feed's FFT (9%) was optimised in O1. So the
remaining bit-identical gains are each ≤ 2% of the slot; larger savings
need result-changing work (the `i16` residual's memory proposal, fewer
candidates or BP rounds, or a cheaper reference).


**Identity.** At 872bd59, 657543e and 66ba24b (each the working tree of
that commit, built in this repository's path): unit, integration, doc,
profile and no-std tests, the 24 subtraction anchors and the exhaustive
`_everywhere` tests (release lib 154 with the ignored); `paired_bench
run` with timings removed equal to the S3 fix round's records (`host`
38 900, every set and block; `host,passes=1` and `small` 34 800 each; 0
differing); the release `cat-ft8-decode`'s output on the 800 WAV slots
(`host` and `small`, `host,passes=1` and `small`; plain, `--raw` and
`--profile`, timings removed) byte-identical to b465b75's. At 66ba24b:
fmt, clippy `-D warnings` (workspace; cat-ft8 all targets and features;
thumbv8m library), workspace tests, `check.sh` and `--stack`, MSRV
1.75, docs, the bulk run. Raspberry Pi 5 at 66ba24b: the release test
suite with the ignored tests (lib 154, then 2 + 12 + 1 bulk + 7 + 2 + 5)
passes; throttle flags 0x50000 before and after. The release
`cat-ft8-decode` at 66ba24b: sha256 in progress.md.

**Not measured:** any Cortex-M at run time (flash, frames and static RAM
only). The BP change also runs on the M33 host image (sum-product);
its effect there (scalar FPU, no vectors) is untested.


## CPU per decoded signal, fifth round (O5, 2026-10-30)

The last bit-identical round before the gated, result-changing work.
Four code commits, each identical in results to 46fdf4d on every
target: f0bb4ab (the decoder's subtractions skip the report's sums),
02ff55e (belief-propagation messages kept in the edge layout), 92970ba
(tests of that layout and round) and 1b47a55 (the time search's symbol
cores named and tied to the ramps); then a5a2034 (docs). Hosts and
timing hygiene as in O3 and O4 (every timed binary built from this
repository's path into its own target directory, hashes compared,
rounds alternated, one benchmark at a time, throttle flags 0x50000
before and after every Raspberry Pi run). Decision rule as in the O3
fix round (under about 2% on the Cortex-A76 is noise; the simpler form
is kept unless memory or structure clearly improves).

**Slot time per commit** (`slotcost host,passes=1 host 200 5`, each
commit against the one before in two alternated rounds, both hosts;
worst / mean ms of the three-pass 40-signal slot; single-pass mean;
stage means):

| | Cortex-A76 three passes | single pass | subtraction / BP | x86-64 three passes | single pass | subtraction / BP |
|---|---|---|---|---|---|---|
| base (46fdf4d) | 217.9–218.1 / 201.1–201.2 | 32.2 | 122.5 / 40.0 | 118.5–122.6 / 109.7–110.9 | 18.7–19.0 | 64.5–65.3 / 21.4–21.5 |
| 1. subtractions without the report (f0bb4ab) | 201.9 / 187.0 (both rounds) | 32.6 | 107.5 / 40.9 | 112.5–112.8 / 103.7–104.2 | 18.7–18.8 | 58.6–58.8 / 21.3 |
| 2. messages in the edge layout (02ff55e) | 202.4 / 187.5 (both) | 32.5 | 108.1 / 40.7 | 114.7 / 104.5–105.1 | 18.7–18.8 | 59.1–59.8 / 21.2–21.4 |
| 3. tests (92970ba) | no library change | | | | | |
| 4. symbol cores (1b47a55) | 202.5–202.6 / 187.5 | 32.5 | 108.1 / 40.7 | 112.4–115.5 / 103.9–104.5 | 18.6–18.9 | 58.9–59.2 / 21.2–21.4 |

Base and 1b47a55 timed together at the end: A76 201.1–201.2 → 187.5 ms
(−6.8%), x86-64 110.2–110.3 → 103.9–104.5 (−5.5%). Feed and sync
unchanged (A76 31.3 / 4.7, x86-64 15.4 / 3.6). The single pass moved
32.2 → 32.5–32.6 ms on the A76 with step 1, whose code it does not run
(BP 40.0 → 40.7–40.9 in the three-pass slot as well): placement.

1. **The decoder's subtractions skip the report's sums** (f0bb4ab; item 1
   of the O5 plan in task_plan.md). `subtract`'s removal took, on every sample,
   the energy subtracted and the residue's six least-squares sums, only
   for `Subtraction::{energy, residue, residue_noise}`; the further
   passes discard the report. A crate-internal `subtract_unreported`
   runs the same subtraction with `Removal` told not to take them (nor
   to split its runs at segment ends, which only the residue's sums
   need). The public `subtract`, its report and the 24 anchors are
   unchanged; the new tests compare the two paths bit for bit over the
   hold, skip, cut and segment matrix and the anchors' cases. Without
   the loop-carried sums the removal loop vectorises on AArch64 (four
   samples per iteration in 44 instructions, against 58 per sample).
   Subtraction per three-pass slot A76 122.5 → 107.5 ms, x86-64
   64.5–65.3 → 58.6–58.8; slot A76 −7.0% (201.1–201.2 → 187.0 ms),
   x86-64 −5.8% (paired base 109.7–110.9 → 103.7–104.2 ms). Host image
   flash +220 B (both removal loops); frames and static RAM unchanged.
   - **First form rejected:** `const REPORT: bool` on `run` and the
     `Removal` methods. Two copies of `run` moved the reference loop
     (`blocks` with the fit closure) out of line: A76 201.2 → 199.8 ms
     (−0.7%; subtraction 122.5 → 121.0), x86-64 109.7–110.1 →
     113.6–114.4 (+4%; subtraction 64.5–64.8 → 68.5–69.0). The kept
     form holds the flag in `Removal` and branches once per run of
     samples, so `run` stays one function.
   - **The fit's `xx` and `n` (corrected in O6):** dead only where the
     fit closure is inlined into `run`, as in the default release build
     on AArch64 (no `x·x` product, no counter). With one codegen unit
     the closure stays out of line and the loop computes both, and
     loads and stores its sums and the correction every sample; in the
     Cortex-M33 image `Sums::add` is a call per sample. O6 gives the fit
     its own five sums, in locals.
2. **Belief-propagation messages kept in the edge layout** (02ff55e;
   item 2 of the O5 plan). The round gathered its inputs from the
   messages (rows by check, 83 × 7) into a 522-value array in degree
   order, worked there and scattered the results back. `BpScratch::msg`
   is now that array and the round works on it in place: no copy, no
   per-round clearing of 2 088 B and no scatter. One function,
   `check_edges`, walks the layout for the inputs, the sums into the
   totals (still in check order) and the min-sum rule. The pinned state
   hashes are unchanged (the test hashes the messages as rows by
   check). Time: A76 BP 40.9 → 40.7 ms, slot 187.0–187.1 → 187.5
   (+0.3%, under the rule); x86-64 BP 21.3–21.4 → 21.2–21.4, slot within
   the rounds' spread. **Kept for memory and structure:** `BpScratch`
   3 892 → 3 656 B and `CandidateScratch` 5 100 → 4 864 B (static RAM
   of the small and host images −236 B; host headroom 3 912 → 4 148 B),
   the round's thumbv8m frame 2 176 → 88 B (the decoder path's 2 496 →
   2 488), flash −36 / −48 B, and one layout instead of two. The decode
   command's `--profile` keys `candidate_scratch_bytes` and `peak_bytes`
   are 236 lower; nothing else in its output changes.
3. **Tests of the layout and the round** (92970ba; item 3 of the O5 plan).
   The pinned hash was the only guard against a mapping error. Now: the
   layout (every edge used once; the degree-6 checks in consecutive
   blocks of 6 in [0, 354), the degree-7 ones in blocks of 7 in
   [354, 522), both in check order; every variable in three checks), and
   the round against its first form (check by check over rows of the
   table) bit for bit over 20 000 random states (±0, subnormals, values
   up to ±1e6), sums and messages. Mutants: a scatter in degree order,
   and the degree-7 block placed first, each fail the new tests as well
   as the pin.
4. **The time search's symbol cores named** (1b47a55; item 4 of the O5
   plan). `CORE_START..CORE_END` is what the closure picking the cores
   uses, and the compile-time check is on that range at both ends
   (`RAMP_SAMPLES <= CORE_START && CORE_END + RAMP_SAMPLES <= T`; a core
   starting at 0 fails to compile). `symbol_correlations` states its
   other two assumptions where it makes them: `WAVEFORM_SAMPLES ==
   SYMBOL_COUNT * T` (compile time) and a generator at its first sample
   (debug assertion). Sizes unchanged.
5. **Docs** (a5a2034): ADR 0024's flash figures are `check.sh`'s
   current ones; the O4 notes' wording. The `at(c)` comment left open
   at the end of O4 went with step 2's rewrite.

**Memory and flash** (`check.sh --stack`, thumbv8m.main-none-eabihf):

| | 46fdf4d | f0bb4ab | 02ff55e | 92970ba, 1b47a55 |
|---|---|---|---|---|
| flash small / host / encoder | 85 544 / 93 928 / 35 536 B | 85 544 / 94 148 / 35 536 | 85 508 / 94 100 / 35 536 | same |
| static RAM small / host / encoder | 212 280 / 241 848 / 0 B | same | **212 044 / 241 612** / 0 | same |
| sum-product round frame | 2 176 B | 2 176 | **88** | 88 |
| decoder path frame (check program) | 2 496 B | 2 496 | 2 488 | 2 488 |
| `subtract` frame (with caller) / time-search closure | 4 600 / 2 800 B | same | same | same |

No `f64` helpers in any image.

**Belief propagation on the Cortex-M33** (item 6 of the O5 plan;
static, from the thumbv8m host image's disassembly of
`sum_product_round`; no Cortex-M run). `exp2_neg` and `log2_power` are
inlined, so the transcendental work is the tanh and atanh loops'
bodies (per value: tanh 86–88 instructions with two `vdiv`, atanh 50–52
with two, the same in every version). Per round, from loop bodies and
trip counts:

| | instructions (static) | tanh / atanh evaluations | `vdiv` | dynamic estimate | calls in the round |
|---|---|---|---|---|---|
| 872bd59 (83 × 8 lanes) | 342 | 664 / 664 | 2 656 | ≈ 128 000 | 83 `memclr` (rows), 83 `memcpy` (scatter) |
| 657543e (522 edges, by degree) | 415 | 522 / 522 | 2 088 | ≈ 109 000 | 1 `memclr` (2 088 B), 83 `memcpy` |
| 02ff55e (in place) | 373 | 522 / 522 | 2 088 | ≈ 100 000 | none |

So on the M33, 657543e removes 21% of the transcendental evaluations
and divides and about 15% of the round's instructions; 02ff55e a
further 8.3% of the dynamic estimate (against 657543e's: ≈ 109 000 →
100 000). The products cost about the same in all three (≈ 14 000: the
fixed-size ones are not unrolled at `opt-level = "s"`); gathers and
sums ≈ 8 000–11 500 each. Cycles are not measured.

### Where a three-pass slot's time goes on the Cortex-A76, at 1b47a55

The same sampler as in O4 (SIGPROF-style, not included), ported onto
1b47a55: 420 967 samples during the three-pass side of `slotcost
host,passes=1 host 200 5`, every one unwound through the interrupted
frame; that run's three-pass mean 191.4 ms against 187.5 unsampled
(+2.1%). Shares of the slot, and ms at 187.5 (O4 at 201.2 in
brackets):

| | share | ms per slot |
|---|---|---|
| **subtraction** | **53.7%** | **100.7** (114.5) |
| fit loop (reference cos/sin, least-squares sums, holding the reference) | 27.6% | 51.8 (48.7) |
| time search: symbol correlations and scores | 10.3% | 19.4 (19.4) |
| removal (amplitude interpolation, subtraction) | 7.8% | 14.7 (30.4) |
| generator, fit run (`fill_phase`, increments) | 4.3% | 8.0 (9.0) |
| time search: its generator runs | 3.6% | 6.8 (6.9) |
| frequency estimate, setup | < 0.1% | 0.1 |
| **belief propagation** | **26.2%** | **49.1** (48.9) |
| **feed (spectrogram, FFT)** | **15.6%** | **29.2** (29.4) |
| sync | 2.6% | 4.8 |
| soft bits | 1.5% | 2.8 |
| refine, SNR, other | 0.4% | 0.8 |

Leaves (share of the slot): the reference's cosine and sine
(`sin_turns`) 11.2% in the fit loop and 4.0% in the time search; the
fit loop's own body 11.0%; the removal's loop 6.0% plus its reference
reads 1.5%; `input` 1.8% (fit) and 1.8% (time search); belief
propagation's round 8.4% with `exp2_neg` 6.7%, `log2` 5.2%, the
products 1.2%; the FFT stages 5.9% and complex multiplies 3.5%; the
generator 8.1% in all. The removal halved (30.4 → 14.7 ms); the fit
loop's attribution moved up by 3 ms, as in O4 (inlining assigns the
shared `input` and `sin_turns` by call site), so the fit and removal
together went 79.1 → 66.5 ms.

With the report's sums gone, the bit-identical work is at its floor as
far as this round's targets go: the fit loop (six fixed-order
chains, interpolated table lookups), the time search's correlations,
belief propagation's pinned `exp2`/`log2` and the FFT. The rest needs
result-changing work.

### Measurements for the gated rounds (out of tree, not committed)

A copy of 1b47a55 with a counter in the time search and a `paired_bench`
command that decodes each slot of the `crowded`, `wide` and `fading`
sets (main block; 600 + 800 + 600 slots, host configuration, one
thread) through `decode_passes_staged`, timing each pass (its
subtractions before it included) and splitting the decode list at each
pass's first entry. Counts and distances are deterministic (the same on
both hosts); times below are the Cortex-A76's (x86-64 in brackets).

**(a) Cost per pass.** Mean per slot in which the pass ran (pass 3
ran in 73–77% of the 10-signal slots, 99–100% of the others and 92.3%
of all; counted in O6); "per decode" is the pass's time over its new
decodes (every new decode in passes 2–3 was a transmitted message:
none false):

| set, signals | pass 1: ms / decodes / ms per decode | pass 2 | pass 3 |
|---|---|---|---|
| crowded 10 | 36.0 / 8.78 / 4.1 | 52.2 / 1.13 / 46.2 | 29.8 / 0.09 / 343 |
| crowded 20 | 34.4 / 15.51 / 2.2 | 72.7 / 3.62 / 20.1 | 36.2 / 0.60 / 59.9 |
| crowded 40 | 32.4 / 24.07 / 1.35 | 97.9 / 10.45 / 9.4 | 57.0 / 2.96 / 19.3 |
| fading 10 | 36.2 / 8.13 / 4.5 | 50.2 / 1.12 / 44.6 | 29.6 / 0.07 / 414 |
| fading 20 | 34.8 / 14.39 / 2.4 | 69.3 / 3.21 / 21.6 | 35.1 / 0.43 / 81.7 |
| fading 40 | 33.1 / 21.77 / 1.52 | 91.2 / 8.76 / 10.4 | 52.0 / 2.12 / 24.6 |
| wide 10 | 36.2 / 7.89 / 4.6 | 49.5 / 1.00 / 49.5 | 29.3 / 0.03 / 856 |
| wide 20 | 34.8 / 13.95 / 2.5 | 68.0 / 2.98 / 22.8 | 34.4 / 0.45 / 76.4 |
| wide 30 | 33.7 / 18.61 / 1.8 | 81.9 / 5.77 / 14.2 | 42.8 / 1.10 / 38.9 |
| wide 40 | 32.9 / 22.07 / 1.49 | 92.0 / 8.96 / 10.3 | 52.6 / 2.19 / 24.0 |
| all 2 000 slots | 34.5 (19.8) / 15.52 / 2.2 (1.3) | 72.5 (40.3) / 4.70 / 15.4 (8.6) | 37.6 (20.9) / 1.00 / 37.6 (20.9) |

Pass 3 costs 37.6 ms per slot (over all 2 000 slots: 75.2 s for 1 998
decodes; 40.7 ms per slot in the 1 846 slots where it ran) for 1.00
decodes per slot (4.7% of the decodes, 26% of the multi-pass time on
the A76); on lightly loaded slots (10 signals) it finds 0.03–0.09
decodes per slot in which it ran, at 340–860 ms per decode. (Corrected
in O6: the all-slots row first gave the per-run figures, 40.7 ms and
1.08 decodes, as per-slot ones, and "28%". An earlier summary's
39.9 ms / 1.00 / ~27% is the unweighted mean of the set rows, which
are per run too.)

**(b) Where later passes find their decodes.** For each new decode of
pass 2 or 3, the frequency distance to the nearest decode of the pass
before (those subtracted just before it):

| pass | decodes | < 3.125 Hz | < 6.25 | < 12.5 | < 25 | < 50 | < 100 | < 200 | ≥ 200 |
|---|---|---|---|---|---|---|---|---|---|
| 2 | 9 402 | 741 | 738 | 1 533 | 2 983 | 3 387 | 19 | 1 | 0 |
| 3 | 1 998 | 149 | 155 | 310 | 621 | 757 | 5 | 1 | 0 |

Median 19.4 / 20.1 Hz, 99th percentile 47.4 / 47.9 Hz, largest 144 /
136 Hz; within 50 Hz 99.8% / 99.7%. For comparison, the part of the
100–3100 Hz band within 25 / 50 / 100 Hz of a decode of the previous
pass is, on average over slots, 25.4% / 44.8% / 66.6% for pass 2 and
8.4% / 15.8% / 28.1% for pass 3. So a later pass that searched only
within 50 Hz of the previous pass's decodes would have kept 99.8% of
its decodes while searching under half the band (pass 2) or a sixth
(pass 3). (Against any earlier pass, pass 3's distances are smaller
still: 79% within 25 Hz, against 62% for the previous pass alone.)

**(c) The time search's repeat step.** Over the 40 435 subtractions:
1 step in 40 379, 2 in 56 (0.14%), 3 in none; no repeat that found
nothing; no subtraction without an estimate. The repeat is a rare path;
its cost is negligible.

**Identity.** At f0bb4ab, 02ff55e and 1b47a55 (each the working tree of
that commit, built in this repository's path; 92970ba adds tests only):
unit, integration, doc, profile and no-std tests, the 24 subtraction
anchors and the exhaustive `_everywhere` tests (release lib 154 with
the ignored at f0bb4ab and 02ff55e, 157 from 92970ba); `paired_bench
run` with timings removed equal to the S3 fix round's records (`host`
38 900, every set and block; `host,passes=1` and `small` 34 800 each; 0
differing); the release `cat-ft8-decode`'s output on the 800 WAV slots
(`host` and `small`, `host,passes=1` and `small`; plain, `--raw` and
`--profile`, timings removed) byte-identical to b465b75's at f0bb4ab,
and from 02ff55e identical except the two memory keys of step 2 (each
of the 1 600 `--profile` lines per run differs only by −236 in
`candidate_scratch_bytes` and `peak_bytes`). At 1b47a55: fmt, clippy
`-D warnings` (workspace; cat-ft8 all targets and features; thumbv8m
library), workspace tests, profile and no-std tests, `check.sh` and
`--stack`, MSRV 1.75, docs, the bulk run. Raspberry Pi 5 at 1b47a55:
the release test suite with the ignored tests (lib 157, then 2 + 12 +
1 bulk + 7 + 2 + 5) passes; throttle flags 0x50000 before and after.
The release `cat-ft8-decode` at 1b47a55: sha256 in progress.md.

**Not measured:** any Cortex-M at run time (flash, frames, static RAM
and static instruction counts only).

## CPU per decoded signal, sixth round (O6, 2026-10-30)

From the open items at the end of round O5 (listed in task_plan.md,
O6). One code commit changes code (1d028a1), one
changes docs (73ebf5d); both identical in results to 3f16549 on every
target. Hosts, timing hygiene and the decision rule as in O5 (both
sides built from one source path into their own target directories,
hashes compared, two alternated rounds, throttle flags 0x50000 before
and after every Raspberry Pi run; under about 2% on the Cortex-A76 is
noise, and the simpler form is kept unless memory or structure clearly
improves).

**Slot time per step** (`slotcost host,passes=1 host 200 5`, each step
against the one before; worst / mean ms of the three-pass 40-signal
slot; single-pass mean; subtraction and BP means):

| | Cortex-A76 three passes | single pass | subtraction / BP | x86-64 three passes | single pass | subtraction / BP |
|---|---|---|---|---|---|---|
| base (3f16549) | 201.9 / 187.0 (both rounds) | 32.6 | 107.5 / 40.8 | 112.0–113.3 / 103.7–104.3 | 18.7–18.9 | 58.7–58.9 / 21.0–21.5 |
| 1. fit sums in locals (1d028a1) | 201.8 / 187.0 (both) | 32.6 | 107.5 / 40.8 | 111.9–112.1 / 103.7–103.8 | 18.5–18.7 | 59.1 / 20.9–21.1 |
| 2. padded sine table (rejected) | 204.9 / 189.5 (both; step 1 187.0) | 32.3–32.4 | 110.4 / 40.4 | 113.2–113.9 / 105.2–105.6 (step 1 104.5–104.8) | 18.7–18.8 | 60.1–60.4 / 21.2 |
| 3. docs (73ebf5d) | no code change | | | | | |

1. **The fit's sums and correction in locals** (1d028a1; the first
   O6 plan item). The fit loop updated its sums and the
   frequency correction through the closure's captured references and
   still took the report's `x·x` and sample count. Each block now
   copies the sums (`Fit`: five fields, `xc xs cc cs ss`), the
   correction and the ring size into locals and writes them back; the
   report's `Sums` is `Fit` plus `xx` and `n`, used only by the
   removal. **Time: unchanged on both hosts** (A76 187.0 → 187.0 ms,
   x86-64 within the rounds' spread). An earlier AArch64 listing was
   built with one codegen unit, where `blocks` and the fit closure stay
   out of line: there the loop loaded and stored the sums and the
   correction every sample and took `x·x` and the count (fixed: the
   fused loop has no state loads or stores and five sum products). The
   release build that is timed (default codegen units) inlines the
   closure into `run`, and its loops were already in registers without
   `x·x` or the count: the shipped AArch64 fit loops are the same
   instructions before and after (83 fused, 69 unfused, per sample). **Kept for
   the Cortex-M33**: in the thumbv8m host image (LTO, `opt-level =
   "s"`) `Sums::add` was an out-of-line call every sample (seven fields
   loaded and stored through memory, the counter incremented); now it
   is inlined: per-sample loop and call 115 + 23 → 110 instructions
   (fused) and 92 + 23 → 90 (unfused), static counts; `subtract` frame
   (with caller) 4 600 → 4 584 B; host image flash +108 B (94 100 →
   94 208); static RAM unchanged. No Cortex-M run.
2. **Padded sine table: rejected** (the second O6 plan item). The
   table held a copy of entry 1024 and `sin_turns` lost its end-point
   case; bit-identical at all 2³² phases on x86-64 and the A76
   (exhaustive check, 2.8 s / 7.0 s) and the encoder's output equal.
   But the A76 slot went 187.0 → 189.5 ms (+1.3%; subtraction 107.5 →
   110.4, +2.7%) in both rounds, and x86-64 subtraction +1%. Without
   the branch LLVM pairs the cosine and sine lookups into two NEON
   lanes: the fused loop shrinks (83 → 70 instructions) but fills with
   general-to-vector moves and lane shuffles (`fmov s, w`, `mov v.s[1],
   w`, `zip`, `bit`) that lengthen the loop's dependency chain. Under
   the 2% slot rule but the stage loss is reproducible, so the branch
   stays. (Patch kept out of tree.)
3. **Docs** (73ebf5d): `Removal::run_as` names the runtime flag
   (`Removal::report`); `decode_passes` links the `subtract` function.
   The O5 notes: (a)'s all-slots pass-3 row (37.6 ms / 1.00 / 37.6:
   the earlier row and the earlier correction were both per-run
   figures), the M33 step against its stated base, step 1's x86-64
   figure with its paired base, the fit's `xx`/`n` claim, and the name of the
   O5 plan's items.

**Memory and flash** (`check.sh --stack`, thumbv8m.main-none-eabihf) at
73ebf5d against 3f16549: flash small / host / encoder 85 508 / 94 208
(+108) / 35 536 B; static RAM 212 044 / 241 612 / 0 B (unchanged);
`subtract` frame with caller 4 600 → 4 584 B; time-search closure
2 800 B and decoder path 2 488 B unchanged. No `f64` helpers.

**Where a three-pass slot's time goes on the Cortex-A76, at 73ebf5d.**
The same sampler as in O5: 422 399 samples during the three-pass side
of `slotcost host,passes=1 host 200 5`, every one unwound through the
interrupted frame; that run's three-pass mean 192.0 ms against 187.0
unsampled (+2.7%). Shares of the slot, and ms at 187.0 (O5 at 187.5 in
brackets):

| | share | ms per slot |
|---|---|---|
| **subtraction** | **53.7%** | **100.4** (100.7) |
| fit loop and setup | 27.2% | 50.9 (51.9) |
| time search: symbol correlations and scores | 10.6% | 19.8 (19.4) |
| removal | 7.7% | 14.4 (14.7) |
| generator, fit run | 4.9% | 9.2 (8.0) |
| time search: its generator runs | 3.4% | 6.4 (6.8) |
| **belief propagation** | **26.3%** | **49.2** (49.1) |
| **feed (spectrogram, FFT)** | **15.5%** | **29.0** (29.2) |
| sync | 2.5% | 4.7 (4.8) |
| soft bits | 1.4% | 2.6 (2.8) |
| other | 0.4% | 0.7 (0.8) |

The shares are O5's within sampling noise, as expected: the A76 code of
the subtraction is the same instructions as at 1d028a1.

**Raspberry Pi 5 test suite at 73ebf5d:** the release test suite with
the ignored tests passes (lib 157, then 2 + 0 + 12 + 1 bulk + 7 + 2 +
5); throttle flags 0x50000 before and after.

### Measurements for the gated rounds (out of tree, not committed)

A copy of 23742aa (73ebf5d before an amend that only moved the fused
loop's `let ring = ring` copy into the block's locals: the same results
by construction) with three settings in the decoder: a gate W (a
later pass keeps only sync points whose base frequency is within W Hz of a decode of the pass before: the union of
those intervals, the same cap filled from inside it, nothing from
outside), a pass-3 minimum k (pass 3 runs only if pass 2 gave at least
k decodes) and the later-pass candidate cap L (now 50). A
`paired_bench` command decodes every slot of the `crowded`, `wide` and
`fading` sets (main block; 600 + 800 + 600 slots, host configuration,
one thread) under each setting in turn, slot by slot, timing the whole
three-pass decode, and compares the transmitted messages decoded with
the reference (the current decoder, run first and again last as a
timing control: 0.0–0.1 ms apart). Counts are deterministic and equal
on both hosts; no setting gave a false decode.

**(a) Candidates per pass.** Sync local maxima at or above the minimum
score, and those kept (mean per search; all 2 000 slots):

| pass | points found | kept (cap) | searches |
|---|---|---|---|
| 1 | 4 612 | 100 | every slot |
| 2 | 5 067 | 50 | every slot |
| 3 | 5 162 | 50 | 92.3% of slots |

The cap binds in every pass of every slot, by two orders of magnitude:
the minimum score passes almost every local maximum, and the cap alone
decides how many candidates are decoded. Inside a 50 Hz gate a later
search still finds 2 203 (pass 2) and 789 (pass 3) points.

**(b) The gate** (later passes search within W Hz of the previous
pass's decodes; total over the 2 000 slots, 42 433 messages decoded by
the reference):

| W | lost / gained | A76 ms per slot (saved) | x86-64 (saved) | pass 3 run |
|---|---|---|---|---|
| — (reference) | — | 144.2 | 76.2 | 92.3% |
| 25 | 4 668 / 2 | 137.3 (7.0) | 72.4 (3.8) | 83.8% |
| 50 | 39 / 0 | 144.2 (0.1) | 76.2 (0.0) | 92.2% |
| 75 | 6 / 1 | 144.3 (−0.0) | 76.3 (−0.1) | 92.3% |
| 100 | 3 / 1 | 144.3 (−0.0) | 76.3 (−0.1) | 92.3% |

Per set at W = 50, lost: crowded 1 / 0 / 4, fading 1 / 5 / 9, wide 4 /
3 / 4 / 8 (10 / 20 / 40 and 10 / 20 / 30 / 40 signals). The gate saves
nothing: with no refill the cap is still filled from inside the
intervals, so the same 50 candidates per pass are refined and decoded;
it only exchanges candidates far from the last decodes for weaker ones
near them, and loses decodes. At 25 Hz it removes about 41% of the later
passes' decodes (4 668 of the 11 400 that passes 2 and 3 found in O5
(b), where 36% lie 25–50 Hz away). The
sync-search saving a real gate could add (scoring only the intervals) is
at most the search's own cost: 1.56 ms (pass 2) and 1.57 ms (pass 3)
per 40-signal slot on the A76 (the `pass2_sync` and `pass3_sync`
means of the O6 slot timings, base and step 1 alike).
**Not proposed** as specified.

**(c) The pass-3 trigger** (k = 1 is the current behaviour: pass 3
already stops when pass 2 decodes nothing), alone and with W = 50:

| k | lost (alone) | A76 saved | x86-64 saved | pass 3 run | lost with W = 50 | A76 saved |
|---|---|---|---|---|---|---|
| 2 | 27 | 4.6 | 2.3 | 76.1% | 67 | 4.6 |
| 3 | 85 | 8.7 | 4.5 | 62.9% | 126 | 8.7 |
| 5 | 311 | 15.8 | 8.3 | 42.9% | 353 | 15.8 |
| 8 | 721 | 23.0 | 12.0 | 26.1% | 765 | 23.0 |

Per set (A76 ms saved per slot, lost), k = 3: crowded 19.8 / 6.1 / 0.1
ms (13 / 18 / 0 lost), fading 21.0 / 7.8 / 0.1 (11 / 16 / 0), wide
20.3 / 11.2 / 0.7 / 0.0 (4 / 22 / 1 / 0). The trigger saves on lightly
loaded slots (10–20 signals, where pass 2 finds few) and nothing at 40;
each step of k costs a few tenths of a percent of the decodes. Combined
with the gate it saves the same time and loses the gate's decodes as
well.

**(d) The later-pass cap** (not asked for; (a) pointed at it):

| cap L | lost | A76 ms per slot (saved) | x86-64 (saved) |
|---|---|---|---|
| 50 (now) | — | 144.2–144.3 | 76.4 |
| 40 | 1 | 139.3 (5.0, 3.5%) | 73.9 (2.5) |
| 30 | 1 | 134.3 (10.0, 6.9%) | 71.3 (5.1) |
| 25 | 2 | 131.8 (12.5, 8.7%) | 70.2 (6.3) |
| 20 | 3 | 129.3 (15.0, 10.4%) | 68.8 (7.6) |
| 15 | 16 | 126.6 (17.7, 12.3%) | 67.5 (8.9) |
| 10 | 196 | 123.0 (21.3, 14.8%) | 65.5 (10.9) |
| 20 with k = 2 | 30 | 126.0 (18.3) | 67.2 (9.3) |
| 20 with k = 3 | 88 | 122.9 (21.4) | 65.5 (10.9) |
| 25 with k = 2 | 29 | 128.3 (16.0) | 68.3 (8.1) |

At L = 20 the 3 lost decodes are all in fading 40; every other set
loses none, and the saving is even (A76 13.4–15.8 ms per slot from 10
to 40 signals): about 0.26 ms per later-pass candidate not decoded
(30 fewer in pass 2 and in the 92% of slots running pass 3).

**Proposal for the gated round.** In this order, each with its own
acceptance (the run sets with the paired gates, the 800 WAV slots, and
a separate generated block used once for acceptance):
1. Later-pass cap 50 → 20 (`HOST.later_candidates`): −10% of the A76's
   three-pass time for 0.007% of the decodes here. `SMALL` runs one
   pass, so its later-pass count is never used and the cap is moot
   there. The cap saves CPU only, no memory: the candidate buffer is
   `candidate_capacity()` = max(100, L) entries, pass 1's 100 either
   way. Re-check against the ceiling run, the `wide` low-SNR messages
   and the fresh block before adopting; 25 is the conservative choice
   (−8.7%, 2 lost). (Superseded by the O7 measurements below.)
2. Pass-3 trigger k = 2 on top (−4.6 ms more on the A76; 27 lost,
   mostly 10-signal slots), if the scheduler's budget wants it; k ≥ 3
   costs more decodes per ms than the cap.
3. Drop the frequency gate as proposed; revisit only as a way to skip
   scoring (a sync-stage saving), not candidates.

## CPU per decoded signal, seventh round (O7, 2026-10-30)

From the open items at the end of round O6 (listed in task_plan.md,
O7). Two code commits, both identical in results
to c472100: 5704d32 (two comments) and a437134 (a test). Hosts, timing
hygiene and the decision rule as in O6.

### Release build profile

The workspace sets no `[profile.release]`: 16 codegen units, thin local
LTO. Four profiles, each built from this repository's path at c472100
into its own clean target directory (`CARGO_PROFILE_RELEASE_*` set in the
environment), timed with `slotcost host,passes=1 host 200 5` in two
alternated rounds on each host (A76: throttle flags 0x50000 before and
after every run). Worst / mean ms of the three-pass 40-signal slot, both
rounds; single-pass mean; sizes of the release `cat-ft8-decode`
(unstripped) and clean build times (x86-64, 16 cores, crates already
downloaded):

| profile | A76 three passes | A76 one pass | x86-64 three passes | x86-64 one pass | CLI bytes aarch64 / x86-64 | clean CLI build | CLI rebuild after a `cat-ft8` change |
|---|---|---|---|---|---|---|---|
| default | 202.5–202.6 / 187.6 | 32.5 | 112.0–112.8 / 104.0 | 18.6–18.7 | 710 976 / 703 808 | 2.2 s | 1.7 s |
| `codegen-units = 1` | 202.4 / 187.6 | 32.5–32.6 | 111.0–114.1 / 102.8–103.3 | 18.6–18.8 | 695 552 / 687 000 | 2.8 s | 3.0 s |
| `lto = "fat"` | 200.4 / 185.7 (−1.0%) | 32.4 | 113.1–115.4 / 104.8–105.3 | 18.8–18.9 | 605 400 / 617 864 | 5.3 s | 5.2 s |
| both | 202.8 / 187.8 | 32.4 | 110.9–112.4 / 103.0–103.2 | 18.6–18.8 | 605 480 / 607 424 | 5.9 s | 5.9 s |

Every profile is bit-identical: `paired_bench run` records equal to the
S3 fix round's (`host` 38 900, `host,passes=1` and `small` 34 800 each; 0
differing) and the 800-WAV `cat-ft8-decode` output (plain, `--raw`,
`--profile` without timings; `host`, `small`, `host,passes=1`) equal to
O5's, for all four. **No profile wins by 2% on the A76** (fat LTO −1.0%,
the rest within 0.1%; x86-64 within 1.3% either way, fat LTO +1.25%), so
none is set:
the workspace keeps cargo's defaults, which every other crate in it
also builds with, and fat LTO would more than double the decoder's
rebuild time for under the noise rule. `target-cpu=cortex-a76` was
measured bit-identical and no faster with the default profile
(2026-10-28) and is not rechecked. The embedded image is its own
workspace with its own profile (`opt-level = "s"`, LTO, one codegen
unit), so it is unaffected either way (`check.sh --stack` at the final
commit below). Timings from here on stay on the default profile.

### Text and test

- 5704d32: `DecodeConfig::HOST`'s doc said further passes decode with 50
  candidates "as many as 200". Measured on the current decoder (three
  passes, `crowded`, `wide`, `fading`, main block): 42 433 with 50,
  42 437 with 200 (crowded +1, fading +3); on two fresh blocks +2 and
  +6. The doc now gives the figure and marks it as one to re-measure
  when the count changes. The fit's per-block locals comment gives the
  reason (a closure's captured `&mut` stays in registers only if the
  closure is inlined) instead of a code-generation outcome.
- a437134: `projection_pinned`. The report's projection was guarded only
  by whole-subtraction pins, and a re-association of its cross term
  (`xc·(xs·cs)`) passed them all: over a real segment `cs` is nearly 0,
  so the cross term never reaches the result's last bit (checked over
  96 subtractions, segments 960–2 640, drift on and off: identical
  bits). The test feeds `Sums` 64 random reference sequences and pins
  every projection and noise term; it fails with either square term or
  the cross term re-associated, the noise divisor changed or `x·x`
  scaled.
- Notes: the O5 and O6 text names its sources plainly; the O5 (a) all-slots row's x86-64 pass-3 figure is
  20.9 ms per slot (per run 22.65 ms × 92.3%); the O6 measurement
  worktree is named as 23742aa; the 25 Hz gate's loss is ~41% of the
  later passes' decodes, not a third; the sync-search cost is backed by
  the slot timings; the O6 profile and Pi suite are filled in.

### Measurements for the gated later-pass cap (out of tree, not committed)

A copy of c472100 (default profile) with the proportional rule in the
decoder (`L_n = clamp(m · d_{n−1}, floor, 25)`, `d_{n−1}` the previous
pass's new decodes; with `d = 0` the pass does not run, as now) and a
`paired_bench` command that decodes every slot of the `crowded`, `wide`
and `fading` sets (600 + 800 + 600 slots, host configuration, one thread)
under each setting in turn, slot by slot, timing the whole three-pass
decode, and counting the transmitted messages lost or gained against
the first setting (the current cap, 50; run again last as a timing
control: 0.0–0.1 ms apart on both hosts). Three blocks: `main`, and two
generated blocks never used before, `cap1` and `cap2` (each slot's
seed from its set, block name and index, as for every block; 2 000
slots and about 42 450 decoded messages each). The separate block kept
for acceptance was not used. Counts are deterministic and equal on both
hosts; no setting gave a false decode.

**(a) Rank of the later passes' decodes.** For each decode of pass 2
and pass 3 at the current cap, its candidate's rank in that pass's
sync-score order (1 = best), by load; cumulative share at each rank,
all three blocks (6 000 slots):

| pass, signals | decodes | ≤ 10 | ≤ 15 | ≤ 20 | ≤ 25 | ≤ 30 | ≤ 40 | deepest |
|---|---|---|---|---|---|---|---|---|
| 2, 10 | 1 843 | 99.95% | 100% | 100% | 100% | 100% | 100% | 11 |
| 2, 20 | 6 011 | 99.78% | 99.87% | 99.90% | 99.92% | 99.95% | 99.97% | 49 |
| 2, 30 (wide only) | 3 543 | 98.22% | 99.75% | 99.92% | 99.97% | 99.97% | 100% | 36 |
| 2, 40 | 17 031 | 84.33% | 98.12% | 99.71% | 99.85% | 99.93% | 99.98% | 50 |
| 2, all | 28 428 | 90.34% | 98.81% | 99.79% | 99.89% | 99.94% | 99.98% | 50 |
| 3, 10 | 103 | 100% | 100% | 100% | 100% | 100% | 100% | 3 |
| 3, 20 | 772 | 99.74% | 99.74% | 99.74% | 100% | 100% | 100% | 25 |
| 3, 30 (wide only) | 680 | 99.85% | 100% | 100% | 100% | 100% | 100% | 15 |
| 3, 40 | 4 309 | 99.05% | 99.61% | 99.72% | 99.84% | 99.88% | 99.98% | 50 |
| 3, all | 5 864 | 99.25% | 99.68% | 99.76% | 99.88% | 99.91% | 99.98% | 50 |

Below 40 signals nearly every later-pass decode comes from the top 10
candidates; at 40 signals pass 2 needs about 15 to reach 98% and 20 to
reach 99.7%. The tail is thin but long (a decode at rank 49 or 50 at
20 and at 40 signals; at 30 the deepest is 36). A decode ranked beyond the cap in pass 2 is not
necessarily lost: pass 3 searches the residual again. The 59 pass-2
decodes ranked beyond 20 here are exactly what a cap of 20 loses with
two passes; with three, pass 3 recovers all but 21 ((b)).

**(b) The S3 sweep and O6 (d) agree.** The S3 sweep's figures for 20
later candidates (crowded −3, wide −6, fading −12 against 50) are its
**two-pass** column ("+1"): with only one later pass nothing recovers
what its cap drops. Its three-pass column ("+2") at 20 against 50 is
crowded 13 439 → 13 439, wide 16 999 → 16 999, fading 11 995 → 11 992:
3 lost, all fading, exactly O6 (d)'s 3 (the S3 sweep's decoder and the
current one give the same three-pass counts on the main block). The "−6" for fading was against 200, not 50. Re-run on
the current decoder, decodes lost against the 50 cap (two passes:
against two passes at 50):

| block | three passes, L = 20 / 25 / 30 | two passes, L = 20 / 25 / 30 | 200 over 50, three passes |
|---|---|---|---|
| main | crowded 0 / 0 / 0, wide 0 / 0 / 0, fading 3 / 2 / 1 | crowded 3 / 2 / 1, wide 6 / 2 / 2, fading 12 / 7 / 2 | +4 |
| cap1 | 1 / 0 / 0, 4 / 1 / 0, 2 / 1 / 1 | 0 / 0 / 0, 10 / 6 / 3, 9 / 4 / 2 | +2 |
| cap2 | 0 / 0 / 0, 0 / 0 / 0, 11 / 6 / 3 | 0 / 0 / 0, 3 / 2 / 1, 16 / 9 / 5 | +6 |
| all | 21 / 10 / 5 | 59 / 32 / 16 | +12 |

The two-pass figures reproduce the S3 sweep's exactly on the main block
(12 713 / 12 710, 16 246 / 16 240, 11 476 / 11 464). The fresh blocks
lose more than the main block at 20 (7 and 11 against 3), almost all in
the 40-signal `fading` and `wide` slots (cap2 fading 40: 10 of its 11).
So the main block understated the loss at 20, and 25 halves it on the
fresh blocks (7 → 2, 11 → 6; main 3 → 2).

**(c) The proportional rule** against the fixed caps, three blocks
(127 336 decoded messages at the current cap; A76 ms per slot saved
against its 50 runs, which took 144.3 / 144.0 / 144.0 ms per slot; x86-64 76.2 / 76.7 / 76.3; `pM:F` is m = M,
floor F, maximum 25):

| setting | lost main / fresh 1 / fresh 2 (all) | false | later-pass candidates p2 / p3 per slot | A76 ms saved per slot: main / fresh 1 / fresh 2 | x86-64: main / fresh 1 / fresh 2 |
|---|---|---|---|---|---|
| f30 | 1 / 1 / 3 (5) | 0 | 30.0 / 27.4 | 10.1 (7.0%) / 10.0 (7.0%) / 10.0 (7.0%) | 5.0 (6.5%) / 5.0 (6.6%) / 5.0 (6.5%) |
| f25 | 2 / 2 / 6 (10) | 0 | 25.0 / 22.8 | 12.6 (8.8%) / 12.5 (8.7%) / 12.5 (8.7%) | 6.3 (8.2%) / 6.3 (8.1%) / 6.3 (8.3%) |
| f20 | 3 / 7 / 11 (21) | 0 | 20.0 / 18.2 | 15.1 (10.5%) / 15.0 (10.4%) / 15.0 (10.4%) | 7.6 (9.9%) / 7.5 (9.8%) / 7.6 (9.9%) |
| p1:0 | 70 / 73 / 65 (208) | 0 | 15.4 / 4.7 | 20.0 (13.8%) / 19.7 (13.7%) / 19.8 (13.7%) | 9.9 (13.0%) / 10.0 (13.0%) / 9.9 (13.0%) |
| p1:5 | 59 / 64 / 54 (177) | 0 | 15.4 / 6.0 | 19.6 (13.6%) / 19.4 (13.5%) / 19.4 (13.5%) | 9.8 (12.8%) / 9.8 (12.8%) / 9.7 (12.7%) |
| p1:10 | 31 / 30 / 29 (90) | 0 | 15.9 / 9.3 | 18.6 (12.9%) / 18.4 (12.8%) / 18.5 (12.8%) | 9.2 (12.1%) / 9.3 (12.2%) / 9.2 (12.1%) |
| p2:0 | 11 / 13 / 16 (40) | 0 | 22.4 / 9.4 | 16.9 (11.7%) / 16.6 (11.6%) / 16.7 (11.6%) | 8.4 (11.0%) / 8.4 (10.9%) / 8.3 (10.9%) |
| p2:5 | 9 / 12 / 15 (36) | 0 | 22.4 / 10.0 | 16.8 (11.6%) / 16.5 (11.5%) / 16.6 (11.5%) | 8.3 (10.9%) / 8.3 (10.8%) / 8.3 (10.8%) |
| p2:10 | 8 / 11 / 15 (34) | 0 | 22.4 / 12.0 | 16.2 (11.2%) / 16.0 (11.1%) / 16.0 (11.1%) | 8.1 (10.6%) / 8.0 (10.4%) / 8.0 (10.5%) |
| p3:0 | 5 / 6 / 9 (20) | 0 | 24.6 / 12.8 | 15.5 (10.7%) / 15.2 (10.6%) / 15.2 (10.6%) | 7.7 (10.1%) / 7.7 (10.0%) / 7.6 (10.0%) |
| p3:5 | 5 / 6 / 8 (19) | 0 | 24.6 / 13.1 | 15.4 (10.7%) / 15.1 (10.5%) / 15.2 (10.5%) | 7.6 (10.0%) / 7.6 (10.0%) / 7.6 (9.9%) |
| p3:10 | 5 / 6 / 8 (19) | 0 | 24.6 / 14.5 | 15.0 (10.4%) / 14.8 (10.3%) / 14.8 (10.3%) | 7.4 (9.8%) / 7.4 (9.7%) / 7.4 (9.7%) |
| p4:0 | 2 / 5 / 7 (14) | 0 | 25.0 / 14.8 | 14.8 (10.3%) / 14.6 (10.1%) / 14.6 (10.1%) | 7.4 (9.7%) / 7.3 (9.6%) / 7.3 (9.6%) |
| p4:5 | 2 / 5 / 7 (14) | 0 | 25.0 / 15.0 | 14.8 (10.2%) / 14.6 (10.1%) / 14.6 (10.1%) | 7.3 (9.5%) / 7.3 (9.5%) / 7.2 (9.5%) |
| p4:10 | 2 / 5 / 7 (14) | 0 | 25.0 / 16.0 | 14.5 (10.0%) / 14.3 (9.9%) / 14.3 (9.9%) | 7.2 (9.5%) / 7.2 (9.4%) / 7.1 (9.4%) |

Per set (lost over the three blocks; ms per slot saved, A76 main block
and x86-64 mean of the three blocks):

| setting | crowded lost (3 blocks) | wide | fading | A76 saved, main: crowded / wide / fading | x86-64 saved, 3 blocks mean: crowded / wide / fading |
|---|---|---|---|---|---|
| f30 | 0 | 0 | 5 | 10.1 / 10.1 / 10.1 | 5.0 / 5.1 / 4.9 |
| f25 | 0 | 1 | 9 | 12.6 / 12.7 / 12.7 | 6.2 / 6.3 / 6.3 |
| f20 | 1 | 4 | 16 | 15.1 / 15.2 / 15.2 | 7.6 / 7.6 / 7.5 |
| p1:0 | 12 | 53 | 143 | 19.6 / 20.0 / 20.3 | 9.8 / 10.0 / 10.0 |
| p1:5 | 7 | 45 | 125 | 19.3 / 19.7 / 19.9 | 9.6 / 9.9 / 9.8 |
| p1:10 | 4 | 21 | 65 | 18.4 / 18.6 / 18.8 | 9.1 / 9.3 / 9.3 |
| p2:0 | 2 | 11 | 27 | 16.6 / 16.9 / 17.2 | 8.2 / 8.4 / 8.4 |
| p2:5 | 2 | 8 | 26 | 16.4 / 16.8 / 17.1 | 8.1 / 8.3 / 8.3 |
| p2:10 | 2 | 7 | 25 | 15.9 / 16.3 / 16.4 | 7.9 / 8.1 / 8.0 |
| p3:0 | 2 | 4 | 14 | 15.2 / 15.4 / 15.7 | 7.6 / 7.8 / 7.7 |
| p3:5 | 2 | 3 | 14 | 15.2 / 15.4 / 15.6 | 7.6 / 7.7 / 7.6 |
| p3:10 | 2 | 3 | 14 | 14.8 / 15.0 / 15.2 | 7.4 / 7.4 / 7.4 |
| p4:0 | 1 | 2 | 11 | 14.8 / 14.7 / 15.1 | 7.4 / 7.3 / 7.4 |
| p4:5 | 1 | 2 | 11 | 14.7 / 14.7 / 15.0 | 7.3 / 7.2 / 7.3 |
| p4:10 | 1 | 2 | 11 | 14.4 / 14.4 / 14.7 | 7.2 / 7.1 / 7.2 |

The rule saves time by giving pass 3 fewer candidates (mean 15 instead of
23 at m = 4) and, for m ≤ 2, pass 2 too; the decodes it loses are the
later passes' deep candidates on crowded slots, where `d` is large
anyway, and lightly loaded slots where pass 2 found one or two. No
setting meets the adoption rule (beat the fixed 25 by more than 2% of
the slot at equal or lower loss): m = 4 is within 1.5 points of 25 and
loses 14 against 10; m = 3 loses 19–20; m ≤ 2 loses 34 or more. The
floor helps only at m = 1–2 and never enough.

**Proposal for the gated round (replaces O6's).**
1. `HOST.later_candidates` 50 → **25**: A76 three-pass slot −8.8% on the
   main block, 8.7% over the three (12.5–12.6 ms per slot on each),
   x86-64 −8.2%, for 10 of 127 336 decodes (0.008%), all 40-signal
   (and one 20-signal) `fading`/`wide` slots; no false decode. 20 saves
   1.7 points more but loses 21 (twice as many, 11 on one fresh block);
   30 loses 5 for −7.0%. Nothing else changes: `SMALL` runs one pass,
   so the later-pass count is never used there; no memory changes (the
   candidate buffer is `candidate_capacity()` = max(100, L) = 100).
   Acceptance as before (the run sets with the paired gates, the 800 WAV
   slots, and the separate generated block used once).
2. No proportional rule and no pass-3 trigger: the rule does not beat
   the fixed cap at equal loss on any block, and it subsumes the
   trigger (the trigger at k = 2 cost 27 decodes for 4.6 ms in O6).
3. No frequency gate (O6 (b)).

**Checks at a437134:** fmt, clippy `-D warnings` (workspace; cat-ft8 all
targets and features; thumbv8m library), workspace tests, profile and
no-std tests, the release tests with the ignored ones (lib 158), the
bulk run, MSRV 1.75, docs, `check.sh --stack` (flash small / host /
encoder 85 508 / 94 208 / 35 536 B, static RAM 212 044 / 241 612 / 0 B:
unchanged from O6). Raspberry Pi 5: the release test suite with the
ignored tests passes (lib 158, then 2 + 0 + 12 + 1 bulk + 7 + 2 + 5);
throttle flags 0x50000 before and after. The release `cat-ft8-decode`
at a437134 gives the same 800-WAV output as O5's. Its sha256 is in
progress.md.

**Not measured:** any Cortex-M at run time; the cap's effect on the
ceiling run and on the `wide` set's lowest-SNR messages separately
(left to the gated round's acceptance).

## Later-pass cap 50 → 25, eighth round (O8, gated, 2026-10-30)

The O7 proposal's first item, as one code commit on ce09c5d:
`HOST.later_candidates` 50 → 25 with its doc and the pins that move.

### Gate criteria, fixed before the acceptance run

Written down before any acceptance record was made; the verdict is the
table below, filled in before it is interpreted.

- **Sets.** The `crowded`, `wide` and `fading` sets on the `fresh` block
  (the block `paired_bench` keeps for confirming a chosen value; the
  cap was measured on `main`, `cap1` and `cap2` only), decoded once at
  the candidate commit for the verdict. Also the existing paired gates:
  `run host` of every set on both blocks (`sweep`, `crowded`, `edges`,
  `wide`, `fading`; `empty` on `main`) compared with `compare` against
  the S3 fix round's `host` records (cap 50; every identity run since,
  ce09c5d's included, equals them) and against the single pass; and the
  800 WAV slots.
- **False decodes:** 0, absolute — in every pass of every record, the
  ceiling run and the WAV output (a decode that is not a transmitted
  message, or for the WAV slots a line not in the cap-50 output that
  cannot be shown to be a transmitted message).
- **Pass 1 byte-identical:** `host,passes=1` and `small` records equal
  to the S3 fix round's (34 800 each, timings removed); in every `host`
  record the pass-1 decodes and pass-1 report equal to cap 50's; WAV
  output with `host,passes=1` and `small` byte-identical, and with the
  default the pass-1 lines of every slot unchanged. Only passes 2 and 3
  may differ.
- **Lost** (a transmitted message decoded at 50 and not at 25, per
  message): each must be a pass-2 or pass-3 decode whose candidate
  ranked beyond 25 at cap 50 (rank = candidate index + 1 in that pass's
  sync order); every one logged. At most 8 on the `fresh` block; above
  8 the change stops (re-measure, no retuning).
- **Gained** (decoded at 25, not at 50): each listed and explained.
- **800 WAV slots:** every difference listed and explained.
- **Ceiling run and low-SNR messages.** The `ceiling` mode on the
  `fresh` block (`crowded`, `wide`, `fading`; its oracle passes use the
  later-pass count), per message against cap 50; and the `wide` set's
  messages below −16 dB (the part of its range below the other sets'),
  per message in the `run` records. Loss rate (lost ÷ decoded at 50) at
  most the set's rate over `main`/`cap1`/`cap2` in O7: crowded 0 of
  40 452, wide 1 of 50 996, fading 9 of 35 888 (so: crowded and `wide`,
  low-SNR included, lose none; `fading` at most 3 of ~12 000).
- **Speed:** the A76 three-pass slot at least 7% faster than ce09c5d.
  The figure is the one O7 proposed the cap on (−8.8%): the mean
  three-pass decode time per slot over every slot of `crowded`, `wide`
  and `fading` (here the `fresh` block, one thread: the `run` records'
  stage times, the process pinned to one core). Same hygiene as O7: both
  binaries built from one source path into a target
  directory per commit, hashes checked on the Pi, rounds alternated,
  `vcgencmd get_throttled` before and after (0x50000 is this supply's
  baseline). The usual `slotcost` 40-signal crowded figure is reported
  too, not gated.

### Verdict table (filled in before interpretation)

Candidate 0f0341c (on ce09c5d, local branch, not merged): the constant,
its doc, `max_decodes` 200 → 150 and the moved pins. Records kept
locally (not committed).

| criterion | value | result |
|---|---|---|
| sets run | `fresh` crowded / wide / fading once for the verdict; every set both blocks; 800 WAV; ceiling on `fresh` | done |
| false decodes | 0 in 38 900 `host` records (every pass), 0 in the ceiling run (every pass, alone), 0 new WAV lines | pass |
| pass 1 byte-identical | `host,passes=1` and `small`: 34 800 records each, 0 differing; pass 1 of all 38 900 `host` records equal; WAV `host,passes=1` and `small` byte-identical; WAV default decode lines identical | pass |
| lost on `fresh`, bound 8 | 3 (crowded 0, wide 1, fading 2) | pass |
| each lost ranked beyond 25 at 50 | 2 of 3: ranks 27 and 30 (pass 3); `fading` fresh slot 597's lost message was a pass-3 decode at **rank 4** | **fail** |
| gains | 0 (every set, both blocks; WAV; ceiling) | pass (nothing to explain) |
| 800 WAV differences | 621 `host --profile` summary lines, only passes 2–3 work counters; no decode line | pass (explained below) |
| ceiling run loss rate | crowded 0 / 13 575, wide 0 / 17 286, fading 2 / 12 140 (1.6 × 10⁻⁴ ≤ 2.5 × 10⁻⁴) | pass |
| `wide` below −16 dB | 0 lost of 1 869 decoded (`fresh`); 0 of 1 843 (`main`) | pass |
| paired gates (`compare`) | against cap 50 and against the single pass: every set, both blocks, PASS; sweep 50% points unmoved (−19.122 / −19.078 dB, shift +0.000 [+0.000, +0.000]) | pass |
| A76 three-pass slot ≥ 7% faster | 144.26 → 131.71 ms per slot (−8.70%), both alternated rounds identical to 0.01 ms; 0x50000 before and after every run | pass |

**Verdict: the cap is not adopted.** One criterion fails as written,
so 0f0341c stays on its own local branch and `HOST` keeps 50.

### Interpretation

**Lost on the `fresh` block (3), each logged** (rank = candidate index
+ 1 in that pass at cap 50):
- `fading` slot 490 (40 signals) `C9I XU3HB R+09`, −12.6 dB, 2 547 Hz,
  fade 0.53 Hz depth 0.63: pass 3, rank 30.
- `fading` slot 597 (40 signals) `V7K H7PNB CH73`, −10.2 dB, 1 962 Hz,
  fade 0.93 Hz depth 0.81: pass 3, **rank 4**.
- `wide` slot 692 (40 signals) `I3ELL KA5AE MP96`, −14.7 dB, 306 Hz:
  pass 3, rank 27.
On `main` (not part of the verdict): `fading` slots 415 (pass 3, rank
50) and 555 (pass 3, rank 29), as O7 counted.

Slot 597 is a knock-on loss, not a ranking one. At cap 50 pass 2
decoded `CJ2JAY LF2K +05` (1 933 Hz) from rank 35; at 25 pass 2 cannot
see it and pass 3 recovers it (rank 23), so it is subtracted one pass
later. `V7K H7PNB` lies 29 Hz above it and at 50 decoded only in pass
3, after `CJ2JAY` had been removed; at 25 pass 3 searches a residual
that still holds `CJ2JAY`, and there is no fourth pass. The ceiling run
shows the same slot (its oracle passes use the later count): it is the
only slot losing messages there, and it loses both (`CJ2JAY` dropped by
the oracle's pass 2 and not recovered, `V7K H7PNB` behind it). The
cap's cost therefore includes messages ranked well inside 25 that sit
behind a deep pass-2 decode; the "ranked beyond 25" criterion does not
allow for that, and it is not relaxed after the fact.

Everything else matches O7's measurement: 3 lost of 42 529 on
`fresh` (expected about 3.3), all in 40-signal `fading`/`wide` slots,
no gain, no false decode.

**800 WAV slots.** Decode output identical in every slot and mode
(plain, `--raw`, the decode lines of `--profile`; `host` default,
`host,passes=1`, `small`). The 621 differing lines are the `host
--profile` summary of every slot where pass 2 runs (225 with two
passes, 396 with three); the other 179 run one pass. In each, only
the work of passes 2 and 3 changes: `pass2/3_candidates_kept` 50 → 25
(and the slot totals), `candidates_dropped` +25, `not_converged` and
`bp_rounds` down by the 25 candidates no longer tried, and
in 11 slots the CRC-failure, all-zero or duplicate counts of a
candidate ranked 26–50 that is no longer tried (e.g. `s0001`: pass 2 kept 50 → 25,
`bp_rounds` 1 500 → 750).

**Timing** (Raspberry Pi 5, release builds of ce09c5d and 0f0341c,
SHA-256 checked on the Pi, `paired_bench run host … crowded,wide,fading
fresh` under `taskset -c 3`, rounds old/new then new/old; ms per slot,
mean of the records' stage times):

| | crowded | wide | fading | all 2 000 slots |
|---|---|---|---|---|
| ce09c5d (50), rounds 1 / 2 | 147.07 / 147.06 | 144.86 / 144.85 | 140.65 / 140.66 | 144.26 / 144.26 |
| 0f0341c (25), rounds 1 / 2 | 134.56 / 134.56 | 132.23 / 132.22 | 128.18 / 128.17 | 131.71 / 131.71 |
| saved | 12.5 (8.5%) | 12.6 (8.7%) | 12.5 (8.9%) | 12.55 (8.70%) |

O7 measured the same on its three blocks (12.5–12.6 ms, 8.7–8.8%).
`slotcost host,passes=1 host 200 5` (the 40-signal crowded slots of
`main`, two alternated rounds): three passes 187.0 / 187.0 → 173.9 /
173.9 ms (−7.0%), worst 201.8–201.9 → 188.8; one pass 32.6 both. The
A76 records equal the x86-64 ones (2 000 of 2 000 for each commit,
timings removed) and each other across rounds.

**Records.** Every multi-pass `host` record differs (23 984 of 38 900)
through the same report counts; the 14 916 equal ones run one pass
(`empty`, and `sweep`/`edges` slots with nothing decoded). `compare`
moves 7–38 misses per crowded/wide/fading set from "not converged" to
"peak dropped by the candidate limit" (messages missed either way, now
never tried; the totals differ only by the logged losses), and counts
fewer CRC failures and a lower CRC-based expected false-decode rate in
every multi-pass set.

### Where a three-pass slot's time goes with the cap, at 0f0341c

For choosing the next round (the `i16` residual or vector sine/cosine),
profiled on the candidate even though it is not adopted: with 50 the
O6 table stands. The same sampler as in O5 and O6, built from a copy of
0f0341c (it is not in the repository): 381 543 samples during the
three-pass side of `slotcost host,passes=1 host 200 5`, every one
unwound through the interrupted frame; that run's three-pass mean 178.7
ms against 173.9 unsampled (+2.8%). The O6 classification, rerun on
O6's samples, gives O6's table exactly. Shares of the slot and ms at
173.9 (O6 at 187.0 in brackets):

| | share | ms per slot |
|---|---|---|
| **subtraction** | **59.6%** | **103.6** (100.4) |
| fit loop and setup | 30.0% | 52.2 (50.9) |
| time search: symbol correlations and scores | 11.7% | 20.4 (19.8) |
| removal | 8.6% | 14.9 (14.4) |
| generator, fit run | 5.4% | 9.3 (9.2) |
| time search: its generator runs | 3.8% | 6.7 (6.4) |
| **belief propagation** | **18.8%** | **32.7** (49.2) |
| **feed (spectrogram, FFT)** | **17.2%** | **29.9** (29.0) |
| sync | 2.9% | 5.0 (4.7) |
| soft bits | 1.1% | 1.9 (2.6) |
| other | 0.4% | 0.8 (0.7) |

The cap's saving is belief-propagation work (later passes try 25
candidates instead of 50: 49.2 → 32.7 ms here); the subtraction does
the same work as with 50 (its 3 ms more here is within the two
profiles' attribution) and is now three-fifths of the slot. The fit loop (with its reference sine and cosine) and the
time search's correlations are 72.6 ms of it: the vector sine/cosine
round addresses the larger share of time; the `i16` residual addresses
memory (720 000 → 360 000 B) and the removal's and fit's sample reads.
Inlined leaves were not resolved in this build, so the split of
`sin_turns` from the fit's own arithmetic (O5: 11.2% and 11.0% of the
slot) was not re-measured.

**Raspberry Pi 5 test suite at 1c1c8a8:** the release test suite with
the ignored tests passes (lib 159, then 2 + 0 + 12 + 1 bulk + 7 + 2 +
5); throttle flags 0x50000 before and after.

**Not measured:** any Cortex-M at run time; off-air audio; slots
beyond 40 signals.

## Later-pass cap 50 → 25, re-gate on a new block (O8b, 2026-10-30)

The eighth round's verdict stands as recorded: under its criterion
("each lost message ranked beyond 25 at cap 50") the `fresh` block
fails. The mechanism found there (a message ranked well inside 25 lost
behind a pass-2 neighbour that the cap pushes into pass 3) is inherent
to any later-pass cap below 50; O7's 10 losses on `main`/`cap1`/`cap2`
were counted per message, so they already include such losses and the
cost estimate is unchanged. The criterion is revised to name that
mechanism and the candidate is tested once more, on a block never
used, with no change to 0f0341c.

### Gate criteria, fixed before any run on the new block

- **Block `accept2`.** Never used: no record, log or ceiling dump named
  `accept2` exists (checked before this commit: the build cache and run
  directories, the repository including its history (`git log --all
  -S accept2` empty), the scratch directories and the Raspberry Pi 5's
  home and temporary directories — no file and no file content). Sets
  `crowded`, `wide`, `fading` decoded with `run host` at ce09c5d (cap
  50) and at 0f0341c (cap 25), plus the `ceiling` mode on the three
  sets (cap 25 and `host,later=50`, one binary). One attempt: a fail
  drops the cap; there is no third block.
- **False decodes:** 0 in every pass of every record and in the ceiling
  run.
- **Pass 1 byte-identical** in every record (pass-1 decodes and the
  pass-1 report).
- **Lost:** at most 8 on `accept2`, and at most 13 pooled with
  `fresh`'s 3 (at O7's rate the pooled mean is about 6.6; 14 or more
  has a probability of about 1%).
- **Every loss explained by script**, as one of:
  (a) a pass-2/3 decode at cap 50 whose candidate ranked beyond 25 in
  that pass; or
  (b) a knock-on: a message M decoded at cap 50 in an earlier pass than
  the lost message L, from rank beyond 25; at cap 25 M decoded in a
  later pass or not at all; |f_M − f_L| ≤ 50 Hz (transmitted
  frequencies); and L's outcome at cap 25 is not `dropped`. M may
  itself be such a knock-on, the chain ending at a message of the
  first kind.
  One unexplained loss fails the gate.
- **Gains** explained by the same rules with the roles of lost and
  gained swapped (the gained message's earlier-pass neighbour M
  displaced by the cap as above, within 50 Hz, the gained message not
  `dropped` at cap 50); otherwise the gate fails.
- **Ceiling run and `wide` below −16 dB:** per-set loss rate at most
  the O7 three-block rates (crowded 0 of 40 452, wide 1 of 50 996,
  fading 9 of 35 888).
- **Speed and the 800 WAV slots** carry over from the eighth round only
  if 0f0341c's tree is unchanged (its tree hash checked).
- **Classifier check first:** on the `fresh` records the classifier
  must give 2 of kind (a) and 1 of kind (b) (a check of the script, not
  part of the verdict). Done before this commit: `wide` slot 692 (pass
  3, rank 27) and `fading` slot 490 (pass 3, rank 30) are (a);
  `fading` slot 597 `V7K H7PNB` (pass 3, rank 4; `not_converged` at
  25) is (b) behind `CJ2JAY LF2K` (−28.2 Hz; at 50 pass 2 rank 35, at
  25 pass 3 rank 23); no gain.
- **If it passes:** 0f0341c joins the branch; `HOST`'s doc names the
  loss as being in 40-signal `fading`/`wide` slots, knock-on losses
  behind a deep pass-2 neighbour included, and says
  `with_later_candidates` restores the old behaviour.

### Verdict table (filled in before interpretation)

One attempt on `accept2`: `run host crowded,wide,fading accept2` with
clean release builds of ce09c5d (cap 50) and 0f0341c (cap 25), each
from its own worktree and target directory; `ceiling` with `host` and
`host,later=50` from one build of 0f0341c with an out-of-tree
per-message dump (the same patch and binary, by SHA-256, as the eighth
round's ceiling run). Classifier and comparison scripts out of tree;
records kept locally (not committed).

| criterion | value | result |
|---|---|---|
| block never used | no file or content named `accept2` anywhere checked (before the gate commit) | pass |
| classifier check on `fresh` | 2 of kind (a), 1 of kind (b), 0 unexplained, 0 gains | as expected |
| false decodes | 0 in 2 000 records at each cap (every pass); 0 in the ceiling run at both counts (single pass, alone, oracle +1, oracle +2, every size) | pass |
| pass 1 byte-identical | 2 000 of 2 000 records: pass-1 decodes and pass-1 report equal; ceiling single-pass outcome equal for every message | pass |
| lost on `accept2`, bound 8 | 5 (crowded 0, wide 0, fading 5) | pass |
| lost pooled with `fresh`, bound 13 | 5 + 3 = 8 | pass |
| every loss explained | 4 of kind (a), 1 of kind (b), 0 unexplained | pass |
| gains | 0 | pass (nothing to explain) |
| `wide` below −16 dB | 0 lost of 1 814 decoded | pass |
| ceiling: crowded, rate ≤ 0 | 0 of 13 559 | pass |
| ceiling: wide, rate ≤ 1 / 50 996 | 0 of 17 172 | pass |
| ceiling: fading, rate ≤ 9 / 35 888 (2.51 × 10⁻⁴) | **5 of 12 168 (4.11 × 10⁻⁴)**; at most 3 allowed | **fail** |
| 0f0341c unchanged | commit and tree (a3acbf93bd18231df067186ebffe83886b46b583) as gated in the eighth round, so its A76 −8.70% and 800-WAV results stand | carried over |

**Verdict: fail. The later-pass cap is dropped.** 0f0341c stays on its
local branch unmerged; `HOST` keeps 50 further-pass candidates. Under
the fixed rule there is no third block.

### Interpretation

**Lost on `accept2` (run records), each classified** (rank =
candidate index + 1 in that pass at cap 50):
- `fading` slot 75 (10 signals) `ZC8P J2IS RR73`, −11.4 dB, 310 Hz:
  pass 2, rank 40 — (a).
- `fading` slot 411 (40 signals) `KX2QX BN9F HD62`, −9.3 dB, 1 523 Hz:
  pass 2, rank 42 — (a).
- `fading` slot 461 (40 signals) `H7HSV BH5QPV +04`, −14.5 dB, 679 Hz:
  pass 2, rank 38 — (a).
- `fading` slot 471 (40 signals) `KS9L PR0IZ RR73`, −14.1 dB, 2 195 Hz:
  pass 3, rank 4, `not_converged` at 25 — (b), behind `LW0D VA0V 73`
  (−34.3 Hz; at 50 pass 2 rank 28, at 25 pass 3 rank 12). The same
  mechanism as `fresh` slot 597.
- `fading` slot 555 (40 signals) `RC2GKO KX5U +03`, −13.4 dB, 1 397 Hz:
  pass 2, rank 47 — (a).

The run-record criteria all pass; one loss is in a 10-signal slot,
the first below 20 signals. The ceiling run (its oracle passes remove
every decoded message with its true waveform and search with the later
count) loses the same four messages of slots 75, 411, 471 and 555 plus
one more, `fading` slot 507 `AF1CKV SS3XY 73` (−12.8 dB, 40 signals,
no sync peak in the single pass, oracle pass 3 at 50, nothing at 25;
no other message of that slot changes pass), against at most 3 the
O7 rate allows at this size. It is one criterion and it fails as
written; 5 against an expected 3.1 is not by itself evidence that
`accept2` is unusual, and the rule set before the run does not allow
weighing it.

**O7's 10 losses, classified with the same script** (both commits
rerun on `main`, `cap1`, `cap2`; 127 336 → 127 326 decoded, as O7
counted): all 10 are of kind (a) (pass 2 or 3, ranks 27–50); none is a
knock-on. The knock-on loss has been seen twice, once per new block
(`fresh` 597, `accept2` 471), both at pass-3 rank 4 behind a pass-2
neighbour of rank 28–35 within 35 Hz.

What is kept: the measurement that 25 further-pass candidates take
8.7% off a three-pass A76 slot; the per-message classifier; and, for a
later attempt at the same saving, the knock-on mechanism (a cap that
moves a deep pass-2 decode into pass 3 leaves its neighbours without
a pass that searches the residual without it).

## Vector sine and cosine in the subtraction, ninth round (O9, gated, 2026-10-30)

The plan is task_plan.md, "O9". Developed on the branch `o9-vsin` from
bafed37; measured on x86-64 and the Raspberry Pi 5's
Cortex-A76 with the hygiene of O5–O8 (every timed binary built from one
source path into its own target directory per commit, SHA-256 checked on
the Pi, rounds alternated, one benchmark at a time, `vcgencmd
get_throttled` 0x50000 before and after every Pi run). **Adopted: 44d01ba
(degree 5/4) passed the gate below on `accept3` and is merged.**

### Design

1. **Sine and cosine by polynomial** (`decode::math::cos_sin_turns`). The
   `u32` phase is reduced in integer arithmetic: `u = phase + 2²⁹`, the
   quarter `k = u >> 30`, the remainder `r = (u & (2³⁰ − 1)) − 2²⁹` in
   [−2²⁹, 2²⁹) (exact), converted once to `f32` and scaled exactly to
   turns (|t| ≤ ⅛). An odd sine `t·(S1 + S3·t² + S5·t⁴)` and an even
   cosine `C0 + C2·t² + C4·t⁴` (minimax for absolute error on |t| ≤ ⅛,
   coefficients rounded to `f32`, Horner, no FMA) are both taken; `k`
   swaps them by a mask and sets the signs by XOR of bit 31. No branch
   and no table, so a loop over samples maps onto 4-lane vectors. The
   encoder (`sin_turns`, `cos_turns`, `Synth`) keeps the quarter-wave
   table; the time search's 15 rotations per trial (not per sample) keep
   it too.
2. **Loops split so that each one vectorises, nothing scalar inside.** A
   first form computed four samples per iteration with the sums in the
   same loop (SLP vectorisation of `[f32; 4]` lanes): the AArch64 code
   filled with lane shuffles and general-to-vector moves (the
   time-search loop 91 instructions per 4 samples with 21 GPR↔SIMD
   transfers; the fit loop 113–174 with 16–28; `input`'s clamp done
   lane by lane with branches), the O6 padded-table failure again. The
   kept form, per block of 240 samples:
   - the frequency correction added to the generator's phases in place,
     four lanes of the exact quadratic phase at a time (`u64` lanes
     stepping by 4·step + 6·accel, then 16·accel: the same integers as
     `Correction::next`, tested), and `Correction::skip` in closed form;
   - a map loop: the reference `w·cos`, `w·sin` (fit, removal's second
     run) or the products `input(x)·cos`, `input(x)·sin` (time search),
     stored as pairs in the held ring or the scratch (the time search's
     block of products goes through the subtraction's scratch, idle at
     that point: no stack growth). On AArch64 (degree 7/6): 40
     instructions per 4 samples for the reference, 59 for the products
     (the input clamp included), `scvtf`/`fmul`/`fadd`/`bsl`/`st2`, no
     general-to-vector move but constants; x86-64 likewise (`cvtdq2ps`);
   - the sums from the stored pairs in four lanes: the fit's five
     (`Fit4`, 27 instructions per 4 samples on AArch64) and the time
     search's two (9 per 4 samples).
3. **Fixed-order sums.** Waveform sample j goes to lane j % 4 (blocks
   start at multiples of 240), each lane in sample order (single samples
   before and after the whole lanes of a block go to their lane by
   index), and the lanes are added `(l0 + l1) + (l2 + l3)` at the end of
   the segment (fit) or block (time search). The order is set by the
   sample index alone, so it is the same on every target, for every
   scratch size and whether the reference is held or rerun (the
   existing tests of both pass). Sine sums are taken positive and their
   sign turned once (exact: negation commutes with rounding). The held
   ring is rounded down to a multiple of 4 pairs so that 4 samples from
   a multiple of 4 never wrap: for speed only, the sums' order does not
   depend on it. Portable Rust only: no `unsafe`, no `std::arch`,
   `f32`, MSRV 1.75 (`next_multiple_of`), `no_std`.

### Degree

Largest absolute error against `libm` in `f64`, over **every** `u32`
phase (exhaustive test, ignored by default; f32 evaluation as compiled),
and what the subtraction leaves of a clean signal (400 single signals of
four messages, 200–2 800 Hz, −0.5…+1.0 s, amplitude 0.1, no noise;
residual energy over the signal's, power mean / median / worst; with the
decoder's estimates, and with start and frequency imposed):

| reference | error (all phases) | left, estimated | left, start and frequency imposed | A76 three-pass slot | x86-64 |
|---|---|---|---|---|---|
| quarter-wave table (bafed37) | 3.5 × 10⁻⁷ | −103.00 / −104.73 / −94.60 dB | −103.01 / −105.48 / −94.66 dB | 187.0 ms | 104.8–105.0 |
| degree 7 / 6 (ee7c0ce) | 1.58 × 10⁻⁷ | −103.05 / −104.79 / −94.61 | −103.06 / −105.55 / −94.66 | 155.5 | 87.4–87.6 |
| **degree 5 / 4 (d4e669e)** | **1.01 × 10⁻⁵** (sine 6.5 × 10⁻⁷) | −100.04 / −100.84 / −94.03 | −100.05 / −101.12 / −94.07 | **151.1** | **85.0** |
| degree 3 / 2 (not built) | 1.9 × 10⁻³ (grid) | — | — | — | — |

Degree 5/4 is chosen: its error leaves −100 dB of a signal, 47 dB below
the −53 dB a start between samples leaves (S2) and far below the noise of
any slot, and it is 2.8% faster on the A76 than 7/6 (4.4 ms per slot,
both rounds), above the 2% rule. Degree 3/2 (error ~2 × 10⁻³, about
−57 dB) would approach the timing residue and was not pursued. The
cosine's constant is the minimax one (0.999 990 05), so at the quarter
turns only the zero part is exact (tested). Effect of the two degrees on
decodes: below.

### Slot time per step

`slotcost host,passes=1 host 200 5` (the 200 40-signal crowded slots of
`main`, three passes against one), five binaries round-robin in two
rounds, the order reversed in the second; worst / mean ms of the
three-pass slot, both rounds; subtraction and BP means:

| step | A76 three passes | A76 subtraction | x86-64 three passes | x86-64 subtraction |
|---|---|---|---|---|
| base (bafed37) | 201.8–201.9 / 187.0 | 107.5 | 114.4–114.6 / 104.8–105.0 | 59.3–59.5 |
| 1. correction skip in closed form (a5c4a6c; identical results) | 200.8–200.9 / 186.1 (−0.5%) | 106.7 | 116.9–117.3 / 107.0–107.1 (+2.0%) | 62.1–62.2 |
| 2. fit and removal by polynomial, fit sums in lanes (a1119ed) | 174.8 / 163.0 (−12.4%) | 84.0 | 97.3–99.7 / 91.0–91.4 (−14.8%) | 46.2 |
| 3. time search by polynomial, in lanes (ee7c0ce) | 165.8–165.9 / 155.5 (−4.6%) | 76.3 | 93.5–94.3 / 87.4–87.6 (−4.1%) | 42.0–42.1 |
| 4. degree 5/4 (d4e669e) | 160.8–160.9 / 151.1 (−2.8%) | 72.1 | 91.3–92.0 / 85.0 (−2.8%) | 40.1 |
| **base → d4e669e** | **−19.2%** (mean), worst −20.3% | **−33%** | **−19.0%** | **−32%** |

BP, feed and sync are unchanged within 0.5 ms; the single pass is
unchanged (A76 32.3–32.5 ms, x86-64 18.6–19.0). Step 1 alone is −0.5% on
the A76 and +2% on x86-64 (subtraction +2.7 ms there, reproducible in two
rounds of an earlier, interrupted pairwise run as well): the closed
form itself costs nothing per sample, so this is code placement in
`run`; it is kept because steps 2–3 need a closed-form skip after the
four-lane phases.
d4e669e's release binaries are byte-identical to the measurement build
of degree 5/4 (ec72aa0, the same code with the coefficients inline):
SHA-256 equal on both targets, so its measurements are d4e669e's.

**Every slot of `crowded`, `wide`, `fading` (`main`, 2 000 slots, the
measure of O8's gate):** `run host` on the A76 pinned to one core
(`taskset -c 3`), bafed37 and d4e669e, rounds old/new then new/old; mean
three-pass decode time per slot from the records' stage times: bafed37
144.51 / 144.51 ms, d4e669e 123.20 / 123.21 ms: **−14.7%** (21.3 ms per
slot). The A76 records equal the x86-64 records of the same commit
(2 000 of 2 000, timings removed) and each other across rounds.

### Sizes (`check.sh --stack`, thumbv8m.main-none-eabihf)

| | bafed37 | ee7c0ce | d4e669e |
|---|---|---|---|
| flash small / host / encoder | 85 508 / 94 208 / 35 536 B | 85 508 / 98 044 / 35 536 | 85 508 / **97 948** / 35 536 |
| static RAM small / host / encoder | 212 044 / 241 612 / 0 B | same | same |
| `subtract` frame (with caller) / run closure | 4 584 / 2 800 B | 4 856 / 2 872 | 4 856 / 2 872 |
| decoder path frame | 2 488 B | 2 488 | 2 488 |

Host image +3 740 B of flash (the split loops), +272 B of stack in the
subtraction; the small and encoder-only images unchanged (the small
configuration runs one pass and does not link the subtraction). No
`f64` helper in any image; **no fused multiply-add** (`vfma`, `vfms`,
`vfnma`, `vfnms`) in any thumbv8m image at either commit (the M33 code
uses `vmla`, which rounds the product and the sum separately). thumbv7em
builds.

### Cortex-M33 instruction counts (static, host image, `opt-level = "s"`)

Per sample, from the loop bodies (no Cortex-M run, no cycles):

| | bafed37 (table) | d4e669e |
|---|---|---|
| fit, reference held (fused) | 110 (one loop) | ≈ 127: correction ≈ 24, reference map 45, sums ≈ 58 |
| fit, not held | 90 | ≈ 127 (the same three loops) |
| removal's second run (reference only) | 77 | ≈ 69 (correction ≈ 24, map 45) |
| time search | 79 | ≈ 78: products 55, lane sums ≈ 23 |

**On the M33 the fit is about 15% (fused) to 40% (not held) more
instructions per sample than the table; the time search is the same and
the second run 10% fewer.** The polynomial is not the cost (45
instructions per sample for sine, cosine and the envelope product
together): at `opt-level = "s"` the four-lane loops are not
unrolled, so the lanes' sums and the correction's `u64` lanes go through
memory every sample. A table path under `cfg` for Cortex-M would give
different residuals there and break the cross-target identity, so there
is one path; the subtraction runs only in the host configuration, which
is not the Cortex-M image's. If an M33 host image ever matters, the
lane loops are the place to look (cycles unmeasured).

### Effect on decodes (out of tree; main, cap1, cap2 only)

`paired_bench run` on x86-64 (all threads; records are deterministic):
`host` on `crowded`, `wide`, `fading` of `main`, `cap1` and `cap2`
(6 000 slots, 127 336 transmitted messages decoded by bafed37), `host`
on `sweep`, `edges`, `empty` of `main`, and `host,passes=1` and `small`
on `sweep`, `crowded`, `edges`, `empty` of `main`. Compared per message
with bafed37 (an out-of-tree script; lost = decoded by bafed37 only):

| | lost | gained | false | pass 1 | decodes moving pass | records differing (timings removed) |
|---|---|---|---|---|---|---|
| degree 7/6 (ee7c0ce) | 0 | 0 | 0 | identical | 0 | 1 724 of 24 600 `host` |
| **degree 5/4 (d4e669e)** | **1** | **0** | **0** | **identical** | 0 | 4 890 of 24 600 `host` |

Per set (both commits): `crowded` 40 452, `wide` 50 996, `fading`
35 888 decoded over the three blocks (127 336), by pass 93 044 / 28 428 /
5 864 (`crowded` 29 088 / 9 227 / 2 137, `wide` 37 424 / 11 368 /
2 204, `fading` 26 532 / 7 833 / 1 523; d4e669e: 5 863 in pass 3);
`sweep` 8 764, `edges` 1 228, `empty` 0, unchanged; `wide` below
−16 dB: 1 788 / 1 887 / 1 843 decoded per block, none lost or gained.
`host,passes=1` and `small`: 19 200 records each, all equal to
bafed37's. What differs in the `host` records (ee7c0ce):
later passes' sync point counts (1 448 pass reports), the reported SNR,
time, frequency and score of later-pass decodes (730 / 421 / 394 / 203),
BP rounds of 4 decodes, one candidate index; the slot reports' counts.

**The one loss** (degree 5/4): `fading` `cap2` slot 464 (40 signals)
`CQ K4TWQ JQ39`, −12.0 dB, 1 552.5 Hz, fade 0.09 Hz depth 0.61: at
bafed37 a pass-3 decode from the candidate ranked 4 with 26 hard errors
in 29 belief-propagation rounds (the limit is 30); at d4e669e the same
message's outcome is `not_converged` (a candidate on it tried, BP did not
converge). A marginal decode at the edge of the round limit, tipped by a
residual that differs in its last bits; nothing ranked differently.
Expected under no systematic effect: losses and gains of this kind
equally likely; here 1 and 0.

`paired_bench compare` of these records against bafed37's (degree 5/4):
PASS on every set of every block; `sweep`'s 50% point unmoved
(−19.122 dB both, shift +0.000 [+0.000, +0.000]); CRC failures and the
CRC-based expected false-decode rate unchanged in every set. The loss
classifier of the gate below, run on these records: the one loss is
kind (m), nothing unexplained (degree 7/6: nothing to classify).

**800 WAV slots** (`cat-ft8-decode`, plain, `--raw` and `--profile`
without timings): `host,passes=1` and `small` output byte-identical to
bafed37's. With `host` (three passes): in every one of the 4 800 runs
the same messages decoded in the same passes; 605 lines differ: 344
`--profile` summaries (later-pass counters) and 261 decode lines of
later-pass decodes in their score (255), frequency (12), SNR (9), time
(6), hard errors and rounds (6), candidate (3); 6 lines are two decodes
of one slot in the other order. (ee7c0ce: 175 lines, the same kinds.)

**Encoder:** `src/waveform`, `src/message`, `lib.rs` and the encode CLI
are untouched (empty diff from bafed37); `cat-ft8-encode` output for 45
cases (5 messages × 3 frequencies × 3 time offsets) byte-identical; the
golden waveform hashes pass; the encoder-only image is the same size.

**Identity across targets:** the polynomial's coefficients and a sweep
hash of its outputs are pinned (`cos_sin_turns_pinned`); the subtraction
anchors and the multi-pass golden hashes re-pinned, checked on x86-64
and (release suite) the A76. The A76's run records of `main`
(`crowded`, `wide`, `fading`) equal x86-64's for bafed37 and d4e669e.

### Pins re-pinned (each change explained)

- `math::tests::cos_sin_turns_pinned`: new (coefficient bits, sweep
  hash 0xc2ee_b714_7593_8caf).
- `subtract::tests::deterministic_and_pinned` and
  `anchored_to_the_unheld_subtraction` (3 + 24 hashes): the residual's
  bits change with the reference's last bits and the sums' order; the
  held and rerun paths still give the same hashes (both tests check);
  re-pinned at a1119ed, ee7c0ce and d4e669e. The anchors' doc says so.
- `tests/decode.rs` `MULTI_GOLDEN` (4 hashes): decodes per slot
  unchanged (`MULTI_COUNTS` as before), residual bits changed; re-pinned
  at the same three commits.
- The tests of the closed-form skip, the four-lane correction and
  in-place application against `Correction::next` (random states,
  wrapping), and of `lane_bounds`, are new; nothing else moved.

### Gate (fixed before any `accept3` record)

Written and committed before any record of block `accept3` exists. Two
candidates against the base bafed37, both decoded on `accept3` in one
attempt, with no retune and no second block:

- **P = 44d01ba** (primary): the branch at degree 5/4 (a5c4a6c, a1119ed,
  ee7c0ce, 0c6f3bf, d4e669e, 4e99bc0) plus a library test of the time
  search's lanes at unaligned starts, a debug assertion on the
  closed-form skip's bound and comment fixes. Its release
  `cat-ft8-decode` and `paired_bench` are byte-identical to d4e669e's
  (SHA-256 equal on x86-64 and AArch64), so d4e669e's measurements above
  are P's.
- **ee7c0ce** (fallback): degree 7/6, as it is.

**Block `accept3`, never used.** No record, log or dump named `accept3`
and no file content `accept3` (records, logs, text) in the build caches,
the run and scratch directories or this repository (the only commit
mentioning it, ba03a1a, is planning text), nor in the Raspberry Pi 5's
bench and temporary directories: checked 2026-10-30 before this commit,
and checked again on both machines immediately before the run.

**Merge rule.** P merges if every criterion below passes for P. Else
ee7c0ce merges if every criterion passes for ee7c0ce with the speed
bounds lowered to 13% (`slotcost`) and 10% (records). Else nothing
merges. Applied mechanically; no criterion is weighed against another.

**Runs.** `paired_bench run host … crowded,wide,fading accept3`,
`run host,passes=1 …` and `run small …` (the same sets) for bafed37, P
and ee7c0ce on x86-64 (all threads; records are deterministic) and
`run host` on the A76 (`taskset -c 3`, two rounds, order reversed in the
second); `ceiling host accept3 SET` for SET in `crowded`, `wide`,
`fading`; the 800 WAV slots; `slotcost host,passes=1 host 200 5` on the
A76 and x86-64 (bafed37, P, ee7c0ce round-robin, two rounds, order
reversed); the embedded checks; the encoder comparison. Every timed
binary built from one source path into its own target directory per
commit, SHA-256 checked on the Pi, one benchmark at a time, `vcgencmd
get_throttled` before and after every Pi run (0x50000 is this supply's
baseline).

Criteria, each for each candidate against bafed37:

- **False decodes: 0**, absolute, in every pass of every record, in the
  ceiling runs and in the WAV output.
- **Pass 1 byte-identical:** the `host,passes=1` and `small` records
  equal to bafed37's (timings removed); in every `host` record the
  pass-1 decodes and the pass-1 report equal; the WAV output with
  `host,passes=1` and `small` byte-identical.
- **Ceiling byte-identical:** `ceiling host accept3 SET` output for
  `crowded`, `wide`, `fading` equal to bafed37's (the elapsed seconds on
  its 50%-point line removed). The mode is single-pass by construction
  (its oracle passes decode the slot rebuilt without the messages found
  so far, and never call the subtraction), so any difference fails.
- **Lost at most 3** on `accept3` (all three sets together), reported
  per set. On `main`, `cap1`, `cap2` degree 5/4 lost 1 of 127 336; the
  upper 95% bound on that rate gives about 1.58 losses per block of this
  size, and P(≥ 4 | 1.58) ≈ 7.6%.
- **`wide` below −16 dB: at most 1 lost.**
- **Every lost, gained and pass-moved message classified** by the
  out-of-tree classifier `classify.py`, SHA-256
  `d1e131a08aff8369e3751e90a186945b9d9e902a52c6256b47149f9f4591898e`
  (its synthetic tests, `classify_test.py`, SHA-256
  `8fd7d039e5f876c81bc1e4af4a11d8807a63a462a5e24547598d5670a84a7b18`),
  as one of:
  (m) *marginal*: a lost or gained message whose decoding side took ≥ 20
  BP rounds or had ≥ 20 hard errors, its outcome on the other side
  `not_converged` or `crc`; a pass-moved message whose earlier-pass
  decode took ≥ 20 rounds or had ≥ 20 hard errors;
  (r) *ranking*: a lost or gained pass-2/3 decode whose outcome on the
  other side is `dropped` or `no_peak`; a message moved between passes
  2 and 3 (the records hold no per-pass outcome);
  (k) *knock-on*: another changed message of the slot, transmitted
  within 50 Hz, in an earlier pass, that itself resolves to (m), (r) or
  (k) by a chain that does not pass through the message again (the
  chain ends at (m) or (r)).
  One unexplained message fails the gate. Every pass move is printed.
  (r) takes any later-pass loss to `dropped` or `no_peak`, and at
  bafed37 1 239 of the 34 292 later-pass decodes on `main`/`cap1`/`cap2`
  meet the (m) threshold, so the classification catches only a
  mechanism outside these kinds: the loss bounds above carry the
  verdict. The classifier was checked first on the `main`/`cap1`/`cap2`
  records (degree 5/4: the one loss comes out (m); degree 7/6: nothing
  changed; `host,passes=1` and `small`: nothing changed) and on its
  synthetic tests (a knock-on chain ending at (m); two knock-ons that
  could only explain each other, unexplained; a chain ending at an
  unexplained message, unexplained; pass moves); that check is not part
  of the verdict.
- **800 WAV slots:** the CLI output has no transmitted messages or
  outcomes, so it is not classified: in every one of the 800 slots, with
  `host`, `host,passes=1` and `small` (plain, `--raw`, `--profile`),
  the same messages decoded in the same passes as bafed37; any message
  or pass that differs fails. Other differing lines (score, SNR, time,
  frequency, hard errors, rounds, candidate, order, `--profile`
  counters) are listed by kind, not judged.
- **Speed (A76):** the three-pass `slotcost` mean at least **15%** below
  bafed37's, and the mean three-pass decode time per slot of the
  `accept3` A76 run records (the records' stage times, both rounds) at
  least **12%** below; for the fallback 13% and 10%. **x86-64:**
  `slotcost` three-pass mean not slower than bafed37's.
- **Identity:** the A76's `accept3` run records equal x86-64's for every
  commit (timings removed); the release test suite with the ignored
  tests passes on the A76; thumbv8m.main-none-eabihf and thumbv7em build;
  `check.sh --stack` passes; no `f64` helper and no fused multiply-add
  (`vfma*`, `vfms*`, `vfnm*`) in any thumbv8m image.
- **Encoder byte-identical:** no change under `src/waveform`,
  `src/message`, `lib.rs` or the encode CLI against bafed37; the golden
  waveform hashes pass; `cat-ft8-encode` output for the 45 cases above
  byte-identical; the encoder-only image's size unchanged.

**If a candidate merges:** its commits go onto `feat/cat-ft8`, with an
ADR note recording the Cortex-M33 static regression of the fit (above)
as known and unmeasured at run time; then local CI, the Raspberry Pi 5
release suite and a fresh A76 profile. **If neither passes:** nothing
merges, and the result is recorded.

### Acceptance run on `accept3` (2026-10-30)

One attempt, as fixed above. Records, ceilings, WAV and x86-64 timing on
x86-64; A76 records and `slotcost` on the Raspberry Pi 5 (SHA-256 of
every binary checked there; `get_throttled` 0x50000 before and after
every run). Scripts and records out of tree.

| criterion | P = 44d01ba (degree 5/4) | result | ee7c0ce (degree 7/6) | result |
|---|---|---|---|---|
| block never used | no file or content named `accept3` on the host or the Pi, before the gate commit and again immediately before the run | pass | (same check) | pass |
| false decodes | 0 in 2 000 `host`, 2 000 `host,passes=1`, 2 000 `small` records (every pass); 0 in the ceiling runs; no WAV message outside bafed37's | pass | the same | pass |
| pass 1 byte-identical | `host,passes=1` and `small` records 2 000 / 2 000 equal; pass-1 decodes and report equal in 2 000 / 2 000 `host` records; WAV `host,passes=1` (2 400 runs) and `small` (2 400) byte-identical | pass | the same | pass |
| ceiling byte-identical | `crowded`, `wide`, `fading` output equal (timing removed) | pass | equal | pass |
| lost ≤ 3 | 0 (`crowded` 0, `wide` 0, `fading` 0) of 42 325 | pass | 0 of 42 325 | pass |
| `wide` below −16 dB, lost ≤ 1 | 0 of 1 824 | pass | 0 of 1 824 | pass |
| every changed message classified | 0 lost, 1 gained (k), 1 moved (m); 0 unexplained | pass | nothing changed | pass |
| 800 WAV slots, same messages in the same passes | 7 200 of 7 200 runs; 605 lines differ, all in 506 `host` runs (kinds below) | pass | 7 200 of 7 200; 175 lines in 164 `host` runs | pass |
| A76 `slotcost` three-pass mean (bound 15%; fallback 13%) | 187.0 / 187.1 → 151.1 / 151.0 ms: **−19.2%** | pass | 155.5 / 155.5: −16.9% | pass |
| A76 `accept3` records, mean per slot (bound 12%; fallback 10%) | 144.15 / 144.16 → 122.88 / 122.87 ms: **−14.8%** | pass | 125.57 / 125.58: −12.9% | pass |
| x86-64 `slotcost` not slower | 104.8 / 104.8 → 84.6 / 84.9 ms (−19.1%) | pass | 86.6 / 86.6 (−17.4%) | pass |
| A76 records == x86-64 | 2 000 / 2 000, both rounds (bafed37 too) | pass | 2 000 / 2 000, both rounds | pass |
| A76 release suite (ignored tests included) | lib 165 (the new lane test included), then 2 + 0 + 12 + 1 bulk + 7 + 2 + 5 | pass | lib 164, then the same | pass |
| thumbv8m / thumbv7em, `check.sh --stack`, no `f64` helper, no FMA | pass; host image 97 948 B flash; 0 `vfma`/`vfms`/`vfnma`/`vfnms` in the three images (`vmla` 143 in host) | pass | pass; 98 044 B; 0 fused (`vmla` 151) | pass |
| encoder byte-identical | no source change; 45 / 45 `cat-ft8-encode` outputs identical; encoder-only image 35 536 B | pass | the same | pass |

**Verdict: P passes every criterion and merges** (the merge rule takes P
first; the fallback is not needed).

**Every changed message** (P; ee7c0ce changed none): `crowded` slot 474
(40 signals):
- moved, (m): `NP9I UT7EH R+15`, −8.5 dB, 2 246.5 Hz: bafed37 pass 3
  rank 2, P pass 2 rank 9, both with 32 hard errors in 30 BP rounds.
- gained, (k): `X5FS E5K 73`, −10.1 dB, 2 228.3 Hz: `not_converged` at
  bafed37, P pass 3 rank 2 (1 hard error, 1 round); 18.2 Hz from the
  moved message above, which P decoded a pass earlier.

**WAV lines that differ** (P, `host` only): later-pass decodes' score
(255), frequency (6), SNR (3), candidate (9); 3 slots with two decodes
in the other order; 344 `--profile` summaries in the candidates-found
and -dropped counters (total and per later pass). ee7c0ce: score (51),
frequency (3), SNR (3); 118 `--profile` summaries, the same counters.

The x86-64 `accept3` records differ from bafed37's in 1 516 of 2 000
`host` records at P (528 at ee7c0ce) in later-pass parameters, as on
`main`.

### Checks at 4e99bc0

Local CI (fmt, clippy `-D warnings` on the workspace, cat-ft8 all targets
and features and the thumbv8m library, workspace tests, profile and
no-std tests, release tests with the ignored ones (lib 164, the
exhaustive sine/cosine test included), the bulk run, MSRV 1.75, docs,
`check.sh --stack`) passes; thumbv7em builds. 4e99bc0 fixes the no-std
test build (the exhaustive test printed through `std`); test only.
Raspberry Pi 5: the release test suite with the ignored tests passes
(lib 164, then 2 + 0 + 12 + 1 bulk + 7 + 2 + 5); throttle flags 0x50000
before and after.

### After the merge (feat/cat-ft8 at 55bbbff)

Release `cat-ft8-decode` and `paired_bench` byte-identical to
44d01ba's (and d4e669e's) on x86-64 and AArch64. Local CI as above
passes (lib 165 with the new lane test). Raspberry Pi 5 release suite
with the ignored tests passes (lib 165, then 2 + 0 + 12 + 1 bulk + 7 +
2 + 5); throttle flags 0x50000 before and after.

**A76 profile** (out-of-tree sampler, `slotcost host,passes=1 host 200
5`, three-pass slot, 348 272 samples; the sampler's own cost puts the
slot at 156.0 ms against 151.1 without it): subtraction 44.1% (the fit
loops, inlined in the subtraction's body, 18.5%; removal 8.6%; time
search 11.1%, its generator 4.0% of that; the generator on the fit path
5.8%; frequency estimate ~0), belief propagation 31.7%, feed 18.9%,
sync 3.1%, soft bits 1.8%. At 1b47a55 the subtraction was 53.7% and BP
26.2%: belief propagation is now the largest single stage; the
subtraction as a whole is 44.1%. (Corrected in O10: this said BP was the
next largest after the subtraction's fit, which is 18.5%.)

**Not measured:** any Cortex-M at run time (static counts only);
off-air audio; slots beyond 40 signals; the `ceiling` mode (single-pass
by construction: it never calls the subtraction; left to the gate).

## The spectrogram's transform, tenth round (O10, bit-identical, 2026-10-30)

The plan is task_plan.md, "O10". Branch `o10-fft` from ddfd48d; every
output equal to ddfd48d's. Measured with the hygiene of O5–O9 (one
source path, a target directory per commit, SHA-256 checked on the Pi,
rounds alternated, one benchmark at a time, `vcgencmd get_throttled`
0x50000 before and after every Pi run).

### Where a row's time went (ddfd48d)

An out-of-tree microbenchmark of one spectrogram row (the band of
`Band::DEFAULT`, noise input; the stages timed one at a time from a
saved buffer), per row:

| | Cortex-A76 | x86-64 |
|---|---|---|
| load (digit reversal, window) | 4.4 µs | 2.6 µs |
| stage 1, radix 4 (l = 1) | 3.4 | 1.7 |
| stage 2, radix 4 (l = 4) | 9.5 | 2.1 |
| stage 3, radix 4 (l = 16) | 9.5 | 2.3 |
| stage 4, radix 2 (l = 64) | 4.1 | 1.9 |
| stage 5, radix 3 (l = 128) | 3.1 | 3.3 |
| stage 6, radix 5 (l = 384) | 10.2 | 4.8 |
| real split and powers | 6.6 | 2.8 |
| per-bin `log2` and level | 2.4 | 4.6 |
| **row** | **53.2** | **26.2** |

So the per-bin `log2` (which the profile could not separate from the
transform: it is inlined into `power_bins`) is 4.5% of the row on the
A76; the transform is 79% and the split 12%. The transform was 39.7 µs
on the A76, about 95 000 cycles at 2.4 GHz. The code explains it: only
the first stage (one butterfly per block, constant twiddles) was
vectorised, across blocks with many lane shuffles; stages 2–6 were scalar
complex arithmetic, stages 2 and 3 with their twelve twiddles spilled to
the stack and reloaded for every butterfly (A76: 114 instructions, 24
stack loads per butterfly). Each complex product is four multiplies and
two adds and cannot change, so the scalar stages were at the A76's
floating-point throughput.

### Change (4089b5b)

No butterfly's operations or their order change:

- the work buffer as separate real and imaginary arrays (same 15 360 B);
- after the first stage, `LANES` butterflies of adjacent k run side by
  side (four where the target has 128-bit `f32` vectors, SSE2 or NEON;
  one elsewhere, so a Cortex-M runs plain loops). Multi-block stages read
  their twiddles once per group of k and reuse them over the blocks; the
  last stage (one block) reads each where it is used, at
  `TWIDDLES[f + j·m]`, f = m·(k + q·l) mod N a multiple of 4m, so its
  lanes never wrap (checked at compile time for the lane count);
- the real split and powers as plain loops over bins (bin 0, bins to 960,
  bins above 960 with −i times the table's twiddle), which vectorise;
  bin by bin with one lane.

Forms tried and dropped (A76 row, µs): twiddles gathered per group for
the last stage too, 17.3 for that stage (store-forwarding stalls, index
arithmetic; worse than 10.2); an incrementally updated twiddle index in
the last stage, 8.9 against 10.4 for the plain `% N` (−0.5% of the slot,
under the 2% rule: the simpler form kept, and it is also the cheaper one
on the M33); explicit four-wide helper functions (no change); stages not
forced inline (68 µs: the stage sizes stop being constants).

### Result

Per row (microbenchmark, same input), ddfd48d → 4089b5b:

| | A76 | x86-64 |
|---|---|---|
| load | 4.4 → 4.4 µs | 2.6 → 2.4 |
| stages 1–6 | 39.7 → 25.9 | 16.2 → 13.0 |
| split and powers | 6.6 → 2.3 | 2.8 → 0.9 |
| `log2` and level | 2.4 → 2.8 | 4.6 → 4.5 |
| **row** | **53.2 → 37.3 (−30%)** | **26.2 → 20.9 (−20%)** |

Slot (`slotcost host,passes=1 host 200 5`, ddfd48d, 4089b5b and the
round's head 7f89879 round-robin, two rounds, order reversed in the
second; worst / mean ms, both rounds; stage means of the three-pass
slot):

| | A76 three passes | A76 one pass | A76 feed (3 passes) | x86-64 three passes | x86-64 feed |
|---|---|---|---|---|---|
| ddfd48d | 160.8 / 151.1, 151.1 | 32.3, 32.3 | 31.2 | 91.4–91.9 / 85.5, 85.6 | 15.4 |
| 4089b5b (FFT) | 152.1–152.3 / 142.5, 142.6 | 29.6, 29.6 | 22.0 | 88.0–89.4 / 82.0, 82.8 | 12.6 |
| 7f89879 (head) | 151.7 / 142.0, 141.9 | 29.3, 29.3 | 22.0 | 87.4–88.8 / 82.3, 81.9 | 12.6–12.7 |
| **ddfd48d → 7f89879** | **−6.0%** | **−9.3%** | **−29.5%** | **−4.0%** | **−18%** |

Belief propagation and the subtraction are unchanged within 0.9 ms
(placement); throttle flags 0x50000 before and after every A76 run. The
A76 profile (out-of-tree sampler, the three-pass side, 348 378 and
327 296 samples; the sampler puts the slot at 156.0 and 145.9 ms):
`power_bins` 17.7% → 13.0% of the slot (27.6 → 18.9 ms); belief
propagation 31.1% → 32.7% (48.5 → 47.8 ms), the subtraction's fit
18.6% → 20.0%, removal 8.7%, time search 6.2% → 6.5%, the generator
9.8% → 10.4%: the transform is now third, behind BP and the fit.

**Cortex-M33** (the small image, `opt-level = "s"`, LTO; the image run
from `_start` in an instruction-set emulator (unicorn, Cortex-M33 model),
instructions counted inside `finish_row` and the helpers it calls, 184
rows of the check program's slot; instructions, not cycles):

| | ddfd48d | 4089b5b |
|---|---|---|
| instructions per row | 1 179 855 | 1 033 614 (−12.4%) |
| of which load / stages 1–6 / split and powers / levels / `memclr` | 55 725 / 909 225 / 39 622 / 46 477 / 128 806 | 55 722 / 849 947 / 47 862 / 46 925 / 33 158 |
| flash small / host / encoder | 85 508 / 97 948 / 35 536 B | 85 376 / 97 816 / 35 536 B |
| `finish_row` frame | 464 B | 464 B |
| static RAM small / host | 212 044 / 241 612 B | same |

Most of the M33 saving is zeroing no longer done (each butterfly's and
each k's arrays are now written whole); the stages' own instructions
fall 7%. The lane count is one there, so the code is the plain loops.
Not measured: M33 cycles (no board).

**Identity.** By construction (each lane is one butterfly's own
operations; Rust never contracts to FMA) and by test: the transform
equals the reference bit for bit at 1, 2 and 4 lanes on the existing
inputs (noise to 2¹⁶, subnormals, signed zeros, full-scale coherent);
a new test compares the powers with the form as first written bit for
bit over all 1 920 bins and ranges starting and ending at 0, 960/961 and
chunk edges, for noise from 10⁻³⁰ to 2¹⁶, tones, constants and
subnormals, with both forms of the per-bin loop. Mutations (twiddle
rotation sign, summation order, twiddle index) fail them; moving the
960/961 boundary by one bin does not, an equivalent change (the reason
given here first was wrong; corrected under "Review follow-ups"
below). Records:
`paired_bench run` of `main`, `cap1`, `cap2` (`host` on `crowded`,
`wide`, `fading`; `host` on `sweep`, `edges`, `empty` of `main`;
`host,passes=1` and `small` on `sweep`, `crowded`, `edges`, `empty` of
`main`): 63 000 records of 7f89879 equal to ddfd48d's (timings removed)
on x86-64, and the A76's records of 7f89879 equal x86-64's of ddfd48d,
63 000 of 63 000. The 800 WAV slots (`cat-ft8-decode`, plain, `--raw`,
`--profile` without timings; `host`, `small`, `host,passes=1`):
byte-identical, 7 200 runs. `ceiling host main SET` for `crowded`,
`wide`, `fading`: stdout identical but for the moved elapsed figure. The
45 `cat-ft8-encode` outputs identical; no change under `src/waveform`,
`src/message`, `lib.rs` or the encode CLI.

### Small fixes (e641a61, 5cb0e43, 7f89879)

- `Correction::skip` halves the even factor of n(n − 1) before the
  wrapping multiply: exact modulo 2⁶⁴ for every n; the bound and its
  debug assertion are gone; a test checks counts up to `u64::MAX`
  against 128-bit arithmetic.
- `paired_bench ceiling`: the elapsed seconds of the 50%-point line go
  to stderr (as the per-set time already did). The ceiling's stdout loses
  that figure and nothing else, and now compares byte for byte.
- `math.rs`: the coefficients were fitted by linear programming on a grid
  of 4 001 points (`f64`), not by a Remez exchange as the comment (and
  1f7d398's message) said; S5 is weakly determined (79.528 here; other
  grids and fitters 79.52–79.53) and was not adjusted for `f32`. The
  exhaustive test's lower bound is marked as a pin to update with the
  coefficients.
- 1f7d398's message reads the mutation backwards. Correct: the lanes are
  keyed by the sample index within the block (the documented order), and
  the mutation the new test kills keys them from the first usable sample
  (`lane_sums(0, …)`); dropping the single samples after the whole lanes
  is the other. History is not rewritten.
- O9's profile paragraph: belief propagation (31.7%) is the largest
  single stage, not the next after the fit (18.5%); corrected above, and
  two long lines rewrapped.

### Checks at 7f89879

Local CI (fmt, clippy `-D warnings` on the workspace, cat-ft8 with all
targets and features and the thumbv8m library, workspace tests,
profile and no-std tests, release tests with the ignored ones (lib 167,
the exhaustive sine/cosine test included), the bulk run, MSRV 1.75,
docs, `check.sh --stack`) passes; thumbv7em builds. In every thumbv8m
image (`llvm-objdump --triple=thumbv8m.main-none-eabihf
--mattr=+fp-armv8d16sp`): 0 `vfma`/`vfms`/`vfnma`/`vfnms` (`vmla` 67 /
143 / 13 in small / host / encoder, as at ddfd48d), no `f64` helper.
Images: small 85 376 B flash (ddfd48d 85 508), host 97 856 B (97 948),
encoder-only 35 536 B (unchanged); static RAM unchanged; the largest
frames unchanged. Raspberry Pi 5: the release test suite with the
ignored tests passes (lib 167, then 2 + 0 + 12 + 1 bulk + 7 + 2 + 5);
throttle flags 0x50000 before and after.

**Not measured:** Cortex-M33 cycles (instructions only, emulated); the
transform on other vector targets (one lane is the fallback, with the
same results).

### Review follow-ups (78d4f2f, 47739f1, 4af3304)

Every output is still equal to ddfd48d's.

- **78d4f2f.** The stage's sub-transform length is a const generic,
  `stage::<R, L, S>` and `butterflies::<R, L, S>`. So `span == N` and
  every stride are constants by type, not by inlining. The table check
  pins each call's (radix, `l`) against `STAGES`.
  - The lane check is now `Lanes::<L>::OK`, evaluated wherever `fft::<L>`
    is instantiated. Two conditions are derived from `STAGES`, not a
    hard-coded 5: L must divide every `l` after the first stage, and N
    must be a multiple of L·m for every m below the last radix.
  - `fft::<8>`, `fft::<3>` and `fft::<0>` fail to build on stable and on
    1.75. `fft::<1>`, `<2>` and `<4>` build.
  - `cargo check` does not evaluate the check (post-monomorphisation);
    `cargo build` and `cargo test` do.
  - M33 instructions per row and the image's flash are unchanged
    (1 033 614; 85 376 B).
- **47739f1.** `bin_powers` has one form, the three plain loops; the
  bin-by-bin path for one lane is gone. Cortex-M33 small image
  (emulated, 184 rows):
  - instructions per row 1 033 614 → 1 027 705 (−0.6%);
  - flash small 85 376 → 85 720 B, host 97 856 → 98 200 B;
  - `finish_row` frame 464 B and static RAM unchanged;
  - `vmla` 67 / 143 → 75 / 151 (small / host; the loops' multiply-adds,
    which round the product first), fused 0, no `f64` helper.
  - The powers test still checks every bin bit for bit against the form
    as first written; it now has one per-bin form to run.
- **Speed and inlining.** With `#[inline(always)]` removed from `stage`,
  the x86-64 microbenchmark gives the same stage times: 13.1 / 13.0 µs
  against 13.1 / 12.9 µs, two rounds each.
  - A76 (rounds alternated, live bits sampled, none set; one sample
    at 1.7 GHz as the first run started, all others at 2.4 GHz):
    - stages 1–6: 7f89879 25.80 / 25.87 µs, 47739f1 25.68 / 25.73;
    - the row with `log2`: 37.31 / 37.31 against 37.33 / 37.32;
    - without the forced inlining: stages 27.53 / 27.53 (stage 3,
      `l` = 16, 3.6 → 5.4 µs measured alone), row 37.12 / 37.14.
  - O10 saw 68 µs per row with the stages not inlined, when `l` was a
    run-time argument. The transform's speed no longer hangs on the
    attribute; it is kept.
  - The transform itself is no slower than 7f89879's.
- **4af3304.** The two O9 lines that 3a91900's rewrap lengthened are
  reflowed, in a commit of their own.
- **Bin 960** (correcting "Identity" above). The table entry
  `SPLIT_TWIDDLES[960]` is [2.83·10⁻¹⁶, −1.0], and the rotated form from
  entry 0 is [−0, −1]. At k = 960, Z[k] pairs with itself (c = a), so
  oi = −0.5·(ar − ar) = ±0 and xi = −ai in both forms. Only xr moves:
  ar + 2.83·10⁻¹⁶·ai against ar. They differ only when
  |ar| ≲ 4.7·10⁻⁹·|ai|, and then xr² is far below half an ulp of xi², so
  the power is the same. That is why moving the 960/961 boundary is an
  equivalent change.
- **The one-lane image's output, observed.** The small image
  decoded seven slots of the 800-WAV corpus in the instruction-set
  emulator: s0000, s0086, s0174, s0180, s0407, s0446, s0748, with 0, 23,
  30, 10, 30, 30 and 10 decodes.
  - The check program was changed out of tree only to read the slot's
    16-bit samples from memory and write its decodes out.
  - Byte for byte, ddfd48d and 47739f1 give the same: the report's
    counts; every decode's frequency, SNR and time bits, score, hard
    errors, rounds, candidate and text; and the whole spectrogram
    (`LEVELS`) and decode list statics.

**Timing, with the live throttle bits.** `get_throttled` bits
16–19 are sticky (0x50000 since boot here), so equal values before and
after a run prove nothing.

- Every timed A76 run now samples the live bits 0–3, the ARM clock and
  the temperature once a second, on core 0, while the benchmark runs on
  core 3. A sample with a live bit set invalidates its run.
- `slotcost host,passes=1 host 200 5`, one source path, a target
  directory per commit, the binary's SHA-256 printed before each run,
  rounds alternated. Results:

| Run (rounds A B, then B A) | three passes, mean ms | one pass | feed | BP | samples; live-bit samples; clock; peak temperature |
|---|---|---|---|---|---|
| ddfd48d r1 / r2 | 151.1 / 151.2 | 32.3 / 32.3 | 31.26 / 31.25 | 40.28 / 40.29 | 208 / 209; 0; 2 400 MHz throughout; 53.2 °C |
| 47739f1 r1 / r2 | 142.7 / 142.8 | 29.6 / 29.6 | 22.14 / 22.15 | 40.92 / 40.92 | 198 / 198; 0; 2 400 MHz; 53.2 °C |
| **ddfd48d → 47739f1** | **−5.6%** | **−8.4%** | **−29.2%** | +0.64 ms | |
| 7f89879 r1 / r2 | 142.0 / 142.0 | 29.3 / 29.3 | 22.00 / 22.01 | 40.49 / 40.49 | 197 / 196; 0; 2 400 MHz; 53.8 °C |
| 47739f1 r1 / r2 | 142.8 / 142.8 | 29.6 / 29.6 | 22.17 / 22.16 | 40.92 / 40.92 | 198 / 198; 0; 2 400 MHz; 53.8 °C |
| **7f89879 → 47739f1** | **+0.6%** | **+1.0%** | +0.16 ms | +0.43 ms | |

Against O10's head (7f89879), 47739f1 is 0.8 ms slower per three-pass
slot (+0.6%), and 0.3 ms per one-pass slot. The difference is the same
in both rounds.
- Most of it is belief propagation: +0.43 ms, though its code did not
  change, so this is placement. O10 saw 0.7 ms of BP drift between
  4089b5b and 7f89879 the same way.
- The rest is the feed (+0.16 ms, 0.3 µs per row) and the subtraction
  (+0.13 ms).
- That is under the 2% rule, and it is reported, not hidden.
- The rows measured with the old method (O10's table above) agree for
  ddfd48d: 151.1 then, 151.1 / 151.2 now.

**Checks at 47739f1.**
- Local CI passes: fmt; clippy `-D warnings` on the workspace, cat-ft8
  with all targets and features, and the thumbv8m library; workspace,
  profile, no-std and release tests with the ignored ones; the bulk run;
  MSRV 1.75; docs; `check.sh --stack`. thumbv7em also builds.
- Records: `paired_bench run` (the O10 sets) on x86-64 gives 63 000 of
  63 000 equal to ddfd48d's (timings removed). On the A76, 47739f1's
  records equal x86-64's of ddfd48d, 63 000 of 63 000.
- The 800 WAV slots × `host`, `small`, `host,passes=1` × plain, `--raw`,
  `--profile`: byte-identical to ddfd48d's (7 200 runs).
- The ceiling's stdout (`crowded`, `wide`, `fading`) is identical to
  7f89879's, and so to ddfd48d's but for the elapsed figure moved in
  O10.
- The 45 `cat-ft8-encode` outputs and their stdout are identical.
- Raspberry Pi 5: the release test suite with the ignored tests passes
  (lib 167, then 2 + 0 + 12 + 1 bulk + 7 + 2 + 5).

### Proposed: `i16` residual, gated (proposed; not yet run)

A result-changing memory round, written here for review before any code
or run. Nothing below has been run; no block is named yet.

**Change.** The multi-pass caller's residual held as 16-bit samples:
720 000 → 360 000 B per slot (180 000 samples). Pass 1 is fed from the
caller's audio as now, so it does not change. The residual is the audio
rounded once to 16 bits before the first subtraction (exact for 16-bit
input such as WAV), and each subtraction's output sample y is stored as

- s = y × 32 768 (exact), clamped to [−32 768, 32 767] with `max` then
  `min` (a NaN goes to −32 768; the subtraction's output is finite by
  construction);
- r = (s + 12 582 912.0) − 12 582 912.0 in `f32`, which rounds to the
  nearest integer, ties to even, for |s| ≤ 2²² (12 582 912 = 1.5 · 2²³):
  two additions, no fused multiply-add, the same bits on every target and
  no dependence on the float-to-integer conversion's rounding;
- r as `i16` (exact: r is an integer in range).

Reading a sample back is `f32::from(i16) / 32 768` (exact). The fit,
the time search and the removal read 16-bit samples (one conversion per
sample read); the removal rounds once per sample written. The API
change (`decode_passes`, `decode_further_passes` and `subtract` taking
`&mut [i16]`, or a choice of either) is settled with the plan.

**Expected cost and effect.** Rounding adds a uniform error of
1/√12 ≈ 0.29 LSB rms per removal at each sample a removal touches;
after about 34 removals in a 40-signal slot, about √34 × 0.29 ≈ 1.7 LSB
rms. That is invisible against the benchmark's noise (0.03 rms, about
980 LSB) and against any real receiver's, and shows only when the input
sits within a few LSB of silence: the low-level sets below are there to
measure it. The O3 prototype (rounding the whole buffer after each
subtraction) changed 31 of about 38 900 slots' messages (22 gained, 9
lost) and no WAV decode; its speed was not measured. On the A76 the
per-sample conversion in the fit and time search (27% of the slot with
the removal) may cost time, while 360 KB fits the 512 KB L2 and 720 KB
does not, which may save it; this is measured on `main`, not assumed.

**Gate (proposed; to be fixed and committed before any record of the
acceptance block exists).** Candidate C (the round's head) against the
base B (the merged head before it), one attempt, no retune:

- **Block.** One block never used, named in the gate commit, checked
  unused on the host and the Raspberry Pi 5 before that commit and
  again immediately before the run (as for `accept3`).
- **Low-level sets** (new `paired_bench` sets, fixed in the gate commit
  and checked on `main`, `cap1`, `cap2` first, not on the acceptance
  block): the `crowded` layouts (10, 20 and 40 signals; the same SNRs
  relative to the noise) with the noise at 2 and at 4 LSB rms (about
  −84 and −78 dBFS) instead of 0.03, so that the accumulated rounding
  (about 1.7 LSB rms) is comparable to the noise.
- **False decodes: 0** in every pass of every record, in the ceiling
  runs and in the WAV output.
- **Pass 1 byte-identical:** the `host,passes=1` and `small` records
  equal to B's (timings removed); in every `host` record the pass-1
  decodes and pass-1 report equal; the WAV output with `host,passes=1`
  and `small` byte-identical.
- **Ceiling byte-identical:** `ceiling host BLOCK SET` stdout for
  `crowded`, `wide`, `fading` equal to B's with nothing removed (the
  elapsed times are on stderr since O10).
- **Lost:** at most 3 on the acceptance block's `crowded`, `wide`,
  `fading` together, reported per set; `wide` below −16 dB at most 1;
  on the low-level sets together at most 3, and lost minus gained at
  most 2.
- **Every lost, gained and pass-moved message classified** by a
  classifier script (not in the repository), its SHA-256 pinned in the
  gate commit, as (m), (r) or (k) with the O9 definitions; one unexplained
  message fails.
- **800 WAV slots:** in every slot, with `host`, `host,passes=1` and
  `small` (plain, `--raw`, `--profile`), the same messages decoded in
  the same passes as B; other differing lines listed by kind, not
  judged. (WAV input is 16-bit, so only the per-removal rounding acts.)
- **CPU (pre-registered, A76):** measured on `main`, never on the
  acceptance block: the `slotcost host,passes=1 host 200 5` three-pass
  mean of C not above B's by more than the larger of B's own spread
  between its two rounds and 0.5%, rounds alternated (B C, then C B);
  and the mean three-pass decode time per slot of the `main` A76 run
  records (`taskset -c 3`, two rounds, order reversed) under the same
  bound. x86-64 `slotcost` not slower than B's by more than 2%.
- **Memory:** the residual 360 000 B per slot (its type's size, stated);
  `check.sh --stack`: static RAM of every image not above B's, the
  subtraction's frame not above B's.
- **Identity:** the A76 run records equal x86-64's for C (timings
  removed); the A76 release suite with the ignored tests passes;
  thumbv8m.main-none-eabihf and thumbv7em build; no `f64` helper and no
  fused multiply-add in any thumbv8m image.
- **Encoder byte-identical:** no change under `src/waveform`,
  `src/message`, `lib.rs` or the encode CLI; the golden waveform hashes
  pass; the 45 `cat-ft8-encode` outputs identical.

**Merge rule.** C merges if every criterion passes; else nothing merges
and the result is recorded. Applied mechanically.

## Belief propagation, twelfth round (O12, bit-identical, 2026-10-30)

A bit-identical round on the local branch `o12-bp` from 593b7c1; the
plan and its stop rule (ecfabf2) were committed before any measurement.
Every output is equal to 593b7c1's; flash, static RAM and the decoder's
stack are no larger; the Cortex-M33 runs fewer instructions in both
images.

### Measured first (593b7c1)

**Counters.** An out-of-tree microbenchmark (not committed): 256 noisy
codewords (eight messages, σ 0.55–1.01, scaled to the host's mean
|LLR| 4; 175 converge; 3 105 sum-product rounds), user-space hardware
counters (`perf_event_open`; no privileges needed on either host)
around `decode_soft` as the decoder calls it, the round alone (30
rounds per vector with the per-round copy and swap), the stopping test
alone, and `decode_soft` with min-sum. Per edge of a round (522 edges):

| | cycles | instructions | IPC | backend-stall cycles | L1D read misses | branch misses |
|---|---|---|---|---|---|---|
| A76, round | 36.7 | 58.1 | 1.58 | 20.5 | 0.0004 | 0.02 |
| A76, `decode_soft` sum-product | 37.6 | 60.2 | 1.60 | 20.8 | 0.001 | 0.02 |
| A76, `decode_soft` min-sum | 19.6 | 46.3 | 2.36 | 2.1 | 0.001 | 0.34 |
| x86-64 (Zen 2), round | 19.8 | 45.4 | 2.28 | — | 0.002 | 0.01 |

The slot agrees: the A76 one-pass slot's 19.18 ms of BP over 2 297
rounds is 20 040 cycles per round, 38.4 per edge. No cache or branch
problem: the A76 round is backend-bound.

**Where.** Shares of the round; A76 from a sampled PC histogram (a
high-resolution timer signal, 104 000 samples, mapped onto the AArch64
code), x86-64 from `perf record`:

| | tanh pass | atanh pass | inputs (gather) | sums (scatter) | products |
|---|---|---|---|---|---|
| A76 | 43.1% | 32.0% | 8.8% | 10.3% | 4.6% |
| x86-64 | 47.2% | 27.9% | 8.9% | 10.7% | 3.9% |

The tanh loop (four values an iteration) was 70 AArch64 instructions
with two divisions, and 96 x86-64 instructions with two `divps`: it
computed `exp2_neg` and the division for x and for −x and chose with a
mask at the end. The compiler had moved the magnitude's select
(`if negative { -x } else { x }`) past the whole chain. On the
Cortex-M33 the same duplication became a branch to one of two chains,
so only one runs there (O5's "tanh … two `vdiv`" counted both chains'
code, not their execution).

**A76 throughput, measured** (independent operations in a loop):
`fdiv v.4s` one per 6.0 cycles (latency 8.0); `fmul v.4s` two per
cycle.

**Floor (step 1 of the plan).** Per round, from the AArch64 loops and
their trip counts, each resource's work over its A76 throughput:
vector and FP operations ≈ 16 100 (two pipes: 8 050 cycles); 520
four-lane divisions (3 120 cycles on the divider); ≈ 3 500 loads (two
a cycle: 1 750); ≈ 1 400 stores (1 400); 30 300 instructions (four a
cycle: 7 580). The largest gives **15.4 cycles per edge** for 593b7c1's
code (12.7 without the duplicated chain, issue-bound). Measured 36.7,
**2.4× the floor**, so the stop rule's first condition did not hold.
The round was latency-bound: each value's chain (polynomial,
conversions, division) is about 70 cycles end to end, and with 41–70
instructions an iteration only one or two iterations' vector operations
fitted in the A76's schedulers.

**Cortex-M33** (instruction-set emulator, the `check.sh` images,
`opt-level = "s"`): the host image's sum-product round 79 485
instructions (2 825 rounds, 46% of its 488 168 028-instruction one-pass
slot). The small image runs min-sum (`DecodeConfig::SMALL`), not
sum-product: with `decode_soft` out of line (a measurement build) it is
106.0 M of the slot's 346.2 M (31%), about 47 700 instructions a round;
the stopping test 3 314 a call, 3 050 of them deciding all 174 bits
although most rounds fail at one of the first checks. The default
small embedded image's slot is 348 830 353 instructions. So the M33's one-pass BP
share is 31% (small) and 46% (host image), not the A76's 65%.

### Changes

1. **a04fc5a, `tanh_half` once per value**: |x| by clearing the sign
   bit. The forms differ only for −0.0 and NaN, where e is 1.0 and 0.0
   either way. Tests: against the first form bit for bit over every
   4 099th bit pattern and the special values, and (ignored) all 2³²
   (also run on the A76). A mutant that changes the sign test only at
   −0.0 fails only the new test. A76 tanh loop 70 → 41 instructions,
   one division; round 36.7 → 31.8 cycles per edge. **Cortex-M33
   +2.0%** (79 485 → 81 104 a round): there the mask costs a few
   instructions more than the branch.
2. **6fd70e8, the select where nothing vectorises**: `VECTOR` (SSE2,
   NEON) picks the mask, otherwise the select. The M33 round's code is
   again instruction for instruction 593b7c1's, flash equal.
3. **3120384, 24 values at a time in the tanh and atanh passes**
   (`each_value::<PASS_LANES>`; one where `VECTOR` is false). On the M33
   the code is not literally unchanged: the normalised disassembly
   differs only in `sum_product_round` (104 lines in each image); the
   small image does not run that round, so its count is the same, and
   the host image's round runs one instruction more (79 486 against
   79 485, +2 825 over the slot). Six independent chains side by
   side. A76 microbenchmark, cycles per edge (tanh × atanh lanes):
   4 × 4 31.8; 8 28.1; 16 24.3; the 12–24 grid 21.8–24.9 (24 × 24 the
   best); 32 lanes 33.3 (the compiler did not unroll them). In the slot:
   24 lanes 126.6 / 21.7 ms, 16 lanes 129.3 / 23.0, 8 lanes 132.5 / 24.6
   (three passes / one; both rounds equal). x86-64 within noise. Test:
   1, 4, 8, 16, 24 and `PASS_LANES` lanes against the plain map bit for
   bit, for both functions.
4. **7a5898d, the min-sum round in place** (`min_sum_round`): no zeroed
   eight-value row per check (a `memclr` call each on the M33) and no
   bounds check on its index. Test: against the first form over 20 000
   random states, a quarter of them with repeated magnitudes in every
   check; a mutant that drops the second-smallest update fails it and
   the pin. M33 small slot −6.8%.
5. **d1a9334, the totals' references swap**, not the arrays (174 values
   each way a round; 1 384 M33 instructions a round). M33 −1.1% small,
   −1.0% host.
6. **26bb83a, the stopping test decides each check's bits as it tests
   it** (`checks_hold`, `hard_bit`; `hard_decisions` only when every
   check holds, before the CRC and the counts). M33 stopping test 3 314
   → 421 instructions a call; −2.0% small, −1.9% host. Test: against
   the first form over 20 000 states (random, and a codeword with up to
   four bits flipped or erased to ±0); a mutant that ignores the index
   in the tie rule fails it.
7. **d2667f1, one exit in `decode_soft`**: 4–6 had added flash (small
   +104 B, host +128 B) and 8 B to the host decoder frame. One loop that
   tests round 0 first removes the duplicated success path: flash below
   593b7c1's, the decoder frame −8 B; M33 +0.1% against 26bb83a.
8. **636350d, the min-sum check's size as a type where the loops
   vectorise.** 4's loops take the check's length at run time, and on
   the hosts the compiler gave each a vector form with run-time checks
   that no check reaches: the small configuration's BP on the A76 went
   from 19.5 to 25.5 cycles per edge. One body, `min_sum_check`, takes
   any `AsRef<[u8]>` and `AsMut<[f32]>`: with `VECTOR` it is called with
   the size-6 or size-7 arrays, elsewhere with the slices (one instance,
   as before). The M33 code is the same instructions with registers
   renamed: same counts, same flash. A76 `slotcost small small 200 5`:
   18.0 → 15.5 ms (593b7c1: 16.1), BP 8.10 → 5.54; the host
   configuration's three-pass slot +0.4% (BP unchanged).

The host-image figures in the messages of d1a9334 and 26bb83a
(487 679 021 and 478 671 277) were measured on a local build that still
had a04fc5a's masked form on the M33. On `o12-bp` the host slot is
488 121 501 → 483 101 689 → 474 093 945: the same −1.0% and −1.9%.

**Decision rule.** 1–3 carry the A76 (BP −41%, three-pass slot −11%).
4 is kept for the M33 one-pass slot (−6.8%). 5 and 6 are the plan's
"per-round copies and the stopping test's bookkeeping" item, −3.1% of
the M33 one-pass slot together (−1.1% and −2.0% alone); neither moves
the A76 three-pass slot by more than 0.5% (d1a9334 −0.2%, 26bb83a
+0.4%). 7 restores the memory bound. 8 removes a regression of the
small configuration on hosts that 4 introduced (below).

*Deviation (recorded in O12b):* 5 and 6 each fall short of the 3% M33
bar alone. They were kept as one plan item, and that grouping was
decided after their numbers were in, not before. Later plans state
whether a bar applies per commit or per plan item.

### Slot time per commit

`slotcost host,passes=1 host 200 5` on the Cortex-A76, every binary in
two rounds (the order reversed in the second), one source path, a
target directory per commit, the binary's SHA-256 printed before each
run, the benchmark on core 3 and on core 0 the live throttle bits, ARM
clock and temperature sampled once a second (`get_throttled` bits 0–3;
0x50000 is the sticky baseline). Means of the two rounds (ms):

| | three passes | one pass | BP, three passes | BP, one pass |
|---|---|---|---|---|
| 593b7c1 | 142.75 | 29.6 | 40.92 | 19.18 |
| a04fc5a, 6fd70e8 (same AArch64 code) | 138.7 | 27.4 | 36.22 | 16.99 |
| 3120384 | 126.6 | 21.7 | 24.00 | 11.26 |
| 7a5898d | 125.8 | 21.7 | 23.88 | 11.20 |
| d1a9334 | 125.55 | 21.6 | 23.69 | 11.12 |
| 26bb83a | 126.05 | 21.7 | 23.91 | 11.22 |
| d2667f1 | 125.75 | 21.7 | 23.84 | 11.20 |
| **636350d** | **126.25** | **21.7** | **23.87** | **11.21** |
| **593b7c1 → 636350d** | **−11.6%** | **−26.7%** | **−41.7%** | **−41.6%** |

(636350d against d2667f1 in a separate pair of rounds: 126.3 / 126.2
against 125.8 / 125.8.) Every timed A76 run of this round is valid
(this table, the lane and candidate runs, the small configuration's):
no sample with a live bit among 4 682 samples over 34 runs; the clock
2 400 MHz in every sample but two (1 700 and 1 500 MHz in the first
second of a run); peak 56.0 °C. The two rounds of a binary differ by at
most 0.1 ms. BP is now 19.0% of the
three-pass slot (32.7% before) and 52% of the one-pass slot (65%).

x86-64 (Threadripper 3970X, `taskset -c 5`, the same alternation; a
first attempt overlapped with CI builds and is void):

| | three passes | one pass | BP, three passes |
|---|---|---|---|
| 593b7c1 | 82.4 | 17.95 | 21.3 |
| a04fc5a | 78.5 | 16.15 | 17.5 |
| 3120384 | 78.7 | 16.2 | 17.6 |
| d2667f1 | 78.3 | 16.05 | 17.35 |
| 636350d (a separate pair with d2667f1: 78.95 / 16.2 there) | 78.3 | 15.9 | 17.17 |
| 593b7c1 → 636350d | −5.0% | −11.4% | −19.4% |

### Cortex-M33 (emulated instructions)

`check.sh` images (thumbv8m.main-none-eabihf, `opt-level = "s"`), the
check program's synthesized slot from `_start`, callees included:

| | small slot (min-sum) | min-sum round | host slot (sum-product) | sum-product round | stopping test, small / host |
|---|---|---|---|---|---|
| 593b7c1 | 348 830 353 | ≈ 47 700 (inline) | 488 168 028 | 79 485 | 3 314 / 3 279 |
| a04fc5a | 348 830 353 | | 492 742 535 | 81 104 | |
| 6fd70e8 | 348 830 353 | | 488 168 028 | 79 485 | |
| 3120384 | 348 830 353 | | 488 170 853 | 79 486 | |
| 7a5898d | 325 105 605 | 35 451 | 488 121 501 | 79 486 | |
| d1a9334 | 321 510 408 | 34 988 | 483 101 689 | 78 938 | 3 313 / 3 278 |
| 26bb83a | 315 156 347 | 34 988 | 474 093 945 | 78 938 | 421 / 330 |
| d2667f1, **636350d** | **315 473 850** | **34 988** | **474 456 751** | **78 938** | inlined |
| 593b7c1 → 636350d | **−9.6%** | **−27%** | **−2.8%** | −0.7% | |

On the seven WAV slots of O10b (the small image reading the slot's
samples, decoding in the emulator): 2 270 754 510 → 2 085 794 491
instructions in all (−8.1%; per slot −6.8% to −10.0%).

**Sizes** (`check.sh --stack`): flash small 85 720 → 85 596 B, host
98 200 → 98 028, encoder 35 536 (unchanged); static RAM unchanged
(212 044 / 241 612 / 0); the decoder path's frame 2 488 → 2 480 B in
both images; the sum-product round's frame 88 B (unchanged), the new
`min_sum_round` 48 B (inline before); `accept` and the stopping test
are inlined. The deepest BP path is 8 B shallower. No fused
multiply-add (0 `vfma`/`vfms`/`vfnma`/`vfnms` with
`--mattr=+fp-armv8d16sp`; `vmla` 75 / 151 / 13 as before) and no `f64`
helper in any image; thumbv7em builds.

### Rejected (numbers)

- **The mask in `tanh_half` on every target** (a04fc5a alone): M33
  +2.0% a round; replaced by 6fd70e8.
- **32 lanes** in the passes: not unrolled by the compiler, 33.3 cycles
  per edge (24: 21.8). **8 or 16 lanes**: slot 132.5 / 129.3 ms
  against 126.6.
- **Inputs and sums by check degree in the sum-product round**
  (`gather::<6/7>`, `scatter::<6/7>`, on 26bb83a): A76 126.05 → 123.5
  ms (−2.0%; −1.8% against d2667f1), one pass 21.7 → 20.6, BP 23.9 →
  21.7; x86-64 −1.1%. But the M33 round 78 938 → 79 296 (+0.45%) and
  flash +156 B (above 593b7c1's), so it fails the M33 and memory
  bounds; a `VECTOR`-only form would be a second code path for under
  2%. Not kept.
- **The min-sum check by degree on every target**
  (`min_sum_check::<6/7>`, also on the M33): M33 small slot +0.55%,
  flash +264 B (above 593b7c1's); A76 min-sum 17.2 cycles per edge,
  x86-64 16.6. Kept only where `VECTOR` is set (636350d).
- **The min-sum loops bounded by seven** (`.take(MAX_DEGREE)`): M33
  +3.0%, x86-64 min-sum +12%.
- **Branch-free tracking of the two smallest magnitudes** (selects on
  the same comparisons): M33 round 34 988 → 37 076 (+6%).
- **The min-sum check in two fused loops** (inputs with the minimum
  search; outputs with the sums, every operation in the same order;
  on d2667f1): M33 small slot −2.6% (315 473 850 → 307 258 736), flash
  −60 B; but x86-64 min-sum 20.9 → 27.5 cycles per edge (+31%), which
  rejects it on its own; it is also under the 3% M33 bar. A candidate
  for a later round that weighs the small configuration on hosts.

### The round after the changes (A76 counters)

The same microbenchmark with the A76's operation-class counters
(`LD_SPEC`, `ST_SPEC`, `ASE_SPEC` + `VFP_SPEC`, `INST_SPEC`) per edge of
the round, and each class over its throughput (the divider from the
code: four-lane divisions per edge × 6 cycles):

| | cycles | vector/FP ÷ 2 | loads ÷ 2 | stores | instructions ÷ 4 | divider | floor | measured ÷ floor |
|---|---|---|---|---|---|---|---|---|
| 593b7c1 | 36.9 | 15.2 | 3.7 | 2.9 | 14.7 | 6.0 | 15.2 | 2.44 |
| 3120384 | 21.7 | 11.7 | 3.9 | 3.1 | 12.5 | 4.5 | 12.5 | 1.74 |
| d2667f1 | 21.7 | 11.7 | 3.9 | 3.1 | 12.5 | 4.5 | 12.5 | 1.73 |

(The counted floor for 593b7c1, 15.2, agrees with the estimate from the
code, 15.4.) The round is now 1.7× an issue-bound floor, so the stop
rule's first condition still does not hold, but what is left lies in
the inputs and sums (the rejected by-degree form took 9% of the round
and failed the M33 and memory bounds) and in the per-value chains'
latency. The second condition held for every other candidate tried
(above). The next bit-identical gain in BP is small; the larger levers
are result-changing (fewer candidates or rounds) or the fit.

### The small configuration on hosts

`DecodeConfig::SMALL` (min-sum) is the M33's configuration; hosts run
it only when asked (`--config small`, the `small` records). On the A76
(`slotcost small small 200 5`, rounds alternated, live bits sampled):

| | slot mean | BP | min-sum, microbenchmark (cycles per edge) |
|---|---|---|---|
| 593b7c1 | 16.1 ms | 6.29 | 19.5 |
| d2667f1 | 18.0 | 8.10 | 25.5 |
| 636350d | **15.5** | **5.54** | **17.6** |

x86-64: min-sum 20.3, 21.2 and 16.5 cycles per edge; `slotcost small
small 200 5` 13.55, 13.45 and 12.45 ms (BP 6.45, 6.40, 5.34).

### Identity

- **Records:** `paired_bench run` of `crowded`, `wide`, `fading` on
  `main`, `cap1`, `cap2` and `sweep`, `edges`, `empty` on `main`, each
  with `host`, `host,passes=1` and `small` (73 800 records), timings
  removed: equal to 593b7c1's at every code commit on x86-64 (a04fc5a,
  6fd70e8, 3120384, 7a5898d, d1a9334, 26bb83a, d2667f1, 636350d). On
  the A76, the records of 26bb83a, d2667f1 and 636350d equal x86-64's of
  593b7c1 (73 800 of 73 800 each).
- **800 WAV slots** × `host`, `small`, `host,passes=1` × plain, `--raw`,
  `--profile` (7 200 runs, timings removed): byte-identical to
  593b7c1's at every code commit. (636350d's x86-64 and AArch64
  binaries are byte-identical to those of a local build of the same
  tree, on which its timing and identity runs were made.)
- **Encoder:** the 45 `cat-ft8-encode` outputs and their stdout
  identical (d2667f1); no change outside `decode/bp.rs` and its tests.
- **The small image in the emulator** (seven slots of O10b: s0000,
  s0086, s0174, s0180, s0407, s0446, s0748; 0, 23, 30, 10, 30, 30 and 10
  decodes): report counts, every decode's fields and text, and the
  `LEVELS` and `DECODES_LIST` statics byte-identical to 593b7c1's at
  26bb83a, d2667f1 and 636350d.
- **Tests:** every new reference test compares with the form as first
  written bit for bit; the pinned state hash is unchanged; each new
  test has a mutant that fails it (above).

### Checks at 636350d

- Local CI passes: fmt; clippy `-D warnings` on the workspace, cat-ft8
  with all targets and features, and the thumbv8m library; workspace,
  profile, no-std and release tests with the ignored ones (the 2³²
  `tanh_half` test among them); the bulk run; MSRV 1.75; docs, with and
  without `std`; `check.sh --stack`. thumbv7em builds. No fused
  multiply-add and no `f64` helper in any thumbv8m image.
- Raspberry Pi 5: the release test suite with the ignored tests passes
  (lib 172, then 2 + 0 + 12 + 1 bulk + 7 + 2 + 5), so the 2³²
  `tanh_half` test and the other bit-for-bit tests pass on AArch64 too.

### Not measured

- Cortex-M33 cycles: only emulated instruction counts (no hardware
  run, no wait states, no branch penalties).
- The lane count on other AArch64 or x86-64 cores: 24 was chosen on the
  A76 (on x86-64 1–24 lanes are within noise); a core with fewer
  vector registers may spill more.
- The 24-lane form's speed depends on the compiler laying out the six
  chains side by side; a different rustc may choose otherwise (the
  results cannot change, only the speed). 32 lanes already were not
  unrolled.

## Belief propagation, review follow-ups (O12b, bit-identical, 2026-10-31)

On `o12-bp` after 1b1cc91 (plan bc54e8e, committed before any
measurement). Every output equal to 593b7c1's. The Cortex-M33 code is
the same as 1b1cc91's (symbol names aside). On x86-64 and AArch64,
`sum_product_round` and `decode_soft` are the same instruction for
instruction; only `min_sum_round` differs (676d274's slices: 646 → 648
and 606 → 600 instructions).

### Changes

1. **76151f8, `VECTOR` by target architecture** (x86-64, AArch64), not
   by target feature: 32-bit ARM with NEON had it set although LLVM
   keeps these loops scalar there (its vector floating point is not
   IEEE). No result can change on any target.
2. **676d274, the min-sum check over plain slices** cut to `[..6]` or
   `[..7]` instead of the `AsRef`/`AsMut` generics. Cortex-M33 images:
   the same code instruction for instruction (addresses and symbol hashes
   normalised), flash unchanged.
3. **49db9e3, both targets' forms under test on every host.**
   `tanh_half::<V>`, `sum_product_round::<V, W>` and `min_sum_round::<V>`
   take the form as a const parameter; the decoder passes `VECTOR` and
   `PASS_LANES`. The host tests run the scalar forms as well: the select
   in `tanh_half` against its first form (sampled, and over all 2³²
   inputs when ignored tests run), the round one value at a time, the
   min-sum check over slices of either size. The sum-product reference
   round now uses `tanh_half`'s first form. M33: only the rounds' symbol
   names differ (4 lines of 12 362 / 16 258), flash unchanged.
4. **c6e0e4e, every check as the first failure.** The old stopping-test
   states reached only 55 of the 83 first-failure positions. New states:
   bits of a codeword flipped, found by elimination over GF(2), so that
   the syndrome is check c alone or c with random later checks; some with
   bits erased to ±0 where the tie rule keeps the decision, and scaled
   magnitudes. The test asserts the first failing check is c and
   `checks_hold` equals the first form, for all 83 × 12 states.
5. **627d356, `decode_soft` against its first form at every round
   limit.** The reference is 593b7c1's control flow (round 0's test
   before the loop, each round's test after it, the arrays swapped by
   contents, every bit decided before the checks) with the first-form
   rounds. Noisy codewords of four messages, of one with a wrong CRC and
   of the all-zero word (σ 0.3–2.9), erased and random vectors, one all
   zero; both rules; limits 0, 1, 2, 3, 7, 30, 255 and each vector's own
   convergence round and the one before. Compared: the result, the
   rounds run, the totals' and the messages' bits. 120 vectors by default
   (1 768 decodes: 224 at round 0, 78 exactly at their limit, 164 at
   other rounds, 1 059 not converged, 125 CRC, 118 all zero); 3 000 when
   ignored tests run (44 600 decodes, 2 245 at their limit).
6. **ee047b8**, the clippy findings in the new tests (a type alias for
   the rounds' forms, an iterator for the every-check loop).

**Mutants** (each against the whole cat-ft8 library suite):

| Mutant | Before | Now killed by |
|---|---|---|
| The round limit tested before the checks | survived | `decode_soft_matches_its_first_form` |
| The last check skipped, `(0..M - 1)` | survived | `the_stopping_test_fails_first_at_every_check` |
| The select's negative branch returns `x` | survived on hosts | `tanh_half_is_its_first_form_bit_for_bit`, `tanh_and_atanh_helpers`, the sum-product round test (scalar form) |
| The scalar min-sum arm given `[..6]` of every check | not reachable on hosts | the min-sum round test (scalar form) |

### The candidate: variable-side sums (rejected)

Each variable is in exactly three checks, so a compile-time
`VAR_EDGES: [[u16; 3]; N]` (each variable's edges in check order) turns
both rules' per-check scatter into one loop,
`next[i] = llr[i] + m[a] + m[b] + m[c]`: the same additions in the same
order, and the per-round copy `*next = *llr` folded in. A second form
also took the sum-product inputs from a flat `EDGE_VARS: [u8; EDGES]`.
Both passed every test (the pinned state hash and the new decode-level
reference included). The rule, fixed in bc54e8e: keep if the A76
three-pass slot gains ≥ 2% or the M33 small image's one-pass slot ≥ 3%;
flash growth up to +2 KB accepted for it (the maintainer's decision, before measuring).

A76, `slotcost host,passes=1 host 200 5`, each binary against 627d356
(the same code as 636350d) in two alternated rounds; 627d356 over four
runs 125.80, 125.81, 125.79, 125.96 ms (sums of the stage means):

| | three passes | one pass | BP, three passes | subtract (untouched) | feed (untouched) |
|---|---|---|---|---|---|
| 627d356 | 125.84 | 21.7 | 23.93 | 72.08 | 22.44 |
| variable-side sums | 124.32 (−1.21%) | 20.7 (−4.6%) | 22.06 (−7.8%) | 72.81 (+0.73) | 22.00 |
| … and flat inputs | 123.40 (−1.94%) | 20.4 (−6.0%) | 21.31 (−11.0%) | 72.62 (+0.54) | 22.06 |

Cortex-M33 (emulated, `check.sh` images): small one-pass slot
315 473 850 → 310 353 198 (−1.62%, both forms; the min-sum round 34 988
→ 33 129); host image 474 456 751 → 465 106 080 (−1.97%) and
460 860 105 (−2.87%); flash +1 052 B and +1 444 B (both images).
x86-64 (indicative, load average about 2): three passes 78.95 → 77.25
(−2.2%) and 76.85 (−2.7%).

**Rejected**: neither form reaches either bar (A76 −1.21% and −1.94%;
M33 small −1.62%). The tables would also change the
`static_table_bytes` figure that `cat-ft8-decode --profile` prints
(+1 044 / +1 566 B). Not merged.

Two things to note. First, the belief-propagation stage itself gains
7.8% and 11.0%, while in the same alternated A/B the subtraction and the
feed, which neither form touches, moved by +0.73 / +0.54 and −0.44 ms.
The slot bar is at the size of that placement noise (O13 below
proposes how gates should account for it). Second, the bar was fixed
before measuring and is applied as fixed.

**Belief propagation is done for bit-identical work.** The A76 round
is 1.7× an issue-bound floor (O12). The last structural candidate, the
variable-side sums, is under the bar, and what is left is the
per-value chains' latency, which 24 lanes already overlap.

### Toolchain bumps

`PASS_LANES = 24` depends on rustc fully unrolling a 24-value block (it
did not unroll 32), and a toolchain change can lose that with every test
passing. After a toolchain bump, re-run the A76 `slotcost` (the BP
stage). Or check `sum_product_round`'s static size in the AArch64
release `paired_bench`: 1 479 instructions (5 916 B) with rustc 1.93.0
at 627d356. A large drop means the blocks are no longer unrolled.
The doc comment of `PASS_LANES` says so too.

### Identity and checks at 627d356 (and ee047b8)

- **Records:** the 73 800 `paired_bench run` records (`crowded`, `wide`,
  `fading` on `main`, `cap1`, `cap2`; `sweep`, `edges`, `empty` on
  `main`; each with `host`, `host,passes=1`, `small`) equal 593b7c1's on
  x86-64 (24 600 + 24 600 + 24 600, 0 differ) and on the A76
  (24 600 + 24 600 + 24 600, 0 differ).
- **800 WAV slots** × `host`, `small`, `host,passes=1` × plain, `--raw`,
  `--profile` (7 200 runs, 78 648 decode lines, timings removed):
  byte-identical to 593b7c1's.
- **Encoder:** the 45 `cat-ft8-encode` WAV files and their stdout
  identical to 593b7c1's.
- **Cortex-M33 in the emulator**, on the seven WAV slots of O10b: the
  small image (min-sum) and, new this round, the host image
  (sum-product, so the `tanh_half` select and the one-value passes run).
  The report counts, every decode's fields and text, and the `LEVELS` and
  `DECODES_LIST` statics are byte-identical to 593b7c1's in both images
  (host image: 0, 23, 30, 10, 30, 30 and 10 decodes).
- **Sizes** (`check.sh --stack`): flash 85 596 / 98 028 / 35 536 B,
  static RAM 212 044 / 241 612 / 0, frames unchanged (decoder path
  2 480 B); no fused multiply-add (0 `vfma`/`vfms`/`vfnma`/`vfnms`;
  `vmla` 75 / 151 / 13) and no `f64` helper; thumbv7em builds.
- **Local CI:** passes at ee047b8: fmt; clippy `-D warnings` on the
  workspace, cat-ft8 with all targets and features, and the thumbv8m
  library; workspace, profile, no-std and release tests with the ignored
  ones (lib 175: the 2³² `tanh_half` test over both forms and the
  3 000-vector decode reference among them); the bulk run; MSRV 1.75;
  docs with and without `std`; `check.sh --stack`. A first run at 627d356
  failed clippy on two findings in the new tests (a complex type, an
  index loop); ee047b8 fixes them (tests only).
- **Raspberry Pi 5:** the release test suite with the ignored tests passes at
  ee047b8 (lib 175, then 2 + 0 + 12 + 1 bulk + 7 + 2 + 5), so both
  forms' bit-for-bit tests, the 2³² `tanh_half` test and the decode
  reference pass on AArch64 too.
- **Timing:** ee047b8's x86-64 and AArch64 `cat-ft8-decode` and
  `paired_bench` are byte-identical to 627d356's. A76, against 1b1cc91 in
  alternated rounds: 126.21 / 126.21 → 125.78 / 125.77 ms (−0.34%), one
  pass 21.7 both, BP 23.87 → 23.90, the subtraction 72.42 → 72.05
  (placement: its code is unchanged). x86-64: three passes 78.4 / 78.4 →
  78.5 / 79.0, one pass 16.0 / 15.9 → 16.3 / 16.3, BP one pass 8.07 → 8.38
  (+3.8%). `sum_product_round`'s x86-64 code is the same instruction
  for instruction; it moved by 240 B, from a 32-byte- to a
  16-byte-aligned address. So this is placement too (O13 below). The
  small configuration (`slotcost small small 200 5`): A76 15.5 / 15.5 →
  15.4 / 15.4 ms, BP 5.54 both; x86-64 12.4 / 12.4 → 12.4 / 12.5, BP 5.32
  → 5.43 (+2%).

Every timed A76 run of this round is valid: 21 runs (the candidates,
the two profiles, the microbenchmark and the final pairs), no live
throttle bit in any of 2 549 samples, the clock 2 400 MHz except in a
run's first second, peak 55.4 °C.

### Not measured

- M33 cycles (emulated instructions only), as before.
- The variable-side sums on other cores. x86-64 was measured only under
  some background load.

## The subtraction, measured first (O13, 2026-10-31; no code change)

The merged head's subtraction (cat-ft8 tree of 627d356) is about 57% of
the A76 three-pass slot: 72.1 of 125.8 ms over 34.5 subtractions, about
2.09 ms (5.0 M cycles) each. Nothing below changes code. The
measurements are out of tree.

### Method

- **Sampled profile:** the slot benchmark (`slotcost host,passes=1 host
  200 5`) with an out-of-tree wall-clock sampler (SIGALRM every 487 µs,
  unwound stacks, 284 282 samples), on core 3, with the live throttle bits
  sampled on core 0. It was built a second time with line tables, so that
  inlined frames resolve. That build's `subtract::run` is the same
  AArch64 code as the plain build's instruction for instruction (other
  functions differ in layout). The sampler costs about 3% (the profiled
  slot 129.6 ms against 125.8). Shares are scaled to the unprofiled
  72.1 ms.
- **Counters:** an out-of-tree microbenchmark, the subtraction's stages
  called one at a time on a crowded synthesized slot (24 signals, −18 to
  +6 dB, decodes a little off the truth). User-space `perf_event_open`
  needs no privileges on the Raspberry Pi 5. Two event sets: generic
  (cycles, instructions, backend/frontend stalls, L1D and last-level
  read misses, branch misses), and the A76's operation classes (`LD_SPEC`,
  `ST_SPEC`, `DP_SPEC`, `ASE_SPEC`, `VFP_SPEC`, `INST_SPEC`), plus L1D/L2
  refills. Each stage is measured with the stages before it, so a stage
  is the difference ("+correction" less "phase and envelope", and so
  on). It was also PC sampled (SIGPROF every 23 µs) for the loops.
- **Cortex-M33:** the host image rewritten (out of tree) to decode a
  WAV slot and then run the further passes. The slot is held as f32 in
  an emulator-only region, since 720 KB does not fit the part's RAM.
  Instructions are counted per function, callees included, in a build
  with the stages out of line (`#[inline(never)]` on the correlations,
  alignment, reference, correction, fit and removal). Slot s0086: 37
  subtractions. s0407 gives the same counts to 0.2%.

### Where the time goes

A76; the stage's ms in the slot (sampled, inlined frames), its cycles
and instructions per waveform sample (151 680 a subtraction; the time
search per sample of the symbols' middle halves, 75 840 a step), IPC,
the issue floor from the operation classes (the larger of instructions
÷ 4 and vector/FP operations ÷ 2), and the Cortex-M33's instructions
per sample and code size:

| Stage | A76 ms / slot | share of slot | cycles / sample | instr. / sample | IPC | floor (cycles) | measured ÷ floor | M33 instr. / sample | M33 code (B) |
|---|---|---|---|---|---|---|---|---|---|
| Time search: correlations, their generator, alignment | 18.2 | 14.5% | 16.7 (per core sample, one step) | 34.8 | 2.08 | 8.7 | 1.9 | 99.8 (per core sample and step) | 2 126 |
| Generator, fit path (phase, envelope) | 8.7 | 6.9% | 4.24 | 10.3 | 2.43 | 2.6 | 1.6 | 21.8 | 1 324 (shared with the time search) |
| Frequency correction | 7.4 (3.8 by counters) | 5.9% | 1.79 | 6.1 | 3.4 | 1.6 | 1.1 | 24.9 | 488 |
| Reference (cosine, sine) | 13.3 | 10.6% | 6.65 | 9.2 | 1.38 | 3.6 | 1.8 | 44.9 | 276 |
| Fit loop | 6.7 | 5.3% | 3.65 (the slot's loop, from PC samples) | 7.75 | 2.1 | 1.9 | 1.9 | 51.8 | 848 |
| Removal | 16.9 | 13.4% | 7.07 | 12.3 | 1.73 | 4.4 | 1.6 | 33.9 | 1 180 |
| Other (block loop, solves, setup) | 0.9 | 0.7% | | | | | | 3.9 | |
| **Subtraction** | **72.1** | **57.3%** | **32.0** | **63.1** | **1.97** | **15.9** | **2.0** | **242** | |

- **Memory is not the bound** anywhere. L1D refills are 0.09 per sample
  for the whole subtraction, L2 refills 0.02, the last-level read misses
  0.001, and branch misses 0.01. Backend stalls are 12.4 of the 32
  cycles.
- The line-table attribution gives the correction twice what its
  counters do. Its integer instructions are interleaved with the
  reference's in the fused loop. Read the two together as about 21 ms.
- The time search runs about 1.0–1.2 steps a subtraction (the M33 counts
  give 1.22: a second step when the start moves by more than ⅛ symbol).

**Annotated AArch64 loops** (the slot's code, as the microbenchmark's
PC samples place them; per iteration of four samples):

- **Reference:** 36 instructions, 29 of them vector. One chain per
  iteration: the phase to an angle, the polynomial (separate
  `fmul`/`fadd`, no FMA), the quadrant selects, then `st2` into the ring.
  IPC 1.38 and 4.0 backend-stall cycles a sample. **Latency-bound**, the
  shape O12 found in the tanh pass: one 4-lane chain at a time.
- **Time-search products:** 53 instructions, 48 vector: the same
  polynomial plus `input()`'s clamp and mask and the products. Seven of
  the 53 are `dup` of polynomial constants held in general registers,
  re-made every iteration (register pressure).
- **Removal:** 45 instructions, 36 vector. The ramp
  `t = (j − from) as f32` is computed per lane in 64-bit integers: four
  scalar `ucvtf s, x` plus four lane inserts, about 12 of the 45. Then
  the clamp and mask of `input()`, the estimate, the store.
- **Fit (the fused loop):** 31 instructions. The five 4-lane sums are
  stored to the stack every iteration (`stp`/`stur`, three stores), and
  the sine lanes are loaded twice (`ld2` and four single-lane `ld1`).
- **Correction:** 24 integer instructions (the 64-bit phase lanes in
  general registers, 32-bit stores back), 1.1× its floor.
- **Generator:** `fill_phase`'s per-sample prefix sum (5 instructions,
  one dependent add a sample) and `increments`' ramp loops (14–34
  instructions).

**Cortex-M33** (emulated, host image, s0086): 36.7 M instructions a
subtraction, 1.36 G for the slot's 37, against 474 M for the host
image's whole one-pass slot. By stage: time search 9.23 M (its
correlations 8.52 M), fit-path generator 3.30 M, correction 3.77 M,
reference 6.81 M, fit 7.86 M, removal 5.14 M. The small image does not
link the subtraction. The correction's 64-bit lane arithmetic is 25
instructions a sample on the M33 (1.8 cycles on the A76).

### Placement noise, and how gates should treat it

- In this round's alternated A/B pairs (one source path, a target
  directory per commit), a binary's two rounds agree within 0.05 ms.
  Yet stages a change does not touch moved by up to 0.74 ms between
  builds: the subtraction +0.73 and +0.54 ms, the feed −0.44 ms, and
  −0.37 ms between 1b1cc91 and ee047b8 (O12b, above). O12 saw the
  subtraction between 72.05 and 72.88 ms with no change to it. On
  x86-64 the one-pass BP moved +3.8% with `sum_product_round`'s code
  identical and 240 B away.
- So a 0.5% (0.6 ms) three-pass bound is inside the layout swing, and a
  2% bar (2.5 ms) carries an error of up to about ±0.8 ms from layout
  alone.
- **Proposal for later gates:**
  1. **Judge a candidate by the stages it changes.** Use their summed
     stage time from the slot benchmark's per-stage means, and its
     counters where it has a microbenchmark. Express the gain as a share
     of the slot.
  2. **Check the rest of the slot for non-regression only.** The
     stages it does not touch may move by at most the layout allowance:
     the larger of 1.0 ms and the largest untouched-stage move seen in
     the same A/B.
  3. **Classify slot-level results three ways.** A gain beyond bar +
     allowance passes. A gain below bar − allowance fails. Anything
     between is inconclusive, and is re-measured with a second
     independent build pair (a one-line source change elsewhere moves
     the layout) before it is decided.
  4. **Optionally, align loops for A/B builds only.** Test first whether
     fixed loop and function alignment (`-C llvm-args=-align-loops=64`
     and function alignment) shrinks the swing on an unchanged pair. If
     it does, use it for measurement builds only.
- Under rule 1, the variable-side sums with flat inputs (O12b) would
  have measured −2.64 ms (2.1% of the slot) in the stage they change.
  That rule was not the pre-registered one, so the O12b result stands;
  the candidate can be gated again if the rule changes.

### Candidates (ranked; bit-identical by construction; not implemented)

1. **Several four-lane chains side by side in the reference and the
   time-search products** (O12's lever for the tanh pass). Each value's
   operations stay the same; only independent samples' instructions
   interleave. A count of 1 where `VECTOR` is false leaves the M33 code
   as it is. The reference is at 1.8× its floor, latency-bound. The
   products loop also re-makes seven constants each iteration.
   - Expected: reference 13.3 → about 8 ms and the products about −3 ms,
     about −8 ms in all (≈ 6% of the three-pass slot).
2. **The removal's ramp index as an exact `f32` per lane.** `j − from`
   is an integer below 2²⁴, so converting it as a 32-bit integer in
   vector, or stepping an `f32` lane counter by 4.0, gives the same bits
   as the 64-bit conversion. That removes about 12 of the loop's 45
   instructions per four samples.
   - Expected: removal 16.9 → about 12.5 ms (≈ 3.5% of the slot).
   - M33: `usize` is 32 bits there, and the conversion is already one
     `vcvt`; the M33 code must not grow.
3. **The fit's sums in registers and the sine loaded once.** The five
   lane sums are stored every iteration and the sine lanes are loaded
   twice.
   - Expected: fit 6.7 → about 4.5 ms (≈ 1.7%).
   - The same order of every addition is kept.
- Not ranked:
  - the correction, which is near its floor (1.1×);
  - the generator's prefix sum, a dependent add per sample in the
    waveform code that the encoder shares.

**Stop rule** (fixed now, before any candidate is written):
- Stop and record that the subtraction is done for bit-identical work if
  either holds:
  1. the measured subtraction is within 1.3× of its issue floor (now
     2.0×, 32.0 against 15.9 cycles a sample);
  2. no candidate reaches 2% of the A76 three-pass slot, judged as
     proposed above: the changed stages' time, and the untouched stages
     within the allowance.
- Each candidate is also bounded:
  - no Cortex-M33 regression in the host image's subtraction
    instructions;
  - no flash growth in the small image (it does not link the
    subtraction);
  - host-image flash +512 B at most;
  - no larger subtraction stack or scratch.
- The bar applies per candidate, not per group. A candidate that misses
  is recorded with its numbers and not combined with another
  afterwards.

### Not measured

- x86-64 per-stage counters: the generic set of seven events
  multiplexes on Zen 2, and the counts were not scaled. Only the A76's
  are reported.
- How the stage shares vary with slot content. The profile is the
  `crowded` `main` block (24.1 and 10.4 subtractions in passes 2 and 3).
- M33 cycles: emulated instructions only, of an out-of-line build. The
  default embedded image inlines the stages into one function (1.42 G against
  1.36 G instructions for the slot's subtractions).

## The subtraction, thirteenth round (O13, bit-identical, 2026-10-31)

On the local branch `o13-sub` from 67e2458; the round's rules (a003d6e)
were committed before any new measurement. Four candidates, each judged
against the branch head before it by the stages it changes; all four
pass. The A76 three-pass slot is 125.84 → 107.29 ms (−14.7%) and the
subtraction 72.09 → 56.58 ms (−21.5%) in the per-candidate pairs.
Decodes, records, residuals and encoder identical; one profile metadata
field changes: `static_table_bytes` in `cat-ft8-decode --profile`
(36 260 → 37 826), the fourth candidate's tables. "Bit-identical" in
this round means that, with this one qualification. The three
subtraction commits alone (e3f5444) change no output.

### Changes

1. **d486205: the reference and the time-search products, 16 samples
   at a time.**
   - `scaled_cos_sin::<CHAINS>` does the work. `CHAINS` is 16 where
     `VECTOR` is set and 1 elsewhere. `VECTOR` moved to `math.rs`, and
     `bp` now uses it from there.
   - Each sample's operations are the plain loop's. Four independent
     four-lane chains interleave.
   - The blocked products loop no longer re-makes its seven constants
     (`dup`). It is 195 AArch64 instructions per 16 samples, against
     4 × 53 before.
   - The count was chosen with the microbenchmark (below). The
     subtraction's cycles per sample, and the reference stage's:

     | samples at a time | subtraction | reference |
     |---|---|---|
     | 4 (before) | 31.86 | 6.66 |
     | 8 | 34.42 | 10.37 |
     | **16** | **29.35** | **5.32** |
     | 24 | 29.49 | 5.23 |
     | 32 | 30.64 | 5.43 |

     At 8 the reference block is not unrolled well.
   - On the M33 (count 1) the small image is the same code instruction
     for instruction. The host image differs only in the order of a few
     stores outside the loops, with the same counts.
   - Test: `scaled_cos_sin` at 1, 4, 8, 16 and 24 samples at a time
     against the plain loop, byte for byte. Inputs: random and edge
     phases and weights, every length up to three blocks and beyond,
     each input the shortest in turn. A mutant that drops the audio's
     length from the count fails only this test.
2. **d116122: the removal's ramp index in `i32`.**
   - Exactness. Every index here is a reference sample or a segment
     centre, at most `WAVEFORM_SAMPLES` (below 2²⁴, asserted at compile
     time). So `j0 + i − from` is exact in `i32`. Integers below 2²⁴
     are exact in `f32`, so its `f32` equals that of the `usize`
     difference. At and before the centre the clamp gives +0.0 in both
     forms.
   - The loop converts four lanes at a time (`smax`, `ucvtf v.4s`). It
     is 28 AArch64 instructions per four samples, down from 45; the
     reporting loop is 168, down from 206.
   - On the M33 (`usize` is 32 bits there), the removal's loops are four
     instructions a sample shorter.
   - Test: `run_as` against its first form, reporting and not. The runs
     start before, at, across and far after the centre (indices up to
     the waveform's end), at every lane offset. A mutant that narrows
     the index to `i16` fails only this test.
3. **94ae1b8: `Fit4::add_all` out of line.**
   - Inlined into the fit's block loop, the compiler assembled the lanes
     from single-lane loads, lane moves and transposes. It also kept
     some sums as scalar lanes: 47 instructions per four samples.
   - Out of line, the five sums stay in vector registers, and the
     reference comes in through one de-interleaving load: 27
     instructions.
   - The sine's lanes are still loaded a second time for its square. No
     source form tried changed that: statement order, separate lane
     loops, products first, an explicit eight-value de-interleave, a
     fixed-size chunk.
   - Test: `add_all` against its definition (sample i in lane i % 4,
     each lane in sample order), from random sums at every start modulo
     the lanes. A mutant that puts the edge samples in the next lane
     fails it and `anchored_to_the_unheld_subtraction`.
4. **e3f5444: the new tests compare finite references** (tests only).
   - An unoptimised build failed the removal test with NaN references:
     the energy came out a NaN of the other sign.
   - Rust does not specify the sign or payload of a NaN that arithmetic
     produces, so a bit-for-bit check means something only where no NaN
     is computed.
   - The reference (an envelope times a cosine or sine) is finite, so
     only finite references are compared. The audio keeps its
     non-finite edge values, which the front end's clamp takes to zero.
5. **57cbc67: belief propagation's sums by variable and inputs by
   edge.** This is the O12b candidate with flat inputs, ported.
   - The round tests now start the next totals as NaN, so the folded
     copy of the soft bits is under test.
   - A mutant that swaps the second and third edges' order fails four
     tests: the two rounds', the pinned state and the decode-level
     reference.
   - Its A76 binaries are byte-identical to those of the candidate
     commit that was measured (5903527, on a side branch from 94ae1b8).

### Decision table

A76, `slotcost host,passes=1 host 200 5`, each commit against its
parent, in two alternated rounds. One source path, a target directory per
commit, the binary hashes printed. Stage means are summed over the
passes (ms); G is the changed stage's gain, B the bar (2% of the
parent's slot), A the allowance:

| Candidate | changed stage | G (ms) | % of slot | B | A (largest untouched move) | untouched net | one pass | result |
|---|---|---|---|---|---|---|---|---|
| 1 d486205 | subtract 72.09 → 67.02 | 5.07 | 4.03 | 2.52 | 1.00 (feed −0.27) | −0.35 | 21.70 → 21.60 | **kept** |
| 2 d116122 | subtract 67.02 → 60.56 | 6.45 | 5.36 | 2.41 | 1.00 (feed −0.14) | −0.14 | 21.60 → 21.50 | **kept** |
| 3 94ae1b8 | subtract 60.56 → 57.09 | 3.48 | 3.05 | 2.28 | 1.00 (BP +0.82) | +0.80 | 21.50 → 21.90 | **kept** |
| 4 57cbc67 | BP 24.64 → 21.31 | 3.34 | 3.00 | 2.22 | 1.00 (subtract −0.47) | −0.48 | 21.90 → 20.30 | **kept** |

The slots (sums of stage means) went 125.84 → 120.41 → 113.82 → 111.12
→ 107.29 ms. No result fell in the inconclusive band, so no second build
pair was run. Two results are near its edge:

- **Candidate 3 passes by 0.20 ms** (B + A = 3.28). Its untouched BP
  moved +0.82 ms. `sum_product_round` is the same AArch64 code
  instruction for instruction at both commits, 0x610 bytes apart, so
  that move is placement.
- **Candidate 4: G 3.34 ms against an inflated parent, about 2.5 ms
  against the same-code median** (corrected after review). Its parent
  94ae1b8 measured BP at 24.64 ms, but BP's code is byte-identical at
  a003d6e, d486205, d116122 and 94ae1b8 (23.91, 23.84, 23.83, 24.64).
  Against the median of those builds (23.87) G is 2.56 ms (about 2.3%
  of the slot): above B (2.22), below B + A (3.22), so with a clean
  baseline it would have been inconclusive. A second build pair would
  then have decided on the mean G against B; to reverse the result it
  would have needed G₂ < 2 × 2.222 − 3.335 = 1.11 ms. Every other
  estimate is about 2.5 ms or more (O12b by stage −2.64 ms; this
  round's whole-round BP 23.91 → 21.30; x86-64 BP −10.6%; M33 −1.6%
  small, −2.9% host, which placement cannot touch), so it was not run
  and the candidate is kept on the corrected number.
- **How the rule was applied, and its weak points** (from review):
  - The measure-first proposal's non-regression clause (untouched stages
    may move by at most the allowance) was circular: A is defined from
    the largest untouched move, so it could never fail. a003d6e
    replaced it with "untouched net above 1.0 ms is inconclusive",
    which is one-sided (a favourable net is ignored) and only sends a
    candidate to a second pair.
  - The second-pair rule decides on the mean G against B and ignores
    the allowance: lenient.
  - Sequential gating hands each candidate its parent's placement: the
    +0.82 ms that 94ae1b8's untouched BP absorbed came back as part of
    candidate 4's G. The standing rule from O13b (task plan) baselines
    the changed stage at the median of the round's same-code builds.
  - Candidate 4 was rejected in O12b under a whole-slot bar and entered
    again here under the changed-stage rule. That re-entry was recorded
    in advance (the measure-first step: "the candidate can be gated
    again if the rule changes"), so it is legitimate, but it was a rule
    change that rescued it.

### Cortex-M33 (emulated instructions) and sizes

The `check.sh` images give the one-pass slots. The subtraction's count
comes from the host image rewritten out of tree to decode slot s0086 and
then run the further passes (37 subtractions).

| | small one-pass slot | host one-pass slot | subtraction, s0086 | flash small / host (B) |
|---|---|---|---|---|
| 67e2458 | 315 473 850 | 474 456 751 | 1 451 983 677 | 85 596 / 98 028 |
| d486205 | 315 473 850 | 474 456 751 | 1 451 983 677 | 85 596 / 98 028 |
| d116122 | 315 473 850 | 474 456 751 | 1 429 616 229 (−1.5%) | 85 596 / 98 024 |
| 94ae1b8 | 315 473 850 | 474 456 748 | 1 345 966 536 (−5.9%) | 85 596 / 97 184 |
| 57cbc67 | 310 353 198 (−1.6%) | 460 860 102 (−2.9%) | (no change to it) | 87 040 / 98 628 |

- No candidate meets the 3% M33 bar, so each is carried by the A76 bar.
  No M33 count rises.
- In the multi-pass image, code outside the subtraction (for example
  `Spectrogram::finish_row`) is inlined differently from build to
  build: ±70 656 instructions with no change to it. That image's
  whole-slot figure is therefore not used. The `check.sh` images do not
  differ there.
- Static RAM is unchanged (212 044 / 241 612 / 0 B).
- Frames: `subtract_one` 4 856 → 4 808 B; the others are unchanged.
- Flash at 94ae1b8 is below 67e2458's in both images. 57cbc67 adds the
  tables, +1 444 B in both, within the +2 KB recorded in O12b.
- No fused multiply-add (0 `vfma`/`vfms`/`vfnma`/`vfnms`; `vmla` 75 /
  146 / 13) and no `f64` helper or instruction in any image. thumbv7em
  builds.

### The subtraction against its floor

The microbenchmark runs the O13 measurement's crowded synthesized slot
on the A76 with its operation-class counters. The floor is the larger of
instructions ÷ 4 and vector/FP operations ÷ 2.

| | cycles / sample | instructions | vector + FP | floor | ÷ floor |
|---|---|---|---|---|---|
| 67e2458 | 31.84 | 63.46 | 30.60 | 15.87 | 2.01 |
| 94ae1b8 | 26.33 | 58.11 | 29.13 | 14.57 | 1.81 |

By stage, in cycles per sample:

| stage | 67e2458 | 94ae1b8 |
|---|---|---|
| correlations (per core sample) | 15.20 | 13.60 |
| reference | 6.66 | 5.31 |
| fit | 6.24 | 3.19 |
| removal | 7.07 | 4.34 |

The generator and the correction are unchanged.

The stop rule's first condition does not hold (1.81× the floor), and
neither does its second: every candidate reached the bar. The fixed list
is used up, so the round ends here. What is left:

- the time search's correlations, the largest stage, at about twice
  their floor;
- the reference, issue-bound at 5.3 cycles a sample against about 4.2
  for its vector operations;
- the sine's second load in the fit.

### x86-64

Threadripper 3970X, `taskset -c 5`, two alternated rounds, on a quiet
host (ms; the means of the two rounds):

| | three passes | one pass | subtract | BP, three passes |
|---|---|---|---|---|
| a003d6e (67e2458's code) | 78.6 | 16.1 | 39.86 | 17.63 |
| 94ae1b8 | 73.2 | 16.25 | 34.29 | 17.73 |
| 57cbc67 | 71.4 | 15.4 | 34.38 | 15.77 |
| a003d6e → 57cbc67 | −9.2% | −4.3% | −13.7% | −10.6% |

### The round as a whole (A76)

a003d6e (67e2458's code; the same `paired_bench` binary as 627d356's)
against 57cbc67 in two alternated rounds (ms):

| | three passes | one pass | subtract | BP, three passes | feed |
|---|---|---|---|---|---|
| a003d6e | 125.81 | 21.70 | 72.07 | 23.91 | 22.44 |
| 57cbc67 | 107.28 | 20.30 | 56.59 | 21.30 | 21.99 |
| change | **−14.7%** | **−6.5%** | **−21.5%** | **−10.9%** | −0.45 (placement) |

The small configuration (`slotcost small small 200 5`) went from 15.4 to
15.2 ms (BP 5.54 → 5.50). The subtraction is now 52.8% of the three-pass
slot, down from 57.3%.

Every timed A76 run of this round is valid: 24 slot runs and 16
microbenchmark runs, with no live throttle bit in any of 3 377 samples.
The clock was 2 400 MHz in every sample except five, all in a run's
first second (1 500–1 700 MHz). Peak 56.0 °C. A binary's two rounds
agree within 0.1 ms.

### Identity

- **Records:** the 73 800 `paired_bench run` records (`crowded`,
  `wide`, `fading` on `main`, `cap1`, `cap2`; `sweep`, `edges`, `empty`
  on `main`; each with `host`, `host,passes=1`, `small`).
  - x86-64: equal to 593b7c1's at d486205, d116122, 94ae1b8 and 57cbc67
    (24 600 + 24 600 + 24 600, 0 differ).
  - A76: equal to x86-64's at 94ae1b8 and at 57cbc67.
- **800 WAV slots** × `host`, `small`, `host,passes=1` × plain, `--raw`,
  `--profile` (7 200 runs, 78 648 decode lines, timings removed):
  - byte-identical to 593b7c1's at d486205, d116122 and 94ae1b8;
  - at 57cbc67, identical except `static_table_bytes` in the 2 400
    `--profile` lines (36 260 → 37 826, the new tables).
- **Encoder:** the 45 `cat-ft8-encode` WAV files identical (57cbc67).
- **Cortex-M33 in the emulator:**
  - the small and host images' decodes on the seven WAV slots of O10b:
    report counts, every decode's fields and text, and the `LEVELS` and
    `DECODES_LIST` statics byte-identical to 627d356's, at 94ae1b8 and
    57cbc67 (14 of 14);
  - new this round, the multi-pass host image's residual after the
    further passes on s0086 and s0407 (180 000 `f32` each): identical at
    67e2458, 94ae1b8 and 57cbc67.

### Checks at 57cbc67

- **Local CI passes:**
  - fmt;
  - clippy `-D warnings` on the workspace, cat-ft8 with all targets and
    features, and the thumbv8m library;
  - workspace, profile, no-std and release tests, with the ignored ones;
  - the bulk run;
  - MSRV 1.75;
  - docs, with and without `std`;
  - `check.sh --stack`.
  - (The first run, at 94ae1b8, failed the unoptimised tests;
    e3f5444 fixes it, above.)
- thumbv7em builds. No FMA and no `f64` in any thumbv8m image.
- **Raspberry Pi 5:** the release test suite with the ignored tests
  passes at 57cbc67 (lib 178, then 2 + 0 + 12 + 1 bulk + 7 + 2 + 5), so
  the new bit-for-bit tests pass on AArch64 too.

### Not measured

- Cortex-M33 cycles (emulated instructions only, as before).
- The chain count on other cores. 16 was chosen on the A76; on x86-64
  it was not swept.
- The 16-sample block and the out-of-line fit both depend on the
  compiler's choices. A toolchain change can lose them with every test
  passing, as with `PASS_LANES`. After a toolchain bump, re-run the A76
  `slotcost`, or check the AArch64 loops' sizes in the release
  `paired_bench` (rustc 1.93.0, 94ae1b8):
  - the blocked reference loops: 142 and 143 instructions;
  - the blocked products loop: 195;
  - `Fit4::add_all`'s loop: 27;
  - the removal's loops: 28 (not reporting) and 168 (reporting).
- A second build pair for candidates 3 and 4; the rule did not call
  for one, and neither reviewer found it could change the decision
  (candidate 3's gain is backed by its instruction counts: AArch64
  47 → 27 per four samples, M33 subtraction −5.9%).

## Review follow-ups (O13b, bit-identical, 2026-10-31)

On `o13-sub` after 41f7368; the rules (9c183f1) were committed before
any new measurement. Five commits: 38e2648, c4c141f, c520517, 3daa258,
f8f2a7b. Decodes, records, residuals and encoder identical to 57cbc67's;
one profile metadata field changes: `static_table_bytes` in
`cat-ft8-decode --profile` (37 826 → 37 245), the padded check list
leaving the tables. Every non-regression check passes, and every change
is kept.

### Changes

1. **38e2648: `scaled_cos_sin` without its type parameter.** All 84
   cat_ft8 functions of the AArch64 `paired_bench` are identical
   instruction for instruction (addresses and names aside), as are both
   M33 images' (83 small, 95 host) and the x86-64 decoder's 744.
2. **c4c141f: the removal at and before the centre** with base −0.0,
   where the clamp's +0.0 is observable. Tests only.
3. **c520517: the removal's index bound as a release `assert!`**, once
   per run. Only `Removal::refs` changes (683 → 657 AArch64
   instructions). Its loops keep their lengths (28 and 168). A76
   subtract 56.62 ms against the same-code median 56.58 (+0.04). M33
   subtraction on s0086, multi-pass image: 36 377 474 → 36 377 957
   instructions per call (+0.0013%). Kept.
4. **3daa258: belief propagation's graph stored once.** Only
   `decode_soft`, `min_sum_round` and `sum_product_round` change, the
   last in how one constant's address is loaded (`adrp` + `ldr` instead
   of `nop` + `adr`), at the same length. A76 BP 21.05 ms against the
   same-code median 21.30 (−0.25), one-pass 9.89 against 10.00 (−0.11).
   M33 one-pass slot: small 310 353 198 → 310 413 261 instructions
   (+0.019%), host 460 860 102 → 461 036 544 (+0.038%), both under the
   0.1% bound; `min_sum_round` 33 129 → 33 104 a call. Flash smaller,
   static RAM and the largest stack frames unchanged (below). Kept.
5. **f8f2a7b: `paired_bench slotcost` prints each pass's new decodes
   and cost per new decode.** Bench output only; every cat_ft8 function
   identical to 3daa258's.

### Timing (A76, against the same-code medians)

Ten runs (two alternated rounds of each of five builds), 1 480 throttle
samples, no live bit, one sample below 2.39 GHz, peak 55.4 °C; each
binary's two rounds agree within 0.04 ms (c520517: 0.23). The changed
stage's baseline is the median of the round's builds whose functions
for it are byte-identical (BP: 41f7368, 38e2648, c520517 at 21.30 ms,
spread 21.07–21.30; subtract: 41f7368, 38e2648 at 56.58 ms). A is
1.0 ms throughout.

| commit | changed stage | ms | vs median | untouched net | three-pass slot |
|---|---|---|---|---|---|
| 41f7368 (base) | — | — | — | — | 107.28 |
| 38e2648 | none | — | — | — | 107.28 |
| c520517 | subtract | 56.62 | +0.04 | −0.06 | 107.25 |
| 3daa258 | BP | 21.05 | −0.25 | +0.39 | 107.43 |
| 3daa258 | subtract (unchanged code) | 56.99 | +0.41 | — | |
| f8f2a7b | BP | 21.34 | +0.04 | +0.10 | 107.44 |

- No regression. The slot moves by at most 0.16 ms, inside placement
  noise.
- **Cost per pass** at f8f2a7b (`slotcost`, crowded `main`): pass 1
  20.30 ms for 24.1 new decodes (0.84 ms each), pass 2 54.33 ms for 10.4
  (5.20 ms each), pass 3 32.82 ms for 3.0 (11.09 ms each). The third
  pass costs about 13 times the first per signal it adds. This is the
  bar any later-pass or OSD proposal has to beat.
- The small embedded configuration on the A76: 15.2 ms (41f7368) and
  15.3 ms (f8f2a7b) a slot, one pass (24.0 decodes, 0.64 ms each).

### Sizes (Cortex-M33 images)

| image | flash before | flash after | static RAM |
|---|---|---|---|
| small | 87 040 B | 86 468 B | 212 044 B (unchanged) |
| host | 98 628 B | 98 128 B | 241 612 B (unchanged) |

`.rodata` −580 B; `.text` +8 B (small), +80 B (host). The ceiling is
245 760 B. The largest stack frames are unchanged.

### Alignment trial (measurement builds only)

The four O13 builds whose non-subtraction functions are byte-identical
(a003d6e, d486205, d116122, 94ae1b8), each built plain and with
`-C llvm-args=-align-loops=64 -C llvm-args=-align-all-functions=6`.
The aligned builds were checked to put the hot functions at offset 0
mod 64, and to keep every non-subtraction function identical across
the four. All eight were timed in one session (16 runs, 2 576 samples,
no live bit, peak 56.0 °C).

| stage (byte-identical) | plain, four builds | spread | aligned | spread |
|---|---|---|---|---|
| BP, three-pass | 23.90 23.84 23.82 24.65 | 0.82 | 24.85 24.86 24.91 24.91 | 0.06 |
| feed, three-pass | 22.44 22.16 22.01 22.00 | 0.44 | 22.48 22.06 22.06 22.07 | 0.41 |
| BP, one-pass | 11.22 11.19 11.18 11.58 | 0.40 | 11.70 11.67 11.69 11.70 | 0.03 |
| sync, three-pass | | 0.03 | | 0.00 |

- The largest spread is 0.82 ms plain and 0.41 ms aligned. The rule
  (aligned at most half the plain, plain at least 0.4 ms) is met:
  **adopted for measurement builds only.** Released builds are
  unchanged.
- **Aligned and plain numbers are not comparable.** Alignment moves
  absolute times (BP about +1.0 ms, the slot about +1 ms), so a gate
  measures every build of a round the same way, and a result is never
  set against a number from the other kind of build.
- The feed's spread barely changes (0.44 → 0.41, mostly a003d6e), so
  alignment does not remove all layout effects. The allowance A stays.

### Standing rule for timed gates

The changed stage's baseline is the median over every build in the
round whose changed-stage functions are byte-identical (compared
function by function in the disassembly, addresses and names aside),
not the parent's measurement alone. The allowance for untouched stages
(the larger of 1.0 ms and the largest untouched move) stays. Measurement
builds use the 64-byte alignment above.

### Identity and checks at f8f2a7b

- **Records:** 73 800 (host, one-pass and small × 24 600) equal on
  x86-64 and on the A76, against x86-64 593b7c1's.
- **800 WAV slots** × 3 configurations × 3 modes (7 200 runs, 78 648
  decode lines): identical to 57cbc67's and 593b7c1's except
  `static_table_bytes` on the 2 400 profile lines.
- **Emulated M33:** the seven slots' decodes, small and host images,
  identical to 627d356's; the multi-pass image's residuals on s0086 and
  s0407 identical to 67e2458's.
- **Encoder:** 45 of 45 cases identical.
- **Local CI passes:** fmt; clippy (workspace, cat-ft8 all targets and
  features, thumbv8m); workspace, profile, no-std and release tests with
  the ignored ones; the bulk run; MSRV 1.75; docs with and without
  `std`; `check.sh --stack`. thumbv7em builds; no FMA and no `f64`
  helper in any thumbv8m image.
- **Raspberry Pi 5:** the release suite with the ignored tests passes
  (lib 179, then 2 + 0 + 12 + 1 bulk + 7 + 2 + 5).

### Not measured

- Cortex-M33 cycles (emulated instructions only, as before).
- Whether 64-byte alignment narrows the spread on x86-64 or for the
  subtraction's own functions (they differ between the trial's builds).

## OSD sizing (measurement only, 2026-10-31)

On `osd-size` from 511bf32; the questions, definitions, threshold rule
and decision metric were committed first (b8ff866) and are in the plan
("OSD sizing round"). The decoder is unchanged: the default release
`cat-ft8-decode` and `cat-ft8-encode` are byte-identical to 511bf32's
(sha256 d111410d… and f87cc8d…, built from one source path at 4657bfe).
Commits: 5ec0eed and 4657bfe (a `profile`-only accessor for a
candidate's soft bits and belief propagation's last totals, placed at
the ends of their files: in the middle it moved the default build's
panic line numbers, and the binary changed), af23f41, 3f48a52, 48dd456
and 2cbb245 (the bench). To reproduce, check out 2cbb245: `paired_bench
osd CONFIG OUTDIR SETS BLOCKS`, `osdcost`, `osdplaus`, `osdcheck` and
`osddump`; the Cortex-M33 image is the embedded-check program with an
OSD run on 16 dumped inputs added before the decode, built out of tree.
The OSD is written from Fossorier and Lin (1995); no other
implementation was read.

### Method

- **OSD** (`examples/paired_bench/osd.rs`, `no_std`, no heap, `f32`
  additions only): reliability order (|value|, ties to the lower
  index), Gauss–Jordan elimination of the generator over GF(2) for the
  most reliable basis, the order-0 codeword re-encoded from the hard
  decisions there, then every one- and two-bit flip of the basis. A
  flip set's discrepancy D is computed from the order-0 codeword's
  (signed weights; for pairs, the rows' shared positions summed with
  four nibble-table accumulators). The CRC-14 has a zero start and no
  final XOR, so its syndrome is linear and form B tests each list
  codeword with a 14-bit XOR. The bit vectors use the target's word
  (64 bits on hosts, 32 on the M33); every sum keeps one order, so
  results are the same on every target.
- **Self-check** (`osdcheck`, 3 000 seeded noisy codewords): every chosen
  word is a codeword; the incremental D matches the direct sum; when
  the basis holds at most i errors, order i returns the truth or a
  codeword of smaller D, and form B never misses a listed truth. Passes.
- **Runs** (x86-64, all threads; results do not depend on the thread
  count): `host` and `small` on `crowded`, `wide`, `fading` × `main`,
  `cap1`, `cap2`; `sweep`, `edges`, `empty` on `main`; `empty` on
  `bulk01`…`bulk20` (E4's blocks). 70 600 slots per configuration, of
  which 48 300 empty. Run twice (4657bfe, then 2cbb245 for the
  non-zero metrics): every set's result hash is equal.
- **Identity across targets**: `osdcost`'s result hashes on the A76
  equal x86-64's (all five modes, 32 481 candidates); the M33 image's
  picks and D bits equal x86-64's on all 16 inputs (both orders, both
  forms, small and host images).

### Deviations from the fixed rules (decided after seeing data)

1. **The false-decode estimate as fixed is unusable.** It counts the
   least-D codeword whatever it is, and on empty slots 1.3% of failed
   candidates have the all-zero codeword at D = 0 (inputs that are zero
   or near it; belief propagation's all-zero rejections). Form A always
   rejects that codeword, but the estimate counts it at 2⁻¹⁴ × π, so no
   threshold, not even D = 0, meets the current rate: **T\* is "none"
   for every input, order, form and metric as fixed.** The estimate
   used below counts only codewords with a non-zero message part
   (`best_nz`, `list_nz`, added in 2cbb245; every other output is
   unchanged).
2. **Candidate subsets in the verdict.** The plan reports by rank (≤ 10,
   ≤ 25, all) and asks which candidates would beat pass 3, but the
   verdict rule names only the input, order, form and metric. Both are
   given: the verdict with all failed candidates (the rule as fixed),
   and with each pass's 10 best-scoring candidates. For a subset, T\* is
   recomputed from the empty slots' candidates of that subset (fewer
   candidates per slot allow a looser threshold).
3. **The replace test** takes the list after pass 2 from `paired_bench
   run host,passes=2` on the same blocks.
4. On the 200 `slotcost` slots, the subset's failed candidates per pass
   are not recorded by slot; their cost is bounded by 10 per pass.

### The factor model holds

On the 48 300 empty slots (host; small alike): form A's least-D codeword
passes the CRC at 5.0–6.5 × 10⁻⁵ of candidates (2⁻¹⁴ = 6.1 × 10⁻⁵), and
0.33–0.46 of those are plausible (π = 0.367 from 1 000 000 random
payloads); form B finds a CRC-passing codeword in 0.0056 (OSD-1) and
0.225 (OSD-2) of candidates, against 1 − (1 − 2⁻¹⁴)^L = 0.0056 and
0.226, and 0.375–0.395 of those are plausible.

**The current rate** (CRC failures per empty slot × 2⁻¹⁴): host 0.0148
→ 9.0 × 10⁻⁷ per slot; small 0.0083 → 5.1 × 10⁻⁷.

### 1. The pool

Host; "missed" is not decoded after all three passes; reach is the
number of missed messages whose best failed candidate has at most i
errors in the most reliable basis (OSD-i lists the truth).

| set, pass | failed / slot | on a missed message / slot | missed messages | dH to truth (ch) q1/med/q3 | reach OSD-0/1/2/3 (ch) | (bp) | ≤ −17 dB messages, reach (ch) |
|---|---|---|---|---|---|---|---|
| crowded/wide/fading × 3 blocks, p1 | 84.5 | 1.65 | 9 918 | 30/42/64 | 317/767/1 339/1 965 | 514/1 084/1 613/2 100 | 3 822: 39/194/403/660 |
| same, p2 | 45.2 | 2.34 | 14 060 | 28/36/50 | 510/1 271/2 351/3 483 | 922/1 945/2 915/3 794 | 6 315: 87/353/800/1 297 |
| same, p3 (5 474 slots) | 48.9 | 2.73 | 14 965 | 27/34/45 | 583/1 473/2 707/4 031 | 1 059/2 263/3 401/4 420 | 6 819: 98/387/900/1 431 |
| `slotcost` slots, p1 | 75.9 | 1.48 | 295 | 38/60/74 | 12/29/42/51 | 15/30/41/50 | — |
| same, p2 | 39.5 | 2.02 | 404 | 32/52/69 | 25/51/67/92 | 31/51/69/92 | — |
| same, p3 | 46.9 | 2.29 | 459 | 30/41/60 | 36/72/96/137 | 38/71/100/130 | — |
| sweep, p1 | 99.4 | 0.41 | 6 167 | 26/33/40 | 125/430/949/1 501 | 380/869/1 347/1 802 | all |
| edges, p1 | 99.1 | 0.06 | 72 | 18/20/24 | 12/26/43/64 | 27/49/61/65 | all |
| empty × 21 blocks, p1 | 100.0 (zero 1.57) | — | — | — | — | — | — |

Small (one pass): crowded/wide/fading 59.5 failed per slot, 36 945
missed messages, reach (ch) 2 352/3 785/5 315/6 847; `slotcost` slots
2 400 missed, reach 207/311/404/491; sweep 6 217, reach
136/463/1 001/1 563. Nearly every failure is "not converged" (CRC
0.008–0.018 and all-zero 0.007–0.05 per slot outside the empty set).

- Most missed messages are far from the hard decisions (median 34–42
  bits on crowded slots, 33 on the sweep) and outside OSD-2's reach:
  OSD-2 could list the truth for 13% (pass 1) to 18% (pass 3) of the
  host's missed crowded messages, and 15% of the sweep's.
- Belief propagation's last totals put more messages in reach than the
  channel's soft bits (about 1.5× at OSD-0/1), but they also put noise
  closer (below).

### 2. Recovery, and 3. false decodes

Distinct messages per slot that OSD recovers and the decoder does not
decode after all passes ("new"), pooled crowded/wide/fading × 3 blocks
(6 000 slots), form A, by threshold on the accepted codeword's Hamming
distance dH; in brackets the ≤ −17 dB part; then the estimated added
false-decode rate per slot (empty slots, non-zero estimate):

| candidates, OSD | dH ≤ 18 | ≤ 20 | ≤ 22 | ≤ 24 | ≤ 26 | ≤ 30 | no threshold |
|---|---|---|---|---|---|---|---|
| host, all, OSD-1 (ch) | 0.019 (0.008) 3.7e-9 | 0.041 (0.014) 6.2e-8 | **0.070 (0.025) 6.4e-7** | 0.109 (0.038) 5.0e-6 | 0.145 2.6e-5 | 0.200 3.2e-4 | 0.235 (0.068) 2.2e-3 |
| host, all, OSD-2 (ch) | 0.040 4.4e-8 | **0.084 (0.035) 6.0e-7** | 0.139 5.7e-6 | 0.207 3.5e-5 | 0.266 1.4e-4 | 0.347 9.1e-4 | 0.384 (0.135) 2.2e-3 |
| host, top 10, OSD-1 (ch) | 0.019 0 | 0.039 5.6e-9 | 0.067 6.5e-8 | **0.104 (0.036) 5.1e-7** | 0.139 2.6e-6 | 0.191 3.2e-5 | 0.225 (0.064) 2.2e-4 |
| host, top 10, OSD-2 (ch) | 0.039 3.2e-9 | 0.080 5.8e-8 | **0.133 (0.055) 5.5e-7** | 0.197 3.5e-6 | 0.255 1.4e-5 | 0.333 9.0e-5 | 0.369 (0.129) 2.2e-4 |
| small, top 25, OSD-1 (ch) | 0.018 9.3e-10 | 0.037 1.8e-8 | 0.064 1.7e-7 | 0.101 1.2e-6 | 0.145 6.5e-6 | 0.232 8.0e-5 | 0.382 (0.018) 5.5e-4 |

Bold: T\*, the largest threshold whose estimate is at most the current
rate (host 9.0 × 10⁻⁷). Other choices at T\*, host, all candidates: the
scale-free Dn = D / Σ|value| recovers far less (ch OSD-1, Dn ≤ 0.0225:
0.002 per slot), and belief propagation's totals less again (bp OSD-1,
Dn ≤ 0.0075: 0.024; dH ≤ 6: 0.015): noise candidates' best codewords
sit as close to the hard decisions as true ones, and the totals pull
both closer. Form B (the best CRC-passing codeword of the list) needs
dH ≤ 9 (ch) and recovers nothing there. Channel soft bits, form A and dH
come out ahead throughout. Wrong codewords from on-message candidates at T\*: none.

- **Recoveries come almost entirely from each pass's 10 best-scoring
  candidates** (0.225 of 0.235 per slot without a threshold), so running
  OSD on those alone keeps the gain while running a tenth of the
  candidates, and with ten candidates per empty slot instead of 100 the
  matched threshold loosens from dH ≤ 22 to ≤ 24.
- At the matched rate OSD keeps 22–46% of what it could recover with
  no threshold: the acceptance test, not the reach, limits it.

**Counted false decodes:** none, at every T\* above, in 70 600 slots
per configuration (empty and off-message candidates of every block,
and wrong codewords of on-message ones): the 95% upper bound is
3 / 70 600 = 4.2 × 10⁻⁵ per slot, 47 times the current rate, so the
count cannot confirm the target; it is consistent with the estimate,
which expects 0.02–0.06 false decodes in these slots at T\*. The
estimate's own count is solid: at host top-10 OSD-1 dH ≤ 24, 1 103 of
483 000 empty-slot candidates have their best non-zero codeword within
24 bits (q = 2.3 × 10⁻³), giving 10 × q × 2⁻¹⁴ × π = 5.1 × 10⁻⁷ per
slot. In a three-pass crowded slot the top-10 subset runs on about
14.5 failed candidates (0.7 + 5.7 + 8.1), not 10, so the rate there is
about 7.4 × 10⁻⁷.

### 4. Cost

| | A76 µs / candidate | M33 instructions / candidate |
|---|---|---|
| preparation (sort, elimination, tables) | 30.6 | 350 692 (324 k–368 k) |
| OSD-1 form A (preparation included) | 31.0 | 352 327 |
| OSD-1 form B | 33.0 | 374 500 |
| OSD-2 form A | 124.9 | 2 091 143 |
| OSD-2 form B | 129.2 | 2 133 801 |

- **A76:** 48dd456, 64-byte-aligned measurement build (sha
  03b1e304…), `osdcost host 200 5` on the 32 481 failed candidates of
  the `slotcost` slots' three passes, one thread on core 3; two runs, 222
  throttle samples, no live bit, one sample below 2.39 GHz, peak
  52.7 °C; the runs agree within 1.4%. The 4657bfe build (the same
  arithmetic, an earlier pair-sum form) gave 30.6 / 30.6 / 32.9 / 118.3 /
  123.3 µs. x86-64 (Threadripper 3970X): 19.0 / 19.2 / 20.5 / 81.0 /
  84.1 µs.
- **Cortex-M33** (emulated, the embedded-check small and host images'
  build settings, `opt-level = "s"`): elimination dominates the
  preparation; OSD-2's 4 095 pairs cost about 425 instructions each.
  Neither was tuned for the M33 beyond 32-bit words (the 6-word XOR
  loop is not unrolled at this optimisation level).
- **Flash:** the OSD's functions 2.7 KB (preparation 1 428 B, search
  996 B, initialisation 318 B) plus the core library's unstable sort
  for the reliability order, 3.6 KB (a small insertion sort would do);
  the image grows by 6.8 KB with both orders (6.7 KB with OSD-1 only),
  test inputs excluded. The generator rows (2 184 B) were built in RAM
  at start; as a table they are 2 184 B of flash.
- **RAM:** the OSD's scratch is 9 200 B (rows 2 184, their message parts
  1 456, the nibble tables 3 072, weights, order and per-row sums
  2 385, the rest small), plus the generator rows 2 184 B. With the
  test harness's 640 B of results, the small image's static RAM goes
  from 212 044 to 224 072 B (ceiling 245 760); the host image's to
  253 640 B, over the ceiling: there it would have to share storage
  (the candidate's belief-propagation scratch, 3.6 KB, is idle once
  belief propagation fails).
- **Stack:** the largest frames are the search (368 B) and the
  preparation (232 B) plus the sort's (quicksort 104 B, recursive;
  depth not analysed); no FMA and no `f64` helper in any image.

### 5. The decision metric

**All failed candidates (the rule as fixed): neither.** On the
`slotcost` slots, at T\*: ch OSD-1 A dH ≤ 22 adds 0.090 new decodes per
slot (p1 0.035, p2 0.035, p3 0.020) for 5.0 ms: 67, 35 and 73 ms per new
decode in passes 1, 2 and 3, against pass 3's 11.09. OSD-2 (dH ≤ 20):
0.050 per slot for 20.3 ms (197–632 ms each). Pooled over 6 000 slots:
OSD-1 77 ms, OSD-2 260 ms per new decode. **Replace pass 3: no** — at
most 0.11 decodes per slot are recovered in passes 1–2 that pass 2's
list lacks, against pass 3's 3.0.

**Each pass's 10 best-scoring candidates: supplement, at a small gain.**

| host, top 10, ch, form A | pass 1 | pass 2 | pass 3 | all passes |
|---|---|---|---|---|
| OSD-1 dH ≤ 24, pooled: failed cand. / slot | 0.7 | 5.7 | 8.1 | 14.5 |
| ms / slot (A76) | 0.02 | 0.18 | 0.25 | 0.45 |
| new decodes / slot | 0.009 | 0.053 | 0.042 | **0.104** (≤ −17 dB 0.036) |
| ms / new decode | 2.5 | 3.3 | 5.9 | **4.3** |
| OSD-1 dH ≤ 24, `slotcost` slots: new / slot | 0.000 | 0.045 | 0.085 | **0.13**, ≤ 0.93 ms → **≤ 7.2 ms each** |
| OSD-2 dH ≤ 22, pooled: ms / slot | 0.09 | 0.71 | 1.01 | 1.81 |
| new / slot (ms each) | 0.008 (10.7) | 0.068 (10.5) | 0.057 (17.6) | 0.133 (13.6) |

- OSD-1 on the top 10 costs under half a millisecond a slot and adds
  about 0.1 decodes per slot (0.13 on the 40-signal slots), at 4–7 ms
  per new decode: below pass 3's 11.09 ms, so it would supplement the
  passes, not replace any. The gain is about 0.5% of a crowded slot's
  decodes, a third of it at or below −17 dB; on the sweep it is 0.015
  per slot (2.6% more of the sweep's decodes) for 0.47 ms.
- OSD-2 over OSD-1 on the same candidates adds 0.029 per slot for
  1.36 ms (47 ms each): not worth it.
- Top 25 instead of top 10: OSD-1 0.086 per slot for 1.62 ms (19 ms
  each). More candidates cost more and, through the false-decode
  budget, force a stricter threshold.

**Small (one pass).** All failed candidates, ch OSD-1 A dH ≤ 22: +0.081
decodes per slot (pooled) for 1.85 ms on the A76 (23 ms each) and 21 M
emulated M33 instructions. Top 25, dH ≤ 23: +0.079 per slot for 0.31 ms
(4.0 ms each, against the small configuration's mean 0.64 ms per
decode) and 3.6 M instructions per slot (45 M per new decode, about
0.22 decodes per 10⁷ instructions); top 10, dH ≤ 23: +0.017 for 0.02 ms.
On the sweep the gain is 0.013 per slot (top 25). OSD-2 on small: 92 ms
per new decode (all), 16 ms (top 25).

**Recommendation: open the gated round, for OSD-1 only, on each pass's
10 best-scoring failed candidates, channel soft bits, accepting the
best codeword when its CRC passes, it is plausible and it is within a
Hamming distance threshold.** It is cheap and beats the third pass's
cost per signal, but the gain is small (about 0.1 decodes per slot) and
rests on a threshold chosen on these blocks with a corrected estimate,
so it must be confirmed on a block never used. Do not pursue OSD-2, the
belief-propagation totals, Dn, or form B.

### What this cannot say

- The knock-on effect of subtracting OSD's decodes before the next pass
  (more decodes later, or damage when one is wrong): not simulated.
- The threshold and subset were chosen on these blocks; the gain on a
  new block may be lower.
- The false-decode rate rests on the factor estimate; counting confirms
  only that it is below 4.2 × 10⁻⁵ per slot.
- M33 cycles, and an M33-tuned OSD (an unrolled elimination and a small
  sort could plausibly halve the preparation): not measured. The sort's
  recursion depth on the M33 stack is not analysed.
- The `slotcost` subset cost is a bound (10 per pass), not a count.
- A76 cost of the 2cbb245 bench (only measurement-mode code changed after
  48dd456; the timed form's source is the same).

## Lean OSD-1 in the library (2026-10-31; option off, bit-identical)

On branch `osd1` from c3b7dd7. Commits 043ee8a (the decoder's OSD-1 and
its option), e7e7f48 (bench: `osdlib`, `osdcal`, the sizing OSD's
renames and notes). Written from Fossorier and Lin (1995) and the sizing
bench's own OSD; no other implementation was read.

### The change

- `DecodeConfig::osd: Option<OsdConfig>`, `None` in both presets (off).
  `OsdConfig::new(candidates, max_distance)`: after belief propagation
  fails on candidate index < `candidates` of a pass, OSD-1 form A on the
  candidate's soft bits; a decode when the least-D codeword's message
  part is non-zero, its CRC holds, its payload unpacks with `NoHashes`
  and it is at most `max_distance` bits from the hard decisions.
  `Decode::hard_errors` is that distance. `Stage::Osd` times it (a new
  stage: the profile line gains `osd_seconds`).
- `decode::osd` (crate-private; public under `profile`): the generator
  as a `static [Bits; K]` built at compile time (2 184 B of flash);
  reliability order by a stable LSD radix sort on `!(|v| bits)`, four
  8-bit digits, counts and the second buffer in the scratch, no
  recursion; Gauss–Jordan elimination with the XOR written out per word
  (three 64-bit or six 32-bit), no per-row `j != rank` test (the pivot
  row cleared with the rest and put back), and message columns no
  cleared pivot row has touched skipped (their single systematic 1 needs
  no clearing; bit-identical by construction and by test); row sums in
  one register accumulator per 64 positions, in the bench's order; the
  pick as `Option<u8>`; the message part and distance computed once, on
  the winner. No nibble tables, no `row_h`, no message array, no form B.
- **Storage:** `OsdScratch` (3 576 B: rows 2 184, signed weights 696,
  order and radix buffer 348, pivots 91, digit counts 256, BP's round
  count 1) shares the candidate's belief-propagation storage
  (`BpScratch`, 3 656 B) through a safe two-variant enum in
  `CandidateScratch` (`Work::Bp` / `Work::Osd`; `forbid(unsafe_code)`
  stays). OSD-1 takes the storage only after belief propagation has
  failed, and the next candidate's belief propagation re-initialises it.
  The enum's tag is the only memory added: 4 B on the Cortex-M33, 8 B on
  64-bit hosts (`CandidateScratch` 4 864 → 4 872 B on x86-64).
- The per-target elimination form is kept: a branch on 32-bit targets,
  a mask on 64-bit hosts (measured below).
- `osd_decode` is out of line, so its frame never adds to the candidate
  decode's own.

### Checks of the result

- **Differential test** (`tests/osd_differential.rs`, `profile`): the
  library's pick, D (bit for bit), codeword and distance equal the
  sizing bench's OSD (`examples/paired_bench/osd.rs`) on 15 008 inputs in
  release builds (1 508 in debug): noisy codewords (σ 0.2–1.8, four
  scales), pure noise, values on a 0.5 and a 1.0 grid, sprinkled exact
  ±0, constant inputs (±0, ±1, 10⁻³⁰, −3.5) and alternating patterns; in
  both elimination forms. Coverage: 5 448 order-0 picks, 1 564 inputs
  with a tie at the least D, 6 475 with ±0, 5 012 with tied magnitudes
  in the basis. Two mutations fail it (the clean-column skip applied to
  every message column; `<=` for `<` in the pick).
- Unit tests: the rows are `ldpc::encode`'s unit codewords; the order
  equals a reference sort (ties, ±0); the reduced rows are codewords in
  systematic form on increasing-rank pivots; the pick's D matches the
  direct sum and is at most the truth's when the basis holds at most one
  error; zero input gives the zero codeword. A decoder test: with belief
  propagation held to round 0, OSD-1 on the first ten candidates
  recovers a −15 dB signal, `Stage::Osd` is reported for every one of
  them, the round count survives, a tighter distance rejects it, and
  zero candidates give the decoder's own result.
- The Cortex-M33 image's 16 dumped inputs: picks and D bits equal the
  sizing's x86-64 values in both forms.

### Identity with the option off (x86-64, against 511bf32)

- **Records:** 73 800 (host, host one-pass and small × 24 600:
  crowded/wide/fading × main/cap1/cap2, sweep/edges/empty main) equal.
- **800 WAV slots** × 3 configurations × 3 modes (7 200 runs, 95 448
  lines, 78 648 decode lines): identical except three fields of the
  2 400 profile lines: `candidate_scratch_bytes` 4 864 → 4 872,
  `peak_bytes` +8, `static_table_bytes` 37 245 → 39 429 (the generator
  table). Timing fields are removed before comparing, as before.
- The committed build (e7e7f48): records again equal (73 800, against
  the references those of 511bf32 equal), and the WAV output
  byte-identical to the working-tree build's above.

### Cost (the lean OSD-1 against the sizing's)

| | sizing OSD-1 A (48dd456) | lean (e7e7f48) | change |
|---|---|---|---|
| A76 µs / candidate (aligned build, `osdlib host 200 5`) | 31.0 (30.8 in the same run) | **23.17** | −25% |
| x86-64 µs / candidate (plain build, one thread) | 19.2 (19.5–19.8 same run) | 14.0 | −28% |
| M33 emulated instructions / candidate (16 inputs) | 352 327 | **204 062** (177 406–219 703; median 207 144) | −42% |
| flash, small / host image | +6.7 KB code (3.6 KB the core sort) + 2.2 KB rows | **+7 420 / +7 460 B** (option on or off) | |
| static RAM, M33 | scratch 9 200 B + rows 2 184 B (host over the ceiling unshared) | **+4 B** (212 044 → 212 048; 241 612 → 241 616) | |
| largest new frame (M33) | search 368 B, preparation 232 B, recursive quicksort 104 B (depth not analysed) | `osd_decode` 488 B (OSD-1 inlined); no recursion | |

- **A76:** 64-byte-aligned measurement build of e7e7f48 (paired_bench
  sha256 eed244c9…), `osdlib host 200 5` on the 32 481 failed
  candidates of the `slotcost` slots' three passes, core 3; two runs,
  138 throttle samples (sticky 0x50000 throughout, no live bit), one
  sample below 2.39 GHz (the first), peak 51.6 °C; the runs agree within
  0.04%. The three forms' result hashes are equal (6fe3fb66…).
- **Per-target elimination split** (the other form in brackets): x86-64
  mask 14.0 µs (branch 28.6), A76 mask 23.2 µs (branch 37.1), M33 branch
  204 062 instructions (mask 314 250). The mask is kept on 64-bit hosts
  and the branch on 32-bit targets.
- **Flash** (the check program's images, option on and off alike, since
  the option is a run-time setting): small 86 468 → 93 888 B, host
  98 128 → 105 588 B. Of the 7.4 KB: the generator table 2 184 B, the
  plausibility test's own instance of `unpack` (with `NoHashes`) 3 520 B
  (measured by leaving it out), OSD-1 and its glue about 1.7 KB.
- **Stack** (`check.sh --stack`): the largest frames are unchanged
  (`_start` 4 096 B; host `subtract_one` 4 808 B, the subtraction 2 872 B);
  `decode_a_slot` 2 480 → 2 504 B; the new `osd_decode` 488 B (504 B in
  the measurement image, where `order1` is out of line at 216 B and the
  radix sort 64 B). On the candidate path the OSD adds at most about
  0.5 KB below `decode_a_slot`, inside the 16 KB stack budget; it never
  runs during a subtraction (the host image's deepest path).
- No FMA and no `f64` helper in any image (five images now: small,
  host, each with the option on, and the encoder alone).

## OSD-1 false-decode calibration (counted, 2026-10-31)

As fixed in 69fcfad, before any of these slots existed: the `empty`
set's blocks `qv0000`–`qv2999` (6 900 000 slots: 3.0 M noise, 1.5 M weak
carriers, 0.6 M each strong carriers, clipped, floored, impulsive), all
30 segments run, nothing stopped or extended; `paired_bench osdcal`
(e7e7f48, the decoder's OSD off; after each pass the decoder's own
OSD-1 on every failed candidate below the rank). x86-64, 16 threads,
3.6 h per configuration. Aggregates only (5.6 MB of JSON).

### The current rate on the same slots

| | BP CRC failures / slot | R0 = × 2⁻¹⁴ | BP's own decodes (all false) |
|---|---|---|---|
| host | 0.01454 | **8.88 × 10⁻⁷** | 7 (1.0 × 10⁻⁶ per slot, 95% bound 1.9 × 10⁻⁶) |
| small | 0.00764 | **4.66 × 10⁻⁷** | 4 (5.8 × 10⁻⁷, bound 1.3 × 10⁻⁶) |

(The sizing's 9.03 × 10⁻⁷ and 5.09 × 10⁻⁷ came from 48 300 slots.)
Belief propagation's counted false decodes agree with the CRC-failure
estimate within their noise.

### Counted OSD-1 false decodes against the estimate

C = OSD runs (host: top 10, 69 000 069, 10.0 per slot; small: top 25,
172 499 999, 25.0 per slot). F_T = accepted codewords (non-zero, CRC,
plausible) at dH ≤ T; none duplicated a belief-propagation decode. The
bound is R_T = n × U(F_T) / C with n = max(C per slot, crowded n) =
14.5 (host) and 25.0 (small); "factor" is n × q_T × 2⁻¹⁴ × π_T with
π_T measured on these codewords' payloads.

| host, T | 19 | 20 | 21 | 22 | 23 | 24 | 25 | 26 |
|---|---|---|---|---|---|---|---|---|
| q_T (non-zero at dH ≤ T, per run) | 8.8e-6 | 3.0e-5 | 9.6e-5 | 3.0e-4 | 8.0e-4 | 2.2e-3 | 5.0e-3 | 1.2e-2 |
| CRC passing, counted (2⁻¹⁴ expects) | 0 (0.0) | 0 (0.1) | 0 (0.4) | 2 (1.3) | 3 (3.4) | 9 (9.4) | 19 (21.1) | 86 (49.6) |
| F_T counted | 0 | 0 | 0 | **2** | **2** | **3** | 5 | 53 |
| R_T bound (95%) | 6.3e-7 | 6.3e-7 | **6.3e-7** | 1.32e-6 | 1.32e-6 | 1.63e-6 | 2.2e-6 | 1.4e-5 |
| point n·F/C | 0 | 0 | 0 | 4.2e-7 | 4.2e-7 | 6.3e-7 | 1.1e-6 | 1.1e-5 |
| factor estimate | 2.8e-9 | 9.3e-9 | 3.1e-8 | 1.0e-7 | 2.6e-7 | 7.7e-7 | 1.7e-6 | 3.9e-6 |

| small, T | 19 | 20 | 21 | 22 | 23 | 24 | 25 | 26 |
|---|---|---|---|---|---|---|---|---|
| CRC passing, counted (2⁻¹⁴ expects) | 0 (0.1) | 0 (0.3) | 0 (1.0) | 2 (3.2) | 6 (8.4) | 21 (23.6) | 45 (53.0) | 204 (124.7) |
| F_T counted | 0 | 0 | 0 | **2** | **2** | **6** | 11 | 120 |
| R_T bound (95%) | 4.3e-7 | 4.3e-7 | **4.3e-7** | 9.1e-7 | 9.1e-7 | 1.72e-6 | 2.6e-6 | 2.0e-5 |
| point | 0 | 0 | 0 | 2.9e-7 | 2.9e-7 | 8.7e-7 | 1.6e-6 | 1.7e-5 |
| factor estimate | 4.8e-9 | 1.7e-8 | 5.4e-8 | 1.7e-7 | 4.5e-7 | 1.3e-6 | 2.9e-6 | 6.6e-6 |

**Decision (the rule as fixed): no T of 22, 23 or 24 qualifies for
either configuration; the largest T from 16 to 21 with R_T ≤ R0 is 21
for both: host T = 21 (R_21 = 6.3 × 10⁻⁷ ≤ 8.88 × 10⁻⁷), small T = 21
(4.34 × 10⁻⁷ ≤ 4.66 × 10⁻⁷, a narrow margin).** At 22 the counts (2 and
2) are not themselves above the current rate (point 4.2 × 10⁻⁷ host,
2.9 × 10⁻⁷ small), but 6.9 M slots cannot bound them below it; the
sizing's draft T = 24 is rejected outright (host bound 1.6 × 10⁻⁶,
small 1.7 × 10⁻⁶; small's point rate 8.7 × 10⁻⁷ is above its R0).

### What the counts say about the factor model

- **Up to dH 25 it holds.** The CRC passes at dH ≤ 24 are 9 against 9.4
  expected (host) and 21 against 23.6 (small): the 2⁻¹⁴ assumption
  holds there, and with π_T measured (0.37–0.39) the factor estimate
  predicts 3.7 (host) and 9.1 (small) accepted at dH ≤ 24 against 3
  and 6 counted. At dH ≤ 22 it predicts 0.5 and 1.2 against 2 and 2,
  within Poisson noise. The threefold under-prediction seen on the
  sizing's 48 300 slots does not reappear at dH ≤ 25 on fresh slots.
- **At dH 26 it fails, through one codeword.** One payload, `<...> <...>
  561202 JE52CU` (EuVhfContest, 0ce89161924b2492da25), is the best
  codeword at dH 26 36 times (host) and 85 times (small), mostly on
  strong and weak carriers: an attractor of structured inputs, not
  noise. With it, CRC passes at dH 26 are 67 against 28.5 expected
  (host). This is the low-dH tail's skew to EuVhfContest in the sizing;
  it sits above every T considered, but it is a structural risk the
  factor model cannot see, and the gate's empty slots must include
  carriers (they do).
- **π_T** (plausible share of the near codewords' payloads, CRC
  ignored) is 0.35–0.44 for T 16–30 (host and small alike), not the
  sizing's πU = 0.46. By type at dH ≤ 24: FieldDay, RTTY Roundup and
  reserved types never unpack; telemetry always; standard, EU VHF
  portable, non-standard call and DXpedition about 0.70; EU VHF contest
  0.52; free text 0.60. Reserved types are the most common (26%).
- **Syndromes:** each bit's share of ones is 0.47–0.53 at dH ≤ 24 (many
  standard deviations from 0.5) and the distribution over 256 groups is
  far from uniform (χ² 36 363 and 90 238 on 255 degrees of freedom); the
  syndrome is not uniform, but its zero value occurs at the 2⁻¹⁴ rate
  (above), which is what the false rate needs.
- By slot kind, the accepted codewords at dH ≤ 24 come from noise,
  carriers, clipped and floored slots; none from strong carriers or
  impulsive noise below dH 25.

### Not measured

- The rate on crowded slots (n = 14.5 is the crowded count; noise on a
  crowded slot is not white). Empty slots carry no gain information, so
  T = 21's gain is the sizing's (host 0.052 per slot, ≤ −17 dB 0.020;
  small 0.050, 0.006) and unconfirmed on a new block.

## OSD-1 follow-ups (2026-10-31; option off, identical)

On branch `osd1`. Commits 923c393 (`unpack` compiled once), 05fe78f
(storage, docs, checks, tests), 624c4de (bench: OSD-1 origin, `osdpath`,
`osdinput`), a7a9f6a (bench: OSD-1 runs per slot).

### Changes

- **`unpack` compiled once.** The public `unpack<H: CallHashes + ?Sized>`
  is a shim over one out-of-line `unpack_dyn(&Payload, &mut dyn
  CallHashes)`, reached through a sized forwarding wrapper (so `H = dyn
  CallHashes` still works; signature unchanged). The decoder's
  plausibility test no longer has its own `unpack::<NoHashes>` instance.
- **Belief propagation's round count** for an OSD-1 candidate lives in
  `Work::Osd`'s payload and is passed to `osd_decode`; `OsdScratch` has
  no `bp_rounds`. **Deviation from the requested form:** the payload is
  one struct, `Work::Osd(OsdWork { bp_rounds, osd })`, not the two-field
  variant `Work::Osd { bp_rounds, osd }`. With two variant fields (tuple
  or struct, `u8` or `u32`, `repr(u32)`, constant or not, out-of-line
  constructor) rustc 1.93 builds every `Work` value on the stack and
  copies it, OSD's and belief propagation's alike: `decode_a_slot`'s
  frame on the Cortex-M33 went from 2 512 to 6 096 B. With one field
  both switches fill the storage in place, as before.
- `Decode::hard_errors` documents the decoders' one difference (a soft
  bit of exactly 0: belief propagation never counts it, OSD-1 decides it
  0 and counts a codeword 1 there); `Decode::rounds` says an OSD-1
  decode carries the failed belief propagation's rounds. No behaviour
  change.
- `TABLE_BYTES` takes the generator rows from their type
  (`osd::ROWS_BYTES`, asserted 2 184 B); `N < 256` asserted for the radix
  sort's `u8` counts and positions; the check program's OSD images use
  (10, 21) host and (25, 21) small.
- Tests: with the option on, every candidate of a busy slot decoded
  right after a candidate OSD-1 ran on gives the result and round count
  fresh storage gives; an OSD-1 decode after one round of belief
  propagation (a −18 dB signal) carries `rounds = 1`.
- **Origin marker** (bench only; no `Decode` field): a decode whose
  candidate reported `Stage::Osd` is OSD-1's (it runs only after belief
  propagation failed); multi-pass runs count candidates from each
  `Refine` and join on the list entry's pass. Records save
  `"osd":true` on those decodes only, and the slot's OSD-1 runs as
  `"osd_runs"` when not 0; `compare` prints runs, gains and false
  decodes by origin. On `crowded` `main` (600 slots, host, `osd=10:21`,
  a smoke run on a used block, not evidence): 8 357 OSD-1 runs (13.9
  per slot; the sizing's 14.5 pooled), 19 OSD-1 decodes, 12
  messages gained through OSD-1 and 5 otherwise (knock-on), no false
  decode. The `cat-ft8-decode` profile line is unchanged: the binary has
  no OSD option, so its `osd_seconds` is always 0 and there is no decode
  to mark.
- Public API changes on the branch (043ee8a, noted in 05fe78f's
  message): `Stage::ALL` 4 → 5 (`Stage::Osd`; `Stage::Snr.index()` 3 →
  4), the `osd` stage in profile output and records (`osd_seconds`),
  `DecodeConfig::osd`, `with_osd` and `OsdConfig` new, the
  `profile`-only `bp_totals` returning an `Option`.

### Sizes (Cortex-M33 check images; option on or off alike)

| | 511bf32 | 2980198 | 923c393 (shim) | 05fe78f |
|---|---|---|---|---|
| flash, small | 86 468 | 93 888 | 90 540 | **90 564** |
| flash, host | 98 128 | 105 588 | 102 232 | **102 256** |
| flash, encoder alone | not measured | 35 528 | 35 676 | 35 676 |
| static RAM small / host | 212 044 / 241 612 | 212 048 / 241 616 | same | same |
| `osd_decode` frame | — | 488 | 264 | 272 |
| `_start` (decoder images / encoder) | 4 096 / — | 4 096 / 4 208 | 4 104 / 4 016 | 4 104 / 4 016 |
| `decode_a_slot` | 2 480 | 2 504 | 2 512 | 2 512 |
| `unpack` (its one instance) | — | 456 (`CallTable`) | 464 (`unpack_dyn`) | 464 |
| host `subtract_one` (largest) | 4 808 | 4 808 | 4 808 | 4 808 |

The OSD option now costs +4 096 B (small) and +4 128 B (host) of flash
against 511bf32 and 4 B of static RAM. The shim adds 8 B to `_start` and
`decode_a_slot` (the wrapper's slot), so the small image's largest frame
is 4 104 B, was 4 096 B.

### CPU of the shim (x86-64, 2980198 against 923c393)

- Decode instructions (`perf stat -e instructions:u`, `cat-ft8-decode`
  on the first 200 of the 800 WAV slots, summed): host 85 871 688 622 →
  85 871 584 480 (−0.0001%), small 22 296 547 181 → 22 296 556 416
  (+0.00004%). The dyn call is in `finish` (once per decode) only.
- `slotcost host` (one thread, 200 crowded 40-message slots, 5 runs,
  two rounds, each binary's two configurations): mean 72.4 / 72.1 /
  72.1 / 71.6 ms (base) against 72.1 / 72.6 / 72.2 / 72.3 ms (shim):
  +0.25 ms mean (+0.35%) within the base's own 0.8 ms spread.
  `slotcost` never unpacks, so this is placement, not the dyn call.
- Encoder (`encode_bench 200 50`, 3 rounds): total 559 / 562 / 559 ns
  per message against 559 / 543 / 567; no move beyond noise.
- **A76** (2026-11-01; aligned builds eed244c9… and 21151a8b…, run
  2980198, 923c393, 923c393, 2980198, one `slotcost host host 200 5`
  each, every run valid: sticky 0x50000, no live bit, one sample below
  2.39 GHz in the first run, peak 55.4 °C). The two builds' decode-path
  functions are identical (function by function, addresses and names
  aside: 70 of `cat_ft8`'s identical, the 18 that differ are `unpack`,
  its helpers and `osd_decode`), so the slotcost path is the same code
  and the median over both builds is the baseline. Three-pass slot
  (sum of the per-pass means): 2980198 107.11 / 107.11 / 107.17 /
  107.18 ms, 923c393 107.11 / 107.10 / 107.06 / 107.05 ms; median of all
  eight 107.11 ms; the shim's builds −0.06 ms (−0.05%) against the base's,
  placement only. Pass 3: 11.05–11.06 ms per new decode.

### OSD-1's cost per candidate through the decoder (624c4de)

Everything the option adds per OSD run: the storage switch and its
zero-fill, OSD-1, its tests, and the next candidate's fresh
belief-propagation storage.

- **x86-64** (Threadripper 3970X, one thread, plain build):
  `osdpath host 10:21 200 21` on the 600 pass inputs of the 200
  `slotcost` slots (crowded `main` 400–599; each pass's input as the
  decoder's own passes leave it, OSD off), 40 000 candidates; OSD-1 ran
  on 1 669 (pass 1 19, pass 2 259, pass 3 1 391: 8.3 per slot) and
  accepted 20. **Mean 13.59 µs per OSD run** (refill included); the
  1 649 not accepted: mean 13.63, median 13.41, largest 35.2 µs (a timing
  outlier: the M33 counts below vary by ±10%); the refill on the next
  candidate −0.45 µs mean (not measurable here); the 20 accepted 21.8 µs
  (with the CRC pass, `unpack`, re-encoding and the SNR). The library
  call alone (`osdlib host 200 5`, same session): 14.06 µs, on other
  inputs (all 32 481 failed candidates of every rank on the same slots,
  not the 1 669 top-10 runs), so the two are close but not like for
  like.
- **Cortex-M33** (emulated, out-of-tree image with the check program's
  build settings; `decode_candidate` on the first 12 candidates of each
  input, option on less option off): host (10, 21) on six inputs (slots
  400 and 401, passes 1–3, `osdinput host`): 15 OSD runs, none accepted,
  **mean 199 731 instructions per run (refill included), largest
  226 010** (the call 224 591 + the refill 1 419); `osd_decode` alone
  196 912 mean (181 624–221 772); the storage switch's zero-fill about
  2 400 and the refill 1 416–1 419. Small (25, 21) on the same slots'
  pass 1: 5 runs, mean 205 064, largest 213 535. `unpack` did not run
  (no CRC passed); on an accept it adds about 2 800 (`unpack_dyn` 2 827
  instructions on `CQ K1ABC FN42`). Candidates OSD did not run on differ
  by 0–3 instructions between the two images (placement).
- **A76** (2026-11-01; a7a9f6a, 64-byte-aligned measurement build,
  paired_bench sha256 f77b33c6…, core 3, the throttle sampled once a
  second: sticky 0x50000 throughout, no live bit; one sample below
  2.39 GHz, the first; peak 53.2 °C): `osdpath host 10:21 200 21` on the
  same 1 669 runs as x86-64: **mean 22.79 µs per OSD run (refill
  included)**; not accepted mean 22.87, median 22.91, **largest 26.69
  µs**; the refill −0.51 µs (not measurable); accepted 29.43 µs.
  `osdlib host 200 5` (two runs): library 23.16 / 23.17 µs, the other
  form 37.08, the sizing's 30.81 / 30.94; result hashes equal x86-64's
  (6fe3fb66…). The decoder-path figure and the library call are close
  (22.79 and 23.16 µs; e7e7f48's library 23.17), but they are taken on
  different inputs (the 1 669 top-10 runs against all 32 481 failed
  candidates), so this is not a like-for-like comparison of the two.

### Findings additions

- **q_T on crowded slots against empty, by pass** (the sizing's data:
  host, each pass's top 10, failed candidates off every message, channel
  soft bits; crowded/wide/fading × main/cap1/cap2 against the 21 empty
  blocks):

  | | runs | q_21 (count) | q_22 | q_24 |
  |---|---|---|---|---|
  | crowded, pass 1 | 1 588 | 0 (0) | 6.3e-4 (1) | 1.26e-3 (2) |
  | crowded, pass 2 | 24 450 | 1.64e-4 (4) | 4.1e-4 (10) | 2.49e-3 (61) |
  | crowded, pass 3 | 32 487 | 0.92e-4 (3) | 2.8e-4 (9) | 2.06e-3 (67) |
  | crowded, all | 58 525 | 1.20e-4 (7) | 3.4e-4 (20) | 2.22e-3 (130) |
  | empty (pass 1 only) | 483 000 | 0.95e-4 (46) | 2.9e-4 (141) | 2.28e-3 (1 103) |

  Small (top 25, one pass): crowded 0.95e-4 (4 of 41 970) against empty
  0.95e-4. The later passes' residual candidates sit no closer to a
  non-zero codeword than empty slots' (each pass within Poisson noise of
  the empty rate), which supports carrying the calibration's q_T, and so
  its bound, to passes 2 and 3. The counts on crowded slots are small
  (7 at T = 21), so this supports the carry-over, it does not prove it.
- **Assumed, not measured, on crowded slots:** that a near codeword of a
  pass 2 or 3 candidate passes the CRC at 2⁻¹⁴ and is plausible at
  π ≈ 0.37 as on empty slots; and that the on-message candidates (about
  4.75 per slot of the 14.5 failed top-10 candidates: 9.75 per slot are
  off every message) give a wrong codeword at no more than the
  off-message rate. Neither is counted; the gate's 0-false requirement
  on a fresh block is the check.
- **n = 14.5 is a mean.** Over three passes at most 30 candidates run
  (10 a pass); on 40-signal slots n reaches about 30, where the same
  calibration bound is 30 × 2.996 / 69.0 M = 1.30 × 10⁻⁶ per slot, above
  the current 8.9 × 10⁻⁷. The rule as fixed uses the mean, and the rate
  per slot averaged over a mix of slots is what it bounds.
- **Syndrome χ² deviation.** The calibration plan fixed χ² of the
  16 384 syndrome values against uniform; it was run over 256 groups of
  64 values (255 degrees of freedom), because at dH ≤ 24 there are 9.4
  codewords per value expected. Not deciding either way; declared here.
- **Shared events, host and small.** The calibration ran both
  configurations on the same slots, and some accepted codewords are the
  same event (same block, slot and payload): at dH ≤ 24, host 3 and
  small 6 events, 2 of them shared (7 distinct); at dH ≤ 22, 2 and 2,
  1 shared (3 distinct). Statements per configuration stand; pooled
  over both they count 7 (not 9) and 3 (not 4).

### Identity with the option off (x86-64, against 511bf32)

- **Records:** 73 800 (host, host one-pass and small × 24 600:
  crowded/wide/fading × main/cap1/cap2, sweep/edges/empty main) equal
  to 511bf32's (stage times aside), at 624c4de and at a7a9f6a; no
  record carries an `osd` mark or `osd_runs`.
- **A76 records** (a7a9f6a, the aligned build, all cores; 2026-11-01):
  the same 73 800 equal to x86-64's; throttle 0x50000 before and after.
- **800 WAV slots** × 3 configurations × 3 modes (7 200 runs, 95 448
  lines, 78 648 decode lines): byte-identical to e7e7f48's output, so
  equal to 511bf32's apart from the three memory fields noted there
  (`candidate_scratch_bytes`, `peak_bytes`, `static_table_bytes`; the
  scratch size is unchanged by `OsdWork`).

### Not measured

- The M33 cost on accepted candidates (none on the six inputs) and on
  more inputs than six pass inputs of two slots.
- The deviation above (`OsdWork`) has not been tried on other compiler
  versions; the stack check catches a regression.

## Pre-gate fixes (2026-11-01; option off, identical)

On branch `osd1`. Commits ef2470e (`cat-ft8-decode` takes `osd=C:T`),
8332aa5 (bench: `gains`, `BLOCK` for `slotcost` and `osdpath`, the
stricter false rule, the join asserted). No gate run, no block named.

### Changes

- `cat-ft8-decode --config host|small[,passes=N][,osd=C:T]`: the option
  on in the command, the output format unchanged; `osd=0:T` gives the
  plain output byte for byte (tested), and without the key the output is
  as before (below).
- `paired_bench gains OLDDIR NEWDIR [SETS]`: gained and lost messages
  per slot pooled over the sets, and at or below −17 dB (`sent.snr <=
  -17.0`, inclusive); gains OSD-1 made itself; the new run's false
  decodes the old run did not list in that slot. This is the committed
  tool for the gain bars.
- `compare`: with OSD-1 runs in the new run, any false decode the old
  run did not list fails the set, marked or not.
- `slotcost A B [SLOTS] [RUNS] [BLOCK]`, `osdpath CONFIG C:T [SLOTS]
  [RUNS] [BLOCK]`: the crowded set's BLOCK (default `main`, output
  unchanged for it).
- The records' origin join: each pass's count of candidates (from their
  `Refine` stage) is asserted equal to its kept candidates (an
  `assert_eq!`, not a `debug_assert!`: the bench runs only in release
  builds, where a debug assertion would never fire).
- Out of tree, pinned by hash in the plan: the classifier
  `classify_osd.py` (new file; the old `classify.py` unchanged) with the
  (o) OSD-decode and (s) pass-sequence knock-on classes and its tests
  (all pass); the M33 decoder-path harness (a manifest over its eight
  inputs, source and scripts; rustc 1.93.0, unicorn 2.1.0).

### Smoke runs (`main`, a used block: not evidence for the gate)

x86-64, 8332aa5's code, `run host` against `run host,osd=10:21` on
`crowded`, `wide` and `fading` `main` (2 000 slots), `paired_bench
gains` and `compare --partial`:

| set (main) | slots | gained (per slot) | ≤ −17 dB gained (per slot) | lost | gained through OSD-1 | OSD-1 runs / slot |
|---|---|---|---|---|---|---|
| crowded | 600 | 17 (0.0283) | 0 | 0 | 12 | 13.9 |
| wide | 800 | 51 (0.0638) | 34 (0.0425) | 0 | 46 | 14.7 |
| fading | 600 | 55 (0.0917) | 0 | 0 | 40 | 15.2 |
| **pooled** | 2 000 | **123 (0.0615)** | **34 (0.0170)** | 0 | 98 | 14.6 |

- Against the bars (0.026 per slot, 0.010 at or below −17 dB): pooled
  0.0615 and 0.0170, about 2.4× and 1.7× the bars. The expected gain is
  not near the bar on these slots, but `main` is the block the sizing
  chose the subset and threshold on, so a fresh block may give less; the
  ≤ −17 dB gains all come from `wide` (the only one of the three sets
  with messages below −16 dB: down to −22 dB). Crowded alone (0.028) is near 0.026, which
  is why the bar is on the pooled slots.
- No loss, no false decode in either run that the other did not list;
  every `compare` set passes. Classifier: 123 gains and 20 pass moves;
  (o) 115, (m) 3, (r) 4, (k) 21, (s) 0, unexplained 0; 28 changes sit
  within 50 Hz of an earlier marked decode.
- Marked decodes (115) by pass: 10, 57 and 48; the 67 in passes 1 and 2
  are subtracted before the next pass.

### OSD-triggered subtraction, in R's numerator

One subtraction on the A76 (2980198-al `slotcost host`, main): the
`subtract` stage 56.62 ms per slot over 34.5 subtractions, **1.64 ms
each**. On the smoke runs that adds 1.64 × 67 / 2 000 = 0.055 ms per slot
to OSD-1's 22.8 µs × 14.62 = 0.333 ms: R = 0.388 / 0.0615 = **6.3 ms per
new decode**, against pass 3's 11.06 ms (A76, `main`). The later passes'
other extra work on the changed residual is not in R (it is inside the
1.0 ms slot cap).

### Identity with the option off (x86-64, 8332aa5 against 511bf32)

- Records: 73 800 (host, host one-pass, small) equal to 511bf32's, stage
  times aside; none carries an `osd` mark or `osd_runs`.
- 800 WAV slots × 3 configurations × 3 modes (`--config host`, `small`,
  `host,passes=1`; plain, `--raw`, `--profile`; 7 200 runs, 78 648 decode
  lines): byte-identical to e7e7f48's output (511bf32's apart from the
  three memory fields noted there). No option-on WAV output has been
  made; the matching rule is in the plan first.

## OSD-1 gated round on `gosd1` (2026-11-01): result

Run once, as written in the plan at 7408489 (binaries built from that
commit: x86-64 `paired_bench` 50cb78d9…, `cat-ft8-decode` debd2936…; A76
aligned `paired_bench` 21a7c789…; the M33 harness manifest checked OK).
Host, `OsdConfig::new(10, 21)` against OSD off, same commit, same run.
Nothing was retuned or re-run. Results verbatim; the verdict is applied
mechanically below.

| criterion | measured | bar | result |
|---|---|---|---|
| G2 false decodes in the new run absent from the old (records: gain block, 20 empty blocks; WAV) | 0 (records, every set and pass); WAV: 0 unmatched added | 0 | **pass** |
| G2 pooled bound | 0 counted in the calibration's 69.0 M runs and this round's | ≤ current rate | **pass** |
| Gain, pooled crowded/wide/fading (2 000 slots, `gains`) | 124 new decodes, **0.0620 per slot** (crowded 18, 0.0300; wide 59, 0.0737; fading 47, 0.0783; 101 of the 124 made by OSD-1 itself) | ≥ 0.026 | **pass** |
| Gain at or below −17 dB, pooled | 42, **0.0210 per slot** (all from `wide`) | ≥ 0.010 | **pass** |
| Sweep 50% point | −19.150 → −19.202 dB, shift **−0.052 dB** [−0.070, −0.038] (better) | not worse than 0.05 dB | **pass** (criterion as worded) |
| `edges` no loss | lost 0 (gained 17) | 0 | **pass** |
| G7 `compare` no-loss gates (per point, per set) | b = 0 in every point and set | b − c ≤ 3√, ≤ 2√ | **pass** |
| G7 losses per set (`gains` per-set lines) | 0 in every set (crowded, wide, fading, sweep, edges, 20 empty) | ≤ ⌊0.1 G⌋ | **pass** |
| G7 classifier (`classify_osd_v2.py`) | 267 gains, 27 moves, 0 losses: (o) 270, (m) 1, (r) 2, (k) 21, (s) 0; **unexplained 0** | 0 | **pass** |
| `compare --partial` per-set verdicts | 23 of 25 PASS; **crowded FAIL, sweep FAIL** (below) | every set | **FAIL** |
| WAV, option off | the 800 slots' `--config host` output byte-identical to the base binary's | identical | **pass** |
| WAV, option on | 12 added decodes, **12 matched** in the pinned reference output, 0 unmatched, 0 lost | 0 unmatched, 0 lost | **pass** |
| G1 R, cost per new decode (A76) | (22.662 µs × 14.2765 runs + 1.640 ms × 0.0405 subtracted marked decodes) per slot = 0.390 ms ÷ 0.0620 = **6.29 ms** | < pass 3's **10.74 ms** (side A) | **pass** |
| G1 slot up by (A76, `slotcost host host,osd=10:21 200 5 gosd1`) | A mean 107.1 ms, B 107.3 ms: **+0.2 ms** | ≤ 1.0 ms | **pass** |
| G1 OSD-1's own time per slot (B's `osd` stage) | **0.20 ms** | ≤ 0.5 ms | **pass** |
| G4 M33 instructions per OSD run (through the decoder) | mean **199 731**, max **226 010** (224 591 + 1 419) | ≤ 0.21 M, ≤ 0.235 M | **pass** |
| G4 static RAM / frames / FMA / f64 / `check.sh --stack` | 212 048 and 241 616 B (+4); largest frames 4 808 (host `subtract_one`), `_start` 4 104, `osd_decode` 272; no FMA in the four decoder images; no `f64` helper; OK | +4 B; ≤ 4 808 / 4 104; ≤ 288; none; OK | **pass** |
| G5 flash | host 102 256 B, small 90 564 B | ≤ 102 400, ≤ 90 700 | **pass** |

Both A76 runs were valid (sticky 0x50000 throughout, no live bit, no
sample below 2.39 GHz, peaks 52.7 and 53.2 °C). Side A's figures used
in R: `subtract` 56.58 ms over 24.0 + 10.5 subtractions per slot (1.640
ms each); pass 3 32.77 ms, 3.0 new decodes, 10.74 ms per new decode.
`osdpath host 10:21 200 21 gosd1`: 1 648 OSD runs, 22.662 µs per run
(not accepted 22.78 mean, 25.87 max). Records, pooled gate slots: 28 553
OSD-1 runs (14.28 per slot); marked decodes by pass 16, 65, 46 (81 in
passes 1–2, 0.0405 per slot). 24 changes sit within 50 Hz of an earlier
marked decode (reported, not a condition).

**The two `compare` failures, as the tool reports them (not adjudicated):**

- **crowded:** the multi-pass CRC-expectation check (`passes: expected
  false decodes 1.93e-6 per slot (limit 3 × 6.10e-7): CRC`). The two runs
  have the same CRC failures (19, 0.0317 per slot) and the same
  expectation (1.93 × 10⁻⁶): the check compares the new run's three
  passes against three times the old run's pass-1 rate, which the OSD-off
  run on this block exceeds by itself. OSD-1 did not change it.
- **sweep:** the two-sided shift check (`shift -0.052 dB [-0.070,
  -0.038]: SHIFT`, limit ±0.05 dB): the 50% point moved 0.052 dB
  **lower** (OSD-1 decodes weaker signals). The plan's criterion is "not
  worse than the existing shift gate (0.05 dB)"; the tool's check is
  symmetric.

**Verdict, applied mechanically: FAIL.** The plan names the per-set gates
as `compare --partial`'s, and two of its 25 set verdicts are FAIL. Every
other criterion passes, including every criterion stated as a number in
the plan. Neither failing check is one the plan states separately (the
plan's own false-decode criterion is the count, 0, and its sweep
criterion is one-sided), so whether these two tool verdicts were meant
to gate is a reading of the plan, not a measurement; it is left to the
user. Nothing merges on this result; no code is deleted; no retune and
no second block.

## The two compare checks fixed (2026-11-01; tool only, no gate run)

Commit 7ccbd48 (`paired_bench compare`). The `gosd1` result above stands
as FAIL; this is for the next round.

- **Multi-pass CRC check.** Was: the new run's CRC expectation summed
  over its passes at most (passes × the old run's pass-1 expectation).
  On `gosd1`'s crowded set a three-pass run exceeds that against itself
  (19 CRC failures, 1.93 × 10⁻⁶ against 3 × 6.10 × 10⁻⁷). Now, when the old
  run is multi-pass, the gate is the two runs' CRC failures over the same
  passes: the new run's at most 3·√(old + new) above the old's (the
  counts' Poisson spread, as the loss gates); the absolute limit is still
  printed, marked "reported". Against a single-pass old run the absolute
  limit is still the gate and the output is unchanged.
- **Sweep shift.** Was and still is by default: the shift and both ends
  of its bootstrap interval within ±0.05 dB. With `compare --gain` (a
  round meant to change results) the gate is one-sided: the shift and
  the interval's upper end at most +0.05 dB; the two-sided result is
  still printed. (The module docs had called the default one-sided; it
  never was, and they now say so.)

**On the `gosd1` records** (`compare … --partial`, x86-64):

| pair | old tool (7408489) | fixed tool | fixed tool, `--gain` |
|---|---|---|---|
| OSD off vs OSD off | FAIL (crowded: CRC) | **PASS** | PASS |
| OSD on vs OSD on | FAIL (crowded: CRC) | **PASS** | PASS |
| OSD off vs OSD on | FAIL (crowded: CRC; sweep: SHIFT) | FAIL (sweep: SHIFT) | **PASS** |

Crowded, fixed tool: "CRC failures over passes 1–3: old 19, new 19, new −
old 0 (limit 18.5): ok"; sweep with `--gain`: "shift −0.052 dB [−0.070,
−0.038]: SHIFT (two-sided); gain round, not worse by more than 0.05 dB:
ok".

**Earlier data.**
- Every saved `compare` output of earlier rounds in the local cache (57
  logs: the E1–E4, S0–S3b, O3, O8 and O9 comparisons) was replayed with
  both tools. For 56 a pair of run directories reproduces the saved
  per-set and overall verdicts with the old tool, and the fixed tool
  gives the same verdicts for every one (no difference). One (E2's small
  rule comparison) could not be matched to its directories and is not
  replayed. (Most of these runs are single-pass, so the CRC change does
  not apply to them; none was run with `--gain`.)
- Every run directory in the cache against itself (283 directories,
  `--partial`): 280 PASS with both tools, 3 unreadable by both (an early
  record format). No earlier block shows the self-failure; `gosd1`'s
  crowded set is the first.
- So neither change alters an earlier verdict; the only verdicts they
  change are the two kinds above (a configuration failing the absolute
  CRC limit against itself, and a sweep shift that is an improvement in
  a `--gain` comparison).

## compare's gates relative to the old run (2026-11-01; tool only, no gate run)

Commit 689a361, after a review of 7ccbd48. The `gosd1` result stands as
FAIL.

- **CRC gate, paired by slot.** Against a multi-pass old run: per slot,
  the CRC failures over every pass each run ran; `up` sums the new run's
  excess, `down` the old's; fail when up − down > 2·√(up + down). Stricter
  than 7ccbd48's 3·√(old + new) on the totals (up + down ≤ old + new; a
  run against itself has up = down = 0). The absolute limit is printed;
  both runs' pass counts are printed when they differ, and each run's
  own passes are all counted.
- **False-decode checks relative to the old run.** On `empty` (per point)
  and in a later pass (per pass), a new false decode fails only if the
  old run did not list that payload in that slot; the absolute counts
  stay in the tables. This is G2's rule; a configuration can no longer
  fail against itself through them.
- `print_passes`' doc corrected; unit tests for `false_absent`,
  `crc_paired` and `shift_gate` (the bench's tests now run under `cargo
  test --features profile`).

**On `gosd1`** (fixed tool, `--partial`): OSD off vs off PASS; on vs on
PASS; off vs on FAIL (sweep only, two-sided) and PASS with `--gain`.
Crowded's line: "CRC failures, every pass, paired by slot: old 19, new 19;
new above old 0, old above new 0 (limit 0.0): ok". This reading of
`gosd1` is exploratory; the round's result there is its recorded FAIL.

**Earlier data, re-paired.** The saved comparisons were re-paired by
their text: a pair of run directories is accepted when the old tool
(7408489) reproduces the log's set headers and per-point table rows
exactly (the version-stable part; the multi-pass per-pass rows and
stage times aside); candidates are the directories in the log's own tree
whose decoded counts per (set, block, point) equal the log's `dec_old`
and `dec_new` columns. 56 of the 57 logs pair and give the same per-set
and overall verdicts under the fixed tool; 1 (`e0-176a49b`'s host vs
small) does not pair (its records are in an early format neither tool
reads). The earlier pairing (findings, "The two compare checks fixed")
was looser and paired four of the six multi-pass comparisons wrongly;
the corrected pairings of those six (old → new), all the same verdict
with the fixed tool, the multi-pass rule exercised:

| saved comparison | old run | new run |
|---|---|---|
| S3 acceptance | `s3/base` | `o1b/ident-c3f080f/host` |
| S3b acceptance | `s3/base` | `o1b/ident-c3f080f/host` |
| O8 cap 50 | `o1b/ident-c3f080f/host` | `o8/accept/host` |
| O8 single pass | `s3/base` | `o8/accept/host` |
| O3 `i16` prototype | `o1b/ident-c3f080f/host` | `o3/proto-i16/host` |
| O9 | `o9/eff/bafed37/host` | `o9/eff/ec72aa0/host` |

(Where several directories hold identical records, the first that
reproduces the text is listed; any of them gives the same verdicts.)
Every run directory against itself with the fixed tool: 280 PASS, the
same 3 unreadable as before.
