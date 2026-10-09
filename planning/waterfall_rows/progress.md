# Progress
- Worktree created at ../radio-cat-rs-waterfall from origin/main (40cfade).
- Added pure `waterfall_upload_plan` + `WaterfallUpload` in app.rs, stub
  reproducing old behaviour (one row at head); draw call uses it.
- Tests in `app::waterfall_upload_tests`: 3 of 4 FAILED against the stub
  (every_row_pushed_between_uploads_reaches_the_gpu failed at round 0 with
  the skipped row all zeros on the simulated GPU).
- Fix: upload the generation-delta rows from head as <=2 contiguous runs;
  Whole when pending >= height, on rebuild, or with no texture. 84/84 pass.
- clippy --workspace --all-targets -D warnings: clean. fmt --check: clean.
