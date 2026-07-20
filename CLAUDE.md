# radio-cat-rs - Agent Guidelines

## Repository status: extracted and in active use

Seven crates exist, are implemented, and are consumed by both `ts570d` and
`ft991a` via git dependency. Extraction from `ts570d` (the sibling repo this
library was originally lifted from) is complete — see
[`docs/adr/0001-scope-and-crate-boundaries.md`](docs/adr/0001-scope-and-crate-boundaries.md)
for the target design that guided it, `ts570d`'s ADR 0004/0005 for the
source design, and [`docs/adr/README.md`](docs/adr/README.md) for the
current status summary and links to every ADR.

Agents working in this repository now touch real source under
`cat-framework/`, `cat-transport-core/`, `cat-transport-serial/`,
`cat-transport-tcp/`, `cat-transport-udp/`, `cat-client/`, `cat-server/`,
plus:

- planning documents under `./planning/`;
- ADRs under `docs/adr/`;
- agent definitions under `.claude/agents/`;
- this file and the root `README.md`.

## Superpowers Coding Model (MANDATORY)
- Use planning-with-files skill for ALL implementation work
- Follow TDD, frequent commits, verification-before-completion
- Check for applicable skills BEFORE any action

## Planning-with-Files Requirement
- Each agent and subagent must maintain their own planning-with-files in a
  directory under `./planning/` with their name
- Directories: `./planning/architect/`, `./planning/cat_framework/`,
  `./planning/cat_transport/`, `./planning/cat_server/`,
  `./planning/code_review/`
- Planning files include: `task_plan.md`, `findings.md`, `progress.md` in
  each agent's directory
- This prevents conflicts between agents working on different aspects of the
  project
- Planning files must be created and maintained before any implementation
  work

## Planning Directory Ownership and Boundaries
- Each agent owns ONLY their planning directory under
  `./planning/{agent_name}/`
- Agents must NEVER edit planning files in other agents' directories
- All planning work MUST use planning-with-files skill
- Planning files must be created BEFORE any implementation work
- Each agent is responsible for: `task_plan.md`, `findings.md`,
  `progress.md` in their own directory only
- Any violation of these boundaries is a critical issue

## Architect Review Workflow (MANDATORY)
- ALL subagents must write their implementation plan to their `task_plan.md`
  BEFORE writing any code
- Plans are reviewed by the architect and user before work proceeds
- Subagents execute ONE task at a time, reporting results before moving to
  the next
- The architect coordinates parallelization across subagents
- No subagent proceeds past planning without architect approval

## Crate Dependency Model

The target design recorded in
[`docs/adr/0001-scope-and-crate-boundaries.md`](docs/adr/0001-scope-and-crate-boundaries.md)
and refined in [ADR 0002](docs/adr/0002-async-runtime-binding-for-transport-crates.md)
(async runtime binding) and [ADR 0004](docs/adr/0004-windows-serial-backend.md)
(Windows serial backend) — this describes what actually exists, not a plan:

```
cat-framework        (NO local crate dependencies — generic, radio-independent)
  └── generic CAT engine: CommandTable<C>, CommandDefinition<C>, CommandForm,
      CommandOperation, CommandRequest, ParameterValues, ResponseBuilder,
      CommandOutcome, CatCommandCatalog / CatRadio traits, CatFramework<R>
  └── generic errors (FrameworkError)
  └── contains NO radio-specific command ids, modes, frequencies, state, or handlers

cat-transport-core   (depends on: cat-framework, for ResponseDisposition/ProtocolErrorKind reuse)
  └── Transport trait (byte-level read/write/flush)
  └── CatSession trait (request/response framing above Transport)
  └── ModemControlLines trait (RTS/DTR/CTS/DSR/DCD line control — additive,
      NOT part of Transport/CatSession, since not every transport has
      physical modem-control lines)
  └── ScriptedCatSession test double, conformance test suite

cat-transport-serial (depends on: cat-transport-core)
  └── Two platform backends, same public types on both:
      Linux — io_uring via monoio (`[target.'cfg(target_os = "linux")'.dependencies]`)
      Windows — native Win32 COM-port I/O via windows-sys: a dedicated
      worker thread (blocking ReadFile/WriteFile) driven by a hand-rolled
      completion primitive, since monoio/tokio don't exist on Windows;
      ModemControlLines via EscapeCommFunction/GetCommModemStatus
  └── SerialPort, SerialConfig, SerialCatSession — identical public API
      on both platforms; application code needs no platform branching

cat-transport-tcp    (depends on: cat-transport-core)
  └── TcpCatSession — length-prefixed frames

cat-transport-udp    (depends on: cat-transport-core)
  └── UdpCatSession — envelope format (session/request IDs) + client-side
      dedup cache + per-request timeout

cat-client           (depends on: cat-framework, cat-transport-core)
  └── generic client-side request/response mechanics: validate against a
      CommandTable<C>, format outgoing bytes, interpret ResponseDisposition
  └── a radio crate (`ts570d`'s `radio`, `ft991a`'s `radio`) wraps this
      with its own typed get/set methods — cat-client itself stays generic

cat-server           (depends on: cat-client, a cat-transport-* implementation)
  └── request broker: client session management, physical radio session
      ownership, single ordered worker, request/response correlation by ID,
      timeout handling, disconnect handling, malformed-request rejection
  └── never the reverse dependency: a radio crate never depends on cat-server
```

### Rules (violation is a blocking issue)
1. **`cat-framework`** has NO local crate dependencies beyond what's listed
   above and contains NO radio-specific types. Verify with `cargo tree -p
   cat-framework` — no other local crate should appear.
2. **`cat-framework`** NEVER depends on a transport crate, `cat-client`,
   `cat-server`, or any radio crate.
3. **`cat-transport-core`** depends only on `cat-framework`; the concrete
   transport crates depend on it, never the reverse.
4. **`cat-client`** never names a concrete transport type — it is generic
   over `CatSession`.
5. **`cat-server`** sits above a radio's client and a transport
   implementation; it never leaks broker/session/client-id concepts into
   `cat-framework` or a radio's state machine.
6. A consuming application's wiring layer (e.g. `ts570d`'s/`ft991a`'s
   `src/main.rs`) is the ONLY place concrete transport types are chosen and
   instantiated.
7. Unit tests use mock/fake implementations of the relevant trait — never a
   concrete transport pulled in from another crate.
8. `monoio` is a Linux-only, target-gated dependency in every crate that
   needs it. Windows code lives in `#[cfg(target_os = "windows")]`-gated
   modules with no `monoio`/`tokio` dependency at all — see ADR 0004.

## Core Technologies
- monoio: io_uring async runtime, Linux only, target-gated
  (`[target.'cfg(target_os = "linux")'.dependencies]`) in every crate that
  needs it
- windows-sys: Win32 FFI bindings, Windows only, target-gated, used solely
  by `cat-transport-serial`'s Windows backend
- Tokio should NEVER be used in this project unless a future ADR explicitly
  changes this
- Error handling: thiserror + `Result<T, E>`
- Imports: std → external → local
- Naming: snake_case for functions/variables, PascalCase for types

## Essential Commands
- Build: `cargo build --workspace` / `cargo build --workspace --release`
- Test: `cargo test --workspace` / `cargo test -p <crate> test_name`
- Lint: `cargo clippy --workspace --all-targets -- -D warnings` / `cargo fmt`
- Windows cross-compile check: `cargo check --target x86_64-pc-windows-gnu -p cat-transport-serial`
  (requires `rustup target add x86_64-pc-windows-gnu`; type-checks only, no
  link/run — actual Windows runtime behavior is validated by consumers on
  real hardware, not in this repo's own test suite)

## Architecture

- `cat-framework/` — generic radio-independent CAT engine
- `cat-client/` — generic client-side request/response mechanics
- `cat-transport-core/` — `Transport` / `CatSession` / `ModemControlLines`
  trait abstractions
- `cat-transport-serial/`, `cat-transport-tcp/`, `cat-transport-udp/` —
  transport implementations
- `cat-server/` — request broker / server mode
- `docs/adr/` — ADRs recording the design (see ADR 0001 for the index)
- `.claude/agents/` — subagent roster for this repository
- `planning/` — per-agent planning-with-files directories
