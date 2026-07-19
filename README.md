# radio-cat-rs

A shared, radio-independent CAT (Computer Aided Transceiver) protocol library
for Rust: a generic command engine, transport implementations (serial, TCP,
UDP), and a request broker for running a physical radio as a network-shared
server. Consumed by more than one radio-control application —
[`ts570d`](https://github.com/kf0uwv/ts570d) (Kenwood TS-570D) and
[`ft991a`](https://github.com/kf0uwv/ft991a) (Yaesu FT-991A) — without either
application needing radio-specific code duplicated into it.

## Status: extracted and in active use

Seven crates, all implemented, tested, and consumed by both sibling
applications via git dependency:

- **`cat-framework`** — the generic CAT command engine: command table,
  parsing, structural validation, dispatch lifecycle, response building.
  Radio-independent — contains no radio-specific command ids, modes,
  frequencies, or handlers. Extracted from `ts570d`'s original `framework`
  crate.
- **`cat-transport-core`** — the `Transport`/`CatSession` trait abstractions
  every concrete transport implements, plus `ModemControlLines` (direct RTS/
  DTR/CTS/DSR/DCD control, additive and separate from the base traits since
  not every transport has physical modem-control lines).
- **`cat-transport-serial`** — serial CAT transport. **Two platform
  backends**: io_uring on Linux (`monoio`), and native Win32 COM-port I/O on
  Windows (`windows-sys`, a dedicated worker thread + hand-rolled completion
  primitive, since `monoio`/`tokio` are unavailable there). Same public
  types (`SerialPort`, `SerialConfig`, `SerialCatSession`) on both platforms
  — application code needs no platform-specific branching to use it.
- **`cat-transport-tcp`** — `TcpCatSession`, length-prefixed framing.
- **`cat-transport-udp`** — `UdpCatSession`, envelope format + client-side
  dedup cache + per-request timeout (UDP has no delivery/ordering guarantee).
- **`cat-client`** — generic client-side request/response mechanics
  (`CatClient<C: CommandId, S: CatSession>`), used by each radio's typed
  controller client.
- **`cat-server`** — the request broker: single ordered worker, TCP/UDP
  accept loops (reusing `cat-transport-tcp`/`-udp`'s own codec functions,
  not duplicating them), request/response correlation, timeout/disconnect/
  malformed-request handling.

125 tests passing across the workspace on Linux; the Windows serial backend
is verified via `cargo check --target x86_64-pc-windows-gnu` (real
cross-compilation type-checking) — actual runtime behavior against physical
Windows hardware has not been validated in this environment.

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

See [`docs/adr/README.md`](docs/adr/README.md) for the full index and current
repository status.

## Using this library

Not published to crates.io — consumed as a git dependency:

```toml
cat-framework       = { git = "https://github.com/kf0uwv/radio-cat-rs", branch = "main" }
cat-client          = { git = "https://github.com/kf0uwv/radio-cat-rs", branch = "main" }
cat-transport-core  = { git = "https://github.com/kf0uwv/radio-cat-rs", branch = "main" }
cat-transport-serial = { git = "https://github.com/kf0uwv/radio-cat-rs", branch = "main" }
```

See `ts570d/Cargo.toml` or `ft991a/Cargo.toml` for real, working examples of
the full dependency wiring.

## Contributing

See [`CLAUDE.md`](CLAUDE.md) for the planning-with-files convention and the
agent roster in `.claude/agents/`. Every crate has its own dependency-boundary
rules — read `docs/adr/0001` before adding a new one or changing an existing
crate's dependency graph.
