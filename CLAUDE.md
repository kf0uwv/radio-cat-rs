# radio-cat-rs - Agent Guidelines

## Repository status: no code yet

This repository holds **planning and agent scaffolding only**. There is no
`Cargo.toml`, no crate source, and no workspace. Extraction from `ts570d`
(the sibling repository this library is extracted from) has not happened and
must not be started without an explicit go-ahead — see
[`docs/adr/0001-scope-and-crate-boundaries.md`](docs/adr/0001-scope-and-crate-boundaries.md)
for why, and `ts570d`'s ADR 0004/0005 for the source design this repository
is scaffolded to receive.

Until that go-ahead is given, all agents working in this repository are
limited to:

- planning documents under `./planning/`;
- ADRs under `docs/adr/`;
- agent definitions under `.claude/agents/`;
- this file and the root `README.md`.

No agent should run `cargo init`/`cargo new`, write `.rs` or `.toml` files,
or move/copy files out of `ts570d`, without an explicit instruction that
overrides this.

## Superpowers Coding Model (MANDATORY, once code work begins)
- Use planning-with-files skill for ALL implementation work
- Follow TDD, frequent commits, verification-before-completion
- Check for applicable skills BEFORE any action

## Planning-with-Files Requirement
- Each agent and subagent must maintain their own planning-with-files in a
  directory under `./planning/` with their name
- Directories today: `./planning/architect/`, `./planning/cat_framework/`,
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

## Architect Review Workflow (MANDATORY, once code work begins)
- ALL subagents must write their implementation plan to their `task_plan.md`
  BEFORE writing any code
- Plans are reviewed by the architect and user before work proceeds
- Subagents execute ONE task at a time, reporting results before moving to
  the next
- The architect coordinates parallelization across subagents
- No subagent proceeds past planning without architect approval

## (Future) Crate Dependency Model — target design, not yet built

This is the target shape recorded in
[`docs/adr/0001-scope-and-crate-boundaries.md`](docs/adr/0001-scope-and-crate-boundaries.md)
and in `ts570d` ADR 0005. It describes what will exist after extraction, not
what exists today.

```
cat-framework        (NO local crate dependencies — generic, radio-independent)
  └── generic CAT engine: CommandTable<C>, CommandDefinition<C>, CommandForm,
      CommandOperation, CommandRequest, ParameterValues, ResponseBuilder,
      CommandOutcome, CatCommandCatalog / CatRadio traits, CatFramework<R>
  └── generic errors (FrameworkError)
  └── contains NO radio-specific command ids, modes, frequencies, state, or handlers

cat-transport-core   (depends on: nothing in this workspace)
  └── Transport trait (byte-level read/write/flush)
  └── CatSession trait (request/response framing above Transport)
  └── MockCatSession / ScriptedCatSession test doubles

cat-transport-serial (depends on: cat-transport-core)
cat-transport-tcp    (depends on: cat-transport-core)
cat-transport-udp    (depends on: cat-transport-core)
  └── each implements CatSession with its own framing:
      serial = read-until-';' (io_uring on Linux today);
      tcp = length-prefixed frames; udp = envelope + dedup cache

cat-client           (depends on: cat-framework, cat-transport-core)
  └── generic client-side request/response mechanics: validate against a
      CommandTable<C>, format outgoing bytes, interpret ResponseDisposition
  └── a radio crate (e.g. ts570d's `radio`, a future `ft991a`) wraps this
      with its own typed get/set methods — cat-client itself stays generic

cat-server           (depends on: cat-client or a radio's client type, a CatSession impl)
  └── request broker: client session management, physical radio session
      ownership, single ordered worker, request/response correlation by ID,
      timeout handling, disconnect handling, malformed-request rejection
  └── never the reverse dependency: a radio crate never depends on cat-server
```

### Rules (violation is a blocking issue, once code exists)
1. **`cat-framework`** has NO local crate dependencies and contains NO
   radio-specific types. It defines the generic CAT engine and generic
   errors only.
2. **`cat-framework`** NEVER depends on a transport crate, `cat-client`,
   `cat-server`, or any radio crate. Verify with `cargo tree -p
   cat-framework` — no other local crate should appear.
3. **`cat-transport-core`** depends on nothing else in this workspace; the
   concrete transport crates depend on it, never the reverse.
4. **`cat-client`** never names a concrete transport type — it is generic
   over `CatSession`.
5. **`cat-server`** sits above a radio's client and a transport
   implementation; it never leaks broker/session/client-id concepts into
   `cat-framework` or a radio's state machine.
6. A consuming application's wiring layer (e.g. `ts570d`'s `src/main.rs`) is
   the ONLY place concrete transport types are chosen and instantiated.
7. Unit tests use mock/fake implementations of the relevant trait — never a
   concrete transport pulled in from another crate.

## Core Technologies (inherited constraint, subject to the open item below)
- monoio: io_uring async runtime — `ts570d`'s `Transport`/`CatSession` traits
  are currently bound to it (`#[async_trait(?Send)]`, re-exporting `monoio`).
  Whether `cat-transport-core` keeps this binding or adopts a
  runtime-agnostic associated-future design is an **open item**, recorded in
  ADR 0001, to be decided when `cat-transport-core` is actually extracted —
  not assumed away in the meantime.
- Tokio should NEVER be used in this project unless a future ADR explicitly
  changes this.
- Error handling: thiserror + `Result<T, E>`
- Imports: std → external → local
- Naming: snake_case for functions/variables, PascalCase for types

## Essential Commands

There is nothing to build yet. Once a workspace exists:
- Build: `cargo build` / `cargo build --release`
- Test: `cargo test` / `cargo test test_name`
- Lint: `cargo clippy` / `cargo fmt`

## Architecture

- `cat-framework/` — generic radio-independent CAT engine (not yet created)
- `cat-client/` — generic client-side request/response mechanics (not yet created)
- `cat-transport-core/` — `Transport` / `CatSession` trait abstractions (not yet created)
- `cat-transport-serial/`, `cat-transport-tcp/`, `cat-transport-udp/` — transport implementations (not yet created)
- `cat-server/` — request broker / server mode (not yet created)
- `docs/adr/` — ADRs recording the target design (see ADR 0001)
- `.claude/agents/` — subagent roster for this repository
- `planning/` — per-agent planning-with-files directories
