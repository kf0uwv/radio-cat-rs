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
| [0006](0006-windows-network-transport.md) | Windows network transport (`cat-transport-tcp`/`cat-transport-udp`/`cat-server`), the shared RS-232 pin-test tool, and `NoModemControlLines` | Accepted |
| [0007](0007-shared-diagnostics-engine.md) | `cat-diagnostics`: a shared, radio-generic diagnostics/self-test engine | Accepted |
| [0008](0008-shared-release-workflow.md) | Shared GitHub Actions release automation | Accepted |
| [0009](0009-civ-engine-for-binary-addressed-protocols.md) | Generalize `cat-framework` over a `CatWireFormat` type parameter, for binary/addressed protocols (Icom CI-V) | Accepted |
| [0010](0010-capability-model-and-normalized-signal-source.md) | A radio capability model, multi-endpoint transports, and a normalized `SpectrumSource`, served by a native protocol with rigctl as a compatibility layer | Accepted |
| [0011](0011-cat-ui-base-widgets-radio-specific-layout.md) | `cat-ui`: shared base widgets for both renderers; layout and features stay radio-specific | Accepted |
| [0012](0012-native-msvc-windows-target.md) | `x86_64-pc-windows-msvc` is the single Windows target; drop `-gnu`, gate platform code across lib/TUI/GUI | Accepted |
| [0013](0013-renderer-parity-tui-and-gui.md) | Renderer parity: the TUI and the GUI expose the same capabilities, and the TUI is permanent | Accepted |
| [0014](0014-rtlsdr-spectrum-source.md) | `cat-signal-rtlsdr`: worker thread, latest-frame backpressure, and a WinUSB driver story | Accepted |
| [0015](0015-model-facts-versus-installation-facts.md) | Separate what a radio *model* can do from what an *installation* has wired | Accepted |

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

[ADR 0006](0006-windows-network-transport.md) resolves ADR 0002's second,
independent revisit trigger ("a consuming application needs an async
runtime other than monoio"): `cat-transport-tcp`, `cat-transport-udp`, and
`cat-server` each gain a Windows backend (no `windows-sys` needed — TCP/UDP
sockets are natively cross-platform in `std`), reusing the completion
primitive moved to `cat_transport_core::completion` for exactly this
purpose. Also records the shared RS-232 pin-test `[[bin]]` (moved from
`ts570d` into `cat-transport-serial`) and `cat_transport_core::
NoModemControlLines` (generalized from `ft991a`'s hand-written adapter).
