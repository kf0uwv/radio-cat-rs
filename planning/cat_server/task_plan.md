# Task Plan — cat_server

No extraction yet; awaiting ts570d refactor completion and an explicit
go-ahead.

`cat-server` does not exist as code yet — it has no direct analog in
`ts570d` today. When extraction is authorized, this file should record the
plan for building the request broker (client session management, physical
radio session ownership, single ordered worker, request/response
correlation, timeout/disconnect handling, malformed-request rejection)
described in `docs/adr/0001-scope-and-crate-boundaries.md`.
