# Task Plan — cat_server

## Task 5 (architect `planning/architect/task_plan.md`): implement `cat-server`

Authorized. This is the first code in this crate. Ground truth read in full
before writing any code: ADR 0001 (+ amendments), ADR 0002,
`planning/architect/task_plan.md` Task 5, `cat-client/src/client.rs`,
`cat-transport-core/src/{session,test_support}.rs`,
`planning/cat_transport/progress.md` (Task 4a/4b framing writeups),
`cat-transport-tcp/src/session.rs`, `cat-transport-udp/src/session.rs`.

**Discrepancy check (writeup vs. source): none found.** Both `progress.md`'s
TCP frame writeup and UDP envelope writeup match the actual `session.rs`
source exactly (length prefix width/endianness/cap, envelope header
width/field order/endianness, zero-length/zero-payload conventions, dedup
cache scope). Proceeding from the writeups as instructed.

## Key design decision: what "raw wire bytes" the broker exchanges with a client

TCP/UDP framing (Task 4a/4b) carries **raw CAT command/response bytes**
verbatim (e.g. `b"FA;"`, `b"FA00014250000;"`) — the same shape
`CatSession::execute` exchanges with a physical radio. So a remote client of
`cat-server` is, conceptually, another `CatClient<C, TcpCatSession>` (or
`UdpCatSession`) pointed at the broker's socket instead of a radio's serial
port. This means the broker's job per request is: take one raw wire frame in,
parse+validate it against the same `&'static CommandTable<C>` the physical
`CatClient<C,S>` was built with (**this is the actual malformed-request
gate** — `CommandTable::parse` performs the structural width/form validation
`CatClient::query`/`set` do NOT do on their own, since those only check
existence + readable/writable, not parameter width against `query_forms`/
`set_forms`), then, if valid, call the appropriate `CatClient` method
(`query_with_param` for `CommandOperation::Query` uniformly — its formatting
is byte-identical to `query()` when params are empty, so no separate code
path is needed; `set` for `Set`/`Action`, since `Action` writes like `TX;`
are documented as going through `is_writable()`), and turn the result back
into raw wire bytes.

## Wire-level error signaling (judgment call, flagged explicitly)

The frozen TCP/UDP frame formats only distinguish two dispositions on the
wire: non-empty payload ("a response") and empty payload ("`NoResponse`").
There is no wire-level "this was a protocol error" bit — that is inherent to
Task 4a/4b's design (a real `TcpCatSession`/`UdpCatSession` peer never
learns disposition beyond empty/non-empty). Since there is no `CatRadio`
downstream to supply a radio-specific error wire convention (per this
crate's charter — no radio state machine exists in this repo), `cat-server`
invents one minimal, explicitly-documented generic convention for **its own
boundary rejections only** (malformed request, broker-level timeout,
physical-session transport error): a non-empty payload of the form
`b"ERR <Display of the error>"` — no leading 2-letter code, no trailing
`;`, so it can never collide with, or be confused for, a real CAT frame
(which is always `<2 letters><data>;`). This is analogous to what a real
`CatRadio::write_protocol_error` would produce, kept intentionally generic
since this crate must never invent radio-specific error text.

## Architecture

- **`Broker<C: CommandId, S: CatSession>`** — owns one `CatClient<C, S>` +
  the same `&'static CommandTable<C>`. `dispatch(&mut self, request: &[u8])
  -> Result<DispatchOutcome, DispatchError<S::Error>>` is the single unit of
  work: parse/validate → route to `query_with_param`/`set` → wrapped in
  `monoio::time::timeout(request_timeout, ..)` (the physical session itself,
  e.g. `TcpCatSession`, may have **no** timeout of its own by design per
  `cat-transport-tcp`'s docs — deferred explicitly to this layer, so this
  wrap is load-bearing, not optional).
- **Single ordered worker**: a hand-rolled, `Rc`/`RefCell`-based, `!Send`
  many-producer/single-consumer local channel (`src/local_channel.rs`,
  std-only — no channel crate is on this crate's dependency list) feeds
  `Job { payload, reply: oneshot }` into one `BrokerWorker` loop that owns
  the `Broker` exclusively and calls `dispatch` once per `Job`, in receive
  order. `BrokerHandle` (cloneable, `Rc`-based) is what accept-loop/
  per-connection tasks hold to `submit()` work and `.await` their own
  reply — no other task ever touches the physical session. This directly
  satisfies "no interior concurrency introduced around the physical
  session."
- **Correlation**: every `Job` carries a monotonically-assigned
  `request_id: u64` (for client-session bookkeeping/tests) but the actual
  reply routing is structural (each `submit()` owns its own oneshot reply
  slot) — stronger than a lookup-table scheme since misrouting is not
  representable, not just avoided by discipline.
- **Timeout**: see `dispatch` above. On timeout the in-flight `CatClient`
  call future is dropped (cancelled) and the worker loop proceeds
  immediately to the next `Job` — verified by a test using a hand-rolled
  never-resolving `CatSession` double.
- **Disconnect**: a client task drops its oneshot receiver (or the whole
  task) without waiting for a reply. `submit()`'s send-into-maybe-dropped
  reply is a non-panicking, non-blocking attempt from the worker's side —
  the worker does not know or care that the receiver is gone, and moves on
  to the next `Job` immediately. Verified by a unit test that drops the
  receiver before the worker replies, then confirms the worker still
  services the next `Job` correctly.
- **Malformed rejection**: `CommandTable::parse` failure (or non-UTF-8
  input) short-circuits `dispatch` before `self.client` (and therefore the
  physical session) is touched at all — verified by asserting the
  `ScriptedCatSession`'s `written()`/script position is untouched after a
  malformed request.
- **Client session management**: `src/registry.rs`'s `ClientRegistry`
  assigns/tracks a `ClientId` per accepted TCP connection or discovered UDP
  peer address — bookkeeping only (no auth, no behavior gating), never
  visible to `Broker`/`CatClient`.

## Server-side framing: direct against raw `monoio` sockets, not via
## `cat-transport-tcp`/`-udp`

Chosen over depending on those crates because `TcpCatSession`/`UdpCatSession`
are **requester**-shaped (their `execute()` writes a request then reads a
response) — reusing them on the accept/answer side would require an
unnatural role inversion. Instead `src/tcp.rs`/`src/udp.rs` re-implement the
documented codecs directly (length-prefixed frame read/write for TCP;
16-byte BE `session_id`/`request_id` envelope for UDP) against
`monoio::net::{TcpListener, TcpStream}` / `monoio::net::udp::UdpSocket`,
matching the frozen wire formats byte-for-bit. This also keeps `cat-server`'s
Cargo dependency graph to `cat-client`, `cat-transport-core` (for
`CatSession`/`ScriptedCatSession`, test-only), `async-trait`, `thiserror`,
`monoio` (Linux-gated) — no dependency on `cat-transport-tcp`/`-udp` at all.

UDP dedup: per `progress.md`'s explicit note, this is a **different, load-
bearing** cache from `UdpCatSession`'s own (client-side, membership-only,
keyed by `request_id` alone). `cat-server`'s cache is keyed by
`(peer_addr, request_id)` and caches the **actual response bytes**, so a
duplicate incoming request from the same peer gets the cached answer resent
without re-executing against the physical radio.

## Tests planned (happy / timeout / disconnect / malformed, per charter)

- `local_channel`: send/recv basic, closed sender/receiver, oneshot dropped
  receiver does not panic sender.
- `broker`: happy-path query, happy-path set/action (NoResponse), malformed
  (unknown command, missing terminator, wrong param width, non-UTF-8) with
  physical session proven untouched, broker-level timeout with a
  never-resolving session double + proof the worker recovers for the next
  job, disconnect (dropped reply receiver) + proof the worker recovers,
  concurrent interleaved requests from multiple simulated clients correlate
  correctly (no cross-wiring).
- `tcp`: at least one true end-to-end test over a real loopback
  `TcpListener`/`TcpStream`, hand-rolled raw peer (independent encoder, not
  calling production code, mirroring `cat-transport-tcp`'s own test-module
  convention) exercising query + malformed + a slow/never-answered request
  timing out without wedging a second, concurrent client's request.
- `udp`: nice-to-have per the task; included if time allows after TCP is
  solid — will flag explicitly in `progress.md` if cut.

## Workflow

Single task (Task 5), no further tasks after this without architect/user
review. `cargo test -p cat-server`, `cargo clippy -p cat-server
--all-targets`, `cargo fmt --all -- --check` must be green before reporting
done. Root `Cargo.toml` `[workspace] members` gets `cat-server` added.
Nothing committed. `ts570d`/`ft991a`/all other crates in this workspace
untouched (read-only reference).

## Task 6 (2026-07-17): de-duplicate codec against newly-`pub`
## `cat-transport-tcp`/`cat-transport-udp` primitives

The `cat_transport` agent made its client-side codec building blocks `pub`
specifically so this crate can stop hand-rolling a second copy:
`cat_transport_tcp::{read_frame_or_eof, write_frame, MAX_FRAME_SIZE}` and
`cat_transport_udp::{encode_envelope, decode_envelope, ENVELOPE_HEADER_LEN,
MAX_PAYLOAD_SIZE}`. Both crates are `monoio`-based (`monoio::net::TcpStream`
/ `monoio::net::udp::UdpSocket`), matching `cat-server`'s own runtime exactly
— no new async-runtime/type mismatch to reconcile, unlike the concern that
motivated Task 5's original "re-implement instead of depend on" decision
(role-inversion: `TcpCatSession`/`UdpCatSession` are requester-shaped, but
`read_frame_or_eof`/`write_frame`/`encode_envelope`/`decode_envelope` are
free functions with no session/role baked in, so no inversion issue applies
to importing just these primitives).

Plan: `tcp.rs` — delete local `read_request_frame`/`write_response_frame`/
`pub const MAX_FRAME_SIZE`; import the three from `cat_transport_tcp`; adapt
`handle_connection`'s match on `read_frame_or_eof`'s `Result<Option<Vec<u8>>,
TcpSessionError>` (structurally identical to the old
`io::Result<Option<Vec<u8>>>`, only the error type name changes — the
`Ok(Some)`/`Ok(None)`/`Err` three-way split carries over unchanged). Add
`cat-transport-tcp` as a direct dependency.

`udp.rs` — delete local `encode_envelope`/`decode_envelope`/
`ENVELOPE_HEADER_LEN`/`MAX_PAYLOAD_SIZE` (and the now-unused local
`SESSION_ID_LEN`/`REQUEST_ID_LEN`, which existed only to compute
`ENVELOPE_HEADER_LEN` and are not part of `cat_transport_udp`'s public
surface); import the four from `cat_transport_udp`. `encode_envelope` now
returns `Result<Vec<u8>, UdpSessionError>` — `send_envelope` must handle
`Err(PayloadTooLarge)` without unwrapping. Add `cat-transport-udp` as a
direct dependency.

Test modules' independent hand-rolled raw encoders/decoders (deliberately
not calling production code, to cross-check the documented format) are
untouched — they were never the thing being de-duplicated.

No behavior change beyond what the type-level adaptation forces. Single
task, report back before anything further.
