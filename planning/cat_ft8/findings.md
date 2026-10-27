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

