# Task Plan — cat_transport

No extraction yet; awaiting ts570d refactor completion and an explicit
go-ahead.

`cat-transport-core`, `cat-transport-serial`, `cat-transport-tcp`, and
`cat-transport-udp` do not exist as code yet. When extraction is authorized,
this file should record the plan for lifting `ts570d`'s
`framework::transport::Transport` and `CatSession` traits, and
`SerialCatSession`, into `cat-transport-core`/`cat-transport-serial`, plus
the design decision on the monoio/runtime-agnostic open item (see
`docs/adr/0001-scope-and-crate-boundaries.md`) before any TCP/UDP or Windows
work begins.
