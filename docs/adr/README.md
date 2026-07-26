# Architecture Decision Records

Decisions are recorded as [ADRs](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions)
(Michael Nygard format). Each file is one decision; numbers are stable and
never reused.

| ADR | Title | Status |
|-----|-------|--------|
| [0001](0001-scope-and-crate-boundaries.md) | Scope and crate boundaries for the shared CAT library | Accepted (amended) |
| [0002](0002-async-runtime-binding-for-transport-crates.md) | Async runtime binding for cat-transport-core and dependents | Accepted |
| [0003](0003-modem-control-lines.md) | `ModemControlLines`: a separate, additive capability trait for RTS/DTR/CTS/DSR/DCD | Accepted |
| [0004](0004-windows-serial-backend.md) | Windows serial backend for `cat-transport-serial` | Accepted |
| [0005](0005-rigctl-bridge-and-radio-trait-boundary.md) | `cat-rigctl`: a generic rigctld bridge behind a `RigctlRadio` trait | Accepted |

## Repository status

**Extraction is authorized and planned; no code has been written yet.**
`ts570d`'s `refactor/generic-cat-framework` work (the `CatSession` migration)
has landed, and the user has given an explicit go-ahead to (A) extract
`ts570d`'s generic engine into real crates here, and (B) implement TCP/UDP
transports and a `cat-server` request broker (see ADR 0005 in `ts570d`).

The open item ADR 0001 recorded — whether `cat-transport-core` stays bound to
`monoio`/`#[async_trait(?Send)]` — is resolved by [ADR 0002](0002-async-runtime-binding-for-transport-crates.md):
the binding is retained. ADR 0001 also carries two amendments recorded once
extraction was actually planned against `ts570d`'s current code: a dependency
correction (`cat-transport-core` depends on `cat-framework`, not on nothing)
and two scope clarifications (`cat-transport-serial` includes the concrete
io_uring `SerialPort` implementation from `ts570d`'s `serial` crate;
`framework::state_machine` is out of scope entirely). See ADR 0001's
"Amendments" section and `planning/architect/findings.md` for the full
reasoning.

The concrete dispatch queue for `cat_framework`, `cat_transport`, and
`cat_server` is in `planning/architect/task_plan.md`. No subagent should
begin implementation on a task that queue doesn't list, and no subagent
proceeds past its own task without an architect/user review checkpoint.

[ADR 0004](0004-windows-serial-backend.md) resolves ADR 0002's Windows
revisit trigger: `cat-transport-serial` gains a `#[cfg(target_os =
"windows")]` backend (Win32 COM ports via `windows-sys`, a dedicated
background thread + hand-rolled completion primitive for `Transport`,
direct `EscapeCommFunction`/`GetCommModemStatus` calls for
`ModemControlLines`) alongside the existing, unchanged Linux io_uring path.
Tasks 6–8 in `planning/architect/task_plan.md` are the dispatch queue for it.
