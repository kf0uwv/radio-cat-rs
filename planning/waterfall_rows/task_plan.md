# Task: waterfall rows pushed but never uploaded

## Goal
Every waterfall row pushed between two texture uploads reaches the GPU.

## Phases
1. [x] Worktree `fix/waterfall-rows` off origin/main; read CLAUDE.md.
2. [x] Factor upload bookkeeping into a pure `waterfall_upload_plan`
       (in cat-ui-egui/src/app.rs) that reproduces current behaviour
       (one row at `head`). Wire draw code to it.
3. [x] Failing test: push 2+ rows between uploads, apply plan to a
       simulated GPU buffer, assert it equals `pixels()`.
4. [x] Fix: plan = pending rows (generation delta) as <=2 contiguous ring
       regions; whole image on first upload / rebuild / pending >= height.
5. [x] cargo test -p cat-ui-egui, clippy -D warnings, fmt --check; commit.

## Decisions
- Keep the 33 ms throttle: once no row is lost it only coalesces uploads.
