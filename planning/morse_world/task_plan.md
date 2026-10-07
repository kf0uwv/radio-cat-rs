# morse_world — task plan: "simulate many calls from all over the world"

Status: APPROVED 2026-10-07 (architect). Tasks 1-2 done; awaiting go for task 3. Branch `feat/morse-world-calls` off
`feat/cat-morse` (PR #17), worktree `../radio-cat-rs-world`. Scope confined to
`cat-morse/src/scenario/` (+ this planning dir). No changes to score.rs/codec.rs unless an
open question is answered that way. Parallel agent on `feat/cw-capability` — no overlap expected.

## Goal
Generated QSOs feature callers from the full current DXCC list (~340 entities) with real prefix
structure, correct CQ/ITU zones, continent, activity weighting, continent-appropriate names/QTHs,
rare-DX pileups, and an operator-home-aware mix — all deterministic and offline.

## Data shape (new `scenario/world.rs`, hand-built, checked in)
```rust
pub(crate) enum Continent { NA, SA, EU, AF, AS, OC, AN }
pub(crate) struct Entity {
    pub dxcc: u16,                 // ARRL entity number (identity, Eq/Debug)
    pub name: &'static str,        // "CANARY IS" — Morse-safe upper case
    pub continent: Continent,
    pub cq: u8, pub itu: u8,       // default zones
    pub prefixes: &'static [&'static str],   // home_of matching (longest wins), e.g. ["EA8","EB8",..]
    pub formats: &'static [CallFormat],      // weighted generation shapes
    pub regions: &'static [Region],          // districts/provinces; may be empty
    pub activity: Activity,        // Common | Regular | Uncommon | Rare | Dxpedition
    pub names: NamePool,           // index into regional name sets
    pub iso: Option<&'static str>, // POTA (kept from DxEntity)
    pub towns: &'static [&'static str],      // used when regions empty
}
pub(crate) struct CallFormat { prefix: &'static str, digits: DigitRule, suffix: (u8,u8), weight: u8 }
pub(crate) enum DigitRule { InPrefix, Any, OneOf(&'static [u8]), FromRegion }
pub(crate) struct Region { digit: Option<u8>, code: &'static str, cq: u8, itu: u8, towns: &'static [&'static str] }
```
- US moves into the same model (entity 291, regions = today's US_AREAS) so there is one table;
  `Home` becomes `Home { entity: &'static Entity, region: Option<&'static Region> }`.
- Regions where it matters: JA 0–9 (+JD1 separate entities), VE1–VE9/VY/VO, UA1–UA0 / R
  districts (EU vs AS Russia are separate entities: UA1-7 vs UA8-0, UA2 Kaliningrad), VK1–8,
  ZL1–4, PY1–9, LU, BY, W/K/N/A 0–9 incl. KL7/KH6 as own entities.
- Portable forms: `EA8/DL1ABC`, `DL1ABC/P`, `/M`, `/QRP`, `W1ABC/KH6`, `/MM` (maritime, no
  entity, zone from ocean — keep simple: rare, only in rag-chew). Generated with small weights.

## Realism
- Activity weights: Common (DL, G, I, EA, F, JA, W, VE, UA, SP, OK, PA, ON, LY…) ≫ Regular ≫
  Uncommon ≫ Rare (e.g. 3Y, FT5, KH1, P5, BS7). Rare/Dxpedition entities appear mainly as the
  DX in `DxPileup` and occasionally as a CQ caller.
- Operator home: `home_of(my_call)` sets the "near" continent; same-continent stations get a
  boost (EU op hears mostly EU; W op hears W/VE + EU/JA). Contest: callers send their real CQ
  zone (region-specific).
- Optional propagation: new additive `Difficulty.hf_band: Option<HfBand>` (160…10 m). If set,
  a crude path model: 160/80 favour same continent; 20/15/10 favour long-haul. `None` = today's
  behaviour with the new world mix. (No real propagation prediction.)
- Names: ~15 regional pools (anglo, german, romance-IT/ES/PT, french, slavic, nordic, JA, chinese,
  arabic, latin-american, african-anglo, …), all Morse-safe ASCII; QTH = region/entity towns.
- Pileup: `DxPileup` gains "you are the rare DX" vs "you chase the rare DX" variants? (open Q).

## Scoring/codec
- No code change expected: `/` is a Morse character; a slash call is one word. Add tests pinning:
  `EA8/DL1ABC` vs copied `DL1ABC` → plausible match, prefix deletions; numerals in prefixes
  (`9A2AB`, `4X1AB`, `3DA0XX`) align as normal characters.

## Determinism & tests
- Seeded output stays reproducible; table change re-baselines golden seeds once (noted in commit).
- Tests: every entity's every format generates calls that `Callsign::parse` and `home_of` maps
  back to that entity (and region); no two entities share an identical prefix string; zones in
  range (CQ 1–40, ITU 1–90); all strings Morse-encodable; distribution test (10k draws: common
  ≥ X%, rare ≤ Y%, every entity reachable); same-continent bias test; slash-call `home_of` tests.

## Tasks (TDD, one at a time, report after each)
1. [x] `world.rs` types + 20-entity seed (existing DX + US) behind the new model; `Home` refactor;
       `home_of` with slash precedence. Existing tests stay green (re-baseline seeds if needed).
2. [x] Round-trip + invariant tests, then fill the full DXCC list (~340) in batches by continent
       (EU, NA, SA, AF, AS, OC, AN) — one commit per continent.
3. [ ] Regions: JA, VE, UA/R, VK, ZL, PY, LU, BY; region-correct zones in contest exchange.
4. [ ] Activity weighting + operator-home continent bias; distribution tests.
5. [ ] Regional name pools + towns; persona uses them.
6. [ ] Portable/slash call generation (`EA8/`, `/P`, `/M`, `/MM`) + score tests pinning slash handling.
7. [ ] Rare-DX pileup scenario using Rare/Dxpedition entities.
8. [ ] (optional, needs approval) `Difficulty.hf_band` propagation weighting.
9. [ ] Size check test (table counts) + docs (scenario module docs, ADR 0021 note).

## Open questions
1. Data source: OK to hand-build from public facts (recommended) rather than vendor cty.dat?
   Or should someone ask AD1C for written permission?
2. Changing golden seeded outputs on this branch (PR #17 still open) — acceptable, or land after merge?
3. `hf_band` on `Difficulty` (public API addition) — wanted now?
4. Pileup: operator as rare DX only (current), or also chasing a rare DX through a pileup?
5. Scoring leniency for base-call-only copies of slash calls — keep strict (recommended)?
6. Deleted entities / special-event prefixes (e.g. `TM`, `GB`, `II`) — include as rare variants?

## Architect decisions (2026-10-07)
1. Hand-build the table from public facts; no cty.dat vendoring. Optional dev-only cross-check
   script is allowed but must never be required by build or tests.
2. Seeded outputs may change once — this branch is separate from PR #17; nothing released.
3. Add `Difficulty.hf_band` in task 8, not before.
4. Pileup: both — operator as rare DX working the pileup, and operator as chaser copying the DX.
5. Strict slash scoring (no base-call leniency).
6. Special-event prefixes (TM, GB, II, ...) as rare, low-weight variants.
