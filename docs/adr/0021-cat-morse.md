# ADR 0021 — `cat-morse`: radio-independent Morse code, deterministic by seed

Status: Accepted (2026-10-06)

## Context

`ts570d` is getting a `ts570d cw` subcommand: a Morse trainer for real
on-air contacts. Copy practice comes first: the PC plays synthesized audio,
the operator types what they copy, and the copy is scored. Sending practice
comes later: the operator keys, and a decoder scores the result.

Almost none of that has anything to do with a TS-570D. The Morse table,
PARIS and Farnsworth timing, a human-sounding fist, a click-free tone, a
band with noise, fading, crashes and QRM, a tone decoder, realistic QSO
scripts and copy scoring are the same for every radio. Putting them in
`ts570d` would mean `ft991a` and `ic7100` copy them later.

## Decision

**A new workspace crate, `cat-morse`, holds all of the radio-independent
Morse machinery.** It has six public modules: `codec`, `timing`, `synth`,
`decoder`, `scenario` and `score`. `rng` is a small seeded generator.

### Scope rules

1. **No radio.** No keying command, no CAT, and no dependency on
   `cat-framework`, any `cat-transport-*` or any radio crate. Keying a
   real transmitter from a `Keying` is the application's job. For
   `ts570d` that means `KY` or DTR.
2. **No device audio.** The crate makes and consumes `f32` PCM buffers.
   It does not open a sound card. Playback and capture belong to the
   application, or to `cat-signal-audio`'s `device` feature.
3. **No runtime.** It is pure and synchronous: no monoio, no tokio, no
   threads and no target gating. It builds and tests the same on Linux
   and `x86_64-pc-windows-msvc`.
4. **No local crate dependencies.** It sits at the bottom of the graph
   beside `cat-layout`. Its only normal dependency is `thiserror`.
   `serde` is behind a default-off `serde` feature. CI runs
   `cargo test -p cat-morse --features serde` explicitly, because code
   behind a default-off feature is code CI does not otherwise build
   (ADR 0014 §5, ADR 0017's amendment).

### The determinism promise

Every seeded output comes from the in-crate xoshiro256** generator
(`cat_morse::rng::Rng`), not from `rand`, so a dependency upgrade can
never change it. The generator is pinned by a test to the reference
output.

- **Keying and scripts are bit-identical across platforms for a given
  seed.** `timing::key` and `scenario::generate` use only integer
  arithmetic and IEEE `+ − × ÷`, rounding and `sqrt`, which are exactly
  rounded everywhere. Rust does not fuse these into FMA. Gaussian draws
  use Irwin–Hall (twelve uniforms), not Box–Muller, so that path has no
  `ln` and no `cos`.
- **Audio is bit-identical on one platform, and equal within float
  tolerance across platforms.** The tone, the raised-cosine ramps, QSB,
  QRN decay and the receive filter call `sin`, `cos`, `exp` and `powf`.
  Those come from the platform's maths library, which may differ in the
  last ulp between glibc and the MSVC CRT. Making PCM bit-exact would
  need a `libm` dependency and nothing needs it: audio is heard, not
  compared.

### Recorded design choices

- **Shared patterns.** `=`/`<BT>`, `+`/`<AR>`, `(`/`<KN>` and `&`/`<AS>`
  are the same Morse. The decoder writes the prosign form by default,
  and `DecodeStyle::Characters` gives the character form. The scorer
  treats each pair as equal by default.
- **SNR** is stated against a 500 Hz reference bandwidth by default
  (a CW filter). It is configurable.
- **Word spaces count** in the character error rate by default.
  `SpacePolicy::Ignore` turns that off.
- **The decoder** is a Hann-windowed sliding Goertzel bin feeding an
  amplitude envelope with hysteresis. Finished marks are re-timed at half
  their own peak. Adaptive two-cluster k-means separates dits from dahs
  and letter gaps from word gaps, so Farnsworth spacing is learned
  rather than assumed. There is no FFT dependency. Automatic pitch
  detection uses a bank of Goertzel bins and replays the audio heard
  before it locked.
- **POTA references** use the ISO 3166 form POTA moved to in 2024
  (`US-0001`, not `K-0001`). SOTA references use the
  `ASSOC/RR-NNN` form (`W0C/FR-001`).

## Consequences

- `ts570d cw` is thin wiring: it generates a script, keys each *them* line
  with its station's fist, renders a `Channel`, plays the PCM, reads what
  the operator typed and scores it. `ft991a` and `ic7100` get the same
  trainer for the cost of their own wiring.
- Scripts and stats can be persisted with the `serde` feature. Maps are
  serialized as lists of pairs, because a `Token` is not a JSON key.
- Changing `Rng`, or the order of draws in `timing` or `scenario`,
  changes every seeded output. The pinned-value test makes that a
  deliberate act, not an accident.
- The decoder is tuned against this crate's own synthesizer. Off-air audio
  may need further tuning. That will happen when sending practice brings
  real keying through it.

## Amendment (2026-10-07): the world model in `scenario`

The first scenarios drew US calls by FCC format and DX from about thirty
entities. The trainer now simulates calls from all over the world. All of
it stays inside `cat-morse::scenario`; nothing radio-specific is added.

### What the model is

- **One table, `scenario::world::WORLD`, of the 340 current DXCC
  entities** (deleted entities are left out; Malyj Vysotskij, 151, is one,
  per the ADIF 3.1.5 DXCC enumeration). Each row has its DXCC number,
  name, continent, default CQ and ITU zones, the ITU prefixes it matches,
  weighted call formats, call areas, POTA ISO code, activity tier,
  name pool and towns. A test pins the count at 340.
- **Hand-built from public facts**: ITU prefix allocations (Radio
  Regulations Appendix 42), the DXCC entity list, and the CQ and ITU zone
  maps. No third-party prefix file (such as `cty.dat`, which states no
  licence) is embedded, and nothing is fetched at build or run time.
- **Matching** is longest prefix first, over the call's base. A location
  designator wins over the home call (`EA8/DL1ABC` is in the Canaries,
  `W1ABC/4` in US call area 4); `/P`, `/M`, `/A`, `/B` and `/QRP` don't
  move a station; `/MM` and `/AM` are in no entity. A few prefixes name
  their entity only with a given suffix length (`KG4` + two letters is
  Guantanamo Bay; other `KG4` calls are US).
- **Call areas** carry their own zones and can name their own prefixes,
  so Canada's VE1, VO1 and VY1 are different areas, and a contest
  exchange sends the caller's area zone.
- **Weighting.** Each entity has an activity tier (Common 100, Regular
  25, Uncommon 5, Rare 1) and each call area a weight (VE3 40, VY0 1). DX
  is three times as likely from the operator's continent, or as
  `Difficulty::hf_band`'s coarse path model says (160 m 12:1 near; 15 m
  and up 1:2 in favour of long haul). The local side of a contact comes
  from the operator's own entity.
- **Calls as sent** are sometimes `/P`, `/M`, `/QRP`, signed abroad
  CEPT-style (`F/DL1ABC`) or, in the US, out of their call area. Every
  generated call is checked to resolve back to the station's home.
- **Scenario kinds.** `DxChase` puts the operator in a rare DX's pileup.
  When the operator runs a pileup (`DxPileup`), the callers come from the
  whole world, never the operator's own entity.
- **`band_channel_for`** places the QRM station around the operator.
  `band_channel` is unchanged and behaves as if the operator were in the
  US.

### Simulation conventions (not real-world facts)

These are deliberate simplifications, recorded so nobody mistakes them
for data:

- **Tagged entities.** Where an entity shares another's prefix block and
  real operations use one-off calls, the trainer marks the entity with a
  call suffix: Rotuma `3D2../R`, Conway Reef `3D2../C`, North Cook
  `E51../N`, Austral `FO0../A`, Marquesas `FO0../M`, Clipperton
  `FO0../C`, Chesterfield `FK8../C`, Ducie `VP6../D`, Malpelo `HK0../M`,
  South Georgia, Orkney, Sandwich and Shetland `VP8../G /O /S /H`, Bouvet
  `3Y0../B`, Peter I `3Y0../P`, Heard `VK0../H`, Macquarie `VK0../M`,
  Swains `KH8../S` and Minami Torishima `JD1../M`. (Mount Athos
  `SV2xxx/A` is the real form.)
- **Russia** is split by call-area digit only: European 1/3/4/6/7,
  Kaliningrad 2, Asiatic 8/9/0. Real zones and the Kaliningrad rule
  depend on the oblast letter after the digit.
- **Multi-zone areas** get one zone each where the real boundary runs
  through them (Canada VE2, VE8, VY0; China BY9; Brazil PY8 and PY9;
  Russia per digit). Argentina has no call areas yet.
- **Turkey** is placed in Europe, as `cty.dat` does; DXCC lists it under
  Asia.
- **Activity tiers, area weights, name pools and band path weights** are
  judgement values, not measured from spot or log data. Countries with
  more than one language use one name pool.
- **Some single-digit choices** are the most common rather than the
  only ones (`6Y5`, `J68`, `FK8`).

### Consequences

- Seeded scripts changed. The pinned-value test was re-baselined for
  each step of this work, as a deliberate act.
- The table costs about 106 KB of data in a stripped x86_64 Linux release
  binary, plus about 100 KB of PIE relocations for its pointers (the
  table has about 3,250 strings). A test bounds the string count and
  bytes so growth is deliberate. `WORLD` is a `static`: as a `const` it
  was materialised at every use site and cost more than twice as much.
- Applications should call `band_channel_for` with the operator's call
  to get QRM from around the operator, and may offer `DxChase` and
  `Difficulty::hf_band`.
