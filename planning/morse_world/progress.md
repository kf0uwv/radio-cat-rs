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
