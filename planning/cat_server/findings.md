# Findings — cat_server

## Task 5 (2026-07-16)

Following the `cat_transport` agent's own precedent (see
`planning/cat_transport/progress.md`'s Task 2 section): this session's
harness disallows writing a separate freestanding findings/report file, so
the reasoning that would normally live here is folded into
`planning/cat_server/task_plan.md` (design decisions made *before* writing
code, including the "why raw wire bytes"/"why direct monoio framing"
sections) and `planning/cat_server/progress.md` (decisions/discrepancies
discovered *during* implementation and testing — notably the `readable`/
`writable`-driven routing fix, the `cat-framework` dependency addition, and
the UDP dedup cache's 3-field key). Read those two files for the full
reasoning; this file is left at its bootstrap placeholder otherwise.
