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
their text: a pair of run directories was accepted when the old tool
(7408489) reproduced the log's set headers and per-point table rows
(the version-stable part of the text; the older logs' set and per-pass
lines were written by earlier versions of `compare`, so they are not
compared), the candidates being directories in the log's own tree whose
decoded counts per (set, block, point) equal the log's `dec_old` and
`dec_new` columns. Where several directories hold identical records,
that method cannot tell them apart, and for three multi-pass logs it
picked the wrong old run; the pairings below are the true ones (checked
by a full-text pairing in review, and re-run here: the old and fixed
tools give the same verdicts, PASS, and the per-point rows match the
saved logs). 56 of the 57 logs pair and keep every verdict under the
fixed tool; 1 (`e0-176a49b`'s host vs small) does not pair (an early
record format neither tool reads).

| saved comparison | old run | new run | both runs multi-pass |
|---|---|---|---|
| S3 acceptance | `s3/base` | `s3/accept` | no (old single-pass) |
| S3b acceptance | `s3b/base` | `s3b/accept` | no (old single-pass) |
| O8 single pass | `s3/base` | `o8/accept/host` | no (old single-pass) |
| O8 cap 50 | `o1b/ident-c3f080f/host` | `o8/accept/host` | yes |
| O3 `i16` prototype | `o3/ident-043e7d8/host` | `o3/proto-i16/host` | yes |
| O9 | `o9/eff/bafed37/host` | `o9/eff/ec72aa0/host` | yes |

So the paired CRC rule (both runs multi-pass) is exercised by three
earlier comparisons only, O8 cap 50, O3 and O9, and passes on each; the
other multi-pass logs compare against a single-pass old run, where the
absolute limit still applies, unchanged.
Every run directory against itself with the fixed tool: 280 PASS, the
same 3 unreadable as before.

## Second OSD-1 gated round on `hosd2` (2026-11-01): result

Run once each, as written in the plan at 037ed44 (CODE 689a361; the
scripts logged their own and every manifest's sha256, all equal to the
pins; inputs, binaries, env and the 800 WAVs checked; A76 aligned
`paired_bench` d91cb36a…). Host, `OsdConfig::new(10, 21)` against OSD
off. Nothing retuned or re-run. Results verbatim.

| criterion | measured | bar | result |
|---|---|---|---|
| G2 new-run false decodes the old run lacks (records: `hosd2` sets, 20 empty blocks; WAV) | 0 in every set and pass; WAV 0 unmatched | 0 | **pass** |
| G2 pooled (gosd1 + hosd2 + the calibration) | 0 counted | ≤ current rate | **pass** |
| Gain, pooled crowded/wide/fading (2 000 slots) | 129 new decodes, **0.0645 per slot** (crowded 10, 0.0167; wide 56, 0.0700; fading 63, 0.1050; 100 made by OSD-1 itself) | ≥ 0.026 | **pass** |
| Gain at or below −17 dB, pooled | 38, **0.0190 per slot** (all from `wide`) | ≥ 0.010 | **pass** |
| Sweep 50% point (`--gain`) | −19.096 → −19.171 dB, shift **−0.075 dB** [−0.103, −0.053]; two-sided SHIFT, one-sided ok | not worse by > 0.05 dB | **pass** |
| `edges` no loss | 0 lost (12 gained) | 0 | **pass** |
| G7 `compare` loss gates | b = 0 in every point and set | ≤ 3√ / ≤ 2√ | **pass** |
| G7 losses per set (`gains` lines) | 0 in every set | ≤ ⌊0.1 G⌋ | **pass** |
| G7 classifier v2 | 274 gains, 13 moves, 0 losses: (o) 256, (m) 0, (r) 3, (k) 28, (s) 0; **unexplained 0** | 0 | **pass** |
| `compare --partial --gain` | **25 of 25 sets PASS**, overall PASS; paired CRC up/down 0/0 in every set but sweep (2/0, limit 2.8) | every set | **pass** |
| WAV, option off | byte-identical to the base (800 slots) | identical | **pass** (reproduces gosd1) |
| WAV, option on | 12 added, 12 matched, 0 lost | 0 unmatched, 0 lost | **pass** (reproduces gosd1) |
| G1 R (A76, side A of the slotcost run) | (22.942 µs × 14.46 runs + 1.641 ms × 0.0345) per slot = 0.388 ms ÷ 0.0645 = **6.02 ms** | < pass 3's **10.59 ms** | **pass** |
| G1 slot up by | A 106.9 → B 107.1 ms: **+0.2 ms** | ≤ 1.0 ms | **pass** |
| G1 OSD-1's own time per slot (B's `osd` stage) | **0.19 ms** | ≤ 0.5 ms | **pass** |
| G4 M33 per OSD run | mean **199 731**, max **226 010** | ≤ 0.21 M, ≤ 0.235 M | **pass** (reproduces gosd1) |
| G4 RAM / frames / FMA / f64 / `check.sh --stack` | +4 B; 4 808 / 4 104 / `osd_decode` 272; no FMA; no f64; OK | as pinned | **pass** (reproduces gosd1) |
| G5 flash | 102 256 / 90 564 B | ≤ 102 400 / ≤ 90 700 | **pass** (reproduces gosd1) |

**A76 validity (0x0 baseline):** both runs valid. Pre-checks read 0x0
before each; every throttle sample read 0x0 (slotcost 236 samples,
osdpath 262; one slotcost sample at 1.6 GHz, the first, clock not being
a criterion); peak 53.2 °C. Side A: `subtract` 56.45 ms over 23.8 + 10.6
subtractions per slot (1.641 ms each); pass 3 32.89 ms, 3.1 new decodes,
10.59 ms per new decode. `osdpath … hosd2`: 1 646 runs, 22.942 µs per
run (not accepted mean 23.05, max 26.19 µs). Records, pooled: 28 920
OSD-1 runs (14.46 per slot); marked decodes by pass 11, 58, 42 (69 in
passes 1–2, 0.0345 per slot). 31 changes sit within 50 Hz of an earlier
marked decode (reported).

**Verdict, applied mechanically: PASS.** Every criterion passes. The
fresh evidence is the `hosd2` records and the A76 runs on its slots; the
WAV, M33, flash, RAM and stack criteria reproduce `gosd1`'s (no code
changed). The `gosd1` FAIL stays recorded; this result is the one retry
the user allowed.

## OSD-1 in the HOST preset (2026-11-01, 748f34d)

`DecodeConfig::HOST` sets `osd: Some(OsdConfig::new(10, 21))`, as gated;
`SMALL` sets `osd: None`. Checks at 748f34d (x86-64, binaries built from
the commit; release `cat-ft8-decode` sha256
`d7bba820812acaa16c4881d44da7e0c6c58e003e71955c950e3f7b932571a434`):

- `paired_bench run host` on the gate blocks equals each gate's OSD-on
  (`host,osd=10:21`) records: `hosd2` and its 20 empty blocks, 64 300
  records equal; `gosd1` and its 20, 64 300 equal (stage times aside).
- `run small` on the identity sets (crowded/wide/fading × main/cap1/cap2,
  sweep/edges/empty main): 24 600 records equal to 511bf32's;
  `host,osd=0:21` and `host,passes=1,osd=0:21`: 24 600 each equal to
  511bf32's host and one-pass records.
- The 800 WAV slots: `--config small` and `--config host,osd=0:21`
  against the base binary's `--config small` and `--config host`
  (2980198's build, the output 511bf32's), plain, `--raw` and `--profile`
  (timings removed): 4 800 runs, all byte-identical.
- Full local CI passed before the commit; the check images' sizes are
  unchanged (102 256 / 90 564 B).

## OSD-1 enable follow-ups (2026-11-01, 8303c71; no decoder change)

- The OSD measurement tools force the decoder's own OSD off (HOST has it
  on since 748f34d). Re-run with the old and new tools on a small check:
  `osdcal host 10 … qv 0 1` (4 600 empty slots) gives the same JSON
  (provenance and times aside; the old tool was the calibration's build,
  e7e7f48); `osd host OUT crowded main` gives the same three output files
  as 2980198's build.
- `osd=off` in both CLIs; zero candidates parse to `None`.
- The 800 WAV slots, found and checked: they are synthetic, written by a
  small generator outside the repository that uses this crate's encoder
  and `synthesize`: for slot i, 0 messages if i mod 8 = 0, 1 if 1 or 2,
  5, 10, 20, then 40 for 6 and 7; standard messages (CQ with a grid, a
  report, RR73) among twelve calls and eight grids; one-signal slots at
  −24 to −12 dB, the others −16 to +6 dB; 200–2 900 Hz, start −1 to +1.5
  s; white Gaussian noise of RMS 0.03 from a seeded generator; every 13th
  slot cut to 150 000 samples; 12 kHz 16-bit mono. Re-running the
  generator now reproduces all 800 files byte for byte (combined sha256
  `3cb4521f…`, the gate's pin).
- **Sparse-slot CPU, A76** (748f34d's `cat-ft8-decode`, aligned
  measurement build, core 3, the throttle sampled once a second: 59
  samples, every one 0x0, peak 50.5 °C): the 300 WAV slots with no signal
  or one, `--config host` against `--config host,osd=0:21` alternating
  per file, three rounds, the profile's in-process `total_seconds` (per
  slot the median of three): **30.553 → 30.946 ms, +0.393 ms per slot
  (+1.3%)**, median +0.283 ms; the OSD stage 0.325 ms per slot. Against
  the crowded set's 40-signal slots (+0.2 ms on 107 ms, the gate) OSD-1
  costs more on sparse slots, where more of the best candidates fail
  belief propagation. The x86-64 figure on 19 off-air recordings is the
  review's (+0.47 ms per slot, +1.4%, one core, alternating runs).
- At 8303c71 (x86-64): `host,osd=off` and `host,passes=1,osd=off`
  records on the identity sets equal 511bf32's host and one-pass records
  (24 600 each); the 800 WAV slots with `--config host,osd=off` in plain,
  `--raw` and `--profile` mode (timings removed) are byte-identical to
  the base binary's `--config host` (2 400 runs). Release
  `cat-ft8-decode` sha256
  `e87c98c311ad0e6c8185172fab63c6d68b720dc7e59def9b1234bfcfad7a2d22`.

## Profile after OSD-1 (2026-11-01; measurement only)

On the local branch `prof14` from 26e057a (OSD-1 at (10, 21) in
`DecodeConfig::HOST`, off in `SMALL`). No decoder change; every tool
below is out of tree. Purpose: where the CPU and memory per decoded
signal go now, on crowded, wide, fading, sparse (0–1 signal) and sweep
slots, and what to cut next.

### Method

- **A76** (Raspberry Pi 5): 64-byte-aligned measurement builds
  (`-align-loops=64 -align-all-functions=6`) of 26e057a and 748f34d, the
  two the same code: all 120 `cat_ft8` functions of their AArch64
  `paired_bench` identical, addresses and names aside (`paired_bench`
  sha256 5eb4624b… and ca900d6a…). One benchmark at a time on core 3, the
  throttle, clock and temperature sampled once a second on core 0; a run
  starts only on 0x0. **Every timed run is valid:** 13 runs, 5 741
  samples, every one 0x0 (start and end checks too); one sample below
  2.39 GHz in three runs, each the first second; peak 54.9 °C.
  - `slotcost host host,passes=1 200 5` four times (the two builds
    alternated), `slotcost small small 200 5` once on each,
    `slotcost host,osd=off host 200 5` once: crowded 40-message slots
    (`main` 400–599). Medians over the runs; the four three-pass slot
    means are 107.4 / 107.5 / 107.4 / 107.4 ms, every stage's spread at
    most 0.09 ms.
  - Per set: `paired_bench run CONFIG DIR crowded,wide,fading,sweep,edges,empty`
    on one core (so one thread), its per-slot stage times (the decoder's
    stage callbacks). Host on 26e057a, and on 748f34d without the sweep
    (the two agree within 0.04 ms a slot on every set); small on
    26e057a. Single runs per slot, so per-set figures carry more noise
    than the `slotcost` medians.
  - A sampled profile (out-of-tree SIGALRM sampler every 487 µs with
    unwound stacks and line tables, as in O13) of the three-pass host
    `slotcost` (242 090 decoder samples) and of `small` (40 087). The
    sampler costs about 2.5% (110.0 against 107.4 ms). Its shares are
    used only to split a stage; stage totals are the timed ones.
  - No `perf` on the Pi and nothing installed, so no hardware counters
    this round.
- **x86-64** (Threadripper 3970X, core 5): aligned builds of the same two
  commits (116 `cat_ft8` functions identical; 5 `Debug` formatters
  differ), the same `slotcost` runs, and `perf record -e cycles:u`
  (user-space cycles) of `slotcost host host 200 5` and
  `slotcost small small 200 5`. **The x86-64 host is a virtual machine
  whose clock source is `acpi_pm`: one clock read costs about 6.7 µs.**
  The bench reads the clock once per stage event (about 700 a crowded
  three-pass slot), so x86-64 stage times carry about 6.6 µs per event
  (refine, 200 events a slot, reads 1.32 ms on x86-64 against 0.09 ms on
  the A76) and the slot about 4.6 ms. x86-64 stage shares below come from
  `perf`, not from the stage times; the x86-64 per-set runs were stopped
  for this reason.
- **Cortex-M33** (emulated, unicorn 2.1.0, the check program's build
  settings, `opt-level = "s"`, LTO): measurement images rewritten out of
  tree to decode a WAV slot with `decode_passes_staged` on an `f32` copy
  (an emulator-only region; 720 KB does not fit the part) and to write
  each pass event to a marker word, so instructions are counted per
  stage exactly as the A76 callbacks time them. Small (OSD off, one
  pass), host (OSD-1 on, three passes) and host with OSD off, on seven
  synthetic WAV slots (0, 1, 5, 10, 20, 40, 40 messages: s0080, s0081,
  s0083–s0087). Stack high water from painted RAM. These images differ
  from the check images (the staged entry point, `feed` instead of
  `feed_i16`, and the small image links the subtraction: flash 102 084
  B against 90 564), so their sizes are not the check images'; sizes
  below are from `check.sh --stack` at 26e057a.
- **Host memory:** an out-of-tree example with a counting global
  allocator and a painted thread stack around `decode_passes_staged` and
  `finish`, buffers made first as the CLI makes them (x86-64; s0080,
  s0081, s0084, s0086, s0087; host, host OSD off, small).

### A76: crowded 40-message slots (`slotcost`, medians of four runs; small two)

| stage (ms per slot) | host, three passes | share | pass 1 | pass 2 | pass 3 | host, one pass | small, one pass | share |
|---|---|---|---|---|---|---|---|---|
| feed (front end, FFT) | 22.10 | 20.6% | 7.37 | 7.37 | 7.36 | 7.43 | 7.31 | 47.6% |
| sync | 4.68 | 4.4% | 1.54 | 1.56 | 1.57 | 1.54 | 1.38 | 9.0% |
| refine | 0.09 | 0.1% | 0.04 | 0.02 | 0.02 | 0.04 | 0.03 | 0.2% |
| soft bits | 2.38 | 2.2% | 1.19 | 0.60 | 0.59 | 1.19 | 0.90 | 5.9% |
| belief propagation | 21.00 | 19.6% | 9.86 | 5.12 | 6.02 | 9.87 | 5.59 | 36.4% |
| OSD-1 | 0.20 | 0.2% | 0.00 | 0.03 | 0.16 | 0.00 | — | — |
| SNR | 0.24 | 0.2% | 0.15 | 0.07 | 0.02 | 0.15 | 0.15 | 1.0% |
| subtract | 56.70 | 52.8% | — | 39.50 | 17.20 | — | — | — |
| **slot** | **107.4** | | **20.16** | **54.26** | **32.95** | **20.2** | **15.35** | |
| new decodes | 37.6 | | 24.1 | 10.5 | 3.0 | 24.1 | 24.0 | |
| **ms per new decode** | **2.86** | | **0.84** | **5.17** | **10.98** | 0.84 | **0.64** | |
| candidates through BP | 200 | | 100 | 50 | 50 | 100 | 75 | |
| subtractions | 34.6 | | | 24.1 | 10.5 | | | |

- Worst slot: 115.2 ms (slot 492) three passes, 20.9 ms one pass,
  16.0 ms small.
- OSD-1 on against off on the same slots: 107.4 against 107.1 ms
  (+0.3 ms, of which the OSD stage 0.20 ms); 37.6 against 37.5 new
  decodes.
- Unchanged since O13b within 0.3 ms per stage (subtract 56.58 → 56.70,
  BP 21.30 → 21.00); passes cost 0.84 / 5.17 / 10.98 ms per new decode
  (f8f2a7b: 0.84 / 5.20 / 11.09).

### A76: every set (host three passes; small one pass; ms per slot)

| set | slots | sent | decoded | slot | feed | sync | soft bits | BP | OSD | subtract | ms per decode | small: slot | small: decoded | small: ms per decode |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| crowded (10, 20, 40) | 600 | 23.3 | 22.43 | 86.2 | 21.55 | 4.58 | 2.33 | 22.29 | 0.33 | 34.87 | 3.84 | 16.04 | 16.09 | 1.00 |
| crowded 10 | 200 | 10 | 9.97 | 66.5 | 20.32 | 4.35 | 2.23 | 22.73 | 0.42 | 16.31 | 6.67 | 16.74 | 8.77 | 1.91 |
| crowded 20 | 200 | 20 | 19.74 | 84.5 | 22.16 | 4.72 | 2.38 | 23.12 | 0.38 | 31.49 | 4.28 | 16.07 | 15.49 | 1.04 |
| crowded 40 | 200 | 40 | 37.57 | 107.6 | 22.17 | 4.68 | 2.38 | 21.03 | 0.20 | 56.80 | 2.86 | 15.32 | 24.02 | 0.64 |
| wide (10–40, −22…+5 dB) | 800 | 25.0 | 21.31 | 85.2 | 21.67 | 4.61 | 2.34 | 22.56 | 0.35 | 33.46 | 4.00 | 16.09 | 15.57 | 1.03 |
| fading | 600 | 23.3 | 20.08 | 83.3 | 21.62 | 4.59 | 2.33 | 22.66 | 0.36 | 31.56 | 4.15 | 16.25 | 14.71 | 1.10 |
| sweep (1 signal, −22…−15 dB) | 15 000 | 1 | 0.59 | 33.7 | 11.77 | 2.53 | 1.54 | 16.48 | 0.36 | 0.98 | 57.0 | 17.46 | 0.58 | 30.1 |
| edges (1 signal, −18 dB) | 1 300 | 1 | 0.96 | 40.0 | 14.34 | 3.09 | 1.75 | 18.77 | 0.44 | 1.55 | 41.8 | 17.37 | 0.80 | 21.8 |
| empty (0 signals) | 2 300 | 0 | 0 | 23.0 | 7.39 | 1.59 | 1.19 | 12.58 | 0.24 | 0 | — | 17.38 | 0 | — |

(Refine 0.03–0.09 ms and SNR at most 0.24 ms everywhere; host figures the
median of the two builds where both ran. Per pass, host, ms and new
decodes per slot: crowded 21.16 / 16.12, 42.02 / 5.08, 23.01 / 1.22;
wide 21.22 / 15.63, 41.26 / 4.71, 22.73 / 0.97; fading 21.37 / 14.77,
39.92 / 4.40, 22.03 / 0.91; edges 23.06 / 0.96, 16.97 / 0; sweep 23.16 /
0.59, 10.56 / 0; empty 23.02 / 0.)

**Sparse slots.** An empty slot costs 23.0 ms (host) and 17.4 ms
(small): belief propagation on 100 (75) candidates that all fail after
30 rounds is 55% (45%) of it, the feed 32% (42%). A slot with one decoded
signal costs 40 ms on the host: pass 2 runs (feed, sync, 50 candidates)
for 17.7 ms and, over the 10 125 such slots of `edges` and `sweep`,
**decoded nothing** (pass 2 added 0 decodes in every one). Pass 2 is 44%
of a one-signal slot.

**Later passes by the previous pass's yield** (host, crowded + wide +
fading, 2 000 slots):

| | previous pass's new decodes | slots | cost when run (ms) | new decodes per slot | ms per new decode |
|---|---|---|---|---|---|
| pass 3 | 1 | 326 | 17.8 | 0.03–0.11 (by set) | 162–526 |
| pass 3 | 2 | 263 | 19.4 | 0.17–0.27 | 71–117 |
| pass 3 | 3–5 | 525 | 22.3 | 0.50–0.67 | 34–45 |
| pass 3 | 6–10 | 573 | 29.4 | 1.9–2.5 | 12–16 |
| pass 3 | > 10 | 163 | 35.3 | 2.2–3.2 | 11–16 |
| pass 2 | 6–10 (the 10-message slots) | 601 | 29.7 | 1.11–1.14 | 26–27 |
| pass 2 | > 10 | 1 392 | 46 | 6.0–7.1 | 6.8–7.6 |
| pass 2 | 1 (`edges`, `sweep`) | 10 125 | 17.7–17.8 | 0 | — |

Pass 1 is 0.84–2.8 ms per new decode on these sets.

**Where belief propagation goes:** over 99% of its rounds are spent on
candidates that fail (decoded candidates' rounds are 0.0–0.5% of the
total on every set; 98% of decodes need under 10 rounds). Failing
candidates cost 22.2 ms per crowded slot (26%), 12.6 ms per empty slot
(55%), 16.5 ms per sweep slot (49%).

### A76: inside the stages (sampled; share of decoder samples, three-pass host)

| part | share | ≈ ms of the 107.4 ms slot |
|---|---|---|
| **subtraction** | 49.3% | 56.7 (timed) |
| reference (cos, sin) | 9.4% | 10.8 |
| generator for the fit and removal (`fill_phase`, `increments`) | 8.0% | 9.2 |
| removal | 7.5% | 8.6 |
| time search: products and sums | 6.8% | 7.8 |
| fit sums | 6.2% | 7.1 |
| time search: its generator | 5.9% | 6.8 |
| frequency correction | 3.5% | 4.0 |
| time search: alignment | 1.3% | 1.4 |
| **belief propagation** (sum-product) | 23.4% | 21.0 (timed) |
| `log2` and `exp2_neg` (the tanh/atanh domain) | 8.9% | ≈ 8 |
| the round's sums, products and folds | 14.5% | ≈ 13 |
| **feed** | 19.2% | 22.1 (timed) |
| FFT butterflies | 9.9% | ≈ 11.4 |
| window (`hann`), input `classify`, `take` bounds, stage glue | ≈ 5.0% | ≈ 5.8 |
| powers, `log2`, quantise to levels | ≈ 1.8% | ≈ 2.1 |
| sync 4.4%, soft bits 2.6%, OSD-1 0.3%, refine 0.3%, SNR 0.2% | | |

- **The subtraction's generator is now its largest part:** 13.9% of the
  slot (about 16 ms, 28% of the subtraction), run about 2.2 times per
  subtraction (1–2 time-search steps and one pass for the fit and
  removal together).
- **Small** (one pass): feed 38% of samples (butterflies 21%), belief
  propagation 29% (`min_sum_check` 20%).

### x86-64 (cross-check; `perf` user cycles, slot synthesis removed)

| | three-pass host share | A76 timed share | small share | A76 small |
|---|---|---|---|---|
| subtraction | 47.5% | 52.8% | — | — |
| of which the generator (`increments`, `fill_phase`) | 20.6% | ≈ 15% | — | — |
| belief propagation | 25.9% | 19.6% | 40% | 36% |
| feed | 17.6% | 20.6% | 32% | 48% |
| sync | 5.4% | 4.4% | 22% | 9% |
| soft bits | 3.1% | 2.2% | 5% | 6% |
| OSD-1, SNR | 0.3%, 0.2% | 0.2%, 0.2% | — | — |

`slotcost` on x86-64 (medians; times include the clock bias above): three
passes 71.55 ms (four runs 71.4–71.7), one pass 15.40, small 12.60; OSD-1
on against off 71.6 against 71.1 ms (OSD stage 0.17 ms). Per pass 15.30 /
34.47 / 21.59 ms, 0.64 / 3.28 / 7.20 ms per new decode. The generator is
a larger share on x86-64 (baseline x86-64 has no vector float-to-integer
conversion for `increments`).

### Cortex-M33 (emulated instructions per slot, by stage)

| slot (messages) | image | decoded | feed | sync | soft bits | BP | OSD-1 | SNR | subtract | **slot** |
|---|---|---|---|---|---|---|---|---|---|---|
| s0080 (0) | small | 0 | 199.9 M | 27.8 M | 9.0 M | 76.7 M | — | — | — | **313.9 M** |
| s0084 (10) | small | 8 | 199.9 M | 27.7 M | 9.0 M | 67.9 M | — | 0.8 M | — | **305.9 M** |
| s0086 (40) | small | 23 | 199.9 M | 27.5 M | 9.1 M | 54.3 M | — | 2.1 M | — | **293.5 M** |
| s0080 (0) | host | 0 | 201.8 M | 31.0 M | 12.0 M | 227.1 M | 2.1 M | — | — | **474.7 M** |
| s0083 (5) | host | 5 | 403.6 M | 61.7 M | 18.0 M | 328.7 M | 3.2 M | 0.5 M | 182.4 M | **999.2 M** |
| s0085 (20) | host | 20 | 605.4 M | 92.3 M | 24.1 M | 411.1 M | 3.4 M | 1.8 M | 727.3 M | **1 867.0 M** |
| s0086 (40) | host | 40 | 605.4 M | 92.0 M | 24.1 M | 365.4 M | 1.9 M | 3.6 M | 1 346.0 M | **2 440.2 M** |
| s0086 (40) | host, OSD off | 40 | 605.4 M | 92.0 M | 24.1 M | 365.4 M | — | 3.6 M | 1 346.0 M | 2 438.2 M |

(Refine 0.5–1.6 M and `finish` at most 0.15 M; setup before the decode,
9.9 M, left out. s0081, one signal at the synthetic slots' −24…−12 dB, decoded
nothing: 314.2 M small, 474.4 M host. s0087: small 291.7 M, host
2 329.1 M.)

- **The feed is 64–68% of a small one-pass slot** (199.9 M a pass,
  1.09 M per spectrogram row) and 25% of the host 40-message slot
  (201.8 M a pass, three passes). It is all inlined into `Decoder::pass`
  at `opt-level = "s"`, so the FFT is not split from the window, powers
  and levels here.
- Belief propagation: 2.27 M instructions per failing host candidate
  (sum-product, 30 rounds; 75.7 k a round), 1.02 M small (min-sum,
  34 k a round); 48% of an empty host slot, 24% of an empty small one.
- Subtraction: 36.4 M per call (O13b: 36.38 M), 55% of the host
  40-message slot. Its generator (`increments`, `fill_phase`,
  `skip_with`) 248 M of the slot's 1 346 M (18%); `Fit4::add_all`
  291 M, the time search's blocks 407 M, `Removal::refs` 168 M.
- OSD-1: 1.3–3.4 M per slot (6–16 runs, 0.21 M each), at most 0.7% of a
  slot.
- Small M33-only items: a 64-bit software division
  (`u64_div_rem`, `__udivmoddi4`) called from `Synth::fill_phase` and
  `subtract::score`, 12.9 M on s0086 (0.5%); `__aeabi_memclr4` zero
  fills 19.5 M host (0.8%) and 6.3 M small (2.0%).

### Memory

**Heap: zero.** The counting allocator saw 0 allocations inside
`decode_passes_staged` and `finish`, host, host OSD off and small, on 0
to 40 messages.

**Storage (host CLI layout, x86-64; the M33 images' statics):**

| buffer | host (x86-64) | small (x86-64) | M33 host image | M33 small image |
|---|---|---|---|---|
| spectrogram levels (`u8`; also the subtraction's working storage) | 179 584 | 161 920 | 179 584 | 162 944 |
| front end | 30 720 | 30 720 | 30 720 | 30 720 |
| sync scratch | 11 532 | 10 380 | 11 532 | 10 380 |
| decode list (128 or 64 entries) | 6 144 | 3 072 | 5 120 | 2 560 |
| candidate scratch (BP or OSD-1, shared) | 4 872 | 4 872 | 4 868 | 4 868 |
| candidates (200) | 1 600 | 1 600 | 1 600 | 1 600 |
| call table | 1 552 | 1 552 | — | — |
| residual (the slot as `f32`, multi-pass only) | **720 000** | — | 8 192 (check buffer) | — |
| **total** | **956 004** | 214 116 | 241 616 static | 212 048 static |
| constant tables (flash / rodata) | 39 429 | 39 429 | | |

- **Nothing scales with the number of signals.** Every buffer has a fixed
  capacity (the decode list's entries, the candidates, one candidate
  scratch reused). Per decoded signal only CPU grows.
- **The residual is 75% of the host decoder's memory** (720 000 of
  956 004 B).
- **Stack, x86-64 (painted):** 2 752 B (a pass with no OSD-1 run),
  4 800 B (OSD-1 ran), 8 992 B (a subtraction ran).
- **Stack, M33:** `check.sh --stack` frames unchanged from the gate
  (`_start` 4 104, `decode_a_slot` 2 512, host `subtract_one` 4 808,
  the subtraction's closure 2 872, `osd_decode` 272 B). Measured high
  water of the whole measurement program (painted, so `_start`'s
  encoder checks included): small 10 168 B, host 5 588 B without a
  subtraction, 10 424 B with one; inside the 16 KB budget. (The small
  image's deepest point is a zero fill inside `Decoder::pass`, deeper
  than the host image's no-subtraction path: the two images inline
  differently.)
- Flash (check images): small 90 564, host 102 256 B; static RAM
  212 048 / 241 616 B against the 245 760 B ceiling (4 144 B of headroom
  in the host image).

### Next candidates, ranked

Savings are estimates from this profile. A76 crowded = the three-pass
40-message slot (107.4 ms); "pooled" = crowded, wide and fading `main`
(85 ms mean).

**Bit-identical (O-round):**

1. **The subtraction's generator** (`fill_phase`'s prefix sum and
   `increments`): 13.9% of the slot sampled (≈ 16 ms), 28% of the
   subtraction, 18% of its M33 instructions, 20.6% of x86-64 cycles.
   The prefix sum is a running `u32` wrapping sum, a dependent add per
   sample (O13); integer addition modulo 2³² is associative, so a vector
   scan is bit-identical by construction. Sharing one block's generator
   output between the time search's second step and the fit when the
   start does not move is a second form. Estimate: −3 to −6 ms (3–5%)
   A76; on x86-64 more; on the M33 only the 64-bit division (below).
2. **The feed outside the butterflies:** window, input classification,
   `take`'s bounds arithmetic and the stage glue are about 5% of the
   three-pass slot sampled (≈ 6 ms, three feeds) and about 12% of a
   small slot. Fusing the window into the first FFT stage and hoisting
   the per-sample checks: −1 to −3 ms three-pass (1–3%), −0.4 to −1 ms
   small (3–7%). Needs a front-end microbenchmark first.
3. **The FFT on the M33:** the feed is 64–68% of a small one-pass slot
   (200 M instructions, 1.09 M a row) and is not split at
   `opt-level = "s"`. First step: an out-of-line M33 build to split the
   butterflies from the window and levels, and a count against a
   reference 3 840-point real FFT. Potentially the largest win for the
   embedded decoder; no estimate yet.
4. **Sum-product's `log2` and `exp2_neg`:** 8.9% of the slot sampled
   (≈ 38% of belief propagation). Only scheduling can change at equal
   results (the O12 lever was chains); −1 to −2 ms (1–2%), uncertain.
5. **M33 only, small:** the 64-bit division in `fill_phase` and `score`
   (12.9 M, 0.5% of the host 40-message slot) and the `memclr4` zero
   fills (0.8% host, 2.0% small).
- Not ranked: the reference (cos, sin) at 10.8 ms is issue-bound
  (O13b); the subtraction's other parts are at 1.3–1.8× their floor
  (O13); sync is 4.4% three-pass, 9% small; OSD-1 is 0.2–1.1% of any
  slot.

**Results-changing (gated):**

1. **Skip a later pass after a pass that added one decode or none
   more.** Pass 2 after a one-decode pass 1 cost 17.7–17.8 ms and found
   nothing in 10 125 `edges` and `sweep` slots: −44% of a one-signal slot
   (40.0 → 22.3 ms), −31% averaged over the sweep. Pass 3 after a
   one-decode pass 2 (326 of the 2 000 pooled slots): −2.9 ms per pooled
   slot (−3.4%) for 26 decodes lost (0.013 per slot, 223 ms per lost
   decode against pass 3's 10.98 ms per decode); after one or two:
   −5.5 ms (−6.4%), 86 lost (0.043 per slot, 127 ms each). Risk: the
   synthetic one-signal sets have nothing for a second pass to unmask;
   real slots with a strong signal over a weak one are what the
   subtraction is for, so the gate needs mixtures where pass 1 decodes
   one or two signals. This is the largest saving on quiet bands.
2. **Belief propagation on candidates that will fail:** over 99% of its
   rounds; 22 ms a crowded slot (26%), 12.6 ms an empty slot (55%),
   16.5 ms a sweep slot (49%); on the M33 48% of an empty host slot.
   E1's stall rule lost decodes (slow convergers plateau 15+ rounds).
   Untried: a screen before belief propagation (the hard decisions'
   unsatisfied checks, or the soft bits' magnitude), and a stall rule
   that keeps going while the unsatisfied count is low. Ceiling −50% of
   belief propagation: −11 ms crowded (−10%), −6 ms empty (−27%).
3. **Fewer pass-1 candidates on quiet slots:** an empty slot runs all
   100 (small: 75) through 30 rounds. A candidate limit from the sync
   score distribution overlaps candidate 2; ceiling the same 12.6 ms.
4. **The later passes' feed:** passes 2 and 3 recompute the whole
   spectrogram (7.4 ms each; 14% of a crowded slot, 18% of a one-signal
   slot). Updating it from the subtracted estimates instead changes
   rounding and needs more storage; ranked last on the memory goal.
5. **Memory: the residual** (720 000 B, 75% of host decoder memory). An
   `i16` residual would halve it (−360 kB) but changes the subtraction's
   arithmetic; not estimated for decodes.

**Dominated by OSD-1:** nothing. OSD-1 is 0.2 ms of a crowded slot
(0.2%), 0.24 ms empty (1.0%), 0.44 ms one-signal (1.1%); 0.7% of M33
instructions at most. **Dominated by subtraction:** the crowded
40-message slot (52.8%; 55% of M33 instructions) and the pooled mean
(about 40%). On sparse slots the subtraction is small (1.0–1.6 ms), but
the pass it starts costs 17.7 ms.

### Not measured

- A76 hardware counters (no `perf` binary; no counter tool this round)
  and M33 cycles (emulated instructions only).
- x86-64 per-set times (stopped: the VM's clock bias); x86-64 stage
  times are biased about 6.6 µs per stage event, so only `perf` shares
  are given.
- A second build of the small per-set runs and of the host sweep (one
  build each; the host sets both builds ran agree within 0.04 ms).
- The FFT's own share on the M33 (inlined), and the subtraction's parts
  on the M33 other than by function.
- Off-air recordings; real-band slots where pass 1 decodes one strong
  signal over a weaker one (needed before results-changing candidate 1).
- Energy and power.

## The subtraction's generator, fourteenth round (O14, bit-identical, 2026-11-01)

On the local branch `o14` from b93516e. Two candidates from the profile
above (its first and fifth bit-identical items): a blocked running sum
for the generator's phase, and the Cortex-M33's 64-bit division. The
first is **not faster and is kept out**; the second is kept (68634cc).
Decodes, records, residuals and encoder output identical, with no
qualification.

### Candidates

1. **The running phase four samples at a time** (a variant not on the
   branch).
   - `fill_phase` turns the increments into phases by a running `u32`
     sum, one dependent add per sample. The candidate gave each of four
     lanes its own running sum: lane l holds the phase before sample
     i + l and moves to sample i + l + 4 by the four increments from
     i + l (a sliding sum of four, read with overlapping loads), so the
     carried dependency is one vector add per four samples. Integer
     addition modulo 2³² is associative, so the phases are the same.
     The Cortex-M33 kept the plain loop (`VECTOR`).
   - A first form (shifted adds inside a group of four, the group's
     base broadcast to every lane) was turned back into scalar code by
     the compiler: the lanes of the base are equal, so it kept one, and
     the result was 19 AArch64 instructions per four samples with no
     vector operation. The sliding form compiles to the intended loop on
     both hosts (AArch64: 12 instructions per four samples, 4 loads, 3
     adds for the sums, 1 for the lanes, 1 store).
   - Test: both forms against the plain sum at every length up to four
     groups and at 239–241, from random and wrapping inputs; a mutant
     that drops one term of the sliding sum fails it and the per-sample
     generator test.
2. **Whole-sample phases without a 64-bit division** (68634cc).
   - The ramp's envelope (`envelope`, d = 480) and the time search's
     step phases (`step_phase`, d = 1 920) took ⌊r · 2³² / d⌋ through
     a `u64` division, a library call (`__aeabi_uldivmod`,
     `__udivmoddi4`, `u64_div_rem`) on the Cortex-M33. With
     2³² = q·d + m it is r·q + ⌊r·m / d⌋ exactly, in `u32` with no
     overflow for r < d < 2¹⁶ (`waveform::phase_of_ratio`); d is a
     constant at both call sites, so the remaining division is a
     multiply.
   - Tests: the 32-bit form against the 64-bit quotient for every
     r < d for every d up to 2 048 and a sample at d = 65 535; the
     envelope at every waveform sample and `step_phase` at every tone
     step and start error in −1 920…1 920, each against its previous
     form. A mutant that halves the remainder term fails these three,
     the pinned subtraction tests and the encoder's golden hashes.

### A76 (Raspberry Pi 5; `slotcost host host,passes=1 200 5`, medians)

64-byte-aligned builds (`-align-loops=64 -align-all-functions=6`), one
source path, a target directory per commit; one benchmark at a time on
core 3, the throttle, clock and temperature sampled once a second from
core 0, a run started only on 0x0. The base's `paired_bench` is
byte-identical to the profile's (sha256 5eb4624b…); the candidate pair
was built twice from one tree (a commit message amended in between):
byte-identical binaries.

| build (runs) | slot (ms) | subtract | feed | BP | one pass |
|---|---|---|---|---|---|
| **round 1** (alternated, 4 each) | | | | | |
| b93516e (base) | 107.40 | 56.70 | 22.10 | 21.00 | 20.20 |
| blocked sum (a variant not on the branch) | 107.40 | 56.59 | 22.235 | 20.99 | 20.30 |
| blocked sum + division (a variant not on the branch) | 107.40 | 56.56 | 22.225 | 21.005 | 20.30 |
| **round 2** (alternated, 4 each) | | | | | |
| b93516e (base) | 107.40 | 56.69 | 22.10 | 21.00 | 20.20 |
| 68634cc (division only) | 107.50 | 56.77 | 22.105 | 21.02 | 20.20 |

- The blocked sum takes 0.11 ms from the subtraction (0.2%) and the
  slot does not move: the feed, which it does not touch, moved
  +0.135 ms the other way (placement). Within the run-to-run spread of
  the subtraction (0.07–0.12 ms). **Not faster.**
- The division alone: +0.08 ms subtraction, +0.1 ms slot, within the
  spread; the A76 never called a library division (its `u64` quotient
  by a constant is a multiply), so no change is expected there.
- **Every timed run is valid:** 20 `slotcost` runs and 4 microbenchmark
  runs, 2 964 samples, every one 0x0, start and end checks too; one
  sample below 2.39 GHz in two runs, each the first second; peak
  55.4 °C.

**Why the profile's estimate did not hold.** An out-of-tree
microbenchmark (`Synth::fill_phase` over whole waveforms in 240-sample
blocks, as the subtraction calls it):

| | A76 ns / sample | x86-64 ns / sample | x86-64 instructions |
|---|---|---|---|
| b93516e | 1.707 | 1.48 | 7.81 G |
| blocked sum | 1.677 (−1.7%) | 1.54 (+4%) | 9.53 G (+22%) |

The generator costs about 4.1 A76 cycles a sample, and the running sum
is a small part of it: the dependent add is one cycle a sample and the
out-of-order core overlaps it with the loop's loads and stores, while
the blocked form's overlapping loads read lines that `increments` has
just stored. On x86-64 `perf` splits the two (they are separate
functions): `increments` 17.1% of the three-pass slot's cycles,
`fill_phase` (the sum, the envelope fill and the ramps) 3.6%; with the
blocked sum `fill_phase` rose to 4.3%. The profile's "generator" share
is almost all `increments`, the per-sample pulse, frequency and
conversion.

### x86-64 (Threadripper 3970X, core 5, `perf stat` user cycles and instructions)

The whole `slotcost host host,passes=1 200 5` program (slot synthesis
included), four alternated runs per build; the stage times carry the
virtual machine's clock bias (`acpi_pm`, about 6.7 µs a read) and are
given only for the subtraction:

| build | cycles (median) | instructions | subtract (ms) |
|---|---|---|---|
| round 1: b93516e | 375.38 G | 1 045.45 G | 34.42 |
| blocked sum | 377.40 G (+0.54%) | 1 074.29 G (+2.76%) | 34.84 |
| blocked sum + division | 377.33 G (+0.52%) | 1 074.66 G | 34.89 |
| round 2: b93516e | 375.12 G | 1 045.45 G | 34.39 |
| 68634cc | 374.92 G (−0.05%) | 1 045.81 G (+0.04%) | 34.37 |

The blocked sum is slower on x86-64; the division change is neutral
(a few more instructions where the 64-bit quotient by a constant was
one multiply).

### Cortex-M33 (emulated instructions; staged measurement image, s0086, 40 messages)

| | host slot | subtraction (37 calls) | per call | library division | stack high water |
|---|---|---|---|---|---|
| b93516e | 2 440.26 M | 1 345.99 M | 36.378 M | 13.80 M | 10 424 B |
| 68634cc | 2 426.54 M (−0.56%) | 1 332.26 M | **36.007 M (−1.02%)** | 0 | 10 392 B |

- The generator's own count is unchanged (`increments` 163.68 M,
  `fill_phase` 73.2–73.3 M, `skip_with` 11.56 M); `score` 6.41 →
  6.34 M. The blocked sum was not tried here (the M33 keeps the plain
  loop by construction).
- The small one-pass slot is unchanged (293 527 765 instructions on
  s0086): its only division was in the encoder checks before the
  decode (setup 9.94 → 9.88 M in the host image).

### Sizes and stack (`check.sh --stack`, thumbv8m)

| image | flash b93516e → 68634cc (B) | static RAM |
|---|---|---|
| small | 90 564 → 89 634 (−930) | 212 048 (unchanged) |
| host | 102 256 → 101 334 (−922) | 241 616 (unchanged) |
| small, OSD on | 90 564 → 89 634 | 212 048 |
| host, OSD off | 102 256 → 101 334 | 241 616 |
| encoder only | 35 676 → 34 766 (−910) | 0 |

Frames unchanged (`_start` 4 104, `decode_a_slot` 2 512, host
`subtract_one` 4 808, the subtraction's closure 2 872, `osd_decode`
272 B) except `Synth::fill_phase` 112 → 80 B in the encoder-only image.
No fused multiply-add (0 `vfma`/`vfms`/`vfnma`/`vfnms`; `vmla` 75 /
146 / 13, unchanged) and no `f64` helper in any image; thumbv7em
builds.

### Identity (68634cc against b93516e)

- **Records, x86-64:** the 73 800 `paired_bench run` records (`crowded`,
  `wide`, `fading` on `main`, `cap1`, `cap2`; `sweep`, `edges`, `empty`
  on `main`; `host`, `host,passes=1`, `small`): 0 differ.
- **800 WAV slots** × `host`, `small`, `host,passes=1` × plain, `--raw`,
  `--profile` (7 200 runs, 78 690 decode lines, timings removed):
  byte-identical, `--profile` metadata included.
- **Encoder:** the 45 `cat-ft8-encode` WAV files identical.
- **Cortex-M33 in the emulator:** the small and host images' decodes on
  the seven WAV slots (report counts, every decode, the `LEVELS` and
  `DECODES_LIST` statics) byte-identical, 14 of 14; the multi-pass host
  image's residual after the further passes on s0086 and s0407
  (180 000 `f32` each) identical, and equal to O13's.
- **A76** (after the Raspberry Pi 5 came back; the interrupted run's
  output discarded and the run repeated from scratch): the 73 800 records
  of 68634cc's AArch64 `paired_bench` equal x86-64's at 68634cc and at
  b93516e (0 differ); the 800 WAV slots × 3 × 3 with its AArch64
  `cat-ft8-decode` (7 200 runs, 78 690 decode lines) byte-identical to
  x86-64 b93516e's. Not timed, so not held to the throttle rule; 0x0
  before and after each part.
- The blocked sum was checked by its tests and the per-sample
  generator test on x86-64 only; it is not on the branch.

### Checks at 68634cc

Local CI: fmt; clippy `-D warnings` on the workspace, cat-ft8 with all
targets and features, and the thumbv8m library; workspace, profile,
no-std and release tests with the ignored ones; MSRV 1.75; docs with
and without `std`; `check.sh --stack`.

Raspberry Pi 5: the release library tests with the ignored ones pass
(189; the one test that reads the source tree skipped, as the binary
runs without it).

### Not measured

- Cortex-M33 cycles (emulated instructions only).
- Where `increments`' 4 A76 cycles a sample go: there are no hardware
  counters on the Pi, so its loop was not split into issue and stalls.
  It is now the generator's cost on every target (x86-64 17% of the
  slot's cycles, the M33 164 M of the subtraction's 1 332 M) and the
  next candidate here; its arithmetic per sample is already minimal for
  bit identity, so the lever is the loop structure (four runs per
  symbol, their setup and scalar tails per 240-sample block) and, on
  x86-64, the two-step conversion of `increment`.
- The blocked sum on the M33 and on other cores.

## The generator's per-sample step, fifteenth round (O15, bit-identical, 2026-11-01)

On the local branch `o15` from `o14` (2fca9a9, the code of 68634cc).
The candidates are the generator's per-sample work: the phase increment
(`increments`, its four runs per symbol and its conversion on x86-64)
and the running sum after it. Each was chosen with an out-of-tree
microbenchmark first. Two commits are kept: b18ded6 (x86-64) and
81cb159 (x86-64 and AArch64). Decodes, records, residuals and encoder
output are identical, with no qualification.

### Measured first: where the generator's time goes

The microbenchmark is a copy of `increments` and the running sum, run
as the subtraction calls them: whole waveforms in 240-sample blocks,
three messages and bases. Each variant is checked bit for bit against
`Synth::fill_phase`. Figures are ns per sample, medians of 9. A76 runs
are on core 3, all valid at 0x0.

| | A76 | x86-64 |
|---|---|---|
| `increments` alone | 0.785 | 1.128 |
| `increments` and the plain running sum | 1.578 | 1.383 |
| the plain running sum alone | 0.793 | 0.290 |
| `increments` alone, blocks of 240 / 480 / 960 / 1 920 | 0.781 / 0.737 / 0.716 / 0.710 | 1.121 / 1.082 / 1.052 / 1.041 |

- **Correction to O14.** On the A76 the plain running sum is half the
  generator. It runs at 1.9 cycles a sample (its loop, one load, one
  store and one dependent add per sample, issues at half a sample a
  cycle). O14 read the sum as a small part only because its sliding
  form gained little. That form reads each entry four times, through
  overlapping vector loads, and costs 0.677 ns a sample alone.
- `increments` has a fixed cost per call: its runs' setup and the
  scalar tails where a run ends inside a block. That cost is about 9%
  of `increments` on the A76 and 7% on x86-64, measured against
  whole-symbol calls. The subtraction's blocks are 240 samples, the
  size of its working storage, so the most a change to the runs'
  structure could save is about 0.07 ns a sample on the A76. That is
  about 0.8 ms of the 107 ms crowded slot. No such change was made.
- On x86-64, `increments` itself is the larger part: the vector
  float-to-integer conversion is two floors (below).

**Running-sum forms** (`increments` with the form, and the form alone):

| form | A76 | A76 alone | x86-64 | x86-64 alone |
|---|---|---|---|---|
| plain loop | 1.578 | 0.793 | 1.407 | 0.290 |
| **groups of four, carried add per group** | **1.187** | **0.394** | 1.403 | 0.326 |
| O14's sliding sums (each lane its own sum) | 1.529 | 0.677 | 1.442 | 0.388 |
| pairs | 1.281 | 0.426 | 1.382 | 0.290 |
| groups of eight | 1.203 | 0.433 | 1.380 | 0.285 |
| fused into `increments` (four at a time) | 2.313 | — | 3.823 | — |

In the fused form the compiler converted each lane separately, so it
is slower everywhere.

**Increment conversions on x86-64** (`increments` alone; with the
plain sum):

| form | alone | with the sum |
|---|---|---|
| two floors (before) | 1.128 | 1.383 |
| **one rounding, one floor** | **0.850** | **1.105** |
| clamped, then `as i32` | 1.469 | 1.739 |

### Changes

1. **b18ded6: the x86-64 phase increment with one rounding and one
   floor.**
   - `increment(x)` for x in [0, 2³¹) took ⌊x / 2¹⁶⌋ and then the floor
     of the rest. Each floor is a rounding (adding 1.5·2²³) and a
     correction.
   - Now r = x / 2¹⁶ is rounded to the nearest integer, with no
     correction. The rest d = x − 2¹⁶·r satisfies |d| ≤ 2¹⁵ and is
     floored once, so ⌊x⌋ = 2¹⁶·r + ⌊d⌋.
   - Each step is exact. The scalings are by 2¹⁶. r·2¹⁶ fits, since
     r ≤ 2¹⁵. d is x itself below 2¹⁵, where r = 0. Otherwise d is a
     multiple of x's spacing, fewer than 2²⁴ of them.
   - This is x86-64 only (`cfg`). Other targets are unchanged. Their
     AArch64 decoder binary is byte-identical to the base's; their
     `paired_bench` differs only in panic-location line numbers.
   - Test: the existing `increment_matches_truncation_everywhere`
     (every `f32` in [0, 2³¹)) and its strided form pass. A mutant that
     truncates the rest instead of flooring it fails the strided test
     and the per-sample generator test.
2. **81cb159: the running phase in groups of four.**
   - On x86-64 and AArch64 (`VECTOR`), four increments are taken at a
     time. Their sums within the group come first, by two shifted adds.
     The group's start phase is then added to each. (As compiled, three
     dependent adds per four samples are carried, not one: see O15b.)
   - The compiler keeps this in scalar registers, with paired loads and
     stores. In the crate's build, 12 AArch64 instructions cover four
     samples, with three dependent adds.
   - Addition modulo 2³² is associative, so the phases are the same.
     The Cortex-M33 keeps the plain loop; its subtraction's count is
     unchanged.
   - Test: both forms against the plain sum, at every length up to four
     groups and at 239–241, from random and wrapping inputs. A mutant
     that adds the wrong lane fails it and four pinned subtraction
     tests.

### A76 (Raspberry Pi 5; `slotcost host host,passes=1 200 5`, medians of 4 alternated runs)

The builds are aligned, from one source path, run on core 3, with the
throttle sampled once a second from core 0.

| build | slot (ms) | subtract | feed | BP | one pass |
|---|---|---|---|---|---|
| 2fca9a9 (base, 68634cc's code; `paired_bench` byte-identical) | 107.50 | 56.775 | 22.100 | 21.020 | 20.20 |
| b18ded6 (x86-64 increment) | 107.35 | 56.695 | 22.095 | 21.010 | 20.20 |
| **81cb159 (grouped sum)** | **104.40 (−2.9%)** | **53.550 (−5.7%)** | 22.230 | 20.995 | 20.30 |

- **The grouped sum takes 3.2 ms from the subtraction.** That is 13
  times the largest run-to-run spread of the stage (0.25 ms). The
  untouched stages move by at most 0.13 ms (feed, placement).
- By pass: pass 2 is 54.32 → 52.11 ms, pass 3 is 32.98 → 32.03 ms, and
  the cost per new decode is 0.84 / 4.97 / 10.64 ms (the logged
  medians).
- b18ded6 does not change AArch64 code. Its −0.08 ms is inside the
  spread.
- **Every timed run is valid.** The 12 `slotcost` runs had 1 764
  samples, every one 0x0, including the start and end checks; one
  sample below 2.39 GHz, in the first second of the first run; peak
  56.0 °C. The 11 microbenchmark runs were also all 0x0, but 4 of them
  had a sample below 2.39 GHz (each its first second; the runs were
  1–6 s long with no warm-up), so their figures carry some ramp-up time;
  the decision rests on the `slotcost` runs above.

### x86-64 (Threadripper 3970X, core 5, `perf stat` user counters over the whole `slotcost` program)

| build | cycles (median) | instructions | subtract (ms) | `increments` share |
|---|---|---|---|---|
| round 1: 18198d7 (base code) | 374.03 G | 1 045.82 G | 34.19 | 17.2% |
| b18ded6's code | **361.30 G (−3.4%)** | 1 030.63 G (−1.45%) | 31.28 | 13.6% |
| round 2: b18ded6 (the same binary) | 362.13 G | 1 030.63 G | 31.45 | 13.6% |
| 81cb159 | 361.79 G (−0.1%) | 1 036.26 G (+0.55%) | 31.52 | 13.5% |

- The stage times carry the clock bias of the virtual machine
  (`acpi_pm`). The cycle counts are the measure.
- On x86-64 the grouped sum is neutral, as in the microbenchmark.

### Cortex-M33 (emulated instructions, staged measurement image, s0086)

| | host slot | subtraction per call (37 calls) | `increments` | `fill_phase` | stack high water |
|---|---|---|---|---|---|
| 68634cc (O14) | 2 426.54 M | 36.007 M | 163.68 M | 73.29 M | 10 392 B |
| 81cb159 | 2 426.44 M | 36.007 M | 163.68 M | 73.29 M | 10 392 B |

- The subtraction is unchanged instruction for instruction, since the
  M33 takes neither change.
- The slot differs by −0.10 M (and the small slot by −34 k), in code
  outside the generator that the compiler inlines differently between
  builds (as in O13).

### Sizes and stack (`check.sh --stack`, thumbv8m)

- Flash is unchanged from 2fca9a9 at both commits: small 89 634 B,
  host 101 334 B, the OSD variants the same, encoder only 34 766 B.
- Static RAM is unchanged (212 048 / 241 616 / 0 B).
- Every recorded frame is unchanged.
- No fused multiply-add (`vmla` 75 / 146 / 13) and no `f64` helper.
- thumbv7em builds.

### Identity (81cb159)

- **Records, x86-64:** the 73 800 `paired_bench run` records equal
  b93516e's (0 differ).
- **800 WAV slots** × 3 configurations × 3 modes: byte-identical to
  b93516e's.
- **Encoder:** the 45 WAV files are identical.
- **Cortex-M33 in the emulator:** the small and host images' decodes
  and statics on seven slots (14 of 14), and the multi-pass residuals
  on s0086 and s0407, are byte-identical to 68634cc's.
- **A76:** the 73 800 records of 81cb159's AArch64 `paired_bench`
  equal x86-64's at 81cb159 and at b93516e (0 differ); the 800 WAV
  slots × 3 × 3 with its AArch64 `cat-ft8-decode` byte-identical to
  x86-64 b93516e's (7 200 runs, 78 690 decode lines). 0x0 before and
  after each part.

### Checks

Local CI at 81cb159 passes: fmt; clippy `-D warnings` (workspace, all
targets and features, thumbv8m); workspace, profile, no-std and release
tests with the ignored ones (including `increment_matches_truncation_everywhere`
on x86-64); the bulk run; MSRV 1.75; docs; `check.sh --stack`.
Raspberry Pi 5: the release library tests with the ignored ones pass
(190, the grouped-sum test among them; the source-tree test skipped).

### Not measured

- Cortex-M33 cycles (emulated instructions only).
- The grouped sum on the M33. The plain loop is kept there by
  construction, and the grouped form was not counted.
- The microbenchmark was not run on other cores.
- After a toolchain change, check that the grouped sum still compiles to
  paired loads and stores on AArch64 (12 instructions per four samples
  in `fill_phase`, rustc 1.93.0).

## Review follow-ups (O15b, bit-identical, 2026-11-01)

On the local branch `o15b` from `o15`. One code commit (641c599):
- the grouped running sum is gated on what was measured;
- `round_small` is shared by `floor_small` and the x86-64 increment;
- `phase_of_ratio` moves to `decode::math`;
- the docs are re-wrapped.

No output changes.

### The running sum's form

Three forms, each built for every target. They are variants not on the
branch.

- **(a)** The shipped grouped form: inclusive sums by two shifted adds,
  then each less its entry. AArch64 emits 12 instructions per four
  entries: two paired loads, two paired stores, three dependent adds.
- **(b)** A serial exclusive prefix per group of four, one carried add
  per group.
- **(c)** The plain loop inside `chunks_exact_mut(4)`.
- (b) and (c) both compile to 11 AArch64 instructions per four entries,
  with the same paired loads and stores but four dependent adds. A
  direct write-out of (a)'s emitted code also compiles to (b)'s.

Measurement setup:
- A76: aligned builds, `slotcost host host,passes=1 200 5`, 4
  alternated runs each.
- x86-64: core 5, `perf stat` over the whole program, 4 alternated runs.
- M33: the staged image on s0086 (37 subtractions) and `check.sh`.

| form | A76 slot (ms) | A76 subtract (spread) | A76 feed | x86-64 cycles (median) | x86-64 instructions | M33 per subtraction | M33 host slot | flash small / host / encoder (B) |
|---|---|---|---|---|---|---|---|---|
| plain loop (reference: O15's M33 and O14) | 107.50 | 56.775 (0.25) | 22.100 | 362.13 G | 1 030.63 G | 36.007 M | 2 426.44 M | 89 634 / 101 334 / 34 766 |
| **(a) grouped** | **104.45** | **53.620 (0.12)** | 22.225 | 361.58 G | 1 036.26 G | 37.552 M (+4.3%) | 2 484.90 M | 89 742 / 101 442 / 34 858 |
| (b) serial per group | 104.65 | 53.945 (0.03) | 22.100 | 360.49 G | 1 033.99 G | 35.501 M (−1.4%) | 2 409.00 M | 89 686 / 101 386 / 34 818 |
| (c) plain in groups | 104.65 | 53.965 (0.10) | 22.100 | 361.20 G | 1 030.84 G | 36.467 M (+1.3%) | 2 444.74 M | 89 686 / 101 386 / 34 794 |

The plain loop's A76 and x86-64 figures come from O15's runs, not this
round's alternation. Its M33 figures are 81cb159's, the code the three
variants were built from; against 641c599's host slot (2 427.73 M, the
layout effect below) the forms' host slots read 1.29 M lower, (b)'s
−18.73 M rather than −17.44 M.

**Decision.**

- **A76.** (b) and (c) are 0.33–0.35 ms slower than (a) on the
  subtraction and about 0.2 ms slower on the slot (0.2%), at the edge of
  code-placement noise: the untouched feed moved 0.125 ms, about a third
  of the subtraction gap. On these runs neither is within spread of (a),
  so (a) stays.
- **M33.** (a) is not neutral there: +4.3% per subtraction and +108 B
  of flash per image.
- **x86-64.** The forms are within 0.3% of each other on x86-64.
- **Result.** (a) is taken on AArch64 only (`GROUPED_SUM =
  cfg!(target_arch = "aarch64")`). x86-64 and the M33 keep the plain
  loop, which is the M33's O15 code. The const parameter stays, so the
  test still runs both forms on every host.
- **A lead for later.** (b) would cut the M33's subtraction by 1.4%
  (+52 B of flash). It is not taken here; if it is later taken on the
  M33, (b) on every target should be reconsidered, given how small the
  A76 margin is.

### The final commit (641c599)

**A76:** 641c599 against 81cb159 (O15), 4 alternated runs each.

| | slot (ms) | subtract | feed | BP |
|---|---|---|---|---|
| 81cb159 | 104.35 | 53.535 | 22.210 | 21.000 |
| 641c599 | 104.40 | 53.565 | 22.220 | 21.000 |

The grouped sum's AArch64 code in `fill_phase` is instruction for
instruction the same as in (a)'s variant. The difference between the
two rows is inside the spread.

**x86-64:** 641c599 runs the plain loop and the `round_small` form of the
increment. Against O15's code (grouped on x86-64):

| build | cycles (median) | range |
|---|---|---|
| O15's code | 361.03 G | 360.75–362.57 G |
| 641c599 | 362.76 G (+0.5%) | 361.29–363.17 G |

- The ranges overlap.
- Instructions fell 1 036.25 → 1 030.63 G (−0.54%).
- `increments` is 13.6% of the slot.

**Validity.**
- All 20 timed A76 `slotcost` runs this round are valid: 2 900 samples,
  every one 0x0, including the start and end checks.
- In two runs the first second was below 2.39 GHz.

**Sizes (`check.sh --stack`, 641c599):** flash, static RAM and every
recorded frame are unchanged from O15: small 89 634 B, host 101 334 B,
encoder only 34 766 B. No fused multiply-add and no `f64`. thumbv7em
builds.

**Identity (641c599):**

- **Records:** the 73 800 records on x86-64, and those of the A76's
  AArch64 build, equal b93516e's (0 differ).
- **WAV slots:** 800 × 3 configurations × 3 modes, byte-identical on
  both targets.
- **Encoder:** the 45 WAV files are identical.
- **M33 (emulated):** the small and host images' decodes and statics on
  seven slots are byte-identical to 68634cc's (14 of 14), as are the
  multi-pass residuals on s0086 and s0407. The subtraction is
  36.007 M instructions a call, unchanged, and the `check.sh` sizes are
  unchanged. The staged image's host slot is not quite: 2 427.73 M
  against 2 426.44 M at 81cb159 (+1.29 M, +0.05%), all of it in
  `Decoder::pass`, where each of the three feeds costs 429 824 more
  (every other function's count identical). The staged image's text is
  16 B larger. No code in the feed changed; it is a layout and inlining
  effect at `opt-level = "s"`, as O15's −0.10 M was.

**Checks:**

- Local CI passes, including the exhaustive `floor_small` and
  `increment` tests through `round_small`.
- The A76 release library tests with the ignored ones pass (190; the
  source-tree test is skipped).
- A mutant that offsets `round_small`'s returned value fails
  `floor_small`'s and belief propagation's pinned tests.
- `round_small` is pinned at ±2²², ±(2²² − ½), ±½, ±1½, ±2½ and ±0
  against `libm::rintf` (O15c). Mutants that round negative ties the
  other way or use 2²³ as the constant fail it.

## The front end outside the butterflies, sixteenth round (O16, bit-identical, 2026-11-01)

Base: 641c599's code (the O15b record commits after it change no code).
The target is the feed's work outside the FFT butterflies: the window,
the input checks, the bounds arithmetic and the glue. That work was
about 5% of the three-pass host slot sampled after OSD-1. Each candidate
was chosen with an out-of-tree microbenchmark first. One code commit
is kept (78b1755). Decodes, records, residuals and encoder output are
identical, with no qualification.

### Measured first: the front end per row

The microbenchmark is a copy of `take` and `finish_row` built from the
crate's own pieces (its internals made public in a scratch copy only).
It runs over three crowded synthetic slots (30 signals each), on the A76
core 3 after a 3 s warm-up in the program and a 5 s busy loop before it
(run valid at 0x0), and on x86-64 core 5. Figures are ns per spectrogram
row, medians of 15. Every variant's levels and counters were checked
against the crate's `Spectrogram::feed` on the three slots and on a slot
with NaN, infinite and out-of-range samples, for whole-slot,
4 801-sample and 517-sample blocks.

| part (alone, ns per row) | A76 | x86-64 |
|---|---|---|
| the crate's `feed` (the replica within 0.3%) | 41 089 | 22 908 |
| the copy and checks into the window, and its shift | 2 222 | 1 316 |
| the windowed, digit-reversed gather (with a 15 KB copy) | 5 849 | 2 954 |
| the six stages (with a 30 KB copy) | 28 828 | 13 049 |
| powers, `log2`, levels and the two counts | 5 614 | 6 166 |

| variant (against the replica) | A76 | x86-64 |
|---|---|---|
| window in the first stage, scalar, `hann` as before | −9.2% | −3.8% |
| the input checks for a block at once, then a copy | −3.2% | −4.4% |
| both | −11.6% | −8.3% |
| window in the first stage, its mirror point resolved per input | −10.7% | −6.9% |
| that, and the block checks | −13.9% | −11.9% |
| the same, four blocks side by side, one table entry per four (**kept**) | **−13.1%** | −8.4% |
| that, and the block checks (**kept**) | **−16.0%** | **−12.8%** |
| four blocks side by side, every input from the table and `hann` | −6.3% | +0.1% |
| natural order, four inputs side by side (contiguous loads) and the block checks | −15.8% | −13.0% |
| a 15 KB digit-reversed window table (measurement only) | −2.1% | −2.9% |

- **The window and the first stage.** The first radix-4 stage stored
  nothing new but read back all 3 840 windowed values the gather had
  just stored, and `hann`'s `min` and bounds checks ran per value.
  Fusing removes the store and reload. Resolving the mirror point per
  input (a block's inputs are elements n₀ + 480m with n₀ < 480, so m = 0,
  1 read `HANN_HALF` ascending and m = 2, 3 descending) removes the `min`
  and lets one slice check cover a block.
- **The natural-order form** ties the kept one but needs an inverse
  table, and dropping `DIGIT_REV` would change `TABLE_BYTES` and with it
  the `--profile` output's `static_table_bytes`. The kept form reads
  every fourth (four lanes: sixteenth) entry of the existing table.
- **The window table** gains least and costs 15 KB; not pursued.
- The crate-level check (`feed` and `feed_i16` through the public API,
  the two builds from one source path, alternated, two runs each; the
  levels' checksum equal). Each cell is run 1 / run 2, the median of 15
  repetitions in each; the change is between the run-2 medians in every
  column:

| | A76 `feed` | A76 `feed_i16` | x86-64 `feed` | x86-64 `feed_i16` |
|---|---|---|---|---|
| 641c599's code | 39 981 / 39 986 | 40 057 / 40 067 | 24 676 / 22 981 | 23 453 / 23 025 |
| 78b1755's code | 34 643 / 34 608 (−13.4%) | 34 527 / 34 497 (−13.9%) | 20 653 / 19 859 (−13.6%) | 20 703 / 19 878 (−13.7%) |

  The x86-64 base's run 1 is noisy (its `feed` repetitions range
  22 702–26 442 ns, its median 7% above run 2's); against it the change
  would read −16.3% and −11.7%. The A76 runs agree within 0.2%.

### Changes (78b1755)

78b1755 makes two optimisations in one commit, so the decoder-level
figures below (A76, x86-64, M33) are their sum. Only the microbenchmark
separates them (A76: the window in the first stage −13.1% of a row, the
block checks −3.2%, both −16.0%). From O16b on, one optimisation per
commit.

1. **The window in the first stage** (`fft::first_stage`).
   - Position p's input is element n = `DIGIT_REV[p]` windowed:
     `samples[2n] · hann(2n)` and `samples[2n + 1] · hann(2n + 1)`. The
     first stage now computes those as it loads them instead of reading
     them back from the work buffer. Each value and each product is the
     one the stored path computes, in the same order, so the output is
     the same bit for bit; nothing is reassociated.
   - Block b takes elements n₀ + 480m, and block 4c + j takes
     n₀ = `DIGIT_REV[16c]` + 120j. A constant copy of the table
     (`DIGIT_REV_TABLE`, the static's initialiser) is checked for this at
     compile time. One table entry therefore places `LANES` adjacent
     blocks (four on x86-64 and AArch64, one on the Cortex-M33), and the
     window entry for each input is read directly from the half table.
   - `hann` and the six-stage `fft` remain for the tests only. The later
     five stages are `later_stages`, unchanged.
   - Tables and `TABLE_BYTES` are unchanged.
2. **Block checks in `feed`** (`frontend::checked_copy`).
   - `take` hands its fill closure a whole destination block (up to the
     window's free space). `feed` checks that every sample is finite and
     within ±2¹⁶ (`abs(x) <= INPUT_LIMIT`, false for NaN and the
     infinities, a fold with no early exit) and copies the block. Only
     a block that fails is classified sample by sample. Where the check
     passes, `classify` would have returned every sample unchanged and
     counted nothing.
   - `feed_i16`'s scaled samples are within ±1, so it no longer
     classifies them; `skip` fills zeros.
   - The samples before the row origin are still checked, through the
     start of the empty window, so the counters cover the slot.
3. **Tests.**
   - `first_stage_equals_the_stored_window_and_stage_bit_for_bit`: one,
     two and four lanes against the stored window and `stage::<4, 1, 1>`,
     every bit including the sign of zero, and `spectrum` against the
     stored path and the six stages. Inputs: noise to ±2¹⁶, signed zeros,
     subnormals, the limit itself, and impulses at the mirror points and
     the ends.
   - `feed_equals_feeding_the_classified_samples`: ±2¹⁶, the next value
     beyond, `f32::MAX`, infinities, NaNs, −0 and the smallest subnormal,
     before the row origin, in and on the edges of blocks of 1 to 180 000
     samples; levels and counters against feeding the classified samples.
   - Mutants caught: a swapped mirrored window entry, a wrong block
     spread, the products of a block summed in reverse order, `abs`
     dropped from the check, the samples before the row origin not
     checked. Swapping the two operands of one addition (exact in IEEE
     arithmetic) survives, as it must.

### A76 (Raspberry Pi 5; `slotcost host host,passes=1 200 5`, medians of 4 alternated runs)

Aligned builds (`-align-loops=64 -align-all-functions=6`) from one
source path, core 3, the throttle, clock and temperature sampled once a
second from core 0, a 5 s busy loop on core 3 (sampled) before each run.
The timed binaries were built from the commit before a comment-only
amendment; their `.text` is byte-identical to 78b1755's for
`paired_bench` and `cat-ft8-decode` on both hosts (only panic line
numbers in the data differ).

| build | slot (ms) | feed (spread) | subtract (spread) | BP | sync | one pass | one-pass feed |
|---|---|---|---|---|---|---|---|
| 641c599's code | 104.40 | 22.230 (0.020) | 53.575 (0.120) | 21.000 | 4.680 | 20.30 | 7.470 |
| **78b1755** | **101.40 (−2.9%)** | **19.130 (0.010), −3.10 ms (−13.9%)** | 53.675 (0.130) | 21.010 | 4.680 | 19.30 (−4.9%) | 6.455 (−13.6%) |

- **The feed is 1.03–1.04 ms faster in every pass** (7.41 → 6.37–6.38
  ms). That is 31 times the largest untouched stage's shift (the
  subtraction, +0.10 ms, within its spread of 0.12–0.13 ms; belief
  propagation +0.01, sync 0, soft bits 0).
- Per pass 20.20 / 52.13 / 32.05 → 19.18 / 51.17 / 31.05 ms; per new
  decode 0.84 / 4.96 / 10.68 → 0.80 / 4.87 / 10.35 ms. Worst slot 111.8
  → 108.8 ms.
- The base matches O15b (104.40 ms, subtraction 53.565, feed 22.220).
- **Every timed run is valid.** The 8 `slotcost` runs had 1 144 samples
  during the runs and 40 during the warm-ups, every one 0x0, including
  the start and end checks; no sample below 2.39 GHz during a run (one
  warm-up sample was, the first of the first run); peak 55.4 °C. The 8
  microbenchmark runs (4 of the variants, 4 of the crate-level check)
  were all valid at 0x0 as well; in 5 of them the warm-up's first
  sample was at 1.5–1.6 GHz (4 variant runs and the base's first
  crate-level run), the busy loop's ramp, with every sample during the
  runs at 2.4 GHz.

### Cortex-M33 (emulated instructions; staged measurement images, s0086)

| | small slot (one pass) | small feed | host slot (40 messages, three passes) | host feed (per pass) | subtraction | stack high water (small / host) |
|---|---|---|---|---|---|---|
| 641c599's code | 293.53 M | 199.94 M | 2 426.54 M | 605.54 M (201.85 M) | 1 332.26 M | 10 200 / 10 392 B |
| 78b1755 | **285.89 M (−2.6%)** | **191.99 M (−4.0%)** | **2 403.40 M (−0.95%)** | **581.72 M (193.91 M), −3.9%** | 1 332.26 M | 10 200 / 10 392 B |

- Same decodes (23 small, 40 host). Sync, belief propagation and the
  subtraction are unchanged to within 0.01 M; soft bits (+0.14 / +0.37 M)
  and SNR (+0.18 / +0.31 M) moved in code the change does not touch,
  inlined differently into `Decoder::pass` at `opt-level = "s"`
  (rustc 1.93.0), as in O15 and O15b.
- This round's base reading of the host slot is 2 426.54 M; O15b read
  2 427.73 M for the same code built at another path (layout).
- The two commits' images differ in a few instructions apart from the
  front end (a scheduling swap and a field offset), so the counts above
  were taken on 78b1755's own images. They equal those of the timed
  commit's images instruction for instruction.

**The feed split (measurement images only).** The feed is inlined into
`Decoder::pass` at `opt-level = "s"`, so measurement copies of the two
commits marked `take`, `checked_copy`, `spectrum`, the transform's
stages, `bin_powers` and `finish_row` `#[inline(never)]` (out of tree).
Per pass, small image, s0086, 184 rows:

| part | 641c599's code | 78b1755 |
|---|---|---|
| butterflies, first stage | 24.08 M (by difference) | in the next row |
| window (and, with the change, the first stage) | 10.60 M (the gather) | 28.68 M |
| butterflies, stages 2–6 | 135.62 M (159.70 M with the first) | 135.62 M |
| input copy and checks | 4.87 M | 2.40 M (`checked_copy` 1.80 M, its copy 0.59 M) |
| window shift (`memmove`) | 2.43 M | 2.43 M |
| powers (`bin_powers`) | 6.49 M | 6.49 M |
| `log2`, levels and counts (`finish_row`) | 9.61 M | 9.61 M |
| **feed** | **200.70 M** | **192.22 M** |

- On the M33 the butterflies are 80% of the feed (159.7 M a pass, 0.87 M
  a row), 54% of the small one-pass slot. The window with the first
  stage fell 34.68 → 28.68 M and the input checks 4.87 → 2.40 M.
- The out-of-line images are not better: 200.70 against 199.94 M
  (base) and 192.22 against 191.99 M (change), 10 448–10 480 B of stack
  against 10 200. Not shipped.

### x86-64 (Threadripper 3970X, core 5, `perf stat` user counters over the whole `slotcost` program)

Four alternated runs per build, aligned builds from one source path.
The stage times carry the virtual machine's clock bias (`acpi_pm`, about
6.7 µs a read); the cycle counts are the measure.

| build | cycles (median, range) | instructions | feed, three passes (ms) | subtract (ms) |
|---|---|---|---|---|
| 641c599's code | 360.97 G (360.23–361.17) | 1 030.63 G | 12.655 | 31.315 |
| 78b1755 | **352.34 G (352.28–353.59), −2.4%** | 995.45 G (−3.4%) | 11.085 (−12.4%) | 31.350 |

- The ranges do not overlap. O15b read 362.76 G for the same code, with
  the same 1 030.63 G instructions.
- `perf record` of the three-pass program: `power_bins` (the transform
  and powers, inlined) 15.76% → 14.71% of user cycles, and
  `Spectrogram::feed`'s own share (the input checks) 0.94% → below 0.3%.

### Sizes and stack (`check.sh --stack`, thumbv8m)

| image | flash 641c599 → 78b1755 (B) | static RAM |
|---|---|---|
| small | 89 634 → 89 734 (+100) | 212 048 (unchanged) |
| host | 101 334 → 101 430 (+96) | 241 616 (unchanged) |
| small, OSD on | 89 634 → 89 734 | 212 048 |
| host, OSD off | 101 334 → 101 430 | 241 616 |
| encoder only | 34 766 (unchanged) | 0 |

Every recorded frame is unchanged (`_start` 4 104, `decode_a_slot`
2 512, host `subtract_one` 4 808, the subtraction's closure 2 872,
`osd_decode` 272 B). No fused multiply-add (`vmla` 75 / 146 / 13,
unchanged) and no `f64` helper in any image; thumbv7em builds. The
tables and `TABLE_BYTES` are unchanged.

### Identity (78b1755)

- **Records, x86-64:** the 73 800 `paired_bench run` records equal
  b93516e's (0 differ).
- **800 WAV slots** × `host`, `small`, `host,passes=1` × plain, `--raw`,
  `--profile` (7 200 runs, 78 690 decode lines): byte-identical to
  b93516e's, `--profile` metadata included.
- **Encoder:** the 45 `cat-ft8-encode` WAV files are identical.
- **Cortex-M33 (emulated):** the small and host images' decodes and
  statics on seven slots (14 of 14), and the multi-pass residuals on
  s0086 and s0407, are byte-identical to 68634cc's.
- **A76:** the 73 800 records of 78b1755's AArch64 `paired_bench` equal
  x86-64's at 78b1755 and at b93516e (0 differ); the 800 WAV slots × 3 × 3
  with its AArch64 `cat-ft8-decode` are byte-identical to x86-64
  b93516e's (7 200 runs, 78 690 decode lines). Not timed; 0x0 before and
  after each part.
- The x86-64 and M33 identity above was also run on the timed commit
  before the comment-only amendment, with the same results.

### Checks

Local CI passes (fmt; clippy `-D warnings` on the workspace, all targets
and features, thumbv8m; workspace, profile, no-std and release tests
with the ignored ones; the bulk run; MSRV 1.75; docs with and without
`std`; `check.sh --stack`), on the timed commit and again on 78b1755.
Raspberry Pi 5: the release library tests with the ignored ones pass
(193; the one test that reads the source tree skipped), 0x0 before and
after.

### Not measured

- Cortex-M33 cycles (emulated instructions only); the M33 on any slot
  but s0086.
- A76 hardware counters (no `perf` on the Raspberry Pi 5).
- The variants not kept were measured in the microbenchmark only, not
  in the decoder.
- The front end's remaining parts outside the butterflies: the window
  shift (`copy_within`, 2 880 values a row; 2.43 M a pass on the M33)
  and the powers and levels (5.6 µs a row on the A76, 16.1 M a pass on
  the M33) were not changed.
- **Next on the M33:** the butterflies, 80% of its feed. A reference
  count of a 3 840-point real FFT for the part has not been made.
- Whether the four-lane first stage is held in vector registers on
  AArch64 and x86-64 was not read from the disassembly; only its time
  was measured (rustc 1.93.0).

## Review follow-ups (O16b, bit-identical, 2026-11-02)

Three code commits on top of O16, one change each: dc9fc22 (one radix-sum
macro), a145178 (the first stage's group, named and asserted) and
4150813 (tests). **The code section of every binary is byte-identical to
78b1755's**, so O16's timings, sizes and identity carry over unchanged.

### Byte comparison of `.text`

Each tree was built at one scratch path: aligned x86-64 and AArch64
`cat-ft8-decode` and `paired_bench`, and the thumbv8m check images
(small, host) and staged measurement images (small, host). The `.text`
sections were compared byte for byte.

| tree | x86-64 (2) | AArch64 (2) | thumbv8m (4) |
|---|---|---|---|
| radix sum as an `#[inline(always)]` function (not kept) | differ: `power_bins`, 2 830 → 2 844 instructions | differ: `power_bins` rescheduled, same count | check images differ (two instructions swapped); staged images identical |
| the same, with the first stage's twiddle as one value per lane | as above, byte for byte | as above | as above |
| the same function in `butterflies` only | differ (`power_bins` +10) | differ | not built |
| **radix sum as a macro (dc9fc22)** | **identical** | **identical** | **identical** |
| per-block window slices, group = lanes (not kept) | differ (+128 B) | differ (−64 B) | differ (+4 B) |
| **group offsets with named constants and asserts (a145178)** | **identical** | **identical** | **identical** |
| either form with a group of four on every target (not kept) | identical to the same form | identical to the same form | differ (+204 to +212 B) |
| **4150813 (head)** | **identical** | **identical** | **identical** |

The function form moved only the transform (`power_bins`). A macro
expands to the loops each caller had, so it changes nothing.

### Per-block window slices (measured, not kept)

The simpler form takes three slices per block instead of one set per
group with offsets. Measured with O16's crate-level check (`feed` and
`feed_i16`, ns per row, 4 alternated runs each, medians over the runs):

| | A76 `feed` | A76 `feed_i16` | x86-64 `feed` | x86-64 `feed_i16` |
|---|---|---|---|---|
| group offsets (78b1755's code) | 34 597 | 34 457 | 20 357 | 20 339 |
| per-block slices | 34 822 (+0.65%) | 34 756 (+0.87%) | 20 327 | 20 342 |

- On the A76 every per-block run is slower than every base run.
- x86-64 cannot separate them: its run-to-run spread is 3%.
- On the M33 the small feed is +0.09 M (191.99 → 192.07 M a pass), and
  flash is +4 B.
- The group-offset form is kept, with its offsets as named constants.
- Validity: the 8 A76 runs are valid, every sample 0x0. One warm-up
  sample was at 1.5 GHz (the first sample of the base's first run); every
  sample during the runs was at 2.4 GHz. A first set of 8 runs was void
  and discarded: a shell quoting error started a program that did not
  exist.

### A group of four on every target (measured, not kept)

The group size is now `GROUP`, separate from `LANES`, with its own
condition (it divides the four blocks whose bases are N/16 apart). Four
on every target changes only the Cortex-M33, where `LANES` is 1:

| M33, s0086, small, per pass | feed | slot | stack | check-image flash |
|---|---|---|---|---|
| group of one (78b1755's code) | 191.99 M | 285.89 M | 10 200 B | base |
| group of four, group offsets | 196.75 M (+2.5%) | 290.81 M | 10 288 B | +212 B |
| group of four, per-block slices | 198.05 M (+3.2%) | 292.18 M | 10 288 B | +204 B |

The group stays the lane count, one on the M33. x86-64 and AArch64 are
unaffected (`.text` identical, as `LANES` is already 4 there).

### Other changes

- **Comments.**
  - The first stage's group is not a set of vector lanes. The compiler
    vectorises each block's butterfly over q.
  - Bit identity is claimed, and tested, for finite input within the
    input limit.
  - The input check is a fold because `all` exits at the first failing
    sample and is not vectorised: checked with rustc 1.93.0 on a copy of
    the loop, which is vectorised as a fold on AArch64 and x86-64 and is
    a scalar loop with an exit as `all` on both.
- **Assertions.** base(b) < N/4 and base(4c) + 3·N/16 < N/4 are now
  asserted in the constant block that states them. `DIGIT_REV`'s first
  element of a block is called its base, no longer n₀.
- **Tests.**
  - After `skip`, `clamped_samples()` is 0.
  - After `feed_i16`, both counters are 0.
  - Mutants that make `skip` return (0, 1), or `feed_i16` (0, 1) or
    (1, 0), now fail; the `skip` mutant passed before.
  - The feed test's comment says that its reference runs through `feed`
    (the copy path), and names the independent anchors: `classify`
    called directly, and the transform's and powers' reference tests.

### Checks

- Local CI at 4150813 passes: fmt; clippy `-D warnings` (workspace, all
  targets and features, thumbv8m); workspace, profile, no-std and
  release tests with the ignored ones; the bulk run; MSRV 1.75; docs;
  `check.sh --stack`.
- `check.sh --stack` at 4150813: flash, static RAM and every frame are
  as at 78b1755. No fused multiply-add and no `f64`; thumbv7em builds.
  The encoder's 45 files are identical.
- Decoder identity was not rerun, since no binary's code changed.

## Closure follow-ups (O16c, 2026-11-02; no code generation change)

Three code commits, one change each, with no new measurements:

- 43079a8: the first stage passes each twiddle as an array with the same
  value in every lane; the one-value type and its `Index` impl are gone.
- 79e8d84: comments and an assertion message only.
  - The macro's doc says the function form compiled differently but was
    not timed, and is to be re-checked after a toolchain change.
  - The group assertion's message names `SPREAD_BLOCKS`.
  - The group bound in the layout check is marked redundant.
  - `GROUP`'s doc names SSE2 and NEON.
- 9dabad0: the first-stage test is renamed
  `first_stage_equals_the_stored_window_and_stage_for_every_group_size`
  (in O16 above, `..._bit_for_bit`), and says group, not lanes.

**`.text` and `.rodata` byte-identical** to 78b1755's on all eight
binaries (x86-64 and AArch64 `cat-ft8-decode` and `paired_bench`;
thumbv8m check and staged images, small and host), each built at one
scratch path from a fresh tree with rustc 1.93.0. Checked after 43079a8
and at 9dabad0. The comparison script now removes every build directory
before building, touches the extracted tree, and records the source
commit, the compiler and both sections' hashes. Local CI and
`check.sh --stack` pass at 9dabad0 (flash, RAM and frames unchanged).

## Belief propagation, seventeenth round (O17, bit-identical, 2026-11-02)

Base: 38045b2 (the code of O16c). The target is belief propagation,
about 21 ms of the A76's 101.3 ms crowded three-pass host slot and 55%
of an empty one. Every candidate was chosen with an out-of-tree
microbenchmark first. Three code commits are kept, one optimisation
each: 6bc283f (the sum-product products over four checks), 63d8dd6
(the inputs in blocks of eight, AArch64 only) and 37ec493 (min-sum over
four checks). Decodes, records, residuals and encoder output are
identical, with no qualification. Cutting the rounds spent on
candidates that fail (over 99% of them) changes results and is not part
of this round.

### Measured first: the round's parts

The microbenchmark is a test module appended to a scratch copy of
`bp/tests.rs`, with `unsafe` allowed in that copy for the counters. It
runs:
- the round as the decoder runs it on failing candidates: 256 noisy
  codewords of eight messages, 30 rounds each from zero messages;
- the round's parts alone, on a mid-decode state;
- `exp2_neg` and `log2` alone, on the values they see in the round;
- the candidate forms.

Each form is checked bit for bit against `sum_product_round` on 456
states over 30 rounds, and on 2 000 single rounds from random messages
(zeros, subnormals and values to ±10⁶ included). A76: core 3, a 3 s
warm-up in the program after the 5 s sampled busy loop, user-space
counters (`perf_event_open`). Every run is valid at 0x0. Figures are
cycles per edge (522 edges a round); the three repetitions in a run
agree within 0.3%.

| part (A76) | cycles per edge | instructions per edge |
|---|---|---|
| the round (`sum_product_round`) | 19.24 | 42.7 |
| inputs (the gather through `EDGE_VARS`) | 3.05 | 11.6 |
| tanh pass, 24 values at a time (4 at a time) | 6.32 (7.65) | 10.8 |
| products, check by check | 2.05 | 3.7 |
| atanh pass, 24 at a time (4 at a time) | 6.81 (8.07) | 11.7 |
| sums (`VAR_EDGES`) | 1.94 | 7.2 |
| `exp2_neg` alone, 24 at a time (4 at a time) | 5.27 (6.21) | 8.4 |
| `log2` alone, 24 at a time (4 at a time) | 4.34 (5.16) | 7.6 |

**What bounds the round.**
- **Not memory, not branches.** L1D read misses are under 0.001 per
  edge (the arrays, 4.9 KB, stay in L1), and mispredictions 0.002 per
  edge.
- **The tanh and atanh passes are 68% of the round.**
  - Their helpers run at 1.6–1.75 instructions a cycle, with backend
    stalls about half the cycles.
  - About three quarters of each pass's instructions are vector
    instructions. So neither four-wide issue (about 2.1 cycles a value
    for `exp2_neg`) nor two vector pipes (about 3.1) is reached at the
    measured 5.3.
  - Twenty-four values side by side are already 15–17% faster than
    four. What limits them further was not identified (no per-pipe
    counters were read).
  - No form with the same operations in the same order was found for
    them, so `exp2_neg` and `log2` are unchanged.
- **The other 36% (inputs, products, sums; 7.0 cycles per edge)** is
  where this round gains.
  - The inputs and the sums are scalar loops issuing 3.7–3.8
    instructions a cycle: issue-bound, with a bounds check, a count and
    a branch per edge or variable.
  - The products are scalar prefix and suffix chains per check, and
    latency-bound (backend stalls half their cycles).

**Forms tried** (the round; A76 cycles per edge, x86-64 ns per edge on
core 5, where the virtual machine's cycle counter dropped out mid-run):

| form | A76 | x86-64 |
|---|---|---|
| the round as it was | 19.24 | 5.43 |
| checks in groups of four (layout), the products as the compiler vectorised them | 18.89 | 5.29 |
| **the same, the products written lane by lane (four checks in four lanes)** | **17.93 (−6.8%)** | 5.33 (−1.8%) |
| that, inputs in blocks of eight, sums in blocks of six | 17.01 (−11.6%) | 5.30 |
| inputs in blocks of eight alone | 18.42 (−4.3%) | 5.52 (+1.6%) |
| sums in blocks of six alone | 19.27 | 5.52 |
| inputs fused into the tanh pass (either layout) | 58.8–61.1 | 12.2–12.4 |
| a whole group's inputs, tanh, products and atanh fused | 52.5 | 12.5 |
| products and atanh fused per check | 31.9 | 14.7 |

- The fused forms were not vectorised (the tanh and atanh chains ran a
  value at a time). Not pursued.
- The products alone, A76 cycles per edge: check by check 2.05; groups
  of four as the compiler vectorised them 1.72; lane by lane, all
  loaded first, 0.82; the same in place, row by row, 1.71; loaded first
  and stored row by row without a staging copy, 1.27.
- Sums in blocks gained only alone (1.94 → 1.77), nothing in the round.
  Not kept.

### Changes

1. **6bc283f: the sum-product products over four checks at a time.**
   - **The layout** (`place`). Each degree's checks are taken in groups
     of `GROUP`. Edge j of a group's k-th check is at the group's first
     edge + j·`GROUP` + k. The checks left over after the last whole
     group (3 of the 59 of degree 6) follow one after another.
   - `GROUP` is four on x86-64 and AArch64. On the Cortex-M33 it is
     one, which is the previous layout, with unchanged tables.
   - **The products.** `group_products` runs a group's prefix and suffix
     products with the four checks in the lanes of each step. Each
     check's multiplications are the same, in the same order. Single
     checks (groups of one, and the leftovers) keep `others_products`.
   - **Why two forms.** One form for both was tried, with rustc 1.93.0.
     Written in place, it lost the A76 gain (1.71 cycles per edge). The
     loaded-first form added a zero fill and a copy per check on the
     M33 (`sum_product_round` 331 → 363–372 instructions). So the form
     follows the group size.
   - **The walkers.** The tables are built from `place` (`var_edges`,
     `edge_vars`). The stopping test, both rounds and the tests walk the
     layout through `check_edges::<G>`. For groups of one, the walk
     keeps each degree's next edge as before, so the M33's code is the
     previous code up to one constant hoisted in `min_sum_round` (below).
     A first form that computed each first edge from its index added
     0.30 M instructions to the M33's small slot, 0.26 M of them in
     `min_sum_round`.
   - **Tests on any host.** Every walker takes the group size as a
     const parameter, and the tests run both layouts (the other size's
     tables are test statics). Table sizes, `TABLE_BYTES` and the
     `--profile` output are unchanged. The crate's array-item scanner
     now skips `const fn` declarations.
   - **Min-sum** copied each interleaved check out and back in this
     commit. That cost the small configuration on hosts (below) and is
     replaced in 37ec493.
   - **Tests:**
     - the layout against the parity-check table, for both group sizes;
     - the sum-product and min-sum rounds in five forms each (both
       targets', and each with the other layout) against the
       check-by-check references, over 20 000 states;
     - the stopping test in both layouts;
     - `decode_soft` against its first form, and the pinned state hash
       (unchanged).
   - **Mutants caught:** a neighbouring lane's suffix in the group
     products; the group stride given to the leftover checks; the
     grouped stopping test skipping an edge; a grouped min-sum check not
     written back.
2. **63d8dd6: the inputs in blocks of eight on AArch64.**
   - `inputs::<8>` takes eight edges per iteration, with one count and
     branch per block. Each edge's subtraction is unchanged.
   - `INPUT_BLOCK` is one elsewhere, where the round keeps the plain
     loop. The x86-64 and M33 binaries' `.text` and `.rodata` are
     byte-identical to 6bc283f's.
   - Test: blocks of 1, 3, 8, 16 and `INPUT_BLOCK` against the plain
     loop, in both layouts. A block that skips its last edge fails it.
3. **37ec493: min-sum over four checks at a time.**
   - Each degree's whole groups run in `group_min_sum`, with the four
     checks in the lanes of each step; then the leftover checks run as
     before.
   - The branches that keep the two smallest magnitudes are selects
     that choose the same values, ties included: both forms make the
     same comparisons.
   - The checks are independent, so their order changes no result.
     Groups of one walk the checks in order as before.
   - Mutants caught: the second smallest never updated; ties taking the
     later edge; a neighbouring lane's sign count; the first leftover
     check skipped.

### A76 (Raspberry Pi 5; `slotcost`, medians of 4 alternated runs)

Aligned builds (`-align-loops=64 -align-all-functions=6`) from one
scratch path, core 3. The throttle, clock and temperature are sampled
once a second from core 0, with a 5 s busy loop on core 3 (sampled)
before each run.

**Host configuration** (`slotcost host host,passes=1 200 5`; the base,
6bc283f and 63d8dd6 alternated):

| build | slot (ms) | BP (spread) | feed (spread) | subtract (spread) | sync | one pass | one-pass BP |
|---|---|---|---|---|---|---|---|
| 38045b2 (base) | 101.30 | 21.010 (0.130) | 19.115 (0.050) | 53.630 (0.030) | 4.680 | 19.30 | 9.870 |
| 6bc283f | 100.45 | 19.850 (0.010), −1.16 | 19.410 (0.020) | 53.600 (0.040) | 4.680 | 18.80 | 9.315 |
| **63d8dd6** | **99.65 (−1.6%)** | **19.050 (0.000), −1.96 (−9.3%)** | 19.400 (0.010) | 53.655 (0.260) | 4.680 | **18.40 (−4.7%)** | **8.940 (−9.4%)** |

- **Placement check.**
  - Between the three AArch64 `paired_bench` builds, only the
    belief-propagation functions differ (compared per function, with
    addresses and immediates normalised). `encode_payload` also
    differs, at equal length; it is outside the timed slot.
  - The untouched stages moved by at most 0.295 ms: the feed, from the
    base to 6bc283f, with identical code, so placement. Sync and soft
    bits moved by 0, the subtraction by up to 0.055.
  - The BP gains are 1.16 ms (3.9 times that largest move) and 0.80 ms
    (6bc283f → 63d8dd6; 15 times the largest untouched move between
    those two, the subtraction's 0.055).
- By pass, BP goes 9.86 / 5.13 / 6.02 → 8.945 / 4.65 / 5.45 ms: every
  pass gains 9–10%.
- The slot gains less than BP (−1.65 against −1.96 ms) because the
  feed's placement moved +0.29 ms.

**The head** (the same benchmark; 63d8dd6 and 37ec493 alternated). 37ec493
changes only `min_sum_round` on AArch64, which the host configuration
does not run:

| build | slot (ms) | BP | feed | subtract | one pass | one-pass BP |
|---|---|---|---|---|---|---|
| 63d8dd6 | 99.60 | 19.050 (0.010) | 19.405 | 53.575 | 18.40 | 8.940 |
| 37ec493 | 99.60 | 19.050 (0.000) | 19.375 | 53.565 | 18.40 | 8.940 |

63d8dd6 agrees with its first measurement (99.65, BP 19.050).

**Small configuration** (`slotcost small small 200 5`; min-sum, one
pass; the base, 6bc283f and 37ec493 alternated):

| build | slot (ms) | BP (spread) | feed (spread) | sync | soft bits |
|---|---|---|---|---|---|
| 38045b2 (base) | 14.40 | 5.580 (0.000) | 6.350 (0.010) | 1.380 | 0.900 |
| 6bc283f (min-sum copies) | 15.60 (+8.3%) | 6.700 (0.000), +20% | 6.440 (0.000) | 1.380 | 0.900 |
| **37ec493** | **12.90 (−10.4%)** | **4.020 (0.020), −28%** | 6.425 (0.010) | 1.380 | 0.900 |

- 63d8dd6 was not timed here: on AArch64 it differs from 6bc283f only
  in `sum_product_round`, which this configuration does not run.
- The untouched stages moved by at most 0.09 ms (the feed); BP's −1.56
  ms is 17 times that.
- 6bc283f's regression is real, and it is the reason for 37ec493. The
  host configuration, the decoder's default, never ran min-sum.

**Validity.** Every timed `slotcost` run is valid: 32 runs (12 host, 12
small, 8 for the head), with 3 324 samples during the runs and 160 in
the warm-ups. Every sample was 0x0, start and end checks included.
- No sample during a run was below 2.39 GHz. Two warm-up samples were
  (minimum 1.5 GHz), each the first of a warm-up, as the busy loop
  ramps.
- Peak 55.4 °C.
- Not counted as evidence: one exploratory host pair (2 runs, valid)
  and the 5 microbenchmark runs. Those were valid at 0x0 too; four of
  their warm-up first samples were at 1.5–1.6 GHz.

### x86-64 (Threadripper 3970X, core 5, `perf stat` user counters over the whole `slotcost` program)

Four alternated runs per build, aligned builds from one scratch path.
Stage times carry the virtual machine's clock bias (`acpi_pm`); the
cycle counts are the measure. 6bc283f and 63d8dd6 are the same x86-64
code (byte-identical `.text`).

| build, configuration | cycles (median, range) | instructions | BP (ms) |
|---|---|---|---|
| 38045b2, host | 353.22 G (352.52–353.95) | 995.45 G | 15.670 |
| 63d8dd6, host | **351.00 G (350.47–352.02), −0.63%** | 987.09 G | 15.310 |
| 38045b2, host (second pair) | 353.61 G (352.21–354.41) | 995.46 G | 15.700 |
| 37ec493, host | **351.06 G (349.67–351.94), −0.72%** | 987.08 G | 15.300 |
| 38045b2, small | 124.02 G (123.46–124.78) | 313.89 G | 5.530 |
| 63d8dd6, small | 130.28 G (129.85–130.56), **+5.0%** | 330.39 G | 6.300 |
| 38045b2, small (second pair) | 124.77 G (123.67–124.83) | 313.90 G | 5.585 |
| 37ec493, small | **97.09 G (96.78–97.77), −22.2%** | 296.82 G | **2.070 (−63%)** |

- The host configuration's ranges do not overlap in either pair. Its
  gain on x86-64 is small, as the microbenchmark showed: SSE2 gains less
  from the grouped products, and the blocked inputs are AArch64 only.
- 6bc283f's min-sum copies cost the small configuration 5% on x86-64;
  37ec493 more than recovers it. (The 37ec493 small pair was measured on
  a build of its working tree whose eight binaries' `.text` and
  `.rodata` are byte-identical to 37ec493's.)

### Cortex-M33 (emulated instructions; staged measurement images, s0086)

| | small slot | small BP | host slot | host BP | stack high water (small / host) |
|---|---|---|---|---|---|
| 38045b2 (base) | 295 769 317 | 54 265 018 | 2 413 281 393 | 365 407 067 | 10 200 / 10 392 B |
| 6bc283f, 63d8dd6 | 295 639 007 (−130 310) | 54 134 708 | 2 413 281 393 | 365 407 067 | 10 200 / 10 392 B |
| 37ec493 | 295 769 317 | 54 265 018 | 2 413 281 393 | 365 407 067 | 10 200 / 10 392 B |

- Same decodes (23 small, 40 host). At 37ec493 every stage's and every
  function's count is the base's, in both images.
- At 6bc283f and 63d8dd6, `min_sum_round` was 2 instructions shorter (a
  constant hoisted out of its loop) and the small slot 0.04% lower.
  37ec493's `min_sum_round` is the base's code with two registers
  swapped.
- `sum_product_round` is the base's code at every commit, up to one
  addressing mode and the operand order of one multiplication. Its
  inputs are tanh values in [−1, 1], never NaN.
- 63d8dd6 leaves all four M33 images byte-identical to 6bc283f's.

### Sizes and stack (`check.sh --stack`, thumbv8m)

| image | 38045b2 | 6bc283f, 63d8dd6 | 37ec493 |
|---|---|---|---|
| small | 89 734 B | 89 722 B | 89 734 B |
| host | 101 430 B | 101 418 B | 101 430 B |
| small, OSD on / host, OSD off | 89 734 / 101 430 B | 89 722 / 101 418 B | 89 734 / 101 430 B |
| encoder only | 34 766 B | 34 766 B | 34 766 B |

- Static RAM is unchanged (212 048 / 241 616 / 0 B).
- Every recorded frame is unchanged at 63d8dd6 and 37ec493 (`_start`
  4 104, `decode_a_slot` 2 512, host `subtract_one` 4 808, the
  subtraction's closure 2 872, `osd_decode` 272 B).
- No fused multiply-add (`vmla` 75 / 146 / 13) and no `f64` helper in
  any image; thumbv7em builds.
- Hosts, `paired_bench` `.text` at 37ec493: x86-64 +5 120 B, AArch64
  +6 400 B.

### Identity

- **Records, x86-64:** the 73 800 `paired_bench run` records of 63d8dd6
  (whose x86-64 code is 6bc283f's) and of 37ec493 equal b93516e's
  (0 differ).
- **800 WAV slots** × `host`, `small`, `host,passes=1` × plain, `--raw`,
  `--profile` (7 200 runs, 78 690 decode lines): byte-identical to
  b93516e's at 63d8dd6 and at 37ec493, `--profile` metadata included.
- **Encoder:** the 45 `cat-ft8-encode` WAV files are identical at
  63d8dd6 and 37ec493.
- **Cortex-M33 (emulated):** the small and host dump images' decodes and
  statics on seven slots (14 of 14), and the multi-pass residuals on
  s0086 and s0407, are byte-identical to 68634cc's, at 63d8dd6 (whose
  M33 code is 6bc283f's) and at 37ec493.
- **A76:** the 73 800 records of 63d8dd6's and of 37ec493's AArch64
  `paired_bench` equal x86-64 b93516e's (0 differ); the 800 WAV slots ×
  3 × 3 with 37ec493's AArch64 `cat-ft8-decode` are byte-identical to
  x86-64 b93516e's (7 200 runs, 78 690 decode lines). Not timed; 0x0
  before and after each part.

### Checks

- Local CI passes at 37ec493: fmt; clippy `-D warnings` (workspace, all
  targets and features, thumbv8m); workspace, profile, no-std and
  release tests with the ignored ones; the bulk run; MSRV 1.75; docs
  with and without `std`; `check.sh --stack`.
- Raspberry Pi 5: the release library tests of 37ec493 with the ignored
  ones pass (194; the one test that reads the source tree skipped), 0x0
  before and after. They include both layouts' round, stopping-test and
  layout tests, the blocked inputs and the 2³² `tanh_half` test on
  AArch64.
- `.text` and `.rodata` of all eight binaries were compared between the
  commits at one scratch path (x86-64 and AArch64 `cat-ft8-decode` and
  `paired_bench`, thumbv8m check and staged images): 63d8dd6 against
  6bc283f differs only in the two AArch64 binaries, as intended.

### Not measured

- Cortex-M33 cycles (emulated instructions only). M33 counts on s0086
  only; M33 identity on seven slots.
- What limits the tanh and atanh passes on the A76 beyond issue: no
  per-pipe counters were read. `exp2_neg` and `log2` are unchanged.
- The microbenchmark ran on A76 core 3 only. The forms were not measured
  on other AArch64 or x86-64 cores. Groups of four match 128-bit vectors
  (SSE2, NEON); wider vectors were not tried.
- The A76 records of 6bc283f (its AArch64 code differs from 63d8dd6's
  only in `sum_product_round`'s inputs loop).
- x86-64 stage times carry the virtual machine's clock bias.
- **After a toolchain change**, the group forms depend on the compiler
  keeping a group's four lanes in one vector (rustc 1.93.0). A change
  can lose that with every test passing. Run `cat-ft8/codegen-check.sh`
  (O17b, O17c), and re-run the A76 `slotcost` in both configurations if
  it fails. The check counts the release library's AArch64 assembly (one
  codegen unit): `sum_product_round` 1 457 instructions, 696 of them
  `.4s`, and `min_sum_round` 1 273 and 156. (The 1 552 and 1 934 given
  here before were counted in the aligned, linked `paired_bench`, a
  different build; use the check's counts.)

## Review follow-ups (O17b, bit-identical, 2026-11-02)

On top of O17 (aec6a07), one change per commit. **The `.text` and
`.rodata` of all eight binaries are byte-identical to aec6a07's at every
commit** (x86-64 and AArch64 `cat-ft8-decode` and `paired_bench`;
thumbv8m check and staged images, small and host; one scratch path,
fresh tree, rustc 1.93.0). So O17's timings, sizes and identity carry
over unchanged, and none was rerun.

### Changes

- **a034ecd: `place`'s doc.** It said every place takes the layout from
  `place`. The tables and the walk in check order are built from it, and
  the walk's group-of-one case unrolls it. The rounds' group loops
  (`products`, `min_sum_round`) do not call it; they rely on its shape,
  contiguous blocks of D·G values for the whole groups and then the
  leftover checks of D values. Comment only.
- **4813087: two comments keep the reason only.** `INPUT_BLOCK` and
  `group_products` no longer carry measurement history; they point to
  these findings (O17).
- **eb9f723: `group_products`' scratch has exactly D rows** (`[[f32; K];
  D]`, no `.take(D)`). Byte-identical everywhere.
  - Not taken this round: the same change in `group_min_sum`. It changed
    `min_sum_round` on both hosts (x86-64 1 447 → 1 414 instructions,
    AArch64 rescheduled at equal length). A follow-up to measure in its
    own commit, with identity and timing.
- **508fc10: `inputs`' doc says the blocked round is tested only on
  AArch64.** `INPUT_BLOCK` is not a parameter of the round; elsewhere the
  tests compare `inputs` itself at several block sizes.
  - Not kept: the `INPUT_BLOCK == 1` plain loop moved into `inputs`
    (inside an if/else, with the blocked loop in a helper). It changed
    `.text` on all three targets.
- **3fe54a0: tests.**
  - A quarter of the min-sum round test's states carry NaNs of both
    signs, ±∞, −0 and ±`f32::MAX` in the totals, messages and soft bits.
    A mutant that differs only for NaN (`!(a >= min1)` for `a < min1`)
    passed the old test and fails the new one (logged).
  - The layout test is renamed
    `the_edge_layout_places_every_edge_once_for_both_group_sizes`.
- **d9dadbd: a code-generation guard, `cat-ft8/codegen-check.sh`.**
  - It emits the release library's AArch64 assembly (one codegen unit,
    about 4 s) and fails if `sum_product_round` has fewer than 500
    four-lane `.4s` instructions, or `min_sum_round` fewer than 100.
    rustc 1.93.0 gives 696 and 156.
  - It runs in the aarch64 CI job and in the local CI. Each
    `toolchain: "1.93"` pin in the workflow says to re-check it after a
    toolchain change.
  - **It fires** (logged, each forced change then reverted):

| forced change | `sum_product_round` | `min_sum_round` | result |
|---|---|---|---|
| none (d9dadbd) | 696 | 156 | pass |
| `GROUP` one on every target | 709 | **29** | fails |
| `PASS_LANES` 4 | **452** | 156 | fails |
| `PASS_LANES` 8 | **177** | 156 | fails |
| `PASS_LANES` 16 | **301** | 156 | fails |
| `PASS_LANES` 32 (not unrolled, as in O12) | **115** | 156 | fails |

  - **Limit:** the sum-product round's count is dominated by its
    24-value tanh and atanh blocks, so the guard does not see the group
    products on their own. Groups of one leave the round's count at 709.
    The products' vector form is guarded only by the A76 timing.

**Tried, a follow-up to measure: the layout's tables as associated
constants**
(`Layout<G>`, in place of `tables<G>()`'s `#[cfg(test)]` hook into the
tests' copies).
- Constant promotion works on Rust 1.75. The tests pass, and the x86-64
  and AArch64 binaries are byte-identical.
- The thumbv8m images are not. Their functions are identical once
  addresses are normalised, but `.rodata` is 4 B smaller and `.text`
  differs (the tables are placed differently).
- This round required byte identity on every target, so the current
  form stays for now: `tables<G>()` returns the statics for `GROUP` and,
  in the tests only, the tests' copies for the other size. Taking the
  associated constants is a follow-up for a measured round: the M33
  images' identity and counts, since only placement changed there.

### Evidence record corrected

- **Mutants.** O17 ran nine, as its section lists (4 for 6bc283f, 1 for
  63d8dd6, 4 for 37ec493). An earlier count of 13 was wrong. All nine were rerun against the commits they were written
  for, one log each, and all are caught. The NaN mutant above makes ten.
- **The A76 library tests.** O17's run (194 passed) logged the binary's
  hash (d5ccf172…) but not its source commit. The run at this round's
  head records both: the binary was built from d9dadbd (`cat-ft8` tree 3918feb7…, the same
  at the planning commit after it; rustc 1.93.0), sha256 725d2ba1…, the
  same hash on the Raspberry Pi 5. 194 passed with the ignored ones (the
  one test that reads the source tree skipped), 0x0 before and after.

### Checks

- Local CI passes at d9dadbd: fmt; clippy `-D warnings` (workspace,
  all targets and features, thumbv8m); workspace, profile, no-std and
  release tests with the ignored ones; the bulk run; MSRV 1.75; docs with
  and without `std`; `check.sh --stack` (flash 89 734 / 101 430 B, as
  O17); the code-generation guard.
- The Raspberry Pi 5 library tests: above.

### Not done this round

- Unifying `min_sum_round`'s two shapes (the groups-of-one walk and the
  degree split) changes the M33 code; it is left for its own measured
  round.
- The series keeps 6bc283f's measured small-configuration regression
  (fixed by 37ec493); O17 records it.

## Closure follow-ups (O17c, 2026-11-02; no code generation change)

On top of O17b (8503bc9), one change per commit:
- ae3d080, c0c265b, cec2739 and 1620c52 change only the code-generation
  check;
- 6827dd9 changes a test comment.

The `.text` and `.rodata` of all eight binaries are byte-identical to
aec6a07's at c0c265b and at 6827dd9 (one scratch path, rustc 1.93.0).
The commits between them change no `.rs` file.

### The code-generation check

- **ae3d080: it cleans the library first.** On a second run cargo saw
  the library as fresh, ran no compiler and wrote no assembly, so the
  check stopped with an error. It never passed falsely. It now runs
  `cargo clean` for the package and target before building. Two runs back
  to back both pass (logged, before and after).
- **c0c265b: it counts in awk.** `grep -P` is GNU only. The counts are
  unchanged (696, 156) with mawk (this machine's awk) and busybox awk;
  GNU awk was not available to try.
- **cec2739: a ceiling on single-lane moves in `sum_product_round`.**
  - The metric is `mov`, `ins`, `dup`, `ld1` and `st1` on a `.s[i]`
    element, counted with a ceiling of 25.
  - The `.4s` floor did not see the group products; this does. Each check
    is a function, a metric and a bound. The forced changes (logged, each
    reverted):

| forced change | `.4s`, sum-product (floor 500) | single-lane moves, sum-product (ceiling 25) | `.4s`, min-sum (floor 100) | result |
|---|---|---|---|---|
| none (the head) | 696 | 15 | 156 | pass |
| group products in place, row by row (O17's 1.71 cycles per edge form) | 695 | **46** | 156 | fails |
| group products loaded first, stored row by row (O17's 1.27 form) | 771 | 15 | 156 | **passes** |
| `GROUP` one on every target | 709 | **38** | **29** | fails |
| `PASS_LANES` 4 / 8 / 16 / 32 | **452 / 177 / 301 / 115** | 15 | 156 | fails |

  - **The in-place form, timed** (Raspberry Pi 5, `slotcost host
    host,passes=1 200 5`, aligned builds, two alternated runs each).
    Belief propagation costs 0.99 ms more, against untouched stages that
    moved at most 0.10 ms:

| build | slot (ms) | BP | feed | subtract | one pass | one-pass BP |
|---|---|---|---|---|---|---|
| the head (c0c265b; the same code as aec6a07) | 99.60 | 19.050 | 19.390 | 53.575 | 18.40 | 8.940 |
| the head with in-place group products | 100.65 | 20.040 (+5.2%) | 19.370 | 53.675 | 18.90 | 9.410 |

  - All four runs are valid: 555 samples during the runs and 20 in the
    warm-ups, every one 0x0, start and end included. No sample was below
    2.39 GHz. Peak 54.3 °C.
  - **Limit.** The intermediate form, about half the in-place form's
    loss in the O17 microbenchmark (1.27 against 0.82 cycles per edge, or
    roughly 2% of the round), passes every bound. Neither metric
    separates it.
- **1620c52:** the script's comment notes the thin margin, 452 for
  passes of 4 values against the floor of 500.

### Other changes

- **6827dd9:** the min-sum round test says why comparing NaN bits is
  sound: a NaN is never one of the two smallest magnitudes, and a NaN
  sum comes from the same additions in every form.
- **Mutants.** The NaN-only mutant's diff is now in the mutant script, as
  mutant ten. It survives the test before the special values (37ec493)
  and is caught by the test with them (3fe54a0), with one log each.
- **Records.** Three corrections, made in place:
  - O17's toolchain re-check now points at the code-generation check and
    its library counts (1 457 / 1 273 instructions, `.4s` 696 / 156).
  - O17b's associated-constant tables and sized min-sum scratch are
    recorded as follow-ups to measure, not as final rejections.
  - The mutant-count note now reads "An earlier count of 13 was wrong."

### Checks

Local CI passes at the head, including the code-generation check and
`check.sh --stack` (flash 89 734 / 101 430 / 34 766 B, unchanged).

## The subtraction, eighteenth round (O18, bit-identical, 2026-11-02)

Base: e24272f (the code of O17c). The target is the signal subtraction,
53.6 ms of the A76's 99.6 ms crowded three-pass host slot. It was
profiled first, stage by stage, on the A76. Three code commits are kept,
one optimisation each: a17baa6 (the input clamp without a branch),
7d747e3 (one phase increment per symbol where the pulse is alone) and
513fd17 (the reference without an envelope of one). Decodes, records,
residuals and encoder output are identical, with no qualification.

### Measured first: the subtraction's parts

The microbenchmark is O13's, brought up to date: a test module appended
to a scratch copy of `subtract/tests.rs`, `unsafe` allowed in that copy
for the counters (`perf_event_open`, user space) and a PC sampler. It
runs O13's crowded synthesized slot (24 signals, −18 to +6 dB, decodes a
little off the truth) on A76 core 3 after the 5 s sampled busy loop.
Each part is timed alone, five repetitions per round, three rounds
(they agree within 0.1%). Every run was valid at 0x0. Figures are
cycles per waveform sample (151 680 a subtraction); the time search's
are per sample of the symbols' middle halves (75 840 a step) and are
halved where they are added to the rest. In this workload every
subtraction takes one time-search step; the decoder's crowded slots
average about 1.2.

| part (A76, e24272f) | cycles | instructions | IPC | vector ops | floor | ÷ floor |
|---|---|---|---|---|---|---|
| **the subtraction** | **24.54** | 54.8 | 2.23 | 28.5 | 14.3 | 1.72 |
| time search, per core sample: generator (fill the cores, skip the rest) | 5.23 | 13.5 | 2.58 | 4.3 | 3.4 | 1.55 |
| … + the products (polynomial, clamp, store) | +6.40 | +12.1 | | +10.4 | 5.2 | 1.23 |
| … + the lane sums | +0.66 | +2.8 | | +1.2 | | |
| step sums and alignment (per core sample, one step) | 1.35 | 3.4 | 2.49 | 1.2 | | |
| fit path: generator (`fill_phase`) | 3.13 | 8.3 | 2.66 | 2.0 | 2.1 | 1.5 |
| … + the frequency correction | +1.77 | +6.1 | | +0.0 | 1.6 (integer) | 1.1 |
| … + the reference (polynomial, envelope, store) | +5.33 | +9.6 | | +8.4 | 4.2 | 1.27 |
| … + the fit (fused) | +3.62 | +8.0 | | +4.1 | | |
| the fit loop alone | 3.19 | 7.3 | 2.28 | 3.8 | 1.88 | 1.70 |
| the removal alone | 4.33 | 8.0 | 1.85 | 5.8 | 2.88 | 1.50 |

(The floor is the larger of instructions ÷ 4, vector operations ÷ 2 and,
for the integer parts, data-processing operations ÷ 3.)

**PC samples of the whole subtraction** (35 790 samples, 23 µs apart):
the fused fit's block body (the reference loop, 149 AArch64
instructions per 16 samples, and the correction) 28.9%; the removal
17.0% (one loop, 28 instructions per four samples); `increments` 15.4%;
the time search's products and sums 14.5%; `Fit4::add_all` 12.5%;
`fill_phase` (the running sum and envelope) 7.5%.

**What bounds it.**
- **Not memory and not branches**, as in O13: L1D refills under 0.08 a
  sample, last-level misses 0.001, branch misses 0.006.
- **Issue and latency of vector work, in that order.** The polynomial
  passes (the reference and the products, 35% of the subtraction) are
  within 1.23–1.27× their vector floor. The fit loop (1.7×) and the
  removal (1.5×, IPC 1.85) are not.
- **The removal's clamp was the largest single waste.** `input`'s branch
  became compares and selects: 11 of the loop's 23 vector instructions
  per four samples (`fcmgt` ×4, `fabs`, `bit`, `bsl`, `orr`, `bic`,
  `and`, a move). The fit already used a branch-free form (`input4`:
  `fmaxnm`, `fminnm`, a mask) for the same values.
- **The generator is 23% of the subtraction**, and part of it is
  redundant: within each symbol, from the end of the previous symbol's
  pulse to the start of the next one's (509 of 1 920 samples), the
  current pulse is exactly 1.0 in the table, so every increment there is
  the same value; and the envelope is exactly 1.0 except in the first
  and last blocks, yet every block filled it and the reference multiplied
  by it.

### Candidates

Each was measured in the microbenchmark before a commit was made
(cycles per sample of the whole subtraction, variant trees not on the
branch, in the order tried):

| form | subtraction | changed part |
|---|---|---|
| e24272f | 24.54 | |
| the removal's clamp without a branch | 23.60 (−3.8%) | removal 4.33 → 3.40 |
| that, and one increment per symbol where the pulse is alone | 23.11 (−2.1%) | fit-path generator 3.12 → 2.88; time-search generator 5.25 → 4.78 per core sample |
| that, and the products' clamp without a branch | 22.89 (−1.0%) | time-search correlations 11.78 → 11.36 per core sample |
| that, and no envelope where it is 1.0 | 22.09 (−3.5%) | generator 2.85 → 2.63; reference 5.33 → 4.81 |
| (not kept) the reference and products in two planes, the clamp in the removal only | 31.32 (+33%) | fit loop 3.20 → 9.27, reference 5.35 → 6.13, removal 3.40 → 3.12 |

- **The two-plane layout** (cosines and sines in separate byte planes,
  so lanes load and store without interleaving) was meant to remove the
  `ld2`/`st2` and the fit's second load of the sines. With rustc 1.93.0
  the loop vectoriser took the fit's lane loop as four iterations side
  by side (`ld4`, sums spilled to the stack every iteration) and the
  reference's stores were split. The removal gained 0.28 cycles a
  sample. Not pursued; recorded as tried.
- The two clamp changes are one optimisation (the same helper in the two
  loops that took `input` per sample) and are one commit.

### Changes

1. **a17baa6: the input clamp without a branch where it vectorises.**
   - `input_select(x)` is `input(x)` written as `max`, `min` and a mask
     on the finite bit pattern, as `input4` already did lane by lane;
     `input4` is now four lanes of it. The removal and the time
     search's products take it; the scalar edges of the fit keep
     `input`.
   - AArch64 (the aligned `paired_bench`): the removal's loop is 22
     instructions per four samples (27 before); `Removal::refs` 734 → 727
     and `symbol_correlations` 837 → 821 instructions.
   - On the M33 it is shorter too (`vmaxnm`/`vminnm`, no branch):
     `Removal::refs` 168.0 M → 145.6 M instructions on s0086.
   - Tests: `input_select` against `input` bit for bit at every 4 099th
     bit pattern and the special values, and (ignored, release) at every
     one of the 2³² patterns.
2. **7d747e3: one phase increment per symbol where its pulse is alone.**
   - From `PREV_END` (706) to `PULSE_FIRST` (1 215) only the current
     pulse is non-zero, and `PULSE_HALF` is exactly 1.0 there (indices
     1 411–1 665, read directly before the centre and mirrored after
     it). So `cur · g` is `cur`, and every increment there is
     `step(cur)`. `increments` computes it once per symbol and fills the
     run; the two runs that took the pulse, the product and the
     conversion per sample are gone, and `MIRROR` with them.
   - The values are the same, so the generator, the encoder and the
     subtraction are unchanged. AArch64 `increments` 553 → 407
     instructions; M33 `increments` 163.7 M → 136.7 M on s0086, and the
     encoder-only image 200 B smaller.
   - Test: the pulse is exactly 1.0 at every sample of the run, read
     directly or mirrored (`the_pulse_is_exactly_one_where_it_is_alone`).
3. **513fd17: the reference without an envelope of one.**
   - `blocks` calls `Synth::fill_phase_only` (the phase alone, new,
     `pub(crate)`) for every block clear of the ramps, and passes no
     envelope; `fill_phase` fills it only for the first and last blocks.
     `reference` then stores the cosine and sine themselves. A value
     times 1.0 is itself, so the reference is the same; the time search
     never read the envelope.
   - `scaled_cos_sin` takes its weights' type as a parameter; without an
     envelope it is called with a constant weight of 1.0, which the
     compiler folds away. That is a second instance of the loop (on the
     hosts the 16-sample block, on the M33 the scalar loop): `.text`
     grows (below).
   - Tests: the reference without an envelope against the reference
     times an envelope of ones, byte for byte, at every length up to
     three blocks of `CHAINS` and beyond; `blocks` against `fill_phase`
     over the whole waveform, with no envelope exactly where it is 1.0
     at every sample of the block.
- **Mutants caught** (one log each): `input_select` without its finite
  mask (4 tests fail, among them `input_select_is_input`); the flat run
  filled with `step(prev)` (36 fail); the flat run started one sample
  early (the per-sample generator test fails); the last ramp block
  passed without an envelope (5 fail, among them the new `blocks` test).

### A76 (Raspberry Pi 5; `slotcost host host,passes=1 200 5`, medians of 4 alternated runs)

Aligned builds (`-align-loops=64 -align-all-functions=6`) from one
scratch path, core 3, the throttle, clock and temperature sampled once a
second from core 0, a 5 s busy loop on core 3 (sampled) before each run.
The four builds were alternated in one sequence (forward, then
reversed).

| build | slot (ms) | subtract (spread) | pass 2 / pass 3 subtract | feed | BP | sync | one pass |
|---|---|---|---|---|---|---|---|
| e24272f (base) | 99.65 | 53.625 (0.100) | 37.355 / 16.270 | 19.370 | 19.050 | 4.680 | 18.40 |
| a17baa6 | 97.30 | 51.295 (0.140), −2.33 | 35.735 / 15.565 | 19.370 | 19.050 | 4.680 | 18.40 |
| 7d747e3 | 96.00 | 49.985 (0.050), −1.31 | 34.825 / 15.165 | 19.385 | 19.050 | 4.680 | 18.40 |
| **513fd17** | **94.00 (−5.7%)** | **48.115 (0.020), −1.87; −5.51 (−10.3%) in all** | 33.515 / 14.595 | 19.275 | 19.050 | 4.680 | 18.40 |

- **Placement check.** Between consecutive AArch64 `paired_bench`
  builds only the subtraction's functions and the generator differ
  (compared per function, addresses and immediates normalised):
  `Removal::refs` and `symbol_correlations`; then `increments` alone;
  then `blocks`, `run`, `symbol_correlations` and the new
  `fill_phase_only`, plus `osd::reliability_order` at equal length. The
  untouched stages moved by at most 0.110 ms (the feed, 7d747e3 →
  513fd17, identical code); sync, soft bits and belief propagation by
  0. Each subtraction gain is at least 12 times that largest move and
  at least 9 times the stage's own spread.
- By pass: 18.34 / 50.73 / 30.55 → 18.30 / 46.86 / 28.84 ms; per new
  decode 0.76 / 4.83 / 10.18 → 0.76 / 4.46 / 9.61 ms. The one-pass slot
  runs no subtraction and does not move. Worst slot 107.2 → 100.7 ms.
- The base agrees with O17's measurements (99.60, subtraction 53.575).
- **Validity.** All 16 timed runs are valid: 2 156 samples during the
  runs and 80 in the warm-ups, every one 0x0, start and end checks
  included. No sample during a run was below 2.39 GHz; one warm-up
  sample was (1.5 GHz, the first of the first warm-up). Peak 55.4 °C.
  The 19 microbenchmark runs were valid at 0x0 as well, start and end
  included (in seven, the first warm-up sample was at 1.5–1.6 GHz);
  their cycle counts, not times, are what is reported from them.

### x86-64 (Threadripper 3970X, core 5, `perf stat` user counters over the whole `slotcost` program)

Four alternated runs per build (the four builds in one sequence),
aligned builds from one scratch path. Cycles are the measure (the
virtual machine's clock biases its stage times).

| build | cycles (median, range) | instructions |
|---|---|---|
| e24272f | 350.74 G (348.59–350.95) | 987.08 G |
| a17baa6 | 348.20 G (345.92–349.29), −0.7% | 978.30 G |
| 7d747e3 | 339.85 G (339.42–340.72), −2.4% | 955.82 G |
| 513fd17 | **334.76 G (334.48–336.30), −1.5%; −4.6% in all** | 949.68 G (−3.8%) |

- a17baa6's range overlaps the base's: on x86-64 its gain is not
  separated from run-to-run spread (instructions −0.9%). The other two
  are.

### Cortex-M33 (emulated instructions; staged measurement images, s0086)

| | host slot | subtraction (37 calls) | per call | `increments` | `Removal::refs` | stack high water (host / small) |
|---|---|---|---|---|---|---|
| e24272f | 2 403.40 M | 1 332.26 M | 36.007 M | 163.68 M | 168.01 M | 10 392 / 10 200 B |
| a17baa6 | 2 375.44 M | 1 304.31 M | 35.251 M (−2.1%) | 163.68 M | 145.64 M | 10 392 / 10 200 B |
| 7d747e3 | 2 348.87 M | 1 277.73 M | 34.533 M (−2.0%) | 136.69 M | 145.64 M | 10 372 / 10 200 B |
| **513fd17** | **2 270.99 M (−5.5%)** | 1 199.85 M | **32.428 M (−6.1%; −9.9% in all)** | 136.69 M | 145.64 M | 10 316 / 10 192 B |

- Same decodes (40 host, 23 small). The small one-pass slot (no
  subtraction) is unchanged at 285 888 810 instructions; only the
  encoder checks before it (setup) shrink, with `increments`.
- At 513fd17 `fill_phase` falls 73.29 M → 2.56 M (only the ramp blocks)
  and `blocks` 407.2 M → 391.4 M.
- No M33 count rises at any commit.

### Sizes and stack (`check.sh --stack`, thumbv8m)

| image | e24272f | a17baa6 | 7d747e3 | 513fd17 |
|---|---|---|---|---|
| small (and small, OSD on) | 89 734 B | 89 734 B | 89 534 B | 89 534 B |
| host (and host, OSD off) | 101 430 B | 101 378 B | 101 178 B | 101 810 B (+380 on the base) |
| encoder only | 34 766 B | 34 766 B | 34 566 B | 34 566 B |

- Static RAM is unchanged (212 048 / 241 616 / 0 B).
- Frames: host `subtract_one` 4 808 → 4 816 B at 513fd17; the
  subtraction's closure 2 872 → 2 880 B at a17baa6 and 7d747e3 and back
  to 2 872 B at 513fd17. The others are unchanged (`_start` 4 104,
  `decode_a_slot` 2 512, `osd_decode` 272 B). The measured high water
  falls (table above).
- 513fd17's host image is 632 B larger than 7d747e3's: the second
  instance of `scaled_cos_sin` (with and without an envelope), on the
  M33 a second scalar loop (`CHAINS` is 1 there; corrected in O18b).
  Hosts' `.text` grows with it, where each instance is the 16-sample
  block: AArch64 `paired_bench` +4 992 B, x86-64 +8 128 B (on 7d747e3).
- No fused multiply-add (0 `vfma`/`vfms`/`vfnma`/`vfnms`; `vmla` 75 /
  146 / 13 → 74 / 157 / 12) and no `f64` helper in any image; thumbv7em
  builds.

### Identity

- **Records, x86-64:** the 73 800 `paired_bench run` records of
  a17baa6, 7d747e3 and 513fd17 equal b93516e's (0 differ). (Corrected in
  O18b: the comparisons in this round's log named an empty reference
  directory and all failed; the result above came from a comparison
  repeated by hand. O18b reruns it from the script and logs it.)
- **800 WAV slots** × `host`, `small`, `host,passes=1` × plain, `--raw`,
  `--profile` (7 200 runs, 78 690 decode lines): byte-identical to
  b93516e's at all three, `--profile` metadata included.
- **Encoder:** the 45 `cat-ft8-encode` WAV files are identical at all
  three.
- **Cortex-M33 (emulated):** the small and host dump images' decodes and
  statics on seven slots (14 of 14), and the multi-pass residuals on
  s0086 and s0407, are byte-identical to 68634cc's at all three commits.
- **A76:** the 73 800 records of a17baa6's, 7d747e3's and 513fd17's
  AArch64 `paired_bench` equal x86-64 b93516e's (0 differ); the 800 WAV
  slots × 3 × 3 with 513fd17's AArch64 `cat-ft8-decode` are
  byte-identical to x86-64 b93516e's (7 200 runs, 78 690 decode lines).
  Not timed; 0x0 before and after each part.

### Checks

- Local CI passes at 513fd17: fmt; clippy `-D warnings` (workspace, all
  targets and features, thumbv8m); workspace, profile, no-std and
  release tests with the ignored ones; the bulk run; MSRV 1.75; docs with
  and without `std`; `check.sh --stack`; the code-generation check.
- Raspberry Pi 5: the release library tests of 513fd17 with the ignored
  ones pass (199; the one test that reads the source tree skipped), 0x0
  before and after. The binary was built from 513fd17 (`cat-ft8` tree
  73e71c76…, rustc 1.93.0), sha256 0f5ac229…, the same hash on the
  Raspberry Pi 5. They include the exhaustive `input_select` test on
  AArch64.

### After the round: what bounds the subtraction now

The microbenchmark at each commit (A76, cycles per waveform sample,
one run each, all valid at 0x0):

| part | e24272f | a17baa6 | 7d747e3 | 513fd17 |
|---|---|---|---|---|
| **the subtraction** | **24.55** | 23.48 | 22.88 | **22.06 (−10.1%)** |
| time-search correlations (per core sample) | 12.29 | 11.93 | 11.33 | 11.08 |
| of which the generator | 5.25 | 5.23 | 4.69 | 4.48 |
| fit-path generator | 3.13 | 3.13 | 2.84 | 2.63 |
| + correction | 1.78 | 1.77 | 1.78 | 1.79 |
| + reference | 5.32 | 5.35 | 5.35 | 4.79 |
| + fit (fused) | 3.63 | 3.62 | 3.63 | 3.43 |
| removal alone | 4.33 | 3.40 | 3.39 | 3.39 |
| step sums and alignment (per core sample) | 1.36 | 1.35 | 1.35 | 1.35 |

At 513fd17 the subtraction is 49.7 instructions and 25.1 vector
operations a sample, a floor of 12.5 cycles against the measured 22.1
(1.76×; 1.72× at the base: the round removed work, not stalls). Its
parts, per waveform sample (one time-search step):

- the polynomial passes, the reference (4.79) and the time search's
  products (3.1), 36%, at 1.2–1.3× their vector floor;
- the generator, 4.9 (22%): `increments` and the running sum, whose
  carried add is about one cycle a sample (O15);
- the fit, 3.4 (16%): its loop is 1.7× its floor, with the sines loaded
  a second time lane by lane (O13);
- the removal, 3.4 (15%): 1.6× its vector floor, IPC 1.9, the
  references through `ld2`;
- the correction, 1.8 (8%), near its integer floor; the alignment, 0.7
  (3%) a step.

Next candidates, not tried: the running sum over the flat runs in closed form
(the phase there is the run's start plus a multiple of one increment;
about 1–1.5% by the counts above); the fit's and the removal's loads,
if a two-plane layout can be had without the loop vectoriser's
interleave (the removal alone gained 0.28 cycles a sample with it); and,
on slots where a second time-search step runs (about 0.2 a subtraction),
keeping each symbol core's starting phase from the first step instead of
running the generator over the skipped samples again.

### Not measured

- Cortex-M33 cycles (emulated instructions only), and the M33 on any slot
  but s0086 for counts (seven slots for identity).
- The microbenchmark ran on A76 core 3 only; the forms were not timed on
  other cores. On x86-64 only the whole program's counters were read.
- How the gain varies with slot content: the timed slots are the crowded
  40-message `main` block, and the microbenchmark's workload takes one
  time-search step per subtraction.
- A second build pair: no result was near a bar (each gain is more than
  nine times its stage's spread and twelve times the largest untouched
  move).
- **After a toolchain change**: the gains depend on the compiler keeping
  `input_select` as `fmaxnm`/`fminnm` and a mask in the removal and
  products loops, and folding the weight of 1.0 in the reference without
  an envelope (rustc 1.93.0). Neither is in the code-generation check.
  The removal's loop is 22 AArch64 instructions per four samples in the
  aligned `paired_bench` here.

## Review follow-ups (O18b, 2026-11-02; no code generation change on the measured targets)

On top of O18 (26f66ad), one change per commit: 8dc7a85, a6feb13,
75729ab, 6f389c4 and 85ba626. **The `.text` and `.rodata` of all eight
binaries are byte-identical to 513fd17's at every commit** (x86-64 and
AArch64 `cat-ft8-decode` and `paired_bench`; thumbv8m check and staged
images, small and host, built with `--cfg cat_ft8_minmaxnm`; one scratch
path, fresh tree, rustc 1.93.0). So O18's timings, counts, sizes and
identity carry over unchanged.

### Changes

1. **8dc7a85: the branch-free clamp only where `max` and `min` are
   instructions.**
   - On thumbv7em (and 32-bit ARMv7) `f32` `max` and `min` are library
     calls, so `input_select` put `fmaxf`/`fminf` calls into the
     subtraction's per-sample loops: 20 in `Removal::refs`, 6 in
     `symbol_correlations`, and 8 in `Fit4::add_all` (from `input4`,
     there since O13) in the thumbv7em library's assembly.
   - `INPUT_SELECT` now picks the form: the branch-free one on x86-64,
     AArch64 and targets built with `--cfg cat_ft8_minmaxnm`, the branch
     elsewhere. The thumbv7em subtraction has no such calls left (34 →
     0). The 22 elsewhere in the thumbv7em library (`finish_row`,
     `sum_product_round`, `snr::estimate`, `soft_bits_at`, `ratio_db`)
     are older than this round and unchanged.
   - **Why a hand-set cfg.** Stable Rust exposes no ARM target feature
     (`--print cfg` lists none for thumbv8m.main or thumbv7em), so no
     `cfg` tells the two apart. A build script reading the target name
     was tried: its presence alone, before any use, changed the code on
     every measured target (43 AArch64 `paired_bench` functions, and on
     the M33 `Removal::refs`'s instruction order). So thumbv8m.main with
     its FPU takes the measured form by `--cfg cat_ft8_minmaxnm`; the
     embedded check builds with it, and the crate declares the cfg for
     `unexpected_cfgs` (the `[lints]` table; MSRV 1.75 builds it without
     warnings). **A thumbv8m.main build without the cfg takes the branch
     form in the subtraction: the host image is then not the code O18
     measured on the M33** (its `.text` 48 464 → 48 544 B, in
     `Fit4::add_all`, `Removal::refs` and one closure of `run`), and not
     measured; the small check image, which has no subtraction, is
     identical (corrected in O18c; the staged measurement images, which
     both link the subtraction, differ: 48 824 → 48 900 and 50 052 →
     50 128 B).
2. **a6feb13: one clamp, `input`, chosen per target.** The branch form
   is `input_branch`; `input` is `input_select` or `input_branch` by
   `INPUT_SELECT`. Both stay in the build because the choice is per
   target. The fit's single edge samples keep `input_branch` by name:
   with `input` there the code changed on every measured target (x86-64
   `cat-ft8-decode` −64 B, the M33 host image −20 B), and that was not
   measured. The tests take `input_branch` as the reference.
3. **75729ab: the reference's unit weights explained** (comment only).
   A slice of `()` as long as a block, in place of `phase` as the
   weights' slice, changed the code on every target (x86-64
   `cat-ft8-decode` +1 088 B, the M33 host image +20 B: the loop's count
   took a third bound), and one release library test failed with it;
   the form was dropped without finding which bound.
4. **6f389c4: `blocks` asserts the generator starts at sample 0**
   (`debug_assert_eq!(synth.position(), 0)`): its ramp test reads the
   block's position from its own count. Release code unchanged.
5. **85ba626: the code-generation check covers the clamp.** New checks
   in `cat-ft8/codegen-check.sh` on the release library's AArch64
   assembly (one codegen unit), counts with rustc 1.93.0:

| check | head | branch form everywhere (forced) | branch form in the removal only (forced) |
|---|---|---|---|
| four-lane `fmaxnm`/`fminnm` in `Removal::refs` (`7Removal4refs`), floor 2 | 6 | **0** | **0** |
| the same in `symbol_correlations`, floor 2 | 8 | **0** | 8 |
| `fcmgt`/`bsl`/`bit`/`bif` in `Removal::refs`, ceiling 2 | 0 | **18** | **18** |
| (the existing checks: 696 / 15 / 156) | pass | pass | pass |

   Each forced run is logged; both fail, as they must. The header now
   names O18.

### What is not guarded, and why

The reference without an envelope relies on the compiler folding the
multiplication by the constant weight 1.0 (an exact fold, `x · 1.0 =
x`, done by LLVM's instruction combining). It is left unchecked: losing
it costs only the two multiplications per sample that 7d747e3 still
had (the reference 4.79 → about 5.3 cycles a sample, about 0.5 of the
subtraction's 22.1), not a cliff, and its signature in the assembly (two
`fmul` fewer per four samples among the polynomial's) is not separable
by a count the way the clamp's is.

### Evidence corrected

- **x86-64 records (O18).** The O18 log's nine record comparisons named
  an empty reference directory and all failed; O18's result (0 differ)
  came from a comparison repeated by hand. The identity script now makes
  the comparisons itself against b93516e's records and WAV output, and was
  rerun at 26f66ad and at 85ba626: the 73 800 records equal b93516e's
  (0 differ, each of `host`, `host,passes=1` and `small`), and the 800
  WAV slots × 3 × 3 are byte-identical (7 200 runs, 78 690 decode lines),
  at both. The old log is replaced.
- **The M33's +632 B (O18)** is a second scalar loop there, not a
  16-sample block; corrected in place.

### Checks

- Local CI passes at 85ba626, including the extended code-generation
  check and `check.sh --stack` (flash 89 534 / 101 810 / 34 566 B, static RAM
  and frames as at 513fd17).
- The thumbv7em library builds, and `clippy -D warnings` passes for it
  and for thumbv8m with and without the cfg.

### Follow-up, not done

- `Synth::envelope_is_one` (or the like): `blocks` and
  `Synth::fill_phase` each know where the ramps are; one place should
  say it. Deferred to the next change to that code.

## Closure follow-ups (O18c, 2026-11-02; no code generation change)

On top of O18b (e1b7e8b), one change per commit: 1d53ad8, 795bb7e,
0d7f951, c4fa1ae, eb0cbfc and 38aec91, then this record. The two that
touch a `.rs` file (0d7f951, 38aec91; documentation only) leave the
`.text` and `.rodata` of all eight binaries byte-identical to 513fd17's
(x86-64 and AArch64 `cat-ft8-decode` and `paired_bench`; thumbv8m check
and staged images built with `--cfg cat_ft8_minmaxnm`; one scratch path,
rustc 1.93.0). The others change scripts and the CI workflow only.

### Changes

- **1d53ad8: the code-generation check fails on a missing function.** A
  label not found printed 0, which a ceiling passed; it now prints
  "function not found" and fails, whatever the metric, and an unknown
  metric fails. Forced (logged): a misspelt segment and an unknown metric
  each fail.
- **795bb7e: the check covers the clamp on the microcontrollers.** It
  also emits the no-std library's assembly (one codegen unit, nothing
  linked) for two targets, counts with rustc 1.93.0:

| build | check | head | forced | result forced |
|---|---|---|---|---|
| thumbv8m.main, `--cfg cat_ft8_minmaxnm` | `vmaxnm`/`vminnm` in `Removal::refs`, floor 8 | 20 | without the cfg: 0 | fails |
| thumbv7em | `fmaxf`/`fminf` calls in `Removal::refs`, `symbol_correlations`, `Fit4::add_all`, ceiling 0 | 0, 0, 0 | with the cfg: 20, 6, 8 | all three fail |

  The aarch64 CI job, which runs the check, now installs both targets.
- **0d7f951: the crate docs' `no_std` section documents the cfg**: the
  target-scoped `.cargo/config.toml` entry
  (`[target.thumbv8m.main-none-eabihf] rustflags = ["--cfg",
  "cat_ft8_minmaxnm"]`), that without it thumbv8m.main takes the branch
  form (the same results; its speed on the Cortex-M33 not measured), and
  that a `RUSTFLAGS` environment variable replaces the configuration's
  rustflags.
- **c4fa1ae: the embedded check prints stack frames only with
  `--stack`** (a `stack` variable; since O18b's cfg `flags` was never
  empty, so every run printed empty headings).
- **eb0cbfc: the embedded check lints with the cfg it builds with**, in a
  target directory of its own (the build's RUSTFLAGS differ under
  `--stack`, and a shared directory would rebuild each time).
  `check.sh` and `check.sh --stack` pass; sizes as before (flash 89 534 /
  101 810 / 34 566 B).
- **38aec91: wording.** `INPUT_SELECT`'s doc and the Cargo.toml comment
  said `max` and `min` are single instructions where the branch-free
  clamp is chosen; on x86-64 they are `maxss` with a compare and blend.
  Both now say: not library calls, and measured faster.
- **Records corrected in place:** O18b names the identity script rather
  than its file; without the cfg only the M33 host image differs among
  the check images (above, in O18b).

### Follow-ups, not done

- **The fit's edge samples** take `input_branch` by name. Measure
  `input` there (it changed code on every measured target in O18b, not
  timed), then unify or keep it with the measurement.
- **No host test runs the branch form's loops end to end**
  (`INPUT_SELECT` false): x86-64 and AArch64 both take the branch-free
  form, and the equivalence rests on the exhaustive scalar test of the
  two forms. An i686 test run would cover them.
- From O18b: `Synth::envelope_is_one`.

### Checks

Local CI passes at 38aec91, including the extended code-generation check
(AArch64 696 / 15 / 156 / 6 / 8 / 0; thumbv8m 20; thumbv7em 0 / 0 / 0)
and `check.sh --stack`.

## The subtraction, nineteenth round (O19, bit-identical, 2026-11-02)

Base: 3ec5266 (the code of O18c). The target is still the signal
subtraction, 48.1 ms of the A76's 94.0 ms crowded three-pass host slot.
Two code commits are kept, one optimisation each: 0286ee8 (the
generator's phase in closed form where the pulse is alone) and 7e388d3
(the subtraction's reference in groups of four lanes). Decodes,
records, residuals and encoder output are identical, with no
qualification. The subtraction falls 48.10 → 43.77 ms (−9.0%) and the
slot 94.00 → 89.70 ms (the base and 7e388d3 were timed in two
sequences, linked by 0286ee8, which read the same in both).

### Measured first

O18's microbenchmark (same workload, A76 core 3, cycles per waveform
sample, medians of six: two runs of three rounds, alternated, all valid
at 0x0) at the base and at each step:

| part | 3ec5266 | 0286ee8 | 0286ee8 (2nd pair) | grouped, first form | grouped, final form |
|---|---|---|---|---|---|
| **the subtraction** | **22.04** | 21.68 | 21.69 | 21.21 | **20.03** |
| the subtraction, reported (`subtract`) | | | 29.20 | 29.50 | 28.34 |
| time search: generator (per core sample) | 4.42 | 4.08 | 4.08 | | 4.12 |
| time search: correlations (per core sample) | 11.06 | 10.76 | 10.77 | 11.03 | 10.58 |
| fit path: generator | 2.59 | 2.40 | 2.40 | | 2.42 |
| + correction + reference | 9.20 | 9.00 | 9.00 | 9.79 | 9.03 |
| + fit (fused) | 12.62 | 12.47 | 12.45 | | 11.40 |
| the fit loop alone | 3.19 | 3.19 | 3.19 | 2.47 | 2.47 |

(The "grouped" columns are work-tree forms measured on the way to
7e388d3; the final form's figures are of 7e388d3 itself.)

What the counts and the code showed:

- **The running sum still runs where every increment is the same.** O18
  stored one increment, `step(cur)`, at each of the 509 samples per
  symbol where the current pulse is alone, and the running sum then
  added them one by one. The phase there is the run's first phase plus
  k increments: integer arithmetic modulo 2³², so a product is exact
  and the same as the sum. (A closed form need not reproduce an `f32`
  running sum: the phase is a `u32`.)
- **The fit loaded the sines twice.** `Fit4::add_all`'s four-lane loop
  was 27 AArch64 instructions per four samples: an `ld2` of the four
  pairs, and then the four sines again by `ldur` and three `ld1` lane
  inserts, only for their squares. Reordering the sums, reading the
  pairs as eight floats, `|s|·|s|` and a copy of the sines all left the
  same 27; removing one sum changed the loop altogether (259
  instructions). The pairs' interleaved layout was the cause: the
  removal also loaded them with `ld2` and the reference and products
  were stored with `st2`.

### Changes

1. **0286ee8: the generator's phase in closed form where the pulse is
   alone.**
   - `Synth::increments` becomes `Synth::phases<SUM_ONLY>`: each symbol
     in its three runs. In the two where pulses overlap the increments
     are stored and then summed (the running sum, or with `SUM_ONLY`
     for `skip_with` their total); in the run where the pulse is alone
     the phase is written as the run's first phase plus k increments (a
     loop the compiler vectorises: 8 samples in 6 AArch64 instructions)
     and the run's total is one product.
   - Integer addition modulo 2³² is associative, so the phases are the
     same. Mutants caught (one log each): the run's total one increment
     short (27 tests fail), its first phase one increment on (7), and
     `skip_with` without the run (7).
   - On the M33, `increments` was one function shared by `skip` and
     `fill_phase`; the two instances of `phases` are each inlined: the
     small check image's `.text` +736 B (+740 B of flash as `check.sh`
     counts it, below).
   - **Not kept: `SUM_ONLY` as a runtime argument.** One shared copy on
     the M33 (`.text` +196 B instead of +736 B), but other code on the
     hosts:
     A76 subtraction 47.315 ms against 47.260 (the second sequence),
     x86-64 335.10 G cycles against 334.48 G (within the base's range),
     M33 32.386 M instructions a call against 32.344 M.
2. **7e388d3: the subtraction's reference in groups of four lanes.**
   - The held reference and the time search's products are stored by
     slot in groups of `GROUP` samples: the group's cosines, then its
     sines. `GROUP` is `LANES` (four) where the target has a vector
     unit and one elsewhere, where the layout is the pair as before.
     Every caller keeps a sample's slot equal to its index modulo the
     lanes (the ring and the blocks are whole groups), so whole lanes
     are whole groups and each part loads and stores as one vector.
   - AArch64 (the aligned `paired_bench`): the fit's loop is 20
     instructions per four samples (27), one `ldp` for the group; the
     removal's 21 (22), `ldp` for `ld2`; no `ld2`, `st2`, `ld4` or
     `st4` is left in the subtraction.
   - Three code-generation hazards were met and are worked around in
     the source:
     - **The removal.** Written over whole groups, its loop was taken
       four groups at a time by the loop vectoriser, with `ld4`/`st4`
       and shuffles (117 instructions per 16 samples, against 24 per
       four for the SLP form it left as the epilogue). Each lane's ramp
       index is now carried as an `f32` (an integer below 2²⁴, so it
       and each step of four are exact, and the clamp at 0 gives
       `ramp_index`'s value): a floating-point recurrence that the loop
       vectoriser does not take, and the ramp becomes one `fadd` and
       one `fmaxnm` per four samples.
     - **The reference's block of 16.** Stored sample by sample through
       `put`, the polynomial was rebuilt with `zip`/`uzp` shuffles (159
       instructions per 16 samples; 136 before); computed into a cosine
       array and a sine array and stored a group's part at a time, it
       is 127.
     - **The time search's lane sums.** Inside `lane_sums` the compiler
       kept the eight sums as scalars (15 instructions per four
       samples, against 9 with the pairs as two-lane vectors); in a
       function of their own (`whole_lane_sums`, not inlined, one call
       per block) the loop is five instructions: `ldp`, two `fadd`.
   - **The M33.** The first grouped form ran the writer's and the
     removal's per-sample steps through closures that the M33 build
     (`opt-level = "s"`) did not inline: 41.410 M instructions a call
     (+28%). Where a group is one sample the writer and the removal now
     keep their sample-by-sample loops, and the removal's step is an
     `#[inline(always)]` function (`remove_one`); the M33 count is then
     32.198 M (−0.45% on 0286ee8). The hosts' code is unchanged by
     that (the branches fold away), except the removal's, which
     changed shape with `remove_one` (its main loop 21 instructions as
     before).
   - Tests: the writer against the plain loop from every first slot
     modulo a group (`GROUP + 2` slots) and every length up to three
     blocks of `W`; the reference without an envelope from every slot;
     the removal against its first form with the reference at slots
     not equal to its sample index modulo a group; the fit with its
     reference from slot `i0` and a lane on.
   - Mutants caught (one log each): the sine part stored beside the
     cosine (8 tests fail), the writer's single samples before the
     first whole group miscounted (3), the removal's carried index one
     on (4), the lane sums' sines subtracted (11), and the removal's
     slot not advanced between runs (13).
   - **The code-generation check** (`codegen-check.sh`, rustc 1.93.0)
     now also fails on interleaving loads or stores (`ld2`–`ld4`,
     `st2`–`st4`) in `Removal::refs` and `Fit4::add_all` (ceiling 0),
     on shuffles in `blocks` (ceiling 12), and on `whole_lane_sums`
     having fewer than two four-lane instructions or not being found.
     Each was shown to fire on a forced form (one log each):

| forced form | `Removal::refs` interleaved | `Fit4::add_all` interleaved | `blocks` shuffles | `whole_lane_sums` |
|---|---|---|---|---|
| 7e388d3 | 0 | 0 | 3 | 2 |
| a group of one on every target | **3** | **1** | **19** | 2 |
| the removal's index as an integer | **2** | 0 | 3 | 2 |
| each sample of the block stored by itself | 0 | 0 | **75** | 2 |
| `whole_lane_sums` inlined | 0 | 0 | 3 | **not found** |

     The removal's four-lane `fmaxnm`/`fminnm` count is 5 (it was 6);
     its floor is 2.

### A76 (Raspberry Pi 5; `slotcost host host,passes=1 200 5`, medians of 4 alternated runs)

Aligned builds (`-align-loops=64 -align-all-functions=6`) from one
scratch path, core 3, the throttle, clock and temperature sampled once a
second from core 0, a 5 s busy loop on core 3 (sampled) before each
run. Two sequences of three builds, each alternated (forward, then
reversed); 0286ee8 is in both. The second sequence's last build is the
work tree that became 7e388d3: its eight binaries' `.text` and
`.rodata` are byte-identical to 7e388d3's (the commit added only a
comment). "Grouped, first form" is the grouped layout before the M33
changes; "runtime argument" is the form of 0286ee8 not kept.

| build | slot (ms) | subtract (spread) | pass 2 / pass 3 subtract | feed | BP | sync | one pass |
|---|---|---|---|---|---|---|---|
| 3ec5266 (base) | 94.00 | 48.100 (0.020) | 33.505 / 14.590 | 19.290 | 19.050 | 4.680 | 18.40 |
| 0286ee8 | 93.00 | 47.255 (0.020), −0.845 | 32.920 / 14.340 | 19.075 | 19.050 | 4.680 | 18.30 |
| grouped, first form | 89.65 | 43.730 (0.080) | 30.465 / 13.265 | 19.295 | 19.050 | 4.680 | 18.40 |
| *second sequence* | | | | | | | |
| 0286ee8 | 93.00 | 47.260 (0.030) | 32.920 / 14.335 | 19.080 | 19.050 | 4.680 | 18.30 |
| runtime argument (not kept) | 93.20 | 47.315 (0.030) | 32.960 / 14.355 | 19.285 | 19.050 | 4.680 | 18.40 |
| **7e388d3** | **89.70** | **43.765 (0.110), −3.495; −4.335 (−9.0%) on the base** | 30.485 / 13.275 | 19.295 | 19.050 | 4.670 | 18.40 |

- **Placement check.** Compared function by function (AArch64
  `paired_bench`, addresses and branch offsets normalised): between
  3ec5266 and 0286ee8 only the generator and its callers differ
  (`fill_phase`, `fill_phase_only`, `skip_with`, `increments` gone,
  `synthesize`, `blocks`, `symbol_correlations`), plus `soft_bits_at`
  in its addressing only (`adr` for `adrp`); between 0286ee8 and
  7e388d3 the subtraction's functions, plus `soft_bits_at` (628 → 611
  instructions; its stage, soft bits, is 2.380 ms in both),
  `reliability_order` and three `paired_bench` functions in addressing
  only.
- The feed's code is the same in all four builds, yet it reads 19.075–
  19.080 ms in 0286ee8's binary and 19.285–19.295 ms in the others
  (0.215 ms): placement. It is the largest untouched move. 0286ee8's
  gain (0.845 ms, reproduced at 47.255 and 47.260 in the two
  sequences) is 3.9 times it and 42 times the stage's spread; 7e388d3's
  (3.495 ms) 16 times it. Sync, soft bits and belief propagation move
  by at most 0.010 ms.
- By pass: 18.31 / 46.86 / 28.84 → 18.31 / 43.84 / 27.52 ms; per new
  decode 0.76 / 4.46 / 9.61 → 0.76 / 4.17 / 9.17 ms. The one-pass slot
  runs no subtraction (18.40 → 18.40; 18.30 in 0286ee8's binary, with
  its feed). Worst slot 100.7 → 95.8 ms.
- The base agrees with O18's 513fd17 (94.00, subtraction 48.115).
- **Validity.** All 24 timed runs are valid: every sample during the
  runs and in the warm-ups 0x0, start and end checks included. No
  sample during a run was below 2.4 GHz; one warm-up sample in each
  sequence was (1.5 and 1.7 GHz, the first sample of its first warm-up).
  3 123 samples during the runs and 120 in the warm-ups. Peak 54.9 °C. The 20
  microbenchmark runs (12 while choosing, 8 of the final pair) were
  valid at 0x0 as well, start and end included.

### x86-64 (Threadripper 3970X, core 5, `perf stat` user counters over the whole `slotcost` program)

Four alternated runs per build (forward, then reversed), aligned builds
from one scratch path, on an otherwise idle machine. Cycles are the
measure (the virtual machine's clock biases its stage times).

| build | cycles (median, range) | instructions |
|---|---|---|
| 3ec5266 | 335.71 G (334.81–336.58) | 949.67 G |
| 0286ee8 | 334.05 G (333.15–334.68), −0.49% | 952.18 G (+0.26%) |
| 7e388d3 | **327.84 G (327.00–329.29), −1.86%; −2.34% in all** | 925.28 G (−2.57%) |

- 0286ee8's range clears the base's by 0.13 G only; an earlier
  sequence (four builds, during part of which other builds ran on the
  machine) gave 335.92 → 334.48 G, ranges touching at 334.99. Its gain
  on x86-64 is small and only just separated. It adds instructions
  (the closed-form loop is vectorised in place of the plain running
  sum, which runs scalar on x86-64).
- The runtime-argument form of 0286ee8 (not kept) read 335.10 G
  (334.47–335.24) in that earlier sequence, within the base's range.
- 7e388d3 is separated from both (it read 328.05 G in the earlier
  sequence).

### Cortex-M33 (emulated instructions; staged measurement images, s0086)

| | host slot | subtraction (37 calls) | per call | generator | the subtraction's closure | stack high water (host / small) |
|---|---|---|---|---|---|---|
| 3ec5266 | 2 280.45 M | 1 199.85 M | 32.428 M | 148.25 M | 202.23 M | 10 316 / 10 192 B |
| 0286ee8 | 2 277.24 M | 1 196.74 M | 32.344 M (−0.26%) | 182.87 M | 193.58 M | 10 316 / 10 192 B |
| **7e388d3** | **2 271.81 M (−0.38%)** | 1 191.31 M | **32.198 M (−0.45%; −0.71% in all)** | 182.87 M | 144.46 M + 43.60 M (`whole_lane_sums`) | 10 308 / 10 192 B |
| (grouped, first form) | 2 612.68 M | 1 532.18 M | 41.410 M (+28%) | | | 10 308 / 10 184 B |
| (runtime argument, not kept) | 2 278.79 M | 1 198.28 M | 32.386 M | 184.03 M | 193.72 M | 10 304 / 10 200 B |

- Same decodes (40 host, 23 small). The generator column is
  `increments` + `skip_with` at the base and `fill_phase_only` +
  `skip_with` after (the shared `increments` was inlined into its two
  callers); at 0286ee8 `blocks` falls 391.38 → 363.09 M with it.
- The small slot (no subtraction) falls 295 352 059 → 295 247 228
  instructions, all in its setup (the encoder checks, −104 829).
- No M33 count rises at either commit.

### Sizes and stack (`check.sh --stack`, thumbv8m)

| image | 3ec5266 | 0286ee8 | 7e388d3 |
|---|---|---|---|
| small (and small, OSD on) | 89 534 B | 90 274 B (+740) | 90 274 B |
| host (and host, OSD off) | 101 810 B | 102 402 B (+592) | 102 570 B (+168) |
| encoder only | 34 566 B | 35 286 B (+720) | 35 286 B |

- Static RAM is unchanged (212 048 / 241 616 / 0 B).
- Frames: `Synth::skip` 296 → 352 B and `Synth::fill_phase` 80 → 120 B
  at 0286ee8 (the generator inlined into each); host `subtract_one`
  4 816 → 4 808 B; the subtraction's closure 2 872 → 2 880 → 2 864 B.
  The others are unchanged (`_start` 4 104, `decode_a_slot` 2 512 B).
  The measured high water does not rise (table above).
- The flash growth is the generator inlined twice at 0286ee8 (the
  runtime-argument form that kept one copy was slower on the hosts,
  above) and, at 7e388d3, `whole_lane_sums` (+126 B) and
  `subtract_one` (+272 B) less the subtraction's closure (−166 B).
- No fused multiply-add (0 `vfma`/`vfms`/`vfnma`/`vfnms`; `vmla` 74 /
  157 / 12 → 79 / 162 / 17) and no `f64` helper in any image; thumbv7em
  builds.
- The encoder-only image grows 720 B (34 566 → 35 286 B) at 0286ee8
  with no benefit to it: the encoder's waveform runs `fill_phase` and
  `skip`, and its count is not what the change was for. (Added in
  O19b.)

### Identity

- **Records, x86-64:** the 73 800 `paired_bench run` records of 0286ee8
  and 7e388d3 equal b93516e's (0 differ, each of `host`, `host,passes=1`
  and `small`), compared by the identity script and logged.
- **800 WAV slots** × `host`, `small`, `host,passes=1` × plain, `--raw`,
  `--profile` (7 200 runs, 78 690 decode lines): byte-identical to
  b93516e's at both commits.
- **Encoder:** the 45 `cat-ft8-encode` WAV files of both commits are
  identical to 3ec5266's (and those to 513fd17's).
- **Cortex-M33 (emulated):** the small and host dump images' decodes and
  statics on seven slots (14 of 14), and the multi-pass residuals on
  s0086 and s0407, are byte-identical to 68634cc's at both commits
  (images built with `--cfg cat_ft8_minmaxnm`).
- **A76:** the 73 800 records of 0286ee8's and 7e388d3's AArch64
  `paired_bench` equal x86-64 b93516e's (0 differ, each configuration);
  the 800 WAV slots × 3 × 3 with 7e388d3's AArch64 `cat-ft8-decode` are
  byte-identical to x86-64 b93516e's (7 200 runs, 78 690 decode lines).
  Not timed; 0x0 before and after each part.
- **A group of one on x86-64** (`GROUP` forced to 1, not a shipped
  configuration; the M33's layout): the release library tests with the
  ignored ones passed except one, `fit_sums_follow_the_sample_index_bit_
  for_bit`, at one case (`i0` 1, 239 samples, slot 1). (Corrected in
  O19b.) The cause is a miscompile by LLVM's loop vectoriser of the
  test's own expected-value loop, not of the code under test:
  `add_all` returned 0.6571188 in lane 0, the value of a plain trace;
  the test's expected sums were wrong (`xc[0]` 0.0, and negative sums of
  squares in lane 0). The 473.9 first recorded here is the test's
  `ss[0]`, not `add_all`'s result. The test is fixed and the forced run
  passes; see O19b.

### Checks

- Local CI passes at 7e388d3: fmt; clippy `-D warnings` (workspace, all
  targets and features, thumbv8m); workspace, profile, no-std and
  release tests with the ignored ones; the bulk run; MSRV 1.75; docs with
  and without `std`; `check.sh --stack`; the code-generation check
  (AArch64 696 / 15 / 156 / 5 / 8 / 0 / 0 / 0 / 2 / 3; thumbv8m 20;
  thumbv7em 0 / 0 / 0).
- Raspberry Pi 5: the release library tests with the ignored ones pass
  at both commits (199 each; the one test that reads the source tree
  skipped), 0x0 before and after. Built from 7e388d3 (`cat-ft8` tree
  1593ebf8…, rustc 1.93.0), sha256 a9eec928…, and from 0286ee8 (tree
  e701cd7a…), sha256 ad023014…, the same hashes on the Raspberry Pi 5.

### After the round: what bounds the subtraction now

The microbenchmark at 7e388d3 (A76, cycles per waveform sample, medians
of six; the time search's figures per core sample):

| part | 0286ee8 | 7e388d3 | instructions | vector ops | ÷ floor |
|---|---|---|---|---|---|
| **the subtraction** | **21.69** | **20.03 (−7.6%)** | 48.13 → 45.19 | 25.22 → 23.66 | 1.72 → 1.69 |
| time search: generator | 4.08 | 4.12 | 10.22 | 3.67 | 1.61 |
| time search: + products and lane sums | 10.78 | 10.58 | 23.66 → 22.45 | 14.31 → 13.20 | 1.51 → 1.60 |
| fit path: generator | 2.40 | 2.42 | 6.34 | 1.70 | 1.53 |
| + correction + reference | 9.00 | 9.03 | 21.12 → 21.29 | 9.36 → 9.37 | 1.70 |
| + fit (fused) | 12.46 | 11.40 | 29.14 → 26.27 | 13.44 → 12.44 | 1.71 → 1.74 |
| the fit loop alone | 3.19 | 2.47 | 7.30 → 5.53 | 3.76 → 3.76 | 1.70 → 1.31 |

(The floor is the larger of instructions ÷ 4 and vector operations ÷ 2.
The microbenchmark's own removal row is not shown: called from the test,
`Removal::refs` is inlined there and compiled without vector operations
(8.69 cycles a sample, 0.01 vector operations). The decoder's binaries
keep one `Removal::refs` with the vectorised loop, called from `run`,
`blocks` and `held`; a scalar copy would fail the code-generation
check's `fmaxnm`/`fminnm` floor.)

At 7e388d3 the subtraction is 45.2 instructions and 23.7 vector
operations a sample, a floor of 11.8 cycles against the measured 20.0
(1.69×). Its parts, per waveform sample (one time-search step):

- the polynomial passes, the reference (4.82) and the time search's
  products (2.89), 38%;
- the generator, 4.48 (22%): 2.42 in the fit path and 2.06 in the time
  search, where half of it skips the non-core samples (about 0.9);
- the removal and the rest, about 2.7 (13%, by difference: 3.2 at
  0286ee8);
- the fit, 2.37 (12%), its loop at 1.31× its floor;
- the correction, 1.79 (9%); the step sums and alignment, 0.68 (3%).

Memory traffic and branches are still not the bound (L1D refills 0.07 a
sample, branch misses 0.003). The polynomial passes and the generator
are each at 1.5–1.7× their floors, with dependent chains per sample.

### Not measured

- Cortex-M33 cycles (emulated instructions only), and the M33 on any
  slot but s0086 for counts (seven slots for identity).
- The microbenchmark ran on A76 core 3 only; on x86-64 only the whole
  program's counters were read.
- How the gain varies with slot content: the timed slots are the
  crowded 40-message `main` block.
- Candidate (c) of O18, keeping each symbol core's starting phase when
  the time search takes a second step, was not tried: the
  microbenchmark's workload takes one step per subtraction, and its
  share (about 0.2 steps a subtraction, each step's skipped half of the
  generator about 0.9 cycles a sample) is near 1% at most. A cache of
  the skipped spans' increment sums by tone pair (64 entries) would
  serve the first step too; not tried.
- The reported subtraction (`subtract`, used outside the decoder's
  passes) was measured only in the microbenchmark.
- **After a toolchain change**: the gains depend on the code generation
  the extended check guards (above), and on the closed-form loop being
  vectorised (not checked: losing it costs a serial add per sample over
  a quarter of the samples, not a cliff).

## Review follow-ups (O19b, 2026-11-02; no code generation change on the measured targets)

On top of O19 (956caaa), one change per commit: 580938c, ffba08e,
954e5b0, 78aab1f, 4eb4fd0, 00ef632, ff3b924 and 35aa6da, then this
record. **The `.text` and `.rodata` of all eight binaries are
byte-identical to 7e388d3's at every commit that touches a `.rs` file**
(x86-64 and AArch64 `cat-ft8-decode` and `paired_bench`; thumbv8m check
and staged images built with `--cfg cat_ft8_minmaxnm`; one scratch path,
fresh tree, rustc 1.93.0). So O19's timings, counts, sizes and identity
carry over unchanged.

### The forced group of one: a loop-vectoriser miscompile in the test

O19 recorded one failing case with `GROUP` forced to 1 on x86-64 and no
cause. The cause is LLVM's loop vectoriser, in the test's own
expected-value loop; the code under test was right.

- **The Rust shape.** A store into a four-element array at an index
  that wraps, in a loop over samples:

  ```rust
  for (k, &x) in xs.iter().enumerate() {
      let l = (i0 + k) % LANES; // LANES = 4; i0 not known modulo 4
      want.xc[l] += w(x) * c(k);
  }
  ```

  The loop vectoriser widened it into four-lane accesses starting at
  `xc[i0 % 4]` without a check that the lane index wraps: lane 0 was
  never updated and the next field of the struct was written (past the
  struct's end for the last field). Safe Rust, no `unsafe`, no
  dependencies. In the failing case `want.xc[0]` was 0.0 and lane 0's
  sums of squares negative; `add_all` returned the plain trace's
  0.6571188. The 473.9 of O19's record is the test's `ss[0]`.
- **Where it was found.** rustc 1.93.0 (LLVM 21.1.8), x86-64, release
  with the default 16 codegen units; the first bad pass by
  `-opt-bisect-limit` is `LoopVectorizePass` on the test function. The
  reduced IR (a top-tested loop with an unconditional latch,
  `for (i = 0; 8·(i + 1) <= len; i++) acc[(i + a) & 3] = src[i];`) is
  miscompiled by `opt` of LLVM 20 and of LLVM 21 alike; the rotated
  form of the same loop gets the right check. With one codegen unit, in
  debug, with `-vectorize-loops=false` or with a print in the loop the
  loop takes another shape and the failure goes away. Whether a later
  LLVM fixes it was not checked, nor was it reported upstream here.
- **The Cortex-M33.** thumbv8m.main without the M-profile vector
  extension has no vector registers for the loop vectoriser; on the
  test function's IR retargeted to thumbv8m (with and without
  `optsize`) the pass makes no change. That was checked on IR from the
  x86-64 pipeline, not the M33's own. No production code has the shape
  (below), and the M33's emulated identity covers its shipped code.
- **Fixes.** 580938c: the two tests that summed by `(i0 + k) % LANES`
  or `i % LANES` take each lane in its own loop, in sample order (the
  same sums). ffba08e: a test helper that read `nan[i % 4]` cycles the
  bytes instead. With `GROUP` forced to 1 the release library tests on
  x86-64 now pass, the ignored ones included (199; 1 filtered out, the
  source-tree test; checked).
- **The guard (954e5b0).** `cat-ft8/lane-index-check.sh`, run by CI and
  the local CI, fails on an array indexed by an expression taken modulo
  or masked to a lane count (`% LANES`, `% GROUP`, a power of two up to
  64, `& 3` and the like) anywhere in `cat-ft8/src`, tests included.
  Shown to fail on 956caaa's sources (the three test lines) and on a
  forced `a[(i + k) & 3] += …` (checked). (Removed in O19c.) It is textual: an index bound
  to a variable first, the fit test's form, is not seen (the script
  says so); a table read at a non-affine modulus (`% 1920`, `% 38`) is
  not flagged. The production sums by lane use selects (`add_lane`) or
  whole lanes, and none of the crate's sources has the written form.

### The layout of targets without a vector unit, on a host (78aab1f)

`--cfg cat_ft8_test_group_one` (declared for check-cfg; read only in
test builds, `cfg!(all(test, …))`) gives `GROUP` 1 on every target. CI
runs the release library tests so on x86-64, beside the bulk run; a
test checks that the cfg takes effect. A mutant in the groups-of-one
writer loop only (cosine and sine swapped) passes the default tests
(193) and fails 5 with the cfg (checked).

**The groups-of-one branches stay.** With them deleted (the writer's
single samples in one loop over the inlined `put`, the removal's
general path for groups of one, `remove_one` kept inline), the M33
staged host image takes 38.628 M instructions a subtraction against
32.198 M (+20%): `Removal::refs` 145.63 → 263.11 M, the subtraction's
closure 144.46 → 186.32 M, `blocks` 363.09 → 144.52 M, and a closure of
`scaled_cos_sin` (the writer's single samples) 0 → 296.09 M, out of
line (s0086, 37 calls; decodes the same; corrected in O19c, which added
the last). Part of the +20% may be that closure not being inlined at
`opt-level = "s"` rather than the general path itself; not separated. Its flash grows 44 B, and the hosts' code
changes too (x86-64 `cat-ft8-decode` −1 728 B, AArch64 −1 280 B), not
timed.

### Other changes

- **4eb4fd0: the ring's rounding is required.** The comment said the
  ring was a multiple of `LANES` for speed only; since O19 the fit needs
  sample j's slot `j % ring` equal to j modulo `LANES`. The comment says
  so and `assert_eq!(ring % LANES, 0)` states it (the compiler proves
  it: `.text` unchanged).
- **00ef632: docs.** `Synth::phases`: with `SUM_ONLY` the output is used
  as working storage, and the flat run carries its phase by one add per
  sample. `whole_lane_sums` names AArch64 as the target its placement was
  measured on (on the M33 it is +126 B; not measured apart). The
  instruction count left `Fit4::add_all`'s doc: with rustc 1.93.0 its
  four-lane loop is 20 AArch64 instructions per four samples (O19).
- **ff3b924:** the writer's whole groups are a compile-time assertion
  for the decoder's block (`CHAINS % GROUP == 0`); the tests' other
  block sizes keep a debug assertion.
- **35aa6da: the code-generation check's ceilings have floors beside
  them.** A ceiling counts only its function's code, so it would pass
  if the guarded loop moved elsewhere. Each function with a ceiling now
  has a floor on its vector instructions (`Removal::refs` had its
  `fmaxnm`/`fminnm` floor):

| check | 7e388d3 | forced | result forced |
|---|---|---|---|
| `.4s` in `Fit4::add_all`, floor 12 | 18 | `add4` out of line: 8 | fails |
| `.4s` in `blocks`, floor 400 | 549 | `scaled_cos_sin` out of line: 0 | fails |

  The header's lines are filled evenly.
- **The guard's build is not the benchmarked one.** The
  code-generation check emits the library's assembly with one codegen
  unit; the hosts' release builds use the default 16 (and the
  miscompile above appeared only with 16). The measured AArch64
  binaries' loops were read directly in O19 (`paired_bench` and
  `cat-ft8-decode` each keep one `Removal::refs` with the 21-instruction
  loop); after a toolchain change both should be read again.

### Records corrected

- O19's "A group of one on x86-64" (above), its note on a closed form
  (now: a closed form need not reproduce an `f32` running sum, the phase
  is a `u32`), and the encoder-only image's +720 B, which buys it
  nothing.
- The A76 identity log now names each build's source commit (0286ee8,
  7e388d3) from its build record, beside the binaries' hashes; the
  script records it from now on.
- The mutant and forced-check logs named the work tree's HEAD, not the
  tree that ran: each now carries a correction (HEAD plus the change
  committed next, plus the mutant or forced edit), and the scripts record
  the work-tree diff's hash from now on.

### Follow-ups, not done

- **A `Slots` view** (`(bytes, first)` in place of the `(refs, k0)`
  pairs through `add_all`, `Removal::run`, `run_as` and `refs`): tried
  on the read side only, it changed `.text` on every measured target
  (x86-64 `cat-ft8-decode` +128 B, AArch64 −128 B, the M33 host check
  image +44 B), so it was not kept untimed. **It is a prerequisite for
  the next caller that threads `(refs, k0)`**: the view is where the
  congruence the fit needs, `k0 ≡ i0 (mod LANES)` (today a debug
  assertion in `Fit4::add_all`), would be checked once.
- **One head/whole/tail split** for the four places that write it out
  (the writer, the removal, the lane sums, the fit): not tried, for the
  same reason; best done with the `Slots` view and timed.
- The flat-run loop of `Synth::phases` is not in the code-generation
  check, and there are no x86-64 checks.

### Checks

- Local CI passes at 35aa6da: as in O19, plus the lane-index check and
  the release library tests with `--cfg cat_ft8_test_group_one`; the
  code-generation check AArch64 696 / 15 / 156 / 5 / 8 / 0 / 0 / 0 / 18
  / 2 / 3 / 549, thumbv8m 20, thumbv7em 0 / 0 / 0.
- clippy `-D warnings` passes with the test cfg as well.
- No measured code changed, so the A76, x86-64 and M33 timings, counts
  and identity of O19 stand; nothing was rerun on the A76.

## Closure follow-ups (O19c, 2026-11-02; no code generation change on the measured targets)

On top of O19b (5f8cba0), one change per commit: 8c81172, 19cc572,
b8a3566, 2f1d160 and 742a710, then this record. The commits that touch
a `.rs` file (19cc572, b8a3566) leave the `.text`
and `.rodata` of all eight binaries byte-identical to 7e388d3's (x86-64
and AArch64 `cat-ft8-decode` and `paired_bench`; thumbv8m check and
staged images built with `--cfg cat_ft8_minmaxnm`, the flags now named
in each build record; one scratch path, fresh tree, rustc 1.93.0).

- **8c81172: the lane-index source check removed**, with its CI and
  local CI steps. It missed the form that was miscompiled (an index
  bound to a variable before the store, the fit test's form), so the
  claim that it caught it was false; it passed when `grep` itself
  failed; and the easiest way around one of its false positives was to
  write exactly that form. The protection is the release library tests,
  whose reference sums go lane by lane, run with groups of four and with
  `--cfg cat_ft8_test_group_one`. No production code in `cat-ft8/src`
  has the miscompiled shape, a sum into a small array at a lane index
  that wraps: the production sums by lane use selects (`add_lane`) or
  whole lanes (checked by reading every `% LANES`, `% GROUP` and masked
  index in the crate's sources at 5f8cba0). The one production use of a
  wrapping index is `slot`, the byte offset of a stored sample
  (`k % GROUP` within its group); it is used by the writer's and the
  removal's single samples before and after the whole groups, through
  bounds-checked slices, and those paths run in the release tests in
  both layouts. ffba08e (the NaN scratch) stays as it is.
- **19cc572: what the groups-of-one tests cover.** `GROUP`'s doc and
  the CI step say so: the reference layout only. `CHAINS` and the other
  `VECTOR` choices (belief propagation's) keep the host's, so it is not
  the Cortex-M33's build on a host.
- **b8a3566: the ring's assertion dropped.** `assert_eq!(ring % LANES,
  0)` restated the line above it. The congruence that matters is the
  fit's `k0 ≡ i0 (mod LANES)`, a debug assertion in `Fit4::add_all`,
  and the `Slots` view (O19b, follow-ups) is where it would be checked
  once; that view is a prerequisite for the next caller that threads
  `(refs, k0)`.
- **2f1d160: the code-generation check's floors are AArch64's.** The
  header said every function with a ceiling has a floor; the thumbv7em
  `fmaxf`/`fminf` ceilings have none. A floor of `vmul`/`vadd`/`vsub.f32`
  there was tried: it fell when `Removal::run_as` or `scaled_cos_sin`
  was forced out of line (39 → 19 in the removal, 39 → 24 in the time
  search), but `Fit4::add_all` holds only 5 such instructions on
  thumbv7em and none of the forced forms moved them, so the sentence
  was narrowed instead. 742a710 fills the header's long line.
- **Whole groups checked at compile time for every `W`: tried, not
  kept.** An associated constant `WholeGroups::<W>::CHECK`
  (`assert!(W == 1 || W % GROUP == 0)`), named in `scaled_cos_sin` and
  so evaluated when it is instantiated, builds on Rust 1.75 and fails
  the build for a bad `W` (shown: a test with `W` 6 does not compile on
  x86-64). It changed code on the hosts, with `in_group` made a
  `const fn` and without: x86-64 `cat-ft8-decode` −64 B and
  `paired_bench` +64 B of `.text`, AArch64 `paired_bench` other code at
  the same size (the M33 images were unchanged). Not timed, so not
  kept: the decoder's `W` (`CHAINS`) keeps its compile-time assertion
  and the tests' other block sizes their debug assertion.
- **Records corrected** in O19b: the groups-of-one deletion's breakdown
  now includes the writer's out-of-line closure (296.09 M); the forced
  run's 199 tests had 1 filtered out (the source-tree test); "(logged)"
  reads "(checked)".
- The code-generation check's `4Fit47add_all` label also matches
  `add_all`'s closure (its single edge samples): the counts for that
  label are the two functions' together (no effect on today's bounds).

### Checks

- Local CI passes at 742a710: as in O19b, less the removed source check
  (the release library tests with `--cfg cat_ft8_test_group_one`
  included; the code-generation check's counts as in O19b).
- No measured code changed, so O19's timings, counts and identity
  stand; nothing was rerun on the A76 or the M33.

## Profile after O16–O19 (prof19, 2026-11-02; measurement only)

At 7f02b12 (the code of 7e388d3; every later commit changes no measured
code). No decoder change; every tool is out of tree. Purpose: where the
CPU and memory per decoded signal go after O16–O19, on crowded, sparse
and empty slots, and what to cut next.

### Method

- **One build.** Aligned host builds (`-align-loops=64
  -align-all-functions=6`), thumbv8m check and staged images (`--cfg
  cat_ft8_minmaxnm`), from one fresh scratch tree of 7f02b12, rustc
  1.93.0. The scratch path differs from O19's, so the binaries' hashes
  are not comparable with O19's (path strings), but their `.text` and
  `.rodata` sizes are the same, the M33 counts equal 7e388d3's to the
  instruction (2 271.81 M, 32.198 M a subtraction) and the A76 slot
  reads O19's 89.70 ms in two of four runs (89.60 in the others).
- **A76** (Raspberry Pi 5): one benchmark at a time on core 3, the
  throttle, clock and temperature sampled once a second from core 0, a
  5 s sampled busy loop on core 3 before each run, a run started only on
  0x0. Four cycles of: `slotcost host host,passes=1 200 5`, `slotcost
  small small 200 5`, `paired_bench run host DIR edges,empty`,
  `paired_bench run small DIR edges,empty` (one core, so one thread).
  Stage figures are medians over the four runs (per slot for `run`, then
  the mean over slots). Then one `paired_bench run host DIR
  crowded,wide,fading` for the later passes by yield (one run; its
  decode counts are exact, its times single samples).
  - **Every run is valid:** 17 runs, 2 333 samples during the runs, all
    0x0, start and end checks included; the 85 warm-up samples all 0x0.
    No sample during a run below 2.40 GHz; the warm-up minimum was
    2.40 GHz in the 16 runs of the cycles and 1.5 GHz in the last run's
    first warm-up sample. Peak 54.3 °C.
  - Run-to-run spread: the slot means of each configuration agree within
    0.10 ms (`slotcost`) and 0.01 ms (`run`, per set).
  - No sampler this round: inside a stage, the A76 splits come from O19's
    microbenchmark (the subtraction) and prof14's samples (the feed),
    and are marked as such.
- **Sparse and empty slots.** The synthetic `edges` set is the sparse
  slot: one signal at −18 dB per slot (1 300 slots, near the decode
  threshold; starts, band edges, a cut and a late input). The `empty`
  set is the empty slot (2 300): its `noise` point (1 000 slots of
  noise only) is quoted beside the whole set (carriers, clipped,
  floored, impulsive and strong-carrier noise included). No
  synthetic set has two or three signals per slot; none was made.
- **Cortex-M33** (emulated instructions, unicorn; staged measurement
  images as in O13–O19, s0086, 40 messages): host (three passes) and
  small (one pass), per function and per stage. A second pair of images
  with the front end's parts `#[inline(never)]` (as in O16) splits the
  feed; their stage totals are within 0.3 M of the inlined images'. On
  that small image, a dynamic count by mnemonic of each function over
  1 M instructions.
- **x86-64** (Threadripper 3970X, core 5): `perf record -e cycles:u` of
  `slotcost host host 200 5` and `slotcost small small 200 5`, symbol
  shares; the bench's own slot synthesis (`Synth::mix`, `fill_phase`,
  the Gaussian noise, `sqrt`, copies, kernel) removed, 7.0% of host and
  28.7% of small samples.
- **Memory:** `check.sh --stack` (static RAM, sizes, frames), `llvm-nm`
  sizes of the statics, the CLI's `--profile` buffer accounting, and the
  out-of-tree counting-allocator and painted-stack probe of prof14
  rebuilt at 7f02b12 (x86-64; s0080, s0081, s0084, s0086, s0087; host,
  host OSD off, small).

### A76: crowded 40-message slots (`main`, `slotcost`, medians of four runs)

| stage (ms per slot) | host, three passes | share | pass 1 | pass 2 | pass 3 | host, one pass | share | small | share |
|---|---|---|---|---|---|---|---|---|---|
| feed | 19.29 | 21.5% | 6.43 | 6.43 | 6.43 | 6.51 | 35.4% | 6.35 | 49.6% |
| sync | 4.67 | 5.2% | 1.54 | 1.56 | 1.57 | 1.54 | 8.4% | 1.38 | 10.8% |
| refine | 0.09 | 0.1% | 0.04 | 0.02 | 0.02 | 0.04 | 0.2% | 0.03 | 0.2% |
| soft bits | 2.38 | 2.7% | 1.19 | 0.59 | 0.59 | 1.19 | 6.5% | 0.89 | 7.0% |
| belief propagation | 19.06 | 21.3% | 8.95 | 4.65 | 5.46 | 8.94 | 48.6% | 4.02 | 31.4% |
| OSD-1 | 0.20 | 0.2% | 0.00 | 0.03 | 0.16 | — | — | — | — |
| SNR | 0.24 | 0.3% | 0.15 | 0.07 | 0.02 | 0.15 | 0.8% | 0.15 | 1.2% |
| subtract | 43.74 | 48.8% | — | 30.47 | 13.27 | — | — | — | — |
| **slot** | **89.65** | | **18.31** | **43.81** | **27.52** | **18.40** | | **12.80** | |
| new decodes | 37.6 | | 24.1 | 10.5 | 3.0 | 24.1 | | 24.0 | |
| **ms per new decode** | **2.38** | | **0.76** | **4.17** | **9.17** | **0.76** | | **0.53** | |
| candidates through BP | 200 | | 100 | 50 | 50 | 100 | | 75 | |

- Worst slot 95.8 ms (three passes), 19.0 (one pass), 13.2 (small).
  Spreads over the four runs at most 0.06 ms a stage.
- Against prof14 (107.4 ms): the subtraction 56.70 → 43.74 ms (O18,
  O19), the feed 22.10 → 19.29 (O16), belief propagation 21.00 → 19.06
  (O17); small 15.35 → 12.80 ms. Per new decode 0.84 / 5.17 / 10.98 →
  0.76 / 4.17 / 9.17 ms a pass.
- **Inside the subtraction** (O19's microbenchmark at 7e388d3, the same
  code; not re-measured): 20.0 cycles a waveform sample at 1.69× its
  issue floor; the polynomial passes (reference and the time search's
  products) 38%, the generator 22%, the removal 13%, the fit 12%, the
  frequency correction 9%.
- **Inside the feed** (not re-measured on the A76): prof14 sampled the
  butterflies at about half the feed before O16 took the window into
  the first stage; about 3 ms a pass is the estimate used below.

### A76: sparse and empty slots (`paired_bench run`, one thread, medians of four runs)

| stage (ms per slot) | host, `edges` (1 signal) | share | host, `empty` | share | (`noise` only) | small, `edges` | share | small, `empty` | share |
|---|---|---|---|---|---|---|---|---|---|
| feed | 12.44 | 34.6% | 6.41 | 30.8% | 6.41 | 6.23 | 43.8% | 6.28 | 44.2% |
| sync | 3.09 | 8.6% | 1.59 | 7.6% | 1.60 | 1.43 | 10.1% | 1.43 | 10.1% |
| refine | 0.06 | 0.2% | 0.04 | 0.2% | 0.04 | 0.03 | 0.2% | 0.03 | 0.2% |
| soft bits | 1.75 | 4.9% | 1.18 | 5.7% | 1.18 | 0.89 | 6.3% | 0.89 | 6.3% |
| belief propagation | 16.97 | 47.2% | 11.38 | 54.6% | 11.54 | 5.63 | 39.6% | 5.59 | 39.3% |
| OSD-1 | 0.44 | 1.2% | 0.24 | 1.2% | 0.24 | — | — | — | — |
| SNR | 0.01 | 0.0% | 0.00 | — | 0.00 | 0.01 | 0.1% | 0.00 | — |
| subtract | 1.20 | 3.3% | — | — | — | — | — | — | — |
| **slot** | **35.96** | | **20.84** | | **21.01** | **14.21** | | **14.22** | |
| decoded per slot | 0.96 | | 0 | | 0 | 0.80 | | 0 | |
| ms per decode | 37.5 | | — | | — | 17.8 | | — | |

- **Host `edges` by pass:** pass 1 20.86 ms, 0.957 decodes; pass 2 ran
  in 1 244 of 1 300 slots (after its one subtraction) at 15.78 ms a run
  and **added no decode in any slot**; pass 3 never ran. Pass 2 is 42%
  of a one-signal slot. (prof14: 40.0 ms, pass 2 17.7 ms.)
- An empty host slot is pass 1 alone: 100 candidates that all fail
  after 30 rounds, 0.114 ms each (small: 75, 0.075 ms each).
- Small `edges` decodes 0.80 a slot: the `f100` and `f3090` points are
  outside its 200–2 900 Hz band.
- Against prof14: host `edges` 40.0 → 35.96 ms, `empty` 23.0 → 20.84,
  small `empty` 17.38 → 14.22.

### A76: later passes by the previous pass's yield (host, crowded + wide + fading `main`, one run)

2 000 slots, 72.33 ms and 21.28 decodes a slot (crowded 73.22, fading
71.12, wide 72.57 ms). The decode counts are those of prof14 (the
decoder's results are unchanged); the times are this round's.

| | previous pass's new decodes | slots | cost when run (ms) | new decodes when run | ms per new decode | ms per pooled slot | new decodes per pooled slot |
|---|---|---|---|---|---|---|---|
| pass 3 | 1 | 326 | 15.85 | 0.080 | 199 | 2.58 | 0.013 |
| pass 3 | 2 | 263 | 17.09 | 0.228 | 75 | 2.25 | 0.030 |
| pass 3 | 3–5 | 525 | 19.35 | 0.596 | 32 | 5.08 | 0.157 |
| pass 3 | 6–10 | 573 | 24.68 | 2.09 | 11.8 | 7.07 | 0.600 |
| pass 3 | > 10 | 163 | 29.28 | 2.82 | 10.4 | 2.39 | 0.230 |
| pass 2 | 3–5 | 7 | 20.75 | 1.86 | 11.2 | 0.07 | 0.007 |
| pass 2 | 6–10 | 601 | 25.02 | 1.12 | 22.3 | 7.52 | 0.337 |
| pass 2 | > 10 | 1 392 | 37.54 | 6.30 | 6.0 | 26.13 | 4.38 |

Over the 2 000 slots, belief propagation's rounds on candidates that
decoded are 0.31–0.52% of all rounds by set (`edges` 0.09%, `empty`
0); a failing candidate runs 30 rounds (29.6 on average on `empty`);
97.7–98.9% of decodes need under 10 rounds.

### Cortex-M33 (emulated instructions, s0086; setup 9.36 M excluded)

| stage | host, three passes | share | small, one pass | share |
|---|---|---|---|---|
| feed | 581.72 M (193.91 M a pass) | 25.7% | 191.99 M | 67.2% |
| of which the butterflies (with the window, fused into their first stage) | 492.92 M | 21.8% | 164.30 M | 57.5% |
| sync | 91.96 M | 4.1% | 27.47 M | 9.6% |
| soft bits | 24.50 M | 1.1% | 9.21 M | 3.2% |
| belief propagation | 365.41 M | 16.2% | 54.27 M | 19.0% |
| OSD-1, SNR, refine, finish | 7.55 M | 0.3% | 2.95 M | 1.0% |
| subtract (37 calls, 32.198 M each) | 1 191.31 M | 52.7% | — | — |
| **slot** | **2 262.45 M** | | **285.89 M** | |
| decoded | 40 | | 23 | |
| **per decode** | **56.6 M** | | **12.4 M** | |

(Host by pass: 415.94 / 1 056.71 / 789.66 M with 23 / 14 / 3 new
decodes: 18.1 / 75.5 / 263 M per new decode.)

**Top 15 functions** (share of all instructions, setup included:
2 271.81 M host, 295.25 M small; inlined images):

| host | M | share | small | M | share |
|---|---|---|---|---|---|
| `Decoder::pass` (the feed, inlined) | 584.18 | 25.7% | `Decoder::pass` (the feed, inlined) | 194.79 | 66.0% |
| `subtract::blocks` | 363.09 | 16.0% | `bp::min_sum_round` | 52.25 | 17.7% |
| `bp::sum_product_round` | 360.16 | 15.9% | `sync::select` closure | 22.71 | 7.7% |
| `Fit4::add_all` | 291.00 | 12.8% | `__aeabi_memclr4` | 6.33 | 2.1% |
| `Removal::refs` | 145.63 | 6.4% | `_start` (setup) | 4.49 | 1.5% |
| `subtract::run` closure | 144.46 | 6.4% | `Decoder::search` | 4.48 | 1.5% |
| `Synth::fill_phase_only` | 129.09 | 5.7% | `Synth::fill_phase_only` (setup) | 2.77 | 0.9% |
| `sync::select` closure | 75.67 | 3.3% | `memmove` | 2.43 | 0.8% |
| `Synth::skip_with` | 53.78 | 2.4% | `Spectrogram::row` | 1.65 | 0.6% |
| `whole_lane_sums` | 43.60 | 1.9% | `decode_a_slot` | 1.26 | 0.4% |
| `__aeabi_memclr4` | 19.45 | 0.9% | `Synth::fill_phase` (setup) | 0.71 | 0.2% |
| `Decoder::search` | 15.48 | 0.7% | `__aeabi_memcpy4` | 0.63 | 0.2% |
| `subtract::aligned` | 7.90 | 0.3% | `sync::costas_score` | 0.48 | 0.2% |
| `memmove` | 7.30 | 0.3% | `encode_payload` (setup) | 0.11 | 0.0% |
| `subtract::score` | 6.34 | 0.3% | `memcpy` | 0.04 | 0.0% |

**The feed split** (front end out of line; per pass, small image; the
host image's three feeds are three times these, to within 0.1 M):

| part | M a pass | share of the feed | instruction mix (dynamic) |
|---|---|---|---|
| butterflies, stages 2–6 (`later_stages`) | 135.62 | 70.6% | floating-point arithmetic 20.3%, `vldr`/`vstr` 17.0%, integer `add` 24.3%, `cmp` and branches 12.4%, integer loads and stores 9.9% |
| window and first stage (`spectrum`) | 28.68 | 14.9% | floating-point arithmetic 24.6%, `vldr`/`vstr` 24.0% |
| `log2`, levels and counts (`finish_row`) | 9.61 | 5.0% | floating-point 32.0% (`vmov`, `vmrs` and conditional execution beside it) |
| powers (`bin_powers`) | 6.49 | 3.4% | floating-point 37.4% |
| window shift (`memmove`), input checks and copy, rows, zero fills, glue | 11.8 | 6.1% | |

- **The butterflies are 57.5% of the small slot and 21.8% of the
  host's**, and only a fifth of their instructions are floating-point
  arithmetic: a quarter are integer adds (index and address
  arithmetic), an eighth loop control. The twiddles for each block are
  gathered with a `% N` index and `__aeabi_memclr4` is called five times
  from `later_stages` (the per-block twiddle arrays). The
  floating-point work and its loads and stores are 50.6 M of the 135.6 M
  a pass.
- `min_sum_round` (small): floating-point 12.0%, with `vcmp`/`vmrs`/`it`
  and branches beside it (compare-and-select by flags).
- `sync::select`'s closure (the Costas sums): integer only, 41% byte and
  word loads.

### x86-64 (`perf` user cycles, slot synthesis removed)

| | host share | A76 host share | small share | A76 small share |
|---|---|---|---|---|
| subtraction | 42.1% | 48.8% | — | — |
| belief propagation | 29.4% | 21.3% | 18.1% | 31.4% |
| feed | 18.0% | 21.5% | 40.9% | 49.6% |
| sync (refine included) | 6.3% | 5.3% | 31.9% | 11.0% |
| soft bits | 3.6% | 2.7% | 7.9% | 7.0% |
| OSD-1, SNR | 0.3%, 0.3% | 0.2%, 0.3% | —, 1.1% | —, 1.2% |

Top symbols, host (share of the decoder's samples): `sum_product_round`
28.3%, `fft::power_bins` (the feed, inlined) 17.3%, `subtract::run`
11.9%, `Synth::fill_phase_only` 11.0%, `symbol_correlations` 5.6%,
`skip_with` 4.0%, `Removal::refs` 3.9%, `soft_bits_at` 3.6%,
`sync::select` closure 3.6%, `Fit4::add_all` 3.2%, `find_candidates`
2.5%. Small: `power_bins` 39.2%, `min_sum_round` 15.3%, the
`sync::select` closure 13.6%, `find_candidates` 9.5%, `costas_score`
8.1%, `soft_bits_at` 7.9%. **The sync search is a third of the small
decoder's x86-64 cycles** (baseline x86-64, no AArch64 byte-vector
forms); on the A76 it is 11%.

### Memory

**Heap: zero** inside `decode_passes_staged` and `finish` (host, host
OSD off, small; 0 to 40 messages). **Nothing scales with the number of
signals.**

| buffer | host (CLI) | small (CLI) | M33 host image | M33 small image | what for | live |
|---|---|---|---|---|---|---|
| residual (the slot as `f32`) | **720 000** | — | 8 192 (check buffer) | — | the audio each later pass feeds; the subtraction writes it | the whole multi-pass decode |
| spectrogram levels (`u8`) | 179 584 | 161 920 | 179 584 | 162 944 | every pass's sync, refine, soft bits, SNR; idle during a subtraction, which uses it as working storage (46 080 B) | whole decode |
| front end (window 15 360 + FFT 15 360) | 30 720 | 30 720 | 30 720 | 30 720 | the feed | **the feed only** |
| sync scratch (3 rows of scores, `i32`) | 11 532 | 10 380 | 11 532 | 10 380 | the candidate search | **the search only** |
| decode list | 6 144 | 3 072 | 5 120 | 2 560 | the decodes | whole decode |
| candidate scratch (BP or OSD-1) | 4 872 | 4 872 | 4 868 | 4 868 | one candidate at a time | **BP and OSD-1 only** |
| candidates (200) | 1 600 | 1 600 | 1 600 | 1 600 | a pass's candidates | a pass |
| call table | 1 552 | 1 552 | — | — | the CLI's hashed calls | whole decode |
| **total** | **956 004** | **214 116** | **241 616** static | **212 048** static | | |
| constant tables (flash / rodata) | 39 429 | 39 429 | | | | |

- **The residual is 75.3% of the host decoder's memory.**
- **Three buffers are never live at once:** the front end (the feed),
  the sync scratch (the search, after the feed) and the candidate
  scratch (belief propagation and OSD-1, after the search; the next
  pass's feed starts after the last candidate). Holding the other two
  in the front end's 30 720 B saves 16 404 B (host CLI 956 004 →
  939 600, −1.7%; small 214 116 → 197 712, −7.7%; M33 host image
  saves 16 400 B: 241 616 → 225 216 B static, its headroom under the
  245 760 B ceiling 4 144 → 20 544 B). From reading the code; not
  built. It holds as long as a caller never runs one slot's candidates
  while feeding the next.
- **Stack, x86-64 (painted):** 2 768 B (a pass, no OSD-1 run), 4 800 B
  (OSD-1 ran), 9 104 B (a subtraction ran) (prof14: 2 752 / 4 800 /
  8 992).
- **Stack, M33:** frames from `check.sh --stack`: host `subtract_one`
  4 808, `_start` 4 104, the subtraction's closure 2 864,
  `decode_a_slot` 2 512, `Synth::fill` and `Synth::mix` 584,
  `finish_row` and `unpack_dyn` 464, `Synth::skip` 352, `osd_decode`
  272 B. Measured high water of the staged images: host 10 308 B,
  small 10 192 B (10 440 B with the front end out of line); within the
  16 KB stack budget.
- Flash: small 90 274 B, host 102 570 B, encoder only 35 286 B; static
  RAM 212 048 / 241 616 / 0 B (unchanged since prof14).

### Next candidates, ranked

Savings are estimates from this profile unless marked measured.
"Pooled" is crowded + wide + fading `main` (72.33 ms, 21.28 decodes a
slot, host).

**(A) Bit-identical (an O-round):**

| rank | candidate | measured cost now | estimated gain | effort / risk |
|---|---|---|---|---|
| 1 | **M33 butterflies' overhead**: per-stage twiddle strides in place of `% N` gathers and zeroed per-block arrays, pointer-stepped loops, for the groups-of-one (M33) path only; the same operations in the same order | 135.6 M + part of 28.7 M a pass; 57.5% of the small slot, 21.8% of host (measured) | −25 to −45% of `later_stages`: small −34 to −61 M (−12 to −21%), host −102 to −183 M (−4.5 to −8%). A76 and x86-64: none (their path has four lanes) | medium; low risk (M33 emulated identity and the group-of-one tests guard it); flash may grow |
| 2 | **The subtraction, again**: the polynomial passes (38%) and the generator (22%), each 1.5–1.7× its floor; on the M33 `blocks` 363 M and `Fit4::add_all` 291 M (7.9 M a call), the groups-of-one path | A76 43.74 ms (48.8% crowded, ≈ 25.7 ms pooled); M33 1 191 M (52.7%) | A76 −5 to −10% of it: −2.2 to −4.4 ms crowded (−2.4 to −4.9%), −1.3 to −2.6 ms pooled; M33 −10 to −20% of it: −5 to −10% host | high (seven rounds on it already); low risk |
| 3 | **One storage for the front end, sync scratch and candidate scratch** (never live together) | 47 124 B in three buffers | **−16 404 B**: small −7.7%, host −1.7% of decoder memory; M33 host image headroom ×5 | low–medium (an API change to the buffers `Decoder::new` takes); no CPU change expected |
| 4 | **The Costas sums**: each eight-tone sum is recomputed for 21 start rows; a sliding sum along the bins, or shared sums across start rows within the scratch | A76 1.54 ms a pass (5.2% crowded, 10.8% small, 8.6% one-signal); M33 27.5 M small (9.6%), 92 M host; x86-64 small 32% | M33 −30 to −45% of `select`: small −7 to −10 M (−2.5 to −3.5%); A76 0 to −30% of sync (the byte vectors are already wide): small 0 to −0.4 ms (0 to −3%); x86-64 small more | low–medium; integer, exact |
| 5 | **Min-sum's compare-and-select on the M33** (`vcmp`/`vmrs`/`it` and branches) | 52.25 M, 17.7% of small (measured) | −15 to −30% of it: −8 to −16 M small (−3 to −5.5%) | low; NaN and signed-zero cases must keep their results |
| 6 | **Soft bits** | A76 2.38 ms crowded (2.7%), 0.89 ms small (7.0%); M33 9.2 M small (3.2%) | −20 to −40% of it: small −0.2 to −0.35 ms (−1.4 to −2.8%) | low–medium; not profiled inside |
| 7 | **The feed's remainder on the M33** (`finish_row`'s `log2` and levels, powers, window shift, zero fills) | ≈ 28 M a pass (9.7% of small) | −20 to −30% of it: −2 to −3% small | low–medium |
| 8 | **Sum-product's scheduling on the A76** (`log2`, `exp2_neg`) | 19.06 ms (21.3% crowded; 47–55% sparse and empty, almost all on failing candidates) | −3 to −8% of it: −0.6 to −1.5 ms crowded; O17's lever is spent | medium; uncertain |

By per-signal CPU on the embedded target, 1 dominates (the small
decoder is 12.4 M instructions a decode, the butterflies 7.1 M of it);
on the A76 host, 2 is the only bit-identical candidate above 2%;
3 is the only bit-identical memory saving found.

**(B) Results-changing (a gated round):**

| rank | candidate | measured cost now | estimated gain | what it risks |
|---|---|---|---|---|
| 1 | **Skip a later pass after a thin pass.** (a) Pass 2 after a pass 1 of one decode; (b) pass 3 after a pass 2 of ≤ 2 (or 1) | (a) 15.78 ms a run, 0 decodes in 1 244 `edges` slots (measured; prof14: 0 in 10 125); (b) measured per pooled slot below | (a) **−42% of a one-signal slot** (35.96 → ≈ 20.9 ms; 37.5 → ≈ 21.8 ms a decode). (b) ≤ 2: **−4.83 ms a pooled slot (−6.7%)** for 0.043 decodes (0.20%; 112 ms per decode lost against pass 3's 9.17); ≤ 1: −2.58 ms (−3.6%) for 0.013 (0.06%) | the synthetic sets have nothing for pass 2 to unmask behind one signal; needs mixtures of a strong signal over weaker ones (and off-air slots) before a rule is chosen |
| 2 | **Belief propagation on candidates that will fail**: a screen before it (unsatisfied checks of the hard decisions, soft-bit magnitude), or a stall rule that continues while the unsatisfied count is low | every failing candidate runs 30 rounds; > 99.5% of rounds; 19.06 ms crowded (21.3%), 16.97 one-signal (47.2%), 11.38 empty (54.6%), small empty 5.59 (39.3%); M33 host 365 M (16.2%), small 54 M (19.0%) | ceiling at half of belief propagation: crowded −9.5 ms (−10.6%), one-signal −8.5 ms (−24%), empty −5.7 ms (−27%), small empty −2.8 ms (−20%); M33 −8% host, −9.5% small | E1's stall rule lost decodes (slow convergers plateau 15+ rounds) |
| 3 | **An `i16` residual** | 720 000 B, 75.3% of host memory | **−360 000 B (−37.7% of the host decoder)**; CPU about neutral (`feed_i16` exists; the removal writes it) | rounding and saturation of the residual change later passes' input; needs the crowded and sparse gates |
| 4 | **Fewer pass-1 candidates on quiet slots** (a limit from the sync score distribution) | an empty slot runs all 100 (75) | overlaps 2; ceiling the empty slot's 11.38 ms (5.59 small) | losses near threshold |
| 5 | **A different FFT on the M33** (radix, twiddle order), beyond (A) 1 | as (A) 1 | a further −10 to −20% of the butterflies (not estimated from a reference count) | float rounding of every level |
| 6 | **The later passes' feed from the subtracted estimates** | 12.86 ms crowded (14.3%), 6.4 ms of a one-signal slot (17.8%); M33 388 M host (17.1%) | up to those figures | changes rounding and needs more storage: last on the memory goal |

By per-signal CPU and memory together: B1 is the largest saving on
quiet and sparse bands and the cheapest; B2 the largest on crowded
slots; B3 the largest memory saving anywhere (with (A) 3 the only
others found). **Dominated by OSD-1:** nothing (0.2–1.2% of any slot,
0.1% of M33 instructions).

### Not measured

- A76 inside the stages this round (no sampler, no `perf` on the
  Raspberry Pi 5): the subtraction's split is O19's microbenchmark, the
  feed's butterfly share prof14's sample before O16.
- Slots with two or three signals (no such synthetic set); off-air
  recordings; a strong signal over weaker ones (needed before (B) 1).
- The `sweep` set (15 000 one-signal slots; `edges` stands for it) and
  the later-pass yield table on more than one A76 run (its times are
  single samples; its counts are exact).
- M33 cycles (emulated instructions only), the M33 on any slot but
  s0086, and the dynamic instruction mix on the host image or outside
  the small image's front end, min-sum and sync.
- x86-64 stage times (only `perf` shares are given).
- Every gain in the rankings is an estimate. The buffer sharing in (A) 3
  was read from the code, not built. No microbenchmark was run this
  round.
- Energy and power.

## The butterflies' overhead on the Cortex-M33, twentieth round (O20, bit-identical, 2026-11-02)

Base: 70be604. The target is the first candidate of the profile above:
with one lane, the butterflies were 57.5% of the small slot and only a
fifth of their instructions were floating-point arithmetic. Three code
commits, one optimisation each: 698d859, a35a698, e9bb614; then 39476a4
(the code-generation check) and 55ec0d8 (a comment). **x86-64 and AArch64
`.text` and `.rodata` are byte-identical at every code commit**, so
nothing changes on the A76 and it was not timed. It follows that the
x86-64 and AArch64 identity runs do not execute the new code. The
one-lane path is covered end to end only by the 14 M33 dumps and 2
residuals (seven slots), plus the host unit tests.

**Superseded in part.** O20b, below, removes e9bb614's change. In this
section "head" and "now" mean 55ec0d8, and the e9bb614 columns describe
that commit. The current figures, stage table, frames, test counts and
guard bounds are in O20b. This section's account of how a form is selected
(const parameters `L` and `G`) and of the first stage's test coverage is
superseded by O20b and O20c.

### Changes

Only the one-lane paths change. They are selected by the existing const
parameters (`L` = 1, `G` = 1), which `LANES` and `GROUP` set to 1 on
targets without SSE2 or NEON, so the tests run them on any host.

1. **698d859: the later stages written out.** With one lane the
   side-by-side form copied the inputs to stack arrays and ran rolled
   loops over q and m (opt-level "s" does not unroll them), reading each
   twiddle back from a zeroed array. `straight_butterfly` now writes each
   butterfly out term by term from token lists: the inputs load into
   locals and output q is a[0] plus the products of m = 1.., added in the
   order of m. The operations, their values and their order are those of
   `radix_sums`. A stage's twiddles are read once per k into an array
   built from the same lists, which needs no zero fill. The last stage
   reads each butterfly's twiddles where it uses them.
2. **a35a698: the first stage written out** the same way for a group of
   one, with the same window products.
3. **e9bb614: the twiddle index by a conditional subtraction** in place
   of `% N` (see "The index form").

No table was added or moved, so per-stage twiddle tables cost nothing:
`.rodata` of `check.sh`'s images is 53 350 B before and after (53 354 B
in images built at another path: path strings). The last
stage's alone would be 61 440 B (384 × 20 × 8), and it was not pursued.

### Cortex-M33 (emulated instructions; staged images as built, rustc 1.93.0, `--cfg cat_ft8_minmaxnm`)

Setup (about 9.36 M instructions) excluded. Same decodes in every row.

| slot, image | 70be604 | 698d859 | a35a698 | e9bb614 | change |
|---|---|---|---|---|---|
| s0086 small, slot | 285 888 808 | 198 036 498 | 183 674 197 | **182 492 733** | **−36.2%** |
| s0086 small, feed | 191 985 903 | 104 133 619 | 89 771 131 | **88 589 667** | −53.9% |
| s0407 small, slot | 278 709 226 | 190 856 960 | 176 494 671 | **175 313 207** | **−37.1%** |
| s0086 host, slot | 2 262 451 610 | 1 998 624 096 | 1 955 479 134 | **1 951 934 742** | **−13.7%** |
| s0086 host, feed | 581 717 209 | 317 889 877 | 274 744 453 | **271 200 061** | −53.4% |
| s0407 host, slot | 2 258 505 554 | 1 994 678 047 | 1 951 533 085 | **1 947 988 693** | **−13.8%** |

- Per decode: s0086 small (23) 12.43 → 7.93 M, host (40) 56.56 → 48.80 M;
  s0407 small (30) 9.29 → 5.84 M, host (38) 59.43 → 51.26 M.
- Only the feed moves, by the same amount on both slots (the transform
  has no data-dependent path). Sync, soft bits, belief propagation and
  the subtraction are unchanged to within 0.001 M.
- The profile estimated −25 to −45% of `later_stages`; it fell 59.4%.

**Per function, as built, s0086:**

| function | small 70be604 | small e9bb614 | host 70be604 | host e9bb614 |
|---|---|---|---|---|
| `Decoder::pass` (the feed, inlined) | 194 791 077 | 97 475 282 | 584 176 348 | 291 900 320 |
| `__aeabi_memclr4` | 6 333 462 | 253 182 | 19 450 424 | 1 209 584 |
| every other function | unchanged to 0.001 M | | unchanged to 0.001 M | |

**The feed split (measurement images only, the front end out of line,
s0086; totals are the three passes on the host image):**

| part | small 70be604 | small e9bb614 | host 70be604 | host e9bb614 |
|---|---|---|---|---|
| `later_stages` | 135 621 984 | 55 103 584 (−59.4%) | 406 865 952 | 165 310 752 |
| `spectrum` (window and first stage) | 28 683 024 | 11 574 520 (−59.6%) | 86 049 072 | 34 723 560 |
| `__aeabi_memclr4` | 7 392 375 | 253 175 | 22 627 163 | 1 209 563 |
| `finish_row`, `bin_powers`, `memmove`, `checked_copy` | unchanged | unchanged | unchanged | unchanged |
| **feed** | 192 221 790 | 87 455 686 | 581 923 654 | 267 625 342 |

The butterflies are now 36.8% of the small slot (66.68 M of 181.16 M in
those images; 57.5% before) and 10.3% of the host's (200.03 M of
1 948.00 M; 21.8% before).

### The index form

The sign of this change depends on the code around it, so it was
measured in each position. Slot instructions, s0086:

| images | small, `% N` | small, subtraction | host, `% N` | host, subtraction |
|---|---|---|---|---|
| later stages written out only, as built | 198 036 498 | 199 394 445 (+1 357 947) | 1 998 624 096 | 2 002 672 026 (+4 047 930) |
| **every stage written out, as built** | **183 674 197** | **182 492 733 (−1 181 464)** | **1 955 479 134** | **1 951 934 742 (−3 544 392)** |
| every stage written out, front end out of line | 180 143 570 | 181 160 354 (+1 016 784) | 1 944 952 921 | 1 948 003 273 (+3 050 352) |

- **Correction: no mechanism is established.** `Decoder::pass` (small,
  static) has 16 `bhs` and 45 `it` with `% N` against 21 and 50 with the
  subtraction, and the same 16 and 45 against 21 and 50 in the second
  row, where the sign is the other way. So these counts do not explain
  the sign. This section first said they did, and gave `it` as 49 and 54,
  which counted `itt` and its kin as well.
- This section first kept the subtraction because "only the second row
  ships". That overstated it: the second row is the staged measurement
  image, and an application's linked image's sign is unknown. The changes are +0.69%,
  −0.64% and +0.56% of the small slot, noise around zero. O20b removes
  the subtraction, for `% N`, the shorter form.

### What bounds the butterflies now

Per block counts of `later_stages`, small image, 184 rows, s0086. The
reference is the arithmetic plus one load of every input and twiddle
and one store of every output, with no loop control.

| stage | butterflies a pass | instructions a pass | a butterfly | reference |
|---|---|---|---|---|
| radix 4, l = 4 | 88 320 | 9 154 184 | 103.6 | 104 |
| radix 4, l = 16 | 88 320 | 9 318 496 | 105.5 | 104 |
| radix 2, l = 64 | 176 640 | 4 617 480 | 26.1 | 24 |
| radix 3, l = 128 | 117 760 | 7 491 376 | 63.6 | 60 |
| radix 5, l = 384 | 70 656 | 24 522 048 | 347.1 | 180 |
| **total** | | **55 103 584** | | 42 393 600 |

- **The first four stages are at the reference.** Correction: the
  radix-4 reference is 104, not 112 (total 42 393 600, not 43 806 720). A
  radix-4 butterfly has 64 arithmetic instructions, not 72: the m = 2
  twiddle is the same table entry for q and q + 2, so those outputs share
  one product and the compiler merges them, exactly. (104 still counts 4
  twiddle loads the compiler need not make: see O20b.) This section first
  put the whole shortfall in loads, which was wrong. The arithmetic and
  the loads and stores are the bound there, and they cannot fall with the
  same operations in the same order.
- **The last stage holds all the remaining overhead:** 167 instructions
  a butterfly, 11.8 M a pass (6.5% of the small slot, 1.8% of the host's).
  Each butterfly addresses 10 inputs 1 536 B apart and 20 twiddles
  3 072 B apart, beyond `vldr`'s reach of 1 020 B, so each needs an
  address in an integer register, and those spill. Its wrap tests are
  also there.
- Mix of `later_stages` (the same count, 55 103 584): floating-point
  arithmetic 47.4%, `vldr`/`vstr` 24.2%, integer add and subtract 12.0%,
  integer loads and stores 6.5%, compares and branches 5.4%, other 4.3%.
  (Before: arithmetic 20.3%.)
- **Not built, estimates only:** k split into its six wrap-free ranges
  (breakpoints 96, 128, 192, 256, 288; about +3.5 KB of code) or a
  twiddle table extended by 1 149 entries (+9.2 KB of flash) would remove
  the wrap tests but not the addressing. Estimated gain 1.5–3% of the
  small slot.
- The first stage is at its reference too: 131 instructions a block
  against 128.

### Sizes and stack (Cortex-M33; sizes and frames from `check.sh --stack`)

| image | flash 70be604 → e9bb614 (B) | static RAM |
|---|---|---|
| small | 90 274 → 91 966 (+1 692) | 212 048 (unchanged) |
| host | 102 570 → 104 262 (+1 692) | 241 616 (unchanged) |
| small, OSD on | 90 274 → 91 966 | 212 048 |
| host, OSD off | 102 570 → 104 262 | 241 616 |
| encoder only | 35 286 (unchanged) | 0 |

- `.text` by commit (check images, small and host alike): +1 632, +72,
  −12 B. `.rodata` unchanged. The flash cost is code, not tables.
- Frames: `finish_row` 464 → 488 B; every other recorded frame is
  unchanged (`_start` 4 104, `decode_a_slot` 2 512, host `subtract_one`
  4 808, the subtraction's closure 2 864, `osd_decode` 272).
- High water, from the emulator's painted stack on the staged images (not
  `check.sh --stack`, and not the check images): small 10 192 B, host
  10 308 B, both the same as those images at 70be604. A sum of frames on
  the check image's deepest path is about 14.3 KB (from review, not
  re-derived); `finish_row` is not on it. Both are within the 16 KB budget.
- No fused multiply-add in any image. `vmla` 79 → 128 (small), 162 → 211
  (host); it is the non-fused form, which rounds the product first. No
  `f64` helper. Heap-free, MSRV 1.75.

### Cortex-M4F (thumbv7em, embedded-check images, no cfg)

| image | flash 70be604 → e9bb614 (B) | static RAM | `fmaxf`/`fminf` call sites | `__aeabi_memclr4` call sites |
|---|---|---|---|---|
| small | 90 628 → 92 340 (+1 712) | 212 048 | 10 → 10 | 33 → 26 |
| host | 103 096 → 104 804 (+1 708) | 241 616 | 14 → 14 | 48 → 41 |

- The written-out transform adds no library call. The `fmaxf`/`fminf`
  calls were there before and are outside the transform: on host,
  `decode_a_slot` 6, `subtract_one` 4, `finish_row` 2,
  `sum_product_round` 2. The code-generation check's M4 ceilings look at
  other functions, in the library at the release profile, so they pass.
- No `f64` helper and no fused multiply-add. The M4 takes the same
  one-lane paths; its instruction counts were not measured.

### Code-generation check (39476a4)

A thumbv8m.main build of the library at opt-level "s", where the
transform is inlined into `finish_row`:

| source | `f32` multiplies (floor 200) | divisions (ceiling 0) | zero-fill calls (ceiling 1) | result |
|---|---|---|---|---|
| head | 237 | 0 | 1 | passes |
| 70be604 | 71 | 5 | 8 | all three fire (the script run on that tree) |
| first stage still as loops | 243 | 1 | 2 | the two ceilings fire (counted on that build) |

At the release profile's opt-level the loops are unrolled anyway (329
multiplies against 313), so the multiplies do not separate the forms
there; the divisions do (23 against 0).

### Identity

The decoder runs used e9bb614's source (`cat-ft8/src` tree identical).
Later commits change a comment and a check script; all eight binaries'
`.text` and `.rodata` at the head are byte-identical to it.

- **Records, x86-64:** the 73 800 `paired_bench run` records equal
  b93516e's (host, one pass, small: 24 600 each, 0 differ).
- **800 WAV slots** × 3 configurations × 3 modes (7 200 runs, 78 690
  decode lines): byte-identical to b93516e's.
- **A76:** the 73 800 records from its AArch64 `paired_bench` equal
  x86-64 b93516e's (0 differ); the 800 WAV slots × 3 × 3 with its AArch64
  `cat-ft8-decode` are byte-identical to them. One benchmark at a time;
  314 throttle samples, start and end included 0x0, none non-zero. Not
  timed. A first run was void and discarded: only its first block ran
  and its configurations ran concurrently, and the run's driver script
  then hung because its `wait` also waited on the throttle sampler. Nothing from
  it was compared.
- **Encoder:** the 45 WAV files are identical to 70be604's and 7e388d3's.
- **Cortex-M33 (emulated):** the small and host images' decodes and
  statics on seven slots (14 of 14), and the multi-pass residuals on
  s0086 and s0407, are byte-identical to 68634cc's.
- The same M33 identity also holds for the `% N` form (a35a698 with one
  comment changed; `.text` and `.rodata` of the eight binaries as
  a35a698).

### Tests and checks

- New: `straight_stages_equal_the_side_by_side_stages_bit_for_bit`
  (every bit, the sign of zero included) and
  `twiddle_index_equals_the_stage_index` (every k, m, q of every stage
  after the first).
  The first stage's group of one is covered by the existing test against
  the stored window and stage.
- Mutants caught: the sums' order of m reversed in each of radix 4
  (both) and radix 3 with loads and twiddles left in order (a control
  with the same rewrite passes); radix 5's order (correction: this line
  first claimed it without a logged run; run and logged in O20b); the
  first stage's sum order, a swapped mirrored window entry and a wrong
  window entry; the index's reduction, by `>` for `>=` (the form then in
  the tree), and q alone in its product; a product's sign. Swapping the
  operands of one addition, which is exact, survives as it must.
  Reversing a stage's whole token list also passes for the four
  `straight_stage!` stages: it reverses the loads and twiddle rows with
  the sums, so the order is unchanged. Radix 5 reads its twiddles
  directly, so there it is caught.
- Local CI passes at the head: fmt; clippy `-D warnings` (workspace, all
  targets and features, thumbv8m); workspace, profile, no-std and release
  tests with the ignored ones (203); the group-one cfg step (195 run,
  8 ignored); the bulk run; MSRV 1.75; docs with and without `std`;
  `check.sh --stack`; the code-generation check.
- The A76 library tests were not run: AArch64 code is unchanged.

### Not measured

- Cortex-M33 and M4 cycles and any run on hardware: every figure is
  emulated instructions, which count a `vldr`, an `add` and a
  `vmul` alike. Flash wait states and cache behaviour on a transform
  1.7 KB larger are unknown, so the cycle saving may differ from the
  instruction saving.
- Any M33 slot but s0086 and s0407. The transform's count is the same on
  both.
- M4 instruction counts, stack and `check.sh`-style frames, and any
  decoder identity run on the M4.
- A76 and x86-64 timing: their `.text` is identical, so none was taken
  and no placement check was needed.
- The two last-stage forms above were not built; their gains are
  estimates.
- Decoder identity at 698d859 alone. All of it ran on e9bb614's source,
  and the M33 part also on the `% N` form above.

## Review follow-ups (O20b, bit-identical, 2026-11-02)

On top of O20; its commits stay as they are. **The head's staged M33
images are byte-identical to a35a698's in `.text` and `.rodata`, so the
head's counts are a35a698's to the instruction.** x86-64 and AArch64
`.text` and `.rodata` are byte-identical to 70be604's on all four
binaries. rustc 1.93.0; M33 `--cfg cat_ft8_minmaxnm`.

### Changes and what each did to the counts

| commit | change | M33 images |
|---|---|---|
| e79e439 | the twiddle index back to `% N` | small +1 181 464, host +3 544 392 a slot; `.text` +12 B |
| 1fa49d2 | one decision on the lane count, in `spectrum`; `later_stages` for the tests only; `straight_first_stage` against `first_stage::<1>` directly | none, bytes identical |
| 3a31751 | a compile-time block that checked nothing about the calls removed | none, bytes identical |
| 0b7442a, d37c5db, 3fd8dfc, cd9ab7c | macro trims: the twiddle expression yields `[re, im]`; the output list passed through whole; clearer binder names; the twiddle rows in a macro of their own | none, bytes identical |
| 1ff84ef | one loop form for k | none, bytes identical |
| 80ff84c | comments | none |
| 6b83c9f | test inputs in a helper; the failing input is named | tests only |
| 0ad1e3f, a3f4264 | the guard's floor, and what it misses | check script only |

No trim was dropped: all five leave both staged images' `.text` and
`.rodata` unchanged, so none needed an emulated count.

The subtraction is removed on the evidence of O20's three-row table: its
sign flips with the surrounding code (+0.69%, −0.64%, +0.56% of the small
slot), and no mechanism for that is established.

### Slot change by optimisation (emulated instructions, s0086)

698d859's and a35a698's messages give no numbers.

| commit | small | host |
|---|---|---|
| 698d859, later stages written out | −87 852 310 | −263 827 514 |
| a35a698, first stage written out | −14 362 301 | −43 144 962 |

On s0407 they are within 44 instructions on small and 7 on host.

### Cortex-M33 at the head (staged images, setup excluded)

| slot, image | 70be604 | head | change | decodes | per decode |
|---|---|---|---|---|---|
| s0086 small, slot | 285 888 808 | **183 674 197** | **−35.8%** | 23 | 12.43 → 7.99 M |
| s0086 small, feed | 191 985 903 | 89 771 131 | −53.2% | | |
| s0407 small, slot | 278 709 226 | **176 494 671** | **−36.7%** | 30 | 9.29 → 5.88 M |
| s0086 host, slot | 2 262 451 610 | **1 955 479 134** | **−13.6%** | 40 | 56.56 → 48.89 M |
| s0086 host, feed | 581 717 209 | 274 744 453 | −52.8% | | |
| s0407 host, slot | 2 258 505 554 | **1 951 533 085** | **−13.6%** | 38 | 59.43 → 51.36 M |

Per function, as built, s0086: `Decoder::pass` 194 791 077 → 98 656 746
(small), 584 176 348 → 295 444 712 (host); `__aeabi_memclr4` 6 333 462 →
253 182, 19 450 424 → 1 209 584. Every other function as before.

The butterflies with the window, measured with the front end out of line:
164 305 008 → 65 507 496 small, a pass (−60.1%), 36.4% of the slot;
492 915 024 → 196 522 488 host, 10.1% of the slot. In those images all of
it now sits in `spectrum`; the O20 split by `later_stages` no longer
applies.

### What bounds the butterflies at the head

From `spectrum`'s block counts (front end out of line, small, 184 rows,
s0086). Reference: the arithmetic, one load of every input and twiddle,
one store of every output, no loop control. Arithmetic is counted from
the blocks.

| stage | butterflies a pass | instructions a pass | a butterfly | arithmetic | reference |
|---|---|---|---|---|---|
| radix 4, l = 4 | 88 320 | 8 539 624 | 96.7 | 64 | 104 |
| radix 4, l = 16 | 88 320 | 8 976 992 | 101.6 | 64 | 104 |
| radix 2, l = 64 | 176 640 | 4 594 296 | 26.0 | 12 | 24 |
| radix 3, l = 128 | 117 760 | 7 372 696 | 62.6 | 36 | 60 |
| radix 5, l = 384 | 70 656 | 24 803 568 | 351.0 | 120 | 180 |
| **later stages** | | **54 287 176** | | | 42 393 600 |
| window and first stage | 88 320 | 11 220 320 | 127.0 | | |

- The m = 2 twiddle of radix 4 is the same table entry for q and q + 2, so
  two outputs share a product and the compiler merges them (64, not 72).
  The reference of 104 still counts the 4 twiddle loads the compiler need
  not make, so 100 is the tighter floor for both radix-4 stages, a total
  of 41 687 040. The conclusion for the first four later stages stands. On
  the main path each radix-4
  butterfly has 19 `vldr`: 8 inputs and 11 twiddle values, the rest being
  the merged ones or held in registers across blocks.
- The first four later stages are at the reference. **Radix 5 holds the
  remaining overhead:** 171 instructions a butterfly, 12.1 M a pass (6.6%
  of the small slot, 1.9% of the host's).
- Dynamic mix of the 65 507 496: `f32` arithmetic 46.9%, `vldr`/`vstr`
  21.5%.

### Sizes and stack at the head

| image | flash 70be604 → head (B) | static RAM |
|---|---|---|
| M33 small | 90 274 → 91 978 (+1 704) | 212 048 (unchanged) |
| M33 host | 102 570 → 104 274 (+1 704) | 241 616 (unchanged) |
| M33 encoder only | 35 286 (unchanged) | 0 |
| M4 small | 90 628 → 92 340 (+1 712) | 212 048 (unchanged) |
| M4 host | 103 096 → 104 804 (+1 708) | 241 616 (unchanged) |

- **Frames (`check.sh --stack`, M33):** every recorded frame equals
  70be604's, `finish_row` 464 B included (its 488 B went with the
  subtraction).
- **Stack paint, staged images:** small 10 192 B, host 10 308 B, the same
  as those images at 70be604; 10 364 B and 10 308 B with the front end out
  of line.
- **M33:** no fused multiply-add (`vmla` 128 small, 211 host), no `f64`
  helper.
- **M4 scan:** `fmaxf`/`fminf` call sites 10 and 14 (as at 70be604),
  `__aeabi_memclr4` 26 and 41, no `f64` helper, no fused multiply-add.

### Guard (`codegen-check.sh`, thumbv8m.main, opt-level "s", `finish_row`)

Floor of `f32` multiplies raised from 200 to 230. Each form was run
through the script, with the rolled loops written by hand with no zero
fill and no division, unless stated.

| form | multiplies | divisions | zero fills | result |
|---|---|---|---|---|
| head | 238 | 0 | 1 | passes |
| radix 4, l = 4, as loops | 202 | 0 | 1 | floor fires |
| radix 4, l = 16, as loops | 202 | 0 | 1 | floor fires |
| radix 3 as loops | 218 | 0 | 1 | floor fires |
| radix 5 as loops | 171 | 1 | 1 | floor and division ceiling fire |
| radix 3 and 2 as loops | 214 | 0 | 1 | floor fires |
| radix 4 (l = 4) and 2 as loops | 198 | 0 | 1 | floor fires |
| every later stage but radix 5 as loops | 142 | 0 | 1 | floor fires |
| `stage` in place of the later stages | 66 | 4 | 7 | all fire |
| `first_stage::<1>` in place of the first stage | 243 | 1 | 2 | ceilings fire |
| **radix 2 alone as loops** | **234** | 0 | 1 | **passes: missed** |

It also misses, as its header says: the form of the twiddle index; a zero
fill by `__aeabi_memclr8` or a tail call; what link-time optimisation does
to the image. A first stage as loops with no division and no zero fill was
not run. A floor of 235 would catch radix 2 alone, with a margin of 3.

### Identity at the head

- **Cortex-M33 (emulated):** the small and host images' decodes and statics
  on seven slots (14 of 14) and the residuals on s0086 and s0407 are
  byte-identical to 68634cc's, on this tree.
- **x86-64 and AArch64:** `.text` and `.rodata` as at 70be604, so O20's
  records and WAV runs carry over. They do not execute the one-lane code.
- **Encoder:** the 45 WAV files are identical to 70be604's.
- The residual comparison was confirmed to fail on one flipped byte in a
  copy of a residual. All 16 comparisons above ran after that confirmation.
- O20's copies of the staged images had been overwritten by a build at
  another path. The head's were built once, from a fresh tree.

### Tests and checks

- New: `straight_first_stage_equals_first_stage_with_a_group_of_one`. With
  the choice now made in `spectrum`, nothing else reaches that function on
  a host. Mutants of it (a swapped mirrored window entry, a wrong window
  entry, the sum order reversed) are caught by this test alone.
- Radix 5's sum order: [4 3 2 1], [1 2 4 3] and [2 1 3 4] are caught, and
  the unmutated control passes.
- Local CI passes at the head: fmt; clippy;
  workspace, profile, no-std and release tests with the ignored ones (204);
  the group-one cfg step (196 run, 8 ignored); the bulk run; MSRV 1.75;
  docs; `check.sh --stack`; the code-generation check (19 bounds).

### Not measured

- Cycles and any run on hardware; flash wait states. Every count is
  emulated instructions.
- Any M33 slot but s0086 and s0407; M33 decoder identity beyond the seven
  slots.
- M4 instruction counts, stack, frames and any decoder identity run.
- The index's sign in an application's linked image.
- A76 and x86-64 timing, and their record and WAV runs at this head: their
  sections are unchanged, so they were not repeated.
- Other one-lane targets.

## Closure follow-ups (O20c, 2026-11-02; no code generation change on the measured targets)

`.text` and `.rodata` of all eight binaries are unchanged: x86-64 and
AArch64 as at 70be604, the M33 images as at a35a698 (rustc 1.93.0; M33
`--cfg cat_ft8_minmaxnm`). So O20b's counts, sizes, frames and M33
identity stand, and the decoders were not rerun.

- **14c5edb: a coverage gap closed.** No host test ran `spectrum`'s
  one-lane branch, so a fault there would have passed on every host. The
  choice is now `spectrum_lanes::<L>`, which `spectrum` calls with `LANES`,
  and `spectrum_is_the_same_for_every_lane_count` compares one, two and four
  lanes bit for bit over the first stage's test windows. `GROUP` had no
  other use and is gone.
- **Faults in that branch,** each caught by the new test alone: the later
  stages' or the first stage's real and imaginary parts swapped, the later
  stages run twice, the later stages omitted. Only the last also fails the
  code-generation check, which looks at the code's shape, not its results.
  An unmutated control passes, and passing two lanes in place of `LANES`
  passes, as it must, since the results are the same. A fault in the vector
  branch is caught by 16 tests.
- ffca661, 3beaf24, f068f0f, ecd5110: comments, macro parameter names, a
  redundant check and the guard's header. The floor stays 230.
- O20's "superseded" notice is widened, and the radix-4 floor of 100 is
  stated. Both radix-4 stages merge the m = 2 terms, so it applies to all
  176 640 of their butterflies: 42 393 600 − 4 × 176 640 = 41 687 040.
- O20b's mutants other than those of `spectrum_lanes` ran before its macro
  trims. A review re-ran equivalents on the trimmed code, with the same
  results.

### Not measured

- As in O20b. In addition, the new test runs on hosts only, so it shows
  that the branch computes the right values on a host, not on a one-lane
  target; the M33 dumps and residuals remain the only end-to-end evidence
  there, and they were not rerun because the images are unchanged.

## One storage for the sync scores and the FFT, twenty-first round (O21, bit-identical, 2026-11-02)

Base: e4cfa58. The target is candidate (A) 3 of the profile above. Three
commits: 499b196 (the change and its tests), df98a1b (a code-generation
check), 725c433 (a comment rewrapped, the same number of lines).

### The profile's claim and its estimate

**The liveness claim holds on one path only.** The profile said that the
front end, the sync scratch and the candidate scratch are never live
together, and that holding the other two in the front end's 30 720 B saves
16 404 B. By reading the code, then by test:

| buffer | live in `decode_passes`, `decode` (HOST: 3 passes, OSD-1; SMALL: 1 pass, OSD off) | live in the split API | carries anything between |
|---|---|---|---|
| FFT work (15 360 B) | inside one row's transform | the same | nothing: a transform writes every cell before it reads one, on each lane count (tested) |
| input window (15 360 B) | from a slot's first feed to its last | **from one feed to the next, across any search and candidates between them** | the samples of the next row, within a slot |
| sync scratch (11 532 / 10 380 B) | one search | the same | nothing: `select` fills each row before reading it (tested) |
| candidate scratch (4 872 B) | one candidate, then the rounds read at once | **one per worker, all live together** (`decode_candidate(&self, ..)`) | `last_rounds()` and the `Work` tag, between candidates only |
| levels | feed to the last candidate; working storage in a subtraction | the same | a pass's rows |

- On `pass()`, for both presets, the three are never live together and
  no scratch carries a result between passes or slots. So the claim is true
  of that path.
- It is not true of the decoder's API, which documents feeding after a
  search. The profile's caveat named one slot's candidates against the next
  slot's feed; the case is within one slot. Keeping the scores in the window
  fails 2 tests (below). `CandidateScratch` per worker is ADR 0024's design.
- **The window need not be touched.** The FFT work alone has room for either
  of the other two, and neither is live with it or with the other, so the
  profile's total did not depend on the window.

**The estimate of 16 404 B did not hold; 11 532 B did (10 380 B on the small
band).** The other 4 872 B is the candidate scratch, not built:

- It has fields of `u8`, `u16`, `f32` and 32- or 64-bit words, so without
  `unsafe` it needs an enum with the FFT buffer, and each change of variant
  writes the whole new variant.
- The one-thread entry points would lose their scratch argument, while the
  split API keeps its own for workers.
- A probe of the fills alone, in a scratch copy (one fill of the FFT buffer
  and one of the candidate scratch a pass; no tag checks, no API change), s0086
  against 499b196: `__aeabi_memclr4` small +7 399, host +22 197 instructions;
  slot small +253 653 (+0.14%), host +952 313 (+0.05%), most of it code moving
  in `Decoder::pass` rather than the fills; stack high water small 10 200 →
  10 656 B (+456 B), host unchanged. That is 4 868 B on the M33 for a fill, a
  stack rise and a second way to call the decoder.
- **An error in the profile:** its small figure subtracted 16 404 B too. Both
  buffers on the small band are 15 252 B (214 116 → 198 864 B, not 197 712 B).

### The change

- `FftBuffer` is one `[f32; 3 840]`, which the transform takes as its two
  parts and the search takes as cells.
- A scratch cell holds a score's bits (`f32::from_bits`, `to_bits`). Small
  scores are subnormals as `f32` and negative ones NaNs; no floating-point
  arithmetic touches them. rustc 1.93.0 folds the conversions into integer
  loads and stores: the M33's summing loop has no `vmov`.
- `Spectrogram` holds its levels, band and row count in a field `Rows`, so
  that the rows can be read while the cells are written.
- `Decoder::sync_scratch_len(band)` is 0 for up to 1 280 base bins (3 975 Hz
  through `Band::from_hz`, whose widths are multiples of 25 Hz). A `const`
  assertion covers both presets and another the check image's band, so that
  image passes `&mut []` and has no buffer. A wider band still passes a scratch.

**API change.** `Decoder::new`'s `sync_scratch` and `sync::find_candidates`'
`scratch` become `&mut [f32]`, and `Decoder::sync_scratch_len` means that
argument's length, so `--profile`'s `sync_scratch_bytes` reads 0. Nothing has
been released, so no compatibility is kept. Recorded in ADR 0024.

### Sizes and stack

From `check.sh --stack` (Cortex-M33), rustc 1.93.0, `--cfg cat_ft8_minmaxnm`:

| image | flash e4cfa58 → 725c433 (B) | static RAM (B) |
|---|---|---|
| small | 91 978 → 92 082 (+104) | 212 048 → **201 668** (−10 380) |
| host | 104 274 → 104 318 (+44) | 241 616 → **230 084** (−11 532) |
| small, OSD on | 91 978 → 92 082 | 212 048 → 201 668 |
| host, OSD off | 104 274 → 104 318 | 241 616 → 230 084 |
| encoder only | 35 286 (unchanged) | 0 |

- Headroom under the host image's 245 760 B ceiling: 4 144 → 15 676 B.
- Cortex-M4F (thumbv7em, no cfg): small flash 92 340 → 92 348, static RAM
  212 048 → 201 668; host flash 104 804 → 104 812, static RAM 241 616 →
  230 084. No `f64` helper, no fused multiply-add, `fmaxf`/`fminf` calls
  unchanged.
- `.rodata` of the images: unchanged in size. No fused multiply-add on the
  M33; `vmla` unchanged (128 small, 211 host).
- **Stack rises by 8 B, far less than the saving.** `decode_a_slot`'s frame
  2 512 → 2 520 B in every decoder image; every other frame listed unchanged
  (host `subtract_one` 4 808, `_start` 4 104, the subtraction's closure 2 864,
  `finish_row` 464, `osd_decode` 272). Paint high water of the staged images:
  small 10 192 → 10 200 B, host 10 308 → 10 308 B.
- Host `size_of::<Decoder>()` 208 → 208 B: the field stays, for wide bands.
- Host decoder memory, from `--profile`'s `peak_bytes`: host 956 004 → 944 472
  B (−1.2%), small 214 116 → 203 736 B (−4.8%).

### Time

**Cortex-M33** (emulated instructions, setup excluded, images of 725c433, whose
sections equal 499b196's; same decodes). No copy or fill was added.

| slot, image | e4cfa58 | 725c433 | change |
|---|---|---|---|
| s0086 small (23 decodes) | 183 674 197 | 183 016 779 | −657 418 (−0.36%) |
| s0407 small (30) | 176 494 671 | 175 802 888 | −691 783 (−0.39%) |
| s0086 host (40) | 1 955 479 134 | 1 953 607 521 | −1 871 613 (−0.10%) |
| s0407 host (38) | 1 951 533 085 | 1 949 658 625 | −1 874 460 (−0.10%) |

| stage, s0086 | small e4cfa58 | small 725c433 | host e4cfa58 | host 725c433 |
|---|---|---|---|---|
| feed | 89 771 131 | 89 135 405 | 274 744 453 | 272 837 275 |
| sync | 27 472 214 | 27 472 192 | 91 956 611 | 91 956 545 |
| soft bits | 9 212 930 | 9 294 742 | 24 502 583 | 24 720 202 |
| belief propagation | 54 265 018 | 54 255 300 | 365 407 067 | 365 388 008 |
| SNR | 2 265 451 | 2 171 445 | 3 953 121 | 3 789 803 |
| subtract | — | — | 1 191 312 698 | 1 191 312 702 |

- The search costs what it did, to 22 instructions in 27.5 M.
- **The feed's fall is not a saving of this design.** The search is not in
  it. All of it is in `Decoder::pass`, where the front end, soft bits and SNR
  are inlined, and soft bits rise while SNR falls. The two regressed forms
  below have the feed at 89 771 124, so it follows the same register choices.
  An application's own image may not get it.

**x86-64** (Threadripper 3970X, core 5, `perf stat` user cycles; builds aligned,
from one source path, alternated; each sample `cat-ft8-decode` 20 times or one
`slotcost`; medians; nothing else running, checked around every sample, none
void):

| run | e4cfa58 cycles | 499b196 cycles | change | spread of samples |
|---|---|---|---|---|
| s0086 small (6 samples) | 29 611 550 | 29 537 978 | −0.25% | 2.2%, 3.7% |
| s0407 small (6) | 28 862 698 | 28 883 390 | +0.07% | 3.2%, 3.1% |
| s0086 host (6) | 231 787 966 | 231 331 080 | −0.20% | 1.4%, 2.9% |
| s0407 host (6) | 230 925 788 | 231 130 403 | +0.09% | 2.4%, 1.4% |
| `slotcost` host, one pass (4) | 328 476 646 832 | 328 054 970 628 | −0.13% | 0.4%, 0.8% |
| `slotcost` small (4) | 95 467 735 636 | 96 038 872 832 | +0.60% | 0.9%, 0.5% |
| `slotcost` small, second run (6; O21b) | 96 271 811 428 | 95 865 852 551 | −0.42% | 0.8%, 0.9% |

Instructions fall by at most 0.003%. The +0.60% did not repeat: a second run
of six alternated samples, checked for other jobs around each sample like the
rest and with its stage times kept, gives −0.42%, and every stage equal within
its spread. These differences are noise; no rise is shown. (Correction, O21b:
the second run first given here, −0.13%, was not checked for other jobs; it is
replaced.)

**Cortex-A76** (Raspberry Pi 5; `.text` changed, so timed). Core 3, builds
aligned from one source path, alternated, four runs each; 16 runs, 1 357
samples during them, all 0x0 at start, during and end; clock 2 400 MHz during
every run; peak 53.8 °C. One warm-up sample read 1.5 GHz (the first run's).

| ms per slot (medians) | host, three passes | host, one pass | small |
|---|---|---|---|
| slot | 89.60 → 89.60 | 18.40 → 18.30 | 12.80 → 12.80 (B); 12.80 → 12.90 (A) |
| feed | 19.265 → 19.210 | 6.51 → 6.46 | 6.355 → 6.370 (B); 6.350 → 6.415 (A) |
| sync | 4.670 → 4.680 | 1.54 → 1.54 | 1.38 → 1.38 |
| soft bits (placement check) | 2.38 → 2.38 | 1.19 → 1.19 | 0.89 → 0.89 |
| belief propagation (placement check) | 19.050 → 19.060 | 8.94 → 8.94 | 4.020 → 4.015 |
| subtract (placement check) | 43.735 → 43.720 | — | — |

The stages this change does not touch agree within 0.015 ms, so placement did
not move the comparison.

**Correction (O21b): the small configuration's feed rose.** `slotcost small
small` measures the same configuration twice, interleaved, as A and B; this table
first gave B alone. A: the feed 6.350 → 6.415 ms (+1.0%) and the slot 12.80 →
12.90 ms, in each of the four runs (feed 6.40–6.42 against 6.34–6.36); B: the
feed 6.355 → 6.370 ms (+0.24%). At e4cfa58 A and B agree; at 499b196 A reads
0.03–0.05 ms above B in every run. The front end's AArch64 code is the same
length but not the same bytes, so a cost of 0.2–1.0% of the small feed (0.1–0.5%
of the slot) on the A76 is not excluded, and its cause is not known. The host
configurations' feeds fell.

### Identity

Decoder runs used 499b196's sources; x86-64, AArch64 and M33 `.text` and
`.rodata` of all eight binaries at 725c433 equal 499b196's (one source path,
compared section by section). The M33 identity was also rerun at 725c433.

- **Records, x86-64 and A76:** 73 800 `paired_bench run` records equal
  b93516e's (host, one pass, small: 24 600 each, 0 differ).
- **800 WAV slots** × 3 configurations × 3 modes (7 200 runs, 78 690 decode
  lines), x86-64 and A76: 93 091 lines byte-identical to b93516e's and 2 400
  `--profile` lines differ as designed, only in `sync_scratch_bytes` N → 0 and
  `peak_bytes` P → P − N, with N the band's 11 532 or 10 380 B. **So this output
  is not byte-identical, unlike earlier rounds', and a new comparator is used.**
- **A76 throttle:** one benchmark at a time; 314 samples, start and end 0x0,
  none non-zero.
- **Encoder:** the 45 WAV files equal 70be604's.
- **Cortex-M33 (emulated):** the small and host images on seven slots (14 of 14)
  and the residuals of s0086 and s0407 equal 68634cc's, at 499b196 and at 725c433.
- **Comparators on a forced difference:** a record field → reported as different;
  a digit of a decode, `peak_bytes` off by one, `sync_scratch_bytes` 4, another
  memory figure off by one → the WAV comparison reports a difference and exits 1
  each; a bit of one M33 dump → the M33 comparison fails and exits 1. Unforced they
  pass. (Correction, O21b: the exit statuses first given here were not the
  comparators' own; see O21b.)

### Tests and mutants

New: `successive_slots_on_one_decoder_match_fresh_decoders` (six different slots,
one of them cut short, on one decoder and scratch against fresh ones; list, reports,
residual and rows; HOST, one pass, SMALL);
`a_search_and_candidates_mid_slot_leave_the_feeding_undisturbed`;
`what_the_shared_cells_held_does_not_reach_the_result`;
`a_search_finds_the_same_wherever_its_scores_are_kept`;
`a_transform_does_not_read_what_its_cells_held` (every lane count);
`select_takes_any_scores_and_ignores_what_its_scratch_held`;
`a_score_survives_a_cell_bit_for_bit`;
`sync_scores_stay_in_the_front_end_up_to_the_fft_cells`.

Faults in a copy of 499b196, host library tests, failing tests per fault:

| fault | failing tests |
|---|---|
| none (control) | 0 |
| scores kept in the input window, the profile's form | 2 |
| a search spoils a held window sample of weight (cell 1 000) | 2 |
| the transform adds what a cell held | 10 |
| a score row not cleared before it is summed | 32 |
| row q + 1 read unfilled at q = 2 | 2 |
| no score written as score 0 | 1 |
| scaling a cell in floating point | 1 |
| a cell loses its sign | 34 |
| a new slot keeps the samples held | 16 |
| the search always uses the work area, wide bands too | 1 |
| the search always uses the separate scratch | 21 |
| the limit one cell lower | 1 |
| the limit a row's worth of cells higher | 3 |
| the limit one part, not both | does not compile (the `const` assertion) |

Three faults pass, each for a reason that was checked: spoiling the window's last
cell (it holds nothing once a row is taken) and its first (its Hann weight is 0, and
the next shift discards it), and real and imaginary parts stored the other way round
throughout (only their place changes). The first two were first written as faults
that should fail; they were wrong faults, and the cell 1 000 one replaces them.

Local CI passes at 725c433 (and at 499b196): fmt; clippy `-D warnings`; workspace,
profile, no-std and release tests with the ignored ones (213); the group-one step
(205 run, 8 ignored); the bulk run; MSRV 1.75; docs; `check.sh --stack`; the
code-generation check. A76 release library tests with the
ignored ones, cross-built at 725c433: 212 pass, 0 fail, the source-tree test skipped
(the Raspberry Pi holds no sources); 48 throttle samples, start and end 0x0, none
non-zero. A first run at 499b196, also passing, sampled only at its start and end, so
it does not count and was repeated.

### The code-generation check (df98a1b)

How the rows reach the search matters more than the storage. With the rows passed
by value through a `Copy` view, or with `Rows::row` inlined, the sum's loop loses
registers: M33 small s0086 189 319 709 (by value), 189 319 772 (inlined) against
183 016 779, +3.44% (+3.07% against e4cfa58), the same decodes. The sync stage alone
rises 5.65 M, about a fifth. No test sees that. The check `m33s 6select sumloop 26`
counts the shortest loop with nine byte loads in the library at opt-level "s":

| form | loop instructions | check |
|---|---|---|
| 725c433 | 24 | passes |
| rows by value | 29 | fires |
| `Rows::row` inlined | 29 | fires |

What it misses:

- Up to two instructions: each costs about 1.1 M, so up to about 1.2% of the
  small slot passes under the ceiling.
- Anything outside that loop: the rest of `select`, the sync's other loops, and
  what the feed, soft bits and SNR do when `Decoder::pass` is compiled otherwise.
- It reads the library alone, not a linked image. It reads opt-level "s" only: in the
  release profile's build no loop of that shape was found, so it could not be counted.
- Slower instructions of the same number; it is a count, not a time.
- Other targets. The A76, x86-64 and the M4 have no such check.

### Other storage that could share (not built; from reading the code)

| what | saving | what it costs |
|---|---|---|
| Subtraction's stack frames in the idle FFT cells, window or levels beyond the 46 080 B | about 7.4 KB of stack (the deepest path), which raises the static ceiling | a change of `subtract`'s internals; the largest stack lever found |
| Candidate buffer sized by `candidate_capacity()` rather than `CANDIDATES` | 800 B host, 1 000 B small; no sharing | the image's and CLI's callers only |
| Candidate scratch in the FFT buffer | 4 872 B | see above |
| The noise histogram in belief propagation's idle storage | 512 B | a re-initialisation of that storage after each decode |
| No input window where the slot is already in memory | 15 360 B, host only | the samples must be sanitised first; pass 1 can no longer be fed in blocks on that path |

### Not measured

- Any run on hardware for the Cortex-M33 or M4: emulated instructions and link
  sizes only, so cycles, wait states and cache effects are unknown.
- M33 slots but s0086 and s0407 for counts. M4 instruction counts and any decoder
  identity on the M4.
- The enum for the candidate scratch: only its fills were probed, not built.
- Bands wider than 3 975 Hz beyond the tests: none is in the identity sets, so the
  separate-scratch path has unit tests and no corpus run.
- x86-64 and A76 timing on real recordings other than s0086 and s0407 (x86-64) and
  crowded synthetic slots (A76). Sparse and empty slots were not timed.
- The saving on any linked application's image; those figures are the check images'.

## Review follow-ups (O21b, 2026-11-02; no code generation change)

Four code commits, then this record: 4a58a5f (comments at `Rows::row` and
`costas_scores` naming the Cortex-M33 dependency), 3c08f8e (the sync scratch's
element type named `ScoreCell`, a crate-internal alias of `f32`), 5147404 (the
summing-loop check's own message, scoping and header), 88fb444
(signalling-NaN patterns in the score round-trip test).

**Sections.** `.text` and `.rodata` of all eight binaries (x86-64 and AArch64
`paired_bench` and `cat-ft8-decode`, M33 check and staged images, small and
host) are byte-identical at 88fb444 and 725c433, one source path, rustc 1.93.0.
So are `.data` and `.bss`, and the M33 images' every section. On x86-64 and
AArch64 `.data.rel.ro` differs in 10 words of the same size: the panic
locations' line numbers, which moved with the added comment lines. Nothing runs
differently, so the M33 identity and counts stand. The x86-64 and A76 binaries
were timed anyway:

- x86-64, `slotcost`, core 5, alternated, four samples each, checked for other
  jobs around each, none void: small 95 865 477 829 → 96 029 175 967 cycles (+0.17%; spreads 0.6%, 1.5%); host, one pass, 329 209 873 703 → 328 922 989 767 (−0.09%; 0.5%, 0.5%).
  Instructions equal to within 0.01%.
- A76: the same `slotcost` runs as O21 (host three passes and one pass,
  small as A and B), alternated, four runs each; 16 runs, 1 353 samples during
  them, all 0x0 at start, during and end, 2 400 MHz throughout, peak 53.8 °C.
  Every stage agrees within 0.01 ms, and every slot within the 0.1 ms print
  resolution. The small A feed reads 6.41 ms in both, as at 499b196: the
  changed binary's A level repeats; the base was not timed in that session.
  (O21c: it follows the buffers' addresses, not the code.)

**Wide bands stay.** A review proposed limiting `Band::from_hz` to 1 280 base
bins and removing the separate scratch. Declined: it would refuse band widths of
4 000 to 5 950 Hz that the decoder now accepts.

**Corrections to O21** (made in its text above):

- **The second x86-64 `slotcost small` run was not checked for other jobs.**
  Rerun with the same check around each sample, alternated, six samples each,
  none void: 96 271 811 428 → 95 865 852 551 cycles (−0.42%; spreads 0.8%,
  0.9%; every stage equal within its spread). The +0.60% did not recur. The
  row is replaced.
- **The A76 small figures were one of two.** `slotcost small small` times the
  same configuration as A and B, interleaved, and O21 gave only B. A shows the
  feed +1.0% (6.350 → 6.415 ms) and the slot 12.80 → 12.90 ms in all four runs;
  B +0.24%. Both are now given. (O21c: it follows the buffers' addresses.)
- **Exit statuses.** O21 said each forced WAV and M33 comparison exits 1. The
  figures printed then were a pipeline's, not the comparators'. Rerun, the
  comparators' own: unforced 0; a decode digit, `peak_bytes` off by one,
  `sync_scratch_bytes` 4, another memory figure off by one, and a `--profile`
  line left as before → 1 each; one M33 dump bit → 1. The records comparison
  used then printed its verdict but always exited 0; a copy that exits 1 on a
  difference gives 0 unforced and 1 on a forced field.
- **The WAV comparison accepted a `--profile` line that did not change.** Only the
  count of 2 400 showed the change. It now fails such a line; rerun on the x86-64
  and A76 outputs, both still pass, with the 2 400 lines.
- The M33 image scripts kept the cfg only when their flags variable was unset, not
  when it was set and empty; they now keep it in both cases and print the flags
  into their logs. This round's images were built with the cfg (`vmaxnm` present,
  no `fmaxf` calls).

**The check (5147404).** A function with no loop of nine byte loads now fails
with "no loop with nine byte loads (moved or unrolled?)" rather than a count of
9999. A function ends at its `.Lfunc_end` label or at the next global label;
before, only at the next mangled one. Run on the committed form at opt-level "s"
(24, passes), on both regressed forms (29, fails), and at the release profile's
opt-level, where there is no such loop (fails with that message).

**Tests.** The round trip now includes the signalling-NaN patterns 0x7F800001,
0x7FBFFFFF, 0xFF800001 and 0xFFBFFFFF; no score can take them (a score's magnitude is at most 74 970: 7 × 255 a Costas symbol, 21 symbols, × 2),
so the test covers storage, not the decoder. Local CI passes at 88fb444
(fmt, clippy, the tests with the ignored ones (213), the group-one step (205 run, 8 ignored), the bulk run, MSRV 1.75, docs, `check.sh --stack`, and the code-generation check with the summing loop at 24). A76 release library
tests with the ignored ones, cross-built at 88fb444: 212 pass, 0 fail, the
source-tree test skipped; 48 throttle samples, start and end 0x0, none non-zero.

**Not done this round:** splitting 499b196 into its refactor and its change, and
the `as_flattened_mut` form of the transform's buffer, which needs MSRV 1.80.

### Not measured

- As in O21. The A76 small feed's +0.24–1.0% was unexplained here (O21c: placement); no profile of the
  front end on the A76 was taken.

## Closure follow-ups (O21c, 2026-11-02; no code generation change)

Two comment commits, 33a7be2 and fba30f1: the Cortex-M33 dependency comments
at `Rows::row` and `costas_scores` now say what must hold ("must stay out of
line", "must stay a reference", +3.4%). `.text`, `.rodata`, `.data.rel.ro`,
`.data` and `.bss` of all eight binaries are byte-identical at fba30f1 and
88fb444 (one source path, rustc 1.93.0). Local CI passes at fba30f1.

### The A76 small feed's rise follows placement, not the code

O21 and O21b saw the small configuration's feed 0.24–1.0% slower at 499b196
than at e4cfa58 on the A76, more on `slotcost`'s side A than on side B. A probe
in one Raspberry Pi 5 session, one benchmark at a time on core 3, four cycles,
order reversed on even cycles, 24 runs, 1 008 throttle samples, all 0x0 at
start, during and end, 2 400 MHz during every run, peak 53.8 °C:

- Out-of-tree measurement builds of e4cfa58 and b6b73bc (`paired_bench` with
  logging added, never committed): each decode prints the address mod 4096 of
  its `FrontEnd` (whose FFT work area lies at a fixed offset in it) and of its
  level buffer, and its feed time. An environment setting holds a 2 048-byte
  allocation while side A decodes, which moves side A's buffers. Those builds'
  code is laid out a little differently from the timed ones.
- Beside them, the unmodified builds of e4cfa58 and 88fb444 (the same code as
  the head).

| build (feed, ms, median of 4 runs) | side A | side B |
|---|---|---|
| e4cfa58, unmodified | 6.350 | 6.360 |
| 88fb444, unmodified | **6.405** | 6.370 |
| e4cfa58, probe | 6.350 | 6.350 |
| b6b73bc, probe | **6.400** | **6.390** |
| e4cfa58, probe, side A moved 2 048 B | 6.350 | 6.350 |
| b6b73bc, probe, side A moved 2 048 B | 6.350 | 6.350 |

- **It reproduces against the base in the same session** (unmodified: side A
  +0.87%, side B +0.16%).
- **It follows the address, not the order or the code.** Each decode's feed
  takes one of two levels: about 6.348 ms or 6.39–6.41 ms. The fast level is the
  same in both builds (median over addresses 6.348 ms at e4cfa58, 6.347 ms at
  b6b73bc), and the base also runs slow where its buffers land at some addresses
  (its `FrontEnd` at 2 288, 304 or 176 mod 4 096: 6.39–6.41 ms, at least 1 280 of
  its 16 000 decodes). The head's buffers landed at slow addresses more often (at
  least 6 676 of its 16 000), with a different heap history behind them. Moving side A's buffers by
  2 KB puts both sides at 6.35 ms in both builds. Sides A and B alternate their
  order every run, so order is not it.
- **What about the address makes it slow is not known.** No single residue mod
  4 096 of the `FrontEnd`, of the levels, or of their difference separates slow
  from fast in both builds, so this is not shown to be one 4 KB aliasing pair; the
  other buffers' addresses (candidates, candidate scratch) were not logged.
- So the 0.24–1.0% is not a cost of the change: an application gets 6.35 or 6.40
  ms from either build, depending on where its buffers land.

A first attempt at this session was void: the probe's own lines began as the
throttle samples do, and the run's check counted them; all its real samples
read 0x0. The prefix was changed and the session rerun from the start.

### Not measured

- Which address relation makes the feed slow; the other buffers' addresses.
- Whether a Cortex-A76 application's statics (fixed addresses) land in the slow
  or the fast class.

## The subtraction, twenty-second round (O22, bit-identical, 2026-11-02)

Base: 1744f1c (the code of O21c). The target is the signal subtraction: 48.8% of
the A76's crowded three-pass host slot (43.7 ms of 89.65 ms) and 52.7% of the
Cortex-M33 host slot in prof19. It was profiled first, at the instruction level,
on all three targets. Four code commits, one optimisation each: 564511b (the
fit's whole lanes one lane at a time without a vector unit), d24eddc (the
frequency correction one sample at a time without a vector unit), c4ac43e (the
time search's lane sums one lane at a time without a vector unit) and 9d05162
(the time search's skipped spans by tone pair). Then ec97574, a code-generation
check for the first three. Decodes, records, residuals and encoder output are
identical; the WAV output differs from b93516e's only in O21's `--profile`
memory figures, as at O21.

| target | measure | 1744f1c | 9d05162 | change |
|---|---|---|---|---|
| Cortex-M33, s0086 host | subtraction, emulated instructions a call | 32.198 M | 24.343 M | **−24.40%** |
| | host slot (setup excluded) | 1 953.61 M | 1 662.98 M | **−14.88%** |
| Cortex-M33, s0407 host | subtraction a call | 32.137 M | 24.294 M | −24.41% |
| | host slot | 1 949.66 M | 1 659.46 M | −14.88% |
| Cortex-A76, crowded host slot | subtraction (medians of 4) | 43.720 ms | 42.635 ms | **−2.48%** |
| | slot | 89.60 ms | 88.50 ms | −1.2% |
| x86-64, `slotcost host host,passes=1 200 5` | user cycles, the whole program (medians of 4) | 327.52 G | 323.28 G | −1.29% |

Most of the M33's gain is code generation at opt-level "s": its lane loops
are not unrolled, so the four lanes' sums and the correction's 64-bit lane
states lived in memory. The A76's subtraction is vector-bound within
1.2–1.3× of its floor in its largest parts; the one gain found for it (and for
x86-64) removes repeated work (the skipped spans).

### Measured first

**Cortex-A76.** O18's microbenchmark (O19's grouped form, appended to a scratch
copy of `subtract/tests.rs`, `unsafe` allowed in that copy for the counters and
a PC sampler; not committed), at 1744f1c, on core 3 after the 5 s sampled busy
loop, three rounds (they agree within 0.1%; all runs valid at 0x0). The
workload is O13's crowded synthesized slot (24 signals, −18 to +6 dB, decodes a
little off the truth), one time-search step per subtraction. Per waveform
sample (151 680 a subtraction); the time search's rows per sample of the
symbols' cores (75 840 a step). Integer operations are the A76's `DP_SPEC`,
vector operations `ASE_SPEC`.

| part (A76, 1744f1c) | cycles | integer ops | vector ops | loads | stores | instructions |
|---|---|---|---|---|---|---|
| **the subtraction** | **19.87** | 12.4 | 23.5 | 4.03 | 2.42 | 45.1 |
| time search, per core sample: generator (fill the cores, skip the rest) | 4.12 | 3.5 | 3.7 | 1.25 | 0.70 | 10.2 |
| … + the products (polynomial, clamp, store) | 9.85 | 3.8 | 13.0 | 1.94 | 1.15 | 21.1 |
| … + the lane sums (`symbol_correlations`) | 10.56 | 4.8 | 13.2 | 1.89 | 1.07 | 22.5 |
| the generator skipping the whole waveform (`skip_with`) | 1.75 | 1.2 | 1.8 | 0.53 | 0.19 | 4.2 |
| step sums and alignment, per core sample | 1.36 | 1.1 | 1.2 | 0.40 | 0.12 | 3.4 |
| fit path: generator | 2.42 | 2.6 | 1.7 | 0.82 | 0.61 | 6.4 |
| … + the frequency correction | 4.20 | 7.4 | 1.7 | 1.33 | 1.11 | 12.5 |
| … + the reference | 8.76 | 7.9 | 8.7 | 1.56 | 1.36 | 20.5 |
| … + the fit (fused) | 11.35 | 8.8 | 12.4 | 2.25 | 1.48 | 26.3 |
| the fit loop alone | 2.47 | 0.9 | 3.8 | 0.57 | 0.06 | 5.6 |

L1D refills 0.074 a sample, last-level misses 0.001, branch misses 0.003. The
parts of the subtraction, per waveform sample with one time-search step:

| part | cycles | share | what bounds it |
|---|---|---|---|
| reference (polynomial, store) | 4.56 | 23% | vector issue: the block of 16 is 127 instructions, 113 vector |
| time search's products | 2.86 | 14% | vector issue: the same polynomial, the clamp and a weight |
| fit (fused) | 2.60 | 13% | vector issue: 20 instructions per four samples, 15 vector |
| removal and the rest (by difference) | 2.55 | 13% | vector issue: 20 instructions per four samples, 15 vector |
| fit path's generator | 2.42 | 12% | the overlap runs' increments and grouped running sum |
| time search's generator | 2.06 | 10% | of which the skipped spans about 0.87 |
| frequency correction | 1.78 | 9% | **integer issue: 24 instructions per four samples, 19 integer, none vector** |
| step sums and alignment | 0.68 | 3% | |
| time search's lane sums | 0.36 | 2% | |

PC samples of the whole subtraction (28 899, 23 µs apart): the fit path's
`blocks` 32.3% (the reference's block of 16 21.1%, the correction's loop
8.8%), `symbol_correlations` 16.0% (the products' block of 16 15.1%),
`fill_phase_only` 15.2%, `Fit4::add_all` 12.2%, `Removal::refs` 12.0%,
`skip_with` 5.2%.

- **Not memory and not branches.** The working set is in L1.
- **Vector issue.** 23.5 vector operations a sample on two pipes is a floor of
  11.8 cycles against 19.87 (1.69×, as in O19). The polynomial passes, the
  fit and the removal are each within 1.2–1.3× their vector floor.
- **The input clamp** is 5 of the 15 vector operations per four samples in the
  fit and in the removal (`fmaxnm`, `fminnm`, `fabs`, `cmhi`, `and`), and is
  taken twice for each sample the fit takes, once in the fit and once in the
  removal.
- **Two parts are not vector-bound:** the frequency correction (scalar 64-bit
  lane states) and the time search's skipped spans, whose phase sums are taken
  again in every step for every symbol boundary although they depend only on
  the two symbols' tones (below).

**x86-64** (`perf record -e cycles:u` of the same microbenchmark, the whole
subtraction only, core 5): `blocks` 26.5%, `fill_phase_only` 24.4%,
`symbol_correlations` 12.0%, `skip_with` 8.6%, `Removal::refs` 8.1%,
`Fit4::add_all` 6.7%, `score` 3.4%, the bench's own copy of the slot
(`memmove`) 3.3%, `whole_lane_sums` 1.9%. The generator is a third of the
subtraction on x86-64 (a quarter on the A76): its overlap runs' running sum is
a scalar dependent add per sample there (13% of `fill_phase_only`'s samples on
that one instruction; O15 found the grouped sum neutral on x86-64).

**Cortex-M33** (emulated instructions per function and per basic block, staged
host image, s0086, 37 subtractions, 1 191.31 M; per sample of the 151 680 a
subtraction unless stated):

| function (1744f1c) | M | per sample | the hottest blocks |
|---|---|---|---|
| `subtract::blocks` (the fit path: correction, reference) | 363.09 | 64.7 | the reference, 39 a sample (216.5 M); **the correction in lanes, 23.9 a sample (133.7 M): the lanes' 64-bit phases and steps through memory, a lane loop of four** |
| `Fit4::add_all` | 291.00 | 51.9 | **the four lanes side by side, the lanes' samples, references and sums copied through stack arrays in lane loops not unrolled: 52 a sample** |
| `Removal::refs` | 145.63 | 26.0 | one loop, 26 a sample |
| the subtraction's closure (the time search's products) | 144.46 | 51 a core sample | the polynomial, clamp and weight, one sample at a time |
| `Synth::fill_phase_only` | 129.09 | 23.0 | the overlap runs' increments and running sum |
| `Synth::skip_with` | 53.78 | 19 a skipped sample | the time search's skipped spans |
| `whole_lane_sums` | 43.60 | 15.6 a core sample | **the eight sums in stack arrays, a lane loop not unrolled** |
| `aligned`, `score` | 14.24 | | the alignment's trials |

On the M33 the lane loops that the hosts vectorise are not unrolled at
opt-level "s" (rustc 1.93.0), so state that the hosts keep in vector
registers goes through memory. Taken one lane (or one sample) at a time, the
same operations in the same order per lane keep it in registers.

### Tried, not kept

- **The correction in vector lanes on AArch64.** In a standalone crate with the
  same code (rustc 1.93.0, `aarch64-unknown-linux-gnu`, opt-level 3), the
  compiler merges the four lanes' steps (each `+16·accel` a group) into one
  induction variable and the lanes stay scalar. The shipped form, the lanes'
  updates in separate loops, two pairs of lanes, and 32-bit halves with
  explicit carries all compiled to scalar loops (17–39 instructions per four
  samples), none vectorised. No form without `unsafe` intrinsics was found.
- **The correction a chunk ahead of the reference**, so that its integer work
  overlaps the vector polynomial (microbenchmark, fit-path generator, correction
  and reference, cycles a sample; all runs valid at 0x0): unchanged 9.00,
  chunks of 16 9.98, of 48 8.96, of 80 8.82, of 240 (the whole block first, no
  overlap) 8.70. No overlap was gained. The 240 case's 0.3 cycles is within the
  0.24 cycles the unchanged form moved between two test binaries (8.76 in the
  first), so it was not pursued.
- **A shorter finite test in the clamp.** With rustc 1.93.0 on AArch64,
  `x.abs() < f32::INFINITY` gives `fabs` and `fcmgt`, and the select form
  `bic`, `cmgt` and `and`: no form tried had fewer than the shipped three
  operations, and `facgt` was not generated.

### Changes

1. **564511b: the fit's whole lanes one lane at a time without a vector unit.**
   Where a group is one sample (`GROUP == 1`: the Cortex-M33 and the tests
   built with `--cfg cat_ft8_test_group_one`), `Fit4::add_all` takes the whole
   lanes one lane at a time, the lane's five sums in locals over the block. Each
   lane still takes its samples in order and the lanes meet only in `sum`, so
   the sums are the same bit for bit. M33 (opt-level "s"): its loop is 20
   instructions a sample, and the function 51.9 → 20.8 a sample. The hosts'
   `.text` and `.rodata` are byte-identical to 1744f1c's.
2. **d24eddc: the frequency correction one sample at a time without a vector
   unit.** `Correction::apply` is `apply_as::<VECTOR>`: four lanes where the
   target has a vector unit, `Correction::next` sample by sample elsewhere.
   Both give exactly `next`'s sequence (integers modulo 2⁶⁴); the correction
   test now checks each form. M33: the correction 23.9 → about 11 instructions
   a sample. **The hosts' code changed** in `subtract::run` only (AArch64
   2 532 → 2 528 instructions, register choices; x86-64 the same size), so
   it was timed (below: no change).
3. **c4ac43e: the time search's lane sums one lane at a time without a vector
   unit.** `whole_lane_sums` likewise for `GROUP == 1`: 9 instructions a core
   sample in its loop at opt-level "s" against 14. Hosts byte-identical to
   d24eddc's.
4. **9d05162: the time search's skipped spans by tone pair.**
   - Between one symbol's core (samples 480–1 440) and the next one's, the time
     search needs only the generator's phase. Those 960 samples lie between
     sample `PULSE_FIRST` (1 215) of symbol k and `PREV_END` (706) of symbol
     k + 1, where only the two symbols' pulses are non-zero, so the increments,
     and their sum modulo 2³², depend only on the two tones and the base
     frequency (`TRANSITION` in the waveform module states the bound; a `const`
     assertion ties the cores to it).
   - `symbol_correlations` now runs the generator over the cores block by
     block and over each skipped span only when its pair of tones is first
     met; after that, and in every later step of the time search, it adds the
     stored sum (`Skips`: 64 sums and a bit mask, 264 B of stack, one per
     subtraction). Integer addition modulo 2³² is associative, so the phases
     are the same. Nothing is generated after the last core.
   - A message has 78 boundaries and at most 64 pairs (the three Costas
     arrays share their six); a step generates the span of each pair not yet
     met, so a later step only those of pairs not yet met (one at a start
     moved earlier can reach boundaries the first did not). (Corrected in
     O22b: this said "later steps none".)
   - `Synth` gains `tone`, `skip_by_sum` and `advance` (crate-internal).
     `SUBTRACT_STACK_BYTES` 4 656 → 4 920 (ADR 0024 updated).
   - All targets: the hosts' code changed and was timed.
5. **ec97574: the code-generation check** (below).

### Cortex-M33 (emulated instructions; staged images as built, rustc 1.93.0, `--cfg cat_ft8_minmaxnm`)

| commit | s0086 host slot | subtraction a call (37 calls) | s0407 host slot | subtraction a call | function that fell |
|---|---|---|---|---|---|
| 1744f1c | 1 953 607 521 | 32 197 641 | 1 949 658 625 | 32 137 275 | |
| 564511b | 1 779 149 881 (−8.93%) | 27 482 569 (−14.64%) | 1 775 592 834 (−8.93%) | 27 432 794 (−14.64%) | `Fit4::add_all` 291.00 → 116.55 M |
| d24eddc | 1 706 840 741 (−4.06%) | 25 528 268 (−7.11%) | 1 703 445 470 (−4.06%) | 25 482 866 (−7.11%) | `blocks` 363.09 → 292.13 M |
| c4ac43e | 1 689 158 335 (−1.04%) | 25 050 365 (−1.87%) | 1 685 804 512 (−1.04%) | 25 006 083 (−1.87%) | `whole_lane_sums` 43.60 → 25.92 M |
| **9d05162** | **1 662 979 755 (−1.55%; −14.88% in all)** | **24 342 836 (−2.82%; −24.40% in all)** | **1 659 459 980 (−14.88% in all)** | **24 294 069 (−24.41% in all)** | `skip_with` 53.78 → 27.76 M |

- Same decodes (40 and 38 host; 23 and 30 small). Every other stage's count is
  the same to the instruction (feed, sync, refine, soft bits, belief
  propagation, OSD-1, SNR). Per decode, host s0086: 48.8 → 41.6 M.
- The small slots (no subtraction) move by +25 and +32 instructions (s0086,
  s0407), in `decode_a_slot`.
- The subtraction is now 54.2% of the s0086 host slot (61.0% before; prof19's
  52.7% was before O20 and O21 cut the feed).
- After the round (s0086 host, M): `blocks` 292.13 (the reference 216.5, the
  correction about 62), the time search's closure 144.29, `Removal::refs`
  145.63, `fill_phase_only` 129.09, `Fit4::add_all` 116.55, `skip_with` 27.76,
  `whole_lane_sums` 25.92.

### Cortex-A76 (Raspberry Pi 5; `slotcost host host,passes=1 200 5`, medians of 4 alternated runs)

Aligned builds (`-align-loops=64 -align-all-functions=6`) of 1744f1c, c4ac43e
(the code of d24eddc on the hosts) and 9d05162 from one scratch path, core 3,
the throttle, clock and temperature sampled once a second from core 0, a 5 s
sampled busy loop on core 3 before each run, alternated (forward, then
reversed), four cycles.

| ms per slot (median, spread) | 1744f1c | c4ac43e | 9d05162 |
|---|---|---|---|
| **subtract** | 43.720 (0.020) | 43.710 (0.020) | **42.635 (0.070): −1.085, −2.48%** |
| pass 2 / pass 3 subtract | 30.455 / 13.260 | 30.445 / 13.260 | 29.695 / 12.935 |
| slot | 89.60 | 89.55 | **88.50** |
| worst slot | 95.6 | 95.6 | 94.2 |
| feed (placement check) | 19.210 (0.030) | 19.205 | 19.205 |
| belief propagation (placement check) | 19.055 | 19.050 | 19.050 |
| sync / soft bits (placement check) | 4.680 / 2.380 | 4.680 / 2.380 | 4.680 / 2.380 |
| one pass: slot, feed | 18.30, 6.460 | 18.30, 6.460 | 18.30, 6.455 |

- **Placement check.** The untouched stages agree within 0.005 ms across the
  three builds (feed, belief propagation, sync, soft bits, OSD-1, SNR, refine;
  and the one-pass slot). 9d05162's gain (1.085 ms) is over 200 times that and
  15 times its own stage's spread, so no placement variation was needed.
- **d24eddc on the A76:** −0.010 ms, within the stage's spread (0.020): its
  host change (`run`'s register choices) costs or gains nothing measurable.
- **9d05162 on the A76:** −2.48% of the subtraction; the microbenchmark (two
  runs of each build, alternated, three rounds each, all valid) gave 19.865 →
  19.376 cycles a sample (−2.5%), the time search's correlations 10.56 → 9.56
  cycles a core sample (its first step: one `Skips` per subtraction).
- By pass: 18.29 / 43.78 / 27.49 → 18.28 / 43.02 / 27.16 ms; per new decode
  0.76 / 4.18 / 9.13 → 0.76 / 4.10 / 9.02 ms (the decode counts are
  unchanged).
- **Validity.** All 12 timed runs are valid: 1 520 samples during the runs and
  60 in the warm-ups, every one 0x0, start and end checks included; every
  sample during a run at 2 400 MHz; one warm-up sample at 1.5 GHz (the first
  of the first warm-up). Peak 55.4 °C. The program's own output is logged
  with a prefix that no throttle sample line has, so none can be counted as
  one (O21c's void session).

### x86-64 (Threadripper 3970X, core 5, `perf stat` user counters over the whole `slotcost` program)

Aligned builds of the same three commits from the same scratch path, four
alternated rounds (forward, then reversed), `slotcost host host,passes=1 200
5` (the three-pass and one-pass configurations both); a check for other jobs
before and after each sample, none void. Cycles are the measure (the virtual
machine's clock biases its stage times, given for scale only).

| build | cycles (median, range) | instructions | subtract stage (ms) |
|---|---|---|---|
| 1744f1c | 327.52 G (327.11–328.71) | 925.27 G | 25.975 |
| c4ac43e | 327.58 G (327.32–328.17), +0.02% | 925.27 G | 25.975 |
| 9d05162 | **323.28 G (322.11–324.08), −1.29%** | 914.21 G (−1.20%) | 24.875 (−4.2%) |

- d24eddc's host change: within the base's range; instructions equal to
  0.001%.
- 9d05162's range clears the base's by 3.0 G.
- In the microbenchmark's profile (`perf record`, one run each), `skip_with`
  falls from 8.1% to 4.7% of the subtraction's samples.


### Sizes and stack (`check.sh --stack`, thumbv8m)

| image | flash 1744f1c → 9d05162 (B) | static RAM (B) |
|---|---|---|
| small, small with OSD on | 92 082 (unchanged) | 201 668 |
| host, host with OSD off | 104 318 → **103 530 (−788)** | 230 084 |
| encoder only | 35 286 (unchanged) | 0 |

- Frames: host `subtract_one` 4 808 → 4 920 B (+112), the subtraction's
  closure 2 864 → 2 536 B (−328); `_start` 4 104,
  `decode_a_slot` 2 520, `finish_row` 464, `osd_decode` 272 B unchanged. The
  small images' frames are unchanged.
- Paint high water of the staged images: host 10 308 → 10 260 B (s0086 and
  s0407). **Small 10 200 → 10 320 B (+120)**, in a slot that runs no
  subtraction: the staged small image inlines the subtraction's driver into
  `decode_a_slot` (it is linked for the passes setting), whose frame there is
  5 072 → 5 192 B (5 072 → 4 920 at d24eddc, then +272 with `Skips` at
  9d05162). The check images' `decode_a_slot` is 2 520 B in both.
- No fused multiply-add (0 `vfma`/`vfms`/`vfnma`/`vfnms`; `vmla` 128 / 211 /
  17, unchanged) and no `f64` helper in any image.
- Cortex-M4F (thumbv7em, embedded-check images, no cfg): small flash 92 348
  (unchanged), host 104 812 → 104 004 B; static RAM unchanged; no `f64`
  helper, no fused multiply-add; `fmaxf`/`fminf` calls 10 and 14, unchanged
  (none in the subtraction's guarded functions).
- Hosts' `.text`: AArch64 `paired_bench` 1 169 752 → 1 170 200 B,
  `cat-ft8-decode` 397 224 → 397 672 B; x86-64 1 411 062 → 1 411 766 and
  456 646 → 457 222 B.

### Sections of all eight binaries

`.text` and `.rodata` (one scratch path, fresh tree, rustc 1.93.0; thumbv8m
with `--cfg cat_ft8_minmaxnm`):

| binary | 564511b against 1744f1c | d24eddc against 564511b | c4ac43e against d24eddc | 9d05162 against c4ac43e |
|---|---|---|---|---|
| x86-64, AArch64 `cat-ft8-decode`, `paired_bench` | identical | `.text` differs (`run`) | identical | differs |
| M33 check, small | identical | identical | identical | identical |
| M33 check, host; staged small and host | differ | differ | differ | differ |

ec97574 changes only `codegen-check.sh`.

### The code-generation check (ec97574)

The M33's gains rest on code generation that no test sees: at opt-level "s"
the one-lane loops must keep their sums in registers, and the correction must
not go back to lanes. Two metrics are added: `fploopN` (the instructions of the
shortest loop with at least N `f32` multiply-accumulates or adds, a ceiling;
a function with no such loop fails with its own message) and `carries` (`adc`
instructions, a ceiling). Each forced form below reverts one change on the
M33 only (one log each, the check's own exit status):

| form | `Fit4::add_all` loop of 5 (ceiling 22) | `whole_lane_sums` loop of 2 (ceiling 11) | carries in `blocks` (ceiling 24) | exit |
|---|---|---|---|---|
| ec97574 | 20 | 9 | 19 | 0 |
| the fit's lanes side by side | **25** | 9 | 19 | 1 |
| the correction in lanes | 20 | 9 | **37** | 1 |
| the lane sums side by side | 20 | **14** | 19 | 1 |

What it misses: up to two instructions in each loop (two a sample in the fit is
about 11 M of the s0086 host slot); other targets (no A76 or x86-64 bound is
added: 9d05162 does not rest on code generation); the linked image (it reads
the library at opt-level "s"; the linked images' loops are the same 20 and 9
instructions); the correction's form is counted by its carries, not timed.

### Identity

All at 9d05162's binaries (whose code ec97574 does not change).

- **Records, x86-64:** the 73 800 `paired_bench run` records equal b93516e's
  (host, one pass, small: 24 600 each, 0 differ; the records comparison exits 0
  each).
- **800 WAV slots** × 3 configurations × 3 modes, x86-64 (7 200 runs, 78 690
  decode lines): 93 091 lines byte-identical to b93516e's, and the 2 400
  `--profile` lines differ only by the sync scratch, as at O21 (the WAV
  comparison exits 0).
- **A76:** the 73 800 records of 9d05162's AArch64 `paired_bench` equal x86-64
  b93516e's (0 differ each; exit 0), and the 800 WAV slots × 3 × 3
  with its AArch64 `cat-ft8-decode` give the same 93 091 identical lines and
  2 400 `--profile` lines (exit 0). One benchmark at a time, in
  fresh directories; 319 throttle samples (start and end checks included),
  all 0x0.
- **Encoder:** the 45 `cat-ft8-encode` WAV files equal 70be604's.
- **Cortex-M33 (emulated):** the small and host dump images on seven slots
  (14 of 14) and the multi-pass residuals of s0086 and s0407 equal 68634cc's
  (the M33 comparison exits 0).
- **Comparators on a forced difference** (copies; each line its own exit
  status): the records comparison 0 unforced, 1 with one record's field
  changed; the WAV comparison 0 unforced, 1 with a decode line's frequency one
  higher, 1 with a `--profile` line's `peak_bytes` one higher; the M33
  comparison 0 unforced, 1 with
  one bit of one dump flipped (15 identical, 1 differ); `cmp` of an encoder WAV
  0 unforced, 1 with one byte changed.

### Tests and mutants

New: `skipped_spans_depend_only_on_the_two_tones` (random messages and bases
with every pair of tones, from a de Bruijn order of the eight tones: each
span's sum equals the first with the same pair, and `advance` by it leaves the
generator where `skip_by_sum` does, the next core's phases included).
`time_search_lanes_follow_the_sample_index_at_any_start` now shares one
`Skips` across its 20 calls, as the steps do; the correction test checks both
forms of `apply`.

Mutants in copies of ec97574's tree, release library tests, failing tests per
mutant (one log each, counts read from the logs; the one-lane code runs on a
host only with `--cfg cat_ft8_test_group_one`):

| mutant | default (206) | group of one (206) |
|---|---|---|
| none (control) | 0 | 0 |
| the fit's lane loop reads lane 0's reference | 0 | 11 |
| the fit's `ss` takes `s · c` | 0 | 14 |
| the one-at-a-time correction one off | 1 | |
| the lane sums' sine part takes the cosine | 0 | 11 |
| a span's pair ignores the next tone | 11 | |
| `advance` by 0 | 9 | |
| `skip_by_sum` returns the phase, not the sum | 10 | |
| `advance` leaves the position | 10 | |
| a span's sum one block short (the block skipped after) | 9 | |
| never reusing a stored sum (equivalent: the cache is not observable) | 0 | |

The last is equivalent by design: the stored sums change only the work, not a
result. The new test catches the `skip_by_sum` and `advance` mutants; the
shared-`Skips` test catches the pair and span mutants.

Local CI passes at ec97574: fmt; clippy `-D warnings` (workspace, all targets
and features, thumbv8m); workspace, profile, no-std and release tests with the
ignored ones (214); the group-one step (206 run, 8 ignored); the bulk run;
MSRV 1.75; docs with and without `std`; `check.sh --stack`; the
code-generation check (the new bounds 20, 9 and 19). A76
release library tests with the ignored ones, cross-built at ec97574 (`cat-ft8`
tree 7085cf14…): 213 pass, 0 fail, the source-tree test skipped; 50 throttle
samples, start and end included, all 0x0.

### What bounds the subtraction now

- **Cortex-M33** (per sample of a subtraction): the reference's polynomial 39
  instructions, of which about 16 are the quarter's selects, done in integer
  registers, and the moves there (`vmov`); the time search's
  products 51 a core sample; the removal 26; the generator 23; the fit 20 a
  sample in its loop, of which 4 are `ldr` and `vmov` pairs for the reference
  where one `vldr` each would do (the reference is read as bytes).
- **Cortex-A76:** vector issue in the polynomial passes, the fit and the
  removal (1.2–1.3× their floors), the scalar correction (1.78 cycles a
  sample), and the generator.

### Next candidates (not built)

| candidate | estimate | cost |
|---|---|---|
| **The clamp once.** Hold the clamped sample the fit takes beside the reference in the ring, so that the fused removal reads it instead of clamping again. Bit-identical: the removal runs behind the fit on the same, unchanged samples. | A76: 5 of the removal's 15 vector operations per four samples less one store in the fit, up to about 0.6 cycles a sample (−3% of the subtraction); M33: about 6 of the removal's 26 instructions a sample less a load and a store, −2 to −3% | the ring grows by a third (`SUBTRACT_SCRATCH_BYTES` 46 080 → 69 120 B; the decoder lends its 179 584 B levels); the layout's tests; the unfused path keeps its clamp |
| **The M33 reference's quarter selects** (in integer registers now) | up to about 16 of 39 instructions a sample in the reference, as many in the products | form not found; needs the same values bit for bit, the sign of zero included |
| **The second time-search step's cores** (about 0.2 steps a subtraction in crowded slots) | the generator and polynomial again for the same phases | the cores' phases are 303 KB; no storage for them |

### Risks

- (Fixed in O22b.) `Synth::skip_by_sum` and `Synth::advance` check their sample counts only in
  debug builds: asked past the waveform's end, the first would loop for ever
  and the second would leave the position past the end. Their two callers ask
  for fixed spans inside the waveform (up to the first core, and between the
  cores of symbols 0 to 78). A clamp to `remaining()`, as `Synth::skip` has,
  changes the hosts' code, so it was not added untimed.
- The M33's gains depend on code generation at opt-level "s" (now checked,
  above); a toolchain change can lose them with every test passing.
- The small staged image's stack high water rose 120 B (above).

### Not measured

- Cortex-M33 cycles (emulated instructions only) and any M33 slot but s0086
  and s0407 for counts (seven slots for identity); any M4 count or identity.
- Sparse and empty slots on any target (the A76 timing is the crowded `main`
  block; the microbenchmark is one synthesized crowded slot), and how the
  skipped spans' gain varies with the number of time-search steps.
- The microbenchmark on other A76 cores; inside-stage figures after the round
  other than the microbenchmark's whole subtraction and time search.
- The forms in "Tried, not kept" in the decoder (microbenchmark and a
  standalone crate only).
- After a toolchain change: the M33 gains rest on the one-lane loops keeping
  their sums in registers at opt-level "s" (checked, above); the A76 and x86-64
  gains do not rest on code generation.
- x86-64 stage times (the virtual machine's clock); x86-64 timing on real
  recordings. The x86-64 microbenchmark profile is from one run of each build.

## Review follow-ups (O22b, 2026-11-03; two commits change code generation, by less than the noise)

On top of O22 (e7d202b), one concern per commit: 7015c8e, 7e4c251, 10dada1,
6cf2efd, 7f1fb3e and e232116, then this record.

- **7015c8e: `Synth::skip_by_sum` is `skip_summed`.** All eight binaries'
  `.text` and `.rodata` are byte-identical to 9d05162's.
- **7e4c251: the generator's skips stop at the waveform's end.** Asked past
  the end, or with no working storage, `skip_summed` looped for ever in
  release builds and `advance` moved the position past the end, so that
  `remaining` wrapped (O22's risk). `skip_summed` now stops when `skip_with`
  makes no progress, and `advance` moves at most to the end; the debug
  assertions on the counts are gone, since the bound now holds in every
  build. `Synth::skip` already clamped its count to `remaining`, so its loop
  could not hang (unchanged). A new test, `skips_stop_at_the_end_of_the_waveform`,
  asks all three for `usize::MAX` samples and `skip_summed` for 10 with no
  storage. A clamp of `n` to `remaining` in `skip_summed` as well was not
  needed: the progress check covers both cases.
- **10dada1: the core walk checks its position and stops like `blocks`.**
  `debug_assert_eq!(j0, s.position())` before each core block, and a return
  when the generator makes no progress (never, within the waveform).
- **6cf2efd: a `Skips` serves one generator, checked in debug builds.** The
  `CoreWalk<'a> { synth: &'a Synth, .. }` shape was built first (the
  correlations a method of the walk, which holds the generator). It changed
  the hosts' code: AArch64 `paired_bench` `.text` 1 170 200 → 1 170 328 B (the
  correlations 919 → 935 instructions, `run` 2 547 → 2 566), x86-64 likewise,
  and both M33 host images; so it was not kept. Instead, debug builds keep the
  first generator a `Skips` serves and panic (`skipped_spans_serve_one_generator`)
  if it is used with another of other tones or base frequency
  (`Synth::same_phases`, debug-only); a comment at `run`'s `Skips` says so.
  `SUBTRACT_STACK_BYTES` counts the sums and mask (`SKIPS_BYTES`, 264 B),
  checked against the release layout at compile time. Sections byte-identical
  to 10dada1's.
- **7f1fb3e: comments.** The one-lane loops' comments say "a group of one"
  and name its two cases (targets without a vector unit; the group-of-one
  tests); `apply_as` says why the correction is chosen by `VECTOR`, not
  `GROUP`: it does not touch the reference's layout, so the group-of-one tests
  keep the lanes there and the tests call both forms directly.
- **e232116: the code-generation check fails if the correction leaves
  `blocks`.** O22's carry-add ceiling passed with `Correction::apply` out of
  line (15 carry adds stay in `blocks`, from the closed-form skips). A new
  metric, `absent`, passes only if no function's label holds the name; the
  M33 check requires that no method of `Correction` is out of line. The
  header's `sumloop` sentence, which O22 had split from its metric, is back
  beside it. (One log each, the check's own exit status:)

| form | carry adds in `blocks` (ceiling 24) | `Correction` out of line | exit |
|---|---|---|---|
| e232116 | 19 | none | 0 |
| `Correction::apply` out of line | 15 | **found** | 1 |
| the correction in lanes | **37** | none | 1 |

- **Wording corrected in O22:** a later step of the time search generates
  the spans of pairs not yet met, not none (a step at a start moved earlier
  can reach boundaries the first did not).

### Code, counts and time

`.text` and `.rodata` (one scratch path, rustc 1.93.0): 7015c8e equals
9d05162; 7e4c251 changes the hosts' code in `symbol_correlations` only
(AArch64 921 → 922 instructions, x86-64 1 022 → 1 026, the same sizes) and
the M33 host images (+12 B flash, 103 530 → 103 542 B); 10dada1 changes
`symbol_correlations` again (AArch64 922 → 919; M33 flash the same); 6cf2efd,
7f1fb3e and e232116 equal 10dada1. At 10dada1 against 9d05162: static RAM and
every `check.sh --stack` frame unchanged; the M4 host image 104 004 → 104 024
B; no fused multiply-add and no `f64` helper; the M4's
`fmaxf`/`fminf` calls unchanged.

| | 9d05162 | 7e4c251 | 10dada1 |
|---|---|---|---|
| M33 s0086 host, subtraction a call | 24 342 836 | 24 343 185 (+349) | 24 341 878 (−1 307) |
| M33 s0407 host, subtraction a call | 24 294 069 | 24 294 419 (+350) | 24 293 114 (−1 305) |
| M33 small slots, s0086 / s0407 | 183 016 804 / 175 802 920 | the same | the same |
| A76 subtract, ms (median of 4, spread) | 42.610 (0.040) | 42.600 (0.060) | 42.465 (0.040) |
| A76 slot, ms | 88.40 | 88.45 | 88.30 |
| x86-64 `slotcost` user cycles (median of 4, range) | 323.51 G (323.04–324.33) | 323.30 G (322.73–324.16), −0.07% | 323.16 G (323.05–323.98), −0.11% |

- **The bound costs nothing measurable:** M33 +349 instructions a
  subtraction (+0.0014%; the progress test is in the skips' loop, run once a
  span); A76 −0.010 ms, within the spread; x86-64 −0.07%, within the ranges.
- **10dada1 reads 0.135 ms faster on the A76** (3 times its spread). Its
  change is a compare and an assertion compiled out; the code of the
  correlations moved (three instructions fewer). It is not claimed as a gain:
  placement can move a stage this much (O21c), and no placement variation was
  run.
- A76: the three builds aligned, from one source path, alternated, four
  cycles, core 3; 12 runs, 1 512 samples during the runs and 60 in the
  warm-ups, all 0x0, start and end checks included; every run at 2 400 MHz;
  one warm-up sample at 1.7 GHz (the first). Peak 53.8 °C. The untouched stages
  agree within 0.005 ms (feed, belief propagation, sync, soft bits, SNR,
  OSD-1, the one-pass slot).

### Identity (7e4c251 and 10dada1, the code of the head)

- x86-64: 73 800 records equal b93516e's at each (the records comparison exits
  0); the 800 WAV slots × 3 × 3 give 93 091 identical lines and the 2 400
  `--profile` lines of O21's rule (the WAV comparison exits 0).
- A76: the records of each build's AArch64 `paired_bench` equal x86-64
  b93516e's (73 800, 0 differ, exit 0 each) and the 800 WAV slots × 3 × 3 with
  its `cat-ft8-decode` pass the same rule (exit 0); 318 throttle samples in
  each run, start and end included, all 0x0.
- Encoder: the 45 WAV files equal 70be604's at each.
- Cortex-M33 (emulated): 14 of 14 dumps and the residuals of s0086 and s0407
  equal 68634cc's at each (the M33 comparison exits 0).
- Comparators at 10dada1: unforced 0; a record's field, a decode's frequency,
  a `--profile` line's `peak_bytes`, one dump bit and one encoder byte each
  give 1.

### Tests and mutants

Mutants in copies of e232116's tree (one log each; failing tests read from
the logs; release or debug library tests as the code needs):

| mutant | tests | failing |
|---|---|---|
| none (control) | release / debug | 0 of 207 / 0 of 208 |
| `advance` not clamped | release | 1 (`skips_stop_at_the_end_of_the_waveform`) |
| `skip_summed` without the progress check | release | the test hangs; stopped by a 900 s timeout (exit 124) |
| the owner check always passes | debug | 1 (`skipped_spans_serve_one_generator`) |
| `same_phases` ignores the base frequency | debug | 1 (the same) |
| the walk's position assertion one off | debug | 27 |
| the walk without the progress return | release | 0: equivalent (the generator always makes progress within the waveform) |

Local CI passes at e232116: as in O22 (release tests with the ignored ones
215; the group-one step 207 run, 8 ignored), with the code-generation check's
new `absent` check. A76 release library tests with the ignored ones,
cross-built at e232116 (`cat-ft8` tree 4c103aa3…): 214 pass, 0 fail, the
source-tree test skipped; 50 throttle samples, all 0x0. x86-64 timing: core 5,
alternated, other jobs checked around each sample, none void; instructions
914.20 → 914.21 → 914.34 G.

### Not measured

- As in O22. The A76 gain read at 10dada1 is not separated from placement.
- The `CoreWalk` form was measured only by its sections and instruction counts
  per function, not timed.

## O22c: closure follow-ups (tests and comments only)

- `skipped_spans_serve_one_generator_of_one_tone_sequence`: the owner check
  with the same base frequency and one data tone moved; the earlier test moved
  only the base. A `same_phases` that ignores the tones passes the earlier test
  and fails this one (shown, then reverted).
- `STACK_BYTES`' doc says it holds for release builds; debug builds add the
  owner check's copy of the generator.
- The code-generation check's header says `absent` sees only a type's own
  methods, not a trait's left out of line.
- No code change: x86-64 and AArch64 `cat-ft8-decode` `.text` and `.rodata`
  identical to e30e553's. Local tests: debug 241 pass, release 239 pass (9
  ignored each), the group-one step 207 run, 8 ignored; the code-generation
  check passes.

## The reference polynomial, twenty-third round (O23, bit-identical, 2026-11-03)

Base: be3907a (the code of O22c). The target is the polynomial that gives the
subtraction's reference and the time search's products their cosine and sine
(`cos_sin_turns`): O22 found it the largest part of the A76's subtraction
(4.56 cycles a sample, about 1.2× its vector floor) and 39 emulated
instructions a sample on the Cortex-M33, about 16 of them the quarter's
selects in integer registers. Three code commits, one optimisation each:
6b6ba4e (the remainder by one shift), 59a59b9 (the quarter by one sign word,
as `f32` selects without a vector unit) and 0104271 (the half turn's sign after
the weight without a vector unit). Then b900ed4, code-generation bounds for
them. No memory is added: static RAM, every stack frame and the slot's
storage are unchanged; flash falls. Decodes, records, residuals and encoder
output are identical; the WAV output differs from b93516e's only in O21's
`--profile` memory figures, as at O21.

| target | measure | be3907a | 0104271 | change |
|---|---|---|---|---|
| Cortex-M33, s0086 host | subtraction, emulated instructions a call | 24.342 M | 22.218 M | **−8.73%** |
| | host slot (setup excluded) | 1 662.94 M | 1 584.35 M | **−4.73%** |
| Cortex-M33, s0407 host | subtraction a call | 24.293 M | 22.174 M | −8.72% |
| | host slot | 1 659.42 M | 1 581.02 M | −4.72% |
| Cortex-A76, crowded host slot | subtraction (medians of 4) | 42.450 ms | 39.615 ms | **−6.68%** |
| | slot | 88.30 ms | 85.50 ms | −3.2% |
| x86-64, `slotcost host host,passes=1 200 5` | user cycles, the whole program (medians of 4) | 324.37 G | 318.72 G | −1.74% |

The A76 gain is fewer vector operations for the same results (the reference's
6.97 → 5.64 a sample); the M33's is mostly the quarter kept in floating-point
registers.

### Measured first (be3907a)

**Cortex-A76** (O18's microbenchmark, O22's form, appended to a scratch copy of
`subtract/tests.rs`, not committed; core 3, after the 5 s sampled busy loop;
cycles a waveform sample, medians over 4 runs × 3 rounds, all valid at 0x0):
the subtraction 19.29; the reference (`F2 − F1`) 4.59, 6.97 vector operations
a sample; the time search's products (`T1 − T0`) 5.80 a core sample, 9.34
vector operations. PC samples (28 307, 23 µs apart): `blocks` 33.6% (the
reference's block of 16 samples 22.6%: 127 instructions, 113 vector),
`fill_phase_only` 15.8%, `symbol_correlations` 15.3% (the products' block 14.5%:
165 instructions, 153 vector), `Fit4::add_all` 13.0%, `Removal::refs` 12.4%,
`score` 3.4%.

**x86-64** (`perf record -e cycles:u` of the same microbenchmark, the whole
subtraction only, core 5): `blocks` 26.9%, `fill_phase_only` 25.0%,
`symbol_correlations` 12.4%, `Removal::refs` 8.5%, `Fit4::add_all` 6.2%,
`skip_with` 5.5%, the bench's own copy of the slot (`memmove`) 4.5%, `score`
3.1%.

**Cortex-M33** (staged host image, s0086, emulated instructions per basic
block): the reference's loop is one block of 39 a sample (216.5 M): the load,
three copies of constants for `vmla`, the remainder (`add`, `bic`, `add`),
the conversion and scaling, the polynomial (nine, with one more copy), the two
parts moved to integer registers (`vmov` ×2), then the quarter's swap and two
sign words in integer registers (`ubfx`, `subs`, `rsbs`, five `and`, two
`orr`, `add`, two `eor`), two stores and the loop's four. The time search's
products, inlined in the subtraction's closure, are one block of 51 a core
sample (142.0 M).

**The generated code** (rustc 1.93.0). Per vector of four samples on AArch64
the remainder was `mov`, `bic`, `add`, `scvtf`, `fmul` (the mask needs a copy),
and the quarter about ten operations (`ushr`, `bic`, `neg` for the swap mask,
`bsl`/`bit` and copies, `and` and `eor` for each of two sign words and an `add`
for the second), with the block's register pressure adding copies and a spill.

### Changes

1. **6b6ba4e: the remainder by one shift.** The remainder r ∈ [−2²⁹, 2²⁹)
   of the phase from its nearest quarter turn is the phase's low 30 bits read
   as signed, so 4r is `phase << 2` read as signed. Converting 4r to `f32`
   rounds to r's bits times 4 and the scaling by 2⁻³⁴ is exact, so t is the
   same `f32`: one shift for the mask, the subtraction and the copy. A test
   compares the function with its first form (kept in the tests) at every
   4 099th phase and the quarter turns' edges; an ignored test at every `u32`
   phase (run: equal).
2. **59a59b9: the quarter by one sign word, as selects without a vector
   unit.** Quarter k gives (cos, sin) = (c, s), (−s, c), (−c, −s), (s, −c): an
   odd k swaps the parts and negates the sine's, then k ≥ 2 negates both
   (`u`'s top bit). The first form built two sign words; now the sine is
   negated once and both parts take one. Where the target has a vector unit
   this stays bit operations (`and`+`cmeq` for the swap mask, one `and` for
   the sign word, `fneg`, two selects, two `eor`); elsewhere it is written as
   `f32` selects and negations, which the Cortex-M33 keeps in floating-point
   registers (`tst`, `vseleq`, `vnmul` for −s). Negation flips only the sign
   bit, as the exclusive or does: the tests check both forms (`BITS` a const
   parameter) against the first at every 4 099th phase and, ignored, at every
   `u32` phase (run on x86-64: equal).
3. **0104271: the half turn's sign after the weight without a vector unit.**
   The subtraction takes cosine and sine only times a weight (the envelope,
   1.0, or the clamped input), so the function takes the weight
   (`cos_sin_turns_times`; `cos_sin_turns` stays for the tests as the weight
   1.0). Without a vector unit the half turn's sign goes on after the
   multiplication, as an exclusive or of the products' bits in the integer
   registers the parts go to for their byte stores anyway: on the M33 that
   replaces two `vneg`, a `cmp` and two `vselge`. w·(−x) = −(w·x) bit for bit
   for every finite weight (corrected in O23b: this said any weight but a
   NaN; an infinite weight times a zero part gives a NaN whose sign the two
   forms can set differently; the decoder's weights are finite); a test checks
   both forms against the first form times 64 finite weights (zeros of both
   signs, subnormals, the largest) at 20 000 phases. The x86-64 and AArch64
   binaries' `.text` and `.rodata` are byte-identical to 59a59b9's.
4. **b900ed4: code-generation bounds** (below).

| loop (instructions; rustc 1.93.0) | be3907a | 6b6ba4e | 59a59b9 | 0104271 |
|---|---|---|---|---|
| AArch64 reference, block of 16 (no envelope / envelope) | 127 / 146 | 118 / 126 | 101 / 111 | the same |
| AArch64 time search's products, block of 16 | 166 | 158 | 137 | the same |
| M33 (opt-level "s") reference a sample (no envelope / envelope) | 38 / 45 | 37 / 44 | 32 / 35 | **30 / 33** |
| M33 products a core sample | 51 | 50 | 41 | **39** |
| M33 staged image (linked), the reference's executed blocks a sample | 39 | | | 31 (10 + 1 + 20) |

The select form compiled for AArch64 (`BITS` false) gives 114 instructions
for the block of 16 against the bit form's 101, so each target keeps its form.

### Cortex-M33 (emulated instructions; staged images as built, rustc 1.93.0, `--cfg cat_ft8_minmaxnm`)

| commit | s0086 host slot | subtraction a call (37 calls) | s0407 host slot | subtraction a call |
|---|---|---|---|---|
| be3907a | 1 662 944 288 | 24 341 877 | 1 659 424 653 | 24 293 113 |
| 6b6ba4e | 1 654 436 845 (−0.51%) | 24 111 946 (−0.94%) | 1 650 937 443 (−0.51%) | 24 063 729 (−0.94%) |
| 59a59b9 | 1 601 066 274 (−3.23%) | 22 669 498 (−5.98%) | 1 597 692 424 (−3.22%) | 22 624 675 (−5.98%) |
| **0104271** | **1 584 352 834 (−1.04%; −4.73% in all)** | **22 217 784 (−1.99%; −8.73% in all)** | **1 581 016 971 (−4.72% in all)** | **22 173 987 (−8.72% in all)** |

- Same decodes (40 and 38 host; 23 and 30 small). Every other stage's count is
  the same to the instruction (762.29 M and 760.58 M outside the subtraction).
- Functions (s0086 host, M): `blocks` 292.13 → 247.19 (the reference's
  blocks 216.5 → 172.6), the subtraction's closure (the products) 144.26 →
  110.61; `Removal::refs` 145.63, `fill_phase_only` 129.09, `Fit4::add_all`
  116.55 unchanged.
- The small slots (no subtraction) move by +1 instruction from 59a59b9
  (183 016 804 → 183 016 805; s0407 likewise), in `decode_a_slot`.
- Paint high water: host 10 260 → 10 252 B; small 10 320 → 10 296 (59a59b9)
  → 10 304 B (0104271), below the base.
- Per decode, host s0086: 41.6 → 39.6 M.

### Cortex-A76 (Raspberry Pi 5; `slotcost host host,passes=1 200 5`, medians of 4 alternated runs)

Aligned builds (`-align-loops=64 -align-all-functions=6`) of be3907a, 6b6ba4e
and 59a59b9 (the code of 0104271 on the hosts) from one scratch path, core 3,
the throttle, clock and temperature sampled once a second from core 0, a 5 s
sampled busy loop on core 3 before each run, alternated (forward, then
reversed), four cycles.

| ms per slot (median, spread) | be3907a | 6b6ba4e | 59a59b9 |
|---|---|---|---|
| **subtract** | 42.450 (0.050) | 41.220 (0.020): −2.90% | **39.615 (0.040): −2.835, −6.68%** |
| pass 2 / pass 3 subtract | 29.575 / 12.880 | 28.710 / 12.505 | 27.595 / 12.020 |
| slot | 88.30 | 87.10 | **85.50** |
| worst slot | 94.1 | 92.7 | 90.7 |
| feed (placement check) | 19.200 | 19.215 | 19.205 |
| belief propagation (placement check) | 19.050 | 19.055 | 19.055 |
| sync / soft bits / SNR (placement check) | 4.680 / 2.380 / 0.240 | the same | the same |
| one pass: slot, feed | 18.30, 6.455 | 18.30, 6.465 | 18.30, 6.450 |

- **Placement check.** The untouched stages agree within 0.015 ms across the
  three builds, and the one-pass slot is the same. The gain (2.835 ms) is
  about 190 times that and 70 times its stage's spread; no placement variation
  was run.
- **Microbenchmark** (cycles a waveform sample, medians over 4 alternated runs
  × 3 rounds, all valid): the subtraction 19.288 → 18.834 → 18.095 (−6.2%);
  the reference (`F2 − F1`) 4.59 → 4.13 → 3.81 (−17%); the time search's
  products (`T1 − T0`) 5.80 → 5.56 → 4.81 a core sample (−17%). Vector
  operations (`ASE_SPEC`, two runs each): the subtraction 22.96 → 20.68 a
  sample, the reference 6.97 → 5.64, the products 9.34 → 7.63. PC samples at
  59a59b9 (26 373): the reference's block 19.6% (101 instructions, 90
  vector), the products' block 13.1% (137, 126). The generator parts whose
  code did not change read 1.3–1.5% slower in the head's test binary (`T0`
  4.075 → 4.126, `F0` 2.388 → 2.425): placement in that binary; `slotcost`'s
  untouched stages do not move.
- **Validity.** All 12 timed runs are valid: 1 557 samples during the runs and
  the warm-ups, every one 0x0, start and end checks included; every sample
  during a run at 2 400 MHz; one warm-up sample at 1.7 GHz (the first). Peak
  53.8 °C. The program's own output is logged with a prefix no sample line
  has. The 17 microbenchmark runs at the head and base likewise (all valid, 2 400 MHz during each run).

### x86-64 (Threadripper 3970X, core 5, `perf stat` user counters over the whole `slotcost` program)

Aligned builds of the same three commits from the same scratch path, four
alternated rounds (forward, then reversed), `slotcost host host,passes=1 200
5`; a check for other jobs before and after each sample, none void.

| build | cycles (median, range) | instructions | subtract stage (ms) |
|---|---|---|---|
| be3907a | 324.37 G (323.14–324.92) | 914.33 G | 24.890 |
| 6b6ba4e | 322.57 G (321.40–323.06), −0.55% | 912.44 G | 24.460 |
| 59a59b9 | **318.72 G (318.45–318.79), −1.74%** | 900.14 G (−1.55%) | 23.630 (−5.1%) |

- 59a59b9's range clears the base's by 4.4 G; 6b6ba4e's only just clears it
  (321.40–323.06 against 323.14–324.92, by 0.08 G).
- Stage times are the virtual machine's clock, for scale only.

### Sizes and stack (`check.sh --stack`, thumbv8m)

| image | flash be3907a → 6b6ba4e → 59a59b9 → 0104271 (B) | static RAM (B) |
|---|---|---|
| small, small with OSD on | 92 082 (unchanged) | 201 668 (unchanged) |
| host, host with OSD off | 103 542 → 103 490 → 103 154 → **103 182 (−360 in all)** | 230 084 (unchanged) |
| encoder only | 35 286 (unchanged) | 0 |

- Frames unchanged at every commit: host `subtract_one` 4 920 B, the
  subtraction's closure 2 536, `_start` 4 104, `decode_a_slot` 2 520,
  `finish_row` 464, `osd_decode` 272. No scratch or storage added
  (`SUBTRACT_STACK_BYTES` and `SUBTRACT_SCRATCH_BYTES` unchanged).
- No fused multiply-add (0 `vfma`/`vfms`/`vfnma`/`vfnms`; `vmla` 128 / 211 /
  17, unchanged) and no `f64` helper in any image.
- Cortex-M4F (thumbv7em, embedded-check images, no cfg; no `vsel` there, the
  select form becomes `it`-predicated `vmov`): small flash 92 348 (unchanged),
  host 104 024 → 103 972 → 103 764 → 103 628 B (−396); static RAM unchanged;
  no `f64` helper, no fused multiply-add; `fmaxf`/`fminf` calls 10 and 14,
  unchanged.
- Hosts' `.text`: AArch64 `paired_bench` 1 170 200 → 1 169 816 → 1 170 648 B
  (+448 in all), `cat-ft8-decode` 397 672 → 397 288 → 398 120; x86-64
  1 412 022 → 1 411 638 → 1 412 022 and 457 478 → 457 094 → 457 478;
  x86-64 `.rodata` −64 B (constants). 0104271 equals 59a59b9 on both hosts.

### Sections of all eight binaries

`.text` and `.rodata` (one scratch path, fresh tree, rustc 1.93.0; thumbv8m
with `--cfg cat_ft8_minmaxnm`):

| binary | 6b6ba4e against be3907a | 59a59b9 against 6b6ba4e | 0104271 against 59a59b9 |
|---|---|---|---|
| x86-64 `cat-ft8-decode`, `paired_bench` | differ | differ | identical |
| AArch64 `cat-ft8-decode`, `paired_bench` | `.text` differs | `.text` differs | identical |
| M33 check, small | identical | identical | identical |
| M33 check, host; staged small and host | differ | differ | differ |

b900ed4 changes only `codegen-check.sh`.

### The code-generation check (b900ed4)

Two metrics: `cvtloopN` (the instructions of the shortest loop with at least
N conversions from integers, `scvtf` or `vcvt.f32.s32`, a ceiling) and `fpneg`
(`vneg.f32`, a ceiling); the loop scan now also knows AArch64's conditional and
test branches. Each forced form below is applied to a copy of the head (one
log each, the check's own exit status):

| form | a64 `blocks` (≤ 104) | a64 `symbol_correlations` (≤ 142) | m33s `blocks` (≤ 31) | m33s closure (≤ 40) | m33s `vneg` in `blocks` (≤ 0) | exit |
|---|---|---|---|---|---|---|
| head | 100 | 137 | 30 | 39 | 0 | 0 |
| f1: the remainder by mask and subtraction | **107** | **148** | 31 | 40 | 0 | 1 |
| f2: the vector form's quarter by two sign words | **112** | **157** | 30 | 39 | 0 | 1 |
| f3: the swap by bit operations without a vector unit | 100 | 137 | **32** | **45** | 0 | 1 |
| f4: the sign before the weight without a vector unit | 100 | 137 | **32** | **41** | **12** | 1 |

No other check failed in any form. What it misses: the remainder's one
instruction a sample on the M33 (within those ceilings; the AArch64 bounds
catch its form); up to one instruction in each M33 loop and four or five in
the AArch64 blocks; other targets (x86-64, the M4F); the linked image (it
reads the library at opt-level "s"; the linked image's blocks are counted
above).

### Identity

All at 0104271's binaries (b900ed4 changes only the check).

- **Records, x86-64:** the 73 800 `paired_bench run` records equal b93516e's
  (host, one pass, small: 24 600 each, 0 differ; the records comparison exits 0
  each).
- **800 WAV slots** × 3 configurations × 3 modes, x86-64 (7 200 runs, 78 690
  decode lines): 93 091 lines byte-identical to b93516e's, and the 2 400
  `--profile` lines differ only by the sync scratch, as at O21 (exit 0).
- **A76:** the records of 0104271's AArch64 `paired_bench` equal x86-64
  b93516e's (73 800, 0 differ; exit 0 each), and the 800 WAV slots × 3 × 3 with
  its `cat-ft8-decode` give the same 93 091 identical lines and 2 400
  `--profile` lines (exit 0). One benchmark at a time, in fresh directories;
  317 throttle samples, start and end included, all 0x0.
- **Encoder:** the 45 `cat-ft8-encode` WAV files equal 70be604's.
- **Cortex-M33 (emulated):** the small and host dump images on seven slots
  (14 of 14) and the multi-pass residuals of s0086 and s0407 equal 68634cc's
  (exit 0).
- **Comparators on a forced difference** (copies; each its own exit status):
  the records comparison 0 unforced, 1 with one record's field changed; the
  WAV comparison 0 unforced, 1 with a decode line's frequency one higher, 1
  with a `--profile` line's `peak_bytes` one higher; the M33 comparison 0
  unforced, 1 with one bit of one dump flipped (15 identical, 1 differ); `cmp`
  of an encoder WAV 0 unforced, 1 with one byte changed.
- **Exhaustively:** both forms equal the first form at every `u32` phase, in
  release on x86-64 (local CI) and on the A76 (below).

### Tests and mutants

New: `cos_sin_turns_is_its_first_form_bit_for_bit` (both forms, every 4 099th
phase and the quarter turns' edges), `cos_sin_turns_is_its_first_form_everywhere`
(ignored: every `u32` phase; run in release on x86-64 and on the A76: equal) and
`cos_sin_turns_times_is_the_weight_times_the_parts` (both forms, 64 finite
weights, 20 000 phases).

Mutants in copies of b900ed4's tree, release library tests, failing tests per
mutant (one log each, counts read from the logs; the form without a vector
unit runs on a host only through its tests, which call it directly):

| mutant | failing (of 209) |
|---|---|
| none (control) | 0 |
| the remainder scaled by 2⁻³² | 13 |
| the remainder shifted by one | 13 |
| the half turn's sign from bit 30 | 17 |
| the vector form's swap from bit 31 | 13 |
| the vector form's sine not negated | 11 |
| the select form's swap inverted | 2 |
| the select form's sine not negated | 2 |
| the select form's half-turn sign dropped | 2 |
| the select form's half-turn sign on the cosine only | 2 |
| the select form's sign before the weight (equivalent: the first form's bits) | 0 |

A76 release library tests with the ignored ones, cross-built at b900ed4
(`cat-ft8` tree 0263f86e…): 217 pass, 0 fail, the source-tree test skipped,
the exhaustive comparison included; 50 throttle samples, all 0x0.

Local CI passes at b900ed4: fmt; clippy `-D warnings` (workspace, all targets
and features, thumbv8m); workspace, profile, no-std and release tests with the
ignored ones (218, the two exhaustive comparisons included); the group-one step
(209 run, 9 ignored); the bulk run; MSRV 1.75; docs with and without `std`;
`check.sh --stack`; the code-generation check (the new bounds 100, 137, 30, 39
and 0).

### What bounds the reference now

- **Cortex-A76:** the reference 3.81 cycles a sample with 5.64 vector
  operations (a floor of 2.82 on two pipes, 1.35×); the products 4.81 a core
  sample with 7.63 (a floor of 3.82, 1.26×). Per vector of four: the
  remainder 3, the polynomial 10 with t², the quarter 7 (`and`+`cmeq`, `and`,
  `fneg`, two selects, two `eor`).
- **Cortex-M33** (the linked image): the reference 31 instructions a sample:
  the load; four copies of constants for `vmla` (Horner with an accumulating
  multiply-add); the remainder, conversion and scaling (`lsls`, `add`,
  `vmov`, `vcvt`, `vmul`); t², the polynomial and −s (seven); the swap
  (`tst`, two `vseleq`); the sign (`and`, two `eor`); two `vmov` to integer
  registers and two `str` (the reference is stored as bytes); the loop's
  four. The products 39 in the library, of which the clamp is 6.

### Tried, not kept

- **Other spellings of the swap mask** (`(u << 1) as i32 >> 31`,
  `0 − (u & 2³⁰ ≠ 0)`): the same code on AArch64 (`and`, `cmeq`); no `cmtst`.
- **The select form on AArch64** (`BITS` false): 114 instructions for the
  block of 16 against 101; the hosts keep the bit form.
- **A fixed-point conversion** (`scvtf`/`vcvt` with fraction bits, folding the
  scaling): the vector conversion at be3907a was already `scvtf` then `fmul`
  (the scalar one used `#32`); 4r would need 34 fraction bits (at most 32),
  and r by `asr` then a fixed-point conversion has as many operations. Not
  built.
- **The polynomial on the unscaled 4r** (every coefficient scaled by a power of
  two, exact): it saves the multiplication by 2⁻³⁴ but needs one to scale each
  part back, unless folded into the weight, which is exact only for weights
  that do not become subnormal; one operation more without an envelope.
  Analysis only.
- **A table in place of the polynomial:** it cannot give the same bits without
  holding the polynomial's value at every remainder (2³⁰); not built.
- **The reference stored as `f32`** (`vstr`, no moves to integer registers on
  the M33): the ring is a byte buffer lent by the decoder; viewing it as `f32`
  needs `unsafe` or a dependency. Not built.
- **The clamp once** (O22's candidate) was not built: it adds 23 040 B of
  scratch.

### Risks

- The M33 gain rests on code generation at opt-level "s" (`vsel` for `f32`
  selects, the exclusive or after the weight), now checked; a toolchain change
  can lose it with every test passing. The M4F's select form (`it`-predicated
  `vmov`) was checked for size and library calls only.
- The two forms give the same bits for every finite weight (corrected in
  O23b: this said every weight but a NaN). A NaN weight, or an infinite one
  times a zero part (the sine at a quarter turn), gives a NaN whose sign bit
  can differ between the forms. The decoder's weights are the
  envelope table, 1.0 and the clamped input (non-finite inputs become 0).
- `cos_sin_turns` is now test-only; production code calls
  `cos_sin_turns_times`.
- AArch64 code grows by 448 B.

### Not measured

- Cortex-M33 cycles (emulated instructions only); M33 counts on any slot but
  s0086 and s0407 (seven slots for identity); any M4F count or identity.
- Sparse and empty slots on any target (the A76 timing is the crowded `main`
  block; the microbenchmark is one synthesized crowded slot).
- The microbenchmark on other A76 cores; a placement variation (the gain is
  well over O21c's ±1% of a stage, and the untouched stages agree).
- The exhaustive comparison on thumbv8m (run on x86-64 and the A76; the M33
  is covered by its identity runs, not by the exhaustive test).
- After a toolchain change: the bounds above.
- x86-64 stage times (the virtual machine's clock); x86-64 timing on real
  recordings. The x86-64 profile is from one run.

## Review follow-ups (O23b, 2026-11-03; no code generation change)

On top of O23 (756e8f1), one concern per commit: 7a734b6, 4e1aaaa, 9b0814d,
4bb9e6e, ee2b5c7 and f633ecb, then this record. `.text` and `.rodata` of all
eight binaries are byte-identical to 0104271's at 7a734b6, 4e1aaaa, 9b0814d
and 4bb9e6e (one scratch path, rustc 1.93.0); ee2b5c7 and f633ecb change only
`codegen-check.sh`. So O23's timing, counts and identity stand for the head.

- **7a734b6: finite weights.** The doc said the two forms of
  `cos_sin_turns_times` agree for any weight but a NaN. An infinite weight
  times a zero part (the sine at a quarter turn) gives a NaN, whose sign bit
  the select form turns and the bit form does not; they agree for every
  finite weight. Corrected in math.rs and in O23 above. The decoder's weights
  are the envelope, 1.0 and the clamped input: finite.
- **4e1aaaa: the bit form keeps its sign before the weight.** Tried: both
  forms with the half turn's exclusive or after the weight, so that `BITS`
  decides only the swap (the library's assembly, rustc 1.93.0):

  | loop (instructions) | split (head) | shared tail |
  |---|---|---|
  | AArch64 reference, block of 16, no envelope (two monomorphs) | 100 / 101 | 100 / 101 |
  | AArch64 reference, block of 16, envelope | 108 / 111 | **109 / 113** |
  | AArch64 products, block of 16 | 137 | 137 |
  | x86-64 reference blocks | 160–180 | 162–176 (four shorter, two longer) |
  | x86-64 products | 208 | 209 |
  | Cortex-M33 (opt-level "s") | 30–34, products 39 | the same code |

  Worse on AArch64's blocks with an envelope, so not adopted (not timed); a
  comment at the bit form says why.
- **9b0814d: the weight test takes the quarter turns' edges.** Each quarter
  turn's neighbours (0 to 3 away) and the ends of its reduced range now go
  through all 64 weights, not only random phases. A mutant that leaves the
  half turn's sign off zero products in the select form fails it (and the
  first-form test); checked by a mutant run.
- **4bb9e6e:** `cos_sin_turns_times_as` is private; a test comment wrapped.
  The products in `scaled_cos_sin` stay a local (`cs`) before `put`: inlined,
  `k0` is used before `phase` and the closure's captures are laid out in
  another order, which changes the hosts' code (60 lines of AArch64 and 154 of
  x86-64 assembly differ); a comment says so.
- **ee2b5c7: the products' check needs one closure.** On the Cortex-M33 the
  products are inlined in the subtraction's closure, named only by its
  parent (`run`), and a loop metric took the shortest loop over every
  function whose label held the name. A fifth field, `one`, now fails a check
  if more than one label holds its name. Forced forms (one log each, the
  check's own exit status; f5 and f6 add a second closure in `run`, called
  through a `dyn Fn`, whose loop of conversions is 32 instructions):

  | form | `blocks` (≤ 31) | products' closure (≤ 40, `one`) | exit | b900ed4's check |
  |---|---|---|---|---|
  | head | 30 | 39 | 0 | |
  | f3: the swap by bit operations | **32** | **45** | 1 | |
  | f4: the sign before the weight | **32** | **41** (and 12 `vneg`) | 1 | |
  | f5: f3 and a second closure | **32** | **more than one** | 1 | closure read 34, ok (the 45 hidden); exit 1 from `blocks` only |
  | f6: a second closure alone | 30 | **more than one** | 1 | closure read 32, ok; exit 0 |

- **f633ecb:** the check's header says what the polynomial's bounds protect
  and points here; the counts are in O23. The FFT's list of what its bounds
  miss, which O23's paragraph had been put in front of, is back after its own
  bullet.

### Tests and checks

Local CI passes at f633ecb: fmt; clippy `-D warnings` (workspace, all
targets and features, thumbv8m); workspace, profile, no-std and release tests
with the ignored ones (218, the exhaustive comparisons included); the
group-one step (209 run, 9 ignored); the bulk run; MSRV 1.75; docs with and
without `std`; `check.sh --stack` (flash and frames as at O23); the
code-generation check (100, 137, 30, 39 with one closure, 0). A76 release
library tests with the ignored ones, cross-built at f633ecb (`cat-ft8` tree
8d6a10f0…): 217 pass, 0 fail, the source-tree test skipped; 50 throttle
samples, all 0x0. Identity was not rerun: no binary's `.text` or `.rodata`
changed.

### Not measured

- As in O23. The shared tail was compared by the library's assembly only (not
  timed, no linked-image counts).
## The time search's products and phase generator, twenty-fourth round (O24, bit-identical, 2026-11-03)

Base: 7f4e73f (the code of O23b). The target is the subtraction's time
search: its products (the audio times the reference's cosine and sine, and
their sums by lane) and the phase generator that feeds it
(`Synth::phases`, through `fill_phase_only` and `skip_with`). After O23 the
A76's products were 4.81 cycles a core sample against the reference's 3.81,
x86-64 spent about a quarter of the subtraction's samples in
`fill_phase_only`, and the M33's products were 39 instructions a core
sample. It was profiled first on all three targets. Three code commits,
one optimisation each: ee64690 (the products summed as they are taken),
5c9977c (the generator's overlap runs summed as they are taken without a
vector unit) and eb210f9 (a skipped span's sum from its pair of tones the
other way round). Then c4d5b08, code-generation bounds for the first two.
Decodes, records, residuals and encoder output are identical; the WAV
output differs from b93516e's only in O21's `--profile` memory figures, as
at O21. No memory is added: static RAM and the slot's storage are
unchanged, two frames fall by 8 B, and flash falls (host −228 B).

| target | measure | 7f4e73f | eb210f9 | change |
|---|---|---|---|---|
| Cortex-M33, s0086 host | subtraction, emulated instructions a call | 22.218 M | 20.474 M | **−7.85%** |
| | host slot (setup excluded) | 1 584.35 M | 1 519.83 M | **−4.07%** |
| Cortex-M33, s0407 host | subtraction (all calls) | 820.44 M | 755.80 M | −7.88% |
| | host slot | 1 581.02 M | 1 516.38 M | −4.09% |
| Cortex-A76, crowded host slot | subtraction (medians of 4) | 39.620 ms | 38.510 ms | **−2.80%** |
| | slot | 85.45 ms | 84.30 ms | −1.35% |
| | time search's correlations (microbenchmark, cycles a core sample) | 8.512 | 7.445 | −12.5% |
| x86-64, `slotcost host host,passes=1 200 5` | user cycles, the whole program (medians of 4) | 319.07 G | 313.81 G | −1.65% |

The M33 gain is the products kept in floating-point registers and summed
in place (no stores, no second pass), the generator's increments summed as
they are taken, and fewer skipped spans generated. The A76 and x86-64 gains
are the products' stores, loads and lane adds removed, one exclusive or
fewer a vector, and fewer skipped spans.

### Measured first (7f4e73f)

**Cortex-A76** (O18's microbenchmark in O22's form, appended to a scratch
copy of `subtract/tests.rs`, not committed; core 3 after the 5 s sampled
busy loop; two runs × three rounds, all valid at 0x0; cycles a waveform
sample, the time search's parts a core sample): the subtraction 18.085;
the time search's generator over the cores with the rest skipped by
`skip_with` (`T0`) 4.126; the products stored (`T1`) 8.910; the
correlations as the decoder runs them (`T2`: generator over the cores, the
skipped spans' sums, products, lane sums) 8.514, of which 10.37 vector
operations and 18.0 instructions (raw counters, two runs). PC samples:

| where (A76, 7f4e73f) | share |
|---|---|
| **the subtraction** (26 359 samples, 23 µs apart) | |
| `blocks` (the reference's block of 16 19.1%, the correction 10.1%) | 31.9% |
| `fill_phase_only` (increments 4.8% + 4.3%, running sums 2.6% + 2.1%, the flat run 1.0%) | 17.0% |
| `Fit4::add_all` | 14.0% |
| `symbol_correlations` (the products' block of 16, 13.1%) | 13.9% |
| `Removal::refs` | 12.3% |
| `score` | 3.6% |
| **the time search alone** (`T2`, 6 575 samples) | |
| `symbol_correlations` (the products' block 53.5%) | 57.0% |
| `fill_phase_only` | 17.9% |
| `skip_with` (the skipped spans of pairs first met) | 12.7% |
| `whole_lane_sums` | 5.4% |
| not attributed (a mapping symbol) | 6.9% |

The products' block of 16 samples is 137 instructions, 126 vector: per
vector of four the remainder 3, the polynomial 10, the quarter 8, the
clamp 5, the weight 2, and over the block 9 `dup`s that rebroadcast
constants (register pressure) and a move; then 8 stores, and
`whole_lane_sums` loads them back (2 loads and 2 adds a vector).

**x86-64** (`perf record -e cycles:u` of the same microbenchmark, the
subtraction only, core 5): `fill_phase_only` 27.8%, `blocks` 24.3%,
`symbol_correlations` 12.1%, `Removal::refs` 9.9%, `Fit4::add_all` 7.9%,
`skip_with` 5.0%, the bench's copy of the slot 4.0%, `score` 3.5%,
`whole_lane_sums` 2.0%. In `fill_phase_only` the running sum's one scalar
add takes 17.7% of its samples (a dependent add per sample). The time
search alone (`T0`–`T4`): `skip_with` 38.0% (most of it `T0`'s whole
skips), `fill_phase_only` 23.1%, `symbol_correlations` 15.1%.

**Cortex-M33** (staged host image, s0086, 37 subtractions; 822.06 M of the
1 584.35 M slot; emulated instructions per function and basic block):

| part (M33, 7f4e73f) | M | per sample |
|---|---|---|
| the products (the subtraction's closure; one block of 39 a core sample, 2.784 M core samples) | 110.61 | 39 |
| `whole_lane_sums` | 25.92 | 9 a core sample |
| `fill_phase_only` (fit path and cores): increments 14 + 1 and 13 + 1, running sums 5 and 5, the flat run 5 | 129.09 | |
| `skip_with` (the skipped spans): increments 14 and 13, sums 4 and 4 | 27.76 | |
| `blocks` (fit path: correction, reference), `Removal::refs`, `Fit4::add_all` | 247.19, 145.63, 116.55 | (not this round's) |

On the M33 the products went to integer registers for the half turn's
exclusive or and their byte stores (2 `vmov`, 2 `eor`, 2 `str`), then
came back in `whole_lane_sums`; the generator's increments were stored
and summed in a second pass.

### Changes

1. **ee64690: the time search's products summed as they are taken.**
   - `symbol_correlations` stored each block's products in the caller's
     scratch (`scaled_cos_sin`) and then summed them by lane (`lane_sums`,
     `whole_lane_sums`). `correlation` now adds each product to its lane's
     sums as it is taken: sample i of the block in lane i % 4, each lane
     from 0.0 in sample order, the lanes added by `sum_lanes`, as before,
     so the sums are the same bit for bit.
   - Where the target has a vector unit, the four lanes side by side: the
     single samples up to the first whole lane, the whole lanes out of
     line (`correlation_whole`, 16 samples at a time, then four), the
     single samples after them. Elsewhere one lane at a time, its two sums
     in locals over the block (the form O22 found keeps the M33's sums in
     registers).
   - The products take the half turn's sign on the weight
     (`cos_sin_turns_signed_as`): (−w)·x = −(w·x) bit for bit for finite w
     and x, so they equal `cos_sin_turns_times`' for the decoder's finite
     weights. The vector form: one `eor` on the weight instead of one on
     each part. The select form: the weight negated by a select (`vneg`,
     `vsel`), so the parts stay in floating-point registers for the sums
     (the stored form took them to integer registers for the stores and
     the exclusive or). The polynomial is shared with
     `cos_sin_turns_times_as` (`reduced`; the reference's code is
     unchanged by that).
   - The time search no longer uses the caller's scratch (its parameter is
     gone; `SUBTRACT_SCRATCH_BYTES` and `SUBTRACT_MIN_SCRATCH_BYTES` are
     unchanged, the reference needs them). The code-generation check's
     entries for the removed functions name `correlation_whole`.
   - Loops (rustc 1.93.0, the library): AArch64 the products' block of 16
     137 instructions (123 vector) with the sums, against 137 (126
     vector, 9 of them constants rebroadcast) without them plus
     `whole_lane_sums`' loop; x86-64 203 against 208 plus the sums;
     Cortex-M33 (opt-level "s") 36 a core sample against 39 for the
     products and 9 for the lane sums.
2. **5c9977c: the generator's overlap runs summed as they are taken
   without a vector unit.** Where two pulses overlap, `Synth::phases`
   stores each increment (a loop that vectorises) and then takes their
   running sum. Without a vector unit that loop is scalar anyway: there
   (`FUSED_SUM`: no vector unit, or the tests built with
   `--cfg cat_ft8_test_group_one`) each increment is added to the phase
   as it is taken, the phase before it stored first. M33 (opt-level "s"):
   the two runs 17 and 15 instructions a sample against 15 + 5 and
   14 + 5; the phase-only skip 16 and 14 against 15 + 4 and 14 + 4. The
   AArch64 binaries are byte-identical to ee64690's; x86-64's `.text`
   differs by one instruction's place in `fill_phase_only`.
3. **eb210f9: a skipped span's sum from its pair of tones the other way
   round.** The time search's skipped span (960 samples between two
   cores, O22) is as many samples before the symbols' boundary as after
   it. There each increment takes the two pulses at mirrored places,
   a·H[NS − x − PF] + b·H[NS + x − PF] at x samples from the boundary
   (H the stored half pulse), in either order; IEEE addition is
   commutative, so the increment at −x for tones (a, b) equals the one at
   +x for (b, a). So the span of (b, a) holds the increments of (a, b)'s
   less its first and plus the first of (b, a)'s:
   S(b, a) = S(a, b) − i(a, b) + i(b, a), where i is the increment at
   the span's first sample (`Synth::transition_increment`, the
   generator's expression there). A pair met after its mirror takes its
   sum so, two increments instead of 960 samples of the generator. Over
   2 000 random payloads a message meets 41.6 ordered pairs at its 78
   boundaries, of which 12.8 now come from their mirror (28.8 spans
   generated).
4. **c4d5b08: the code-generation check** (below).

### Cortex-M33 (emulated instructions; staged images as built, rustc 1.93.0, `--cfg cat_ft8_minmaxnm`)

| commit | s0086 host slot | subtraction (all 37 calls) | s0407 host slot | subtraction (all calls) |
|---|---|---|---|---|
| 7f4e73f | 1 584 352 834 | 822 058 015 | 1 581 016 971 | 820 437 528 |
| ee64690 | 1 550 625 922 (−2.13%) | 788 331 103 (−4.10%) | 1 547 369 470 (−2.13%) | 786 790 027 (−4.10%) |
| 5c9977c | 1 526 363 100 (−1.56%) | 764 068 281 (−3.08%) | 1 523 137 356 (−1.57%) | 762 557 913 (−3.08%) |
| **eb210f9** | **1 519 833 890 (−0.43%; −4.07% in all)** | **757 539 071 (−0.85%; −7.85% in all)** | **1 516 383 326 (−4.09% in all)** | **755 803 883 (−7.88% in all)** |

- A call on s0086 (37): 22.218 M → 20.474 M. Per decode, host s0086:
  39.6 → 38.0 M.
- Same decodes (40 and 38 host; 23 and 30 small). Every other stage's count
  is the same to the instruction, except `setup` (the image's start-up
  encoder check, which runs the generator): −403 545 from 5c9977c on both
  slots. The small slots (no subtraction) are unchanged: 183 016 805 and
  175 802 921.
- Functions (s0086 host, M): the products' closure 110.61 → 102.80 and
  `whole_lane_sums` 25.92 → gone (ee64690); `fill_phase_only` 129.09 →
  109.56 and `skip_with` 27.76 → 22.62 (5c9977c); `skip_with` → 16.08
  (eb210f9). `blocks`, `Removal::refs` and `Fit4::add_all` unchanged
  (247.19, 145.63, 116.55).
- Paint high water: host 10 252 → 10 244 B (ee64690 on); small 10 304 B
  unchanged.
- The linked image's executed blocks (s0086, per block): the products and
  sums one block of 36 a core sample (98.97 M; 39 + 9 before); the
  generator's overlap runs 15 + 1 and 14 + 1 a sample (the running sums'
  5 + 5 gone); the skipped spans' generator 524 132 + 504 960 samples
  (733 892 + 714 720 before, −29%).

### Cortex-A76 (Raspberry Pi 5; `slotcost host host,passes=1 200 5`, medians of 4 alternated runs)

Aligned builds (`-align-loops=64 -align-all-functions=6`) of 7f4e73f,
ee64690 (whose AArch64 binaries' `.text` and `.rodata` 5c9977c's equal)
and eb210f9 from one scratch path, core 3, the throttle, clock and
temperature sampled once a second from core 0, a 5 s sampled busy loop on
core 3 before each run, alternated (forward, then reversed), four cycles.

| ms per slot (median, spread) | 7f4e73f | ee64690 | eb210f9 |
|---|---|---|---|
| **subtract** | 39.620 (0.040) | 38.920 (0.030): −1.77% | **38.510 (0.010): −1.110, −2.80%** |
| pass 2 / pass 3 subtract | 27.600 / 12.020 | 27.115 / 11.810 | 26.820 / 11.680 |
| slot | 85.45 | 84.70 | **84.30 (−1.35%)** |
| worst slot | 90.75 | 89.90 | 89.45 |
| feed (placement check) | 19.195 (0.030) | 19.120 (0.020) | 19.130 (0.020) |
| belief propagation / sync / soft bits / SNR (placement check) | 19.060 / 4.680 / 2.380 / 0.240 | 19.060 / the same | 19.055 / the same |
| one pass: slot, feed | 18.30, 6.450 | 18.30, 6.460 | 18.30, 6.460 |

- **Placement check.** Belief propagation, sync, soft bits, SNR, OSD-1,
  refine and the one-pass slot agree within 0.005 ms. The feed, whose
  code did not change, reads 0.065–0.075 ms (0.4%) faster in both new
  builds: placement (O21c saw ±1% of a stage). Each step's gain in the
  subtraction (0.700 and 0.410 ms) is 5–10 times that and 10–40 times
  its stage's spread. No placement variation was run.
- **Microbenchmark** (cycles, medians over 4 alternated runs × 3 rounds,
  all valid): the subtraction 18.085 → 17.782 → 17.505 a waveform sample
  (−3.2%); the time search's correlations as the decoder takes them
  (`T2`) 8.512 → 7.863 → 7.445 a core sample (−12.5%). Raw counters (two
  runs each): `T2` 10.37 → 9.90 vector operations and 18.0 → 16.0
  instructions a core sample, integer operations 4.02 → 2.89, stores 0.89
  → 0.74, loads 1.53 → 1.64 (the constants the loop reloads from the
  stack); the subtraction 20.68 → 20.45 vector operations, 41.24
  → 40.20 instructions a sample. `T1` (the bench's own form, the generator's
  whole skips) now takes the sums too, so it is not compared (8.913 →
  9.010 → 8.925); the fit path's parts (`F0`–`F4`, `R`) move by less than
  0.6%.
- PC samples at eb210f9 (25 540 in the subtraction): `blocks` 34.4% (the
  reference's block 20.7%), `fill_phase_only` 17.5%, `Fit4::add_all`
  14.1%, `correlation_whole` 13.5% (its loop 13.2%: 140 instructions in
  the test binary, 124 vector), `Removal::refs` 11.5%, `score` 3.7%;
  `whole_lane_sums` gone. The time search alone (5 810): the whole lanes
  60.6%, `fill_phase_only` 18.7%, `skip_with` 10.5% (12.7% before).
- **Validity.** All 12 timed runs are valid: 1 472 samples during the
  runs and 60 in the warm-ups, every one 0x0, the 12 end checks included;
  every sample during a run at 2 400 MHz; one warm-up sample at 1.7 GHz
  (the first). Peak 54.3 °C. The program's own output is logged with a
  prefix no sample line has. The 18 microbenchmark runs likewise (all
  valid; 142 samples, all 0x0; 2 400 MHz during each run).
### x86-64 (Threadripper 3970X, core 5, `perf stat` user counters over the whole `slotcost` program)

Aligned builds of the four commits from the same scratch path, four
alternated rounds (forward, then reversed), `slotcost host host,passes=1 200
5`; a check for other jobs before and after each sample.

| build | cycles (median, range) | instructions | subtract stage (ms) |
|---|---|---|---|
| 7f4e73f | 319.07 G (318.55–319.59) | 900.14 G | 23.715 |
| ee64690 | 315.59 G (315.34–316.35), −1.09% | 895.33 G | 22.850 |
| 5c9977c | 315.73 G (314.70–316.93), −1.05% | 895.33 G | 22.805 |
| eb210f9 | **313.81 G (312.43–314.11), −1.65%** | 891.36 G (−0.98%) | 22.420 (−5.5%) |

- ee64690's range clears the base's by 2.2 G; 5c9977c's x86-64 change (one
  instruction's place) is within ee64690's range, its instructions equal;
  eb210f9's range clears ee64690's and 5c9977c's.
- **A first session was void**: the A76 identity's local driver (a shell
  that only waits on its remote commands) ran beside it and the job check
  counted it, so 10 of its 16 samples were marked VOID. Its 6 valid
  samples agree (base 318.20 G, ee64690 314.25 G, 5c9977c 313.96–315.95 G,
  eb210f9 314.41–315.36 G). The session above ran after every other job
  had ended: none void.
- Stage times are the virtual machine's clock, for scale only.
- Profile at eb210f9 (`perf record` of the microbenchmark, one run each,
  core 5): the subtraction: `fill_phase_only` 28.6%, `blocks` 26.6%,
  `correlation_whole` 11.7% (`symbol_correlations` 12.1% and
  `whole_lane_sums` 2.0% before), `Removal::refs` 9.2%, `Fit4::add_all`
  8.1%, the bench's copy 4.5%, `skip_with` 3.8% (5.0%), `score` 3.3%. The
  generator's share rose because the rest fell; its code on x86-64 is
  unchanged.


### Sizes and stack (`check.sh --stack`, thumbv8m)

| image | flash 7f4e73f → ee64690 → 5c9977c → eb210f9 (B) | static RAM (B) |
|---|---|---|
| small, small with OSD on | 92 082 → 92 082 → 91 982 → **91 982 (−100)** | 201 668 (unchanged) |
| host, host with OSD off | 103 182 → 102 838 → 102 734 → **102 954 (−228 in all; +220 at eb210f9)** | 230 084 (unchanged) |
| encoder only | 35 286 → 35 286 → 35 186 → **35 186 (−100)** | 0 |

- Frames: the subtraction's closure 2 536 → 2 528 B (ee64690),
  `Synth::skip` 352 → 344 B (5c9977c; all images); host `subtract_one`
  4 920, `_start` 4 104, `decode_a_slot` 2 520, `finish_row` 464,
  `osd_decode` 272 B unchanged. No scratch or storage added:
  `SUBTRACT_STACK_BYTES` (4 920) and `SUBTRACT_SCRATCH_BYTES` unchanged;
  the time search no longer uses the scratch. Paint high water above.
- No fused multiply-add (0 `vfma`/`vfms`/`vfnma`/`vfnms`); `vmla` small 128
  and encoder 17 unchanged, host 211 → 213 (ee64690) → 217 (eb210f9:
  `transition_increment`'s sums). No `f64` helper in any
  image.
- Cortex-M4F (thumbv7em, embedded-check images, no cfg): small flash
  92 348 → 92 248 B (5c9977c), host 103 628 → 103 288 → 103 184 →
  103 400 B (−228); static RAM unchanged; no `f64` helper, no fused
  multiply-add; `fmaxf`/`fminf` calls 10 and 14, unchanged (the
  codegen check's `symbol_correlations` ceiling of 0 holds).
- Hosts' `.text` (one scratch path, aligned): AArch64 `paired_bench`
  1 170 648 → 1 170 584 (ee64690, 5c9977c) → 1 170 904 B, `cat-ft8-decode`
  398 120 → 398 056 → 398 376; x86-64 `paired_bench` 1 412 022 →
  1 410 870 → 1 410 870 → 1 411 190, `cat-ft8-decode` 457 478 → 456 454
  → 456 454 → 456 774.

### Sections of all eight binaries

`.text` and `.rodata` (one scratch path, fresh tree, rustc 1.93.0; thumbv8m
with `--cfg cat_ft8_minmaxnm`):

| binary | ee64690 against 7f4e73f | 5c9977c against ee64690 | eb210f9 against 5c9977c |
|---|---|---|---|
| x86-64 `cat-ft8-decode`, `paired_bench` | differ | `.text` differs (one instruction's place in `fill_phase_only`), `.rodata` identical | differ |
| AArch64 `cat-ft8-decode`, `paired_bench` | differ | identical | differ |
| M33 check, small | identical | differ (the generator) | identical |
| M33 check, host; staged small and host | differ | differ | differ |

c4d5b08 changes only `codegen-check.sh`.

### The code-generation check (c4d5b08)

ee64690's gain on AArch64 rests on the whole lanes staying out of line
(inlined, the sums were split across half vectors and spilled) and its
one `eor` a vector; the M33's gains on the one-lane loop with the sign
on the weight and on the fused generator. Two metrics are added: `memsums`
(loops that load with no floating-point or vector instruction, a ceiling)
and `eors` (`eor`, a ceiling). ee64690 had already replaced the entries of
the removed `whole_lane_sums` (its `.4s` floor and its M33 `fploop2`) and
moved the AArch64 clamp floor and the `cvtloop4` ceiling of the products
from `symbol_correlations` to `correlation_whole`. Each forced form below
is applied to a copy of the work tree with c4d5b08's check (one log each,
the check's own exit status):

| form | a64 `correlation_whole` loop (≤ 140) / `eor` (≤ 8) | m33s products' closure (≤ 37, `one`) | m33s `memsums` `fill_phase_only` / `skip_with` (≤ 0) | exit |
|---|---|---|---|---|
| head | 137 / 7 | 36 | 0 / 0 | 0 |
| f1: AArch64, the sign after the weight | 138 / **14** | 36 | 0 / 0 | 1 |
| f2: the whole lanes inlined | **not found** | 36 | 0 / 0 | 1 |
| f3: the products stored and summed after (7f4e73f's subtract.rs, tests and math.rs) | **not found** | **39** | 0 / 0 | 1 |
| f4: without a vector unit, the sign after the weight | 137 / 7 | **42** | 0 / 0 | 1 |
| f5: without a vector unit, the lanes side by side | 137 / 7 | **69** | 0 / 0 | 1 |
| f6: without a vector unit, the generator's increments stored and summed after | 137 / 7 | 36 | **2 / 2** | 1 |

No other check failed in any form. What it misses: the sign on the weight
on AArch64 is caught by its exclusive ors, not by the loop's length (138
against 137, under the ceiling); one instruction in the M33 loop (36
against a ceiling of 37); x86-64 and the M4F (no bounds); the linked
images (it reads the library at opt-level "s"; the linked images' loops
are counted above); eb210f9 (the mirrored spans) does not rest on code
generation and has no bound.

### Identity

All at eb210f9's binaries (c4d5b08 changes only the check).

- **Records, x86-64:** the 73 800 `paired_bench run` records equal b93516e's
  (host, one pass, small: 24 600 each, 0 differ; the records comparison exits 0
  each).
- **800 WAV slots** × 3 configurations × 3 modes, x86-64 (7 200 runs, 78 690
  decode lines): 93 091 lines byte-identical to b93516e's, and the 2 400
  `--profile` lines differ only by the sync scratch, as at O21 (the WAV
  comparison exits 0).
- **A76:** the records of eb210f9's AArch64 `paired_bench` equal x86-64
  b93516e's (73 800, 0 differ; exit 0 each), and the 800 WAV slots × 3 × 3 with
  its `cat-ft8-decode` give the same 93 091 identical lines and 2 400
  `--profile` lines (exit 0). One benchmark at a time, in fresh directories;
  317 throttle samples, start and end included, all 0x0.
- **Encoder:** the 45 `cat-ft8-encode` WAV files equal 70be604's.
- **Cortex-M33 (emulated):** the small and host dump images on seven slots
  (14 of 14) and the multi-pass residuals of s0086 and s0407 equal 68634cc's
  (the M33 comparison exits 0).
- **Comparators on a forced difference** (copies; each line its own exit
  status): the records comparison 0 unforced, 1 with one record's field
  changed; the WAV comparison 0 unforced, 1 with a decode line's frequency
  one higher, 1 with a `--profile` line's `peak_bytes` one higher; the M33
  comparison 0 unforced, 1 with one bit of one dump flipped (15 identical,
  1 differ); `cmp` of an encoder WAV 0 unforced, 1 with one byte changed.

### Tests and mutants

New: `time_search_correlation_is_the_lanes_sums_in_both_forms` (both forms
of `correlation` against the sums' definition: every first sample modulo
four up to 11, 14 lengths, random and edge phases, audio with the values the
front end treats specially). Extended: the weight test checks both forms of
`cos_sin_turns_signed_as`; `skipped_spans_depend_only_on_the_two_tones`
checks `transition_increment` against the generator's increment at each
span's first sample and the mirror relation for every pair of tones met.
`symbol_correlations` takes no scratch in the tests either.

Mutants in fresh copies of c4d5b08's tree (`cat-ft8` tree af126d4f…),
release library tests (210); the fused generator runs on a host only with
`--cfg cat_ft8_test_group_one` (g1). One log each, the tree, the edit and
cargo's exit status in it; failing tests read from the logs:

| mutant | mode | failing (of 210) |
|---|---|---|
| none (control) | release / g1 | 0 / 0 |
| a1 one lane at a time: the lane's first sample one on | release | 1 |
| a2 one lane at a time: a step of three | release | 1 |
| a3 side by side: the first single sample before the whole lanes dropped | release | 2 |
| a4 side by side: the sine's sums take the cosine | release | 12 |
| a5 side by side: the chains' lanes reversed | release | 2 |
| a6 side by side: the whole lanes after the chains dropped | release | 3 |
| a7 side by side: the first single sample after the whole lanes dropped | release | 2 |
| a8 the bit form: the half turn's sign dropped | release | 13 |
| a9 the select form: the weight's sign inverted | release | 2 |
| a10 the select form: the swap inverted | release | 2 |
| a11 the bit form: the sine not negated | release | 10 |
| b1 fused, first run: the phase stored after its increment | g1 | 9 |
| b2 fused, last run: the previous tone for the next | g1 | 14 |
| b3 fused, last run: the phase not stored | g1 | 29 |
| b4 fused: the phase stored in the phase-only skip (equivalent: working storage) | g1 | 0 |
| b5 b1's site off by one, default build (the fused form is not compiled there) | release | 0 |
| c1 mirror: the two increments swapped | release | 8 |
| c2 mirror: the sum without the two increments | release | 3 |
| c3 mirror: the generator not advanced | release | 9 |
| c4 `transition_increment`: the next pulse one sample on | release | 4 |
| c5 `transition_increment`: the tones swapped | release | 9 |
| c6 the mirror never taken (equivalent: every span generated) | release | 0 |
| c7 the mirrored sum not marked known (equivalent: taken again next time) | release | 0 |

b4, c6 and c7 are equivalent by design (they change only the work); b5 shows
that the default build does not reach the fused form, which is why the
group-one step runs it.

Local CI passes at c4d5b08: fmt; clippy `-D warnings` (workspace, all
targets and features, thumbv8m); workspace, profile, no-std and release
tests with the ignored ones (219, the exhaustive comparisons included); the
group-one step (210 run, 9 ignored); the bulk run; MSRV 1.75; docs with and
without `std`; `check.sh --stack`; the code-generation check (the new bounds
140, 8, 37, 0 and 0). A76 release library tests with the ignored ones,
cross-built at c4d5b08 (`cat-ft8` tree af126d4f…): 218 pass, 0 fail, the
source-tree test skipped, the exhaustive comparisons included; 50 throttle
samples, start and end included, all 0x0.

### Tried, not kept

- **The whole lanes inlined** into the block loop (the first form built):
  on AArch64 the compiler split the four lanes' sums across half vectors
  (`.2s` adds) and spilled them: 180 instructions in the loop of 16
  samples against 137 out of line (forced form f2 above).
- **Eight samples at a time** in the whole lanes (`W` 8 against 16) on
  the A76 (microbenchmark, two alternated cycles, all valid): the time
  search 8.018 cycles a core sample against 16's 7.863, the subtraction
  17.836 against 17.784. 16 is kept (its loop has no constant
  rebroadcasts, 6 loads of constants from the stack and one spill).
- **The hosts' overlap runs summed as taken, four increments at a time
  with the sums within the four written out** (an in-register prefix sum):
  rustc 1.93.0 did not vectorise it on AArch64 (47 scalar instructions
  for four samples, against 34 for eight vectorised increments plus 12
  for four sums) and gave 96 instructions on x86-64; not built further.
  Without `unsafe` intrinsics no form that keeps the increments in vector
  registers through the sum was found; the hosts keep the stored
  increments and the separate sum.
- **The fused generator chosen by a const parameter** (`phases_as::<_,
  FUSED>` called from `phases`): it moved the x86-64 code of
  `fill_phase_only` (register choices, frame 104 → 120 B) though the
  hosts take the unfused form; the form chosen by a module constant
  (`FUSED_SUM`, with the group-one cfg for the tests) leaves AArch64 byte
  for byte and moves one x86-64 instruction.

### What bounds the time search now

- **Cortex-A76:** the whole lanes' loop, 137 instructions (123 vector) a
  block of 16 core samples, about 31 vector operations a vector of four
  (the remainder, the polynomial and its quarter, the clamp, the weight and
  its sign, the two sums); a floor of about 3.8 cycles a core sample on
  two vector pipes. Then the generator over the cores (the overlap runs'
  increments vectorised, their running sum scalar on the hosts) and the
  skipped spans of pairs met first.
- **Cortex-M33** (the library at opt-level "s"): the products and sums 36
  a core sample (the clamp 6, the constants' copies for `vmla` 4, the loop
  4); the generator 17 and 15 a sample in the overlap runs, 5 in the flat
  run.

### Next candidates (not built)

| candidate | estimate | cost |
|---|---|---|
| **The hosts' running sum in vector registers** (an in-register prefix sum fused with the increments) | x86-64: the running sum's scalar add was 17.7% of `fill_phase_only`; A76: the sums 4.7% of the subtraction | no form without `unsafe` intrinsics found (rustc 1.93.0 kept it scalar) |
| **The clamp checked once per subtraction** (if every usable sample is finite and within ±2¹⁶, the clamp is the identity: one pass, then loops without it) | A76: 5 of the removal's and the fit's 15 vector operations per four samples and 5 of the products' 29; M33: 6 a sample in each | a second form of three loops (flash); a pass over the audio |
| **The second time-search step's cores** | as O22 | 303 KB; not revisited |

### Risks

- The A76 and x86-64 gains of ee64690 rest on `correlation_whole` staying
  out of line and on its `eor` count; the M33's on the one-lane loop with
  the sign on the weight and the fused generator: checked (c4d5b08); a
  toolchain change can lose them with every test passing.
- The fused generator runs on a host only in the group-one tests (b5: the
  default tests do not reach it); the M33 identity runs and the group-one
  CI step are its evidence.
- `cos_sin_turns_signed_as` equals `cos_sin_turns_times` for finite weights
  only (as O23b found for the select form); the decoder's weights are the
  clamped input: finite.
- The mirrored spans rest on the pulse's two terms being added in either
  order with the same result (IEEE addition is commutative) and on the span
  being symmetric about the boundary (a `const` assertion); the test checks
  every pair of tones on six bases.
- 5c9977c moves one x86-64 instruction in `fill_phase_only` (not timed
  separately from the round; its x86-64 sample is in the table above).

### Not measured

- Cortex-M33 cycles (emulated instructions only); M33 counts on any slot but
  s0086 and s0407 (seven slots for identity); any M4F count or identity
  (sizes and library calls only).
- Sparse and empty slots on any target (the A76 timing is the crowded
  `main` block; the microbenchmark is one synthesized crowded slot), and
  how the mirrored spans' gain varies with the number of time-search steps
  (the pair counts are over random payloads).
- A placement variation on the A76 (the feed, untouched, read 0.4% faster
  in both new builds; the gains are 5–10 times that).
- The microbenchmark on other A76 cores; the forms in "Tried, not kept"
  beyond the microbenchmark and the library's assembly.
- x86-64 stage times (the virtual machine's clock); x86-64 timing on real
  recordings. The x86-64 profiles are one run each.
- After a toolchain change: the bounds above.

## Review follow-ups (O24b, 2026-11-03; one commit moves two x86-64 instructions, the rest leave every section unchanged)

On top of O24 (184c9c0), one concern per commit: 7176be1, 1292abc, 1a17c90,
d2213d1, 8a0a9d7, f8e1a95 and b81a05c, then this record. Section hashes of
all eight binaries were taken at each commit (one scratch path, rustc
1.93.0, thumbv8m with `--cfg cat_ft8_minmaxnm`): every one equals eb210f9's
except b81a05c's x86-64 `.text`.

- **7176be1: `cos_sin_turns_times_as`' doc back on it.** ee64690 had put
  `reduced` between the doc and the function, so rustdoc joined the doc to
  `reduced`'s.
- **1292abc: one predicate.** `SCALAR_FORMS` (math.rs: `!VECTOR`, or the
  tests built with `--cfg cat_ft8_test_group_one`) now decides the
  reference's group (`GROUP`), the generator's fused overlap runs
  (`FUSED_SUM`) and the time search's sums one lane at a time
  (`correlation`). Before, `GROUP` and `FUSED_SUM` each spelled the test
  cfg and `correlation` keyed on `VECTOR` alone, so the group-one tests ran
  the fused generator with the hosts' time search. The group-one step now
  runs the M33's time-search form end to end: a mutant of the one-lane form
  fails 4 tests there (1 in the default build, the direct test). The const
  parameters for "the form for a vector unit" are all `VECTOR_FORM` (were
  `BITS` and `SIDE_BY_SIDE`). `CHAINS` and `GROUPED_SUM` unchanged.
- **1a17c90: the check's comments.** The time search's four-lane
  `fmaxnm`/`fminnm` count is 14 in `correlation_whole` (8 was
  `symbol_correlations`' before O24); `memsums` counts `ldr` mnemonics only
  (not `ldm`) and is valid for the m33 builds only (it reads "no mnemonic
  starting with v" as "no floating point").
- **d2213d1:** `correlation_whole` asserts (debug builds) that it is given
  whole lanes.
- **8a0a9d7: one expression for the generator's increment**, `step(base,
  dev)`, used by `Synth::phases` and `Synth::transition_increment`. Tried
  first as a method (`self.step(dev)`): the closure in `phases` then
  captured the generator instead of its base frequency and the hosts'
  `.text` changed (x86-64 and AArch64); the free function leaves all eight
  unchanged.
- **f8e1a95:** `cos_sin_turns_signed_as` is `cos_sin_turns_weight_signed_as`,
  its doc's first line saying the weight is signed, not the parts;
  `scaled_cos_sin`'s doc says it serves only the reference.
- **b81a05c: one function for the overlap runs** (`overlap`, holding the
  `FUSED_SUM` choice once; each run passes its two pulse slices in their
  directions and the other tone). AArch64 and thumbv8m sections unchanged,
  `memsums` 0 for `fill_phase_only` and `skip_with`; on x86-64 two `lea`
  instructions move (one in `fill_phase_only`, one in `fill_phase`; no
  other instruction differs once the `.llvm.` symbol suffixes are set
  aside). A first form passing one iterator of pulse pairs changed every
  binary and was not kept.

### Measured, not adopted

- **The reference's vector form with the sign on the weight** (the
  BITS arm of `cos_sin_turns_times_as` replaced by
  `cos_sin_turns_weight_signed_as::<true>`). AArch64 blocks of 16
  (library, rustc 1.93.0): with an envelope 108 / 111 → 104 / 108, but
  without one (all but the ramps' blocks) 100 / 101 → 102 / 104: with a
  weight of 1.0 the parts need no product, and the signed form needs one
  by ±1. A76 microbenchmark (two alternated cycles, all valid): the
  reference (`F2`) 8.006 → 8.171 cycles a sample (+2.1%), the subtraction
  17.550 → 17.683 (+0.8%). Not adopted; the BITS arm stays.
- **A skipped span's sum filling its mirrored pair's when taken** (built
  as a trial commit, not kept): the third branch goes,
  but every generated span whose mirror is never met now costs two
  increments. M33 emulated instructions: the subtraction +10 579 (s0086)
  and +9 298 (s0407), +0.0014%; every other stage the same; decodes the
  same. A76 (`slotcost`, four alternated cycles against eb210f9, all 8 runs valid): the subtraction 38.505 → 38.500 ms (spreads 0.030 and 0.020), the slot 84.30 both; the microbenchmark's time search 7.444 → 7.467 cycles a core sample, within the 1.3% its untouched generator parts moved in the same binaries (placement). x86-64: −0.24% user cycles, within the ranges (below), instructions +0.03 G. Not adopted (more work, if
  very little); eb210f9's form stays.

### x86-64 (b81a05c; core 5, `perf stat` user counters, `slotcost host host,passes=1 200 5`, four alternated rounds, none void)

| build | cycles (median, range) | instructions |
|---|---|---|
| eb210f9 (the sections of 7176be1 to f8e1a95) | 314.33 G (313.81–314.74) | 891.36 G |
| b81a05c | 314.46 G (314.06–314.72), +0.04% | 891.36 G |
| the mirrored fill (not adopted) | 313.58 G (312.61–314.28), −0.24% | 891.39 G |

b81a05c's two moved instructions cost nothing measurable (within eb210f9's
range, instructions equal). eb210f9 reads 314.33 G here against 313.81 G in
O24's session: the machine's drift between sessions; within a session the
builds alternate.

### Identity and checks (b81a05c)

- x86-64: the 73 800 records equal b93516e's (exit 0 each); the 800 WAV
  slots × 3 × 3 give the 93 091 identical lines and the 2 400 `--profile`
  lines of O21's rule (exit 0).
- AArch64 and thumbv8m: the sections equal eb210f9's, whose A76 identity
  (records and WAVs) and M33 identity O24 recorded. The M33 dump and
  residual images were rebuilt and run at b81a05c anyway (16 of 16 equal
  68634cc's, exit 0), for the comparators' forced run.
- Encoder: the 45 WAV files equal 70be604's.
- Comparators: unforced 0; a record's field, a decode's frequency, a
  `--profile` line's `peak_bytes`, one M33 dump bit and one encoder byte
  each give 1. (A first forced run had no M33 outputs at b81a05c yet and
  its M33 lines failed for that reason; rerun after them.)
- Local CI passes at b81a05c: fmt; clippy `-D warnings` (workspace, all
  targets and features, thumbv8m); workspace, profile, no-std and release
  tests with the ignored ones (219); the group-one step (210 run, 9
  ignored), which now runs the time search one lane at a time; the bulk
  run; MSRV 1.75; docs with and without `std`; `check.sh --stack` (flash
  and frames as at eb210f9); the code-generation check (137/140, 7/8,
  36/37, `memsums` 0 and 0). A76 release library tests with the ignored
  ones, cross-built at b81a05c (`cat-ft8` tree 71dedf67…): 218 pass, 0
  fail, the source-tree test skipped; 50 throttle samples, all 0x0.
- The A76 runs of this round (the two measurements above, 8 timed runs and
  12 microbenchmark runs): every throttle sample 0x0 (976 during the timed
  runs and 40 in their warm-ups, 96 in the microbenchmarks), every sample during a run at
  2 400 MHz; peak 54.3 °C.

### Mutants

Fresh copies of b81a05c's tree (`cat-ft8` tree 71dedf67…), release
library tests (210), g1 with `--cfg cat_ft8_test_group_one`; one log each
with the tree, the edit and cargo's exit status:

| mutant | mode | failing |
|---|---|---|
| none (control) | release / g1 | 0 / 0 |
| one lane at a time: the lane's first sample one on | release / g1 | 1 / **4** |
| the select form: the weight's sign inverted | g1 | 3 |
| `SCALAR_FORMS` without the test cfg | g1 | 1 (`the_group_follows_the_target_and_the_test_cfg`) |
| `overlap`, fused: the phase stored after its increment | g1 | 7 |
| `overlap`, fused: the phase not stored | g1 | 41 |
| `overlap`, stored: not summed | release | 32 |
| the last run with the previous tone | release / g1 | 14 / 14 |
| the first run's previous pulse not reversed | release | 15 |
| `transition_increment`'s `step` without the base | release | 4 |

The first mutant's g1 count (4, against 1 in the default build, where only
the direct test reaches the one-lane form) is 1292abc's point. A first run
of this set reused stale test binaries: the copies' files carried the
commit's time, older than an earlier run's builds in the shared target
directory, so cargo rebuilt nothing for some and both controls failed.
The script now gives each copy fresh times; the run above is the rerun
(its controls 0). O24's mutant run was not affected: every one of its logs shows
`cat-ft8` compiled, and a mutation's edit gives the edited file a fresh
time, so only unedited copies (the controls) could have run a stale
binary; O24's controls read 0 with the binary rebuilt.

### Not measured

- As in O24. b81a05c's AArch64 and thumbv8m sections equal eb210f9's, so
  O24's A76 and M33 timing, counts and identity stand for them; only its
  x86-64 code was measured again.
- The reference's signed form was measured by the library's assembly and
  the microbenchmark only (no `slotcost`, no M33 counts: its select form
  was unchanged).

## O24c: closure follow-ups (no code generation change)

- The reference's polynomial form follows `SCALAR_FORMS`, like the time
  search's, so the group-of-one tests run the select form there too; no
  shipped binary changes (`SCALAR_FORMS` is `!VECTOR` outside tests).
- `Correction::apply_as`'s doc compares against `SCALAR_FORMS`; the
  generator's closure no longer shares the name `step`; `Cargo.toml` says
  what the group-of-one cfg now switches.
- The O24b record no longer cites the trial commit of the mirrored fill,
  and gives the right reason O24's mutant run read current binaries.
- All 8 binaries' `.text` and `.rodata` identical to b81a05c's (thumbv8m
  with `--cfg cat_ft8_minmaxnm`). Local tests: debug 244, release 242, the
  group-one step 210 run, 9 ignored; clippy and the code-generation check
  pass.

## A whole-slot profile; the input checked once, and the M33's Costas sums and min-sum, twenty-fifth round (O25, bit-identical, 2026-11-03)

Base: b335b0a (the code of O24c). After O22–O24 cut the A76's subtraction
from 43.7 to 38.5 ms, about half of the crowded host slot was elsewhere. The
whole slot was profiled first, per stage and per function, on all three
targets; the ranking below chose the round's changes. Three code commits, one
optimisation each: da69ed4 (the subtraction's input checked once, its loops
without the clamp when every sample is within the limit), 831e9a5 (the sync
search's Costas sums by running sums along the bins without a vector unit)
and 6f89e49 (min-sum's inputs taken in its magnitude loop without a vector
unit). Each carries its own tests and code-generation bounds. Decodes,
records, residuals and encoder output are identical. No memory is added:
static RAM, the slot's storage and the scratch are unchanged; the M33 stack's
high water rises by 32 B; flash grows on the host image (below).

| target | measure | b335b0a | 6f89e49 | change |
|---|---|---|---|---|
| Cortex-M33, s0086 host | slot, emulated instructions (setup excluded) | 1 519.83 M | 1 437.85 M | **−5.40%** |
| | subtraction, a call | 20.474 M | 19.003 M | −7.18% |
| | sync search (three passes) | 91.96 M | 64.39 M | −30.0% |
| Cortex-M33, s0086 small | slot | 183.02 M | 172.57 M | **−5.71%** |
| Cortex-M33, s0407 host / small | slot | 1 516.38 / 175.80 M | 1 434.58 / 165.67 M | −5.40% / −5.76% |
| Cortex-A76, crowded host slot | subtraction (medians of 4) | 38.515 ms | 35.075 ms | **−8.93%** |
| | slot | 84.30 ms | 80.80 ms | **−4.15%** |
| x86-64, `slotcost host host,passes=1 200 5` | user cycles, the whole program (medians of 4) | 313.56 G | 308.85 G | **−1.50%** |

The M33's small slot gains from the sync search (831e9a5) and min-sum
(6f89e49), its host slot from the subtraction (da69ed4) and the sync
search; the hosts' code changes only in da69ed4 (the A76 and x86-64 figures
are da69ed4's binaries, whose sections 6f89e49's equal).

### The profile at b335b0a

#### Method

- **One build per target** of b335b0a from one scratch path (aligned hosts,
  `-align-loops=64 -align-all-functions=6`; thumbv8m with `--cfg
  cat_ft8_minmaxnm`), rustc 1.93.0. The scratch path differs from O24's,
  so the hashes are not comparable with O24's (path strings), but the
  `.text` and `.rodata` sizes of all eight binaries equal b81a05c's (O24b),
  and the M33 counts equal O24's to the instruction (s0086 host 1 519.83 M,
  s0407 1 516.38 M).
- **A76 stages** (Raspberry Pi 5, core 3, one benchmark at a time, the
  throttle, clock and temperature sampled once a second from core 0, a 5 s
  sampled busy loop first): four cycles of `slotcost host host,passes=1 200
  5` and `slotcost small small 200 5`; medians of the four runs.
- **A76 and x86-64 PC samples by stage.** An out-of-tree sampler in a
  scratch copy of `paired_bench` (a `SIGPROF` timer every 100 µs, 50 µs for
  small; each sample keeps its PC and the number of the decoder's step it
  fell in, the step named by the staged callback that ends it), over
  `slotcost host host 200 5` and `slotcost small small 200 5`: 1 726 579 and
  535 547 samples in the decoder's steps on the A76, the bench's own work
  (slot synthesis, outcome classification) left out by label. Loops are
  attributed by their backward branches. On x86-64 (a virtual machine whose
  clock source is `acpi_pm`) the bench's clock reads between steps run in
  the kernel; their samples (14.3% host, 22.2% small, almost all in the
  short steps: refine, soft bits, SNR) are left out of the x86-64 shares
  below. The A76's clock is read in user space (0.5% outside the binary).
- **x86-64** (Threadripper 3970X, core 5, nothing else of this round
  running; checked before and after each run): the sampler above, and
  `perf record -e cycles:u` of the same two programs for the function
  ranking.
- **Cortex-M33** (emulated instructions, unicorn; the staged images of
  O13–O24): s0086 and s0407, host and small, per stage, per function within
  each stage, and per basic block (small, s0086).

#### Crowded slots, by stage

| stage | A76 host (ms) | share | A76 small (ms) | share | x86-64 host | x86-64 small | M33 host s0086 (M) | share | M33 small s0086 (M) | share |
|---|---|---|---|---|---|---|---|---|---|---|
| subtract | 38.505 | 45.7% | — | — | 41.8% | — | 757.54 | 49.8% | — | — |
| feed | 19.120 | 22.7% | 6.355 | 49.6% | 20.5% | 50.1% | 272.84 | 18.0% | 89.14 | 48.7% |
| belief propagation | 19.050 | 22.6% | 4.020 | 31.4% | 26.2% | 21.8% | 365.39 | 24.0% | 54.26 | 29.6% |
| sync | 4.680 | 5.6% | 1.380 | 10.8% | 6.6% | 14.6% | 91.96 | 6.1% | 27.47 | 15.0% |
| soft bits | 2.380 | 2.8% | 0.890 | 7.0% | 3.8% | 11.1% | 24.72 | 1.6% | 9.29 | 5.1% |
| SNR, OSD-1, refine | 0.530 | 0.6% | 0.180 | 1.4% | 1.2% | 2.4% | 7.24 | 0.5% | 2.77 | 1.5% |
| **slot** | **84.30** | | **12.80** | | | | **1 519.83** | | **183.02** | |

(The x86-64 columns are shares of the sampler's decoder samples, the clock
reads left out; the A76 PC samples give the A76 times' shares within 0.2
points. A76 one pass: 18.30 ms. M33 s0407: host 1 516.38 M, the same shares
within 0.1 points; small 175.80 M: feed 50.7%, belief propagation 26.3%,
sync 15.6%, soft bits 5.3%.)

#### Inside the stages

**A76, host** (PC samples, share of the decoder's samples):

| where | share | what bounds it |
|---|---|---|
| subtraction: `run` (the reference's block of 16 samples 9.4%, the frequency correction's lanes 4.6%) | 15.3% | the reference: O23's polynomial; the correction: 24 integer instructions a group of four samples, scalar (the four 64-bit lanes were not vectorised) |
| subtraction: `fill_phase_only` | 8.3% | the generator (O24: no form without `unsafe` intrinsics) |
| subtraction: `Fit4::add_all` (its loop 20 instructions a group of four, 15 vector) | 6.5% | 5 of the 15 are the clamp |
| subtraction: `correlation_whole` (140 a block of 16, 124 vector) | 6.2% | 20 of the 124 are the clamp |
| subtraction: `Removal::refs` (21 a group of four, 15 vector) | 6.1% | 5 of the 15 are the clamp |
| subtraction: `score`, `skip_with` | 1.7%, 1.1% | |
| feed: `power_bins` | 21.4% | the last stage (radix 5) 6.0%: 150 instructions a step, 63 vector of which about 37 move lanes to gather its twiddles (stride m in the table); the first stage (window, digit-reversed loads) 3.3%; no other loop over 2.6% |
| belief propagation: `sum_product_round` | 21.5% | its tanh and atanh passes (263 and 240 instructions a loop) 15.4%, the inputs' gather 2.6%, the sums 2.1% |
| sync: the Costas sums (`select`'s closure; 41 instructions a vector of 16 bins) | 3.2% | vectorised byte sums |
| sync: `search_rows` (the local maxima, branches) | 2.3% | |
| soft bits: `soft_bits_at` | 2.8% | spread; no loop over 0.3% |

**A76, small:** `power_bins` 46.5% (the radix-5 stage's loop 13.1%, the
first stage 7.4%), `min_sum_round` 28.6% (its loops over groups of four
checks 15.3% and 7.8%, the sums 4.4%), `soft_bits_at` 7.0%, the Costas sums
6.3%, `search_rows` 4.4%, `decode_soft` 2.5%, `Spectrogram::feed` 2.4%.

**x86-64** (sampler, clock reads left out): host `sum_product_round` 24.8%,
`power_bins` 19.7%, `fill_phase_only` 12.8%, `run` 12.3% (the correction's
lanes, 23 integer instructions, a quarter of it), `correlation_whole` 4.8%,
`Removal::refs` 4.3%, the Costas sums 4.0%, `Fit4::add_all` 3.7%,
`soft_bits_at` 3.5%, `search_rows` 2.6%; small `power_bins` 47.9% (the radix-5
loop a quarter), `min_sum_round` 18.0%, `soft_bits_at` 10.9%, the Costas sums
8.5%, `search_rows` 6.1%, `decode_soft` 3.6%. `perf record` agrees within a
point for the decoder's functions; it also counts the bench's slot synthesis
(`Synth::mix`, `fill_phase`, the noise: 5–20%) and its outcome
classification (`costas_score` 5.8% of small), which the sampler leaves out.

**Cortex-M33** (s0086, emulated instructions, share of the slot): host:
`subtract::blocks` (the reference and the correction) 16.3%, `Removal::refs`
9.6%, `Fit4::add_all` 7.7%, `fill_phase_only` 7.1%, the products' closure
6.8%, `sum_product_round` 23.7%, the feed (`Decoder::pass`, the transform
inlined) 17.4%, the Costas sums 5.0%, `Decoder::search` 1.0%. Small: the feed
47.1%, `min_sum_round` 28.5%, the Costas sums 12.4% (**one block of 24
instructions a bin, 931 392 times: 22.35 M, the slot's largest block**),
`Decoder::search` 2.4%, soft bits 5.1%. The clamp is about 6 of the
removal's, the fit's and the products' instructions a sample. Min-sum takes
each check's inputs in one loop (7 an edge) and their magnitudes in a second
(22–25).

#### Ranked (bit-identical candidates by gain on each target)

| target | candidate | cost now | estimate | outcome |
|---|---|---|---|---|
| A76, x86-64, M33 | **the clamp checked once per subtraction** (O24's next candidate) | the clamp: about a third of the fit's and the removal's vector operations, a sixth of the products'; M33 6 a sample in each | A76 −10% of the subtraction less the check (−4% of the slot); M33 −2.5 M a call less the check | **built, da69ed4** |
| M33 | **the Costas sums by running sums** (prof19's candidate 4) | 22.35 M, 12.2% of the small slot; 75.7 M host | −35 to −40% of the loop: small −4 to −5% | **built, 831e9a5** |
| M33 | **min-sum's inputs in its magnitude loop** | the inputs' loop, 7 an edge | −2 to −3 an edge: small −1% | **built, 6f89e49** |
| A76, x86-64 | the radix-5 stage's twiddle gathers (lane moves) | A76 6.0% host, 13.1% small; x86-64 a quarter of `power_bins` | up to half of the loop: small −5%, host −2.5% | not built: the gathers are the compiler's lowering of strided table reads; a layout read whole needs a copy of the twiddles per stride (46 KB more memory), and forms it might lower to whole-vector loads were not tried |
| A76, x86-64 | the correction's 64-bit lanes in vector registers | A76 4.6% host (24 integer instructions a group of four) | 1.5–2 cycles a group of four as 32-bit halves with carries: −1% host | not built (small; two compare-and-carry steps a lane a step) |
| A76, x86-64 | the sync search's local maxima (`search_rows`) | A76 2.3% host, 4.4% small | uncertain | not built |
| all | belief propagation's passes, the reference, the generator | 21.5%, 9.4%, 8.3% (A76 host) | none bit-identical found beyond O17, O23 and O24 | — |

On the A76 the subtraction remains the only stage where a bit-identical
change gains over 2% of a crowded slot; the feed (the FFT) and belief
propagation are each about a fifth of it, with no bit-identical lever found
beyond the twiddle gathers above. On the M33 small slot, the feed (48.7%) is
O20's butterflies, and belief propagation and the sync search were the
levers.

### Changes

1. **da69ed4: the subtraction's input checked once, loops without the
   clamp.**
   - The subtraction reads each sample as the front end takes it
     (`input`: a non-finite value as 0.0, a finite one beyond ±2¹⁶
     clamped), in the time search's products, the fit and the removal: on
     the A76 5 of the fit's and the removal's 15 vector operations a group
     of four and 20 of the products' 124 a block of 16; on the M33 about 6
     instructions a sample in each.
   - `run` now checks the usable audio once a call (`within_limit`). Where
     every sample is finite and within the limit, `input` is the identity
     on them, so the three loops take a second form without it
     (`input_as::<IN_LIMIT>`: `correlation`, `correlation_whole`,
     `Fit4::add_all_as`, `Removal::run_as`, `remove_one`); otherwise the
     clamped forms run as before. Each sample the subtraction reads is read
     before it writes it, so one check holds for every read. The result is
     the same bit for bit either way.
   - The check: the samples' bits without the sign, as unsigned integers,
     order the finite magnitudes and put the infinities and NaNs above
     them. Their bitwise or is at least the largest and, for audio within
     ±1, below the limit's bits, so one pass decides; otherwise the largest
     decides in a second pass. On AArch64 the first pass is 8 samples a
     loop of 6 instructions (`orr` on two vectors); on the M33 (opt-level
     "s") 4 instructions a sample. Not inlined, so its loops are checked
     on their own.
   - Loops (rustc 1.93.0, the library): AArch64 the whole lanes 107
     instructions a block of 16 without the clamp (137 with it), the fit 15
     a group of four, 10 of them vector (20 and 15); M33
     (opt-level "s") the products 30 a core sample (36), the fit 14 (20),
     the removal 20 unreported (26) and 27 reported (33).
   - Flash grows by the second forms: M33 host image +1 800 B `.text`;
     the hosts' `.text` +8.2 KB (x86-64) and +6.3 KB (AArch64).
2. **831e9a5: the Costas sums by running sums without a vector unit.** The
   sync search adds, for every base bin of a Costas symbol's row,
   8 · the expected tone's level − the eight tones' levels. The eight-tone
   sum at bin b is the one at b − 2 less tone 0's level there and plus
   tone 7's at b, so on the `SCALAR_FORMS` side (`add_row_running`) the
   sums run along the even and the odd bins: three byte loads a bin
   instead of nine, 15 instructions instead of 24 (M33, opt-level "s").
   Integers: the same scores. Hosts keep each bin's eight tones added
   afresh (it vectorises); their binaries are unchanged. Tried first: two
   bins at a time over `chunks_exact` pairs (22 instructions a bin: the
   remainders' bounds checks stayed in the loop) and `step_by` iterators
   (37); the index form is kept.
3. **6f89e49: min-sum's inputs in its magnitude loop without a vector
   unit.** `min_sum_check` took a check's inputs (the variable's total
   less the message) in one loop and their magnitudes, signs and two
   smallest in a second. With `ONE_PASS` (the decoder's `VECTOR` false)
   each input is taken in the second loop, stored as before and then
   compared: 32 instructions an edge instead of 10 + 25 (M33, opt-level
   "s"). Hosts keep the two loops (a macro holds the shared body, so their
   code is unchanged: a first form with one loop shape for both changed the
   hosts' binaries).

### Cortex-M33 (emulated instructions; staged images as built, rustc 1.93.0, `--cfg cat_ft8_minmaxnm`)

| commit | s0086 host slot | subtraction (37 calls) | s0086 small slot | s0407 host slot | s0407 small slot |
|---|---|---|---|---|---|
| b335b0a | 1 519 833 890 | 757 539 071 | 183 016 805 | 1 516 383 326 | 175 802 921 |
| da69ed4 | 1 465 413 560 (−3.58%) | 703 118 741 (−7.18%) | 183 016 805 (0) | 1 462 146 854 (−3.58%) | 175 802 921 (0) |
| 831e9a5 | 1 437 845 369 (−1.88%) | 703 118 741 | 174 758 800 (−4.51%) | 1 434 578 663 (−1.89%) | 167 544 916 (−4.70%) |
| **6f89e49** | **1 437 845 369 (−5.40% in all)** | **703 118 741 (−7.18%)** | **172 570 220 (−1.25%; −5.71% in all)** | **1 434 578 663 (−5.40%)** | **165 674 168 (−1.12%; −5.76% in all)** |

- A subtraction on s0086: 20.474 → 19.003 M. Per decode, host s0086 38.0 →
  35.9 M; small 7.96 → 7.50 M (s0086, 23 decodes).
- Same decodes (40 and 38 host; 23 and 30 small). Every other stage's count
  is the same to the instruction: da69ed4 changes only the subtraction,
  831e9a5 only the sync search (−27 568 191 host, three passes; −8 258 005
  small), 6f89e49 only small's belief propagation (−2 188 580 and −1 870 748;
  the host decodes by sum-product). `setup` unchanged.
- Functions (s0086 host, M): the fit 116.55 → 85.08, the removal 145.63 →
  112.10, the products' closure 102.82 → 86.15, the check (`within_limit`,
  new) 26.64 (0.72 M a call: 4 a sample over the 180 000); the Costas sums
  75.67 → 48.10 (831e9a5); `min_sum_round` (small) 52.25 → 50.06 (6f89e49).
- The trial builds these were counted on have the commits' sections (each
  of the eight binaries' `.text` and `.rodata` equal), and the commits'
  own images were counted again: the same counts.
- Paint high water: host 10 244 → 10 276 B (da69ed4), small 10 304 B
  unchanged.

### Cortex-A76 (Raspberry Pi 5; `slotcost host host,passes=1 200 5`, medians of 4 alternated runs)

Aligned builds of b335b0a and da69ed4 from one scratch path (831e9a5's and
6f89e49's AArch64 sections equal da69ed4's), core 3, the throttle, clock and
temperature sampled once a second from core 0, a 5 s sampled busy loop on
core 3 before each run, alternated (forward, then reversed), four cycles.

| ms per slot (median, spread) | b335b0a | da69ed4 |
|---|---|---|
| **subtract** | 38.515 (0.040) | **35.075 (0.010): −3.440, −8.93%** |
| pass 2 / pass 3 subtract | 26.825 / 11.680 | 24.435 / 10.640 |
| slot | 84.30 | **80.80 (−4.15%)** |
| worst slot | 89.40 | 85.50 |
| feed (placement check) | 19.125 (0.020) | 19.100 (0.010) |
| belief propagation / sync / soft bits / SNR / OSD-1 / refine | 19.050 / 4.680 / 2.380 / 0.240 / 0.200 / 0.090 | 19.060 / the same |
| one pass: slot, feed | 18.30, 6.460 | 18.30, 6.450 |

- **Placement check.** The untouched stages agree within 0.025 ms (the feed,
  0.13%); the subtraction's gain is about 140 times that and 86 times its
  spread. No placement variation was run.
- A trial build with the check by the largest magnitude alone (inlined; the
  same forms otherwise) read 35.105 ms in an earlier session of four
  alternated cycles (base 38.520): the A76 is indifferent to the check's
  form (both vectorise); the M33 is not (below, "Tried").
- **Validity.** All 8 timed runs are valid: 964 samples during the runs
  and 40 in the warm-ups, every one 0x0, the 8 end checks included; every
  sample during a run at 2 400 MHz; one warm-up sample at 1.6 GHz (the
  first). Peak 53.8 °C. The profile's 10 runs and the trial session's 8
  likewise (894 and 964 samples, all 0x0).

### x86-64 (Threadripper 3970X, core 5, `perf stat` user counters over the whole `slotcost` program)

Aligned builds of b335b0a and da69ed4 (831e9a5's and 6f89e49's x86-64
sections equal da69ed4's) from the same scratch path, four alternated rounds
(forward, then reversed), `slotcost host host,passes=1 200 5`; a check for
other jobs of the round (builds, emulators, comparators, the A76 drivers)
before and after each sample: none void.

| build | cycles (median, range) | instructions | subtract stage (ms) |
|---|---|---|---|
| b335b0a | 313.56 G (312.59–314.96) | 891.36 G | 22.435 |
| da69ed4 | **308.85 G (307.69–309.57), −1.50%** | 870.70 G (−2.32%) | 21.210 (−5.5%) |

da69ed4's range clears the base's by 3.0 G. Stage times are the virtual
machine's clock, for scale only. The whole program includes the bench's
slot synthesis and the one-pass configuration (no subtraction).


### Sizes and stack (`check.sh --stack`, thumbv8m)

| image | flash b335b0a → da69ed4 → 831e9a5 → 6f89e49 (B) | static RAM (B) |
|---|---|---|
| small, small with OSD on | 91 982 → 91 982 → 91 966 → **91 954 (−28)** | 201 668 (unchanged) |
| host, host with OSD off | 102 954 → 104 754 → 104 738 → **104 726 (+1 772)** | 230 084 (unchanged) |
| encoder only | 35 186 → 35 186 → 35 190 → 35 190 (+4: `.rodata`, its `.text` unchanged) | 0 |

- Frames: host `subtract_one` 4 920 → 4 928 B (da69ed4); `_start` 4 104,
  the subtraction's closure 2 528, `decode_a_slot` 2 520 B unchanged. No
  scratch or storage added: `SUBTRACT_STACK_BYTES` and
  `SUBTRACT_SCRATCH_BYTES` unchanged. Paint high water above (+32 B host).
- No fused multiply-add (0 `vfma`/`vfms`/`vfnma`/`vfnms`); `vmla` small 128
  and encoder 17 unchanged, host 217 → 241 (da69ed4: the forms without the
  clamp). No `f64` helper in any image.
- Cortex-M4F (thumbv7em, embedded-check images, no cfg): flash small 92 248
  → 92 248 → 92 228 → 92 220 B, host 103 400 → 105 348 → 105 328 → 105 320
  B; static RAM unchanged; no `f64` helper, no fused multiply-add;
  `fmaxf`/`fminf` calls 10 and 14, unchanged.
- Hosts' `.text` (one scratch path, aligned): x86-64 `cat-ft8-decode`
  456 774 → 464 966 B, `paired_bench` 1 411 190 → 1 419 254; AArch64
  `cat-ft8-decode` 398 376 → 404 712, `paired_bench` 1 170 904 → 1 177 240
  (da69ed4; unchanged after).

### Sections of all eight binaries

| binary | da69ed4 against b335b0a | 831e9a5 against da69ed4 | 6f89e49 against 831e9a5 |
|---|---|---|---|
| x86-64 and AArch64 `cat-ft8-decode`, `paired_bench` | differ | identical | identical |
| M33 check, small | identical | differ | differ |
| M33 check, host; staged small and host | differ | differ | differ |

### The code-generation check (in each commit)

da69ed4's gains rest on the check's first pass vectorising on the hosts
and costing four instructions a sample on the M33, and on the three loops'
forms without the clamp; 831e9a5's on the running sums' loop (M33). New
metrics: `shortloop` (the function's shortest loop, a ceiling) and `vecor`
(`orr` on vector registers, a floor); `sumloop` takes its byte loads from
its name (`sumloop3`). The fit's checks follow its renamed function
(`Fit4::add_all_as`); the time search's `eor` ceiling covers its two forms
(16). Each forced form below is applied to a copy of 6f89e49's tree with
its own check (one log each, the check's own exit status):

| form | a64 `correlation_whole` loop (≤ 110) | a64 `within_limit` `vecor` (≥ 2) | m33s `within_limit` `shortloop` (≤ 4) | m33s products / fit / removal loops (≤ 31 / 15 / 21) | m33s `select` `sumloop3` (≤ 16) | exit |
|---|---|---|---|---|---|---|
| head (6f89e49) | 107 | 4 | 4 | 30 / 14 / 20 | 15 | 0 |
| g1: no form without the clamp (`in_limit` false) | **137** | **not found** | **not found** | 30 / 14 / 20 | 15 | 1 |
| g2: the first pass with an early exit (`all`) | 107 | **0** | **6** | 30 / 14 / 20 | 15 | 1 |
| g3: the largest magnitude alone | 107 | **0** | **7** | 30 / 14 / 20 | 15 | 1 |
| g4: the Costas sums' vector form on every target | 107 | 4 | 4 | 30 / 14 / 20 | **no loop** | 1 |
| g5: the forms without the clamp keep it | **137** | 4 | 4 | **36 / 20 / 26** | 15 | 1 |

No other check failed in any form. g1 leaves the M33 loops (at opt-level
"s" the flag reaches them through calls that are not inlined, so both
forms are still compiled); g5 is their revert. What it misses: the x86-64
and M4F code (no bounds); the linked images (it reads the library at opt-level
"s"); min-sum's one pass (6f89e49), whose loop is a source-level fusion
with no bound (32 instructions an edge against 10 + 25; a revert is a
second loop that the shortest-loop metrics would not tell from the
round's other loops).

### Identity

The references are b93516e's outputs (x86-64 records and WAV lines, the
A76's compared with them) and 68634cc's (M33 dumps and residuals), which no
round since has changed, b335b0a included: equal to them is equal to
b335b0a's. The x86-64 and A76 runs were made at 6f89e49 only and stand for
da69ed4 and 831e9a5 as well, whose x86-64 and AArch64 sections equal
6f89e49's (below); each commit's thumbv8m sections differ and the M33 runs
were made at each.

- **Records, x86-64:** the 73 800 `paired_bench run` records of 6f89e49
  equal b93516e's (host, one pass, small: 24 600 each, 0 differ; the records
  comparison exits 0 each).
- **800 WAV slots** × 3 configurations × 3 modes, x86-64 (7 200 runs, 78 690
  decode lines): 93 091 lines byte-identical to b93516e's, and the 2 400
  `--profile` lines differ only by the sync scratch, as at O21 (the WAV
  comparison exits 0).
- **A76:** the records of 6f89e49's AArch64 `paired_bench` equal x86-64
  b93516e's (73 800, 0 differ; exit 0 each), and the 800 WAV slots × 3 × 3
  with its `cat-ft8-decode` give the same 93 091 identical lines and 2 400
  `--profile` lines (exit 0). One benchmark at a time, in fresh directories;
  1 548 throttle samples, once a second, start and end included, all 0x0.
- **Encoder:** the 45 `cat-ft8-encode` WAV files of each commit equal
  70be604's.
- **Cortex-M33 (emulated):** at each of da69ed4, 831e9a5 and 6f89e49 the
  small and host dump images on seven slots (14 of 14) and the multi-pass
  residuals of s0086 and s0407 equal 68634cc's (16 of 16; the M33
  comparison exits 0 each).
- **Comparators on a forced difference** (copies; each line its own exit
  status): the records comparison 0 unforced, 1 with one record's field
  changed; the WAV comparison 0 unforced, 1 with a decode line's frequency
  one higher, 1 with a `--profile` line's `peak_bytes` one higher; the M33
  comparison 0 unforced, 1 with one bit of one dump flipped (15 identical, 1
  differ); `cmp` of an encoder WAV 0 unforced, 1 with one byte changed.

### Tests and mutants

New: `within_limit_is_where_input_is_the_identity` (the check against
`input`'s identity over the values the front end treats specially and
their neighbours, every exponent with random mantissas and signs, and one
sample out of the limit at each place in slices of 13 lengths) and
`a_sample_out_of_the_limit_anywhere_is_taken_as_the_front_end_takes_it`
(one NaN, infinity or −2¹⁷ at the usable audio's first sample, inside the
signal, after it and at the slot's last sample with a signal cut short:
the same residual and report as the audio with that sample as the front
end takes it). Extended: the time search's correlation, its blocks, the
fit and the removal (and its zero-sign test) in the forms without the
clamp on the audio as the front end takes it; the Costas row sums in both
forms on every band down to the narrowest (nine base bins).

Mutants in fresh copies of 6f89e49's tree (`cat-ft8` tree 914795c8…),
release library tests (212); g1
with `--cfg cat_ft8_test_group_one`. One log each with the tree, the edit
and cargo's exit status; failing tests read from the logs:

| mutant | mode | failing (of 212) |
|---|---|---|
| none (control) | release / g1 | 0 / 0 |
| a1 the check's second pass: `<` for `<=` | release | 1 |
| a2 the check's first pass: `<` for `<=` (equivalent: the second pass decides) | release | 0 |
| a3 the check's second pass with the sign kept | release | 2 |
| a4 the check always true | release | 2 |
| a5 the check inverted | release | 2 |
| a6 the check from the second usable sample | release | 1 |
| a7 the check to the last sample but one | release | 1 |
| a8 the time search's clamped form within the limit (equivalent: work only) | release | 0 |
| a9 the time search's form without the clamp out of the limit | release / g1 | 2 / 2 |
| a10 the fit's form without the clamp out of the limit | release | 3 |
| a11 the removal's form without the clamp out of the limit (reporting) | release | 2 |
| a12 `input_as` inverted | release | 5 |
| a13 the fit's single samples without the clamp | release | 2 |
| b1 running sums: the leaving level one bin on | release / g1 | 1 / 3 |
| b2 running sums: tone 6's level entering | release | 1 |
| b3 running sums: seven tones to start | release | 1 |
| b4 running sums: one bin past the end | release | 1 |
| b5 running sums: the even bins only | release | 1 |
| b6 running sums on the hosts too (equivalent: the same integers) | release | 0 |
| c1 min-sum in one pass: the message taken before the input | release | 1 |
| c2 min-sum in one pass: the input not stored | release | 1 |
| c3 the scalar form in two passes (equivalent: work only) | release | 0 |

a2, a8, b6 and c3 are equivalent by design (they change only the work).

### Checks

- **Local CI** passes at 6f89e49: fmt; clippy `-D warnings` (workspace, all
  targets and features, thumbv8m); workspace, profile, no-std and release
  tests with the ignored ones (221, the exhaustive comparisons included);
  the group-one step (212 run, 9 ignored), which runs the time search one
  lane at a time and the running Costas sums end to end; the bulk run; MSRV
  1.75; docs with and without `std`; `check.sh --stack`; the
  code-generation check (above). The check passes at 6d6ba7b too (comments
  only after 6f89e49).
- **A76 release library tests** with the ignored ones, cross-built at
  6f89e49 (`cat-ft8` tree 914795c8…): 220 pass, 0 fail, the
  source-tree test skipped, the exhaustive comparisons included; 241
  throttle samples, once a second, start and end included, all 0x0.

### Tried, not kept

- **The check by the largest magnitude alone** (the first form built; it
  was also inlined into `run`): on the M33 a loop of 7–8 instructions a
  sample (compare and conditional move), 53.4 M a host slot, which took
  half of the subtraction's gain (−3.67% a call instead of −7.18%; the
  three loops' savings were the same). The bitwise-or pass is 4 a sample.
  On the A76 both forms vectorise and the subtraction read 35.105 against
  35.075 ms (separate sessions).
- **Running Costas sums two bins at a time** (`chunks_exact` pairs of the
  bins, the sent tone's levels and the levels entering and leaving): 44
  instructions a pair (22 a bin; the chunks' bounds checks stayed in the
  loop); **by `step_by` iterators**: 37 a bin. The index form, 15, is kept.
- **Min-sum's one-pass loop for both forms** (the hosts' fixed-size checks
  through it too): it changed the x86-64 (`.text` −128 B) and AArch64
  binaries, which were not timed; the hosts keep their two loops and the
  shared body is a macro, so their binaries are those of 831e9a5.

### What bounds the stages now (6f89e49)

- **Subtraction, A76:** the reference's block of 16 (O23's polynomial),
  the generator's increments and running sums, the frequency correction's
  scalar 64-bit lanes (4.6% of a crowded slot at b335b0a), then the fit,
  the removal and the time search without the clamp (the check's own share
  was not sampled).
- **Subtraction, M33:** `blocks` (247 M a host slot: the reference and the
  correction's carries), the removal (112 M), `fill_phase_only` (107 M),
  the products (86 M), the fit (85 M), the check (27 M).
- **Sync, M33:** the running sums, 15 instructions a bin and Costas symbol;
  the local maxima (`Decoder::search`, 4.5 M small).
- **Min-sum, M33:** 32 instructions an edge for the inputs and two
  smallest, 17 for the messages.

### Next candidates (not built)

| candidate | estimate | cost |
|---|---|---|
| **The radix-5 stage's twiddles without lane gathers** (hosts) | A76 up to half of a 6.0% (host) / 13.1% (small) loop | a layout read whole needs a copy of the twiddles per stride (46 KB of tables) or forms the compiler lowers to whole-vector loads (none tried) |
| **The correction's lanes as 32-bit halves with carries** (hosts) | A76 about a third of a 4.6% loop: −1.5% of a crowded slot | two compare-and-carry steps a lane a step; whether rustc vectorises it is untried |
| **The check over the waveform's reach only** | M33 −0.1 M a call (the 180 000 samples to about 157 500) | a bound on the time search's moves that the code must keep |
| **Min-sum's magnitude selects on the M33** (prof19's candidate 5) | −15 to −25% of `min_sum_round` | NaN and −0 cases must keep their results |

### Risks

- The forms without the clamp rest on the check: a sample out of the limit
  anywhere in the usable audio must send the call to the clamped forms. The
  check covers every usable sample (the subtraction reads none outside),
  the test puts one at the first and last usable samples, inside and after
  the signal, and the mutants that narrow the check or invert it fail.
- The A76 and x86-64 gains of da69ed4 rest on the check's first pass
  vectorising (`vecor`) and on the forms without the clamp; the M33's on the
  check's four-instruction loop (`shortloop`) and the three loops' lengths:
  checked; a toolchain change can lose them with every test passing.
- The audio's scale: audio whose magnitudes are all below 2 (a slot is
  within ±1) is decided by the first pass. Audio with samples of 2 or more
  beside smaller ones takes the second pass too (the or of their exponents
  can exceed the limit's), and audio with any sample out of the limit
  takes the clamped forms: correct, but slower than before by the check
  (on the M33 up to about 2 M a call when both passes run: 4 and 7
  instructions a sample).
- The running Costas sums run on a host only in the group-one tests and
  the direct test of both forms; min-sum's one pass only in the direct test
  of the scalar round (the group-one cfg does not switch belief
  propagation's forms). The M33 identity runs are their end-to-end
  evidence.
- Flash: +1 772 B on the M33 host image (the second forms), −28 B small.

### Not measured

- Cortex-M33 cycles (emulated instructions only); M33 counts on any slot
  but s0086 and s0407 (seven slots for identity); any M4F count or identity
  (sizes and library calls only).
- Sparse and empty slots on any target (the A76 timing is the crowded
  `main` block; the subtraction runs only after a decode, so a quiet slot
  gains nothing from da69ed4; the sync and min-sum changes are per pass on
  the M33 and were counted on two crowded slots only).
- The A76 small configuration after the commits: its binaries' sections
  equal da69ed4's, whose subtraction it never runs (no small-slot timing
  was rerun).
- The x86-64 and A76 stage shares inside the commits (only the subtraction
  and the whole program were timed); the PC sampler was run at b335b0a
  only.
- A placement variation on the A76 (the untouched stages moved by at most
  0.13%).
- The clamped forms end to end on a target: every identity slot and set is
  within ±1, so the x86-64, A76 and M33 identity runs take the forms
  without the clamp only, and the clamped forms (`IN_LIMIT` false) and the
  check's second pass ran only in the host unit tests; no M4F run at all.
  (O25b adds one M33 emulated run with samples out of the limit.)
- Off-air recordings; the M33 or A76 with audio out of the limit (the
  clamped forms' timing is unchanged code, not re-measured).
- After a toolchain change: the bounds above.

## Review follow-ups (O25b, 2026-11-03; every section of all eight binaries unchanged)

On top of O25 (307d68f): b6a62a8 (the subtraction's within-limit forms and
tests), 6bb996c (the code-generation check's header and the sync search's
note) and 3cc7ca8 (min-sum's comment), then this record. Section hashes of
all eight binaries were taken at each commit (one scratch path, rustc
1.93.0, thumbv8m with `--cfg cat_ft8_minmaxnm`): every `.text` and
`.rodata` equals 307d68f's (whose code is 6f89e49's), so O25's timing,
counts and identity stand for them.

- **b6a62a8.** `input_branch_as::<IN_LIMIT>` beside `input_as`, used by
  the fit's single samples instead of the choice spelled in place; the
  check's first comparison written `(any & MAGNITUDE) <= limit`.
  `within_limit_is_where_input_is_the_identity` also holds the check
  against `input` (the target's form) and `input_select`, not only
  `input_branch`. The out-of-limit test has a sixth place, `skip − 1`,
  just outside the checked audio: the results equal those without the bad
  sample, and the sample is left as it was. Its count's `− 3` is explained
  in place: at dt 2.0 the place after the signal is past the slot's end, so
  its three values are not run.
- **6bb996c.** The check's header states each bound's current count and
  the form it catches, without the earlier counts (those stay here). It
  says that the loop metrics take the shortest loop and so bound only the
  forms without the clamp (the fit 14 of 15, the products 30 of 31, the
  time search's whole lanes 107 of 110, the removal 20 of 21); the clamped
  forms (20, 36, 137, 26) carry no loop bound. That is accepted: they run
  only on audio with a sample beyond ±2¹⁶ or not finite, which a receiver's
  samples scaled to ±1 never are; their results are the tests' (each form
  against the other, bit for bit); and the count metrics (floors and
  ceilings on instructions) are summed over both forms, so a ceiling such
  as the M4F's library calls binds each form, a floor the two together.
  `m33s within_limit shortloop 4` stays exact: 4 is the least a loop of one
  sample a turn has (a load that steps its pointer, the or, the count, the
  branch), each instruction more is a quarter more on that pass, and a
  faster loop unrolled to several samples a turn would be longer and fail,
  to be measured then; no notch of headroom has a form it should pass.
  The branch form of the clamp (`INPUT_SELECT` false), forced on a copy:
  the removal's four-lane `fmaxnm`/`fminnm` 1 (floor 2), the time search's
  0 (floor 2), the M33's `vmaxnm`/`vminnm` 0 (floor 8), each failing; the
  removal's selects 0 (the O18 branch form had 18; the floors catch it
  now).
- **The rows by value.** With `costas_scores` taking the rows by
  value through a `Copy` view, as O21 measured it: the running sums' loop
  is 15 instructions as with the reference (`sumloop3`), and with
  `Rows::row` inlined also 15, so neither form fails the check now (O21's
  +3.4% was the nine-load loop's). M33 emulated instructions, s0086, staged
  images:

  | form | small slot | sync search | host slot | sync search (three passes) | decodes |
  |---|---|---|---|---|---|
  | by reference (307d68f) | 172 570 220 | 19 214 187 | 1 437 845 369 | 64 388 354 | 23 / 40 |
  | by value | 173 226 180 (+0.38%) | 19 212 758 (−1 429) | 1 439 712 608 (+0.13%) | 64 384 067 (−4 287) | 23 / 40 |

  The rise is in the feed, +635 719 a pass (+0.71%: other code moved; the
  subtraction's counts are equal), not the sync search. The reference stays;
  the note on `costas_scores` gives this number instead of O21's.
- **One min-sum loop on every target, measured, not adopted.**
  `min_sum_check` as one loop for every target, without `ONE_PASS`, the
  `take!` macro or the sizes as constants (`min_sum_round` without `V`):
  the M33's four images are 6f89e49's to the byte; x86-64 `.text` the same
  size, other code; AArch64 +64 B.
  - A76, `slotcost small small 200 5`, six alternated runs each (12 sides a
    build), every run valid (504 throttle samples and 60 in the warm-ups,
    all 0x0, 2 400 MHz, peak 52.7 °C): belief propagation 4.02 ms in all 12
    sides at 6f89e49, 4.02–4.04 one loop (median 4.03, 9 of 12 above 4.02,
    none below); the pass 12.83 ms median for both (12.82–12.84,
    12.83–12.85). About +0.25% of belief propagation, at the bench's
    resolution of 0.01 ms, and in one direction.
  - x86-64, core 5, `perf stat` user cycles of the whole program, builds
    alternated, none void: `slotcost small small 200 5`, six rounds,
    95.84 → 95.75 G (−0.09%; ranges 95.22–96.23 and 95.27–96.12),
    instructions 295.10 → 295.18 G; `slotcost host host,passes=1 200 5`,
    four rounds, 309.29 → 308.97 G (−0.10%; 308.51–309.39 and
    308.02–310.01), instructions equal (870.70 G). Within noise.
  Not faster on either and slower on the A76 by one step of its
  resolution every time it differed, so the hosts keep their two loops;
  the comment in `min_sum_check` gives these numbers.
- **The clamped path end to end on the M33.** The multi-pass host
  image (first pass fed as i16, further passes on the slot as `f32`) with
  four samples written out of the limit before the further passes (a NaN
  at 30 011, +∞ at 77 777, −10⁶ at 120 001 and 7·10⁴ at 150 003, all in
  the usable audio, so `within_limit` is false and the clamped forms and
  the second pass run), s0086, emulated: the residual of b335b0a and of
  307d68f equal byte for byte (180 000 samples). The four samples come back
  taken and subtracted (−0.0386, 0.0705, −64 940.6, 64 922.3); no residual
  sample is non-finite or beyond 2¹⁶. 3cc7ca8's thumbv8m sections equal
  307d68f's.
- **Accepted as is:** `in_limit` as a parameter, a wrapper and a field, with
  the 2×2 match of `Removal::run`: consistent with `report`;
  another flag of the kind would call for measuring one top-level
  monomorphisation instead.
- **Follow-up, not this round:** the front end's `checked_copy` is a second
  spelling of the within-limit check (a compare fold, 7–8 instructions a
  sample on the M33 against the or pass's 4); one shared helper may trim
  the feed (48.7% of the M33 small slot).

### Checks (3cc7ca8)

- Local CI passes at 3cc7ca8: fmt; clippy `-D warnings` (workspace, all
  targets and features, thumbv8m); workspace, profile, no-std and release
  tests with the ignored ones (221); the group-one step (212 run, 9
  ignored); the bulk run; MSRV 1.75; docs with and without `std`;
  `check.sh --stack` (flash and static RAM as at 6f89e49); the
  code-generation check (33 of 33, the counts of O25's head).
- A76 release library tests with the ignored ones, cross-built at 3cc7ca8:
  220 pass, 0 fail, the source-tree test skipped (the Raspberry Pi holds no
  sources); 241 throttle samples, start and end included, all 0x0.

### Mutants (3cc7ca8)

Fresh copies of 3cc7ca8's tree, release library tests (212), one log each
with the tree, the edit and cargo's exit status:

| mutant | failing |
|---|---|
| none (control) | 0 |
| `input_branch_as` inverted | 2 (the out-of-limit test, the fit's sums) |
| `input_branch_as` always the sample | 2 (the same) |
| `input_branch_as` always clamped | 0: equivalent (the same values; the fit's few edge samples a block take the clamp, speed only) |
| the select form without its upper clamp | 6 (the identity test among them, through `input` and `input_select`) |
| the check from `skip − 1` | 0: equivalent (the clamped forms, the same results) |
| the usable audio from `skip − 1` | 4 (the out-of-limit test's new place among them) |

### Not measured

- As in O25; no code generation changed, so its timing, counts and identity
  stand.
- The one-loop min-sum on the M33 (its images unchanged) and on the A76's
  host configuration; the by-value rows on the hosts (no host form uses
  the running sums).
- The out-of-limit M33 run is one slot, the host image only, and compares
  residuals (not the decode lists); no M4F or A76 run with audio out of
  the limit.

## The feed: the input checked by the or pass, the FFT's last stage vectorised, the level counts in one pass, the bins' powers apart on the M33, twenty-sixth round (O26, bit-identical, 2026-11-03)

Base: 669666d (the code of O25b; every section of all eight binaries as
307d68f's). O25's profile put the feed (the front end's copy, the FFT, the
bins' powers and levels) at about half of every small slot (A76 49.6%, M33
48.7%, x86-64 50.1%) and a fifth of the crowded host slot. This round cuts
it with four code commits, one optimisation each: 3ea1327 (the block check
by the subtraction's or pass), 2a7ab53 (the FFT's last stage as loops the
compiler vectorises), 129f934 (a row's floor and top levels counted in one
pass) and 6db1324 (the bins' powers out of line and indexed without a vector
unit). Each carries its tests and code-generation bounds. Decodes,
records, residuals and encoder output are identical. No memory is added:
static RAM, the slot's storage and the scratch are unchanged, the M33
stack's high water is unchanged; flash grows by 84 B (M33 small) and 12 B
(host); the hosts' `.text` grows by 26 176 B net over the round on x86-64
and 10 944 B on AArch64 (`cat-ft8-decode`): 2a7ab53 alone +25 408 B and
+14 848 B, its six segments' loops (its commit message does not give
that cost), the other commits −384 and +1 152 B, −1 664 and −2 240 B. O26b
replaces the segments (+3 712 B and −2 432 B `.text` against 669666d, with
9 192 B more `.rodata`).

| target | measure | 669666d | 6db1324 | change |
|---|---|---|---|---|
| Cortex-A76, `slotcost small small 200 5` | feed (medians of 4) | 6.350 ms | 5.670 ms | **−10.7%** |
| | slot | 12.8 ms | 12.1 ms | **−5.5%** |
| Cortex-A76, crowded host slot | feed | 19.100 ms | 17.040 ms | **−10.8%** |
| | slot | 80.8 ms | 78.7 ms | **−2.6%** |
| Cortex-A76, one pass | slot (feed) | 18.3 (6.450) ms | 17.6 (5.770) ms | −3.8% (−10.5%) |
| Cortex-M33, s0086 small | slot, emulated instructions (setup excluded) | 172.57 M | 169.19 M | **−1.96%** |
| | feed | 89.14 M | 85.68 M | −3.87% |
| Cortex-M33, s0086 host | slot | 1 437.85 M | 1 427.09 M | **−0.75%** |
| Cortex-M33, s0407 small / host | slot | 165.67 / 1 434.58 M | 162.32 / 1 423.83 M | −2.03% / −0.75% |
| x86-64, user cycles of the whole program (medians of 5) | `slotcost small small 200 5` | 96.36 G | 93.93 G | **−2.52%** |
| | `slotcost host host,passes=1 200 5` | 309.22 G | 305.61 G | **−1.17%** |

(The A76 and x86-64 figures are 129f934's binaries; 6db1324's host code is
129f934's instruction for instruction, below.)

### The ranking used

O25's profile at b335b0a (whose feed code is 669666d's), the feed's parts:

| target | part | share | lever |
|---|---|---|---|
| A76 small / host | the FFT's last stage (radix 5): 150 instructions a step of the inner loop over q, 63 vector, about 37 of them lane moves gathering twiddles m apart in the table, and an index `% N` per twiddle | 13.1% / 6.0% of the slot | **2a7ab53** |
| A76 small | the first stage (window, digit-reversed loads); the radix-4, -3, -2 stages' butterflies | 7.4%; 5.7%, 4.0%, 3.2%, 1.9% | butterflies only, no lane moves: not attempted |
| A76 small | `Spectrogram::feed` (the copy and check, the window's shift, the level counts) | 2.4% | 3ea1327, **129f934** (the counts widened every byte to 64-bit lanes, two passes) |
| x86-64 small / host | `power_bins` (a quarter of it the radix-5 loop) | 47.9% / 19.7% | 2a7ab53 |
| M33 small | the feed: the transform's butterflies (O20's), the bins' powers 42 instructions a bin, the levels about 47 a bin, the block check 10 a sample, the level counts 2 × 6 a bin, the window's shift (`memmove`, 2.26 M) | 48.7% of the slot | **3ea1327, 129f934, 6db1324** |

### Changes

1. **3ea1327: the front end checks each block with the subtraction's or
   pass.** `checked_copy` decided whether a fed block was within the input's
   limit by a compare fold, `abs(x) <= INPUT_LIMIT` over every sample: on the
   M33 10 instructions a sample (`vabs`, `vcmp`, `vmrs`, a conditional move,
   an `and`). `within_limit`, O25's check for the subtraction (the
   magnitudes' bitwise or, then their largest only when the or is past the
   limit's bits), decides exactly the same samples, so it moves to the front
   end (`pub(crate)`, still not inlined) and both call it: 4 instructions a
   sample on the M33. The per-block contract is unchanged (a block that
   passes is copied; any other is classified sample by sample and counted).
   M33 small −0.94 M a slot (−0.54%), host −2.83 M (three feeds); A76 feed
   −0.015 ms (small −0.01).
2. **2a7ab53: the FFT's last stage as loops the compiler vectorises.** The
   last stage (radix 5, l = 384, one block) ran four butterflies side by side
   and read each twiddle where it was used; lane j of (m, q) is m entries
   after lane j − 1, so the compiler built every twiddle vector lane by lane,
   and each index took a remainder by N. Its index m·(k + q·l) mod N is m·k
   plus a constant between the points where one of the 16 (m, r) pairs
   reaches N; those points (k = 96, 128, 192, 256, 288) are computed at
   compile time from `STAGES` (`LAST_BOUNDS`). Within each of the six
   segments the stage is a plain loop over k, one butterfly an iteration
   with `radix_sums`' operations in their order (one lane), which the loop
   vectoriser takes four k at a time, loading m = 1's twiddles with `ld2`
   and m = 2's with `ld4` (m = 3 and 4 by lanes still, without the index
   arithmetic). Per four butterflies on AArch64: about 336 instructions
   (each segment's last four k run a scalar loop of 232) instead of 5 × 143
   + 44 = 759; x86-64 494 instead of 5 × 154 + 49 = 819. The M33
   (`LANES` one) keeps `last_straight_stage`: its sections are unchanged.
   A76 feed −1.545 ms host (−8.1%), −0.53 small; slot −1.6 / −0.5 ms.
3. **129f934: a row's floor and top levels counted in one pass.**
   `finish_row` counted the levels at 0 and at 255 with two filter counts
   into `usize`: on AArch64 each widened every byte to 64-bit lanes (66
   instructions a 32-byte step, two passes: about 4 instructions a level);
   on the M33 two loops of 6 a level. `level_counts` takes both in one pass
   in 16-bit counts (a row holds at most `BINS` levels): byte compares and
   widening subtractions on AArch64 (32 instructions a 32-byte step); one
   loop of 9 a level on the M33. A76 feed −0.50 ms host, −0.14 small; M33
   small −0.49 M.
4. **6db1324: the bins' powers out of line and indexed without a vector
   unit.** On the M33 the powers' loop, inlined into the row, recomputed its
   five addresses from the front end's base each bin (the base reloaded
   from the stack) and checked the zipped iterators' lengths: 42
   instructions a bin. Where `SCALAR_FORMS` holds, `bin_powers` calls
   `bin_powers_apart` (not inlined), whose `powers` loop indexes its slices
   (`INDEXED`): 35 instructions a bin (34 above bin 960). The hosts keep the
   inlined zipped loop (the code of 129f934: below). M33 small −1.96 M a
   slot (−1.14%), host −6.31 M.

### Cortex-M33 (emulated instructions; staged images built from one scratch path, rustc 1.93.0, `--cfg cat_ft8_minmaxnm`)

| commit | s0086 small slot | feed | s0086 host slot | feed (three passes) | s0407 small | s0407 host |
|---|---|---|---|---|---|---|
| 669666d | 172 570 220 | 89 135 405 | 1 437 845 369 | 272 837 275 | 165 674 168 | 1 434 578 663 |
| 3ea1327 | 171 633 309 (−0.54%) | 88 198 574 | 1 435 018 167 (−0.20%) | 270 000 838 | 164 734 355 | 1 431 751 084 |
| 2a7ab53 | as 3ea1327 (thumbv8m sections equal) | | | | | |
| 129f934 | 171 146 812 (−0.28%) | 87 712 077 | 1 433 399 700 (−0.11%) | 268 382 371 | 164 247 858 | 1 430 132 617 |
| **6db1324** | **169 188 866 (−1.14%; −1.96% in all)** | **85 682 937 (−3.87%)** | **1 427 092 090 (−0.44%; −0.75%)** | **261 963 199 (−3.99%)** | **162 318 338 (−2.03%)** | **1 423 827 980 (−0.75%)** |

- 669666d's counts equal O25's at 6f89e49 to the instruction (small
  172 570 220, host 1 437 845 369), as its sections do.
- Same decodes (23 and 40 at s0086; 30 and 38 at s0407). The subtraction
  and `setup` are the same to the instruction. Belief propagation, the
  sync search and soft bits move by up to 0.03% (other code's placement:
  min-sum's own count is equal, 50 057 880).
- Functions (s0086 small, the feed): `within_limit` 0.72 M (4 a sample over
  the 180 000, and a call a block); `bin_powers_apart` 5.83 M; the pass
  (the rest of the transform and levels) 98.00 → 88.07 M; `memmove` (the
  window's shift) 2.26 M unchanged.
- Paint high water: small 10 304 B, host 10 276 B, unchanged at every
  commit. The feed's path is deeper but stays under it: `finish_row`'s
  frame 464 → 472 B (+8), and below it `bin_powers_apart`'s 56 B (five
  and four registers pushed, 20 B of locals), 64 B more in all on that
  path.

### Cortex-A76 (Raspberry Pi 5; medians of 4 alternated runs)

Aligned builds of 669666d, 3ea1327, 2a7ab53 and 129f934 from one scratch
path, core 3, the throttle, clock and temperature sampled once a second
from core 0, a 5 s sampled busy loop on core 3 before each run; per cycle
`slotcost host host,passes=1 200 5` then `slotcost small small 200 5` of
each build, the builds forward on odd cycles and reversed on even, four
cycles.

| ms (median, spread) | 669666d | 3ea1327 | 2a7ab53 | 129f934 |
|---|---|---|---|---|
| host: feed | 19.100 (0.010) | 19.085 (0.020) | 17.540 (0.030) | **17.040 (0.020): −2.060, −10.8%** |
| host: slot / worst | 80.8 / 85.5 | 80.8 / 85.5 | 79.2 / 83.9 | **78.7 / 83.4 (−2.6%)** |
| host: subtract | 35.075 (0.030) | 35.085 | 35.000 | 35.010 (0.010) |
| host: belief propagation / sync / soft bits | 19.060 / 4.680 / 2.380 | the same | the same | the same |
| one pass: slot, feed | 18.3, 6.450 | 18.3, 6.450 | 17.8, 5.920 | **17.6, 5.770** |
| small: feed | 6.350 (0.000) | 6.340 (0.010) | 5.810 (0.000) | **5.670 (0.010): −0.680, −10.7%** |
| small: slot / worst | 12.8 / 13.2 | 12.8 / 13.2 | 12.3 / 12.7 | **12.1 / 12.6 (−5.5%)** |
| small: belief propagation / sync / soft bits | 4.020 / 1.380 / 0.890 | the same | the same | the same |

- **Placement check.** Belief propagation, the sync search and soft bits
  agree within 0.01 ms in all four builds (both configurations); the
  subtraction moves by −0.065 ms (−0.19%) with no change to its code
  (layout). The feed's gains are 30 to 200 times their spreads.
- **Validity.** All 32 timed runs are valid: 2 548 samples during the runs
  and 160 in the warm-ups, every one 0x0, the 32 end checks included; every
  sample during a run and every warm-up minimum at 2 400 MHz. Peak 55.4 °C.
- 6db1324's AArch64 code is 129f934's (below); it was not timed.

### x86-64 (Threadripper 3970X, core 5, `perf stat` user counters over the whole `slotcost` program)

Aligned builds of 669666d, 3ea1327, 2a7ab53 and 129f934 from the same
scratch path (6db1324's x86-64 code is 129f934's), per round `slotcost
host host,passes=1 200 5` then `slotcost small small 200 5` of each build,
four alternated rounds (forward, then reversed) and a fifth forward; a
check for other jobs of the round (builds, emulators, comparators; not the
A76 drivers' remote sessions, which wait on the network) before and after
each sample. Two samples of 3ea1327 in the first round are void
(the A76 identity's comparison and the A76 test build ran on this machine
then); the medians are of the remaining samples (4 for 3ea1327, 5 for the
others).

| build | small: cycles (median, range) | instructions | host: cycles (median, range) | instructions | feed, host (ms) |
|---|---|---|---|---|---|
| 669666d | 96.36 G (95.27–96.53) | 295.11 G | 309.22 G (308.48–309.91) | 870.69 G | 11.05 |
| 3ea1327 | 95.86 G (95.14–96.48) | 294.92 G | 309.22 G (308.72–310.28) | 870.34 G | 11.06 |
| 2a7ab53 | 94.72 G (94.19–95.34) | 284.18 G (−3.70%) | 307.47 G (307.28–308.11) | 848.83 G (−2.51%) | 10.64 |
| 129f934 | **93.93 G (93.34–94.47), −2.52%** | 281.57 G (−4.59%) | **305.61 G (304.25–306.23), −1.17%** | 843.03 G (−3.18%) | 10.37 (−6.2%) |

129f934's ranges clear the base's by 0.80 G (small) and 2.25 G (host).
3ea1327 is within noise on x86-64 (its check vectorises either way).
Stage times are the virtual machine's clock, for scale only; the whole
program includes the bench's slot synthesis and outcome classification
(5–20% of it at O25), which this round does not change.

### Sizes and stack (`check.sh --stack`, thumbv8m)

| image | flash 669666d → 3ea1327 → 2a7ab53 → 129f934 → 6db1324 (B) | static RAM (B) |
|---|---|---|
| small, small with OSD on | 91 954 → 92 002 → 92 002 → 91 994 → **92 038 (+84)** | 201 668 (unchanged) |
| host, host with OSD off | 104 726 → 104 702 → 104 702 → 104 694 → **104 738 (+12)** | 230 084 (unchanged) |
| encoder only | 35 190 (unchanged) | 0 |

- No scratch or storage added; the stack's high water unchanged (above).
- No fused multiply-add (0 `vfma`/`vfms`/`vfnma`/`vfnms`); `vmla` small
  128, host 241, encoder 17, unchanged. No `f64` helper in any image.
- Cortex-M4F (thumbv7em, embedded-check images, no cfg): flash small 92 220
  → 92 264 → 92 264 → 92 252 → 92 304 B, host 105 320 → 105 292 → 105 292
  → 105 280 → 105 332 B; static RAM unchanged; no `f64` helper, no fused
  multiply-add; the library calls (`fmaxf`/`fminf` 10 and 14 among them)
  the same at every commit.
- Hosts' `.text` (one scratch path, aligned): x86-64 `cat-ft8-decode`
  464 966 → 464 582 → 489 990 → 491 142 B, `paired_bench` 1 419 254 →
  1 445 558; AArch64 `cat-ft8-decode` 404 712 → 403 048 → 417 896 →
  415 656, `paired_bench` 1 177 240 → 1 188 120. 2a7ab53's six segments
  (a vector loop and the scalar loop for its last four k each) are +25.4 KB
  on x86-64 and +14.8 KB on AArch64. `.rodata` within 48 B.

### Sections of all eight binaries

| binary | 3ea1327 against 669666d | 2a7ab53 against 3ea1327 | 129f934 against 2a7ab53 | 6db1324 against 129f934 |
|---|---|---|---|---|
| x86-64 and AArch64 `cat-ft8-decode`, `paired_bench` | differ | differ | differ | differ (sizes equal) |
| M33 check and staged, small and host | differ | **identical** | differ | differ |

6db1324's host binaries differ from 129f934's only in panic locations: the
AArch64 `paired_bench` disassembly, addresses aside, differs in 13 `adr`
operands that point at `Location` records within one `.rodata` block (their
order), every other instruction equal; their `.text` and `.rodata` sizes
are equal.

### The code-generation check (in each commit)

New metrics: `checks` (calls of `within_limit`, a floor), `ldn` (`ld2`–`ld4`,
a floor), `vec2` (two-lane `.2d` instructions, a ceiling); and checks with
existing ones. The FFT's multiply floor in `finish_row` (m33s) drops from
230 to 199 because 6db1324 moves the powers' 31 multiplies out (207 now): a
copy of the head with the radix-3 stage as loops (O20b's worst form, 218
then) reads 187 and fails it, with the division and zero-fill ceilings.
Each forced form below is a copy of the commit's tree with its own check
(one log each, the check's own exit status):

| form | a64 `feed` checks (≥ 2) | m33s `feed` checks (≥ 2) | a64 `finish_row` ldn (≥ 48) | a64 `finish_row` vec2 (≤ 32) | m33s `bin_powers_apart` fploop4 (≤ 35) | exit |
|---|---|---|---|---|---|---|
| head (6db1324) | 2 | 2 | 62 | 16 | 34 | 0 (38 of 38) |
| f1: the block's compare fold | **0** | **0** | | | | 1 |
| f2: the last stage side by side, lanes gathered | | | **3** | | | 1 |
| f3: the counts by two filters | | | | **170** | | 1 |
| f4a: the powers apart, zipped | | | | | **37** | 1 |
| f4b: the powers inlined | | | | | **not found** | 1 |
| f4c: the radix-3 stage as loops | | | 62 | 16 | 34 | 1 (fpmul 187, divisions 1, zero fills 2) |

(f1–f3 were run at their commits, f4a–f4c at 6db1324. No other check failed
in any form.) What it misses: the x86-64 code (no bounds; its loops were
counted by hand above); the vectoriser's choice for m = 3 and 4 (lanes);
the scalar loop at each segment's end; the linked images.

### Identity

The references are b93516e's outputs (x86-64 records and WAV lines, the
A76's compared with them), 68634cc's (M33 dumps and residuals) and
70be604's (encoder WAVs), which no round since has changed: equal to them
is equal to 669666d's.

- **Records, x86-64:** the 73 800 `paired_bench run` records of each of
  3ea1327, 2a7ab53, 129f934 and 6db1324 equal b93516e's (host, one pass,
  small: 24 600 each, 0 differ; the records comparison exits 0 each).
- **800 WAV slots** × 3 configurations × 3 modes, x86-64, at each of the
  four (7 200 runs, 78 690 decode lines): 93 091 lines byte-identical to
  b93516e's, and the 2 400 `--profile` lines differ only by the sync
  scratch, as at O21 (the WAV comparison exits 0).
- **A76:** the records of 6db1324's AArch64 `paired_bench` equal x86-64
  b93516e's (73 800, 0 differ; exit 0 each), and the 800 WAV slots × 3 × 3
  with its `cat-ft8-decode` give the same 93 091 identical lines and 2 400
  `--profile` lines (exit 0). One benchmark at a time, in fresh directories;
  1 524 throttle samples, once a second, start and end included, all 0x0.
  (2a7ab53's and 129f934's AArch64 code was run on the A76 only in the
  timing, so their AArch64 identity is inferred from their x86-64 identity
  and 6db1324's A76 runs, not run.)
- **Encoder:** the 45 `cat-ft8-encode` WAV files of each commit equal
  70be604's.
- **Cortex-M33 (emulated):** at 3ea1327, 129f934 and 6db1324 (2a7ab53's
  thumbv8m sections equal 3ea1327's) the small and host dump images on
  seven slots (14 of 14) and the multi-pass residuals of s0086 and s0407
  equal 68634cc's (16 of 16: 14 dumps and 2 residuals; the M33 comparison
  exits 0 each).
- **Comparators on a forced difference** (copies of 129f934's outputs; each
  line its own exit status): the records comparison 0 unforced, 1 with one
  record's field changed; the WAV comparison 0 unforced, 1 with a decode
  line's frequency one higher, 1 with a `--profile` line's `peak_bytes`
  one higher; the M33 comparison 0 unforced, 1 with one bit of one dump
  flipped (15 identical, 1 differ); `cmp` of an encoder WAV 0 unforced, 1
  with one byte changed.

### Tests and mutants

New: `the_block_check_is_where_classify_is_the_identity` (the check
against `classify`'s identity over the special values and every exponent
with both signs; one bad sample at every place of 14 block lengths, where
`checked_copy` counts one and writes `classify`'s values; a block with
−2¹⁶ beside small samples, which the or pass alone refuses, taken whole),
`last_twiddle_index_equals_the_stage_index` (every k, m and q of each
segment against the remainder, and each segment's index m·k plus one
constant), `level_counts_equal_the_filtered_counts` (13 widths up to
`BINS`, five kinds of row). Extended: `powers_equal_the_reference_bit_for_bit`
runs both loop forms of `powers` over each range. The existing
`straight_stages_equal_the_side_by_side_stages_bit_for_bit` now compares
the new last stage with `last_straight_stage`, every bit including the sign
of zero, and `transform_equals_the_reference_bit_for_bit` the whole
transform with one, two and four lanes against the reference.

Mutants in fresh copies of 6db1324's tree, release library tests (215);
g1 with `--cfg cat_ft8_test_group_one`. One log each with the tree, the
edit and cargo's exit status:

| mutant | mode | failing (of 215) |
|---|---|---|
| none (control) | release / g1 | 0 / 0 |
| a1 the block check inverted | release | 4 |
| a2 every block copied | release | 4 |
| a3 the check from the block's second sample | release | 2 |
| a4 every block classified (equivalent: work only) | release | 0 |
| b1 the wrap past N rather than at it | release | 58 |
| b2 the wrap decided per k (equivalent: the same index, not affine) | release | 0 |
| b3 a segment left out | release | 10 |
| b4 each segment one k short | release | 12 |
| b5 a twiddle's parts swapped | release | 26 |
| c1 the top at 254 | release | 2 |
| c2 the floor at 0 and 1 | release | 1 |
| c3 the first level left out | release | 2 |
| e1 the indexed loop's c not reversed | release / g1 | 1 / 11 |
| e2 the indexed loop's rotation without its sign | release | 1 |
| e3 the powers apart, zipped (equivalent: work only) | release | 0 |
| e4 the form's choice inverted (equivalent: the same values) | release | 0 |

a4, b2, e3 and e4 are equivalent by design.

### Checks

- **Local CI** passes at 6db1324: fmt; clippy `-D warnings` (workspace, all
  targets and features, thumbv8m); workspace, profile, no-std and release
  tests with the ignored ones (224, the exhaustive comparisons included);
  the group-one step (215 run, 9 ignored), which runs `bin_powers_apart`
  and the other `SCALAR_FORMS` forms end to end on the host; the bulk run;
  MSRV 1.75; docs with and without `std`; `check.sh --stack`; the
  code-generation check (38 of 38).
- **A76 release library tests** with the ignored ones, cross-built at
  6db1324 (`cat-ft8` tree 1fd5d788…): 223 pass, 0 fail, the source-tree
  test skipped (the Raspberry Pi holds no sources), the exhaustive
  comparisons included; 241 throttle samples, once a second, start and end
  included, all 0x0.

### Tried, not kept

- **The bins' powers indexed but inlined** (the M33): 38 instructions a bin
  (the base still reloaded each bin); **out of line but zipped**: 37–39
  (the iterators' length checks stay). Both together: 35.
- **The level counts in runs of 255 with 8-bit counts**: on AArch64 the
  runs unrolled to seven 32-byte steps plus tails (more code, the same
  order of cost); the 16-bit counts are one loop.
- **The last stage's twiddles from a copy laid out per stride** (O25's
  first idea): 46 KB more memory; not built. The segments need none.

### What bounds the feed now (6db1324)

- **A76:** the transform's butterflies; the last stage now about 336
  instructions a step of four butterflies, of which the m = 3 and m = 4
  twiddles are still taken lane by lane (`ld1` to lanes, `zip`), and a
  scalar loop of 232 for each segment's last four k (24 of 384); the first
  stage's digit-reversed loads; the levels' `log2`.
- **M33:** the butterflies (O20's), the levels (about 47 instructions a
  bin), the powers (35), the window's shift (`memmove`, 2.26 M a small
  slot), the check (4 a sample).

### Next candidates (not built)

| candidate | estimate | cost |
|---|---|---|
| The last stage's m = 3, 4 twiddles as whole loads (hosts) | up to a third of the stage's remaining loop | the vectoriser gathers factors 6 and 8 by lanes; a form it loads whole is not known |
| The last stage's scalar tails (hosts) | 24 of 384 butterflies scalar | the interleaved loads read past the last k; code size too (+25 KB x86-64) |
| The window's shift as a ring (all) | M33 2.26 M a small slot | the first stage's loads would wrap |
| The levels' loop on the M33 | about 47 instructions a bin | `log2_power`'s division and the quantiser's branches |

### Risks

- 2a7ab53's gain rests on the loop vectoriser taking each segment's loop
  with interleaved loads; the `ldn` floor holds it on AArch64, nothing holds
  it on x86-64. Its results rest on the segments' boundaries, computed at
  compile time and checked by the index test and the bit-for-bit stage
  tests.
- 6db1324 changes the M33's code only; on a host its form runs only in the
  group-one tests and the direct test of both forms. The M33 identity runs
  are its end-to-end evidence.
- 3ea1327 takes a second pass over a block whose samples' or is past the
  limit's bits (samples of 2 or more beside smaller ones; audio fed at
  16-bit scale as `f32`): 4 + 7 instructions a sample on the M33 instead of
  the fold's 10, and a block out of the limit is classified as before.
- Code size: the hosts' `.text` +26 176 B net (x86-64), +10 944 B
  (AArch64); 2a7ab53 alone +25 408 B and +14 848 B.

### Not measured

- Cortex-M33 cycles (emulated instructions only); M33 counts on any slot but
  s0086 and s0407; any M4F count or identity (sizes and library calls only).
- 6db1324's host binaries timed (their instructions are 129f934's).
- The A76 and x86-64 per-function shares after the commits (only stages and
  the whole program were timed; no PC sampling this round).
- Sparse and empty slots, off-air recordings; audio out of the limit or at
  16-bit scale fed as `f32` (the check's second pass) on any target.
- A placement variation on the A76 (the untouched stages moved by at most
  0.01 ms, the subtraction 0.065 ms).
- After a toolchain change: the bounds above.

## Review follow-ups (O26b, 2026-11-03; the FFT's last stage one loop over a twiddle table past one turn)

On top of O26 (b6a7e5c): 8f225df (the last stage as one loop over an
extended twiddle table, replacing 2a7ab53's six segments, with its tests
and code-generation bounds, x86-64 among them), ff43e16 (names: the
check's `calls_within_limit` metric, `bin_powers_out_of_line`; the level
count test's top level), then this record. Every section of all eight
binaries at ff43e16 equals 8f225df's, and both equal the build timed
below (one scratch path, rustc 1.93.0, thumbv8m with `--cfg
cat_ft8_minmaxnm`).

### The shape chosen

2a7ab53 split the last stage (radix 5, l = 384) at the five k where one
of its twiddle indices m·k + 384·(m·q mod 5) passes N, so that within each
segment the index is m·k plus a constant and the loop vectoriser loads
each m's twiddles as interleaved vectors. Each segment compiled as a
vector loop and a scalar tail: +25 408 B `.text` on x86-64 and +14 848 B
on AArch64 (`cat-ft8-decode`), which its commit message does not say.

8f225df instead lets the index run unreduced: the largest is 4·383 +
384·4 = 3 068, so the twiddle table, where vectors are taken (`LANES` above
one), holds 3 069 entries (N + 1 149: the turn's entries again, built at
compile time from the generated turn, `TWIDDLE_TURN`) and the stage is
one loop over k. That is the least extension the unwrapped index allows,
and one table: every other reader takes its index mod N and reads the
same table unchanged, so the hosts link one 24 552 B table rather than
the turn plus a second one (`TWIDDLE_ENTRIES` is N where one lane is
taken, from the same `LANES` predicate that picks `straight_stages`). The
same AArch64 vector loop as each segment's (336 instructions a step of
four butterflies, 10 `ld2`/`ld4`), one scalar tail of the last four k
(the `ld4` of m = 2 would read past the stage's last entry, so the
vectoriser peels one step), no bounds-check call in the stage. On x86-64
the loop of 80 `mulps` is 450 instructions, as was the first segment's.

### Memory (one scratch path, aligned; `.text` + `.rodata`, B)

| binary | 669666d | 6db1324 (segments) | ff43e16 (table) | against 6db1324 | against 669666d |
|---|---|---|---|---|---|
| x86-64 `cat-ft8-decode` | 464 966 + 84 272 = 549 238 | 491 142 + 84 320 = 575 462 | 468 678 + 93 504 = 562 182 | −22 464 + 9 184 = **−13 280** | +3 712 + 9 232 = +12 944 |
| x86-64 `paired_bench` | 1 419 254 + 147 236 = 1 566 490 | 1 445 558 + 147 228 = 1 592 786 | 1 422 966 + 156 428 = 1 579 394 | −22 592 + 9 200 = **−13 392** | +3 712 + 9 192 = +12 904 |
| AArch64 `cat-ft8-decode` | 404 712 + 76 632 = 481 344 | 415 656 + 76 632 = 492 288 | 402 280 + 85 840 = 488 120 | −13 376 + 9 208 = **−4 168** | −2 432 + 9 208 = +6 776 |
| AArch64 `paired_bench` | 1 177 240 + 137 940 = 1 315 180 | 1 188 120 + 137 948 = 1 326 068 | 1 174 872 + 147 132 = 1 322 004 | −13 248 + 9 184 = **−4 064** | −2 368 + 9 192 = +6 824 |

- The twiddle table is 24 552 B in each host binary (`llvm-nm`), 15 360 B
  in each M33 and M4F image; no binary holds two. `decode::TABLE_BYTES`
  and `--profile`'s `static_table_bytes` read 48 621 B on hosts (39 429
  before and on the Cortex-M targets); ADR 0024 is amended with both.
- RAM, scratch and slot storage unchanged on every target.
- `finish_row`, the function holding the transform: AArch64 2 882
  instructions (5 898 with the segments), x86-64 3 364 (7 394); the
  x86-64 figure was 41 KB of code with the segments, over the 32 KB
  instruction cache of the Zen 2 core it was timed on.
- **Cortex-M33:** flash small 92 038 B, host 104 738 B, static RAM
  201 668 / 230 084 B, as at 6db1324 (`check.sh --stack`); `.text` and
  `.rodata` of all four images the same sizes. Their hashes differ: the
  table, now `fft::TWIDDLES` built from the turn, sits elsewhere in
  `.rodata` (0xd844 rather than 0x111ec in the small staged image), which
  moves the literal addresses; the instructions are the same in number
  (below). **M4F:** flash 92 304 / 105 332 B and static RAM as at
  6db1324; `fmaxf`/`fminf` 10 and 14, no `f64` helper, no fused
  multiply-add.

### CPU

**Cortex-A76** (Raspberry Pi 5, core 3; medians of 6 alternated cycles,
`slotcost host host,passes=1 200 5` then `slotcost small small 200 5` of
each build, forward on odd cycles and reversed on even; 6db1324's binaries
stand for b6a7e5c, whose code they are; the table build's every section
is ff43e16's):

| ms (median, spread) | 669666d | 6db1324 (segments) | ff43e16 (table) |
|---|---|---|---|
| host: feed | 19.105 (0.030) | 17.040 (0.020) | **16.840 (0.020)**: −0.200 (−1.2%), −2.265 (−11.9%) against 669666d |
| host: slot / worst | 80.8 / 85.5 | 78.7 / 83.4 | 78.6 / 83.3 |
| one pass: slot, feed | 18.3, 6.455 | 17.6, 5.770 | 17.6, **5.690** |
| small: feed | 6.350 (0.010) | 5.660 (0.010) | **5.590 (0.010)**: −0.070 (−1.2%), −0.760 (−12.0%) |
| small: slot / worst | 12.8 / 13.2 | 12.1 / 12.6 | 12.1 / 12.5 |

- **Placement check.** Belief propagation, the sync search and soft bits
  agree within 0.01 ms in all three builds and both configurations (soft
  bits small 0.890 → 0.900 at the table build, one step); the subtraction,
  whose code no commit here touches, reads 35.065, 35.000 and 35.080 ms
  (layout, ±0.2%). The feed's gain over the segments is ten times its
  spreads, in each of the 6 cycles.
- **Validity.** All 36 runs valid: 2 850 samples during the runs and 180
  in the warm-ups, every one 0x0, the end checks included; every sample
  during a run at 2 400 MHz (one warm-up's minimum 1 600 MHz, the first);
  peak 54.3 °C.

**x86-64** (Threadripper 3970X, core 5, `perf stat` user counters over the
whole `slotcost` program; medians of 4 alternated rounds, none void):

| | 669666d | 6db1324 | ff43e16 (table) |
|---|---|---|---|
| small: cycles (range) | 95.66 G (95.44–95.88) | 94.22 G (93.83–94.32) | **92.21 G (91.79–92.40)**: −2.13%, −3.60% against 669666d |
| small: instructions | 295.11 G | 281.55 G | 279.21 G (−0.83%) |
| host: cycles | 308.66 G (308.25–310.08) | 305.65 G (305.53–306.01) | **301.55 G (301.38–303.08)**: −1.34%, −2.30% |
| host: instructions | 870.70 G | 843.04 G | 838.34 G (−0.56%) |

The cycles fall more than the instructions: the five scalar tails it
drops are 20 of 384 butterflies; the rest is likely the instruction cache
(above; not measured). O26's own x86-64 rounds (medians of 5, 96.36 →
93.93 G small) were another session; these 4 rounds compare the three
builds side by side.

**Cortex-M33** (emulated, staged images, s0086 and s0407, small and host):
every stage's and function's instruction count at ff43e16 equals
6db1324's (function names aside: `bin_powers_out_of_line`), decodes 23 /
40 and 30 / 38, the paint high water 10 304 / 10 276 B. The M33 runs
`last_straight_stage`; O26's M33 figures stand.

So the table form is at least as fast everywhere it runs and smaller in
code and code + data on both hosts than the segments; against 669666d it
costs 6.8 KB (AArch64) and 12.9 KB (x86-64) of code + data for −12% of
the A76 feed. It is kept.

### Tried, not kept

- **The segments at run time** (one loop over a list of bounds, the
  segment a function with `lo`, `hi`): out of line (a trial build) the
  stage lost its vectorisation (no `ld2`/`ld4`, 10 bounds-check
  calls); inlined (forced form g4 below) AArch64 kept 12 interleaved loads
  but `finish_row` grew to 3 703 instructions with 103 two-lane `.2d`
  instructions, x86-64 to 4 296 with its vector loop 629 instructions. Not
  timed.
- **A table of 2N** (a trial build, a second table beside the
  turn): the same AArch64 loop, +30 KB `.rodata`; with the turn's own
  readers on the first half, 15 360 B more than the N + 1 149 table that
  replaces the turn. That build's x86-64 timing of it (feed −5.8%
  against 6db1324) had the hosts' bins' powers indexed as well.
- **The index reduced mod N** (g2): no vector loop on x86-64, 9
  interleaved loads on AArch64; equal values (mutant t7).

### The code-generation check (ff43e16)

Changes: an x86-64 build (`x64`); `mulloop80` (the shortest loop with 80
`mulps`, a ceiling, x86-64: 460 against 450); `insns` (a function's
instructions, a ceiling: `finish_row` 3 000 on AArch64 against 2 882,
3 500 on x86-64 against 3 364); the `ldn` floor at its count (12, of which
10 the stage's; it was 48 against 62, so a segment could have stopped
vectorising); `checks` renamed `calls_within_limit` (it shared its name
with the script's list of checks); CI installs the x86-64 target. Each
form below is a fresh copy of ff43e16's tree with one edit, its own log
and the check's exit status:

| form | a64 ldn (≥ 12) | x64 mulloop80 (≤ 460) | a64 insns (≤ 3 000) | x64 insns (≤ 3 500) | a64 / m33s calls_within_limit (≥ 2) | exit |
|---|---|---|---|---|---|---|
| head (ff43e16) | 12 | 450 | 2 882 | 3 364 | 2 / 2 | 0 (41 of 41) |
| g1: the last stage side by side, twiddles gathered (O26's f2) | **2** | **699** | 2 941 | 3 403 | | 1 |
| g2: the index reduced mod N | **9** | **no loop** | **3 140** | 2 757 | | 1 |
| g3: O26's six segments, each compiled | 62 | 450 | **5 898** | **7 394** | | 1 |
| g4: the segments at run time, inlined | 12 | **629** | **3 703** | **4 296** | | 1 (and `vec2` 103) |
| a4 (O26's mutant): every block classified | 12 | 450 | 2 882 | 3 364 | **0 / 0** | 1 |

O26's b2 (the wrap decided per k) has no counterpart: the table form has
no wrap. Its nearest form, the index reduced per twiddle, is g2. O26's e3
and e4 (the powers' form on the M33, equivalent by value) stay as O26
left them: e3 fails `fploop4` as O26's f4a, e4 is guarded by nothing on
the hosts (equivalent, unguarded).

### Identity (ff43e16)

- **Records, x86-64:** 73 800 `paired_bench run` records equal b93516e's
  (24 600 each of host, one pass, small; 0 differ, exit 0 each).
- **800 WAV slots** × 3 × 3, x86-64: 93 091 lines byte-identical to
  b93516e's, 2 400 `--profile` lines differing only by the sync scratch
  and, new, by `static_table_bytes` 39 429 → 48 621 in every one of them
  (the line comparison: O21's rule plus that one change; exit 0). On forced
  copies it exits 0 unforced, 1 with one line's table bytes at 48 629, 1
  with one left at 39 429, 1 with one decode's frequency one higher.
- **A76:** the 73 800 records equal b93516e's (24 600 each of host, one
  pass, small; 0 differ, exit 0 each), and the 800 WAV slots × 3 × 3 the
  same as x86-64's above under the same comparison (exit 0); throttle 0x0 in
  all 1 527 samples.
- **Encoder:** the 45 `cat-ft8-encode` WAVs equal 70be604's.
- **Cortex-M33 (emulated):** the small and host dump images on seven slots
  and the multi-pass residuals of s0086 and s0407 equal 68634cc's: 16 of
  16, 14 dumps and 2 residuals (exit 0).

### Tests and mutants

`last_twiddle_index_equals_the_stage_index` is replaced by
`twiddles_repeat_the_turn_as_far_as_the_last_stage_reads` (every entry
against the turn's at t mod N, bit for bit; every (k, m, q) of the last
stage's unreduced index against the stage's index mod N; the largest + 1
equal to the table's length where vectors are taken). The bit-for-bit
stage and transform tests are unchanged and cover the loop.
`level_counts_equal_the_filtered_counts`' third kind of row mixes 0, 127
and 255 (it gave 0, 127 and 254).

Fresh copies of ff43e16's tree, release library tests (215), one log each:

| mutant | failing |
|---|---|
| none (control) | 0 |
| t1 the table one entry short | 58 |
| t2 one turn on the hosts too | 58 |
| t3 the extension one entry off | 9 |
| t4 the extension zeros | 16 |
| t5 the index's m·q not reduced mod 5 | 25 |
| t6 q for m·q | 26 |
| t7 the index reduced mod N (equivalent: the same values; g2 above fails the check) | 0 |
| t8 the last stage side by side (equivalent; g1 fails the check) | 0 |
| t9 a twiddle's parts swapped | 26 |
| c1 the top at 254 (O26's, again with the new row) | 2 |

### Checks (ff43e16)

- Local CI passes at ff43e16, 17 of 17 steps, among them codegen-check
  (41 of 41) and the group-one step.
- A76 release library tests with the ignored ones: 223 pass, 0 fail (the
  source-tree test filtered out); throttle 0x0 in all 240 samples.
- `check.sh --stack`: flash and static RAM above, the largest frame
  `finish_row` 472 B, no fused multiply-add (`vmla` 128, 241, 17 as
  before).

### Not measured

- The table form on the M33 or M4F (not linked there; the M33's counts
  are equal to the instruction).
- The runtime-segment form's and the 2N table's timing on the A76; the
  x86-64 per-function shares; d-cache misses of the larger table (24 KB
  of 64 KB L1d on the A76; the stage reads 3 069 entries a transform).
- After a toolchain change: the bounds above.

## Min-sum vectorised as a release build compiles it, the correction's lanes vectorised, the sum-product inputs without branches, twenty-seventh round (O27, bit-identical, 2026-11-03)

Base: 958f04e (the code of O26b). A short profile first (A76 PC samples,
M33 per-function and per-block counts, x86-64 `perf`), then three code
commits, one optimisation each: c2a99ed (min-sum's groups vectorised as a
release build compiles them, and the code-generation check's AArch64 build
in that form), 2491a13 (the frequency correction's lanes set up out of
line, so their loop is vectorised) and 66d99a4 (the sum-product inputs'
blocks without bounds-check branches); then 11a783b (a count in the
check's comment). Each code commit carries its code-generation bounds.
Decodes, records, residuals and encoder output are identical. No memory
is added: static RAM, scratch, slot storage and the M33 stack's high
water are unchanged; every thumbv8m section is unchanged (hashed), and
the thumbv7em images' sizes and library calls are unchanged (their
sections were not hashed); the hosts' code shrinks (AArch64 `cat-ft8-decode` −4 352 B, x86-64
−3 840 B, `.rodata` unchanged).

| target | measure | 958f04e | 66d99a4 | change |
|---|---|---|---|---|
| Cortex-A76, `slotcost small small 200 5` | belief propagation (medians of 4) | 4.020 ms | 2.370 ms | **−41.0%** |
| | slot | 12.1 ms | 10.4 ms | **−14.0%** |
| Cortex-A76, crowded host slot | subtraction | 35.085 ms | 33.315 ms | **−5.0%** |
| | belief propagation | 19.060 ms | 18.780 ms | −1.5% |
| | slot | 78.6 ms | 76.5 ms | **−2.7%** |
| Cortex-A76, one pass | slot (belief propagation) | 17.6 (8.940) ms | 17.4 (8.810) ms | −1.1% (−1.5%) |
| x86-64, user cycles of the whole program (medians of 4) | `slotcost small small 200 5` | 92.50 G | 91.78 G | −0.78% (ranges overlap) |
| | `slotcost host host,passes=1 200 5` | 302.40 G | 300.67 G | **−0.57%** |
| Cortex-M33 | every count | | | 0 (sections identical) |

The A76 timings in the column 66d99a4 are of a build whose `.text` and
`.rodata` in all eight binaries equal 66d99a4's and 11a783b's (below).

The M33 runs none of the changed code: its images are identical at every
commit (section hashes), so its counts are 958f04e's. Measured at
11a783b (s0086 small 169 188 866, host 1 427 092 090; s0407 162 318 338,
1 423 827 980), every stage equals 958f04e's run on s0086 and O26b's
ff43e16 run on s0407 to the instruction.

### The profile at 958f04e

#### Method

- One build per target of 958f04e from one scratch path (aligned hosts,
  `-align-loops=64 -align-all-functions=6`; thumbv8m with `--cfg
  cat_ft8_minmaxnm`), rustc 1.93.0; every section's size equals O26b's
  ff43e16 (the head's code).
- **A76 PC samples** (Raspberry Pi 5, core 3, one benchmark at a time,
  throttle 0x0 throughout): O25's sampler in a scratch copy of
  `paired_bench` over `slotcost host host 200 5` (100 µs, 1 609 990
  samples in the decoder's steps) and `slotcost small small 200 5` (50 µs,
  503 986). Loops attributed by their backward branches.
- **x86-64** (Threadripper 3970X, core 5): `perf record -e cycles:u` of
  the same two programs, and the sampler on the host one (its small run
  overlapped this round's builds and is left out).
- **Cortex-M33** (unicorn, the staged images): per stage, function and
  basic block, s0086 small and host. Counts equal O26's to the
  instruction (small 169 188 866, host 1 427 092 090).

#### Ranked

| target | where | share | lever |
|---|---|---|---|
| A76 small | `min_sum_round`: the groups of degree 6 (a loop of 604 instructions a group, scalar) and 7 (723) | 16.2% + 8.3% of the slot; the function 30.4% | **c2a99ed**: the groups were scalar in the release build (below) |
| A76 host | `sum_product_round`: tanh and atanh passes / the inputs' blocks (61 a block of 8, 9 branches) / the sums | 8.8 + 7.7% / 2.8% / 2.3% | the passes: none bit-identical found; **66d99a4** the inputs |
| A76 host | `subtract::run`: the reference's block / the frequency correction's lanes (24 scalar instructions a group of four) | 10.0% / 4.9% | **2491a13** the lanes |
| A76 host, small | `power_bins` (the FFT: the radix-5 loop 4.7% / 10.1%) | 20.7% / 44.3% | none this round (O26's next candidates) |
| A76 host | `fill_phase_only`, `correlation_whole`, `Removal::refs`, `Fit4` | 8.8, 5.5, 4.8, 4.6% | none |
| A76 small | `soft_bits_at`, the Costas sums, `search_rows` | 7.4, 6.6, 4.7% | none |
| x86-64 host (perf) | `sum_product_round` 29.0%, `power_bins` 16.0%, `fill_phase_only` 11.3%, `run` 10.9% | | 2491a13 |
| x86-64 small (perf) | `power_bins` 26.1%, `min_sum_round` 11.4% (vectorised there), the Costas sums 10.1%, `search_rows` 7.2%; the bench's synthesis and classification about a quarter | | |
| M33 small | the feed (`Decoder::pass` 52.1%), `min_sum_round` 29.6% (blocks of 16 and 17 instructions an edge, 7.8% and 6.9%), the Costas sums 8.5% | | none found (below) |
| M33 host | `sum_product_round` 25.2%, `blocks` 17.3%, the feed 18.3%, `Removal::refs` 7.9%, `fill_phase_only` 7.7% | | none found |


### What the release build did to min-sum (c2a99ed)

The code-generation check builds the library with one codegen unit
(`--emit=asm` takes one unless told otherwise). The release profile builds
it with 16 and local ThinLTO, whose pipeline inlines and unrolls the round
a second time before the vectorisers run. In that form min-sum's groups
(`group_min_sum`, four checks in the lanes of a vector since O17) were
scalar on AArch64: `min_sum_round` held 14 four-lane instructions in the
measured binaries (`cat-ft8-decode`, `paired_bench`) against 156 in the
check's build, and its group loops were 604 and 723 scalar instructions a
group (degree 6 and 7) with a bounds-check branch per gather. A build of
958f04e with `cat-ft8` at one codegen unit (trial, timed beside the base
for two cycles) read belief propagation 4.01 → 2.69 ms on the A76 small
slot. x86-64 was vectorised in both forms.

The cause, from the IR before the SLP vectoriser in the ThinLTO pipeline
(`-print-before=slp-vectorizer`, then `opt` alone on that module): the
group's 24–28 gathers each sat in its own basic block behind a
bounds-check branch, and the rows' chains of selects (j fully unrolled)
reached past the vectoriser's tree depth from the stores (`opt
-slp-recursion-max-depth=20` vectorised the same IR: 258 vector operations
against 15). Forms tried in the release form (`min_sum_round` four-lane
instructions):

| form | `.4s` | group loop (degree 6 / 7) |
|---|---|---|
| 958f04e: inputs into a local array, then the rows, gathers checked | 14 | 604 / 723 scalar |
| inputs stored into the group first, a loop of their own | 14 | 689 / 835 |
| the local array as `[[f32; K]; D]`, rows copied out | 14 | 613 / 745 |
| that, the magnitude by a sign-bit exclusive or | 14 | 697 / 832 |
| inputs stored into the group first, the magnitude by exclusive or | 100 | 518 / 625 |
| the gathers' index clamped, inputs into the local array | 14 | 598 / 727 |
| out of line, the rows at run time (`degree` an argument) | 91 / 103 (clamped) | 404 / 310 |
| **each row's inputs in place in the step that compares them, the index clamped** (kept) | **162** | **259 / 301** |
| the same, the index checked | 166 | 322 / … |

The kept form: each row of four lanes takes its inputs into the group in
place (as the M33's one-pass check does) and compares them in the same
step; a variable's index is taken no further than the last variable,
`total[usize::from(v).min(N - 1)]`, which every index already is, so the
row has no branch. Values are unchanged (the same subtractions,
comparisons and messages; an index is never clamped). The M33 keeps
`min_sum_check` (groups of one); its images are unchanged.

`min_sum_round` on AArch64 in the release form: 1 933 → 877 instructions,
its two group loops 259 and 301 instructions a group of four checks.

The check: a sixth build, `a64r`, AArch64 with the release profile's 16
units (`-C codegen-units=16` named, one assembly file a unit, read as
one), and `a64r min_sum_round vec4 100` (162; 14 at 958f04e).

### The correction's lanes (2491a13)

`Correction::lanes` built four 64-bit lanes from one state. Inlined, the
compiler saw each lane's step as the first's plus a multiple of the
acceleration, shared one running sum and kept the lanes scalar: 24
instructions a group of four samples on AArch64, 23 on x86-64 (O25's
"4.6% of the crowded slot"). A micro-crate reproduced it and showed the
cost model's margin (`-slp-threshold=-4` vectorised it); with the lanes'
setup opaque (out of line, or from memory) the loop is two-lane 64-bit
adds and one `uzp2`: 10 instructions a group of four on AArch64, 24 a
group of eight on x86-64. `lanes` is now `#[inline(never)]`, one call a
block. Tried first: the lanes as 32-bit halves with carries (O25's
candidate): 36 instructions on AArch64 (half vectorised) and a 96
instruction loop with spills on x86-64; as two pairs: 23. The M33 never
uses the lanes (`apply_as::<false>`).

New metric `add2d` (two-lane 64-bit vector adds), a floor: `a64 6blocks`
8, `a64r 6blocks` 4 (0 with the setup inlined).

### The sum-product inputs (66d99a4)

On AArch64 the round takes its inputs eight edges a block
(`INPUT_BLOCK`); each gather was bounds-checked by a branch: 61
instructions a block in the release form, 2.8% of the crowded host slot.
With the index clamped as above, a select replaces each branch and the
block's subtractions become two four-lane ones: 46 instructions. x86-64
and the M33 use the plain loop (`INPUT_BLOCK` one) and are unchanged
(their sections identical). Bound: `sumloop8` in `sum_product_round`, a
ceiling of 50 (48 one-unit, 46 release; 63 and 61 with the branches).

### Cortex-A76 (Raspberry Pi 5; medians of 4 alternated cycles)

Aligned builds of 958f04e, 66d99a4's build (its sections equal
11a783b's: `.text` and `.rodata` in all eight binaries) and the last
group padded (below, dropped) from one scratch path; core 3, the throttle, clock and
temperature sampled once a second from core 0, a 5 s sampled busy loop on
core 3 before each run; per cycle `slotcost host host,passes=1 200 5` then
`slotcost small small 200 5` of each build, forward on odd cycles and
reversed on even.

| ms (median, spread) | 958f04e | 66d99a4 (= 11a783b) | the last group padded |
|---|---|---|---|
| small: belief propagation | 4.020 (0.000) | **2.370 (0.000)** | 2.370 (0.000) |
| small: slot / worst | 12.1 / 12.5 | **10.4 / 10.7** | 10.4 / 10.6 |
| small: feed / sync / soft bits | 5.595 / 1.380 / 0.900 | 5.590 / 1.380 / 0.900 | 5.580 / 1.380 / 0.900 |
| host: subtract | 35.085 (0.040) | **33.315 (0.090)** | 33.305 (0.020) |
| host: belief propagation | 19.060 (0.000) | **18.780 (0.000)** | 18.810 (0.000) |
| host: feed / sync / soft bits | 16.835 / 4.680 / 2.380 | 16.835 / 4.680 / 2.380 | 16.810 / 4.680 / 2.380 |
| host: slot / worst | 78.6 / 83.2 | **76.5 / 81.0** | 76.5 / 81.0 |
| one pass: slot, belief propagation | 17.6, 8.940 | 17.4, 8.810 | 17.5, 8.830 |

- **Placement check.** The stages no commit touches (feed, sync, soft
  bits, SNR, OSD-1, refine) agree within 0.005 ms between 958f04e and
  66d99a4 in both configurations. Belief propagation's gain on the small
  slot is 1.65 ms with spreads of 0; the subtraction's 1.77 ms is 20 times
  its spread. No placement variation was run.
- Attribution by stage: the subtraction is 2491a13 alone; small belief
  propagation (min-sum) c2a99ed alone; host belief propagation
  (sum-product) 66d99a4 alone (c2a99ed changes only min-sum, which the
  host configuration does not run).
- **Validity.** All 24 timed runs valid: 1 842 samples during the runs and
  the warm-ups, every one 0x0, the end checks included; every sample
  during a run at 2 400 MHz; peak 54.3 °C. The trial session (958f04e
  against the one-unit build, two cycles, 8 runs) likewise all 0x0.

### x86-64 (Threadripper 3970X, core 5, `perf stat` user counters over the whole `slotcost` program)

Aligned builds of 958f04e and 11a783b from the same scratch path, per
round `slotcost host host,passes=1 200 5` then `slotcost small small 200
5` of each build, four alternated rounds (forward, then reversed); a check
for other local jobs of the round before and after each sample: none void.

| build | small: cycles (median, range) | instructions | host: cycles (median, range) | instructions |
|---|---|---|---|---|
| 958f04e | 92.50 G (91.83–92.65) | 279.21 G | 302.40 G (301.66–303.52) | 838.34 G |
| 11a783b | 91.78 G (91.52–92.38), −0.78% | 280.96 G (+0.63%) | **300.67 G (300.24–300.71), −0.57%** | 824.89 G (−1.60%) |

x86-64's min-sum groups were vectorised before (the release form too), so
the small configuration gains little: its cycles' ranges overlap, and its
instructions rise by 0.6% (the one-pass rows and the clamp's compare and
select a gather). The host configuration's range clears the base's by
0.95 G (the correction's lanes; the inputs' blocks are AArch64 only). The
whole program includes the bench's slot synthesis and classification.

### Sizes (one scratch path, aligned; `.text` + `.rodata`, B)

| binary | 958f04e | c2a99ed | 2491a13 | 66d99a4 = 11a783b | against 958f04e |
|---|---|---|---|---|---|
| x86-64 `cat-ft8-decode` | 468 678 + 93 504 | 464 774 | 464 838 | 464 838 + 93 504 | −3 840 |
| x86-64 `paired_bench` | 1 422 966 + 156 428 | 1 419 062 | 1 419 126 | 1 419 126 + 156 428 | −3 840 |
| AArch64 `cat-ft8-decode` | 402 280 + 85 840 | 398 056 | 397 992 | 397 928 + 85 840 | −4 352 |
| AArch64 `paired_bench` | 1 174 872 + 147 132 | 1 170 648 | 1 170 584 | 1 170 520 + 147 132 | −4 352 |

- Cortex-M33 (`check.sh --stack` at 11a783b): flash small 92 038 B, host
  104 738 B, encoder 35 190 B; static RAM 201 668 / 230 084 / 0 B; paint
  high water 10 304 / 10 276 B: all as at 958f04e (the images are the
  same). No fused multiply-add; `vmla` 128 / 241 / 17. M4F (measured:
  sizes and library calls, not section hashes): flash 92 304 / 105 332 B
  as at 958f04e, static RAM unchanged, `fmaxf`/`fminf` 10 and 14, no
  `f64` helper, no fused multiply-add, the same library calls.
- Scratch and slot storage: no type changes size (`BpScratch`, the
  subtraction's scratch). The hosts' frames were not measured.

### Sections of all eight binaries

| binary | c2a99ed against 958f04e | 2491a13 against c2a99ed | 66d99a4 against 2491a13 | 11a783b against 66d99a4 |
|---|---|---|---|---|
| x86-64 `cat-ft8-decode`, `paired_bench` | differ | differ | **identical** | identical |
| AArch64 `cat-ft8-decode`, `paired_bench` | differ | differ | differ | identical |
| M33 check and staged, small and host | identical | identical | identical | identical |

### The code-generation check

New bounds (above): `a64r min_sum_round vec4 100`, `a64 6blocks add2d
8`, `a64r 6blocks add2d 4`, `a64 sum_product_round sumloop8 50`, `a64r
sum_product_round sumloop8 50`. Each form below is a fresh copy of
11a783b's tree with one edit, its own log and the check's exit status:

| form | a64 / a64r `min_sum_round` vec4 (≥ 100) | a64 / a64r `6blocks` add2d (≥ 8 / 4) | a64 / a64r inputs `sumloop8` (≤ 50) | exit |
|---|---|---|---|---|
| head (11a783b) | 165 / 162 | 8 / 4 | 48 / 46 | 0 (46 of 46) |
| g1: the min-sum group as at 958f04e (inputs into a local array, gathers checked) | 156 / **14** | 8 / 4 | 48 / 46 | 1 |
| g2: the group's index checked, in place (equivalent by value) | 143 / 166 | 8 / 4 | 48 / 46 | 0 |
| g3: the correction's lanes' setup inlined | 165 / 162 | **0 / 0** | 48 / 46 | 1 |
| g4: the inputs' index checked | 165 / 162 | 8 / 4 | **63 / 61** | 1 |

No other check failed in any form. g1 shows why the one-unit bound alone
missed it (156 passes). g2 passes: the clamp in the group is not bounded;
without it the group still vectorises in the release form (322 against
259 instructions a group of degree 6, objects of the release build), so
only its cost, not the form, is unguarded. What the check misses: the
x86-64 forms (no new bounds; x86-64 min-sum was vectorised before), the
linked binaries (it reads the library), dependents' other profiles.

### Identity (11a783b)

The references are b93516e's outputs (x86-64 records and WAV lines, the
A76's compared with them), 68634cc's (M33 dumps and residuals) and
70be604's (encoder WAVs), unchanged by every round since.

- **Records, x86-64:** the 73 800 `paired_bench run` records equal
  b93516e's (host, one pass, small: 24 600 each, 0 differ; exit 0 each).
  66d99a4's build (its sections equal 11a783b's) likewise.
- **800 WAV slots** × 3 × 3, x86-64: 93 091 lines byte-identical, 2 400
  `--profile` lines differing only by the sync scratch and
  `static_table_bytes` 39 429 → 48 621 (O26b's comparison; exit 0).
- **A76:** the records of 11a783b's AArch64 `paired_bench` equal x86-64
  b93516e's (73 800, 0 differ; exit 0 each), and the 800 WAV slots × 3 ×
  3 with its `cat-ft8-decode` give the same 93 091 identical lines and
  2 400 `--profile` lines (exit 0). One benchmark at a time; 1 495
  throttle samples, start and end included, all 0x0.
- **Encoder:** the 45 `cat-ft8-encode` WAVs equal 70be604's.
- **Cortex-M33 (emulated):** the small and host dump images on seven slots
  and the multi-pass residuals of s0086 and s0407 equal 68634cc's (16 of
  16; exit 0).
- **Comparators on a forced difference:** (copies of 11a783b's outputs; each its own exit status) the records
comparison 0 unforced, 1 with one record's SNR changed in its last digit;
the WAV comparison 0 unforced, 1 with a decode line's frequency one
higher, 1 with a `--profile` line's `peak_bytes` one higher; the M33
comparison 0 unforced (16 identical), 1 with one bit of one dump flipped
(15 identical, 1 differ); `cmp` of an encoder WAV 0 unforced, 1 with one
byte changed.

### Tests and mutants

No test changes: the existing bit-for-bit tests cover the forms
(`the_min_sum_round_matches_its_first_form_bit_for_bit` runs the groups of
four on any host with ties and NaN, infinity and −0 inputs;
`the_inputs_are_the_same_in_blocks_of_any_size` runs `inputs` at several
block sizes; `correction_in_lanes_and_in_closed_form_follows_the_sequence`
the lanes).

Mutants in fresh copies of 11a783b's tree, release library tests (215);
g1 with `--cfg cat_ft8_test_group_one`. One log each with the tree, the
edit and cargo's exit status:

| mutant | mode | failing (of 215) |
|---|---|---|
| none (control) | release | 0 |
| m1 the group's index clamped one short (`N - 2`) | release | 3 |
| m2 the group's last row not compared | release | 6 |
| m3 the group's inputs not stored (the messages' sign from the old message) | release | 6 |
| m4 the group's index checked, not clamped (equivalent; g2) | release | 0 |
| l1 the lanes' setup inlined (equivalent; g3 fails the check) | release | 0 |
| l2 the lanes' step with 5 accelerations, not 6 | release / g1 | 3 / 3 |
| i1 the inputs' index clamped one short | release | 1 |
| i2 the inputs' index checked (equivalent; g4 fails the check) | release | 0 |

m4, l1 and i2 are equivalent by design.

### Checks

- **Local CI** passes at 11a783b, 17 of 17 steps: fmt; clippy `-D
  warnings` (workspace, all targets and features, thumbv8m); workspace,
  profile, no-std and release tests with the ignored ones; the group-one
  step; the bulk run; MSRV 1.75; docs with and without `std`;
  `check.sh --stack`; codegen-check (46 of 46).
- **A76 release library tests** with the ignored ones, cross-built at
  11a783b: 223 pass, 0 fail (the source-tree test filtered out); 241
  throttle samples, all 0x0.

### Tried, not kept

- **The last group padded (O26's question).** The 3 checks of
  degree 6 left over after the groups of four laid out as a last group of
  three (`place` with stride r = 59 mod 4), taken by the group forms with
  the other lane as padding (inputs 0 or products 1, never stored), so the
  hosts no longer run `min_sum_check`'s two-loop form, its macro or the
  `V` parameter (−32 source lines; layout tests and bit-for-bit tests
  passed, the M33's images unchanged). A76: small belief propagation 2.370
  against 2.370 ms, host 18.810 against 18.780 (4 cycles, above); AArch64
  `.text` +832 B and `.rodata` +144 B, `min_sum_round` 877 → 1 118
  instructions in the release form. No gain and more code: not kept.
- **The correction's lanes as 32-bit halves with carries** and **as two
  pairs** (above).
- **min-sum's scaled magnitudes taken once a check** (`ALPHA` times the
  two smallest before the output loop) on the M33: the same 17
  instructions an edge (the compiler already sinks them back); not kept.
- The one-unit build of `cat-ft8` (`[profile.release.package.cat-ft8]
  codegen-units = 1`) as a fix: belief propagation 2.69 ms small, but it
  is a build setting a dependent crate does not inherit, and the
  source-level form is faster (2.37).

### What bounds the stages now (A76)

PC samples of the head's code (66d99a4's build, its sections equal
11a783b's), as at
958f04e (all 0x0, 2 400 MHz):

- **Small** (shares of the decoder's samples): the feed 53.6%
  (`power_bins` 51.3%), belief propagation 22.7% (`min_sum_round` 19.4%:
  the degree-6 groups 8.5%, the degree-7 groups 4.1%, the sums loop 5.5%,
  20 instructions a variable with three bounds-check branches; the three
  checks left over about 1.3%), the sync search 13.3% (the Costas sums
  7.7%, `search_rows` 5.5%), soft bits 8.7%.
- **Crowded host:** the subtraction 43.5% (the reference's block 10.3%, the
  correction's lanes 2.5%, from 4.9%), belief propagation 24.5%
  (`sum_product_round` 23.4%: tanh 9.0%, atanh 7.9%, the inputs' blocks
  2.5%, the sums 2.4%), the feed 21.9%, `fill_phase_only` 9.1%.

### Next candidates (not built)

| candidate | estimate | cost |
|---|---|---|
| The sum-product round's tanh and atanh passes (A76 host 16.4% at 958f04e) | none bit-identical found | — |
| The FFT's radix-5 twiddles m = 3, 4 as whole loads (O26) | up to a third of 10% (A76 small) | no load form known |
| `soft_bits_at`, the Costas sums, `search_rows` (A76 small 7.4, 6.6, 4.7%) | uncertain | — |
| The rounds' sums (`sums`: three bounds-checked gathers a variable) | A76 small 5.5%, host 2.4% at the head | a clamp or a layout the compiler can prove; not tried |
| The code-generation check's other bounds in the release form | the one-unit build may pass forms the release build does not take: only min-sum differed at 958f04e (per-function `.4s` counts compared) | more builds |
| `total` and `next` padded to `[f32; 256]`, so that every `u8` gather is in bounds without a clamp (O27b) | the remaining checked gathers (the sums, `min_sum_check`, the plain inputs loop) branch-free; perhaps the M33's `min_sum_check` | +656 B RAM on the M33 (two arrays of 82 more `f32`); to be measured |

### Risks

- The release form's vectorisation of min-sum rests on the SLP vectoriser's
  cost and depth limits under local ThinLTO; it is bounded (`a64r`), but a
  toolchain change or a change of the crate's codegen units (a dependent's
  profile) can move it. Dependents building with other profiles (LTO
  `fat`, one unit) were not measured.
- The clamped index (`min(N - 1)`) is a no-op only while every entry of
  `EDGE_VARS` is a variable; the tables are built at compile time and the
  layout test checks every entry. A wrong index would read the last
  variable rather than panic. (O27b: a const assertion checks every entry,
  and with debug assertions the helper asserts the index.)
- `lanes` out of line rests on the compiler not seeing through the call;
  inlined by a later compiler, the check's `add2d` fails.

### Not measured

- Cortex-M33 cycles (no code changed for it); M4F counts.
- The hosts' stack frames; sparse and empty slots; off-air recordings.
- The x86-64 per-function shares after the commits; the A76 per-loop
  profile after the commits.
- A placement variation on the A76.
- Other dependents' build profiles (above).

## Review follow-ups (O27b, 2026-11-03; every section of all eight binaries unchanged)

Base: 2064882 (O27). Three code commits, none of which changes a section:

- 644f97c: `total_of`, the one helper for the clamped gathers (the
  min-sum groups and the sum-product inputs' blocks), documented once
  with the reason (`edge_vars` writes only `i as u8` for i < N, so the
  clamp never changes an index; it makes the gather a select, so the
  rows vectorise in the release form). With debug assertions it asserts
  the index, so a bad table panics in debug builds and their tests rather
  than reading the last total (a new test, debug only, checks the
  panic; another checks each variable's own total). Beside `EDGE_VARS`
  two const assertions: N fits a `u8`, and every entry of the table, built
  again at compile time, is below N (forced to `< N - 1` it fails to
  compile; MSRV 1.75 builds it). The four plain gathers stay checked,
  each with a line why: the plain inputs loop and `min_sum_check` (where
  every check takes it, without a vector unit) were not measured with the
  clamp; the inputs' remainder is at most seven edges a round. Their code
  is unchanged this round. The source comments no longer carry
  instruction counts; they name the check.
- 2537329: the same for the correction's lanes' note.
- 366e3a1: the code-generation check (below).

**Sections.** At 366e3a1 the `.text` and `.rodata` of all eight binaries
(x86-64 and AArch64 `cat-ft8-decode` and `paired_bench`, the M33 check
and staged images small and host, thumbv8m built with `--cfg
cat_ft8_minmaxnm`) equal 11a783b's, so O27's identity, timings and counts
stand for it; the identity was not run again.

### The code-generation check (366e3a1)

- The release-form (`a64r`) entries are marked `one`: with 16 units a
  function copied into two units would otherwise be summed.
- A fifth field `each` (counts only) checks the least of each matching
  function's count. `a64 6blocks add2d 2 each`: the one-unit build has two
  instantiations of `blocks`, 4 two-lane 64-bit adds each, and a sum
  would pass with one of them lost. `a64r 6blocks add2d 2 one`: the
  release form has one `blocks` out of line (4); the other is inlined into
  `subtract::run`, which no check reads for this.
- `sumloop8` (the inputs' blocks): a ceiling of 54, from 50; the head is
  48 one-unit and 46 release, the bounds-checked form 63 and 61.
- The release build fails with its own message when it leaves no
  per-unit assembly (`cat_ft8-*.rcgu.s`), and the note on its 16 units
  says they are cargo's default for the release profile and must follow
  any profile that sets `codegen-units`.
- The header's history of each bound moved here (below); each check entry
  has one line on what it guards.

Forced forms, each a fresh copy of 366e3a1's tree with O27's edit (on the
helper where the edit was on the clamp), its own log and the check's exit
status:

| form | a64 / a64r `min_sum_round` vec4 (≥ 100) | a64 (least of two) / a64r `6blocks` add2d (≥ 2) | a64 / a64r inputs `sumloop8` (≤ 54) | exit |
|---|---|---|---|---|
| head (366e3a1) | 165 / 162 | 4 / 4 | 48 / 46 | 0 (46 of 46) |
| g1: the min-sum group as at 958f04e | 156 / **14** | 4 / 4 | 48 / 46 | 1 |
| g2: the group's index checked, in place (equivalent by value) | 143 / 166 | 4 / 4 | 48 / 46 | 0 |
| g3: the correction's lanes' setup inlined | 165 / 162 | **0 / 0** | 48 / 46 | 1 |
| g4: the inputs' index checked | 165 / 162 | 4 / 4 | **63 / 61** | 1 |

No other check failed in any form. The new paths, on copies of the script:
`each` with a floor of 5 fails (4, the least of the two); `each` on a loop
metric fails with its own message; `one` on the one-unit `6blocks` fails
(two functions); a release build with no per-unit assembly stops with its
message (exit 1). A summed `a64 6blocks add2d` floor of 7 passed (8),
which is why the check takes each function.

### Checks (366e3a1)

- **Local CI** passes at 366e3a1, 17 of 17 steps: fmt; clippy `-D
  warnings` (workspace, all targets and features, thumbv8m); workspace,
  profile, no-std and release tests with the ignored ones; the group-one
  step; the bulk run; MSRV 1.75; docs with and without `std`;
  `check.sh --stack`; codegen-check (46 of 46).
- Library tests: debug 219 pass and 9 ignored (the debug-only panic test
  among them); release with the ignored ones 225 pass; the group-one
  step (release, `--cfg cat_ft8_test_group_one`) 216 pass, 9 ignored.

### Not done

- The `total`/`next` arrays padded to `[f32; 256]` (every `u8` gather in
  bounds without a clamp, +656 B RAM on the M33): a candidate for a later
  round, to be measured on the A76 and the M33 (O27's next candidates).
- The four checked gathers keep their code: changing them is unmeasured.
- The M4F images' sections were not hashed; O27's M4F claim now says
  what was measured (sizes and library calls).

### The code-generation check's bounds

Moved from `codegen-check.sh`'s header at O27b; the counts are as they
stood at O27, with rustc 1.93.0. Since O27b: `a64 6blocks add2d` is a
floor of 2 on each instantiation (4 each), `a64r 6blocks add2d` a floor
of 2, and the inputs' `sumloop8` a ceiling of 54.

- four-lane `.4s` instructions, a floor: with rustc 1.93
  sum_product_round has 698 and min_sum_round 165 (696 and 156 before
  O27); forms that lost the gain counted 115-452 (passes of 4 to 32
  values, 32 not unrolled) and 29 (min-sum check by check). The margin
  under the passes of 4 values is thin: 452 against the floor of 500.
  min_sum_round also as a release build compiles it (`a64r`, findings
  O27): with 16 codegen units the local ThinLTO pipeline inlines and
  unrolls the round again before the vectorisers, and with each row's
  inputs first in a local array and its gathers bounds-checked it left
  the groups scalar, 14 with rustc 1.93.0, while the one-unit build
  counted 156 and passed; with each row's inputs taken in place and the
  variables' indices taken no further than the last, 162 (a floor of
  100). The sum-product round's inputs, eight edges a block, likewise
  without a branch (findings O27): its shortest loop with eight byte
  loads (`sumloop8`, a ceiling of 50: 48 with rustc 1.93.0, and 46 as
  a release build compiles it; 63 and 61 with the indices checked);
- single-lane moves in sum_product_round, a ceiling: 15 with rustc
  1.93; 46 with the group products written in place row by row, 38
  with the products check by check;
- four-lane `fmaxnm`/`fminnm` in the removal (`Removal::refs`) and in
  the time search's whole lanes (`correlation_whole`), a floor, and
  compares and selects (`fcmgt`, `bsl`, `bit`, `bif`) in the removal, a
  ceiling: with rustc 1.93.0 the removal has 6 and 0, the time search
  14; with the branch form of the clamp (`INPUT_SELECT` false) 1, 0 and
  0 (the floors catch it);
- interleaving loads and stores (`ld2`-`ld4`, `st2`-`st4`) in the
  removal and in `Fit4::add_all`, a ceiling of 0: with rustc 1.93 both
  0; with the reference's parts interleaved sample by sample (a group
  of one) 3 and 1, and with the removal's ramp index carried as an
  integer (the loop vectoriser takes four groups at a time) 2 and 0;
- shuffles (`zip`, `uzp`, `trn`) in `blocks` (the reference's block
  of 16 samples), a ceiling of 12: 3 with rustc 1.93 when recorded, 5
  (4 and 1 in its two instantiations) after O27's `lanes` out of line;
  with the parts interleaved 19, and with each sample stored by itself 75;
- a ceiling counts only the code in its function, so it would pass if
  the loop it guards moved out (into a function of its own, or inlined
  elsewhere): on AArch64 each function with a ceiling also has a floor
  on its vector instructions (the thumbv7em `fmaxf`/`fminf` ceilings
  have none). `Removal::refs` has its `fmaxnm`/`fminnm` floor;
  `Fit4::add_all_as` a floor of 12 `.4s` instructions (28 with rustc
  1.93.0 over its two forms; 8 with its four-lane step `add4` out of
  line) and `blocks` 400 (440 with rustc 1.93.0; 0 with
  `scaled_cos_sin` out of line).
- thumbv8m.main with the cfg: `vmaxnm`/`vminnm` in the removal, a
  floor: 20 with rustc 1.93, 0 without the cfg;
- thumbv7em: calls of `fmaxf`/`fminf` in the removal,
  symbol_correlations and `Fit4::add_all_as`, a ceiling of 0: with the cfg
  (wrongly set) they are 20, 6 and 8.
- thumbv8m.main with the cfg at opt-level "s" (the size the embedded
  check builds at, though this is the library, not the linked image):
  the FFT, inlined into `finish_row`, written out butterfly by butterfly
  with one lane. With rustc 1.93.0: 207 `f32` multiplies (`vmul`, `vmla`,
  `vmls`; a floor of 199: 238 and 230 before the bins' powers went out
  of line with their 31, findings O26), 0 divisions (`umull`, `udiv`; a ceiling of 0)
  and 1 zero-fill call (`__aeabi_memclr4`, `__aeabi_memclr`, `memset`; a
  ceiling of 1).
  The forms it was run against are listed in findings O20b.
- thumbv8m.main with the cfg at opt-level "s": the subtraction's sums
  where a group is one sample, taken one lane at a time with the sums in
  registers. With rustc 1.93.0 the shortest loop with five `f32`
  multiply-accumulates or adds in `Fit4::add_all_as` is the form
  without the clamp's, 14 instructions (a ceiling of 15; 20 with the
  clamp, a form this does not bound: see the input's check below; 25
  with the four lanes side by side); the frequency
  correction one sample at a time leaves 19 carry
  adds (`adc`) in the two `blocks` (a ceiling of 24; 37 with the
  correction in lanes), and no method of `Correction` out of line (with
  the correction's `apply` out of line, `blocks` keeps 15 carry adds,
  under the ceiling, so the count alone would pass). The forms were run
  in findings O22 and O22b.
  It misses: the radix-2 stage alone as loops (234 passes), the form of
  the twiddle index (`% N` and a subtraction both pass), a zero fill by
  `__aeabi_memclr8` or a tail call, and what link-time optimisation does
  to the image. A first stage as loops with no division and no zero fill
  was not run, and the floor would not be expected to catch it.
- the polynomial of the subtraction's reference and of the time search's
  products (`cos_sin_turns_times`): on AArch64 its remainder by one shift
  and quarter by one sign word (`cvtloop4` in `blocks` and
  `correlation_whole`); on thumbv8m.main with the cfg at opt-level "s"
  also the quarter's swap as `vsel` and the half turn's sign after the
  weight (`cvtloop1` in `blocks` and in the subtraction's one closure,
  which holds the products; `fpneg` in `blocks`). The ceilings sit one
  to five instructions above rustc 1.93.0's code; the forms they catch,
  and what they miss, are in findings O23 and O23b (the closure's
  ceiling, 40 there, is O24's, below).
- the time search's sums taken as its products are (findings O24): on
  AArch64 its whole lanes out of line (`correlation_whole`; inlined, the
  compiler split the sums across half vectors and spilled them), whose
  shortest loop of four conversions, the form without the clamp's, has
  107 instructions with rustc 1.93.0 (a ceiling of 110; 137 with the
  clamp), and the half turn's sign on the weight, one `eor` a vector (7
  in each of the two forms, a ceiling of 16; a form with the sign after
  the weight had 14, findings O24); on thumbv8m.main with the cfg at
  opt-level "s" one lane at a time with the sign on the weight, the
  products' loop in the subtraction's one closure 30 without the clamp
  (a ceiling of 31; 36 with it; with the clamp, 39 with the products
  stored and summed after, 42 with the sign after the weight, 69 with
  the lanes side by side). There
  too the generator sums its overlap runs' increments as it takes them,
  so `fill_phase_only` and `skip_with` keep no loop that loads with no
  floating point (`memsums`, a ceiling of 0; 2 each with the increments
  stored and summed after).
- the input checked once per subtraction (findings O25): the loops that
  read the audio have a form without the clamp, taken when every usable
  sample is within the limit (`within_limit`), whose bitwise or decides
  in one pass for audio within ±1. On AArch64 that pass is vectorised
  (`vecor`, vector `orr`, a floor of 2: 4 with rustc 1.93.0, 0 as a
  scalar loop). On thumbv8m.main with the cfg at opt-level "s" it is 4
  instructions a sample (`shortloop`, the function's shortest loop, a
  ceiling of 4; 7 with the largest magnitude alone). The ceiling is
  exact because 4 is the least a loop of one sample a turn can have (a
  load that steps its pointer, the or, the count and the branch), and
  each instruction more is a quarter more on the pass; a loop unrolled
  to several samples a turn would be longer and fail it while perhaps
  faster, and is to be measured then. The removal's form without the
  clamp, unreported, is 20 instructions a sample (`fploop4`, a ceiling
  of 21; 26 with the clamp).
  The loop metrics take the shortest loop, so in the fit, the products,
  the time search's whole lanes and the removal they bound the forms
  without the clamp only; the forms with it (20, 36, 137 and 26 above)
  carry no bound. They run only on audio with a sample beyond ±2¹⁶ or
  not finite, which a receiver's samples scaled to ±1 never are, so
  their speed is not on any measured path, and their results are the
  tests' (each form against the other, bit for bit). The counts above
  are summed over both forms: a ceiling (the m4 library calls, the
  removal's selects) binds each form, a floor only the two together.
- the front end checks each block it is fed with the same function
  (findings O26): `Spectrogram::feed` calls `within_limit`
  (`calls_within_limit`, a floor of 2 on AArch64 and on thumbv8m.main with the cfg at opt-level
  "s": 2 with rustc 1.93.0, the row's first block and the others; 0
  with the block's own compare fold, 10 instructions a sample on the
  Cortex-M33 against the or pass's 4).
- the FFT's last stage (radix 5, one block) as one plain loop over k
  whose twiddle indices run past one turn of the table rather than wrap
  (`last_stage`, findings O26 and O26b), which the compiler vectorises
  with each m's twiddles, m apart in the table, loaded as interleaved
  vectors. On AArch64 `ld2`-`ld4` in `finish_row` (`ldn`, a floor of 12:
  12 with rustc 1.93.0, 10 of them the stage's; 2 with the butterflies
  side by side and their twiddles gathered lane by lane, 9 with the
  index reduced mod N). On x86-64 the shortest loop in `finish_row` with
  80 four-lane multiplies (`mulloop80`, a ceiling of 460: 450 with rustc
  1.93.0; 699 gathered lane by lane, 629 with the segments between the
  wraps taken at run time, none with the index reduced mod N). And
  `finish_row`'s instructions (`insns`, a ceiling of 3 000 on AArch64
  and 3 500 on x86-64: 2 882 and 3 364 with rustc 1.93.0; with the six
  segments between the wraps compiled each as its own loop, O26's form,
  5 898 and 7 394; taken at run time 3 703 and 4 296; with the index
  reduced mod N 3 140 on AArch64), so that the stage stays one loop.
- a row's levels at the floor and at the top counted in one pass in
  16-bit counts (`level_counts`, findings O26): two-lane `.2d`
  instructions in `finish_row` (`vec2`, a ceiling of 32: 16 with rustc
  1.93.0, from the transform; 170 with two counts in `usize`, which
  widened each byte to 64-bit lanes).
- the powers of a row's bins on thumbv8m.main with the cfg at opt-level
  "s" out of line and with indexed slices (`bin_powers_out_of_line`, findings
  O26): the shortest loop with four `f32` adds, a ceiling of 35 (34 with
  rustc 1.93.0; 37 with the zipped iterators out of line, 42 inlined
  into `finish_row`, where the function is not found).
- the frequency correction's four 64-bit lanes in the subtraction's
  reference blocks (`Correction::lanes` out of line, findings O27):
  two-lane 64-bit vector adds in `blocks` (`add2d`, a floor of 8 on
  AArch64 and 4 as a release build compiles it, `a64r`: 8 and 4 with
  rustc 1.93.0, two blocks' loops of four adds against the one out of
  line in the release form; 0 with the lanes' setup inlined, whose
  loop was 24 scalar instructions a group of four against 10).
- `sumloop3` in `select` (thumbv8m.main at opt-level "s"): the sync search's running sums (findings O25) read three levels a bin (the expected tone's, and tone 7's and tone 0's entering and leaving the sum), 15 instructions with rustc 1.93.0 (a ceiling of 16; the eight tones added afresh, 24 instructions with nine byte loads, have no such loop). The rows passed to `costas_scores` by value or `Rows::row` inlined into it leave that loop at 15 (findings O25b; the loop of nine loads they lengthened is gone).

### Closing review (O27c)

- `each` is for the floors only: on a ceiling the least count would let
  one instantiation pass for another, and the sum is already the
  stricter check, so the script now fails `each` on any metric that is
  not a floor. It does not check how many functions match, so an
  instantiation inlined away leaves the check on the others.
- The release form's `blocks` floor takes `each` rather than `one`, so
  it holds if the second instantiation stops being inlined into
  `subtract::run`.
- `EDGE_VARS` comes from one constant, which the compile-time check
  reads, so the check cannot drift from the static.
- `total_of`'s doc corrected: only the sum-product inputs' blocks need
  the clamp to vectorise (g4); min-sum's groups vectorise with the index
  checked too (g2), where the clamp only removes the branch.
- The code is unchanged: release sections of all eight binaries equal
  11a783b's.

## Pass 2 skipped after a single weak pass-1 decode (G1, a results-changing round, 2026-11-03)

One hypothesis: a later pass finds signals that the decodes subtracted
before it were hiding, so after a pass 1 of one weak decode, pass 2 is
very unlikely to add anything, yet it costs nearly a whole pass. Target:
CPU per slot on sparse and one-signal slots, with no accuracy loss
beyond the paired tests' noise and no new false decode anywhere.
Change: ea75d2f. Base: 9685acf.

### Measured first (9685acf records, main block only)

A rule that ends the passes early gives exactly the decodes of the
passes it runs (the passes before do not change), so every rule was
first simulated on 9685acf's `run host` records: its losses, and the
stage time of the passes it skips.

**A count rule alone fails.** Pass 2 after a one-decode pass 1 found
nothing in the existing one-signal sets (`sweep` 8 881 slots, `edges`
1 244), and those sets never have a pass 1 of 2–4 decodes. The new
`masked` set (below) has them: there pass 2 found 444 messages after
one pass-1 decode (in 380 of 502 slots), 363 after two (224 of 430), 57
after three (57 of 66). Skipping pass 2 after at most 1, 2 or 3 pass-1
decodes loses 525, 892 and 949 `masked` messages. Pass 3 after a
one-decode pass 2 also finds messages (crowded 10, wide 4, `masked`
83): skipping it loses 9–10 per crowded set, which fails the per-set
sign test (b − c ≤ 2√(b + c): with no gain, at most 4 losses). Pass 3
is left alone; by the pass-2 decode's SNR its yield thins only below
−18 dB (0 of 114 slots), worth 0.39 ms a crowded slot (under 1%).

**The lone decode's SNR separates the cases.** Pass 2 after a single
pass-1 decode, by that decode's SNR as the decoder estimates it (main
block; `sweep`, `edges`, `masked`, crowded, wide, fading):

| pass-1 decode SNR | slots | of them `masked` | pass-2 messages | slots with any |
|---|---|---|---|---|
| below −18 dB | 3 074 | 13 | 1 | 1 |
| −18 to −17 dB | 2 525 | 17 | 0 | 0 |
| −17 to −16 dB | 2 188 | 20 | 1 | 1 |
| −16 to −15 dB | 1 767 | 22 | 2 | 2 |
| −15 to −14 dB | 646 | 27 | 10 | 10 |
| −14 to −10 dB | 101 | 77 | 37 | 37 |
| −10 to 0 dB | 226 | 226 | 180 | 176 |
| 0 to +10 dB | 196 | 196 | 185 | 159 |
| +10 dB and up | 260 | 260 | 219 | 185 |

Every find below −14 dB is in `masked`'s `low` point (a weak masker
with a weaker signal beside it); the lowest is a −18.3 dB decode hiding
a −20.4 dB signal 41 Hz away. Of the 635 pass-2 finds after a lone
decode, 633 lie within 60 Hz of it (the set places them there; `far`'s
distant weak signal is found in pass 2 only twice).

**The rule simulated on main:** pass 2 after a single pass-1 decode only
if that decode's SNR ≥ X (stage time saved, share of the pass time, one
thread's records):

| X (dB) | −21 | −20 | −19 | −18 | −17 | −16 | −15 | −14 |
|---|---|---|---|---|---|---|---|---|
| `masked` lost (limit) | 0 | 0 | 0 | 1 (2.0) | 1 (2.0) | 2 (2.8) | 4 (4.0) | 14 (fails) |
| `sweep` saved | 0% | 0% | 3% | 8% | 15% | 21% | 27% | 29% |
| `edges` saved | 0% | 0% | 1% | 15% | 35% | 39% | 40% | 40% |

Every other set: nothing lost at any X. Chosen on main: −16 (−15 is at
the limit, −14 fails).

### The `masked` set

New in `paired_bench` (own seeds, 1 500 slots): a strong signal
(−4…+20 dB) with one (`near`) or two (`near2`) weak ones (−20…−8 dB)
5–60 Hz beside it; a strong one (+5…+20 dB) with a weak one near the
decode threshold (−21…−15 dB) anywhere (`far`); two strong ones, each
with a weak one beside it (`pair`); 250 slots each; and `low`, 500
slots: a weaker masker (−20…0 dB) with a signal beside it from −22 dB
to 2 dB below the masker. Every slot is a mixture a subtraction pass
exists for, and every one has an overlap, far more than a real band
has: it is a stress test of any rule that skips a pass. The `low` point
was added during the round, after the first simulation: without it no
`masked` slot had a lone pass-1 decode below −17 dB, so the other four
points never reach the rule. It puts slots where the rule acts, and is
harder on it, not easier. It is all white noise with no fading, and
only about 30 of a block's slots (`low`'s weakest maskers) are exposed
to the rule, so the per-set sign test on `masked` has little power;
the stronger evidence is the find rate by lone-decode SNR above, where
every pass-2 find below −14 dB is a `low` slot.

### Settings tried and the confirmation

- **−16** (chosen on main) run on `main`, `fresh` and a third block not
  used before (`confirm`): `masked` b = 2, 1 and **4** (limits 2.8, 2.0
  and **4.0**), every other set 0, within the limit on every block. But
  `confirm` sits at the limit, the three blocks pooled give b = 7 >
  2√7 = 5.3, and one step looser (−15) fails on `fresh` (10) and
  `confirm` (8): −16 is on the edge of the cliff. Not taken.
- **−17, the next more conservative value, taken:** `masked` b = 1, 0
  and 0 on the three blocks; every other set 0 in every block. The one
  loss: `masked` main slot 1301 (`low`), `KM7MD WK5J RR73` at −18.1 dB
  (decoded at −18.3) hiding `N3L P1Q HR03` at −20.4 dB, 41 Hz away.

### The change (ea75d2f)

`DecodeConfig::lone_decode_min_snr: Option<f32>`, −17 dB in `HOST`:
when pass 1 decodes a single message, pass 2 runs only if that decode's
`snr_db` is at least the minimum. `None` gives 9685acf's behaviour:
with `lone=off` the records of every set but `masked` on `main` and
`fresh` equal 9685acf's. 9685acf has no `masked` set, so this round
compared `masked` against working-tree builds of 9685acf's decoder with
the set added: on `main` (the selection's base) one that also had an
experimental option at a setting that leaves the output unchanged, whose
records equal `lone=off`'s on all 1 500 slots; on `main` and `fresh` one
from before `low` was added, equal to `lone=off`'s on its 1 000 slots a
block. A clean comparison over every `masked` slot is in G1b.
NaN is an invalid setting (`Decoder::new` rejects it); a NaN decode SNR
counts as below any minimum. A slot ended this way reports
`PassesEnd::WeakLoneDecode` (`weak_lone_decode`), checked before the
full-list end (an unlimited decode ends there too), and is not "stopped
early". The rule is checked only before pass 2; a single pass-2 decode
does not end pass 3. `SMALL` runs one pass, so it is not
affected: its records, and `host,passes=1`'s, equal 9685acf's on every
set. The decode command and `paired_bench` take `lone=X|off`. No new
state or storage; one `f32` comparison per multi-pass slot. Tests: the
rule on lone signals above and below the minimum, `None`, a crowded slot
untouched by any minimum, its precedence over a full list, the NaN
setting; the decode command's spellings and bad values.

### Results (−17)

| check | result |
|---|---|
| paired sign test (`compare`), against `lone=off`, `main` (7 sets) | within the limit on every set and point; `masked` b = 1, c = 0 (limit 2.0); all others b = c = 0 |
| the same against the selection's base records, `main` (9685acf's; `masked` as above) | identical figures |
| `fresh` (7 sets) | b = c = 0 everywhere |
| `confirm` (`sweep`, `edges`, `masked`, `empty`) | b = c = 0 everywhere |
| sweep 50% point | −19.177 / −19.158 / −19.200 dB on the three blocks, shift +0.000 dB [+0.000, +0.000] |
| false decodes | 0 on `empty` (2 300 slots, each block); no new false decode in any pass of any record; the skipped passes held none |
| CRC failures (expected false decodes) | the same or fewer (sweep main 295 → 246, edges 24 → 15: the skipped passes' own) |
| comparison with a reference decoder: five synthetic sets (sweep, dt, freq, crowded, fading) and a live recording | decode output identical to the base on every slot |
| live recording | 72 of the 109 messages a reference decoder finds, unchanged; one decode not in its output, the same one as before |
| 60 fixed regression slots | every expected message still decoded; regenerating the expected output would add the same 3 messages for the base as for this change, and remove none |
| 800-WAV corpus | decode lines identical in every slot and mode (host, small, single pass; plain, raw, profile); 33 `host --profile` summary lines differ, each a slot of one pass-1 decode below −17 dB whose empty pass 2 no longer runs (`passes_run` 2 → 1, `weak_lone_decode`, pass 1's counts unchanged) |

The reference comparison's sets change nowhere because their
one-signal slots decode mostly above −17 dB: its sweep skips pass 2 in
106 of its 487 lone-decode slots, fading in 7, dt, freq, crowded and
the live session in none (the live session's only lone decode is at
−5 dB).

**−16, for the record** (not taken): the reference comparison and the
regression slots also give identical decode output.

### Cost

x86-64 (Threadripper 3970X, one core, `taskset -c 5`, `paired_bench run`
per set on `main`, `lone=off` and `host` alternated over two cycles;
decode CPU per slot from the records' stage times, cycle mean):

| set, slot type | slots | off ms/slot | −17 ms/slot | change | ms per decoded signal |
|---|---|---|---|---|---|
| sweep, all | 15 000 | 22.93 | 19.61 | −14.5% | 38.73 → 33.13 |
| sweep, lone decode, pass 2 skipped | 4 481 | 27.54 | 16.36 | −40.6% | 27.54 → 16.36 |
| sweep, lone decode, pass 2 kept | 4 400 | 27.50 | 27.43 | −0.3% | |
| sweep, nothing decoded | 6 119 | 16.27 | 16.38 | +0.7% | |
| edges, all | 1 300 | 27.06 | 17.69 | −34.7% | 28.28 → 18.48 |
| edges, pass 2 skipped | 1 088 | 27.54 | 16.32 | −40.8% | |
| masked, all | 1 500 | 34.04 | 33.77 | −0.8% | 15.59 → 15.47 |
| crowded, all | 600 | 48.32 | 48.25 | −0.1% | 2.15 → 2.15 |
| empty | 2 300 | 16.32 | 16.32 | 0.0% | |

`perf stat` of the whole program (slot synthesis and the misses'
unlimited re-search included): user instructions sweep −6.0%, edges
−15.6%, masked −0.33%, crowded and empty equal to 1 part in 10⁷; user
cycles sweep −4.4%, edges −11.6%, crowded +0.7% (cycles +0.2% and
+1.2% in the two cycles with equal instructions: run-to-run noise),
empty +0.02%. In `sweep` the saving is all at the sent SNRs whose decodes
fall below −17 dB: −20.5 dB −3%, −20 −8%, −19.5 −16%, −19 −29%, −18.5
−36%, −18 −39%, −17.5 −33%, −17 −23%, −16.5 −8%, −16 −2%, nothing above.

Raspberry Pi 5 (A76, 2.4 GHz, one core, `taskset -c 3`), ea75d2f's
AArch64 `paired_bench run` on `main`, `host` and `host,lone=off` from
the same binary plus 9685acf's `host`, alternated over four cycles
(forward, reverse); every run valid: throttle word 0x0 throughout
(sampled each second), no rise in the kernel's under-voltage count,
clock 2 400 MHz in every sample, peak 54.3 °C. Decode CPU per slot from
the records' stage times, mean of the four cycles (each cycle within
0.01 ms of the mean):

| set, slot type | slots | off ms/slot | −17 ms/slot | change | ms per decoded signal |
|---|---|---|---|---|---|
| edges, all | 1 300 | 33.91 | 21.67 | −36.1% | 35.44 → 22.65 |
| edges, pass 2 skipped | 1 088 | 34.53 | 19.91 | −42.3% | |
| edges, lone decode, pass 2 kept | 156 | 34.52 | 34.53 | 0.0% | |
| masked, all | 1 500 | 42.97 | 42.67 | −0.7% | 19.68 → 19.54 |
| masked, pass 2 skipped | 30 | 35.17 | 19.98 | −43.2% | |
| crowded, all | 600 | 64.15 | 64.15 | 0.0% | 2.86 → 2.86 |
| empty | 2 300 | 19.88 | 19.88 | 0.0% | |

9685acf against `lone=off`: crowded 64.17 / 64.15, edges 33.92 /
33.91, empty 19.89 / 19.88 ms: the new branch costs nothing measurable.
`sweep` was not run on the A76 (15 000 slots); from the skipped slots'
saving (14.6 ms each) and their share (4 481 of 15 000) it saves about
4.4 ms a sweep slot. Four earlier cycles, taken before the Pi was
restarted with its throttle word at 0x50000, were discarded.

### Memory

Unchanged: the decoder's peak working memory 944 472 B on every slot
(the profile's `peak_bytes`), max RSS within run-to-run noise (the
reference comparison's sweep 22 812 → 22 704 KB); Cortex-M33 static
RAM 201 668 / 230 084 / 0 B (small / host / encoder) as before, flash small 92 038 → 92 058 B, host
104 738 → 104 758 B (+20 B each), encoder 35 190 B.

### Identity

- x86-64: `lone=off` records equal 9685acf's on every set of `main` and
  `fresh` but `masked` (for `masked`, see "The change"); `small` and
  `host,passes=1` records equal 9685acf's.
- A76: the records of all four timing cycles (`host`, `lone=off`, and
  9685acf's `host`) equal the x86-64 records on every slot of `crowded`,
  `edges`, `empty` and `masked`.
- Cortex-M33 (the multi-pass host image, `--cfg cat_ft8_minmaxnm`, in
  the emulator): on a slot with one weak pass-1 decode (WAV s0025, −19
  dB) ea75d2f leaves the residual bit-identical to the input, as x86-64
  reports for that slot (`weak_lone_decode`), while 9685acf subtracts;
  on a slot with one strong decode (s0001) both images give the same
  residual (SHA-256 4715a776…).

### Checks (ea75d2f)

fmt; clippy `-D warnings` (workspace; cat-ft8 all targets and features;
thumbv8m library); workspace tests; profile and no-std tests; the
release tests with the ignored ones (lib 226); the group-of-one forms;
the bulk run; MSRV 1.75; docs (std and no-std); `check.sh --stack`;
the code-generation check. All pass.

### Risks

- The masked mixtures are synthetic and over-represent overlaps; on air
  a weak lone decode with a decodable signal hidden beside it should be
  rarer still, but no off-air slot here has one either way (the live
  session's only lone decode is at −5 dB). The rule's loss grows fast
  above −17 dB (−16 is at the edge, −15 fails).
- The minimum compares the decoder's own SNR estimate (against the noise
  near the signal); a receiver whose noise floor is not flat near the
  signal shifts it. A signal decoded at −17 dB or more always keeps pass
  2, so the effect of an error is only on weak lone signals.
- The saving is on slots whose only decode is weak: none on a band whose
  lone signals are strong, as in the live session and the reference
  comparison's dt and freq sets; pass 2 after a strong lone decode still costs a whole
  pass.
- The A76 figures are of `paired_bench`'s records on one core; in use,
  the decoder shares the CPU with other work.

### Not measured

Any Cortex-M at run time
(the M33 image runs one pass in practice; the multi-pass image only in
the emulator); off-air slots with a weak lone signal.

### Next

- A pass 2 after a lone strong decode searching only near the subtracted
  signal (its finds here sit within 60 Hz of it, 633 of 635) would keep
  them for a fraction of the cost. Results-changing; its own round.

## G1b: review follow-ups to G1 (2026-11-03)

Changes: e3153ea (a truncating pass 1), e055e2f (tests), 8aeeed1
(docs), 4250fb4 (the decode command's error), e5120f9 (the bench's
`masked` points). Base: 8edba26. The rule's results are unchanged: the
records of every set and block equal G1's (below).

### The general form of the rule, simulated

The shipped rule tests one pass and one count: before pass 2, after
exactly one pass-1 decode below −17 dB. Its general form: before pass
N, end the passes if every decode of pass N − 1 is below X. Simulated
on G1's `lone=off` records (every set and block run; their `masked`
records equal those of 9685acf's decoder, below), for pass 1→2 with
any number of decodes and, separately, pass 2→3, X = −18 and −17;
messages lost (true decodes of the passes skipped), against the shipped
rule:

| block, set | shipped | 1→2, any count, −18 / −17 | 2→3, −18 / −17 |
|---|---|---|---|
| `main` `masked` | 1 | 1 / 1 | 0 / 0 |
| `fresh` `masked` | 0 | 0 / 0 | **1 / 1** |
| `fresh` `wide` | 0 | 0 / 0 | **1 / 1** |
| `confirm` `masked` | 0 | 0 / 0 | **1 / 3** |
| every other set and block | 0 | 0 / 0 | 0 / 0 |

- Pass 1→2 with any count loses exactly what the shipped rule loses at
  −17 (and at −18 the same one `main` message). It fires in one more
  slot a block on `main` and `fresh` only: two `low` slots whose pass 1
  decoded two weak messages (−17.3 and −19.9 dB; −19.8 and −17.2), with
  nothing found after. Two slots in three blocks are too few to choose a
  minimum for a pass 1 of several weak decodes.
- Pass 2→3 loses messages the shipped rule keeps, at both values. In
  each `masked` loss (all `near2`) pass 1 decoded the strong signal
  (−0.5 to +11.2 dB), pass 2 one of its two weak neighbours (−17.2 to
  −19.9 dB), and pass 3 the other neighbour (−18.5 to −19.8 dB, 7–46 Hz
  from the pass-2 decode). Both weak signals sit beside the strong one,
  so a weak pass-2 decode does not mean that nothing decodable is left
  beside the signals subtracted before it. The `wide` loss: seven
  pass-1 decodes, one at −19.2 dB in pass 2, and a −19.4 dB signal 63 Hz
  from it in pass 3. It would save 0.4–1.3 ms a `masked` or `wide`
  slot, nothing elsewhere.

So the general form loses more than the shipped rule; the shipped rule
stays, with its field doc saying why it is only before pass 2 and only
after exactly one decode. Not renamed.

### Fixes

- A pass 1 that truncated decodes no longer counts as a single decode:
  in a list of one entry, a weak decode kept and a second truncated now
  ends as `ListFull` (stopped early), as before G1, not
  `WeakLoneDecode`. Only a list of one entry can reach this; the
  command's (64 and 128) and the bench's cannot.
- Docs: the field, `DecodeConfig::lone_decode_min_snr`, says the test is
  made only before pass 2, that a single pass-2 decode does not end pass
  3, and why exactly one decode; `decode_passes_staged` says the ends
  checked before a stop (`NothingNew`, `WeakLoneDecode`, `ListFull`)
  are reported instead of `Stopped` when they apply; `PassesEnd` and
  the crate docs' `passes_end` are called diagnostic: a stop asked for
  during pass 1 can show as `weak_lone_decode`, and deadline logic
  should not infer an overrun from it. ADR 0024 likewise.
- The decode command's `--config` error lists `lone=X|off`, from the
  same text as the usage line.
- `paired_bench`'s `masked` points are an enum with an exhaustive match
  (draw order kept); the shadowed `weak` is gone.

### Tests and mutants

New: pass 2 adding a single decode below the minimum (a +6 dB signal
hiding a −18 dB one 15 Hz away, decoded at −18.9 dB in pass 2) still
runs pass 3, with output equal to no minimum; a minimum equal to the
lone decode's SNR runs pass 2, one 0.001 dB above it does not; two weak
pass-1 decodes in a list of one end as `ListFull`, with one truncated;
the weak test itself: NaN below any minimum, `None` never weak, ±∞.
Each mutant, applied alone, now fails a test (release):

| mutant | killed by |
|---|---|
| `pass == 2` → `pass >= 2` | the pass-2 decode test |
| `pass == 2` removed | the pass-2 decode test |
| `<` → `<=` | the boundary case; the NaN unit test |
| the NaN clause removed | the NaN unit test |
| the truncation test removed | the list-of-one case |

### Identity

x86-64, every slot of every set run (`main` and `fresh`: `sweep`,
`crowded`, `edges`, `empty`, `wide`, `fading`, `masked`; `confirm`:
`sweep`, `edges`, `masked`, `empty`), timings ignored:

- `host` records equal G1's (ea75d2f, −17) on every set and block, so
  the truncation test and the enum change nothing there; this includes
  the `masked` signals themselves (the records hold each slot's signals
  and sample hash), so the enum keeps the draw order.
- `lone=off` records equal G1's `lone=off` on every set and block, and
  9685acf's own records on every set but `masked` (`main`, `fresh`).
- `masked`, against 9685acf: a build of 9685acf with only `paired_bench`'s
  final set file added (the decoder as at 9685acf) gives records equal
  to `lone=off`'s on every slot of all three blocks (1 500 each, `low`
  included). G1 had shown this only for the first 1 000 slots a block
  and, on `main`, against a working-tree build (see "The change" above).

No cross-target or A76 run: the rule's output is unchanged.

### Checks (e5120f9)

fmt; clippy `-D warnings` (workspace; cat-ft8 all targets and features;
thumbv8m library); workspace tests (debug); profile and no-std tests;
the release tests with the ignored ones; the group-of-one forms; the
bulk run; MSRV 1.75; docs (std and no-std); `check.sh --stack`; the
code-generation check. All pass.
