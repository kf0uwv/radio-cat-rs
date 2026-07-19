# Findings — cat_transport

(none logged during the earliest extraction tasks)

## ADR 0004 Task 6 (config.rs extraction + oneshot.rs) — 2026-07-19

- Grepped the whole workspace for `SerialConfig`/`FlowControl`/`Parity\b`
  before touching the re-export path: the only references outside
  `cat-transport-serial` itself are in that crate's own doc comments
  (`lib.rs`). No other in-workspace crate (`cat-client`, `cat-server`,
  `cat-transport-tcp`, `cat-transport-udp`, `cat-framework`,
  `cat-transport-core`) imports these types, so moving them from
  `io_uring::{...}` to `config::{...}` behind the same crate-root re-export
  path was confirmed zero-risk before doing it, not just assumed safe
  because "the re-export path didn't change."
- `Pin<&mut Self>::get_ref()` does not exist (only `Pin<&Self>::get_ref()`
  is stable) — had to use `&*self` (via `Pin`'s `Deref` impl) instead in
  `CompletionRx::poll`. Caught immediately by `cargo build`, not a design
  issue, just a compile-time correction while implementing `oneshot.rs`.
- `oneshot.rs`'s items are entirely unreferenced by non-test code as of
  this task (its only future consumer, the Windows worker-thread
  `SerialPort`, is Task 7/8 — explicitly out of scope here), which trips
  `dead_code` under `cargo clippy --all-targets -- -D warnings` on this
  Linux-only sandbox. Resolved with a documented `#![allow(dead_code)]` at
  the top of the module (same shape as the pre-existing
  `#[allow(dead_code)]` on `io_uring.rs`'s `write_to_master`, from the
  earlier `ModemControlLines` task) rather than inventing a fake caller
  just to silence the lint — flagged here per this crate's "don't
  improvise around an obstacle silently" guidance, not discovered
  silently.
- Verified the `config.rs` extraction alone (before writing `oneshot.rs`)
  reproduced the exact pre-refactor test count (18/18, same names) by
  temporarily removing `oneshot.rs`/its `mod` declaration, running the
  suite, then restoring both — an explicit intermediate checkpoint so the
  two pieces of this task's behavior claims (refactor is behavior-neutral;
  `oneshot.rs` only adds new tests) are each verified independently rather
  than only checked in combination.

## ADR 0004 Task 7 (Windows `SerialPort::open`/`configure_dcb`/
## `SetCommTimeouts`) — 2026-07-19

- **`io_uring.rs`/`lib.rs` were not actually Linux-gated post-Task-6**,
  contrary to what both the task brief and ADR 0004 §2's own prose assumed
  ("`cat-transport-serial` gains a `#[cfg(target_os = "windows")] mod
  windows;` alongside the existing `#[cfg(target_os = "linux")] mod
  io_uring;`"). Reading `lib.rs` showed `pub mod io_uring;`/`pub use
  io_uring::SerialPort;` unconditional in the file text. Confirmed
  empirically before writing any of my own code: `cargo check --target
  x86_64-pc-windows-gnu -p cat-transport-serial` (right after installing
  the target, against the untouched post-Task-6 tree) failed with 19
  errors, all Linux-only `libc` symbols (`TIOCMBIS`, `O_NONBLOCK`,
  `tcflush`, etc.) missing from `libc`'s Windows surface. This is a
  structural prerequisite for this task's own "Done when" bar, not
  something a new `windows.rs` could work around. Resolved by adding
  `#[cfg(target_os = "linux")]` to both the module declaration and the
  re-export — full reasoning for why this is Task 7's own scope (not a
  reach into Task 8's stated lib.rs work) is in `task_plan.md`'s Task 7
  section.
- **`windows-sys` exposes `DCB`'s packed bitfield members
  (`fBinary`/`fParity`/`fOutxCtsFlow`/`fRtsControl`/`fDtrControl`/...) as
  one opaque `_bitfield: u32`, with zero generated named accessor
  methods** — confirmed by grepping the entire locally-vendored
  `windows-sys` 0.52.0 source tree for `fParity`/`set_fParity`-style
  method names (none exist). Required hand-rolling the documented
  `winbase.h` bit layout in a private `dcb_bits` submodule.
- **`CreateFileW`'s generated `windows-sys` binding requires the
  `Win32_Security` feature**, not just the three features the task brief
  literally listed (`Win32_Foundation`/`Win32_Storage_FileSystem`/
  `Win32_Devices_Communication`) — because `lpSecurityAttributes`'s type
  (`SECURITY_ATTRIBUTES`) is defined under `Win32_Security`, and
  `windows-sys` gates the whole `CreateFileW` binding behind
  `#[cfg(all(feature = "Win32_Foundation", feature = "Win32_Security"))]`.
  Confirmed twice: by reading the vendored source directly, and
  empirically by removing the feature after the implementation compiled
  and re-running the acceptance command, which then failed with
  `error[E0432]: unresolved import ... no CreateFileW in
  Win32::Storage::FileSystem`. Added the feature; documented as a
  mechanical windows-sys binding-structure requirement, not a design
  change, and unrelated to the deliberately-excluded `Win32_System_IO`
  feature.
- **`windows-sys`'s `HANDLE` type changed between the locally-vendored
  0.52.0/0.48.0 (`pub type HANDLE = isize;`) and the version actually
  resolved by `Cargo.toml`, 0.59.0 (`pub type HANDLE = *mut
  core::ffi::c_void;`)** — a real, version-specific breaking change, not
  something visible from the vendored source alone. Caught immediately by
  the compiler on the first `cargo check --target x86_64-pc-windows-gnu`
  run against the actually-written `windows.rs`: `error[E0308]: mismatched
  types ... expected *mut c_void, found usize` on `CreateFileW`'s
  `hTemplateFile` argument (previously a literal `0`). Fixed with
  `std::ptr::null_mut()`. Side effect worth noting: this makes
  `RawHandle`'s `unsafe impl Send` genuinely necessary for the compiler on
  0.59 (a raw pointer is `!Send` by default), not merely documentation, as
  it would have been against 0.52's `isize`-typed `HANDLE`.
- `windows-sys = "0.59"` (the exact version literally specified in the task
  brief) is confirmed real and resolvable: `cargo check --target
  x86_64-pc-windows-gnu -p cat-transport-serial` actually downloaded
  `windows-sys v0.59.0` from crates.io during this task (log line: "Adding
  windows-sys v0.59.0 (available: v0.61.2)"). Not merely assumed from the
  task brief's wording.
- **Residual, not fixed (out of this task's file scope)**:
  `cargo clippy --target x86_64-pc-windows-gnu -p cat-transport-serial
  --all-targets` (as opposed to the task's actual "Done when" bar, plain
  `cargo check` on the lib target) fails — but the failure is entirely
  inside `session.rs` (untouched by this task, and by ADR 0004 §2's own
  description explicitly out of scope: "has no platform-specific code, so
  this decision doesn't reach it at all"), whose `#[cfg(test)]` module
  uses `#[monoio::test(driver = "legacy")]` unconditionally. This was
  already true before this task (pre-existing, not introduced by Task 7)
  and would fail identically even with an empty `windows.rs`, since
  `monoio` is Linux-only-target-gated and `session.rs`'s test attribute
  isn't itself gated. `cargo check --target x86_64-pc-windows-gnu -p
  cat-transport-serial` (lib only, the crate's actual default/non-test
  target, and this task's literal acceptance bar) and `cargo clippy
  --target x86_64-pc-windows-gnu -p cat-transport-serial --lib -- -D
  warnings` both pass cleanly. Flagged here for the architect to route to
  a future task (likely bundled with Task 8, which already touches
  `lib.rs` and is the first task with a real reason to run
  `cargo test`-shaped commands against the Windows target) rather than
  silently worked around or silently left unmentioned.
