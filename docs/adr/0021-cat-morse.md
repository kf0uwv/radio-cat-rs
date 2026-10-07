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
