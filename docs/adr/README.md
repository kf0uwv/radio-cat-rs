# Architecture Decision Records

Decisions are recorded as [ADRs](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions)
(Michael Nygard format). Each file is one decision; numbers are stable and
never reused.

| ADR | Title | Status |
|-----|-------|--------|
| [0001](0001-scope-and-crate-boundaries.md) | Scope and crate boundaries for the shared CAT library | Accepted |

## Repository status

**No extraction has happened.** This repository holds planning and agent
scaffolding only — no crate source exists yet. Extraction from `ts570d` is
gated on:

1. `ts570d`'s own `refactor/generic-cat-framework` work completing (see
   `ts570d`'s `docs/adr/README.md` refactor-status table), and
2. an explicit go-ahead to begin the move, since ADR 0004 in `ts570d`
   deliberately keeps extraction itself out of scope of that refactor.

When extraction begins, start from [ADR 0001](0001-scope-and-crate-boundaries.md)
and the source ADRs it points to in `ts570d`, rather than re-deriving the
target design from scratch.
