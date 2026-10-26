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
