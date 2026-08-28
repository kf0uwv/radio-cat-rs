# radio-cat-rs

A shared, radio-independent CAT (Computer Aided Transceiver) protocol library
for Rust: a generic command engine, transport implementations (serial, TCP,
UDP), and a request broker for running a physical radio as a network-shared
server. Consumed by more than one radio-control application —
[`ts570d`](https://github.com/kf0uwv/ts570d) (Kenwood TS-570D) and
[`ft991a`](https://github.com/kf0uwv/ft991a) (Yaesu FT-991A) — without either
application needing radio-specific code duplicated into it.

## Status: extracted and in active use

Nine crates, all implemented, tested, and consumed by both sibling
applications via git dependency:

- **`cat-framework`** — the generic CAT command engine: command table,
  parsing, structural validation, dispatch lifecycle, response building.
  Radio-independent — contains no radio-specific command ids, modes,
  frequencies, or handlers. Extracted from `ts570d`'s original `framework`
  crate.
- **`cat-transport-core`** — the `Transport`/`CatSession` trait abstractions
  every concrete transport implements; `ModemControlLines` (direct RTS/
  DTR/CTS/DSR/DCD control, additive and separate from the base traits since
  not every transport has physical modem-control lines) plus
  `NoModemControlLines<S>`, a reusable adapter that gives any `CatSession`
  (e.g. a TCP-backed one) an honest-error `ModemControlLines` impl instead
  of each application hand-writing one; the shared `completion` (single-slot
  async/thread-bridging primitive) and `timeout` (portable, `monoio`-free
  "run this or give up after a duration" combinator) building blocks every
  Windows transport backend below is built on.
- **`cat-transport-serial`** — serial CAT transport. **Two platform
  backends**: io_uring on Linux (`monoio`), and native Win32 COM-port I/O on
  Windows (`windows-sys`, a dedicated worker thread + the shared completion
  primitive, since `monoio`/`tokio` are unavailable there). Same public
  types (`SerialPort`, `SerialConfig`, `SerialCatSession`) on both platforms
  — application code needs no platform-specific branching to use it. Also
  ships `pin-test`, a runnable `[[bin]]` RS-232 null-modem cable pin tester
  (TXD/RXD loopback + RTS/DTR/CTS/DSR/DCD checks), genuinely cross-platform,
  for both apps' packaging to install.
- **`cat-transport-tcp`** — `TcpCatSession`, length-prefixed framing.
  **Windows-capable**: a `std::net`-based backend alongside the Linux
  `monoio` one, same public type, no new external dependency.
- **`cat-transport-udp`** — `UdpCatSession`, envelope format + client-side
  dedup cache + per-request timeout (UDP has no delivery/ordering
  guarantee). **Windows-capable**, same shape as `cat-transport-tcp`.
- **`cat-client`** — generic client-side request/response mechanics
  (`CatClient<C: CommandId, S: CatSession>`), used by each radio's typed
  controller client.
- **`cat-server`** — the request broker: single ordered worker, TCP/UDP
  accept loops (reusing `cat-transport-tcp`/`-udp`'s own codec functions,
  not duplicating them), request/response correlation, timeout/disconnect/
  malformed-request handling. **Windows-capable**: the Job queue and
  listener concurrency substrate are rebuilt on genuine OS threads (no
  cooperative `monoio` tasks available there); the broker/dispatch logic
  itself is fully shared, unduplicated, across both platforms.
- **`cat-diagnostics`** — a generic, radio-independent diagnostics/self-test
  engine: exercises every documented **read** form in a `CommandTable<C>`
  via a `CatClient<C, S>` (read-only by construction — never writes/mutates
  radio state) and returns a structured per-command report (success/
  failure/timeout/skipped, latency, raw request/response text). Not a port
  of any single radio's diagnostics screen — see
  [ADR 0007](docs/adr/0007-shared-diagnostics-engine.md) for the exact API.
- **`cat-rigctl`** — a generic Hamlib `rigctld`-compatible bridge behind a
  `RigctlRadio` trait.

191 tests passing across the workspace on Linux; the Windows-capable crates
(`cat-transport-serial`, `cat-transport-tcp`, `cat-transport-udp`,
`cat-transport-core`, `cat-diagnostics`) are additionally verified via
`cargo check` on a `windows-latest` CI runner targeting
`x86_64-pc-windows-msvc` (per [ADR 0012](docs/adr/0012-native-msvc-windows-target.md),
which retired the previous `x86_64-pc-windows-gnu` cross-check); `cat-server`'s
Windows backend is verified the same way for
its listener/worker modules specifically (see
[ADR 0006](docs/adr/0006-windows-network-transport.md) §4 for why those
modules' *tests* — as opposed to production code — stay Windows-only).
Actual runtime behavior against physical Windows hardware has not been
validated in this environment.

A shared, reusable GitHub Actions release workflow
(`.github/workflows/release-app.yml`) is available for consuming
applications to build a Linux binary + `.deb`, a Windows binary + package,
and upload all four as GitHub Release assets from a ~10–20 line caller
workflow — see [ADR 0008](docs/adr/0008-shared-release-workflow.md) for the
exact contract.

## Design record

- [ADR 0001](docs/adr/0001-scope-and-crate-boundaries.md) — scope and crate
  boundaries (as amended once extraction actually happened).
- [ADR 0002](docs/adr/0002-async-runtime-binding-for-transport-crates.md) —
  why `monoio`/io_uring was retained for Linux rather than a runtime-agnostic
  redesign, with an explicit revisit trigger.
- [ADR 0003](docs/adr/0003-modem-control-lines.md) — `ModemControlLines`, a
  separate, additive capability trait for RTS/DTR/CTS/DSR/DCD line control.
- [ADR 0004](docs/adr/0004-windows-serial-backend.md) — the Windows COM
  backend: resolves ADR 0002's revisit trigger by adding a genuinely separate
  platform backend rather than redesigning the Linux path.
- [ADR 0005](docs/adr/0005-rigctl-bridge-and-radio-trait-boundary.md) —
  `cat-rigctl`: a generic rigctld bridge behind a `RigctlRadio` trait.
- [ADR 0006](docs/adr/0006-windows-network-transport.md) — Windows backends
  for `cat-transport-tcp`/`-udp`/`cat-server` (resolving ADR 0002's second
  revisit trigger), the shared `pin-test` tool, and `NoModemControlLines`.
- [ADR 0007](docs/adr/0007-shared-diagnostics-engine.md) — `cat-diagnostics`,
  the shared, radio-generic diagnostics engine.
- [ADR 0008](docs/adr/0008-shared-release-workflow.md) — the shared GitHub
  Actions release workflow.

See [`docs/adr/README.md`](docs/adr/README.md) for the full index and current
repository status.

## Using this library

Not published to crates.io — consumed as a git dependency:

```toml
cat-framework       = { git = "https://github.com/kf0uwv/radio-cat-rs", branch = "main" }
cat-client          = { git = "https://github.com/kf0uwv/radio-cat-rs", branch = "main" }
cat-transport-core  = { git = "https://github.com/kf0uwv/radio-cat-rs", branch = "main" }
cat-transport-serial = { git = "https://github.com/kf0uwv/radio-cat-rs", branch = "main" }
cat-transport-tcp   = { git = "https://github.com/kf0uwv/radio-cat-rs", branch = "main" }
cat-transport-udp   = { git = "https://github.com/kf0uwv/radio-cat-rs", branch = "main" }
cat-server          = { git = "https://github.com/kf0uwv/radio-cat-rs", branch = "main" }
cat-diagnostics     = { git = "https://github.com/kf0uwv/radio-cat-rs", branch = "main" }
```

See `ts570d/Cargo.toml` or `ft991a/Cargo.toml` for real, working examples of
the full dependency wiring.

## Contributing

See [`CLAUDE.md`](CLAUDE.md) for the planning-with-files convention and the
agent roster in `.claude/agents/`. Every crate has its own dependency-boundary
rules — read `docs/adr/0001` before adding a new one or changing an existing
crate's dependency graph.
