# Progress — cat_transport

## Task 7 — `ModemControlLines` trait (RTS/DTR/CTS/DSR/DCD) (2026-07-18)

### Created / changed

- `cat-transport-core/src/modem.rs` (new) — `ModemControlLines` trait,
  verbatim signatures from the dispatch. Re-exported from `lib.rs`
  (`pub mod modem;` / `pub use modem::ModemControlLines;`), doc comment
  updated alongside the other trait summaries.
- `cat-transport-serial/src/io_uring.rs`:
  - New consts `TIOCM_RTS = 0x004`, `TIOCM_DTR = 0x002` (hoisted out of
    `open()`'s old inline block), plus new `TIOCM_CTS = 0x020`,
    `TIOCM_DSR = 0x100`, `TIOCM_CAR = 0x040` (DCD).
  - New private helpers `modem_bits_set(fd, bit, asserted)` (TIOCMBIS/
    TIOCMBIC) and `modem_bit_get(fd, bit)` (TIOCMGET + bitmask test), both
    returning `Result<_, TransportError>` via
    `TransportError::Io(std::io::Error::last_os_error())` on `ioctl`'s `-1`.
  - `impl ModemControlLines for SerialPort`: `set_rts`/`set_dtr` call
    `modem_bits_set`; `read_cts`/`read_dsr`/`read_dcd` call `modem_bit_get`.
  - `SerialPort::open`'s old inline "assert RTS+DTR via one combined
    TIOCMBIS ioctl call, ignore the error" block replaced: constructs
    `Self` first, then calls `port.set_rts(true)`/`port.set_dtr(true)`
    (discarding errors, same as before) gated on the new
    `config.initial_rts`/`config.initial_dtr` fields. Minor behavior note:
    this is now two separate ioctl syscalls instead of one combined
    `TIOCMBIS(RTS|DTR)` call — same end state, not atomic together; judged
    harmless since nothing depends on that atomicity.
  - `SerialConfig` gained `pub initial_rts: bool` / `pub initial_dtr: bool`,
    both `true` in `Default` — preserves today's unconditional-assert
    behavior exactly for every existing caller.
- `cat-transport-serial/src/session.rs` — blanket
  `impl<T: Transport + ModemControlLines> ModemControlLines for
  SerialCatSession<T>`, each method one-line-forwarding to
  `self.transport.*`, copying `CatSession::flush_rx`'s existing delegation
  shape exactly.
- Also fixed (pre-existing, unrelated to this task, discovered while
  running the required clippy check — see "Judgment calls" below):
  `#[allow(dead_code)]` added to the never-called `write_to_master` test
  helper in `io_uring.rs`'s test module.

### `TransportError` handling

No new variant. `TransportError::Io(#[from] std::io::Error)` already exists
and fits ioctl failures exactly — `std::io::Error::last_os_error()` after
`libc::ioctl` returns `-1` produces the same shape any other I/O failure in
this crate already uses. Checked all variants (`NotOpen`, `WriteTimeout`,
`ReadTimeout`, `Other(String)`) before concluding `Io` was the right fit;
none of the others describe a syscall failure.

### `SerialConfig::initial_rts`/`initial_dtr` — added, low risk confirmed

Added both fields. Before adding, grepped every `SerialConfig` construction
site in this workspace AND in `ft991a`/`ts570d` (read-only, not edited):
all three use `SerialConfig { ..., ..SerialConfig::default() }`
functional-update syntax, never an exhaustive struct literal — so no
existing caller's compile breaks. `cargo test --workspace` (this repo's own
7 crates) confirms nothing in this workspace broke either.

### Test-infrastructure limitation (real, not worked around)

Verified empirically (Python `fcntl.ioctl` against a fresh `pty.openpty()`
pair) that `TIOCMGET`/`TIOCMBIS`/`TIOCMBIC` return `ENOTTY` on both sides of
a Linux PTY on this kernel — consistent with the pre-existing comment on
`SerialPort::open`'s old RTS/DTR block. This means the existing
`TestPtyPair` test double (built on `nix::pty`, from Task 2) cannot
exercise the SUCCESS path of any `ModemControlLines` method — only the
error path. Did not invent new test infrastructure (no `libc::ioctl` mock,
no fake character device) to fabricate a success path; instead:
- `io_uring.rs::tests::test_modem_control_lines_return_err_on_pty` —
  exercises the real production ioctl code path against a real PTY-backed
  `SerialPort` for all 5 methods, asserting each returns
  `Err(TransportError::Io(_))` with `raw_os_error() == Some(libc::ENOTTY)`
  (not a panic, not a false success).
- `session.rs::tests::modem_control_lines_delegate_to_transport` — a
  `FakeTransport` (in-memory, `Cell`-backed for `&self` interior
  mutability) implementing `ModemControlLines` directly, proving the
  `SerialCatSession<T>` blanket-delegation SUCCESS path (values pass
  through unchanged in both directions) without needing real hardware.
- `io_uring.rs::tests::test_default_config_initial_rts_dtr_are_true` and
  `test_open_with_initial_rts_dtr_opted_out_still_succeeds` — cover the new
  `SerialConfig` fields' default and opt-out behavior.

The bit-actually-toggles-on-real-hardware success path for
`SerialPort::{set_rts,set_dtr,read_cts,read_dsr,read_dcd}` is not
verifiable in this environment (no real serial hardware, no kernel virtual
null-modem pair such as `tty0tty` available) and is reported as a genuine
gap, not silently glossed over.

### Acceptance checks (all run 2026-07-18, after restoring from an
### accidental mid-task `git stash`/`git stash pop` — verified no changes
### were lost)

- `cargo test -p cat-transport-core -p cat-transport-serial`: 12 + 18 = 30
  passed, 0 failed.
- `cargo clippy -p cat-transport-core -p cat-transport-serial --all-targets
  -- -D warnings`: clean, after the pre-existing `write_to_master`
  dead-code fix above (confirmed via a throwaway `git worktree` on HEAD
  that this failure predates this session's changes).
- `cargo fmt --all -- --check`: clean.
- `cargo test --workspace` (all 7 crates): 13 + 8 + 46 + 12 + 18 + 7 + 15 =
  119 passed, 0 failed, plus 7 clean (0-test) doc-test runs.
- `cargo clippy --workspace --all-targets -- -D warnings`: also run for
  extra confidence beyond the two required crates — clean.

### Judgment calls

1. `write_to_master` dead-code fix (see above) — pre-existing, unrelated,
   fixed with a 1-line `#[allow(dead_code)]` plus a comment explaining why,
   rather than deleting the helper or leaving the required clippy check
   red. Disclosed here rather than silently bundled in.
2. Two separate ioctl syscalls at `open()` time instead of the original
   single combined `TIOCMBIS(RTS|DTR)` call (see above) — same end state,
   judged harmless.
3. Mid-task, ran `git stash` while investigating whether a test module was
   pre-existing, which stashed this session's uncommitted work-in-progress
   changes. Caught immediately (system reminders showed reverted file
   content) and recovered with `git stash pop` — confirmed via `grep -c
   ModemControlLines` across all 4 changed files that nothing was lost.
   Recorded here for transparency, not because any work was actually lost.

## Task 2 — `cat-transport-core` + `cat-transport-serial` (2026-07-16)

### Created

- `cat-transport-core/` — `Cargo.toml`, `src/lib.rs`, `src/transport.rs`,
  `src/errors.rs`, `src/session.rs`, `src/test_support.rs`.
- `cat-transport-serial/` — `Cargo.toml`, `src/lib.rs`, `src/session.rs`,
  `src/io_uring.rs`.
- Root `Cargo.toml` `[workspace] members` updated to
  `["cat-framework", "cat-transport-core", "cat-transport-serial"]`.

### Notable decisions / discrepancies vs. `ts570d` source (not silently made)

1. **`cat-transport-core` re-exports `ResponseDisposition`/`ProtocolErrorKind`
   from `cat-framework`.** Required because the architect's dependency list
   for `cat-transport-serial` (and future TCP/UDP crates) names only
   `cat-transport-core`, not `cat-framework` directly, but `CatSession::execute`
   returns `Result<ResponseDisposition, Self::Error>`. Confirmed via
   `cargo tree -p cat-transport-serial`: `cat-framework` appears only
   transitively (under `cat-transport-core`), never as a direct `Cargo.toml`
   dependency.

2. **PTY test helper rebuilt locally instead of depending on `ts570d`'s
   `emulator` crate.** `ts570d`'s `serial/src/io_uring.rs` test module uses
   `emulator::pty::PtyPair`, which wraps the external `serialport` crate —
   neither `emulator` nor `serialport` is in the architect's authorized
   dependency list for `cat-transport-serial` (`cat-transport-core`,
   `monoio`, `async-trait`, `thiserror`, `libc`, `nix`). Built a minimal
   `TestPtyPair` in `cat-transport-serial/src/io_uring.rs`'s test module
   directly on `nix::pty::{posix_openpt, grantpt, unlockpt, ptsname_r}` —
   the same primitives `serialport::TTYPort::pair()` uses internally,
   already reachable via the `term` feature the spec already requires. No
   new dependency; no change to production code, framing, or the
   `Transport`/`CatSession` design. All 6 of `ts570d`'s original
   hardware/PTY-backed tests (`test_serial_port_open_on_pty_slave`,
   `test_transport_read_from_master`, `test_transport_write_to_master`,
   `test_read_blocks_until_data_arrives`, `test_transport_roundtrip`, plus
   the 3 pure-function unit tests) were reproduced and pass.

3. **`write_to_master` dead-code warning is pre-existing, not introduced
   here.** Verified byte-identical in `ts570d`'s own
   `serial/src/io_uring.rs` (`cargo clippy -p serial --all-targets` in the
   `ts570d` checkout reproduces the same warning). Left as-is rather than
   silently editing behavior mid-move.

### Acceptance checks (all green)

```
$ cargo test -p cat-transport-core -p cat-transport-serial
cat-transport-core:   12 passed; 0 failed
cat-transport-serial: 14 passed; 0 failed
(doc-tests: 0/0 for both, as expected — no doc examples written)

$ cargo test --workspace
34 passed; 0 failed   (adds cat-framework's 8)

$ cargo clippy -p cat-transport-core -p cat-transport-serial --all-targets
0 errors; 1 pre-existing dead-code warning (write_to_master — see note 3 above)

$ cargo fmt --all -- --check
clean (no diff)
```

`cargo tree -p cat-transport-core`: local deps = `cat-framework` only
(plus external: async-trait, thiserror, monoio + transitive deps) — matches
Task 2's "Done when" criterion exactly.

`cargo tree -p cat-transport-serial`: local deps = `cat-transport-core`
only (direct); `cat-framework` appears solely as a transitive dependency
underneath `cat-transport-core` — matches
`.claude/agents/cat_transport.md`'s dependency rule ("depend on
cat-transport-core only — never ... cat-framework").

### Linux target-gating (ADR 0002)

Both crates declare `monoio` under
`[target.'cfg(target_os = "linux")'.dependencies]`, never a plain
`[dependencies]` entry. `cat-transport-core`'s `pub use monoio::{...}`
re-export is `#[cfg(target_os = "linux")]`-gated to match. `libc`/`nix`
are plain (unconditional) dependencies of `cat-transport-serial`, per the
architect's spec (only `monoio` is called out for target-gating).

### Status: STOPPING here

Per the one-task-at-a-time workflow (`.claude/agents/cat_transport.md`),
no further crates started. `cat-transport-tcp`/`cat-transport-udp` (Task
4a/4b) are separate later tasks, gated on architect/user review of this
task first. Nothing committed, per this session's standing rule.

Note: this repo's convention (established by `planning/architect/findings.md`
and similar) is a per-agent `findings.md` alongside `task_plan.md`/
`progress.md`. This session's harness disallows agents writing separate
"findings"-named report files, so the reasoning that would normally live in
`findings.md` is folded into this file and into `task_plan.md`'s "Judgment
call flagged before implementation" section instead; `findings.md` in this
directory is left at its original bootstrap placeholder content.

## Task 4a -- `cat-transport-tcp` (2026-07-16)

### Created

- `cat-transport-tcp/` -- `Cargo.toml`, `src/lib.rs`, `src/session.rs`
  (production code + test module).
- Root `Cargo.toml` `[workspace] members` updated to add
  `"cat-transport-tcp"` (now `["cat-framework", "cat-transport-core",
  "cat-transport-serial", "cat-transport-tcp", "cat-client"]`).

### Wire format writeup (for a future cat-server Task 5 agent -- read this,
### not the source, to build a wire-compatible listener)

Every message (request *or* response) sent over the TCP connection is one
frame:

```
+-----------------------------+----------------------------------+
| length prefix (4 bytes)     | payload (`length` bytes)          |
| u32, big-endian              | raw request/response bytes        |
+-----------------------------+----------------------------------+
```

- **Prefix width**: 4 bytes.
- **Prefix type/endianness**: unsigned 32-bit integer, **big-endian**
  (network byte order). E.g. a 3-byte payload is preceded by the bytes
  `00 00 00 03`.
- **What the prefix counts**: the payload's length in bytes ONLY. It does
  NOT include itself (the 4 prefix bytes), and it does NOT include any
  terminator.
- **Payload contents**: the raw CAT command/response bytes exactly as they
  would appear on the wire for serial (e.g. `FA;` or `FA00014250000;`), with
  NO additional wrapping, escaping, length-encoding, or terminator added by
  this framing layer. In particular: a payload does not need to end with
  `;`, and the framing layer never scans for one -- the length prefix alone
  determines where the payload ends. (Whether the payload text itself
  happens to end in `;` is a CAT-protocol-layer detail above this framing,
  not something the frame format requires or strips.)
- **Zero-length frames are valid and meaningful**: length prefix
  `0x00000000` followed by zero payload bytes is a complete, valid frame. On
  the response side this represents `ResponseDisposition::NoResponse` (the
  dispatch deliberately produced no response bytes) -- mirrors
  `cat-transport-core::ScriptedCatSession`'s existing convention that an
  empty response means `NoResponse`.
- **Max frame size**: 65536 (64 KiB) payload bytes, enforced on both the
  encode and decode side. A frame declaring a length greater than this MUST
  be rejected as soon as the length prefix is parsed -- BEFORE attempting to
  read, or allocate a buffer for, the declared payload. (Rationale for the
  specific number is a judgment call, see below -- the important
  wire-compatibility fact for Task 5 is just that both sides must apply the
  *same* cap, or a legitimate large-but-under-the-cap frame from one side
  could be rejected by the other.)
- **Request/response pairing is 1:1 and connection-ordered.** A client
  writes exactly one request frame, then reads exactly one response frame
  in reply, in that order, on the same connection, before sending its next
  request. There is no in-frame request/session ID in this format -- TCP's
  own ordered, reliable, connection-scoped delivery makes that unnecessary;
  request/session IDs are `cat-transport-udp`'s concern (Task 4b), not
  TCP's.
- **Hard requirement this places on cat-server's TCP listener**: it MUST
  write back exactly one response frame for every request frame it reads
  from a client -- even when the underlying `CatRadio`/broker dispatch
  produces no response bytes at all (e.g. a set-shaped command), the
  listener must still send an explicit *empty* (zero-length) frame back,
  never silence. `TcpCatSession` (the client side, this crate) blocks
  reading a response frame after every request it writes, with no
  timeout of its own -- if a peer ever goes silent after a request instead
  of sending at least a zero-length frame, the client's read hangs
  forever. (Timeout/liveness enforcement against a misbehaving radio or
  worker is `cat-server`'s own broker-level concern per its charter, not
  something this session type provides.)

### Judgment calls (not derived from any existing spec -- flagged, not
### silently assumed)

1. **Max frame size = 65536 (64 KiB).** Known TS-570D-class CAT
   command/response frames are on the order of tens of bytes (the widest
   known response type, e.g. `IF`/status frames, is well under 100 bytes).
   64 KiB leaves roughly three orders of magnitude of headroom for future
   radios or coarser batched frames, while still bounding the worst-case
   single-frame buffer allocation to a small, fixed amount per connection --
   relevant once `cat-server` (Task 5) is holding many concurrent client
   connections open. A sender needing a payload larger than this is a
   bug/misuse this framing does not support today, not an invitation to
   silently raise the limit.
2. **`TcpCatSession` wraps `monoio::net::TcpStream` directly, not generic
   over `cat_transport_core::Transport<T>`.** Unlike
   `SerialCatSession<T: Transport>`, this type is not generic over the
   byte-level `Transport` trait. The architect's Task 4a spec names
   `monoio::net::TcpStream` specifically, and length-prefixed framing needs
   owned-buffer `read_exact`/`write_all` primitives (`monoio::io::{
   AsyncReadRentExt, AsyncWriteRentExt}`), not `Transport`'s
   borrowed-`&mut [u8]``read`/`write` shape used by the serial byte loop. No
   part of the `Transport` trait was changed or reused here.
3. **New crate-local error type `TcpSessionError`, not a reuse of
   `cat_transport_core::TransportError`.** `CatSession::Error` is an
   associated type -- nothing requires every implementor to share one error
   enum, and `SerialError` (a separate type from `TransportError`) is
   existing precedent for a transport crate defining its own error type
   where its concerns don't map cleanly onto the shared one.
   `TcpSessionError` has two variants: `Io(#[from] std::io::Error)`
   (covers all I/O failures, including a peer disconnecting mid-frame,
   which surfaces as `io::ErrorKind::UnexpectedEof` from
   `read_exact`) and `FrameTooLarge { len: u32, max: u32 }` (a dedicated
   variant so an oversized frame is rejected cleanly and diagnosably,
   rather than overloaded onto `TransportError::Other(String)`).
4. **`CatSession::send` is NOT overridden** (unlike
   `SerialCatSession::send`, which overrides it to avoid reading after a
   set command the real radio never answers). The default (forward to
   `execute`, discard the response) is correct here specifically because
   this wire format's 1:1 framing guarantees a response frame -- possibly
   zero-length -- for every request, including fire-and-forget ones. See
   the "hard requirement" note above; this is the flip side of the same
   design point, called out explicitly since it is a discrepancy from the
   serial transport's behavior that a future reader might otherwise assume
   was an oversight.

### Tests

`cat-transport-tcp/src/session.rs`'s `tests` module, all run against a real
loopback `monoio::net::TcpListener`/`TcpStream` pair (an in-test "scripted
peer" task on the far end, driven with `monoio::spawn`), not just in-memory
fakes:

- `conformance_query_round_trip`, `conformance_set_is_fire_and_forget`,
  `conformance_surfaces_transport_error` -- call
  `cat_transport_core::conformance`'s three functions completely unchanged
  against `TcpCatSession`, per the "Done when" requirement.
- `handles_response_length_prefix_and_payload_arriving_in_separate_reads` --
  peer sends the 4-byte length prefix, then the payload split into two
  separate writes with real I/O completions between them; proves the
  client's framing reassembles a response delivered in multiple chunks
  rather than assuming one `read()` == one frame.
- `rejects_oversized_frame_without_reading_payload` -- peer declares a
  length of `MAX_FRAME_SIZE + 1` and sends zero payload bytes; asserts the
  client returns `FrameTooLarge` without hanging (which it would, if it
  tried to `read_exact` the declared length before checking the cap).
- `disconnect_mid_length_prefix_returns_error_not_hang` -- peer sends only 2
  of the 4 length-prefix bytes, then closes; asserts
  `Io(UnexpectedEof)`, not a hang or panic.
- `disconnect_mid_payload_returns_error_not_hang` -- peer sends a full
  4-byte prefix declaring 14 bytes, sends only 5 payload bytes, then
  closes; asserts `Io(UnexpectedEof)`, not a hang or panic.

The peer-side test helpers (`write_raw_frame`/`read_raw_frame` in the test
module) are a deliberately independent, hand-rolled encoder/decoder -- they
do NOT call the production `write_frame`/`read_frame` functions. This means
the test suite cross-checks the *documented* wire format (reproduced in
this section) against the production implementation, rather than only
checking `write_frame` against `read_frame` (which would pass even if both
silently agreed on some format other than the documented one).

### Acceptance checks (all green)

```
$ cargo test -p cat-transport-tcp
running 7 tests
test session::tests::disconnect_mid_length_prefix_returns_error_not_hang ... ok
test session::tests::conformance_set_is_fire_and_forget ... ok
test session::tests::conformance_query_round_trip ... ok
test session::tests::conformance_surfaces_transport_error ... ok
test session::tests::handles_response_length_prefix_and_payload_arriving_in_separate_reads ... ok
test session::tests::rejects_oversized_frame_without_reading_payload ... ok
test session::tests::disconnect_mid_payload_returns_error_not_hang ... ok
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
Doc-tests: 0/0 (no doc examples written)

$ cargo test --workspace
41 passed; 0 failed   (adds cat-transport-tcp's 7 to the prior 34)

$ cargo clippy -p cat-transport-tcp --all-targets
Finished, 0 warnings, 0 errors

$ cargo fmt --all -- --check
clean (no diff)
```

`cargo tree -p cat-transport-tcp`: local deps = `cat-transport-core` only
(direct); `cat-framework` appears solely as a transitive dependency
underneath `cat-transport-core`; no other local crate (`cat-transport-serial`,
`cat-client`, etc.) appears anywhere in the tree -- matches
`.claude/agents/cat_transport.md`'s dependency rule.

### Status: STOPPING here

Per the one-task-at-a-time workflow, no further crates started.
`cat-transport-udp` (Task 4b) is a separate later task, gated on
architect/user review of this task first. Nothing committed, per this
session's standing rule.

## Task 4b -- `cat-transport-udp` (2026-07-16)

### Created

- `cat-transport-udp/` -- `Cargo.toml`, `src/lib.rs`, `src/session.rs`
  (production code + test module).
- Root `Cargo.toml` `[workspace] members` updated to add
  `"cat-transport-udp"` (now `["cat-framework", "cat-transport-core",
  "cat-transport-serial", "cat-transport-tcp", "cat-transport-udp",
  "cat-client"]`).

### Envelope format writeup (for a future cat-server Task 5 agent -- read
### this, not the source, to build a wire-compatible listener)

This is an **independent design from `cat-transport-tcp`'s length-prefixed
framing**, not derived from it -- UDP is connectionless, guarantees neither
delivery nor ordering, but (unlike a TCP byte stream) already preserves
datagram/message boundaries on its own. The envelope is designed around
those actual properties rather than pretending UDP has a TCP-shaped
connection.

Every datagram (request *or* response) sent over this transport is one
**envelope**:

```
+-------------------+-------------------+----------------------------+
| session_id         | request_id        | payload (remaining bytes) |
| 8 bytes, u64 BE    | 8 bytes, u64 BE    | raw request/response bytes |
+-------------------+-------------------+----------------------------+
```

- **Header width**: 16 bytes total -- `session_id` (8 bytes) followed
  immediately by `request_id` (8 bytes).
- **Field type/endianness**: both fields are unsigned 64-bit integers in
  **big-endian** (network) byte order -- same endianness convention as
  `cat-transport-tcp`'s length prefix, kept consistent across the two
  crates deliberately (terminology/style parity for a reader comparing
  both writeups), even though the two wire formats are otherwise
  independent.
- **`session_id`**: identifies one logical session. Randomized once when a
  `UdpCatSession` is constructed (via `std::collections::hash_map::
  RandomState` -- deliberately NOT the external `rand` crate, which is not
  on this crate's authorized dependency list; see "Judgment calls" below).
  Constant for the life of the session object. Purpose: let a receiver
  (this session, or **especially** a future `cat-server` UDP listener
  serving many client sessions on one bound socket) distinguish which
  logical session a given datagram belongs to -- this is the field
  `cat-server`'s listener MUST use to demultiplex incoming client
  datagrams to the right per-client state, since many clients' datagrams
  can arrive at the same server socket/port.
- **`request_id`**: identifies one request within a session. A per-session
  counter, starting at 1, incrementing by exactly 1 for every
  `execute()`/`send()` call. Never reset, never reused, monotonically
  increasing for the lifetime of the session object. Purpose: correlates a
  response datagram with the specific request it answers, and is the
  deduplication cache's key (see below).
- **No length field.** This is the key structural difference from
  `cat-transport-tcp`'s frame: a UDP datagram already arrives (or doesn't)
  as one complete, boundary-preserving unit via `recv_from` -- there is
  nothing to reassemble across multiple reads the way TCP's byte stream
  requires. The payload is simply "every byte in this datagram after the
  16-byte header." A **hard requirement this places on any wire-compatible
  peer** (e.g. a future `cat-server` UDP listener): it must never split one
  logical response across multiple datagrams, and must never coalesce more
  than one logical message into a single datagram -- one envelope per
  datagram, always.
- **Payload contents**: same convention as `cat-transport-tcp` -- the raw
  CAT command/response bytes exactly as they would appear on the wire for
  serial (e.g. `b"FA;"` or `b"FA00014250000;"`), no additional wrapping,
  escaping, or terminator added by this framing layer.
- **Zero-length payload is valid and meaningful**: a datagram containing
  exactly the 16-byte header and nothing else represents
  `ResponseDisposition::NoResponse` -- identical convention to
  `cat-transport-tcp` and `cat-transport-core::ScriptedCatSession`.
- **Max payload size**: 1024 bytes (`MAX_PAYLOAD_SIZE`), enforced on the
  send side -- a request/response payload longer than this is rejected
  (`UdpSessionError::PayloadTooLarge`) before anything is sent. Deliberately
  much smaller than TCP's 64 KiB cap -- see "Judgment calls" below for the
  MTU/fragmentation reasoning. **Wire-compatibility fact for Task 5**: both
  sides must apply the same cap (or agree on one), same as TCP's max-frame
  requirement.
- **Receive-side allocation and its limitation**: this session allocates a
  fixed `16 + MAX_PAYLOAD_SIZE` = 1040-byte buffer for every `recv_from`.
  Unlike TCP's length prefix (which lets a reader reject an oversized frame
  *before* reading its payload), UDP gives no such preview -- a peer sending
  an oversized datagram anyway will have it silently truncated by the
  kernel to the receive buffer's size, with no explicit
  `FrameTooLarge`-equivalent signal. In practice a truncated datagram will
  almost certainly fail envelope decoding or `session_id`/`request_id`
  matching and simply be discarded as noise (see below), but this is a
  known, weaker guarantee than TCP's, not a hard rejection.
- **Request/response pairing is NOT connection-ordered** (unlike TCP).
  `execute()` sends one request envelope to a single, fixed peer address,
  then filters every received datagram through three checks before
  accepting it as *the* answer: (1) source address equals the configured
  peer, (2) `session_id` matches this session's own, (3) `request_id`
  matches the request just sent. Anything failing any check is silently
  discarded and the wait continues (bounded by the timeout below) -- never
  surfaced as an error, never misattributed as the answer.
- **This session never calls a kernel-level UDP `connect()`.** The
  source-address filter above is an explicit application-level check
  (`from != peer_addr`), not a kernel-level one -- deliberate, per the
  charter's "do not force connection-oriented semantics onto UDP." A future
  `cat-server` UDP listener, which serves *many* clients on one bound
  socket, cannot use `connect()` for this purpose anyway (that only works
  for a 1:1 socket); it should demultiplex by `(source_addr, session_id)`
  using the same envelope fields this session validates.
- **Hard requirement this places on cat-server's UDP listener, mirrored
  from TCP's "every request gets exactly one response" but adapted for
  UDP**: a well-behaved peer SHOULD answer every request envelope with
  exactly one response envelope carrying the same `session_id` and
  `request_id` (empty payload for `NoResponse`) -- but because UDP cannot
  guarantee delivery in either direction, `UdpCatSession` cannot *require*
  this the way `TcpCatSession` leans on TCP's reliability. The backstop
  instead is this session's own response timeout (below), not a protocol
  guarantee. **`cat-server`'s listener still must not skip sending an
  explicit empty-payload response envelope for a set-shaped command that
  produced no response bytes** -- doing so would make every fire-and-forget
  request indistinguishable from a lost packet, forcing the client to wait
  out its full timeout every time.

### Deduplication cache writeup

- **Key**: `request_id: u64` alone. Not `(session_id, request_id)` --
  unnecessary for this session type, since a single `UdpCatSession`
  instance has exactly one constant `session_id` for its whole lifetime;
  the cache is inherently already scoped to "this session's own completed
  requests." (**Note for Task 5**: `cat-server`'s own, separate server-side
  dedup cache -- which exists to avoid *re-executing* a duplicated incoming
  request, a genuinely load-bearing use of the same idea from the other
  side -- will need to key by `(session_id, request_id)` or
  `(peer_addr, request_id)`, since one server-side cache serves many
  client sessions at once. That is a distinct cache from this one, serving
  a distinct purpose -- see "Judgment calls" below.)
- **What is stored**: just the fact that a `request_id` has been completed
  (a `VecDeque<u64>`, checked with `.contains()`) -- no response bytes are
  cached, because this session is the *requester*, not the *answerer*; it
  has nothing useful to replay for a duplicate (it would just discard the
  duplicate and keep waiting for -- or already have -- the real answer to
  its currently outstanding request).
- **Eviction policy**: bounded FIFO, capacity 32 (`DEDUP_CACHE_CAPACITY`).
  Oldest completed `request_id` is evicted first once the cache is full.
- **When it is consulted**: only when an incoming envelope's `session_id`
  matches but its `request_id` does NOT match the request currently being
  waited on. The cache membership check classifies the mismatch as either
  an explicit **recognized duplicate** (in the cache) or an **unrecognized
  stale/foreign id** (not in the cache) -- but **both are discarded
  identically** (the wait loop continues either way).
- **Honest limitation, stated plainly rather than overclaimed**: because
  `request_id` is strictly monotonic and never reused within a session's
  lifetime, the plain "does this match the request I'm currently waiting
  for" check is already sufficient for correctness on its own -- the dedup
  cache does not change what gets accepted or rejected in this design
  today. It is implemented anyway because (a) the project charter
  (`.claude/agents/cat_transport.md`) names a deduplication cache as a
  required design element of this transport, (b) it gives an explicit,
  directly-testable/inspectable mechanism instead of an implicit side
  effect of integer comparison, and (c) it bounds memory for a long-lived
  session instead of growing an unbounded history. This is recorded
  explicitly so a future reader does not assume the cache is doing more
  structural work than it is.

### Response timeout (UDP-specific; `TcpCatSession` deliberately has none)

`response_timeout: Duration`, supplied at construction (`UdpCatSession::new`
takes it explicitly; `bind_to`/`bind_to_with_timeout` are convenience
constructors, the former defaulting to `DEFAULT_RESPONSE_TIMEOUT` = 2s).
Applied as **one fixed deadline per `execute()` call**
(`monoio::time::Instant::now() + response_timeout`, computed once at the
call's start), not a duration re-applied on every loop iteration -- if the
deadline were recomputed each time an irrelevant datagram was discarded, a
flood of noise could extend the wait indefinitely, defeating the bound.
`monoio::time::timeout_at(deadline, ...)` is used specifically (not
`timeout(duration, ...)`) to guarantee this.

This is a deliberate divergence from `cat-transport-tcp::TcpCatSession`,
which has **no** timeout of its own (see this file's Task 4a section --
`TcpCatSession` hangs forever if a peer goes silent after a request, by
design, deferring liveness enforcement to `cat-server`'s broker layer).
UDP cannot defer this the same way: TCP gives an OS-level EOF/disconnect
signal when a peer's connection drops, but a UDP peer that simply stops
sending produces **no signal at all** -- there is no lower layer capable of
detecting "the peer vanished" the way a TCP socket can. The project's own
task instructions for this crate require "some timeout or bounded-wait
behavior," which is why `UdpCatSession` owns one directly rather than
deferring it, unlike its TCP sibling.

**Hard requirement on any binary constructing a `UdpCatSession`**: the
monoio runtime must be built with its timer enabled
(`RuntimeBuilder::enable_timer()`, or `#[monoio::main(timer_enabled =
true)]`) -- `monoio::time::timeout_at` does nothing useful otherwise. This
is load-bearing, not optional.

### Judgment calls (not derived from any existing spec -- flagged, not
### silently assumed)

1. **`session_id` generated via `std::collections::hash_map::RandomState`,
   not the `rand` crate.** `rand` is not on this crate's authorized
   dependency list (`cat-transport-core`, `monoio`, `async-trait`,
   `thiserror` only). `RandomState::new().build_hasher().finish()` yields a
   value derived from a freshly OS-seeded `SipHash` instance without adding
   any new dependency. This is explicitly NOT cryptographic-quality
   randomness and is not used as one -- it only needs to make collisions
   between concurrently-alive sessions implausible, which it comfortably
   does.
2. **Max payload size = 1024 bytes**, smaller than TCP's 64 KiB. A UDP
   datagram larger than the path MTU (typically ~1472 payload bytes after
   Ethernet/IP/UDP headers) risks IP fragmentation, and a fragmented
   datagram is dropped in full if even one fragment is lost -- there is no
   partial-retransmission the way TCP recovers a dropped segment. 1024
   bytes (1040 with the header) stays comfortably under common MTU limits
   while still leaving roughly an order of magnitude of headroom over known
   CAT frame sizes (tens of bytes).
3. **Dedup cache capacity = 32 entries, FIFO eviction.** Since `request_id`s
   are monotonic and never repeat, a duplicate/stale datagram arriving more
   than 32 requests after the one it duplicates is not realistic for any
   plausible network delay/duplication behavior; bounds memory to a small
   constant regardless of session lifetime.
4. **No kernel-level `UdpSocket::connect()`.** Considered and rejected: it
   would add a kernel-level source-address filter "for free," but the
   charter explicitly says not to force connection-oriented semantics onto
   UDP, and mixing a kernel-level filter with the application-level
   `session_id`/`request_id` checks (which do the real correctness work
   regardless) would obscure which layer is actually responsible for
   correctness. The application-level `from != peer_addr` check achieves
   the same filtering without implying a connection exists.
5. **`bind_to` naming, not `connect` (unlike `TcpCatSession::connect`).**
   Deliberately avoided "connect" terminology for the convenience
   constructor despite the stylistic parity otherwise sought with TCP's
   writeup -- calling it `connect` would misleadingly imply a handshake or
   kernel-level peer filter that does not exist for this transport. `new`
   (wrap an existing socket) and `bind_to`/`bind_to_with_timeout` (bind a
   fresh ephemeral socket) are the two constructors.
6. **This session's dedup cache is a different mechanism from what
   `cat-server`'s server-side dedup will need**, flagged explicitly above
   under "Deduplication cache writeup" so Task 5 does not assume this
   crate's cache is reusable as-is: this crate's cache (client/requester
   side) only needs to recognize "have I already seen the answer to this,"
   never replays anything, and is keyed by `request_id` alone because one
   session has one constant `session_id`. `cat-server`'s cache (server/
   answerer side) needs to prevent *re-executing* a duplicated incoming
   request (a genuinely load-bearing use, unlike this crate's
   defense-in-depth one) and must key by something that discriminates
   between the many client sessions sharing one server socket -- i.e.
   `(session_id, request_id)` or `(peer_addr, request_id)` -- and must
   cache the actual **response bytes** so a duplicate request gets the same
   cached answer resent rather than a second execution against the
   physical radio.
7. **`UdpCatSession` is the requester side only, symmetric with
   `TcpCatSession`/`SerialCatSession`.** It does not "execute" incoming
   requests -- it sends requests and awaits responses from a remote
   answerer. The task's test-naming language ("duplicate delivery... must
   not double-execute or double-respond") is written from a more general
   angle than this specific crate's role; the tests below implement the
   client-side analogue faithfully (a duplicate/stale delivery must not be
   misattributed as the answer to a different, later request -- i.e. the
   session must not "double-respond" to the caller), rather than literally
   guarding against re-executing a command this crate never executes in the
   first place. `cat-server`'s own dedup cache (see point 6) is where
   "must not double-execute" is literally load-bearing.

### Tests

`cat-transport-udp/src/session.rs`'s `tests` module, 15 tests total:

- `conformance_query_round_trip`, `conformance_set_is_fire_and_forget`,
  `conformance_surfaces_transport_error` -- call
  `cat_transport_core::conformance`'s three functions completely unchanged
  against `UdpCatSession`, per the "Done when" requirement. The
  transport-error case is realized as a peer that receives the request and
  never answers -- UDP's equivalent of TCP's "peer disconnects", since UDP
  has no connection to drop; the session's own bounded wait is what turns
  silence into a surfaced error.
- `duplicate_response_delivery_does_not_corrupt_next_request` -- peer
  answers request 1 correctly, TWICE in a row (duplicate delivery); then
  answers request 2 once, correctly. Asserts execute(1) returns the correct
  answer and execute(2) still returns request 2's own answer, not a leaked
  copy of the request-1 duplicate still sitting in the socket buffer.
- `stale_response_for_older_request_is_not_misattributed_to_newer_request`
  -- peer answers request 1 normally, then (after receiving request 2)
  re-sends a stale copy of request 1's response before finally answering
  request 2 -- simulating a late/reordered delivery arriving only once a
  newer request is already outstanding. Asserts the stale reply is not
  misattributed to request 2.
- `never_answered_request_times_out_instead_of_hanging` -- peer receives
  the request and never responds. Asserts `Err(UdpSessionError::Timeout)`
  with the correct peer/timeout fields, and measures wall-clock elapsed
  time to prove a genuinely bounded wait (>= the configured timeout, and
  well under a generous multiple of it) rather than merely "eventually
  errored for some unrelated reason."
- `ignores_malformed_short_datagram_and_still_receives_real_response` --
  peer sends a 3-byte garbage datagram (too short for a full envelope
  header) before the real response; asserts the garbage is silently
  ignored as noise, not treated as an error or a hang.
- `ignores_datagram_with_foreign_session_id` -- peer sends a well-formed
  envelope with the correct `request_id` but a WRONG `session_id`, then the
  correctly-keyed response; asserts the foreign-session datagram is
  ignored, not accepted as the answer.
- `encode_decode_round_trip`, `encode_rejects_oversized_payload`,
  `decode_rejects_datagram_shorter_than_header`,
  `decode_accepts_header_only_datagram_as_empty_payload` -- pure-logic
  tests of the envelope encode/decode functions directly, no socket
  involved.
- `dedup_cache_recognizes_a_completed_request_id`,
  `dedup_cache_evicts_oldest_beyond_capacity`,
  `session_id_is_randomized_across_instances` -- pure-logic tests of the
  dedup cache and session-id randomization directly on a constructed
  `UdpCatSession`. These run under `#[monoio::test]` (not plain `#[test]`)
  even though no datagram is ever sent or received in them, because
  `UdpSocket::bind` itself touches monoio's internal driver thread-local
  (via its `set_non_blocking` call under the default `legacy` feature) and
  panics outside a monoio runtime context -- discovered while writing these
  tests, noted here so a future reader isn't confused by a "no I/O" test
  requiring `#[monoio::test]`.

As with `cat-transport-tcp`'s test module, the test-side `raw_envelope`/
`parse_raw_envelope` helpers are a deliberately independent, hand-rolled
encoder/decoder -- they do NOT call the production `encode_envelope`/
`decode_envelope` functions, so the suite cross-checks the *documented*
wire format against the production implementation rather than only
checking the encoder against itself.

### Acceptance checks (all green)

```
$ cargo test -p cat-transport-udp
running 15 tests
test session::tests::conformance_query_round_trip ... ok
test session::tests::conformance_set_is_fire_and_forget ... ok
test session::tests::conformance_surfaces_transport_error ... ok
test session::tests::decode_accepts_header_only_datagram_as_empty_payload ... ok
test session::tests::decode_rejects_datagram_shorter_than_header ... ok
test session::tests::dedup_cache_evicts_oldest_beyond_capacity ... ok
test session::tests::dedup_cache_recognizes_a_completed_request_id ... ok
test session::tests::duplicate_response_delivery_does_not_corrupt_next_request ... ok
test session::tests::encode_decode_round_trip ... ok
test session::tests::encode_rejects_oversized_payload ... ok
test session::tests::ignores_datagram_with_foreign_session_id ... ok
test session::tests::ignores_malformed_short_datagram_and_still_receives_real_response ... ok
test session::tests::never_answered_request_times_out_instead_of_hanging ... ok
test session::tests::session_id_is_randomized_across_instances ... ok
test session::tests::stale_response_for_older_request_is_not_misattributed_to_newer_request ... ok
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
Doc-tests: 0/0 (no doc examples written)

$ cargo test --workspace
running total across all crates: 69 passed; 0 failed
(cat-framework 8, cat-transport-core 12, cat-transport-serial 14,
cat-transport-tcp 7, cat-transport-udp 15, cat-client 13)

$ cargo clippy -p cat-transport-udp --all-targets
Finished, 0 warnings, 0 errors

$ cargo clippy --workspace --all-targets
0 errors; 1 pre-existing warning (cat-transport-serial's write_to_master
dead-code, documented in this file's Task 2 section -- not introduced here)

$ cargo fmt --all -- --check
clean (no diff) -- one formatting pass was needed and applied before this
was clean (long single-line `assert!` calls with message args that
rustfmt wraps across lines); nothing else needed reformatting
```

`cargo tree -p cat-transport-udp`: local deps = `cat-transport-core` only
(direct); `cat-framework` appears solely as a transitive dependency
underneath `cat-transport-core`; no other local crate (`cat-transport-tcp`,
`cat-transport-serial`, `cat-client`, etc.) appears anywhere in the tree --
matches `.claude/agents/cat_transport.md`'s dependency rule exactly.

### Status: STOPPING here

Per the one-task-at-a-time workflow, no further crates started. Task 5
(`cat-server`) is a separate agent's later task, gated on architect/user
review of this task first. Nothing committed, per this session's standing
rule. `ts570d`/`ft991a` untouched, `cat-transport-tcp` untouched.

## Task 6 -- expose codec primitives as `pub` for `cat-server` reuse (2026-07-17)

### Why

The coordinating session's own code review of `cat-server` (built by a
separate `cat_server` agent in a prior task) found that `cat-server/src/
tcp.rs` and `cat-server/src/udp.rs` each hand-rolled an independent copy of
this crate's exact codec logic (frame/envelope encode-decode, size
constants) rather than importing it, purely because the relevant functions
were private -- `MAX_FRAME_SIZE`/`ENVELOPE_HEADER_LEN`/`MAX_PAYLOAD_SIZE`
were already `pub` but `write_frame`/`read_frame` (TCP) and
`encode_envelope`/`decode_envelope` (UDP) were not. Two independently
hand-written copies of the same wire format have no compiler-enforced
guarantee of staying in sync. This task closes that gap: visibility/
API-surface changes only, no wire-visible behavior change, no redesign.

### `cat-transport-tcp` changes

- `write_frame(stream: &mut TcpStream, payload: &[u8]) -> Result<(),
  TcpSessionError>` -- now `pub`. Unchanged in shape/behavior.
- `read_frame(stream: &mut TcpStream) -> Result<Vec<u8>, TcpSessionError>`
  -- now `pub`, but re-shaped into a thin wrapper over the new
  `read_frame_or_eof` (this task's option (a), per the task instructions).
  Behavior from a caller's perspective is unchanged: any I/O failure,
  including a clean disconnect before a single byte arrives, is still
  `Err(TcpSessionError::Io(..))` with `ErrorKind::UnexpectedEof` in that
  case (the exact message text differs -- "connection closed before a
  frame was received" instead of monoio's own read_exact message -- but no
  test or caller inspects that text, only `.kind()`).
- New: `read_frame_or_eof(stream: &mut TcpStream) -> Result<Option<Vec<u8>>,
  TcpSessionError>`. `Ok(Some(payload))` on a complete frame;
  `Ok(None)` **only** when a clean disconnect is observed with zero bytes
  of a new frame already read (a boundary hangup between requests -- not
  an error); `Err` for anything else (mid-frame disconnect after >=1 byte
  already arrived, oversized declared length, or any other I/O failure).
  This is exactly the primitive a server-side accept loop needs
  (`cat-server/src/tcp.rs`'s own current `read_request_frame` reimplements
  this same distinction today, but imperfectly -- see "Known
  discrepancy" below).
- New private helper `read_exact_or_eof(stream, len) -> Result<Option<Vec<u8>>,
  TcpSessionError>`: tracks bytes read so far itself, because monoio's own
  `AsyncReadRentExt::read_exact` does not expose the partial byte count on
  failure (only success/failure), so a zero-byte-read EOF and a
  1..len-byte-read EOF are otherwise indistinguishable from its return
  value alone. Not `pub` -- no caller outside this crate's own 4-byte
  length-prefix read needs it today.
- `MAX_FRAME_SIZE` unchanged (`pub const u32`, still 64 KiB).
- `cat-transport-tcp/src/lib.rs` now re-exports: `pub use session::{
  read_frame, read_frame_or_eof, write_frame, TcpCatSession, TcpSessionError,
  MAX_FRAME_SIZE};`

**Known discrepancy for the `cat_server` follow-up task to be aware of**:
`cat-server/src/tcp.rs`'s current (soon to be replaced) `read_request_frame`
treats *any* `UnexpectedEof` from its single `read_exact(vec![0u8; 4])` call
as a clean boundary (`Ok(None)`) -- it cannot actually distinguish "0 bytes
read" from "1-3 bytes read then EOF" the way `read_frame_or_eof` now does,
because `read_exact` alone doesn't expose that. This crate's new
`read_frame_or_eof` is the *more correct* behavior (a disconnect after the
peer has already started sending a new request's length prefix is a
genuine mid-frame error, not a clean hangup) -- the follow-up task should
expect its existing tests (e.g. `end_to_end_disconnect_mid_stream_does_not_
wedge_the_server`, which disconnects with *zero* bytes sent) to keep
passing unchanged when it switches to importing `read_frame_or_eof`, since
that test's disconnect happens at a true zero-byte boundary either way.

### `cat-transport-udp` changes

- `encode_envelope(session_id: u64, request_id: u64, payload: &[u8]) ->
  Result<Vec<u8>, UdpSessionError>` -- now `pub`. Signature unchanged
  (still returns `Result`, rejecting a payload over `MAX_PAYLOAD_SIZE`
  before allocating/sending anything). Note for the follow-up task:
  `cat-server/src/udp.rs`'s current hand-rolled `encode_envelope` returns a
  bare `Vec<u8>` (no size check, no `Result`) -- switching to this crate's
  version means the follow-up task's call sites will need to handle (or
  deliberately discard/unwrap, since server-generated responses are
  presumably already bounded) the `Result`.
- `decode_envelope(datagram: &[u8]) -> Option<(u64, u64, &[u8])>` -- now
  `pub`. Signature unchanged (identical to `cat-server`'s own hand-rolled
  copy already).
- `ENVELOPE_HEADER_LEN`/`MAX_PAYLOAD_SIZE` unchanged (`pub`, already were).
  `SESSION_ID_LEN`/`REQUEST_ID_LEN` visibility deliberately left as-is
  (crate-private) -- confirmed by reading (not editing)
  `cat-server/src/udp.rs`: its own duplicated `SESSION_ID_LEN`/
  `REQUEST_ID_LEN` constants are only used internally by its own
  `decode_envelope`/`encode_envelope` copies, which the follow-up task will
  delete in favor of importing this crate's `encode_envelope`/
  `decode_envelope` directly -- nothing in that file needs the two length
  constants split out on their own once that happens.
- `cat-transport-udp/src/lib.rs` now re-exports: `pub use session::{
  decode_envelope, encode_envelope, UdpCatSession, UdpSessionError,
  DEDUP_CACHE_CAPACITY, DEFAULT_RESPONSE_TIMEOUT, ENVELOPE_HEADER_LEN,
  MAX_PAYLOAD_SIZE};`

### What `cat-server`'s follow-up task needs to import

```rust
use cat_transport_tcp::{read_frame, read_frame_or_eof, write_frame, MAX_FRAME_SIZE};
use cat_transport_udp::{decode_envelope, encode_envelope, ENVELOPE_HEADER_LEN, MAX_PAYLOAD_SIZE};
```

`cat-server`'s TCP listener (`cat-server/src/tcp.rs`) should replace its
own `read_request_frame`/`write_response_frame`/local `MAX_FRAME_SIZE` with
`read_frame_or_eof`/`write_frame`/`cat_transport_tcp::MAX_FRAME_SIZE`
directly -- `read_frame_or_eof`'s `Ok(None)`/`Err` shape already matches
what `read_request_frame`'s callers expect (`Ok(Some(payload))` ==
request read, `Ok(None)` == clean disconnect at a boundary, `Err` ==
mid-frame/oversized), modulo the error type being `TcpSessionError` instead
of `io::Error` (the follow-up task will need to adapt its `match` arms and
its "tell the client why" `format!("ERR {e}")` formatting, which still
works since `TcpSessionError` implements `Display` via `thiserror`).

`cat-server`'s UDP listener (`cat-server/src/udp.rs`) should replace its
own `encode_envelope`/`decode_envelope`/local `SESSION_ID_LEN`/
`REQUEST_ID_LEN`/`ENVELOPE_HEADER_LEN`/`MAX_PAYLOAD_SIZE` with the
imports above -- again modulo `encode_envelope` now returning a `Result`
rather than a bare `Vec<u8>` (see "Known discrepancy" note above).

This crate's own `MAX_DATAGRAM_SIZE`/`DedupCache`/registry/broker-wiring
code in `cat-server` is out of scope for this task and untouched -- only
the codec-level building blocks are addressed here, per the task's explicit
"do not edit `cat-server/`" instruction.

### Acceptance checks (all green)

```
$ cargo test -p cat-transport-tcp -p cat-transport-udp
cat-transport-tcp:  7 passed; 0 failed  (all pre-existing tests, unchanged)
cat-transport-udp: 15 passed; 0 failed  (all pre-existing tests, unchanged)
Doc-tests: 0/0 for both (no doc examples written)

$ cargo test --workspace
115 passed; 0 failed across all 7 crates (cat-framework 13, cat-client 8,
cat-server 46, cat-transport-core 12, cat-transport-serial 14,
cat-transport-tcp 7, cat-transport-udp 15) -- confirms `cat-server`'s
existing (not-yet-updated) hand-rolled codec still compiles and passes
unchanged; this task did not touch it.

$ cargo clippy -p cat-transport-tcp -p cat-transport-udp --all-targets -- -D warnings
Finished, 0 warnings, 0 errors

$ cargo clippy --workspace --all-targets
0 errors; 1 pre-existing warning (cat-transport-serial's write_to_master
dead-code, documented in this file's Task 2 section -- not introduced
here, not in scope for this task)

$ cargo fmt --all -- --check
clean (no diff) -- `cargo fmt` reformatted `read_frame_or_eof`'s new
multi-line signature onto one line during development; re-verified clean
afterward.
```

### Status: STOPPING here

Per the one-task-at-a-time workflow, no further work started. This was a
direct dispatch (not routed through `planning/architect/`), so there is no
architect task-plan entry to cross-reference, but the same "stop and report"
discipline applies: nothing committed, per standing rule. The actual
`cat-server` consumption of this new `pub` API is explicitly a different
agent's follow-up task, not started here. `ts570d`/`ft991a` untouched;
`cat-server/` read only, never edited.

## ADR 0004 dispatch queue, Task 6 — `config.rs` extraction + `oneshot.rs`
## completion primitive (2026-07-19)

Full spec: `docs/adr/0004-windows-serial-backend.md` (read in full) +
`planning/architect/task_plan.md`'s `### Task 6` heading (that file's
Task 6, not this file's own unrelated, already-completed, identically
numbered "Task 6" section above — see the disambiguating note in
`task_plan.md`'s new section for why the collision exists). Both pieces are
groundwork for Tasks 7/8's Windows `SerialPort`; neither Windows-specific
code nor `Cargo.toml` changes are part of this task.

### Baseline, before touching any file

```
$ cargo test -p cat-transport-serial
18 passed; 0 failed; 0 ignored
```
(io_uring::tests × 12, session::tests × 6 — full list matches the "after"
run below minus the 4 new `oneshot::tests`.)

```
$ cargo test --workspace
cat-client 13, cat-framework 8, cat-server 46, cat-transport-core 12,
cat-transport-serial 18, cat-transport-tcp 7, cat-transport-udp 15
= 119 passed total across all 7 crates; 0 failed.
```

### Created / changed

- `cat-transport-serial/src/config.rs` (new, ungated) — `SerialConfig`,
  `Parity`, `FlowControl`, and `SerialConfig`'s `Default` impl, moved out of
  `io_uring.rs` verbatim (same fields, same `Default` values, same doc
  comment prose). The `initial_rts`/`initial_dtr` doc comments' intra-doc
  links were re-qualified to resolve from the new location (`SerialPort` and
  `ModemControlLines` aren't in scope in `config.rs` the way they were in
  `io_uring.rs`) — `[`SerialPort::open`]` → `[`crate::SerialPort::open`]`,
  `[`ModemControlLines::set_rts`]` →
  `[`cat_transport_core::ModemControlLines::set_rts`]`. Prose text is
  unchanged; only the link targets were adjusted, which is a necessary
  correctness fix for the move, not a content change — flagged here as a
  judgment call since "byte-for-byte identical" doc comments was the
  starting instruction and this is the one place actual bytes differ.
- `cat-transport-serial/src/io_uring.rs` — the `SerialConfig`/`Parity`/
  `FlowControl` definitions and `Default` impl removed; replaced with
  `use crate::config::{FlowControl, Parity, SerialConfig};`. No other line
  in this file changed — `configure_termios`, `baud_rate_from_u32`,
  `SerialPort::open`/`Transport`/`ModemControlLines` impls, and every
  existing test are untouched (diff is exactly the deletion + one new
  `use` line, confirmed via `git diff --stat`: 49 deletions, 0 unrelated
  insertions besides the new `use`).
- `cat-transport-serial/src/oneshot.rs` (new, **private**: `mod oneshot;`
  in `lib.rs`, no `pub`) — the completion primitive. Key signatures:

  ```rust
  pub struct Canceled;

  enum Slot<T> { Empty, Value(T), Canceled }

  struct Completion<T> {
      slot: Mutex<Slot<T>>,
      waker: Mutex<Option<Waker>>,
  }

  pub struct CompletionTx<T> { shared: Arc<Completion<T>> }
  pub struct CompletionRx<T> { shared: Arc<Completion<T>> }

  impl<T> CompletionTx<T> {
      pub fn send(self, value: T) { .. }
  }
  impl<T> Drop for CompletionTx<T> { .. } // Empty -> Canceled + wake, if `send` was never called

  impl<T> Future for CompletionRx<T> {
      type Output = Result<T, Canceled>;
      fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> { .. }
  }

  pub fn channel<T>() -> (CompletionTx<T>, CompletionRx<T>) { .. }
  ```

  `Slot<T>` (not a bare `Option<T>`) because the empty/canceled distinction
  needs a third state a plain `Option` can't express — still the
  `Mutex<Option<T>> + Option<Waker>` *shape* ADR 0004 §1 specifies, just
  with a 3-variant enum standing in for the 2-variant `Option`.
  `#![allow(dead_code)]` added at module top (documented inline): every
  item here is genuinely unreferenced by non-test code until Task 7/8 wires
  in the Windows worker thread, which trips `-D warnings` dead-code
  otherwise; same pattern as the pre-existing `write_to_master`
  `#[allow(dead_code)]` in `io_uring.rs`'s tests.
- `cat-transport-serial/src/lib.rs` — added `pub mod config;` and
  `mod oneshot;`; changed `pub use io_uring::{FlowControl, Parity,
  SerialConfig, SerialPort};` to `pub use config::{FlowControl, Parity,
  SerialConfig};` + `pub use io_uring::SerialPort;` (two lines instead of
  one, same net re-export set from the crate root — `SerialPort` still
  comes from `io_uring`, the config types now come from `config`). Crate
  doc comment extended with a short paragraph pointing at ADR 0004 §1/§2
  for why `config.rs`/`oneshot.rs` exist, without touching the existing
  historical extraction-provenance paragraphs above it.

### Compile-error correction while implementing `oneshot.rs`

`Pin<&mut CompletionRx<T>>::get_ref()` does not exist — `get_ref` is only
defined for `Pin<&Self>` (immutable), not `Pin<&mut Self>`. Caught by
`cargo build -p cat-transport-serial` immediately (`E0599`). Fixed by using
`&*self` (via `Pin`'s `Deref` impl) instead, which is sound here because
`CompletionRx<T>` has no `!Unpin` fields (just an `Arc`) and nothing in
`poll` needs a pinned field projection — documented inline in the method's
comment.

### Test scenarios and results (`oneshot::tests`, all pure `std`, no new
### dependency — `Cargo.toml` untouched, confirmed via `git diff`)

1. `poll_before_send_returns_pending_and_registers_waker` (task's (a)) — a
   `RecordingWaker` (`impl std::task::Wake`, records a bool) is polled
   against a fresh `CompletionRx` before any `send`: asserts `Poll::Pending`
   and that the waker has NOT fired yet. Then `send(7)` is called
   (same-thread, synchronous) and asserts the *previously-registered* waker
   now HAS fired, then polls again and asserts `Ready(Ok(7))`. Result: PASS.
2. `cross_thread_send_after_delay_wakes_and_resolves_with_value` (task's
   (b)) — a separate `std::thread::spawn`, after a genuine
   `sleep(Duration::from_millis(50))` (not same-tick/immediate), calls
   `tx.send(42)`. The main thread awaits via a small test-only
   `block_on_with_timeout` helper (park-loop + a `ThreadWaker` that
   unparks, the same shape ADR 0004 §1 describes for `ft991a`'s eventual
   Windows `block_on`), bounded by a 5-second deadline. Asserts
   `Some(Ok(42))` (not `None`, which would mean it timed out instead of
   being woken). Result: PASS — genuinely exercises the cross-thread wake
   path, not a same-thread immediately-ready shortcut.
3. `dropping_sender_before_send_resolves_to_canceled_not_hang` (task's
   (c)) — same cross-thread-with-delay shape as (2), but the spawned
   thread `drop(tx)`s instead of sending. Bounded by the same
   `block_on_with_timeout` 5-second deadline, so a regression in the
   cancellation-wake path (e.g. `Drop` failing to check `Slot::Empty`
   correctly, or failing to wake the registered waker) fails this test
   with a clear assertion mismatch instead of hanging the test binary.
   Asserts `Some(Err(Canceled))`. Result: PASS.
4. `send_before_first_poll_resolves_immediately` (extra, not explicitly
   required by the task but a cheap gap-closer) — `send` called before any
   `poll` at all; first poll must resolve `Ready(Ok(..))` immediately
   without ever registering/needing a waker. Result: PASS.

### Before/after verification (full commands + output)

```
$ cargo test -p cat-transport-serial
running 22 tests
... (all 18 pre-existing tests, same names as the baseline run above, all `ok`) ...
test oneshot::tests::poll_before_send_returns_pending_and_registers_waker ... ok
test oneshot::tests::cross_thread_send_after_delay_wakes_and_resolves_with_value ... ok
test oneshot::tests::dropping_sender_before_send_resolves_to_canceled_not_hang ... ok
test oneshot::tests::send_before_first_poll_resolves_immediately ... ok
test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

Intermediate checkpoint (not part of the final diff — done to verify the
`config.rs` extraction alone is behavior-neutral, independent of
`oneshot.rs`'s additions): temporarily moved `oneshot.rs` aside and removed
its `mod oneshot;` line, re-ran `cargo test -p cat-transport-serial` →
**18 passed, 0 failed**, same 18 test names as the pre-change baseline,
confirming the extraction alone changes nothing observable. Then restored
`oneshot.rs` and the `mod` declaration and re-ran the full suite (22/22,
above).

```
$ cargo clippy -p cat-transport-serial --all-targets -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) -- 0 warnings, 0 errors

$ cargo fmt --check -p cat-transport-serial
(no output, exit 0 -- clean)

$ cargo test --workspace
cat-client 13, cat-framework 8, cat-server 46, cat-transport-core 12,
cat-transport-serial 22 (was 18), cat-transport-tcp 7, cat-transport-udp 15
= 123 passed total; 0 failed. Every crate's count identical to the 119
baseline except cat-transport-serial's +4 (the new oneshot tests).

$ cargo clippy --workspace --all-targets -- -D warnings
Finished -- 0 warnings, 0 errors (no pre-existing warnings surfaced this
time; the write_to_master allow from Task 7 is still in place and still
suppresses that one).

$ cargo fmt --all --check
(no output, exit 0 -- clean)
```

Grep confirming no other in-workspace crate references
`SerialConfig`/`Parity`/`FlowControl` (so the re-export-path move is
zero-risk beyond `cat-transport-serial` itself): logged in `findings.md`'s
matching section.

### Files touched (confirmed via `git status`/`git diff --stat`)

```
 M cat-transport-serial/src/io_uring.rs   | 49 deletions, 1 insertion (the new `use`)
 M cat-transport-serial/src/lib.rs        | 14 insertions/changes (mod + re-export split + doc paragraph)
?? cat-transport-serial/src/config.rs     | new file
?? cat-transport-serial/src/oneshot.rs    | new file
```

`Cargo.toml` (crate-level and workspace-level) untouched. No file outside
`cat-transport-serial/` touched by this task's code changes (planning files
in `planning/cat_transport/` excepted, per this crate's own planning
convention). No Windows-specific `SerialPort`/`Transport`/
`ModemControlLines` code written. `cat-transport-core`,
`cat-transport-tcp`, `cat-transport-udp`, `cat-server`, `cat-framework`,
`cat-client` untouched. `ts570d`/`ft991a` (sibling repos) untouched.

### Judgment calls / discrepancies vs. ADR 0004 §1

- ADR 0004 §1 names the shape as "`struct Completion<T>` / `channel<T>() ->
  (CompletionTx<T>, CompletionRx<T>)`, a `Mutex<Option<T>> + Option<Waker>`
  pair." Implemented `Completion<T>` as the private shared struct exactly
  as named, but its slot is `Mutex<Slot<T>>` (a 3-variant enum: `Empty` /
  `Value(T)` / `Canceled`) rather than a literal `Mutex<Option<T>>`. This is
  necessary, not a deviation in spirit: the ADR's own next sentence requires
  distinguishing "no value yet" from "sender dropped without sending" (the
  `Result<T, Canceled>` output type it specifies), and a bare
  `Mutex<Option<T>>` cannot represent three states. Flagged explicitly
  rather than silently picked, per this crate's "if the plan specifies a
  shape, don't substitute without flagging" rule — judged as filling in an
  implementation detail the ADR's prose left slightly underspecified
  (`Option<T>` vs. a 3-state slot), not overriding a decision it made
  explicitly.
- ADR 0004 §1 says `CompletionRx<T>: Future<Output = T>` in one sentence,
  then "(or `Result<T, Canceled>` if the sender can be dropped before
  sending — needed for the worker-thread-exits-mid-request case)" in the
  next — and the ADR's own later prose ("SerialPort::drop drops the request
  sender ... the thread exits its loop") confirms the sender-dropped case
  is real and expected. Implemented `Result<T, Canceled>`, matching the
  task's own dispatch text (`planning/architect/task_plan.md`'s Task 6
  section), which states the `Result` form directly rather than as an
  alternative — no actual ambiguity once both documents are read together.
- No other discrepancy found. Everything else (private module, not
  `Transport`-facing, pure `std`, `Waker`-stores-on-`Pending`,
  `send`/`Drop` both wake) matches ADR 0004 §1 as specified.

### Status: implementation complete, all acceptance checks green, awaiting
### architect/user review

Not committed, per standing rule. STOPPING here per the one-task-at-a-time
workflow — Tasks 7/8 (Windows `SerialPort::open` / `Transport` /
`ModemControlLines` implementations over `windows-sys`) are separate, later
tasks, not started, not authorized by this task.

## ADR 0004 Task 7 (Windows `SerialPort::open`/`configure_dcb`/
## `SetCommTimeouts`) — 2026-07-19

### Files touched

```
 M cat-transport-serial/Cargo.toml       (+27, new [target.'cfg(target_os = "windows")'.dependencies] section)
 M cat-transport-serial/src/io_uring.rs  (baud_rate_from_u32 delegates to crate::baud; READ_TIMEOUT moved out)
 M cat-transport-serial/src/lib.rs       (mod baud/timeouts; #[cfg(target_os = "linux")] on io_uring mod+re-export; #[cfg(target_os = "windows")] pub mod windows;)
?? cat-transport-serial/src/baud.rs       (new, shared, ungated)
?? cat-transport-serial/src/timeouts.rs   (new, shared, ungated)
?? cat-transport-serial/src/windows.rs    (new, #[cfg(target_os = "windows")]-gated)
```

`git diff --stat` for the three modified files:
```
 cat-transport-serial/Cargo.toml      | 27 ++++++++++++
 cat-transport-serial/src/io_uring.rs | 83 ++++++++++--------------------------
 cat-transport-serial/src/lib.rs      | 37 +++++++++++++++-
```

No file outside `cat-transport-serial/` touched by this task's code
changes (planning files in `planning/cat_transport/` excepted). Confirmed
via `git status`: `docs/adr/0002-...md`, `docs/adr/README.md`,
`planning/architect/*`, `docs/adr/0003-...md` (untracked), `docs/adr/0004-...md`
(untracked) all show as already-modified/untracked *before* this task
touched anything — pre-existing state from earlier sessions (ADR 0004
authoring, Task 6 landing), not something this task changed.
`cat-transport-core`, `cat-transport-tcp`, `cat-transport-udp`,
`cat-server`, `cat-framework`, `cat-client` untouched. `ts570d`/`ft991a`
(sibling repos) not present in this workspace at all, untouched.

### `windows-sys` dependency added

```toml
[target.'cfg(target_os = "windows")'.dependencies]
windows-sys = { version = "0.59", features = [
    "Win32_Foundation",
    "Win32_Storage_FileSystem",
    "Win32_Devices_Communication",
    "Win32_Security",
] }
```

Version `0.59` is exactly what the task brief specified, empirically
confirmed real/resolvable (see `task_plan.md`'s Task 7 section — `cargo
check` actually resolved and downloaded `windows-sys v0.59.0`, log line
"Adding windows-sys v0.59.0 (available: v0.61.2)"). `Win32_Security` is an
addition beyond the task brief's literal three-feature list — mechanically
required for `CreateFileW`'s own `windows-sys` binding to exist at all
(confirmed by source inspection and by an empirical remove/re-add
experiment, both logged in `findings.md`). `Win32_System_IO` deliberately
NOT added, per the ADR.

### Verification — full command/output log

**1. Baseline, before touching anything** (establishes the pre-existing gap
this task had to fix as a prerequisite, and the pre-existing green state to
diff against):

```
$ rustup target list --installed
x86_64-unknown-linux-gnu

$ rustup target add x86_64-pc-windows-gnu
info: downloading component 'rust-std' for 'x86_64-pc-windows-gnu'
info: installing component 'rust-std' for 'x86_64-pc-windows-gnu'
(succeeded)

$ cargo check --target x86_64-pc-windows-gnu -p cat-transport-serial   # BEFORE any of this task's edits
error[E0425]: cannot find value `TIOCMBIS` in crate `libc`
... (19 errors total, all Linux-only libc symbols missing on the Windows
    target, from io_uring.rs being unconditionally compiled)
error: could not compile `cat-transport-serial` (lib) due to 19 previous errors

$ cargo build --workspace     # BEFORE
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.02s

$ cargo test -p cat-transport-serial   # BEFORE
test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**2. The acceptance bar itself, after all edits (including the `io_uring.rs`
gating fix and the `HANDLE`-type/`Win32_Security` fixes discovered along the
way):**

```
$ cargo check --target x86_64-pc-windows-gnu -p cat-transport-serial
    Checking cat-transport-serial v0.1.0 (.../cat-transport-serial)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.10s
```

Clean. No Windows toolchain/linker was invoked or needed — `cargo check`
stops before the link step, as the task brief anticipated ("a cargo
check cross-compile, not a link/run").

```
$ cargo clippy --target x86_64-pc-windows-gnu -p cat-transport-serial --lib -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.12s
```

Clean, zero warnings, on the lib target (the crate's actual default build
target and this task's literal "Done when" surface).

`cargo clippy --target x86_64-pc-windows-gnu -p cat-transport-serial
--all-targets -- -D warnings` (broader than the task's own bar) fails, but
only inside `session.rs` (untouched, pre-existing, out of this task's
scope per ADR 0004 §2 -- see `findings.md`'s "Residual, not fixed" entry
for the full explanation and why it isn't this task's to fix).

**3. Linux side, after all edits — confirming complete non-regression:**

```
$ cargo test -p cat-transport-serial
running 24 tests
test baud::tests::accepts_every_supported_rate ... ok
test baud::tests::rejects_unsupported_rate ... ok
... (all 22 pre-existing tests, same names, still ok) ...
test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo build --workspace
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.02s

$ cargo clippy -p cat-transport-serial --all-targets -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.25s   -- 0 warnings

$ cargo fmt -p cat-transport-serial -- --check
(exit 0, clean, after running `cargo fmt -p cat-transport-serial` once to
apply two formatting fixups `rustfmt` wanted in windows.rs -- a long match
arm and a long function-call argument list, both purely cosmetic line
wraps, no logic change)

$ cargo test --workspace
cat-client 13, cat-framework 8, cat-server 46, cat-transport-core 12,
cat-transport-serial 24 (was 22, +2 new baud::tests), cat-transport-tcp 7,
cat-transport-udp 15 = 125 passed total; 0 failed. Every crate's count
identical to before except cat-transport-serial's +2.

$ cargo clippy --workspace --all-targets -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.04s   -- 0 warnings

$ cargo fmt --all -- --check
(exit 0, clean)
```

Every pre-existing Linux test (all 24 now, 22 pre-existing + 2 new
`baud::tests`) passes with unchanged names/behavior. `Transport::read`'s
`READ_TIMEOUT`-driven tests (`test_read_blocks_until_data_arrives`, etc.)
still pass, confirming the `crate::timeouts::READ_TIMEOUT` move didn't
change its `#[cfg(test)]` 100ms value or behavior.

### `DCB` field-by-field mapping — confirmed against ADR 0004 §5's table,
### row by row

| ADR 0004 §5 row | `configure_dcb` implementation |
|---|---|
| `baud_rate: u32` — reuse Linux's validated rate set, same `SerialError::InvalidConfig` | `dcb.BaudRate = crate::baud::validate_baud_rate(config.baud_rate)?;` — same shared validator Linux's `baud_rate_from_u32` now calls first |
| `data_bits: u8` (5-8) → `DCB.ByteSize` | `match config.data_bits { 5=>5, 6=>6, 7=>7, _=>8 }` — mirrors `configure_termios`'s exact fallback-to-8 shape |
| `stop_bits: u8` (1 or 2) → `DCB.StopBits`: `ONESTOPBIT`(0)/`TWOSTOPBITS`(2) | `if config.stop_bits >= 2 { TWOSTOPBITS } else { ONESTOPBIT }` — identical test to Linux's `if config.stop_bits >= 2` |
| `parity: Parity` → `DCB.Parity` + `fParity` | `Parity::None => (NOPARITY, fParity=0)`, `Even => (EVENPARITY, fParity=1)`, `Odd => (ODDPARITY, fParity=1)` |
| `flow_control: FlowControl` → `fOutxCtsFlow`/`fRtsControl` (Hardware); `fInX`/`fOutX` (Software); all-off + `RTS_CONTROL_ENABLE` (None) | `Hardware => (outx_cts=true, in_x=false, out_x=false, RTS_CONTROL_HANDSHAKE)`; `Software => (false, true, true, RTS_CONTROL_ENABLE)`; `None => (false, false, false, RTS_CONTROL_ENABLE)`; `XonChar`/`XoffChar` set to `0x11`/`0x13` unconditionally (harmless when `fInX`/`fOutX` are off) |
| `initial_rts`/`initial_dtr` — Task 8's `EscapeCommFunction` calls, not this task | Not implemented here (correctly out of scope); `fDtrControl` set to `DTR_CONTROL_ENABLE` at configure time so Task 8's calls will actually take effect (judgment call, logged in `task_plan.md`/`findings.md`) |

Every row matches. Two fields the table doesn't name (`fBinary`,
`fDtrControl`) and several conservative-default fields with no
`SerialConfig`/Linux-termios equivalent (`fOutxDsrFlow`,
`fDsrSensitivity`, `fTXContinueOnXoff`, `fErrorChar`, `fNull`,
`fAbortOnError`) are set as judgment calls, documented inline in
`windows.rs` and in `task_plan.md`'s Task 7 section.

### `SetCommTimeouts` mapping

`ReadIntervalTimeout = 0`, `ReadTotalTimeoutMultiplier = 0`,
`ReadTotalTimeoutConstant = READ_TIMEOUT.as_millis() as u32` (the shared
`crate::timeouts::READ_TIMEOUT`, moved verbatim out of `io_uring.rs` — same
2000ms production / 100ms test split), `WriteTotalTimeoutMultiplier = 0`,
`WriteTotalTimeoutConstant = 5000` (5s, matching ADR 0004 §3's own
suggested value; reasoning logged in `task_plan.md`'s Task 7 section — no
real Windows hardware reachable from this sandbox to empirically tune it
further).

### `SerialPort::path` parity confirmed

Linux: `pub fn path(&self) -> &str { &self.path }`. Windows: identical
signature, identical body, in `windows.rs`.

### Judgment calls / discrepancies — summary (full reasoning in
### `task_plan.md`/`findings.md`, this is the index)

1. Gated `mod io_uring;`/`pub use io_uring::SerialPort;` to
   `#[cfg(target_os = "linux")]` — a prerequisite this task's own Done-when
   bar could not be met without, contrary to what the ADR's prose assumed
   was already true post-Task-6.
2. Added `Win32_Security` to the `windows-sys` feature list, beyond the
   task brief's literal three — mechanically required for `CreateFileW`'s
   binding to exist in `windows-sys` at all.
3. Extracted `baud.rs`/`timeouts.rs` as new shared, ungated modules
   (mirroring Task 6's `config.rs` precedent) rather than duplicating
   `READ_TIMEOUT`/the baud-rate-validated-set into `windows.rs`.
4. Set `fBinary = 1` and `fDtrControl = DTR_CONTROL_ENABLE` unconditionally
   in `configure_dcb` — fields ADR 0004 §5's table doesn't name a row for,
   filled in with documented reasoning (raw-mode prerequisite; manual DTR
   control so Task 8's `EscapeCommFunction` calls work), not overriding
   anything the ADR stated explicitly.
5. `WriteTotalTimeoutConstant = 5000` (5s) — the ADR's own suggested
   starting value, adopted as-is.
6. Used `std::ptr::null_mut()` (not a literal `0`) for `CreateFileW`'s
   `hTemplateFile` argument — forced by `windows-sys` 0.59's `HANDLE` being
   `*mut c_void`, not `isize` as in the locally-vendored 0.52.0/0.48.0
   sources initially consulted; caught by the compiler on the first real
   `cargo check` run against `windows.rs`, not missed.

None of these reach into Task 8's scope (`Transport`/`ModemControlLines`/
worker thread) — all are either mechanical prerequisites for this task's
own stated verification bar, or judgment calls explicitly invited by the
task brief's own wording ("your call", "use your judgment").

### Not fixed, flagged for the architect (out of this task's scope)

`session.rs`'s `#[cfg(test)]` module uses `#[monoio::test(driver =
"legacy")]` unconditionally (no `#[cfg(target_os = "linux")]`), so `cargo
check --target x86_64-pc-windows-gnu -p cat-transport-serial --all-targets`
(as opposed to this task's actual lib-only Done-when bar) fails. Pre-
existing, not introduced by this task, and outside this task's authorized
file scope (`session.rs` is explicitly untouched per ADR 0004 §2). See
`findings.md` for full detail.

### Done when (task's literal bar, restated and confirmed)

`cargo check --target x86_64-pc-windows-gnu -p cat-transport-serial`
compiles cleanly — **confirmed, exact command/output above**. `cargo
clippy`/`cargo fmt` for whatever compiles — confirmed clean on the Windows
lib target and on the full Linux crate/workspace. Linux `cargo test -p
cat-transport-serial` (24/24) and `cargo build --workspace` completely
unaffected in behavior (only test count grew by the 2 new `baud::tests`,
every pre-existing test name/behavior unchanged).

### Status: implementation complete, all acceptance checks green, awaiting
### architect/user review

Not committed, per standing rule. STOPPING here per the one-task-at-a-time
workflow — Task 8 (Windows `Transport`/`ModemControlLines`/worker thread)
is a separate, later task, not started, not authorized by this task.

## ADR 0004 Task 8 — Windows `Transport`/`ModemControlLines` + worker
## thread (2026-07-19)

Completes the Windows `SerialPort` (`cat-transport-serial/src/windows.rs`),
per ADR 0004 §1/§3/§4 and `planning/architect/task_plan.md`'s Task 8
(verbatim spec) / this file's own `task_plan.md` Task 8 section (full
re-statement). Depends on Task 6 (`oneshot.rs`) and Task 7 (`SerialPort::
open`/`configure_dcb`/`set_comm_timeouts`), both already landed and
independently verified.

### Worker-thread request/reply design (as actually built)

```rust
enum WorkerRequest {
    Read {
        len: usize,
        reply: oneshot::CompletionTx<Result<Vec<u8>, TransportError>>,
    },
    Write {
        data: Vec<u8>,
        reply: oneshot::CompletionTx<Result<usize, TransportError>>,
    },
}

fn worker_loop(handle: RawHandle, rx: mpsc::Receiver<WorkerRequest>) {
    while let Ok(request) = rx.recv() {
        match request {
            WorkerRequest::Read { len, reply } => reply.send(worker_read(handle, len)),
            WorkerRequest::Write { data, reply } => reply.send(worker_write(handle, &data)),
        }
    }
}
```

`worker_read` performs one blocking, non-overlapped `ReadFile` call
(`lpOverlapped = null_mut()`); a successful zero-byte completion (the
configured `ReadTotalTimeoutConstant` elapsed with no data, per Task 7's
`set_comm_timeouts`) is mapped to `Err(TransportError::ReadTimeout)`
*inside the worker*, before it ever replies — `Ok(bytes)` is therefore
always non-empty, matching `Transport::read`'s "`Ok(n)` is always `> 0`"
contract exactly (confirmed against `io_uring.rs`'s own doc comment on this
same contract, which is identical in spirit).

`worker_write` performs one blocking, non-overlapped `WriteFile` call; a
successful completion with `bytes_written < data.len()` (a short write —
the well-documented Win32 serial-I/O behavior when `WriteTotalTimeoutConstant`
elapses before the full buffer is transmitted, matching `pyserial`'s own
`serialwin32.py` handling) is mapped to `Err(TransportError::WriteTimeout)`
rather than returned as `Ok(partial_n)` — this gives `WriteTotalTimeoutConstant`
its first real caller on this platform, exactly as ADR 0004 §3 anticipated
("an existing variant that has no caller on Linux today and gets its first
real one here"). **Not independently verifiable against real hardware in
this sandbox** — the "partial write on timeout, not an outright `WriteFile`
failure" behavior is documented/standard Win32 serial-I/O interpretation,
not something this environment can empirically confirm; flagged here as a
judgment call rather than overclaimed as tested.

`SerialPort` gained two new fields: `request_tx: Option<mpsc::Sender<
WorkerRequest>>` and `worker: Option<thread::JoinHandle<()>>`, both `Some`
for the entire lifetime of an open port and only ever `take()`n by `Drop`.
The `Option` wrapping (rather than a bare `Sender`/`JoinHandle`) is what
makes `Drop::drop`'s exact required sequencing possible: `self.request_tx
.take()` drops the real sender in place (not a clone) so the worker's
`recv()` observes disconnection immediately; only then is `worker.join()`
called; only then `CloseHandle`. Joining before dropping the sender would
hang forever (the worker would still be parked in `recv()` with nothing
left to wake it) — this exact ordering is called out as "load-bearing for
clean shutdown" in both ADRs 0004 §1 and the dispatching prompt, and is
implemented exactly as specified.

`SerialPort::open` (Task 7's implementation, extended): after `configure_dcb`/
`set_comm_timeouts` succeed, constructs the `mpsc::channel()`, spawns
`thread::spawn(move || worker_loop(raw_handle, request_rx))`, builds `Self`,
then calls `self.set_rts(true)`/`self.set_dtr(true)` when `config.
initial_rts`/`initial_dtr` (both default `true`), discarding errors —
identical sequencing to `io_uring.rs`'s Linux implementation.

### `Transport for SerialPort`

- `write`/`read`: build an `oneshot::channel()` pair, send a `WorkerRequest`
  over `request_tx`, `.await` the `CompletionRx`. Both the `mpsc::Sender::
  send` failure case (worker's `Receiver` already dropped — i.e. the
  worker thread already exited, most plausibly via a panic since the
  ordinary shutdown path always joins first) and an `Err(oneshot::Canceled)`
  from the awaited `CompletionRx` (the `CompletionTx` was dropped without
  sending — same underlying cause) both map through a new helper,
  `worker_gone_error()`, to `TransportError::Io` wrapping a synthetic
  `ErrorKind::BrokenPipe` `std::io::Error`. Judgment call: `TransportError`
  has no dedicated "worker thread gone" variant; reusing `Io` matches ADR
  0004's own "no new `SerialError` variant is expected to be needed"
  spirit for the open/configure path, extended here to the read/write path
  for the same minimal-surface-change reasoning — `Io` already models
  "something went wrong at the OS/transport boundary," and a gone worker
  thread is exactly that.
- `read` copies the worker's returned `Vec<u8>` into the caller's `buf`
  (`buf[..n].copy_from_slice(&bytes)`), mirroring `io_uring.rs`'s own
  `VecBuf`-to-caller-buffer copy shape.
- `flush_rx` (plain sync `fn`, overriding the base trait's no-op default):
  `PurgeComm(handle, PURGE_RXCLEAR)`, called directly and synchronously —
  NOT routed through the worker thread, per ADR 0004 §3.
- `flush` (`async fn`): `FlushFileBuffers(handle)`, called directly and
  synchronously inside the async body — NOT routed through the worker
  thread, mirroring `io_uring.rs`'s `flush`'s own `tcdrain`-inside-async-
  body exception precisely (same doc-comment justification: a short,
  deliberately-blocking synchronous call is acceptable in an async context
  where callers invoke `flush` deliberately).

### `ModemControlLines for SerialPort`

Direct, synchronous Win32 calls on the calling thread against the same
`HANDLE` the worker thread also uses — never touching `request_tx`/a
`CompletionTx`/`CompletionRx`, per ADR 0004 §4:

```
set_rts(true/false)  → EscapeCommFunction(SETRTS / CLRRTS)
set_dtr(true/false)  → EscapeCommFunction(SETDTR / CLRDTR)
read_cts()            → GetCommModemStatus, test MS_CTS_ON
read_dsr()            → GetCommModemStatus, test MS_DSR_ON
read_dcd()            → GetCommModemStatus, test MS_RLSD_ON
```

via two small private helpers, `escape_comm_function`/`modem_status_bit`,
mirroring `io_uring.rs`'s `modem_bits_set`/`modem_bit_get` shape exactly
(same method-to-primitive mapping, same "no I/O wait" reasoning restated
in the doc comments).

### `Drop for SerialPort`

Implemented exactly as specified: `self.request_tx.take()` (drops the real
sender in place) → `self.worker.take()` then `.join()` (panic payload
swallowed via `let _ =`, per the ordinary Rust guidance against panicking
during unwind/drop) → `CloseHandle(self.handle.0)`.

### `lib.rs`

Added `#[cfg(target_os = "windows")] pub use windows::SerialPort;`
alongside the existing (already-gated, per Task 7) Linux `pub use
io_uring::SerialPort;` line, and updated the crate-root doc comment to
describe Task 8's completion instead of deferring to it.

### `session.rs` test-gating fix (the flagged Task 7 residual)

`#[cfg(test)] mod tests` → `#[cfg(all(test, target_os = "linux"))] mod
tests`, with an explanatory doc comment. Every async test in that module
uses `#[monoio::test(driver = "legacy")]`, and `monoio` is a Linux-only
*target-gated* Cargo dependency (`[target.'cfg(target_os = "linux")'.
dependencies] monoio`) — entirely absent from the dependency graph on a
Windows target, so the `monoio::test` attribute macro cannot resolve there
regardless of any code inside the module. `io_uring.rs` has no analogous
in-file gate to mirror (confirmed by re-reading it, as the task instructed):
that whole file is already `#[cfg(target_os = "linux")]`-gated at the
`lib.rs` module-declaration level, so its test module inherits the gate for
free without needing one of its own. `session.rs` is different because the
file itself (`SerialCatSession<T: Transport>`) is genuinely cross-platform
and compiles on both targets — only its test module, which happens to lean
entirely on `monoio`, needed the extra gate. The module's one plain
`#[test]` fn (`modem_control_lines_delegate_to_transport`, no `monoio`
dependency of its own) is swept into the same gate rather than special-cased
out, since it lives in the same `mod tests` block and gating the whole
block is simpler and no less correct than gating tests individually one by
one — the task's own phrasing explicitly allowed either shape ("on the test
module, or on each test").

### A real discrepancy against ADR 0004 §3's literal feature list, found
### and resolved the same way Task 7 resolved its `Win32_Security` finding

The first `cargo check --target x86_64-pc-windows-gnu -p cat-transport-serial
--all-targets` run against the actually-written `worker_read`/`worker_write`
failed: `error[E0432]: unresolved imports ... no ReadFile in
Win32::Storage::FileSystem` / `no WriteFile in Win32::Storage::FileSystem`.
Investigated the same way Task 7 investigated `CreateFileW`/`Win32_Security`
— grepped the actually-resolved `windows-sys` 0.59.0 source (now present in
the local registry cache at `~/.cargo/registry/src/index.crates.io-.../
windows-sys-0.59.0/src/Windows/Win32/Storage/FileSystem/mod.rs`) directly,
rather than guessing:

```
#[cfg(feature = "Win32_System_IO")]
windows_targets::link!("kernel32.dll" "system" fn ReadFile(hfile: HANDLE,
    lpbuffer: *mut u8, nnumberofbytestoread: u32, lpnumberofbytesread: *mut u32,
    lpoverlapped: *mut super::super::System::IO::OVERLAPPED) -> BOOL);
```

`ReadFile`'s and `WriteFile`'s own generated bindings are themselves
`#[cfg(feature = "Win32_System_IO")]`-gated — because their `lpOverlapped`
parameter's *type* (`OVERLAPPED`) is defined under that feature, exactly
the same shape as Task 7's `CreateFileW`/`Win32_Security` finding
(`SECURITY_ATTRIBUTES` gating `CreateFileW` even though the parameter is
always `null`). This function-existing-at-all requirement is unrelated to
*using* overlapped I/O — this code always passes `lpOverlapped =
null_mut()` and never sets `FILE_FLAG_OVERLAPPED` on the handle, so no
overlapped/IOCP-based I/O is actually performed or newly introduced.

**Judgment call, not a design change, but flagged prominently because it
brushes directly against ADR 0004 §3's literal text**: added
`Win32_System_IO` to `Cargo.toml`'s Windows feature list. ADR 0004 §3 says
"No `Win32_System_IO`/`OVERLAPPED` features" — read in context, that
sentence is explaining a *design* decision (don't reimplement IOCP-based
overlapped async I/O; confine everything to simple blocking calls on a
dedicated worker thread), not asserting that the `windows-sys` crate
feature literally named `Win32_System_IO` would never need to be enabled
for any reason. Confirmed empirically, the same way Task 7 confirmed its
own analogous finding: removed the feature again after implementation was
otherwise complete, re-ran `cargo check --target x86_64-pc-windows-gnu -p
cat-transport-serial --all-targets`, watched it fail with the exact same
`unresolved imports` error, added the feature back, watched it pass. This
is windows-sys's own binding structure — the same *kind* of mechanical
requirement as `Win32_Security`, not a reopening of the async-execution
decision (worker thread + hand-rolled completion primitive, ADR 0004 §1,
implemented exactly as specified above, with zero IOCP/overlapped-I/O
machinery anywhere in this code). Full reasoning also recorded inline as a
`Cargo.toml` comment.

### `WriteFile`/`ReadFile` pointer types

`windows-sys` 0.59.0 generates `lpBuffer: *mut u8` / `*const u8` for
`ReadFile`/`WriteFile` (not `*mut c_void`, unlike some other Win32
bindings) — `buf.as_mut_ptr()`/`data.as_ptr()` (already `*mut u8`/`*const
u8` from a `Vec<u8>`/`&[u8]`) pass directly with no cast needed; an initial
`.cast()` call was written defensively and then removed once the compiler
confirmed the types already matched exactly.

### Verification — same unusual bar as Task 7, both halves confirmed

`cargo check --target x86_64-pc-windows-gnu -p cat-transport-serial
--all-targets` — **compiles cleanly**: `Finished \`dev\` profile
[unoptimized + debuginfo] target(s) in 0.11s` (this run, after all fixes
above; the `--all-targets` flag now succeeds, clearing the exact residual
Task 7 flagged and did not clear). `cargo clippy --target
x86_64-pc-windows-gnu -p cat-transport-serial --all-targets -- -D warnings`
— **clean**, no downgrade to `--lib`-only needed: `Finished \`dev\` profile
[unoptimized + debuginfo] target(s) in 0.15s`.

Linux, re-measured directly before touching any file and again after every
change:
- `cargo test -p cat-transport-serial`: **24 passed** before and after —
  identical test names, zero behavior change (confirmed the dispatching
  prompt's cited baseline of 24 was accurate; this file's own Task 6
  section's "22" figure was stale, predating Task 7's `baud.rs`/
  `timeouts.rs` extraction, which added 2 more tests without this session's
  involvement).
- `cargo test --workspace`: **125 passed** before and after (13
  cat-client + 8 cat-framework + 46 cat-server + 12 cat-transport-core +
  24 cat-transport-serial + 7 cat-transport-tcp + 15 cat-transport-udp) —
  matches the dispatching prompt's cited baseline of 125 exactly, every
  other crate's count unchanged.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo fmt --all -- --check`: clean (one round of `cargo fmt --all`
  needed after the initial implementation — import-list line-wrapping in
  `windows.rs`'s new `windows_sys::Win32::Devices::Communication` import
  block only; re-verified clean afterward, and re-ran every other check
  above after formatting to confirm nothing regressed).

### What is, and is not, verified (explicit, per this task's standing
### instruction not to overclaim)

Verified: the Windows-specific code (`windows.rs`'s worker thread,
`Transport`/`ModemControlLines` impls, `Drop`) type-checks correctly against
the real `windows-sys` 0.59.0 FFI surface for `x86_64-pc-windows-gnu`,
including `--all-targets` (test-target code, not just the library). Linux
is completely unaffected — full workspace test suite, clippy, and fmt all
green, byte-for-byte the same test counts as before this task.

NOT verified, and not claimed to be: any actual runtime behavior of the
worker thread, `ReadFile`/`WriteFile`/`EscapeCommFunction`/
`GetCommModemStatus`/`PurgeComm`/`FlushFileBuffers` against a real Windows
serial port or COM device, the exact shutdown-ordering behavior under
`Drop` (no way to construct/drop a real `SerialPort` in this Linux
sandbox), or the `WriteFile` short-write-on-timeout interpretation. Per
ADR 0004's Consequences section and this crate's standing decision for
Windows-targeting work, the type-check bar above is the acceptance
criterion for this task — actual hardware validation is the user's own,
separate, later step against a physical FT-991A.

### Status: implementation complete, all acceptance checks green, awaiting
### architect/user review

Not committed, per standing rule. This completes ADR 0004's dispatch queue
(Tasks 6/7/8, all now landed) — no further Windows serial-backend work is
authorized or outstanding in this queue. STOPPING here per the
one-task-at-a-time workflow.
