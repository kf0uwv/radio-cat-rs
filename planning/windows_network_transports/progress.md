# Progress log: Windows network transports

## Completed 2026-07-26

All phases in task_plan.md done. Key deviation from the original plan,
recorded in the ADR: attempted a single universal (monoio-free) timeout
combinator for cat-server's Broker::dispatch; this broke cat-server's own
monoio-driven test suite (real hang, not a compile error) because monoio's
Waker does not reliably support being woken from a foreign OS thread.
Reverted to a target_os-gated split (monoio on Linux, portable combinator
on Windows) and scoped the "test cat-server's Windows-shaped modules on
Linux too" bonus down to the two transport crates only (cat-transport-tcp/
-udp's Windows backends have no such dependency and are fully tested on
Linux; cat-server's worker_windows/tcp_windows/udp_windows are Windows-only
test-gated, matching ADR 0004's original precedent).

Final state: docs/adr/0006-windows-network-transport.md written and
accepted, covering Deliverables 1, 3, and 4. docs/adr/README.md updated.
Full workspace test suite: 187 passed, 0 failed. cargo check --target
x86_64-pc-windows-gnu clean for cat-transport-serial, cat-transport-tcp,
cat-transport-udp, cat-server (including --all-targets for cat-server).
