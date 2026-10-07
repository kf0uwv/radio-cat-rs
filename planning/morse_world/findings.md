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
