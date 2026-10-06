# cat_morse — task plan

## Goal

A new workspace crate `cat-morse`: the radio-independent Morse machinery
(codec, timing, synthesis + channel simulation, decoding, QSO scenario
generation, scoring) that `ts570d`'s future `ts570d cw` trainer is built on.
Pure, synchronous, cross-platform (Linux + `x86_64-pc-windows-msvc`), f32
PCM in and out, no device audio, no runtime, no radio.

## Current Phase

Approved 2026-10-05 (Q1 prosign default + DecodeStyle; Q2 500 Hz; Q3 serde
default-off + CI step; Q4 spaces counted; Q5 ADR 0021, no agent file; Q6 no
version bump). Implementing steps 1-7 on branch `feat/cat-morse`.

## Hard constraints

- No tokio, no monoio, no async at all. Nothing target-gated.
- Nothing TS-570D-specific: no KY, no CAT, no `cat-framework` dependency.
  `cat-morse` has **no local crate dependencies** (sits beside `cat-layout`
  and `cat-signal` at the bottom of the graph).
- No device audio I/O (no `cpal`). Callers own the sound card.
- No `rand`: an in-crate seeded PRNG.
- thiserror + `Result`; imports std → external → local; Apache-2.0 header
  on every file; every package field `.workspace = true`; MSRV 1.75 (no
  `LazyLock`, `is_none_or`, inline `const {}` blocks, etc.).

## Dependencies (proposed)

| Crate | Kind | Why |
|-------|------|-----|
| `thiserror` (workspace) | normal | Repo error convention. |
| `serde` (workspace, `derive`) | **optional**, feature `serde` | See D5. The orphan rule means a caller cannot derive `Serialize` on our types; without this they must hand-mirror every stats/script type. |
| `serde_json` (workspace) | dev-only | Round-trip test of the `serde` feature. |

Explicitly **not** taken: `rand` (portability/reproducibility, D2),
`rustfft` (Goertzel and a Goertzel bank cover tone detection and pitch
search at a fraction of the cost; no FFT-sized latency), `cpal` (out of
scope), `libm` (see D2 — not needed once keying avoids transcendentals).

## Crate layout

```
cat-morse/
  Cargo.toml
  src/lib.rs          re-exports, crate docs (what it is / is not)
  src/rng.rs          Rng (SplitMix64-seeded xoshiro256**)
  src/codec.rs        table, Token, Prosign, Element, encode/decode
  src/timing.rs       Speed (PARIS/Farnsworth), Fist, Keying, key()
  src/synth.rs        ToneShape, render_tone, Channel, Signal, impairments
  src/decoder.rs      Goertzel, PitchTracker, Envelope, Classifier, Decoder
  src/scenario/mod.rs     ScenarioConfig, Script, Transmission, generate()
  src/scenario/callsign.rs Callsign (parse + generate), prefix table
  src/scenario/data.rs     names, QTHs, rigs, WX, POTA/SOTA refs, zones
  src/scenario/qso.rs      per-kind script builders
  src/score.rs        align, Score, CharStats
  tests/roundtrip.rs  synth → decoder end-to-end (integration)
```

## Public API by module

### `rng`
```rust
pub struct Rng;                         // xoshiro256**, seeded via SplitMix64
impl Rng {
    pub fn new(seed: u64) -> Self;
    pub fn fork(&self, stream: u64) -> Rng;   // independent sub-stream
    pub fn next_u64(&mut self) -> u64;
    pub fn unit_f64(&mut self) -> f64;        // [0,1), 53-bit
    pub fn range_u32(&mut self, lo: u32, hi_inclusive: u32) -> u32; // unbiased (Lemire)
    pub fn chance(&mut self, p: f64) -> bool;
    pub fn gauss(&mut self) -> f64;           // Irwin–Hall(12)−6, no ln/cos
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T;
}
```
`fork(stream)` lets each concern (fist jitter, noise, QRN, scenario text)
draw from its own stream so adding a knob to one does not perturb another.

### `codec`
```rust
pub enum Element { Dit, Dah }
pub enum Prosign { Ar, Sk, Bt, Kn, As, Hh, Sos }   // Display: "<AR>" ...
pub enum Token { Char(char), Prosign(Prosign), WordSpace }
pub fn pattern(token: Token) -> Option<&'static [Element]>;
pub fn table() -> &'static [(Token, &'static [Element])];   // whole table
pub fn tokenize(text: &str) -> Result<Vec<Token>, CodecError>;
    // case-insensitive; runs of whitespace → one WordSpace; trims ends;
    // "<AR>" etc. parsed as prosigns
pub fn encode(text: &str) -> Result<Vec<(Token, &'static [Element])>, CodecError>;
pub fn decode_group(elements: &[Element]) -> Option<Token>;  // canonical form, D3
pub fn tokens_to_text(tokens: &[Token]) -> String;
pub fn canonical(token: Token) -> Token;   // folds collisions, D3

#[derive(thiserror::Error)]
pub enum CodecError {
    UnknownChar { ch: char, offset: usize },
    UnknownProsign { name: String, offset: usize },
    UnterminatedProsign { offset: usize },
}
```
Table: A–Z, 0–9, `. , ? / = - ' : " ( ) @ +`, non-ITU `!` `&`, prosigns AR
SK BT KN AS HH(8 dits) SOS. Unknown chars are an error, never dropped.

### `timing`
```rust
pub struct Speed { /* char_wpm, effective_wpm */ }
impl Speed {
    pub fn wpm(wpm: f64) -> Result<Self, TimingError>;
    pub fn farnsworth(char_wpm: f64, effective_wpm: f64) -> Result<Self, TimingError>;
    pub fn dot(&self) -> Duration;          // 1.2 s / char_wpm
    pub fn letter_gap(&self) -> Duration;   // 3 dots, or Farnsworth 3·ta/19
    pub fn word_gap(&self) -> Duration;     // 7 dots, or Farnsworth 7·ta/19
}
pub struct Fist {
    pub dash_ratio: f64,        // 3.0 ideal
    pub weight: f64,            // mark lengthening as fraction of a dot (0 ideal)
    pub element_jitter: f64,    // σ as fraction of a dot, per mark & intra gap
    pub letter_gap_jitter: f64, // σ as fraction of the nominal gap
    pub word_gap_jitter: f64,
    pub speed_drift: f64,       // σ of a slow per-character speed walk
}
impl Fist { pub fn perfect() -> Self; pub fn good() -> Self;
            pub fn typical() -> Self; pub fn sloppy() -> Self; }
pub struct Segment { pub key_down: bool, pub duration: Duration }
pub struct Keying { /* segments */ }
impl Keying {
    pub fn segments(&self) -> &[Segment];
    pub fn total(&self) -> Duration;
    pub fn append(&mut self, other: &Keying, gap: Duration);
}
pub fn key(tokens: &[Token], speed: Speed, fist: &Fist, seed: u64) -> Keying;
pub fn key_text(text: &str, speed: Speed, fist: &Fist, seed: u64)
    -> Result<Keying, CodecError>;
pub enum TimingError { NonPositiveWpm(f64), EffectiveExceedsChar { .. }, InvalidFist(&'static str) }
```
Durations are `std::time::Duration` (integer ns) so `Fist::perfect()` at
20 wpm is exactly 60 ms. Jittered values are clamped to a floor (≥ 0.3 dot)
so a sloppy fist never produces zero/negative segments. Keying uses only
`+ − × ÷ sqrt` → bit-identical on every platform for a seed (D2).

### `synth`
```rust
pub struct ToneShape { pub rise: Duration /* default 5 ms */, }
pub fn render_tone(keying: &Keying, sample_rate: u32, pitch_hz: f32,
                   amplitude: f32, shape: &ToneShape) -> Result<Vec<f32>, SynthError>;

pub struct Signal {
    pub keying: Keying,
    pub start: Duration,         // offset into the channel timeline
    pub pitch_hz: f32,
    pub level_db: f32,           // relative to 0 dBFS-sine reference
    pub qsb: Option<Qsb>,        // { depth_db, period, phase from seed }
    pub shape: ToneShape,
}
pub struct Awgn { pub snr_db: f32, pub ref_bandwidth_hz: f32 } // D4
pub struct Qrn  { pub crashes_per_sec: f32, pub level_db: f32, pub decay: Duration }
pub struct RxFilter { pub center_hz: f32, pub bandwidth_hz: f32 } // optional 2-pole BPF
pub struct Channel {
    pub sample_rate: u32, pub signals: Vec<Signal>, pub noise: Option<Awgn>,
    pub qrn: Option<Qrn>, pub filter: Option<RxFilter>, pub gain: f32, pub seed: u64,
}
impl Channel {
    pub fn duration(&self) -> Duration;
    pub fn render(&self) -> Result<Vec<f32>, SynthError>;
    pub fn renderer(&self) -> Result<ChannelRenderer<'_>, SynthError>;
}
pub struct ChannelRenderer<'a>;              // streaming
impl ChannelRenderer<'_> {
    pub fn fill(&mut self, out: &mut [f32]) -> usize;  // samples written; 0 = done
    pub fn clipped(&self) -> u64;
}
pub enum SynthError { ZeroSampleRate, PitchAboveNyquist { .. }, InvalidParameter(&'static str) }
```
- Rise/fall: raised cosine over `rise`, centred so the half-amplitude point
  sits on the nominal key edge (keeps timing honest for the decoder).
  If a mark is shorter than 2·rise the ramps shrink to fit.
- Segment boundaries → sample indices via cumulative time rounding, so
  long transmissions do not drift.
- QRM is not a separate type: it is another `Signal` at a pitch offset
  (helper `Signal::qrm(...)` documented in an example).
- Output clamped to [−1, 1]; `clipped()` counts clamped samples so a caller
  can lower `gain`.

### `decoder`
```rust
pub enum PitchMode { Fixed(f32), Auto { min_hz: f32, max_hz: f32 } }
pub struct DecoderConfig {
    pub sample_rate: u32,
    pub pitch: PitchMode,
    pub hop: Duration,            // default 4 ms
    pub window: Duration,         // default 12 ms (≈ 85 Hz Goertzel BW)
    pub on_threshold: f32,        // fraction of floor→peak span, default 0.55
    pub off_threshold: f32,       // default 0.40 (hysteresis)
    pub initial_wpm: f64,         // seed for the dot estimator, default 20
    pub squelch_db: f32,          // min peak-over-floor to decode at all
}
pub enum Decoded { Token(Token), Unknown(Vec<Element>), WordSpace }
pub struct Decoder;
impl Decoder {
    pub fn new(cfg: DecoderConfig) -> Result<Self, DecoderError>;
    pub fn feed(&mut self, samples: &[f32]) -> Vec<Decoded>;   // streaming
    pub fn flush(&mut self) -> Vec<Decoded>;                    // end of input
    pub fn pitch_hz(&self) -> Option<f32>;
    pub fn wpm_estimate(&self) -> Option<f64>;
    pub fn reset(&mut self);
}
pub fn decode_all(cfg: DecoderConfig, samples: &[f32]) -> Result<String, DecoderError>;
pub fn goertzel_power(samples: &[f32], sample_rate: u32, freq_hz: f32) -> f32; // reused by tests
```
Pipeline: sliding Goertzel (window/hop) at pitch → power in dB → noise
floor (slow min-follower) and peak (fast attack / slow release) trackers →
two-threshold hysteresis → mark/space edges timed to hop resolution.
Classifier: marks split dit/dah at the geometric mean of two adaptive
clusters (seeded from `initial_wpm`); spaces split intra / letter / word
using their **own** adaptive clusters (not fixed 2/5-dot cuts), because
Farnsworth letter gaps would otherwise read as word gaps. A trailing gap
longer than the letter threshold flushes the pending character without
waiting for the next mark. Unknown groups come out as `Decoded::Unknown`
(rendered `*` by `Display`).
Auto pitch: a Goertzel bank (10 Hz steps over `[min,max]`) on ~100 ms
windows; lock to the strongest persistent bin, re-acquire after the
squelch has been closed for a while. Pitch is exposed so a UI can show it.

### `scenario`
```rust
pub struct Callsign(String);
impl Callsign { pub fn parse(s: &str) -> Result<Self, ScenarioError>;
                pub fn as_str(&self) -> &str; }
pub enum ScenarioKind {
    RagChew,
    Contest(ContestExchange),          // CqZone | Serial | (cut numbers knob)
    Pota { park_to_park: bool },
    Sota { summit_to_summit: bool },
    DxPileup { callers: u8 },
}
pub struct Difficulty {
    pub wpm: (f64, f64), pub farnsworth: Option<f64>,
    pub fists: FistRange, pub strength_db: (f32, f32),
    pub pitch_offset_hz: (f32, f32),
    pub repeat_requests: f64,           // prob. of QRZ? / AGN / PSE RPT inserts
}
pub struct ScenarioConfig { pub kind: ScenarioKind, pub my_call: Callsign,
                            pub difficulty: Difficulty, pub seed: u64 }
pub struct SignalProfile { pub pitch_offset_hz: f32, pub speed: Speed,
                           pub fist: Fist, pub fist_seed: u64, pub strength_db: f32 }
pub struct Station { pub call: Callsign, pub name: String, pub qth: String,
                     pub rig: String, pub signal: SignalProfile, /* zone, ref… */ }
pub enum Party { Them(usize /* index into stations */), You }
pub enum Start { AfterPrevious { gap: Duration }, WithPrevious { delay: Duration } }
pub struct Transmission { pub from: Party, pub text: String, pub start: Start }
pub struct Script { pub kind: ScenarioKind, pub my_call: Callsign,
                    pub stations: Vec<Station>, pub transmissions: Vec<Transmission> }
impl Script { pub fn signal_for(&self, t: &Transmission) -> Option<&SignalProfile>; }
pub fn generate(cfg: &ScenarioConfig) -> Result<Script, ScenarioError>;
```
- Every `text` is codec-valid by construction (checked in debug + tested).
- Callsign generator from real structure: US 1x2 (K/N/W + digit + 2),
  2x1 (AA–AL/KA–KZ/NA–NZ/WA–WZ + digit + 1), 2x2, 1x3, 2x3; KL7/KH6
  occasionally; DX from a curated prefix table (G, M, DL, F, I, EA, ON,
  PA, OH, SM, LA, OK, SP, HA, S5, 9A, EI, GM, OE, HB9, CT, JA, VK, ZL,
  VE/VA, XE, LU, PY, ZS, UA, …) each with country, continent, CQ zone and
  QTH list. Blocklist for special calls (W1AW etc.) and `my_call`.
- Kinds: rag-chew (CQ, answer, RST/name/QTH, rig/ant, WX, 73/<SK>/TU EE);
  contest (`CQ TEST`, `5NN 4` zone or `5NN 123` serial, optional cut
  numbers T/N, `TU`); POTA (`CQ POTA`, `UR 5NN MN`, park refs in ISO
  form `US-1234`/`CA-0456`/…, never the retired `K-` form; P2P); SOTA
  (`CQ SOTA`, `W0C/FR-001`-style refs, S2S); DX pileup (N callers with
  `Start::WithPrevious` overlap, distinct pitch/speed/strength, partial
  calls `W1?`, `AGN`, `QRZ?`).
- `you` lines are what the operator would send (unused by copy practice,
  the target text for sending practice later).

### `score`
```rust
pub struct ScoreOptions { pub spaces: SpacePolicy /* Score | Ignore */,
                          pub fold_equivalents: bool /* D3, default true */ }
pub enum Unit { Token(Token), Space }
pub enum AlignOp {
    Match { expected: Unit },
    Substitution { expected: Unit, copied: Unit },
    Deletion { expected: Unit },
    Insertion { copied: Unit, attributed_to: Option<usize> /* expected idx */ },
}
pub struct Alignment { pub ops: Vec<AlignOp> }
pub struct Score { pub expected: usize, pub matches: usize,
                   pub substitutions: usize, pub deletions: usize, pub insertions: usize }
impl Score { pub fn cer(&self) -> f64; }      // (S+D+I)/expected
pub fn align(expected: &str, copied: &str, opts: &ScoreOptions)
    -> Result<Alignment, CodecError>;         // copied text may contain anything:
                                              // unknown copied chars are kept as
                                              // Unit::Token(Token::Char(c)) — see D6
impl Alignment { pub fn score(&self) -> Score; }
pub struct CharTally { pub sent: u32, pub correct: u32, pub missed: u32,
                       pub inserted_near: u32, pub confused_as: BTreeMap<Unit, u32> }
pub struct CharStats { /* BTreeMap<Unit, CharTally> */ }
impl CharStats { pub fn record(&mut self, a: &Alignment); pub fn merge(&mut self, o: &CharStats);
                 pub fn get(&self, u: Unit) -> Option<&CharTally>;
                 pub fn iter(&self) -> impl Iterator<Item = (&Unit, &CharTally)>;
                 pub fn weakest(&self, n: usize) -> Vec<(Unit, f64)>; }
```
Levenshtein DP with traceback and a fixed tie-break (match/sub, then
deletion, then insertion) so attribution is deterministic. O(n·m) memory;
documented as "score per transmission" (a line is ≤ a few hundred units).
Insertion attribution: to the nearest expected unit, preferring the one
whose op is a substitution (catches "N copied as ET"), else the preceding
one, else the first (D6).

## Tests (TDD — each written failing first)

codec
- whole-table round-trip: for every entry, `decode_group(pattern(t)) ==
  canonical(t)`; every pattern unique except the four documented
  collisions (asserted as exactly that set).
- `tokenize("cq de <ar> k")`, case folding, whitespace collapse.
- `UnknownChar` with offset for `#`, `UnknownProsign` for `<ZZ>`,
  `UnterminatedProsign` for `<AR`.

timing
- `Speed::wpm(20).dot() == 60 ms` exactly; letter 180, word 420.
- PARIS keyed with `Fist::perfect()` at 20 wpm totals exactly 3.000 s
  (50 units incl. trailing word gap).
- Farnsworth 18/10: letter/word gaps match the ARRL formula; chars still
  at 18 wpm.
- Same seed → identical `Keying`; different seed → different; `sloppy()`
  never yields a segment below the floor; jittered mean dash:dot ≈ ratio
  over 10k elements.
- Invalid speeds → `TimingError`.

synth
- No discontinuities: max |x[n]−x[n−1]| ≤ 2π·f/fs·A·(1+ε) over a whole
  render; first and last sample of every mark are ≈ 0.
- Ramp: hard-keyed vs 5 ms ramp, out-of-band energy (pitch ± 300 Hz+,
  via Goertzel) is ≥ 20 dB lower with the ramp (key-click test).
- Sample count == round(total · fs); no cumulative drift over 10 min.
- AWGN: measured noise power in ref bandwidth within ±0.5 dB of the
  stated SNR. QSB depth measured on the envelope. QRN crash count ≈ rate.
- Same seed → bit-identical PCM (same platform); streaming `fill` in
  odd-sized blocks == one-shot `render`.

decoder (tests/roundtrip.rs; texts include prosigns and punctuation)
- clean, Fixed pitch, 15/20/30 wpm → exact text.
- clean, Auto pitch → exact text; detected pitch within ±15 Hz.
- AWGN at a moderate SNR (proposed +6 dB in 500 Hz) → CER ≤ 2 %
  over a ~200-char text, several seeds.
- `Fist::typical()` at 20 wpm → CER ≤ 2 %; Farnsworth 18/8 → word
  spaces correct.
- QRM 400 Hz off with Fixed pitch → target still decoded.
- streaming: arbitrary block sizes (1, 37, 4096) give identical output.

scenario
- same seed → equal `Script`; different seeds → different.
- 1000 seeds × every kind: every `text` passes `codec::encode`; every
  station call parses; no station equals `my_call`; POTA refs match
  `^[A-Z]{2}-\d{4,5}$`; SOTA refs match the association/region form.
- pileup: ≥ 2 `WithPrevious` transmissions with distinct pitch offsets.
- `my_call` invalid → `ScenarioError`.

score (hand-made)
- identical → all matches, CER 0.
- `CQ`/`CO` → one substitution on `Q`; `TEST`/`TST` → deletion on `E`;
  `5NN`/`5NNN` → insertion; `N`/`ET` → substitution + insertion attributed
  to `N`; `<BT>`/`=` → match when folding, substitution when not.
- `CharStats::merge` is associative over three alignments.
- `serde` feature: `CharStats` and `Script` JSON round-trip.

## Phases (one at a time, report after each if the architect wants)

1. Scaffold crate + workspace member + `rng` + `codec` (tests first).
2. `timing`.
3. `synth` (tone, then channel impairments, then streaming renderer).
4. `decoder` (fixed pitch, then classifier adaptivity, then auto pitch).
5. `scenario` (callsign + data tables, then each kind).
6. `score` + `serde` feature.
7. Docs: crate docs, `CLAUDE.md` architecture/diagram rows, README crate
   count; ADR 0021 if the architect wants one; CI step for the feature.

## Verification before "done"

- `cargo test -p cat-morse` and `cargo test -p cat-morse --features serde`
- `cargo test --workspace` (nothing else broken)
- `cargo clippy -p cat-morse --all-targets --all-features -- -D warnings`
  (includes `incompatible_msrv`, our MSRV-1.75 signal — no 1.75 toolchain
  is installed locally) and `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo xwin check --target x86_64-pc-windows-msvc -p cat-morse --all-features`
  (cargo-xwin is installed); CI windows-latest runs the tests.
- `cargo tree -p cat-morse`: no local crates, no monoio/tokio/rand.
- Decoder round-trip tests finish in reasonable time in debug
  (target < 10 s total; otherwise downscale to 8 kHz in tests).

## Decisions Made

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | Zero local deps; only `thiserror` (+ optional `serde`). | Bottom-of-graph utility like `cat-layout`; trivially Windows-portable. |
| D2 | Determinism contract: `Keying` and `Script` are bit-identical across platforms per seed; PCM is bit-identical per platform and equal across platforms within tolerance. | Integer PRNG + IEEE basic ops are exact everywhere; `sin/cos/powf` go to platform libm. Gaussian via Irwin–Hall avoids `ln/cos` in keying. Avoids a `libm` dependency just to make PCM bit-exact, which nothing needs. |
| D3 | The four colliding patterns (`=`/`<BT>`, `+`/`<AR>`, `(`/`<KN>`, `&`/`<AS>`) decode to one canonical form chosen by a `DecodeStyle` knob (default per Q1); `canonical()` folds each prosign↔char pair and the scorer folds by default. | The code itself is ambiguous; scoring must not punish a correct copy. |
| D4 | SNR is stated against a reference bandwidth (default 500 Hz, configurable). Noise is generated white over Nyquist and scaled so its power in the reference bandwidth hits the SNR; an optional `RxFilter` band-limits it like a CW filter. | "SNR" is meaningless without a bandwidth; 500 Hz matches a CW receive filter (WSJT's 2500 Hz convention is noted as the alternative, Q2). |
| D5 | `serde` as a **default-off** optional feature on plain-data types (Token, Prosign, Speed, Fist, Script and parts, Score, Alignment, CharStats), plus a CI step running `cargo test -p cat-morse --features serde`. | Orphan rule: caller cannot derive on our types. Default-off keeps the crate lean; the CI step answers the "default-off code is code CI doesn't compile" lesson recorded in `cat-signal-audio/Cargo.toml`. Pure Rust, so default-on is also defensible (Q3). |
| D6 | `align` accepts arbitrary copied text (an operator can type `#`); only the **expected** text must be codec-valid. Insertions are attributed to an expected unit (rule above). | Scoring must never fail on what a human typed; every error gets a home in per-char stats. |
| D7 | QRM is just another `Signal`. | One mixing path; a pileup is the same thing at scale. |
| D8 | No `rustfft`; Goertzel + Goertzel bank. | Single known pitch needs one bin; auto-pitch needs ~90 bins at 10 Hz — cheap, no new dep, no FFT block latency. |
| D9 | POTA refs in ISO form (`US-1234`); SOTA `W0C/FR-001` form. | Matches current program practice (see progress/hand-off for sources). |

## Open questions for the architect

- Q1: For the colliding patterns, should `decode_group` emit the char
  (`=` / `+` / `(` / `&`) or the prosign (`<BT>` / `<AR>` / `<KN>` /
  `<AS>`)? Proposal: prosign for BT/AR/KN (ham usage in QSOs), `&` → `<AS>`
  too, with a `DecodeStyle` knob. Scoring folds either way.
- Q2: SNR reference bandwidth default — 500 Hz (CW filter) or 2500 Hz
  (WSJT convention)?
- Q3: `serde` default-off + CI step (proposed) vs default-on?
- Q4: Word spaces in CER — scored by default (proposed) or ignored?
- Q5: Does a new crate warrant ADR 0021 and a `.claude/agents/cat_morse.md`?
  Proposal: yes to a short ADR (scope + "no device audio, no radio" rule +
  determinism contract D2); agent file at the architect's discretion.
- Q6: Workspace version is shared (0.8.0); adding a crate is additive —
  confirm no version bump is expected from this task.

## Findings

(findings.md was refused by the Write tool for subagents; the architect
directed findings to live here.)

- Conventions: workspace package fields, Apache-2.0 header per file, comment
  each dependency in Cargo.toml (`cat-signal-audio` pattern).
- CI clippy/test run without `--all-features`; default-off features need an
  explicit CI step (precedent: `cargo check -p cat-signal-rtlsdr --features device`).
- No 1.75 toolchain locally; clippy `incompatible_msrv` is the MSRV signal.
- POTA: ISO country prefixes since March 2024 (`K-0001` → `US-0001`,
  `VE-` → `CA-`), 4 digits zero-padded, 5 where needed.
- SOTA: `ASSOC/RR-NNN` (`W0C/FR-001`, `G/LD-001`, `VE7/GV-001`).
- Morse pattern collisions: `=`/BT, `+`/AR, `(`/KN, `&`/AS.
- Farnsworth (ARRL): `ta = (60c − 37.2s)/(s·c)` s; letter gap `3ta/19`,
  word gap `7ta/19`.
- Determinism: integer PRNG + IEEE basic ops + sqrt are bit-exact
  everywhere; libm transcendentals are not.

## Errors Encountered

| Error | Attempt | Resolution |
|-------|---------|------------|
| Write tool refused `findings.md` for a subagent ("return findings as text") | 1 | Did not work around it; findings delivered in the hand-off report instead. Architect to decide whether to create it. |
