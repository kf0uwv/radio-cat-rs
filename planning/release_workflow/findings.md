# Findings: shared release workflow (Deliverable 5)

- Read ts570d/.github/workflows/{ci.yml,release.yml} and
  packaging/build-deb.sh in full. release.yml builds `-p serial` which no
  longer exists locally (extracted to this repo) -- stale/broken, not this
  repo's problem to fix, noted in the ADR for ts570d's own follow-on.
  build-deb.sh already expects a `target/release/pin-test` binary and
  installs it as `rs232c-pintest` -- confirms Deliverable 3's `[[bin]] name
  = "pin-test"` choice matches ts570d's existing packaging convention
  exactly.
- ft991a has no .github/ directory at all.
- Verified via GitHub's docs + community discussions (WebSearch/WebFetch,
  cited in the ADR): a cross-repo `workflow_call` reusable workflow
  automatically receives a `secrets.GITHUB_TOKEN` scoped to the CALLING
  repo (no `secrets: inherit` needed for it specifically), permissions can
  only be downgraded not escalated relative to the calling job's
  `permissions:` block, and `actions/checkout` with no `repository:` arg
  inside the reusable workflow checks out the CALLING repo. This confirms
  a workflow_call design is practical here (no unresolved secrets/
  permissions friction), so no composite-action fallback was needed.

## Decision
`.github/workflows/release-app.yml` (workflow_call), two jobs (linux,
windows), contract: consuming repo provides `packaging/build-deb.sh
[--skip-build]` (ts570d already has it) and a new
`packaging/build-windows-package.ps1` (new contract, neither app has this
yet), plus a `[[bin]]` matching `main_binary`. See
docs/adr/0008-shared-release-workflow.md for the full record and the exact
calling snippet.

## Verification limits
No network access to actually trigger a GitHub Actions run in this
sandbox. Verified: YAML parses correctly (python yaml.safe_load), the
bash `package:binary` parsing loop behaves as intended (tested directly).
Not verified: an actual end-to-end release run. Explicitly flagged in the
ADR that this cannot fire until a human pushes this repo's main branch
(uses: kf0uwv/radio-cat-rs/...@main resolves against the remote).
