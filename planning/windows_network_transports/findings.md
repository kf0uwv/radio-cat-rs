# Findings: Windows network transports (Deliverable 1)

## Read in full before designing
- CLAUDE.md, docs/adr/0001-0005, docs/adr/README.md, .claude/agents/cat_server.md
- cat-transport-core/src/{lib,transport,session,modem,errors}.rs
- cat-transport-tcp/src/{lib,session}.rs (TcpCatSession wraps `monoio::net::TcpStream`
  directly, NOT generic over `Transport` -- length-prefixed framing, MAX_FRAME_SIZE
  64KiB, `write_frame`/`read_frame`/`read_frame_or_eof` are `pub` for cat-server reuse)
- cat-transport-udp/src/{lib,session}.rs (UdpCatSession wraps `monoio::net::udp::UdpSocket`
  directly -- envelope format: 8B session_id + 8B request_id BE + payload, no length
  field, MAX_PAYLOAD_SIZE 1024, dedup cache (client-side, membership only),
  response_timeout via `monoio::time::timeout_at`. `encode_envelope`/`decode_envelope`
  are ALREADY pure functions with no monoio dependency.)
- cat-server/src/{lib,broker,broker_session,local_channel,registry,tcp,udp,test_fixtures}.rs
  - `Broker<C,S>::dispatch` is the ONLY place in broker.rs/broker_session.rs/
    local_channel.rs/registry.rs that touches monoio (`monoio::time::timeout` wrapping
    the request). Everything else in those four files is pure `std` (Rc/RefCell/
    poll_fn), not monoio-dependent at all -- confirmed by grep.
  - `tcp.rs`/`udp.rs` (server-side accept/dispatch loops) are the only other
    monoio-dependent files: `monoio::net::{TcpListener,TcpStream}` /
    `monoio::net::udp::UdpSocket` + `monoio::spawn` per connection/datagram.
  - `ClientRegistry` (registry.rs) and the per-file `DedupCache` (udp.rs) are both
    already pure `std`, reusable unchanged by a Windows implementation once wrapped
    in `Arc<Mutex<_>>` instead of `Rc<RefCell<_>>`.
- cat-transport-serial/src/{oneshot,windows,session,lib,config}.rs (ADR 0004's
  precedent: dedicated background `std::thread` + `oneshot.rs`'s hand-rolled
  single-slot completion primitive; `SerialConfig`/`Parity`/`FlowControl` extracted
  to ungated `config.rs` as the "shared pure data, platform-gated code" pattern to
  replicate for tcp/udp's codec logic).

## Key design decisions (see docs/adr/0006-windows-network-transport.md for the
full record)

1. `oneshot.rs` moves to `cat-transport-core/src/completion.rs` as `pub mod
   completion` (renamed from the crate-private `oneshot` name since it's now
   shared public infrastructure). `cat-transport-serial` imports it via
   `use cat_transport_core::completion as oneshot;` -- zero behavior change,
   existing tests untouched.
2. `cat-transport-tcp`/`cat-transport-udp` each gain: an ungated `codec.rs` with
   the pure encode/decode/error/const logic (extracted from `session.rs`, mirroring
   ADR 0004 Sec 2's `config.rs` precedent exactly); `session.rs` becomes
   `#[cfg(target_os = "linux")]`-gated (monoio-based, otherwise unchanged); a new
   `windows.rs` implements the same public session type (`TcpCatSession`/
   `UdpCatSession`) via a dedicated background `std::thread` doing blocking
   `std::net` I/O + `cat_transport_core::completion` for the async boundary --
   same shape as ADR 0004's serial backend, one dedicated worker thread PER
   SESSION (not per byte op) since both session types already do a whole
   request/response exchange as one unit (no generic `Transport` layer to drive
   separately).
3. `cat-server` is structurally different (charter: single ordered worker serving
   MANY concurrent remote clients), so it does NOT get a byte-for-byte copy of the
   serial pattern. Decision: keep `Broker<C,S>` (dispatch/timeout-wrap logic),
   `DispatchOutcome`, `DispatchError`, `outcome_to_wire`, `ClientRegistry`, and the
   (now-extracted) `DedupCache` **fully shared, ungated** -- only the *channel*
   used to submit `Job`s to the worker, and the *listener concurrency substrate*,
   are platform-gated:
   - Linux: unchanged (`local_channel`, `monoio::spawn` per connection/datagram,
     `Rc<RefCell<_>>` shared state, `monoio::time::timeout` inside `dispatch`).
   - Windows: `std::sync::mpsc` (already `Send`) for the Job queue,
     `cat_transport_core::completion` for the reply slot, one dedicated OS thread
     per accepted TCP connection / per received UDP datagram (mirrors "OS threads
     instead of cooperative tasks" -- the natural Windows analog of monoio's
     thread-per-core cooperative model, since ADR 0002 forbids introducing a
     second general-purpose async executor), `Arc<Mutex<_>>` shared state, and a
     hand-rolled `timeout()` combinator (`cat-server/src/timeout.rs`, pure `std`,
     tested on Linux despite its only production caller being Windows -- same
     "give it real tests anyway" precedent ADR 0004 set for `oneshot.rs`) plus a
     minimal thread-parking `block_on` (`cat-server/src/block_on.rs`, same
     precedent) to drive the worker loop and each connection-handling thread.
   - `BrokerHandle::submit` stays `pub async fn` with an IDENTICAL signature on
     both platforms (the one truly shared hot-path call site); the *bootstrapping*
     of the worker/listener (spawn a monoio task vs. spawn an OS thread) is the
     one place this repo's own ADR 0004 already conceded platforms must differ
     (`ft991a`/`ts570d` need their own Windows entry point regardless).
   - Module paths kept IDENTICAL across platforms (`cat_server::tcp::serve`,
     `cat_server::udp::serve`, `cat_server::{Job, BrokerWorker, BrokerHandle,
     build, build_with_timeout}`) via `#[path = "..."]`-aliased `mod` declarations
     in `lib.rs`, selecting a different source file per `cfg(target_os)` under one
     logical module name -- an extension of ADR 0004's "same public name,
     platform-gated internals" convention to the module-path level.
4. No new external dependency needed for any of this -- `std::net`, `std::thread`,
   `std::sync::{mpsc, Arc, Mutex}`, and the moved `completion` primitive are
   sufficient. No `windows-sys` needed (TCP/UDP sockets are natively
   cross-platform in `std`).
