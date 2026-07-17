# Progress — cat_server

## Task 5 — `cat-server` (2026-07-16)

### Created

- `cat-server/` — `Cargo.toml`, `src/lib.rs`, `src/local_channel.rs`,
  `src/broker.rs`, `src/registry.rs`, `src/tcp.rs`, `src/udp.rs`,
  `src/test_fixtures.rs` (test-only, shared fake `CommandTable` for
  `tcp`/`udp` test modules).
- Root `Cargo.toml` `[workspace] members` updated to add `"cat-server"`.

### Discrepancy check (writeup vs. source): none found

Read `planning/cat_transport/progress.md`'s Task 4a/4b framing writeups
alongside `cat-transport-tcp/src/session.rs` and
`cat-transport-udp/src/session.rs` directly. Both writeups match the actual
source exactly (length-prefix width/endianness/cap, envelope header
width/field order/endianness/session_id/request_id semantics, zero-length/
zero-payload conventions, dedup-cache scope note). No conflict to report.

### Dependency note: `cat-framework` added as a direct dependency (not in the

### architect's literal Task 5 list)

The architect's Task 5 dependency list names `cat-client`,
`cat-transport-core`, optionally a transport crate, `monoio`, `async-trait`,
`thiserror` — not `cat-framework` directly. In practice the broker's actual
malformed-request-rejection gate is `cat_framework::CommandTable::parse`
(the only thing that validates parameter *width* against `query_forms`/
`set_forms` — `CatClient::query`/`query_with_param`/`set` only check command
existence and the `readable`/`writable` flags, not width), and routing a
parsed request also needs `CommandOperation`/`ParseError`/`CommandId`
directly. Neither `cat-client` nor `cat-transport-core` re-exports these
(`cat-transport-core` re-exports only `ResponseDisposition`/
`ProtocolErrorKind`). Added `cat-framework` as a direct dependency, exactly
mirroring ADR 0001 Amendment 2's precedent for `cat-transport-core` — a
corrected/completed dependency list, not a deviation from the plan. Recorded
here per the "surface the conflict" instruction rather than silently adding
it.

### Design decision discovered through testing: routing by `readable`/

### `writable`, not blindly by `CommandTable::parse`'s operation label

`CommandTable::parse`'s `Query` branch is hardcoded to width 0 only (see
`cat-framework/src/cat.rs`'s `parse`) — a selector-parameterized read (e.g.
a TS-570D-style `SM0;` signal-meter query) can only pass structural
validation via the `Set` branch's width match, per
`CommandDefinition::readable`'s own doc comment ("some reads take a selector
parameter... that the query/set/action form model would otherwise classify
as a Set"). Discovered this the hard way: an initial `happy_path_query_with_
selector_param` test failed because `Broker::dispatch` blindly routed
`CommandOperation::Query` → `query_with_param` and `Set`/`Action` → `set`,
which sent a documented-read-only command (`writable: false`) into
`CatClient::set()`, which correctly rejected it (`CommandNotWritable`).
Fixed by deciding "read vs. write" from the command definition's `readable`/
`writable` flags whenever `parse` labels the operation `Set` (trusting
`writable` first, falling back to `readable`), not from the structural label
alone. See `broker.rs`'s `dispatch` doc comment and the
`happy_path_query_with_selector_param` test for the worked example. This
matters for a real physical session too, not just this crate's own
validation: sending a genuine write to `query_with_param` (read-and-await-
a-response) instead of `set()` (fire-and-forget) would be the wrong choice
against a real `SerialCatSession`.

### Architecture

- **`Broker<C: CommandId, S: CatSession>`** (generic over both — never
  names a concrete radio's command-id type or a concrete transport)
  owns one `CatClient<C, S>` + the same `&'static CommandTable<C>`.
  `dispatch(&mut self, request: &[u8])` is the single unit of work: UTF-8
  decode → `CommandTable::parse` (malformed-request gate, before
  `self.client` is touched) → route to `query_with_param`/`set` per the
  `readable`/`writable`-driven rule above → wrapped in
  `monoio::time::timeout(request_timeout, ..)` (default 5s, configurable via
  `Broker::with_timeout`) — load-bearing since `TcpCatSession` has **no**
  timeout of its own by design, deferring liveness enforcement explicitly to
  this layer per its own docs.
- **Single ordered worker**: `src/local_channel.rs` is a hand-rolled,
  `Rc`/`RefCell`-based, `!Send`, std-only (no channel crate on this crate's
  dependency list) many-producer/single-consumer queue plus a oneshot reply
  primitive. `BrokerWorker::run()` is the one task that ever owns a `Broker`
  — it pulls `Job`s off the queue strictly in receive order and calls
  `dispatch` once per `Job`. `BrokerHandle` (cheap `Rc`-based `Clone`) is
  what every TCP connection task / UDP datagram task holds to `submit()`
  work and `.await` its own reply. No other code path ever touches the
  physical session — verified structurally (only `BrokerWorker::run` calls
  `Broker::dispatch`) and by a concurrency test
  (`worker_serializes_concurrent_requests_from_multiple_clients_correctly`)
  where a `ScriptedCatSession` with a fixed exchange order would panic on
  out-of-order access.
- **Correlation**: each `Job` carries a monotonically-assigned
  `request_id: u64` and a `ClientId` for bookkeeping/observability, but
  actual reply delivery is structural — each `submit()` call owns its own
  single-use oneshot reply slot, so misrouting under interleaved concurrent
  clients isn't representable, not just avoided by discipline.
- **Timeout**: proven with a hand-rolled `NeverRespondingSession` (`execute`
  awaits `std::future::pending::<()>()`) — `never_answered_request_times_
  out_instead_of_hanging` and `worker_recovers_after_a_timeout_and_services_
  the_next_request` show the timeout fires within the configured bound and
  the broker remains usable afterward. `ScriptedCatSession::simulate_
  timeout()` was **not** sufficient for this (it returns an immediate `Err`,
  not a hang) — it exercises the "physical session/transport error" path
  instead (`physical_session_transport_error_surfaces_as_session_error`),
  a genuinely different case, also tested.
- **Disconnect**: `disconnect_before_reply_does_not_wedge_the_worker` pushes
  a `Job` directly and drops its `OneshotReceiver` before the worker replies
  (no `.recv()` ever called) — proves `OneshotSender::send`'s failure path
  (receiver gone) doesn't panic/block the worker, and that the next queued
  client is still served correctly. `end_to_end_disconnect_mid_stream_does_
  not_wedge_the_server` (tcp.rs) does the analogous thing over a real
  socket (drop a connected `TcpStream` before sending anything, then prove a
  fresh connection still works).
- **Malformed rejection**: `CommandTable::parse` (or a non-UTF-8 check
  first) short-circuits before `self.client` is touched — six broker-level
  tests plus TCP/UDP end-to-end equivalents assert both the specific
  `DispatchError`/`ParseError` variant AND (via
  `malformed_request_leaves_script_untouched_for_next_valid_request` and its
  tcp/udp analogs) that the `ScriptedCatSession`'s script position is
  provably untouched afterward.
- **Wire-level error signaling** (judgment call, see `task_plan.md`): the
  frozen TCP/UDP frame formats only distinguish empty-vs-non-empty payload —
  there's no wire bit for "this was a protocol error," and no `CatRadio`
  downstream to invent a radio-specific one. `outcome_to_wire` renders every
  rejection as a non-empty `b"ERR <message>"` payload (no leading two-letter
  code, no trailing `;`, so it can never collide with a real CAT frame)
  rather than silently answering with an empty frame that would hide the
  rejection.
- **Client session management**: `registry::ClientRegistry` assigns/tracks
  a `ClientId` per accepted TCP connection or first-seen UDP peer address —
  bookkeeping only, never visible to `Broker`/`CatClient`. UDP peer→id
  mapping has no expiry (a peer that stops sending stays "registered"
  forever) — acceptable for this task's scope, flagged below as a possible
  Task 6 item.

### Server-side framing: direct against raw `monoio` sockets (chosen over

### depending on `cat-transport-tcp`/`-udp`)

`TcpCatSession`/`UdpCatSession` are requester-shaped (`execute()` writes a
request then reads a response); reusing them on the accept/answer side would
need an unnatural role inversion. `src/tcp.rs`/`src/udp.rs` re-implement the
documented codecs directly against `monoio::net::{TcpListener, TcpStream}` /
`monoio::net::udp::UdpSocket`, matching the frozen wire formats exactly
(`MAX_FRAME_SIZE`/`ENVELOPE_HEADER_LEN`/`MAX_PAYLOAD_SIZE` constants
duplicated with the same values, not imported, since there's no dependency
on those crates). `cargo tree -p cat-server` confirms neither
`cat-transport-tcp` nor `cat-transport-udp` appears anywhere in the tree.

Each test module's hand-rolled raw frame/envelope encoder-decoder is
deliberately independent of the production `read_request_frame`/
`write_response_frame` (tcp) and `encode_envelope`/`decode_envelope` (udp)
functions, mirroring `cat-transport-tcp`/`-udp`'s own test-module
convention — cross-checks the *documented* format against the
implementation rather than only the encoder against itself.

**UDP dedup cache** (server-side, answerer-side — a different, load-bearing
mechanism from `UdpCatSession`'s own client-side membership-only cache, per
that writeup's explicit note): keyed by `(peer_addr, session_id,
request_id)` — one field beyond what the writeup requires (`(session_id,
request_id)` or `(peer_addr, request_id)` alone were both explicitly
allowed). Chosen because `peer_addr` alone can collide across a client
restart (a fresh `UdpCatSession` resets its `request_id` counter to 1 but
may keep the same source port) and `session_id` alone relies purely on
randomization; combining both with `request_id` costs one extra tuple field
and closes both edge cases at once. Caches the actual response bytes (not
just membership), so a duplicate request is answered without re-executing
against the physical radio — proven by
`duplicate_request_gets_cached_response_without_re_executing`, which scripts
only ONE exchange and would panic (`ScriptedCatSession` panics on an
exhausted script) if the duplicate were re-executed.

### Tests (all four required categories, not just happy path)

`cargo test -p cat-server`: **46 passed, 0 failed.**

- **Happy path**: `happy_path_query_returns_response_text`,
  `happy_path_query_with_selector_param`, `happy_path_set_yields_no_
  response`, `happy_path_action_yields_no_response` (broker unit level);
  `end_to_end_query_round_trip_over_real_loopback_tcp`, `end_to_end_set_
  gets_explicit_empty_response_frame`, `end_to_end_two_concurrent_
  connections_get_correctly_correlated_responses` (tcp, real sockets);
  `end_to_end_query_round_trip_over_real_loopback_udp`, `end_to_end_set_
  gets_explicit_empty_response_envelope` (udp, real sockets).
- **Timeout**: `never_answered_request_times_out_instead_of_hanging`,
  `worker_recovers_after_a_timeout_and_services_the_next_request` (using a
  hand-rolled never-resolving `CatSession`, since `ScriptedCatSession` can't
  simulate a true hang).
- **Disconnect**: `disconnect_before_reply_does_not_wedge_the_worker`
  (broker/channel level); `end_to_end_disconnect_mid_stream_does_not_
  wedge_the_server` (tcp, real socket); `worker_shuts_down_once_every_
  handle_is_dropped` (clean shutdown, not a hang, once every `BrokerHandle`
  is gone).
- **Malformed request**: `malformed_unknown_command_never_touches_session`,
  `malformed_missing_terminator_never_touches_session`, `malformed_wrong_
  param_width_never_touches_session`, `malformed_non_utf8_never_touches_
  session`, `malformed_request_leaves_script_untouched_for_next_valid_
  request`, `worker_malformed_request_gets_error_wire_response` (broker);
  `end_to_end_malformed_request_gets_error_frame_not_forwarded_to_radio`,
  `end_to_end_oversized_frame_gets_error_response_then_connection_closes`
  (tcp); `end_to_end_malformed_request_gets_error_envelope_not_forwarded_
  to_radio`, `malformed_short_datagram_is_ignored_as_noise` (udp).
- Plus: `local_channel`'s own primitive-level tests (send/recv ordering,
  closed sender/receiver, oneshot semantics), `registry`'s bookkeeping
  tests, `outcome_to_wire`'s wire-convention tests, and the UDP dedup
  cache's pure-logic tests (including the peer-addr/session-id collision
  case the 3-field key exists to close).

### Acceptance checks (all green)

```
$ cargo test -p cat-server
running 46 tests
... (all 46 ok, see full list above by category)
test result: ok. 46 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
Doc-tests: 0/0 (no doc examples written)

$ cargo test --workspace
115 tests total across all crates, all passing
(cat-framework 13, cat-client 12 [renumbered since Task 3's report — see
note below], cat-server 46, cat-transport-core 14, cat-transport-serial
7 [io_uring PTY tests skipped/counted per that crate's own environment],
cat-transport-tcp 7, cat-transport-udp 15)

$ cargo clippy -p cat-server --all-targets
Finished, 0 warnings, 0 errors

$ cargo clippy --workspace --all-targets
0 errors; 1 pre-existing warning (cat-transport-serial's write_to_master
dead-code, documented in planning/cat_transport/progress.md's Task 2
section -- not introduced here)

$ cargo fmt --all -- --check
clean (no diff) -- one auto-fmt pass was applied and re-verified clean
```

`cargo tree -p cat-server`: local deps = `cat-client`, `cat-framework`,
`cat-transport-core` only (all direct — `cat-framework` per the note above).
Neither `cat-transport-tcp` nor `cat-transport-udp` nor any radio crate
appears anywhere in the tree. `grep -rl cat-server **/Cargo.toml` shows only
the root workspace `members` list and `cat-server/Cargo.toml` itself
reference `cat-server` — no other crate depends on it, satisfying the
one-way dependency rule.

### Judgment calls / discrepancies flagged (summary, cross-referenced above)

1. `cat-framework` added as a direct dependency, not literally listed by the
   architect — same pattern as ADR 0001 Amendment 2, justified above.
2. Read-vs-write routing decided by `readable`/`writable` flags rather than
   blindly by `CommandTable::parse`'s structural operation label, for
   selector-parameterized reads — discovered via a failing test, not
   assumed up front; documented in `broker.rs` and above.
3. Wire-level error signaling (`b"ERR <message>"` convention) is this
   crate's own invention, since the frozen frame formats have no error bit
   and there's no `CatRadio` to supply a radio-specific one.
4. UDP dedup cache keyed by `(peer_addr, session_id, request_id)` — one
   field beyond the writeup's stated minimum, justified above (closes a
   client-restart collision edge case).
5. Direct `monoio` socket implementation for both listeners instead of
   depending on `cat-transport-tcp`/`-udp`, per the task's own "your call"
   framing — justified in `task_plan.md` and above.

### Follow-on items (flagged as a hypothetical "Task 6," not undertaken —

### last task in the current dispatch queue)

- **UDP peer→`ClientId` registry entries never expire.** A UDP "client" is
  discovered on first datagram and never explicitly unregistered (UDP has no
  connection-close signal to hook, unlike TCP). For a long-lived server this
  is an unbounded (though slow-growing, one entry per distinct peer address
  ever seen) memory growth. An idle-timeout/LRU eviction policy for
  `registry`'s UDP-side bookkeeping would be a reasonable Task 6.
  (`DedupCache`'s own bound is unaffected — that's already capacity-limited
  independent of the registry.)
  registry.
- **No configurable cap on the broker's job queue depth.** `local_channel`'s
  queue is unbounded — a burst of concurrent client requests all queue up in
  memory rather than applying backpressure. Not a problem for the scope of
  this task (no load/stress requirement was specified), but worth a
  bounded-queue-with-backpressure follow-on if `cat-server` is ever exposed
  to untrusted/high-volume clients.
- **No authentication/authorization layer.** Per this crate's charter,
  explicitly out of scope for Task 5 (`ClientRegistry` is bookkeeping only)
  — noting it only because a real deployment exposing a physical radio over
  a network would need one before going further than a private/trusted
  network.
- **UDP oversized-datagram truncation has no explicit signal**, mirroring
  `UdpCatSession`'s own documented limitation on the client side: a
  request payload larger than `MAX_PAYLOAD_SIZE` sent by a misbehaving/
  buggy client is silently truncated by the kernel to this listener's
  receive buffer size, then very likely fails envelope decoding and is
  discarded as noise rather than being cleanly rejected the way the TCP
  listener can reject an oversized frame before reading it. Inherent to
  UDP, not something this task introduces or can close without a length
  field the wire format deliberately omits.

### Status: DONE, stopping here

Task 5 is the last task in the current dispatch queue per
`planning/architect/task_plan.md`. Nothing committed, per this session's
standing rule. `ts570d`/`ft991a` untouched; every other crate in this
workspace untouched (read-only reference) except the root `Cargo.toml`'s
`[workspace] members` list.

## Task 6 — de-duplicate `tcp.rs`/`udp.rs` against newly-`pub`
## `cat-transport-tcp`/`cat-transport-udp` codec primitives (2026-07-17)

The `cat_transport` agent made its client-side codec building blocks `pub`
(`planning/cat_transport/progress.md`'s "Task 6" section) specifically so
this crate could stop maintaining a byte-for-byte duplicate. Read that
writeup plus `cat-transport-tcp/src/{lib,session}.rs` and
`cat-transport-udp/src/{lib,session}.rs` directly before touching anything.
Both crates are `monoio`-based, matching `cat-server`'s own runtime exactly
— no runtime mismatch to reconcile.

### `tcp.rs`

Removed the private `read_request_frame`/`write_response_frame` functions
and the redeclared `pub const MAX_FRAME_SIZE`. Added
`cat-transport-tcp = { path = "../cat-transport-tcp" }` to `Cargo.toml` and
imports `cat_transport_tcp::{read_frame_or_eof, write_frame}` (production
code) / `cat_transport_tcp::MAX_FRAME_SIZE` plus
`monoio::io::{AsyncReadRentExt, AsyncWriteRentExt}` (test-module-only now,
since production code no longer calls stream I/O methods directly — moved
into `mod tests`'s own `use` block to avoid an unused-import warning).

`handle_connection`'s match arms carried over unchanged in shape:
`read_frame_or_eof` returns `Result<Option<Vec<u8>>, TcpSessionError>`,
structurally identical to the old `io::Result<Option<Vec<u8>>>` — only the
error type name changed, so `Ok(Some(payload))` / `Ok(None)` / `Err(e)`
mapped over directly. `write_response_frame` calls became `write_frame`
calls (`write_frame(&mut stream, &response).await.is_err()` for the
happy-path reply, `write_frame(&mut stream, format!("ERR {e}").as_bytes())`
for the read-error path) — no behavior change: both old and new write paths
treat any write failure as "client gone, close the connection."

One real (harmless) wire-text difference: the old code formatted the raw
`io::Error` directly into `ERR {e}`; now `TcpSessionError`'s own
`thiserror` `Display` is used. For the oversized-frame case the text is
actually byte-identical (`cat_transport_tcp`'s `FrameTooLarge` message is
worded the same as the old hand-rolled one); for a mid-frame disconnect the
wrapped `Io(#[error("I/O error: {0}")])` variant prepends `"I/O error: "` to
the underlying `io::Error`'s text where the old code had none. No test
asserts exact `ERR` message text (only `starts_with(b"ERR ")`), so this is
not a behavioral regression against any existing assertion — flagging it
here per the task's "discrepancy" instruction rather than silently deciding
it doesn't matter.

### `udp.rs`

Removed the private `encode_envelope`/`decode_envelope` functions and the
redeclared `ENVELOPE_HEADER_LEN`/`MAX_PAYLOAD_SIZE` constants. Also removed
`SESSION_ID_LEN`/`REQUEST_ID_LEN` (not explicitly named by the task, but
they existed only to compute the now-imported `ENVELOPE_HEADER_LEN` and to
slice header bytes inside the now-deleted `decode_envelope` — dead once
those were gone; `cat_transport_udp` does not re-export these two at its
crate root, only `session.rs`-internal `pub`, so they were not available to
import even if kept). Added
`cat-transport-udp = { path = "../cat-transport-udp" }` to `Cargo.toml` and
imports `cat_transport_udp::{decode_envelope, encode_envelope,
ENVELOPE_HEADER_LEN, MAX_PAYLOAD_SIZE}`.

**`encode_envelope`'s `Result` return** (rejects a payload wider than
`MAX_PAYLOAD_SIZE` = 1024 bytes): the production call site is
`send_envelope`, which encodes this listener's own dispatch output (a real
CAT response, or this crate's `b"ERR <message>"` convention) — never
expected to approach 1024 bytes, but "expected" isn't "guaranteed" (e.g. a
pathological error message echoing back a large malformed input). Rejected
both `.unwrap()` (would let a client-triggerable input panic a production
listener) and silent truncation (a truncated CAT response could still
decode as a different, wrong answer — worse than an explicit error).
Chosen: on `Err`, fall back to encoding a short, fixed
`b"ERR response too large to send"` payload instead of the original —
`encode_envelope` cannot fail for a payload that small, so the fallback's
own `.expect(...)` is not a live panic risk (it would only fire if
`MAX_PAYLOAD_SIZE` were ever configured absurdly small, which it is not).
Documented inline on `send_envelope`.

Test call sites adapted for the `Result` return (`.unwrap()`/`.expect(...)`
added at `send_request` and `encode_decode_round_trip` in the `#[cfg(test)]`
module) — no test assertions changed, only the extra `Result` unwrap
needed to keep compiling.

Note: unlike `tcp.rs`'s test module (which hand-rolls independent
`write_raw_frame`/`read_raw_frame`, deliberately not calling production
code), `udp.rs`'s test module already called the module-level
`encode_envelope`/`decode_envelope` directly for its own fixtures (`
send_request`/`recv_response`, `encode_decode_round_trip`) rather than a
separate hand-rolled duplicate — so switching those to the imported
`cat_transport_udp` functions is exactly the same call shape as before,
just via `use` instead of a local `fn`.

### Verification (all green)

```
$ cargo build -p cat-server        # clean, 0 warnings after import cleanup
$ cargo test -p cat-server         # 46 passed; 0 failed — same 46 as Task 5, unchanged
$ cargo clippy -p cat-server --all-targets -- -D warnings   # clean
$ cargo fmt --check                # clean
$ cargo clippy --workspace --all-targets   # 0 errors; 1 pre-existing warning
    (cat-transport-serial's write_to_master dead-code, same one noted in
    Task 5's report — not introduced here)
$ cargo test --workspace           # all green across every crate
$ cargo tree -p cat-server
```
`cargo tree -p cat-server` now shows `cat-transport-tcp` and
`cat-transport-udp` as direct local dependencies (alongside the
pre-existing `cat-client`, `cat-framework`, `cat-transport-core`) — the
required proof this task actually wired up the new dependency edges.

### Discrepancies found: none blocking

Only the cosmetic `ERR` message-text difference noted above under `tcp.rs`
(no test depends on the exact text, so nothing needed to change to keep
existing tests passing). No genuine behavioral conflict between the old
hand-rolled codec and `cat-transport-tcp`/`cat-transport-udp`'s versions
was found — the wire bytes produced/accepted are identical in every case
exercised by this crate's test suite.

### Status: DONE, stopping here

Nothing committed, per this session's standing rule. `ts570d`/`ft991a`/
`cat-transport-tcp`/`cat-transport-udp` untouched (read-only reference, as
instructed) aside from being added as dependencies in `cat-server`'s own
`Cargo.toml`.
