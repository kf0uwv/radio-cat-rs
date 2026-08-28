# 12. `x86_64-pc-windows-msvc` is the single Windows target; drop `-gnu`, and gate platform code explicitly across lib, TUI and GUI

Date: 2026-08-27

## Status

**Accepted** (2026-08-27) — user sign-off; implementation authorized via
`planning/architect/task_plan.md` (Task 10), which is in progress.
Amends [ADR 0004](0004-windows-serial-backend.md),
[ADR 0006](0006-windows-network-transport.md) and
[ADR 0008](0008-shared-release-workflow.md) in this repo, and `ts570d`'s
ADR 0006, on the question of which Windows target is authoritative.

## Context

This workspace currently uses **two** Windows targets for two purposes, a
split [ADR 0008](0008-shared-release-workflow.md) made deliberately:

> **Why `windows-latest` (MSVC host toolchain), not a Linux runner
> cross-compiling to `x86_64-pc-windows-gnu`:** this repo's own Windows
> verification (`cargo check --target x86_64-pc-windows-gnu`) is a
> type-check-only convenience for a Linux sandbox with no Windows runner
> available (per ADR 0004). [...] This is a deliberate divergence from the
> sandbox's own dev-verification target, not an inconsistency: dev-time
> verification and the release artifact's target triple are different
> concerns with different constraints.

Concretely today:

| | Target | Where | Depth |
|---|---|---|---|
| Dev / PR verification | `x86_64-pc-windows-gnu` | Linux runner, `make windows-check`, CI `windows-check` job | type-check only |
| Release artifact | `x86_64-pc-windows-msvc` | `windows-latest` | real build |

The divergence was justified by "no Windows runner available." That premise
is **no longer true** — `release.yml` already builds on `windows-latest`,
described in ADR 0008 as "a real MSVC host." The runner is available; it is
simply not used for verification.

Three things now make the split actively harmful rather than merely
redundant.

1. **The verification target is not the shipped target.** Everything ADR
   0004's and ADR 0006's Windows backends are checked against differs from
   what users run in ABI, C runtime, and linking. A green `-gnu` check is
   evidence about a binary nobody ships.

2. **A GPU GUI makes `-gnu` a *false* signal, not just a weak one.**
   `radio-cat-rs` ADR 0011 introduces `cat-ui-egui` with a `wgpu` waterfall
   pass, and `ts570d` ADR 0008 a GUI on top of it. On Windows that means the
   DX12 backend and DXC shader compilation, an area where MSVC is the
   well-supported path and some GPU-adjacent crates ship MSVC-only import
   libraries. A `-gnu` check could pass while MSVC fails, or fail on
   something MSVC handles — either way it misinforms.

3. **`cat-signal-rtlsdr` has a genuinely different Windows story.** Its
   Linux path is librtlsdr via the system package manager. On Windows the
   dongle needs a WinUSB driver (Zadig), libusb comes from a different
   place, and linkage is not pkg-config. This is not a cfg detail bolted on
   late; it is a platform port that must be designed with the crate.

The user's direction: a truly native Windows build, with platform code
gated by explicit compiler directives across the library, the TUIs, and the
GUIs.

## Decision

### 1. `x86_64-pc-windows-msvc` is the only Windows target

`x86_64-pc-windows-gnu` is removed from `Makefile`s, CI workflows, ADR
"done when" criteria, `CLAUDE.md`s and READMEs across `radio-cat-rs`,
`ts570d`, `ft991a` and (before it is scaffolded) `ic7100`.

### 2. CI on `windows-latest` becomes authoritative, on every PR

The `windows-check` job moves from an Ubuntu runner cross-compiling `-gnu`
to a `windows-latest` runner, and is upgraded from `cargo check` to
`cargo check` **plus `cargo test`** for the Windows-capable crates.

This is a strict improvement in the residual risk both ADR 0004 and ADR
0006 recorded. Those ADRs' Windows backends have never had their *tests*
run on Windows — ADR 0006 §4 documents test modules staying Windows-only
and therefore unexecuted. Under this decision they execute on every PR.

`emulator` remains excluded: it hosts a pseudo-terminal pair
(`serialport::TTYPort`, `std::os::unix`) and is Unix-only by nature, not by
toolchain (`ts570d` CLAUDE.md, Windows support).

### 3. Local Linux verification is best-effort, not authoritative

`make windows-check` on a Linux host becomes `cargo-xwin`, which
cross-compiles to `x86_64-pc-windows-msvc` by fetching the Microsoft CRT
and Windows SDK. It is a fast local signal only; **CI is the source of
truth.**

#### Caveat 1 — GPU crates: **resolved, affirmative** (2026-08-27)

Whether `cargo-xwin` handles the `cat-ui-egui`/GUI crates was measured
before acceptance, with `cargo-xwin` 0.23.1 against a probe crate depending
on `eframe` 0.29 (`wgpu` feature), `egui-wgpu` 0.29 and `wgpu` 22:

- `cargo xwin check` — clean.
- `cargo xwin build` — clean, **including the link step**, producing a
  `PE32+ executable for MS Windows, x86-64`.
- The DX12 path built in full: `d3d12` 22.0.0, `wgpu-hal`, `gpu-allocator`
  0.26, `com`/`com_macros`, and `hassle-rs` 0.11 (the DXC shader-compiler
  binding that was the specific concern).

No exclusion is needed. The same run confirmed both existing workspaces:
`radio-cat-rs` (all crates) and `ts570d` (`--workspace --exclude emulator`)
check *and* link to MSVC, emitting a working `ts570d.exe`.

#### Caveat 2 — Microsoft licence: **open, and the user's call**

Unchanged in substance, sharper in fact. `cargo-xwin` 0.23.1 downloaded
~1.1 GB of Microsoft CRT and Windows SDK components into
`~/.cache/cargo-xwin` **with no interactive licence prompt of any kind**.
The tool will not force the question, so accepting Microsoft's terms has to
be a conscious decision rather than a side effect of running `make`.

Two facts narrow the exposure:

- **CI does not use `xwin` at all.** Decision §2 puts CI on a real
  `windows-latest` runner with Microsoft's own licensed toolchain. The
  licence question is scoped to developer machines only.
- It is a `Makefile` convenience, not a build requirement. Nothing ships
  from it.

Until the user settles this, `make windows-check` stands as written and
each developer accepts the terms by choosing to run it.

#### Caveat 3 — new: the local check cannot run tests

Discovered during the same evaluation, and it constrains what §3 can ever
be. `cargo-xwin` cross-compiles and links; it cannot *execute* the produced
binaries on Linux. So the `cargo test` half of Decision §2 — the entire
point of the upgrade, since these tests have never run — is reachable
**only** on the CI runner. The local check is a compile-and-link signal and
nothing more, and must not be described as a substitute.

### 4. Platform code is gated explicitly, at every layer

`#[cfg(target_os = "windows")]` / `#[cfg(unix)]` at the point of divergence,
in three layers rather than one:

| Layer | Already gated | Newly gated by this decision |
|---|---|---|
| Library | monoio Linux-only; `windows-sys` serial backend (ADR 0004); TCP/UDP/server listeners (ADR 0006) | `cat-signal-rtlsdr` — libusb/WinUSB acquisition and linkage |
| TUI | `ui/src/win_sched.rs`'s two-future scheduler (`ts570d` ADR 0006) | — |
| GUI | — | `cat-ui-egui` / `gui` — `wgpu` backend selection (DX12 vs Vulkan/GL), DPI handling, window creation |

The existing pattern holds: gate at the lowest layer that can absorb the
difference, and keep one public API across platforms so application code
does not branch. ADR 0004's serial backend is the reference — same public
API, two implementations, no branching in consumers.

### Explicitly out of scope for this ADR

- **32-bit or ARM64 Windows.** One target triple, `x86_64-pc-windows-msvc`.
- **macOS.** Not currently a target in any repo; unaffected either way.
- **The `cat-signal-rtlsdr` Windows port design.** Named here as a
  requirement; designed in that crate's own follow-up ADR.
- **Retiring `packaging/build-windows-package.ps1`.** It already targets a
  real Windows host and is unaffected.

## Consequences

**Good.**

- The target that is verified is the target that ships. ADR 0008's
  "deliberate divergence" closes on the side of the real artifact.
- ADR 0004's and ADR 0006's Windows backends get their **tests actually
  run**, on every PR, for the first time — directly reducing the residual
  risk both ADRs recorded and `ts570d` ADR 0006 inherited.
- The GUI is verified on the only Windows toolchain where its GPU path is
  well supported, instead of on one where a green check would mean nothing.
- `cat-signal-rtlsdr`'s Windows driver story is forced into its design ADR
  rather than discovered during packaging.

**Costs and risks.**

- **CI gets slower and more expensive.** Windows runners cost more minutes
  than Ubuntu ones, and this adds a full build-and-test rather than a
  type-check. Accepted: correctness on the shipped target is worth it.
- The cheap local Linux check degrades to best-effort, or disappears. In a
  sandbox with no Windows machine — still true of this environment — that
  lengthens the feedback loop for Windows-specific breakage.
- Many ADR "done when" criteria and planning files name the `-gnu` target
  (`radio-cat-rs` ADRs 0004, 0006, 0007, 0008 and its `planning/`; `ts570d`
  ADR 0006 and CLAUDE.md; `ft991a` ADR 0003). These are historical records
  of how something *was* verified and must not be rewritten to claim
  otherwise — they get an amendment note pointing here, not an edit.
- `cargo-xwin` adds a dependency on Microsoft-hosted SDK artifacts for local
  development, with the licence question above.
