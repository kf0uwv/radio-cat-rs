# 4. Windows serial backend for `cat-transport-serial`

Date: 2026-07-19

## Status

Accepted


> **Amended 2026-08-27 by [ADR 0012](0012-native-msvc-windows-target.md).**
> Every `x86_64-pc-windows-gnu` reference below is a historical record of
> how this work *was* verified at the time, deliberately left unedited.
> `-gnu` is retired: `x86_64-pc-windows-msvc` is now the only Windows
> target, and verification is `cargo check` **and `cargo test`** on a
> `windows-latest` runner.

## Context

[ADR 0002](0002-async-runtime-binding-for-transport-crates.md) retained the
`monoio`/`#[async_trait(?Send)]` binding and named an explicit revisit
trigger: "Windows serial support (no io_uring) enters scope for
`cat-transport-serial`." That trigger has now fired. The user wants real
Windows COM-port control of a physical FT-991A from a native Windows build of
`ft991a` (and, eventually, `ts570d`), and has given explicit direction on the
shape of the answer: **keep `monoio`/io_uring for Linux exactly as it is
today; add a genuinely separate Windows-specific serial backend alongside
it, not a runtime-agnostic redesign that touches the Linux path.** No
Windows-compatible emulator is required this round — the user will test
against real hardware themselves.

This ADR records that decision for `cat-transport-serial`, the only crate
this trigger affects (`cat-transport-core`'s traits are already
platform-neutral; `cat-transport-tcp`/`-udp`/`cat-server` are unaffected —
network sockets are not the trigger).

### The problem, stated precisely

`monoio` supplies two things at once: an async **executor** (thread-per-core,
`!Send` futures) and io_uring-backed **I/O primitives**. Windows has neither
`monoio` nor `tokio` (banned project-wide, both repos' `CLAUDE.md`) available.
`monoio` itself cannot compile on Windows at all — io_uring is a Linux kernel
interface, not a library choice — so this is not a question of "does
`SerialPort::open` merely fail on Windows," it is that the crate dependency
itself is absent. Adding a Windows `Transport`/`ModemControlLines`
implementation therefore requires deciding what satisfies the
`#[async_trait(?Send)]` method signatures at all on that platform, and
separately, what eventually drives (polls) those futures to completion in a
consuming application's `main()` — since `#[monoio::main]` cannot exist on
Windows either.

### What was read before deciding

- `ft991a/ui/src/terminal.rs` and `ft991a/src/main.rs` (read in full, not
  touched): a **single sequential loop** inside `#[monoio::main]`. No
  `monoio::spawn` anywhere in this repo. Each iteration: `poll_radio_state`
  (10 sequential `.await`ed CAT round trips), `draw_frame`, a blocking
  `crossterm::event::poll(10ms)` call (already synchronous even on today's
  Linux build), then `monoio::time::sleep(5ms).await`. There is exactly one
  live task in this application; nothing else on the executor could ever be
  starved by that task blocking.
- `ts570d/ui/src/terminal.rs` (read in full, not touched): a genuinely
  **concurrent two-task** design. `run()` uses `monoio::spawn` to run a
  `radio_task` (polling + diagnostics) alongside a `ui_task` (rendering +
  key handling) on the same OS thread, linked by `Rc<RefCell<VecDeque<T>>>`
  channels — explicitly so key events (including Quit) stay responsive
  during a slow poll or a 107-step diagnostic run, per the module's own doc
  comment ("radio polling/command task and the UI rendering/key-event task
  run concurrently ... so key events ... are always responsive regardless of
  radio latency"). This is real cooperative concurrency on one thread: if
  either task ever blocks that thread synchronously, the other stalls too.
- `cat-transport-serial/src/io_uring.rs`, `session.rs`, `lib.rs`: `SerialPort`
  wraps a `monoio::net::UnixStream` over the real fd (readv/writev via
  io_uring); `SerialCatSession<T: Transport>` (`session.rs`) is already
  fully generic over `T: Transport` and contains zero platform-specific
  code — it needs no changes for this decision.
- `docs/adr/0003-modem-control-lines.md` and `cat-transport-core/src/modem.rs`:
  `ModemControlLines` methods are **plain sync `fn`s**, not `#[async_trait]`
  — "direct `ioctl(2)` calls with no I/O wait." This precedent carries
  directly to Windows.

## Decision

### 1. Async execution: a dedicated background OS thread + a small hand-rolled completion primitive — not blocking-in-async-fn, not a third runtime crate

Three shapes were weighed, per the framing this decision was scoped against:

1. **Blocking Win32 calls disguised as `async fn`** (never actually
   `.await`s anything real) — rejected as the crate's general mechanism.
   It would be harmless for `ft991a`'s confirmed single-sequential-loop
   architecture (no concurrent task to starve), but `cat-transport-serial`
   is *shared* infrastructure, and `ts570d`'s existing concurrent two-task
   design depends on genuine concurrency between two `monoio`-spawned tasks
   on one OS thread. A naive blocking Windows `Transport` would silently
   defeat that design's entire purpose the moment `ts570d` targets Windows —
   an outcome invisible from inside `cat-transport-serial` until it happens
   on real hardware. Baking "the caller never needs concurrency" into the
   shared transport crate is the wrong layer for that assumption to live,
   and it is far cheaper to avoid now than to discover as a live
   responsiveness regression later. (`Transport::flush` is a deliberate,
   narrow exception to this — see "Flush" below, mirroring an identical
   exception the Linux implementation already makes for `tcdrain`.)
2. **A dedicated background OS thread doing blocking `ReadFile`/`WriteFile`,
   with the `async fn` methods sending a request and awaiting a completion
   signal** — **chosen**. See design below.
3. **A third, cross-platform async-executor crate** (`smol`,
   `async-executor`, `futures::executor` as a scheduler) — rejected for the
   same reason ADR 0002 rejected the runtime-agnostic redesign: no real
   consumer has asked for a general-purpose executor, `tokio` is banned, and
   the actual requirement is much narrower than "run arbitrary futures." It
   is exactly "let a background OS thread report one I/O completion back to
   whatever is already polling this future" — which needs a
   `Future`-compatible completion primitive, not an executor.

**Design:** `SerialPort::open` on Windows spawns one `std::thread` that owns
the raw `HANDLE` and performs blocking, non-overlapped `ReadFile`/`WriteFile`
calls in a loop, driven by requests received over a `std::sync::mpsc`
channel. `Transport::write`/`read` send a request (carrying the data, or the
requested length) plus one half of a small hand-rolled single-slot
completion primitive — `struct Completion<T>` / `channel<T>() ->
(CompletionTx<T>, CompletionRx<T>)`, a `Mutex<Option<T>> + Option<Waker>`
pair where `CompletionRx<T>: Future<Output = T>` stores the `Waker` on a
`Pending` poll and the worker thread calls `Waker::wake()` after storing the
result — then `.await` the `CompletionRx`. This genuinely suspends the
calling task (returns `Poll::Pending`) rather than blocking its thread;
`SerialPort::drop` drops the request sender (the worker's `recv()` then
returns `Err`, the thread exits its loop) and joins the thread before
`CloseHandle`.

This primitive contains **zero scheduling logic and no reactor** — it does
not decide what runs when, it only lets one Rust `Future` resolve once,
safely, when signaled from a different OS thread. That is not a "third async
runtime" in the sense option 3 was rejected for; it is a data structure, the
same shape as `futures::channel::oneshot`, hand-rolled here to avoid adding
`futures` as a dependency for one type. Its correctness rests entirely on
`std::task::Waker`'s own documented contract — `Waker::wake()` **must** be
safely callable from any thread; that is the entire reason `Waker` (as
opposed to a plain callback) exists in `core::task`. Any executor that
implements `Future`/`Waker` correctly, including a minimal hand-rolled one
(see below), honors this by construction. Note explicitly: **`monoio` itself
is never in the picture on the Windows side at all** (it cannot compile
there), so there is no need to verify anything against `monoio`'s specific
`Waker` implementation — the correctness burden here is the completion
primitive satisfying `std::task::Future`'s ordinary contract, a well-trodden
pattern, not an empirical question about a specific executor's internals.

**What this means for the top-level executor question (informational —
belongs to `ft991a`'s/`ts570d`'s own repos, not implemented here, not
authorized here):** since `#[monoio::main]` cannot exist on Windows, each
consuming application needs its own minimal Windows entry point to drive
`ui::run(...).await`. Because the completion primitive above is a plain
`Future` with no scheduling opinion, it composes with *any* correct
executor a Windows build chooses:

- **`ft991a`** (confirmed single-sequential-loop, no `monoio::spawn`
  anywhere): a hand-rolled ~30-line thread-parking `block_on` (poll in a
  loop; on `Poll::Pending`, `std::thread::park()`; the `Waker` calls
  `Thread::current().unpark()`) is sufficient — no new crate dependency at
  all. `monoio::time::sleep(5ms).await` becomes a plain
  `std::thread::sleep(5ms)`, behaviorally identical since nothing else needs
  to run concurrently in this architecture. This keeps the Windows "second
  runtime" footprint at effectively zero: standard-library `Waker`/
  `thread::park` plumbing, not a crate.
- **`ts570d`** (real concurrent two-task design via `monoio::spawn`): a
  future Windows port would need to replace `monoio::spawn`'s cooperative
  task with a genuine `std::thread::spawn`-based worker feeding results back
  over a channel to the UI thread's `block_on` loop, to preserve the
  "key events stay responsive during a slow poll" property. Heavier than
  `ft991a`'s needs, but still `monoio`-free, `tokio`-free, and does not
  require a third async-runtime crate. This is bookkeeping for a decision
  `ts570d` will need to make when it actually targets Windows — not
  authorized or dispatched by this ADR.

### 2. Crate/module structure: same crate, same public type names, platform-gated internals — not a new crate

`cat-transport-serial` gains a `#[cfg(target_os = "windows")] mod windows;`
alongside the existing `#[cfg(target_os = "linux")] mod io_uring;`, each
defining its own `SerialPort` (a fundamentally different internal shape —
`monoio::net::UnixStream` vs. a raw `HANDLE` + worker thread — cannot be one
struct definition), selected for the crate's public surface via cfg-gated
`pub use` in `lib.rs`. `SerialConfig`/`Parity`/`FlowControl` are pure data
with no platform-specific code at all; they move to a new **shared, ungated**
`cat-transport-serial/src/config.rs`, used by both platform modules, instead
of being duplicated. `SerialCatSession<T: Transport>` (`session.rs`) is
untouched — it was already generic over `T: Transport` and has no
platform-specific code, so this decision doesn't reach it at all.

**Rejected: a separate `cat-transport-serial-windows` crate.** `SerialPort`/
`SerialConfig` are the *only* integration point `ft991a`'s and `ts570d`'s
wiring code (`main.rs`) touches (confirmed by reading `ft991a/src/main.rs`).
A second crate would fork the type identity of `SerialConfig`/`SerialPort`
across platforms, forcing every downstream consumer to conditionally select
which crate to depend on and import per target — doubling the API surface
for a type family whose entire point is to present one platform-neutral
configuration shape. Keeping one crate with cfg-gated internals means `use
cat_transport_serial::{SerialPort, SerialConfig, SerialCatSession, ...}`
compiles unchanged on both platforms; only `Cargo.toml`'s own target-gated
dependency selection differs, invisibly to application code. This also
mechanically extends a convention this exact crate already established for
`monoio` (`[target.'cfg(target_os = "linux")'.dependencies]`, per ADR 0002's
Consequences) rather than inventing a new one.

### 3. Win32 implementation shape

- **Dependency**: `windows-sys` (the official Microsoft FFI-bindings crate;
  no `winapi`), added as
  `[target.'cfg(target_os = "windows")'.dependencies]` in
  `cat-transport-serial/Cargo.toml`, mirroring the existing
  `[target.'cfg(target_os = "linux")'.dependencies] monoio` entry exactly.
  Feature list: `Win32_Foundation`, `Win32_Storage_FileSystem`
  (`CreateFileW`/`ReadFile`/`WriteFile`/`CloseHandle`),
  `Win32_Devices_Communication` (`DCB`, `GetCommState`/`SetCommState`,
  `SetCommTimeouts`/`COMMTIMEOUTS`, `EscapeCommFunction`,
  `GetCommModemStatus`, the `MS_*_ON` bits). **No `Win32_System_IO`/
  `OVERLAPPED` features** — the design deliberately uses simple,
  non-overlapped (blocking) I/O confined to the dedicated worker thread
  rather than reimplementing IOCP-based overlapped I/O on top of it, which
  would be redundant complexity (effectively a second io_uring-equivalent)
  for a single-connection, low-throughput, one-request-at-a-time CAT
  protocol.
- **Open**: `CreateFileW` on `\\.\COMn`. `SerialPort::open` must prepend
  `\\.\` to whatever port name a caller passes (e.g. `"COM3"`) if not
  already present — required for COM ports numbered 10 and above, and safe
  for all of them; a naive pass-through of a bare `"COM3"`-style string
  would silently fail for two-digit port numbers. Error mapping to the
  existing `SerialError` variants: `ERROR_FILE_NOT_FOUND`/
  `ERROR_PATH_NOT_FOUND` → `DeviceNotFound`; `ERROR_ACCESS_DENIED` →
  `PermissionDenied`; else → `Io`. No new `SerialError` variant is expected
  to be needed.
- **Configure**: `GetCommState`/mutate `DCB`/`SetCommState`, the Windows
  analog of `configure_termios` — see the field-mapping table below.
- **Timeouts**: `SetCommTimeouts`, `ReadIntervalTimeout = 0`,
  `ReadTotalTimeoutMultiplier = 0`, `ReadTotalTimeoutConstant =
  READ_TIMEOUT.as_millis()` (reusing the existing 2s-production/100ms-test
  split already in `io_uring.rs`) — a single blocking `ReadFile` call then
  waits up to that bound and returns whatever arrived, possibly zero bytes.
  The worker thread maps a successful `ReadFile` returning 0 bytes to
  `Err(TransportError::ReadTimeout)`, preserving `Transport::read`'s
  existing contract exactly ("`Ok(n)` is always `> 0`; callers may treat
  `Ok(0)` as impossible"). `WriteTotalTimeoutConstant` is set to a generous
  bound (e.g. 5s) to fail a hung/removed-device write rather than hang
  forever, mapped to `TransportError::WriteTimeout` — an existing variant
  that has no caller on Linux today and gets its first real one here.
- **Read/write**: via the worker thread + completion primitive, as above.
- **`flush_rx`** (plain sync `fn` on `Transport`, default no-op, overridden
  by `SerialPort`): `PurgeComm(handle, PURGE_RXCLEAR)`, called directly and
  synchronously — same reasoning as `ModemControlLines` below, not routed
  through the worker thread.
- **`flush`** (`async fn`, Linux calls `tcdrain` synchronously inside the
  async body with the comment "intentionally short ... acceptable in an
  async context"): Windows equivalent is `FlushFileBuffers(handle)`, called
  the same way — directly and synchronously inside `async fn flush()`,
  **not** routed through the worker thread. This is a deliberate, narrow
  exception to the read/write design above, mirroring the Linux
  implementation's own existing exception exactly — not a reopening of the
  blocking-vs-worker-thread decision for the hot path (`read`/`write`).
- **`AsRawFd`**: `SerialPort` on Linux implements `std::os::fd::AsRawFd`.
  Grepping both `ft991a` and `ts570d` (application code, not this crate's
  own tests) found no caller of it. No Windows `AsRawHandle` parity is
  planned unless a concrete consumer needs it later.

### 4. `ModemControlLines` for Windows

Direct, synchronous Win32 calls on the calling thread against the same raw
`HANDLE` the worker thread also uses for I/O — **not** routed through the
worker thread or the completion primitive, mirroring ADR 0003's own
rationale for the Linux `ioctl` implementation exactly ("direct `ioctl(2)`
calls with no I/O wait"). `EscapeCommFunction`/`GetCommModemStatus` are the
Windows equivalents of that same "no I/O wait" shape. A Win32 `HANDLE` is
safe to use concurrently from multiple threads for independent operations —
`EscapeCommFunction`/`GetCommModemStatus` on the calling thread do not race
meaningfully against a concurrent blocking `ReadFile` on the worker thread
(asserting RTS/DTR does not invalidate an in-flight read) — so no
`DuplicateHandle` or second handle is needed.

```
set_rts(true/false)  → EscapeCommFunction(SETRTS / CLRRTS)
set_dtr(true/false)  → EscapeCommFunction(SETDTR / CLRDTR)
read_cts()            → GetCommModemStatus, test MS_CTS_ON
read_dsr()            → GetCommModemStatus, test MS_DSR_ON
read_dcd()            → GetCommModemStatus, test MS_RLSD_ON
```

Same public trait, same signatures (`&self`, `Result<_, TransportError>`) —
only the body differs per platform, exactly as the trait was designed to
allow.

### 5. `SerialConfig` field mapping (Linux termios ↔ Windows DCB)

Every field maps cleanly. No field is dropped or requires a shape change.

| Field | Linux (`termios`) | Windows (`DCB`) | Notes |
|---|---|---|---|
| `baud_rate: u32` | `cfsetispeed`/`cfsetospeed` via a fixed `BaudRate` enum (`baud_rate_from_u32`, rejects unsupported values) | `DCB.BaudRate: u32` — Windows accepts an arbitrary `u32` directly, no enum required | Windows is more permissive than Linux here. **Decision**: reuse the same validated rate set on Windows too (reject values `baud_rate_from_u32` would reject, with the same `SerialError::InvalidConfig`) — a deliberate cross-platform *consistency* choice, not a Win32 requirement, so `SerialConfig::default()` and every documented supported rate behave identically on both platforms. |
| `data_bits: u8` (5–8) | `CS5`/`CS6`/`CS7`/`CS8` in `control_flags` | `DCB.ByteSize: u8` (Windows legally allows 4–8; this crate only ever sets 5–8) | clean |
| `stop_bits: u8` (1 or 2) | `CSTOPB` flag | `DCB.StopBits`: `ONESTOPBIT`(0) / `TWOSTOPBITS`(2) | clean; Windows also has `ONE5STOPBITS`(1) — unreachable through this shared `u8` field on either platform, not a gap |
| `parity: Parity` (None/Even/Odd) | `PARENB`/`PARODD` | `DCB.Parity`: `NOPARITY`(0)/`EVENPARITY`(2)/`ODDPARITY`(1), plus `fParity = 1` to enable checking | clean; Windows also has `MARKPARITY`/`SPACEPARITY` — unused by this enum on either platform, not a gap |
| `flow_control: FlowControl` (None/Software/Hardware) | `CRTSCTS` (Hardware); `IXON`+`IXOFF` (Software) | Hardware: `fOutxCtsFlow=1`, `fRtsControl=RTS_CONTROL_HANDSHAKE`; Software: `fInX=1`, `fOutX=1` (`XonChar`/`XoffChar` default `0x11`/`0x13`, matching Linux's default `VSTART`/`VSTOP`); None: all off, `fRtsControl=RTS_CONTROL_ENABLE` | clean; **inherited tension, not new**: `Hardware` hands RTS to the driver on both platforms, which can conflict with a manual `ModemControlLines::set_rts` call — this pre-exists on Linux (`CRTSCTS` vs. `TIOCMBIS`) and is not a Windows-specific gap to solve differently |
| `initial_rts: bool` | `SerialPort::open` calls `set_rts(true)` post-open, via `ModemControlLines` | identical — `SerialPort::open` calls the same `set_rts`, now via `EscapeCommFunction` | clean, zero behavior difference |
| `initial_dtr: bool` | same, `set_dtr` | same, via `EscapeCommFunction` | clean |

## Consequences

- The Linux io_uring path is unaffected in behavior, wire format, and public
  API. The only Linux-visible change is mechanical: `SerialConfig`/`Parity`/
  `FlowControl` move from `io_uring.rs` into a new shared `config.rs`
  (same fields, same defaults, same doc comments, same re-export shape from
  `lib.rs`) so Windows doesn't duplicate/drift a second copy of pure data
  types — existing Linux tests are expected to pass unchanged.
- `cat-transport-serial`'s public surface (`SerialPort`, `SerialConfig`,
  `Parity`, `FlowControl`, `SerialCatSession`) is identical on both
  platforms; `ft991a`'s and `ts570d`'s `main.rs` wiring code needs no
  platform-specific branching to use it (only `Cargo.toml`'s existing
  target-gating mechanism differs, invisibly).
- `cat-transport-serial` gains a `windows-sys` dependency, target-gated to
  `cfg(target_os = "windows")`, alongside its existing Linux-gated `monoio`
  dependency — no unconditional new dependency for Linux builds.
- This crate's own test suite cannot execute Windows-target code in this
  project's Linux-only sandboxed environment. Verification for the Windows
  module is `cargo check --target x86_64-pc-windows-gnu -p
  cat-transport-serial` (compiles cleanly) — not `cargo test` — matching the
  user's explicit statement that they will validate against real Windows
  hardware themselves this round. The new completion primitive
  (`oneshot.rs`) is the one piece of new code with zero OS dependency of its
  own, so it alone gets real, executable unit tests, runnable on Linux CI
  even though its only consumer is the Windows module.
- No changes to `cat-transport-core`, `cat-transport-tcp`,
  `cat-transport-udp`, or `cat-server`. `ModemControlLines`'s trait
  definition (`cat-transport-core/src/modem.rs`) is unchanged — only
  `cat-transport-serial`'s Windows `impl` is new.
- `ft991a` and `ts570d` each still need their own follow-on work (not
  authorized or dispatched by this ADR, not touched by this planning pass)
  to replace `#[monoio::main]` with a Windows-compatible entry point, since
  `monoio` cannot compile on Windows at all. Section "1." above records the
  concrete shape that follow-on work should take for each repo, for
  whichever architect session eventually plans it.
- The dispatch queue implementing this ADR is in
  `planning/architect/task_plan.md` (Tasks 6–8, appended below the existing
  extraction/TCP/UDP/server queue).
