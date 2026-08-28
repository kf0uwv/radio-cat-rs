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

---


## Task 10 — migrate to `x86_64-pc-windows-msvc`, retire `-gnu` (2026-08-27)

Governing decision: [ADR 0012](../../docs/adr/0012-native-msvc-windows-target.md).

### 1. `cargo-xwin` handles the GPU stack — ADR 0012 caveat 1 closes affirmative

Measured, not assumed. `cargo-xwin` 0.23.1, probe crate depending on
`eframe` 0.29 (`wgpu` feature, `default-features = false`), `egui-wgpu`
0.29, `wgpu` 22:

| Step | Result |
|---|---|
| `cargo xwin check --target x86_64-pc-windows-msvc` | clean, 23s |
| `cargo xwin build --target x86_64-pc-windows-msvc` | clean **including link**, 29s |
| Artifact | `PE32+ executable for MS Windows 6.00 (console), x86-64` |

The specific crates the caveat worried about all built: `hassle-rs` 0.11
(DXC shader-compiler binding), `d3d12` 22.0.0, `wgpu-hal` 22, `gpu-allocator`
0.26, `com`/`com_macros` 0.6. **No crate needs excluding from the local
check.** Task 19 (`cat-ui-egui`) can rely on a local Linux signal.

### 2. Both existing workspaces cross-build to MSVC today

Not just type-check — link, with real binaries out:

- `radio-cat-rs`, all crates: `cargo xwin check` clean.
- `ts570d`, `--workspace --exclude emulator`: check **and** build clean;
  produced `ts570d.exe` (6.0 MB, PE32+) and `ts570d-line.exe`.

No source changes were needed to reach MSVC from `-gnu`. The migration is
toolchain and CI configuration only.

### 3. NEW — the local check can never run tests (ADR 0012 caveat 3)

`cargo-xwin` cross-compiles and links; it cannot execute Windows binaries on
Linux. The `cargo test` half of ADR 0012 §2 — the whole point of the
upgrade, since ADR 0006 §4 records those tests have never executed — is
reachable **only** on the `windows-latest` runner.

Consequence for how this is described: `make windows-check` is a
compile-and-link signal. Anyone treating it as "the Windows check" will
believe the Windows tests are passing when they have never run.

### 4. NEW — `ft991a`'s Windows CI scope was stale by a month

`ft991a/.github/workflows/ci.yml` checked only `-p ft991a`, justified in a
comment by `server` being "genuinely not Windows-buildable yet — its
`cat-rigctl` dependency has no Windows backend upstream in `radio-cat-rs`."

That stopped being true on **2026-07-26**, when `cat-rigctl` gained a real
Windows backend (`radio-cat-rs` ADR 0006's amendment). `ft991a`'s own
`Cargo.toml` was updated at the time and says so in a comment on the
`server` dependency; the CI job was not. So `server` has been unverified on
Windows for a month while both the Cargo manifest and the ADR said it was
supported.

Fixed here: the job is now `--workspace --exclude emulator`, matching
`ts570d`.

**Worth generalizing:** a justification comment pinned to an upstream gap
goes stale silently when the gap closes. Neither the `radio-cat-rs`
amendment nor the `ft991a` manifest update propagated to CI.

### 5. Microsoft licence — ADR 0012 caveat 2 stays open, and is the user's call

`cargo-xwin` downloaded ~1.1 GB of Microsoft CRT and Windows SDK components
into `~/.cache/cargo-xwin` **with no interactive licence prompt**. The tool
will not force the decision.

Scope is narrower than the ADR first assumed: **CI does not use `xwin` at
all** (it runs on a real `windows-latest` host with Microsoft's own licensed
toolchain), so this is developer-machines-only, and nothing shipped is built
from it. Not blocking; recorded for the user to settle.

### 6. Not verified here

- **No `windows-latest` CI run has happened.** These repos are not pushed
  from this environment. The green-CI half of Task 10's "done when" is
  outstanding, and it is where the pre-existing Windows test failures ADR
  0012 predicts will surface.
- Runtime behaviour on a real Windows machine — unchanged, still validated
  only by users running the released binary.

### 7. First real Windows test execution (2026-08-27) — ADR 0012's prediction confirmed

Run on real hardware: `radiombf` / "Victoria", Windows 11 build 22621,
`x86_64-pc-windows-msvc`, rustc 1.93.1, over SSH from the Linux dev box.
Cloned from published `main`; Task 10 changed **zero** `.rs` files, so this
is a valid reading of HEAD.

`cargo check --workspace` — **clean**, 19s. Production code is fine.

`cargo test --workspace` — **does not compile**, and then two tests fail.
Three distinct problems, none of them a production bug:

#### 7a. The Windows test target does not build at all (32 sites, 3 crates)

`cat-transport-core` (15), `cat-client` (13), `cat-diagnostics` (4) all fail
with `E0433: unresolved module or unlinked crate 'monoio'`. Each crate
correctly gates `monoio` to `cfg(target_os = "linux")` in `Cargo.toml`, but
its `#[cfg(test)] mod tests` uses `#[monoio::test(driver = "legacy")]`
**ungated**. On Windows the attribute references a crate that isn't there.

These are pure in-memory logic tests — `ScriptedCatSession` is a fake, no
io_uring is involved. `monoio::test` is being used as nothing more than a
`block_on`. Two ways out, and they differ in what they buy:

- Gate the test modules to Linux (~6 lines). Cheap, but Windows then runs
  almost nothing in those three crates.
- Replace the attribute with a runtime-agnostic `block_on` (32 sites, one
  unconditional `futures` dev-dependency). The tests then run on both
  platforms, which is what ADR 0012 was actually for.

Deferred to the user; not fixed here.

#### 7b. `cat-server::tcp_windows` — deterministic failure, 8/8 runs

`end_to_end_two_concurrent_connections_get_correctly_correlated_responses`,
`tcp_windows.rs:267`: `assert_eq!(registry...active_count(), 2)` gets `0`.

The two `TcpStream`s are **moved into** the worker threads, so both are
dropped when those threads exit — before the assertion runs. The server then
deregisters them, so `0` is arguably the *correct* answer. The 20 ms
`thread::sleep` above the assertion makes failure **more** likely, not less,
by giving the server time to notice the disconnects.

The test's own comment says the sleep is "best-effort, not load-bearing for
correctness above" — and then asserts on it anyway. Fix: keep the streams
alive across the assertion (return them from the threads), rather than
lengthening the sleep.

#### 7c. `cat-server::worker_windows` — genuine race, 3/8 runs

`worker_serializes_requests_from_multiple_handles_correctly`,
`worker_windows.rs:276`, failing inside `test_support.rs:159` with
`ScriptedCatSession: request mismatch`.

Two threads submit `FA;` and `IF;` concurrently; `ScriptedCatSession`
demands them in that fixed order. Nothing orders the two submissions — the
test conflates **serialized** (one at a time, which the worker does
guarantee and which is what it means to test) with **ordered** (which it
does not). Whichever thread wins the race decides whether the run passes.

Fix: either script the session order-insensitively, or sequence the
submissions and prove serialization some other way.

#### Summary

| | Result |
|---|---|
| `cargo check --workspace` | clean |
| `cargo test` — `cat-framework` | 8 passed |
| `cargo test` — `cat-rigctl` | 22 passed |
| `cargo test` — `cat-server` | 23-24 passed, 1-2 failed (8/8 runs red) |
| `cargo test` — `cat-transport-{tcp,udp,serial}` | compiled; blocked behind `cat-server`'s failure |
| `cargo test` — `cat-transport-core`, `cat-client`, `cat-diagnostics` | **does not compile** (7a) |

**The production code is in better shape than the failures suggest.** All
three problems are in test code. But they are real: these tests have been
broken or flaky on Windows since they were written, and nothing caught it
because ADR 0006 §4's "Windows-only, therefore unexecuted" modules had, in
fact, never once executed.

### 7d. NEW — a real production bug: the Windows UDP session times out early

Surfaced only after 7a was fixed, because the compile failure had been
hiding `cat-transport-udp`'s tests entirely.

`windows::tests::never_answered_request_times_out_instead_of_hanging`
failed with *"returned before the configured timeout elapsed:
193.1327ms"* against a 200 ms timeout.

This is **not** a test bug. `UdpCatSession::execute` computes a `deadline`
from `Instant::now()` (QPC on Windows), then arms `set_read_timeout`
(`SO_RCVTIMEO`) with the remaining time each pass. On `WouldBlock` /
`TimedOut` it **returned `Timeout` immediately**, without consulting its own
deadline — trusting the socket's clock over its own.

Windows measures `SO_RCVTIMEO` in system clock ticks (~15.6 ms granularity
by default); `Instant` uses QPC. The two disagree, so the socket's timer can
expire slightly *before* the deadline it was armed from, and the session
gives up early. On Linux `SO_RCVTIMEO` does not fire early, so the same code
has always behaved correctly there — the bug is invisible on the platform
the tests ran on.

Fix: `continue` instead of returning. The check at the top of the loop
already owns the timeout decision and is the only place that reads
`deadline`; looping re-arms the socket with whatever time genuinely remains
and cannot spin, because `remaining` is recomputed from a fixed `deadline`
and converges to zero.

**This is the concrete payoff of ADR 0012.** A radio that is slow to answer
could have been declared unreachable up to a tick early, on Windows only,
for as long as this backend has existed.

### 8. Fixes applied under Task 10, and the result

| Fix | Where | Kind |
|---|---|---|
| 32 × `#[monoio::test]` → runtime-agnostic `block_on` | `cat-transport-core`, `cat-client`, `cat-diagnostics` | test infrastructure |
| `block_on_probe`, split per platform | `cat-diagnostics/src/engine.rs` | test infrastructure |
| `ScriptedCatSession::with_unordered_script` | `cat-transport-core/src/test_support.rs` | test infrastructure (additive) |
| Streams held open across the registry assertion; fixed sleep → bounded poll | `cat-server/src/tcp_windows.rs` | test bug (7b) |
| Unordered script for two concurrent connections | `cat-server/src/tcp_windows.rs` | test bug (7c, second instance) |
| Unordered script for two concurrent submitters | `cat-server/src/worker_windows.rs` | test bug (7c) |
| `continue` instead of early `Timeout` return | `cat-transport-udp/src/windows.rs` | **production bug** (7d) |

`cat-diagnostics`' 4 timer tests needed the platform-split driver rather
than a plain `futures::executor::block_on`: on Linux `with_probe_timeout`
calls `monoio::time::timeout`, which panics outside a real monoio reactor
with its timer enabled. The helper mirrors the split that function already
has, and carries a comment saying it must stay in step with it.

**Result:**

| Platform | Tests | Runs |
|---|---|---|
| Windows 11 / MSVC | **123 passed, 0 failed** | 10 consecutive clean |
| Linux | **197 passed, 0 failed** | 3 consecutive clean |

Repeated runs were the point, not decoration: two of these bugs were
intermittent (3/8 and ~3/6), and a single green run would have "confirmed"
a fix for either without meaning anything.

Windows runs fewer tests than Linux because the Linux-only io_uring
(`cat-transport-serial`) and PTY paths have no Windows counterpart — that
gap is by design, not a remaining defect.
