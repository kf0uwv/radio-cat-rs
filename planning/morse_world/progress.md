# morse_world — progress

## 2026-10-07
- Created worktree `../radio-cat-rs-world` on `feat/morse-world-calls` from `feat/cat-morse`.
- Read scenario/{data,callsign,qso,band,mod}.rs, score.rs align_unordered.
- Licence check on cty.dat: page 403, no licence found → recommend hand-built table.
- Wrote task_plan.md / findings.md. Awaiting approval; no code written.
- Architect approved plan; decisions recorded in task_plan.md.
- Task 1 done: `scenario/world.rs` (Continent, DigitRule, CallFormat, Region, Entity, WORLD = USA
  + 29 seed entities with DXCC numbers, continent, CQ+ITU zones); US folded in as entity 291 with
  10 call-area regions; US 2-letter blocks no longer issue KH/KL/KP/NH.../AH/AL (lower 48 only).
  `Home` is now `{ entity, region }`; `home_of` honours location designators (`F/DL1ABC`,
  `VE3/W1XYZ`, `W1ABC/4`), ignores /P /M /A /B /QRP, returns None for /MM /AM and unknown
  designators. `call_from` draws weighted formats. data.rs lost US_AREAS/DX.
- Re-baselined `script_is_pinned_across_platforms` (first call G7UQZ -> M1PZU, FNV 0x8dc99b110efd7e04),
  approved decision 2.
- Verified: cargo test -p cat-morse --all-features: 98 lib + 18 + 10 + 1 doc = 127 pass
  (was 87+18+10+1 = 116); default features 96+17+10+1; clippy --all-targets --all-features clean;
  fmt --check clean.
- Task 2 done: mechanism commit (tags, row macro, bounded draws) + one commit per
  continent (EU 70, NA 49, SA 30, AF 76, AS 54, OC 59, AN 2; 341 rows incl. US).
  Pinned pileup script re-baselined each commit. Per-continent home tests added.
  Final: --all-features 108 lib + 18 + 10 + 1 doc = 137 pass; default 106 + 17 + 10 + 1;
  clippy clean; fmt clean. Uncertainties recorded in findings.md.
- Fix 1: Malyj Vysotskij (151) is deleted per ADIF 3.1.5; removed; count pinned at 340.
- Fix 2: Guantanamo = KG4 + exactly 2 letters (EXACT_SUFFIX); KG4A/KG4ABC are US area 4.
- Task 3 done: region prefixes, 8 entities' call areas with zones, persona region rule.
  --all-features 110 lib + 18 + 10 + 1 doc = 139 pass; default 108 + 17 + 10 + 1; clippy, fmt clean.
- Task 4 done: Activity tiers, NEAR_BOOST continent bias, random_local, weighted call areas.
  --all-features 114 lib + 18 + 10 + 1 doc = 143 pass; default 112 + 17 + 10 + 1; clippy, fmt clean.
- Task 5 done: 30 regional name pools, persona draws from the entity's pool.
  --all-features 116 lib + 18 + 10 + 1 doc = 145 pass; default 114 + 17 + 10 + 1; clippy, fmt clean.
  task_plan task 8 now also covers the band-noise US-centred draw.
- Task 6 done: station_call portable/abroad/out-of-area forms, /MM rag-chews, strict slash
  scoring tests. --all-features 119 lib + 18 + 10 + 1 = 148; default 117 + 17 + 10 + 1; clippy, fmt clean.
- Task 7 done: DxChase kind, world-wide callers for DxPileup.
  --all-features 121 lib + 18 + 10 + 1 = 150; default 119 + 17 + 10 + 1; clippy, fmt clean.
- Task 8 done: HfBand path model, Difficulty::hf_band, band_channel_for. --all-features
  124 lib + 18 + 10 + 1 = 153; default 122 + 17 + 10 + 1; clippy, fmt clean; workspace check clean.
- Task 9 done: WORLD static (size fix), table-size guard test, module docs, ADR 0021 amendment.
- Review fixes (planning/code_review 2026-10-07), one commit each, test first:
  1 modifiers only after the base (M/ MM/ B/ are prefixes); 2 base = longest digit-bearing
  non-modifier piece, call over bare prefix on ties; 3 digit-led prefixes take the next digit
  as area; 4 CT9, FTnY/ZS7/R1AN, 4U1VIC, China 3H-3U/XS, KG4 as a designator; 5 DxChase
  fallback avoids the DX's entity. --all-features 130 lib + 18 + 10 + 1 = 159; default
  128 + 17 + 10 + 1; clippy, fmt clean.
