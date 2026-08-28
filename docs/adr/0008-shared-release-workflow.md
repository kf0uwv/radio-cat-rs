# 8. Shared GitHub Actions release automation

Date: 2026-07-26

## Status

Accepted


> **Amended 2026-08-27 by [ADR 0012](0012-native-msvc-windows-target.md).**
> Every `x86_64-pc-windows-gnu` reference below is a historical record of
> how this work *was* verified at the time, deliberately left unedited.
> `-gnu` is retired: `x86_64-pc-windows-msvc` is now the only Windows
> target, and verification is `cargo check` **and `cargo test`** on a
> `windows-latest` runner.

## Context

Both sibling applications need to: build a Linux x86_64 release binary,
cross-build a Windows x86_64 release binary, build a Debian `.deb` package,
build a Windows package (zip and/or installer), and upload all four as raw
GitHub Release assets. `ts570d` already has this partially automated
(`.github/workflows/{ci.yml,release.yml}`, `packaging/build-deb.sh`, read in
full below); `ft991a` has no `.github/` directory at all yet.

### What was read before deciding

- `ts570d/.github/workflows/ci.yml` — a straightforward test/lint job
  (`cargo fmt --check`, `clippy`, unit + integration tests) on
  `ubuntu-latest`. Not this ADR's concern (it is not release automation),
  but confirms the toolchain pin convention (`dtolnay/rust-toolchain@stable`
  with an explicit `toolchain: "1.93"`) this ADR's workflow mirrors.
- `ts570d/.github/workflows/release.yml` — triggers on `release: published`,
  builds release binaries (`cargo build --release`, `-p emulator`, `-p
  serial` — the last one is now stale/broken, since `serial` no longer
  exists as a local crate post-extraction to this repo; not this ADR's
  problem to fix, but worth `ts570d`'s own follow-on noticing), runs
  `./packaging/build-deb.sh --skip-build`, uploads only the `.deb` via `gh
  release upload`, then bumps `ts570d`'s own `Cargo.toml` minor version on
  `main` via a bot commit (an app-specific policy choice, not a build-artifact
  concern — deliberately **not** absorbed into this ADR's shared workflow;
  each app keeps that step, if it wants it, in its own thin caller).
- `ts570d/packaging/build-deb.sh` — stages a `DEBIAN` control-file tree,
  installs `ts570d`→`ts570d-control`, `emulator`→`ts570d-emulator`, and
  **already** `pin-test`→`rs232c-pintest` (confirming this repo's own
  Deliverable 3 choice — `cat-transport-serial`'s new `[[bin]] name =
  "pin-test"`, [ADR 0006](0006-windows-network-transport.md) §6 — matches
  `ts570d`'s pre-existing packaging convention exactly), writes a DEP-5
  copyright file from `LICENSE.txt`, and `dpkg-deb --build`s into the repo
  root as `*.deb`. Takes `--skip-build` to skip its own `cargo build
  --release` when the caller has already built binaries — exactly the shape
  this ADR's shared workflow needs to call it.
- GitHub's own documentation and community discussions on reusable
  (`workflow_call`) workflows called across repositories, specifically:
  whether `secrets.GITHUB_TOKEN` is automatically available inside a
  cross-repo reusable workflow call, what `actions/checkout` inside it
  checks out by default, and how permissions propagate. Confirmed (not
  assumed):
  - The called workflow **is** automatically granted `github.token`/
    `secrets.GITHUB_TOKEN`, scoped to the **calling** repository — no
    `secrets: inherit` or explicit secret-passing is needed for
    `GITHUB_TOKEN` specifically (only for genuinely custom secrets, which
    this workflow doesn't need).
  - Its permissions can only be **downgraded**, never escalated, relative
    to what the calling job declares — so the calling job must declare
    `permissions: contents: write` for `gh release upload` (inside the
    reusable workflow) to have write access at all.
  - `actions/checkout` with no `repository:` argument, run **inside** the
    reusable workflow, checks out the **calling** repository (`ft991a`/
    `ts570d`), not `radio-cat-rs` — exactly what's needed for the calling
    repo's own `Cargo.toml`/`packaging/*` scripts to be present on the
    runner.

## Decision

### 1. A reusable `workflow_call` workflow, not a composite action

The task's own escape hatch ("if a reusable `workflow_call` workflow is
impractical... prefer a composite action") does not apply here: the
GitHub-documented behaviors above (automatic scoped `GITHUB_TOKEN`,
caller-repo checkout, permission downgrade-only propagation) show a
cross-repo reusable workflow works cleanly for this need, with no
unresolved secrets/permissions friction — the only requirement on the
calling side is one `permissions:` line. `.github/workflows/release-app.yml`
in this repo is a `workflow_call` workflow with two jobs, `linux` and
`windows` (the latter `if: inputs.build_windows`), matching the four
required artifacts.

### 2. The exact interface (inputs, and what each job does)

```yaml
# .github/workflows/release-app.yml (this repo)
on:
  workflow_call:
    inputs:
      app_name: { required: true, type: string }        # display name only
      main_binary: { required: true, type: string }      # cargo package name, e.g. "ts570d"
      extra_binaries: { required: false, type: string, default: "" }
        # space-separated "package" or "package:binary" entries, e.g.
        # "emulator cat-transport-serial:pin-test"
      rust_toolchain: { required: false, type: string, default: "stable" }
      apt_packages: { required: false, type: string, default: "" }
      build_windows: { required: false, type: boolean, default: true }
      deb_glob: { required: false, type: string, default: "*.deb" }
      windows_package_glob: { required: false, type: string, default: "*.zip" }
```

- **`linux` job** (`ubuntu-latest`): installs Rust + `apt_packages`,
  `cargo build --release -p <main_binary>` and each `extra_binaries` entry,
  tars the raw `main_binary` executable
  (`<main_binary>-linux-x86_64.tar.gz`), runs `./packaging/build-deb.sh
  --skip-build`, and `gh release upload`s the tarball plus `deb_glob`'s
  matches.
- **`windows` job** (`windows-latest`, skipped if `build_windows: false`):
  same build step, zips the raw `.exe`
  (`<main_binary>-windows-x86_64.zip`), runs `pwsh
  ./packaging/build-windows-package.ps1`, and `gh release upload`s the zip
  plus `windows_package_glob`'s matches.

**Why `windows-latest` (MSVC host toolchain), not a Linux runner
cross-compiling to `x86_64-pc-windows-gnu`:** this repo's own Windows
verification (`cargo check --target x86_64-pc-windows-gnu`) is a
type-check-only convenience for a Linux sandbox with no Windows runner
available (per ADR 0004). A real GitHub Actions **Windows runner is
available** for actual release builds, and produces a native
`x86_64-pc-windows-msvc` binary with better real-world compatibility (no
mingw runtime dependency) than a cross-compiled `-gnu` one — the better
choice for an artifact end users will actually run. This is a deliberate
divergence from the sandbox's own dev-verification target, not an
inconsistency: dev-time verification and the release artifact's target
triple are different concerns with different constraints.

### 3. The consuming-repo contract

A calling repository (`ft991a`, `ts570d`) must provide, at fixed paths:

1. **`packaging/build-deb.sh [--skip-build]`** — `ts570d` already has this;
   unchanged contract, no update needed there beyond fixing its own stale
   `-p serial`/`pin-test`-sourcing build step (out of this ADR's scope,
   `ts570d`'s own follow-on — see §5's note on where `pin-test` now lives).
   `ft991a` needs to add one, following `ts570d`'s as a template.
2. **`packaging/build-windows-package.ps1`** (new contract, no existing
   precedent in either app) — invoked via `pwsh`, no arguments, after
   `cargo build --release` has already produced `target/release/
   <main_binary>.exe` (and any `extra_binaries`). Must produce one or more
   package files (a `.zip`, an installer `.exe`, or both) directly in the
   repo root. Left deliberately unopinionated about *how* the app packages
   itself (a plain zip of the `.exe` + docs is sufficient to satisfy the
   contract; an NSIS/Inno Setup installer is an enhancement either app can
   add later without changing this workflow at all, since it's just
   globbed by `windows_package_glob`).
3. **A `[[bin]]` named `main_binary`**, buildable via `cargo build --release
   -p <main_binary>` — already true for both apps' main binaries today.

### 4. The calling side (thin, ~10–20 lines per app)

```yaml
# ft991a's or ts570d's own .github/workflows/release.yml
name: Release
on:
  release:
    types: [published]

jobs:
  release:
    permissions:
      contents: write   # required: gh release upload runs inside the called workflow
    uses: kf0uwv/radio-cat-rs/.github/workflows/release-app.yml@main
    with:
      app_name: "ts570d-radio-control"
      main_binary: "ts570d"
      extra_binaries: "emulator cat-transport-serial:pin-test"
      apt_packages: "libudev-dev"
```

No `secrets:` block is needed — `secrets.GITHUB_TOKEN` propagates
automatically, scoped by the `permissions:` above (§ Context). An app that
also wants `ts570d`'s existing "bump version on main" step keeps that as a
**second job** in its own local `release.yml`, `needs: release` — that
policy (whether/how to auto-bump a version) is app-specific and
deliberately not part of the shared workflow.

### 5. Where `pin-test` now comes from (cross-reference, not new scope)

Per [ADR 0006](0006-windows-network-transport.md) §6, the RS-232 pin-test
tool moved from `ts570d/src/bin/pin_test.rs` to a `[[bin]]` inside
`cat-transport-serial` (this repo). Both apps' `extra_binaries` input
should reference it as `cat-transport-serial:pin-test` (package differs
from binary name) — `ts570d`'s own `packaging/build-deb.sh` already expects
a `target/release/pin-test` to exist (it does today, from `ts570d`'s own
now-defunct local build step); once `ts570d` migrates its release workflow
onto this shared one, `main_binary`'s and `extra_binaries`' build steps
above produce it via `cargo build --release -p cat-transport-serial --bin
pin-test`, landing in the **same** `target/release/pin-test` path
`build-deb.sh` already reads from — no change needed to `build-deb.sh`
itself for this specific binary.

## Explicit callout: this will not fire until `main` is pushed

**This pass does not push to `origin`** (per this task's own constraint).
`uses: kf0uwv/radio-cat-rs/.github/workflows/release-app.yml@main`
cross-repo reusable-workflow calls resolve `@main` against the **remote**
`kf0uwv/radio-cat-rs` repository on GitHub, not against this local
checkout — so any `ft991a`/`ts570d` release workflow written against this
contract will not actually resolve or run (`workflow not found` at call
time) until a human reviews and pushes this repository's `main` branch.
This is expected and not a bug in either consuming repo's workflow; do not
spend time debugging a "missing reusable workflow" error against this
specific file before that push has happened.

## Consequences

- New file: `.github/workflows/release-app.yml` in this repo (`workflow_call`
  only — it is never triggered directly, only referenced via `uses:`).
- `ft991a`/`ts570d` are **not modified** by this ADR — writing their own
  thin caller workflows and `packaging/build-windows-package.ps1` scripts
  (and, for `ts570d`, fixing its release workflow's stale local-`-p
  serial`/`pin-test` build step) is each app's own follow-on work, per this
  task's scope boundary.
- No secrets beyond the automatic `GITHUB_TOKEN` are required by this
  workflow. A future need (e.g. code-signing a Windows installer) would add
  a new named `secrets:` input at that point, not before.
- This workflow has not been exercised against a real GitHub Release in
  this sandboxed environment (no network access to trigger real Actions
  runs) — its correctness rests on the documented GitHub Actions behaviors
  cited in Context, not on an end-to-end dry run. The first real signal
  will be the first actual `release: published` event on either consuming
  repo once wired in.
