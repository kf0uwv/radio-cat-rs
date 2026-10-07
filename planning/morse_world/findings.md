# morse_world — findings

## Current state (feat/cat-morse @ a01477b)
- `scenario/data.rs`: `US_AREAS` (10 call areas, states, towns, CQ zone), `DX: &[DxEntity]`
  (~30 entities: prefixes, `add_digit`, `suffix_len`, `zone`, `country`, `iso` for POTA, `towns`),
  `NAMES_US`, `NAMES_DX` (20 generic EU/JA names, no per-region set).
- `scenario/callsign.rs`: `Callsign::parse` already accepts `/` (3–14 chars, prefix/suffix
  forms); `base()` = longest `/` piece; `home_of` = longest DX prefix match on `base()`, then
  US prefix test; `dx_call` = prefix + optional digit 1–9 + suffix letters; `fresh_call` dedups on base.
  Bug-ish: `home_of("VE3/W1XYZ")` looks at `W1XYZ` (base) — the *operating* entity is the `VE3/`
  prefix. Must change for slash calls: a prefix/suffix designator that is itself a prefix wins
  (`EA8/DL1ABC` → Canary Is., `W1ABC/KH6` → Hawaii, `/P` `/M` `/QRP` ignored, `/MM` → no entity, `/AM`).
- Uses of `DX`: band.rs:328 (QRM station call), qso.rs 147 (tests), 229 (random home), 243,
  468-489 (POTA iso), persona (qso.rs:86) takes `e.towns`, `e.country`, `e.zone`.
- `Persona.zone` is CQ zone; contest exchange `ContestExchange::CqZone` uses persona.zone.
  Only one zone per entity today — wrong for multi-zone entities (W 3/4/5, VE 1-5, UA 16-19/…, VK 29/30, JA 25, BY 23/24, PY 11/13).
- `Difficulty.band` is `BandConditions` (SNR/QSB/QRN/QRM/filter), NOT an HF band. Propagation-aware
  weighting needs a new optional field (e.g. `Difficulty.hf_band: Option<HfBand>`), additive.
- score.rs: `codec::tokenize` handles `/` as a character (Morse `-..-.`); `align_unordered`
  matches whole words; a slash call is one word, plausibility threshold = half its length,
  so `EA8/DL1ABC` copied as `DL1ABC` is a plausible match with 4 deletions. That is correct
  scoring (the operator missed the prefix) — no change needed, but add a test pinning it.
  `ScoreOptions` may want an opt-in "base call only" leniency — open question, not planned.
- Rng: in-crate xoshiro; any change to table order/length changes seeded output. ADR 0021
  promises determinism per version, not across versions — confirm; golden-seed tests in
  mod.rs will need re-baselining once (call that out in the commit).
- No serde on data tables; Home is pub(crate). Keep all new types pub(crate) except an
  optional public `HfBand`/`Continent` if exposed via Difficulty.

## Data source & licence
- cty.dat (country-files.com, Jim Reisert AD1C): fetch of the format/licence page returned 403;
  search found no explicit licence. It is distributed freely and bundled by loggers (tlf, N1MM,
  etc.), but there is NO stated licence grant, so it cannot be assumed Apache-2.0-compatible
  for redistribution in this repo. Treat as "all rights reserved" until the maintainer confirms.
- Underlying facts (ITU prefix blocks — ITU RR Appendix 42; DXCC entity names/numbers; CQ/ITU
  zones) are facts, not copyrightable. ARRL's DXCC List document is copyrighted as a document,
  but the entity facts are not.
- Recommendation: hand-built table from public facts (ITU Appendix 42 blocks + DXCC entity list
  + zone maps), written as a checked-in `world.rs`. Optionally, a dev-only checker script
  (`scripts/check-world-vs-cty.py`, user supplies their own cty.dat path, not committed) that
  diffs our prefix→entity mapping against cty.dat to catch errors. No build.rs, no network.

## Size estimate
- ~340 entities × (prefix list ~30 B + format/zone fields ~16 B + 2–4 towns ~40 B + name ~12 B
  + struct/slice overhead ~80 B) ≈ 60–70 KB rodata. Name pools: ~15 regional sets × 30 × 7 B
  ≈ 3 KB. District tables (JA 0–9, VE1–VY0, UA districts, VK, ZL, PY, LU, BY, W already) ~5 KB.
  Budget: ≤ 100 KB added to the cat-morse rlib's rodata; enforce with a test on table sizes
  (counts), not on binary bytes.

## Task 2 — full DXCC table (2026-10-07)
- WORLD had 341 entries; resolved (fix 1): Malyj Vysotskij (151) is deleted per the ADIF 3.1.5
  DXCC enumeration (diffed all 340 current codes against the table). Removed; now 340, pinned by a test.
  Unique DXCC numbers are enforced by a test, but a test cannot check that a number is right.
- Uncertain facts, flagged for verification (not silently guessed):
  - **Tagged entities are a simulation convention**, not real allocations: Rotuma
    `3D2../R`, Conway Reef `3D2../C`, North Cook `E51../N` (South Cook is plain E51),
    Austral `FO0../A`, Marquesas `FO0../M`, Clipperton `FO0../C`, Chesterfield `FK8../C`,
    Ducie `VP6../D`, Malpelo `HK0../M` (San Andres is plain HK0), the VP8 dependencies
    `/G /O /S /H`, Bouvet `3Y0../B`, Peter I `3Y0../P`, Heard `VK0../H`, Macquarie
    `VK0../M`, Swains `KH8../S`, Minami Torishima `JD1../M`. Real operations use
    one-off calls (3Y0J, VP8PJ, TX5S, ...) that cty.dat lists by exact call.
    Mount Athos `SV2xxx/A` is the real form.
  - Kaliningrad = any Russian prefix + digit 2. The real rule depends on the
    suffix's first letter (e.g. UA2F/UA2K); approximate.
  - European Russia digits taken as 1/3/4/6/7; Asiatic as 8/9/0. Russian districts
    and zones (CQ 16/17/18/19/23, ITU 19-35) are one default per entity until task 3.
  - Franz Josef Land `R1FJ`/`RI1FJ`/`UA1FJ`, Malyj Vysotskij `R1MV`, Antarctica
    `KC4AA`/`KC4US`/`RI1AN`/`8J1R`/`CE9`: literal prefixes, partial.
  - Guantanamo: fixed (fix 2) — `KG4` + exactly two letters (EXACT_SUFFIX rule); KG4 1x/x3 suffixes are US.
  - Turkey continent set EU (cty.dat's choice as remembered; DXCC lists it
    under Asia). Cyprus and UK bases set AS.
  - Some single-digit choices are "most common" rather than exhaustive
    (e.g. `6Y5`, `J68`, `FK8`, `TR8`, `5Z4`).
  - CQ/ITU zones of multi-zone entities (US, VE, VK, UA, BY, PY, LU, JA, ...) are
    one default per entity until task 3's regions.
- Mechanism added: `Entity::tag`, `ent!` row macro + `cf`, `call_from` redraw
  (≤64) so generated calls always round-trip, `fresh_call` → Option (≤256 draws)
  with `add_station` moving to another DX home when a one-call entity (4U1UN,
  4U1ITU, BS7H) is exhausted.
- world.rs ≈ 3000 lines source; rodata well under the 100 KB budget (no size test yet — task 9).

## Task 3 — call areas with their own zones (2026-10-07)
- Regions now carry optional `prefixes` (Canada: VE1 NS vs VO1 NL vs VY1 YT), looked up by
  prefix + digit; `FromRegion` formats draw the area's own letters.
- Areas added: Canada (14), Japan (0-9), European Russia (1/3/4/6/7), Asiatic Russia
  (9/8/0), China (BY0-9), Australia (VK1-8), New Zealand (ZL1-4), Brazil (PY1-9); US had 10.
- Persona region: a two-letter subdivision code (US state, Canadian province) is sent;
  any longer area name leaves the region as the entity name (JA, UA, VK, ...).
- Uncertain, flagged:
  - Canada ITU: VE2 QC = 4 (eastern Quebec is 9), VE8 NT = 2 (geographic; may be 4 in
    cty.dat), VY0 NU = 4 (spans 2/4/75). CQ values are the usual ones.
  - Russia: one zone per digit is coarse — real zones follow the oblast (suffix letter).
    UA0 taken as Far East CQ 19 / ITU 34 (Krasnoyarsk/Irkutsk are CQ 18 / ITU 32);
    UA8 as Siberia CQ 18 / ITU 31; Volga (4) ITU 29 (parts are 30).
  - China BY9 taken as CQ 23 / ITU 43 (Shaanxi is CQ 24); BY2 ITU 33.
  - Brazil PY9 (Centro-Oeste) ITU 15 (Mato Grosso is 12); PY8 ITU 13 (Amazonas is 12).
  - Japan 7K-7N (always Kanto, digits 1-4) are no longer generated; `home_of` still
    reads 7K3ABC as area 3. Argentina LU areas left out: digit-to-province mapping not
    known with confidence.
  - Regions are drawn uniformly (VY0 as often as VE3) until task 4's weighting.

## Task 4 — activity, home continent, area weighting (2026-10-07)
- `Activity` per entity: Common 100 / Regular 25 / Uncommon 5 / Rare 1 (14 Common, 54 Regular,
  77 Rare, rest Uncommon). Tiers are judgement, not measured spot data — a later pass could
  calibrate from public spot/log statistics (e.g. Club Log most-wanted) if wanted.
- `random_dx(me)`: weight = activity x NEAR_BOOST (3) on the operator's continent; the
  operator's own entity is never DX. `random_local(me)`: the operator's entity (US if unknown).
  `add_station` uses local/DX by the existing `dx_share`, so a DL operator now hears mostly DL
  locals (was: US) — intended.
- Call-area weights: US (W4 20 ... W1/W3/W9 8), VE3 40 vs VY0/VY1/VO2/VE8 1, JA1 30, UA3 30,
  UA9 20, VK2/VK3 25, PY2 30, etc.
- Band QRM stations keep a US-centred draw (`random_dx(None)`), since the band model has no
  operator call.
- Distribution tests are seeded (fixed `Rng::new(n)`), so deterministic: Common share > 30%,
  Rare < 5%, every non-US entity drawn in 100k, JA operator's Asia share > 1.5x neutral,
  VE3 > 10x VY0, JA1 > 20%.

## Task 5 — regional names (2026-10-07)
- `Names` enum in data.rs: 30 pools (Us, Anglo, German, French, Italian, Iberian, LatAm,
  Lusophone, Nordic, Finnish, Baltic, Dutch, WestSlavic, EastSlavic, Danube, Greek, Turkish,
  Caucasus, Albanian, Arabic, Hebrew, Persian, Indian, Japanese, Chinese, Korean, SeAsian,
  Mongol, African, Expedition), 5-20 common given names each, ASCII and Morse-safe.
  Only plausible first names; no callsign is ever paired with a real person's name.
- Every entity names its pool (`Entity::names`); uninhabited/DXpedition-only entities use
  the international Expedition pool, as do operators whose call has no known home.
- Towns were already per entity and per call area (tasks 2-3); unchanged here.
- Mixed-language countries take one pool (Belgium Dutch, Canada Anglo, Switzerland German,
  Luxembourg German) — a simplification.

## Task 6 — portable and slash calls (2026-10-07)
- `station_call` (random stations only; activations keep plain calls and add /P themselves):
  4% `/P` `/M` `/QRP`, 3% signed abroad CEPT-style (`F/DL1ABC`, `EA8/G4ABC`, `VE3/W1AB`;
  the licence drawn by `random_dx` from the operating home, so mostly a neighbour), 3% of US
  stations out of area (`W1ABC/4`). Every form is checked to home back to the station's home;
  one that would not (e.g. `JD1AB/M` reads as Minami Torishima's tag) is sent plain.
  Tagged-entity calls are never decorated further.
- Rag-chew partner is maritime mobile 3% of the time: base call + `/MM`, QTH "AT SEA",
  region "MM", no home.
- Name pool of a visitor follows the operating entity, not the licence (simplification).
- Scoring unchanged and strict, pinned by tests: dropping `EA8/` costs 4 deletions; in a
  pileup the bare base still pairs with its slash call and is charged the same 4.
