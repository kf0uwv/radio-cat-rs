# Task plan: Windows network transports (Deliverable 1)

## Phases
1. [x] Read CLAUDE.md, ADRs 0001-0005, all relevant source (see findings.md).
2. [ ] Move `cat-transport-serial::oneshot` -> `cat-transport-core::completion`
       (pub module). Update serial's `windows.rs` to import it. Verify
       `cargo test -p cat-transport-serial` and `-p cat-transport-core` green.
3. [ ] `cat-transport-tcp`: extract `codec.rs` (pure), gate `session.rs` to
       Linux, add `windows.rs` (worker-thread + completion). Update
       `Cargo.toml` (no new deps needed -- std only). Tests: Linux unchanged;
       Windows type-checked only.
4. [ ] `cat-transport-udp`: same shape (codec.rs/session.rs/windows.rs).
5. [ ] `cat-server`: extract `broker.rs`'s timeout call behind a small
       cfg-gated helper; split Job/BrokerWorker/BrokerHandle/build into
       worker_linux.rs / worker_windows.rs (via `#[path]`-aliased `mod worker`);
       split tcp.rs/udp.rs similarly (`#[path]`-aliased `mod tcp`/`mod udp`);
       add `timeout.rs` (pure combinator, tested on Linux) and `block_on.rs`
       (pure, tested on Linux) for the Windows path.
6. [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
       `cargo test --workspace` (Linux, must stay green, 125+ tests).
7. [ ] `rustup target add x86_64-pc-windows-gnu` (if needed);
       `cargo check --target x86_64-pc-windows-gnu -p cat-transport-tcp -p
       cat-transport-udp -p cat-server` (and -p cat-transport-serial to confirm
       the completion move didn't regress it).
8. [ ] Write docs/adr/0006-windows-network-transport.md (Deliverable 1, folds in
       Deliverable 3's pin-test-tool placement decision and Deliverable 4's
       NoModemControlLines decision per the task's "your call" on ADR grouping).
       Update docs/adr/README.md index.
9. [ ] Commit incrementally per sub-step above.

## Decision log
See findings.md for the full design. Summary: move+share `completion`
primitive; ungated `codec.rs` per transport crate (mirrors ADR 0004's
`config.rs`); Windows session types are worker-thread-backed, same public
type names; cat-server keeps `Broker`/`DispatchOutcome`/`DispatchError`/
`ClientRegistry`/`DedupCache` fully shared, forks only the Job-queue channel
and listener concurrency substrate (OS threads + `Arc<Mutex<_>>` on Windows
vs. cooperative monoio tasks + `Rc<RefCell<_>>` on Linux), via same-path
`#[path]`-aliased modules so `cat_server::{tcp,udp,worker}` resolve
identically on both platforms.
