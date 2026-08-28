# 11. `cat-ui`: shared base widgets for both renderers; layout and features stay radio-specific

Date: 2026-08-27

## Status

**Accepted** (2026-08-27) — user sign-off; implementation authorized via
`planning/architect/task_plan.md` (Tasks 18-20). No code has been written yet.

Revision 4 (2026-08-27), after user direction that the TUI is permanent and
that a generic *terminal* widget set is wanted alongside the GPU one.
`cat-ui-ratatui` is promoted from deferred to in-scope, and migrating the
existing TUIs onto it is part of this decision rather than a later one. See
[ADR 0013](0013-renderer-parity-tui-and-gui.md) for the parity rule that
makes this necessary rather than merely nice.

Three earlier drafts of this number were reviewed and rejected: the first
proposed a fully shared UI (rejected — a shared `ConsoleModel` would have
had to encode layout intent); the second proposed no shared UI at all
(rejected — leaf widgets and radio-concept components are genuinely
reusable); the third scoped the shared set to the GPU renderer only
(superseded here).

## Context

Each radio app owns a `ui` crate, and a GPU-rendered `gui` crate is planned
for `ts570d` (ADR 0008 there). The question is not *whether* UI code is
shared but *where the seam falls* — and, since revision 4, whether the
shared set serves one renderer or both.

### The duplication is real, and it is measurable at function level

[ADR 0005](0005-rigctl-bridge-and-radio-trait-boundary.md) established this
workspace's thesis while de-duplicating two rigctld bridges:

> This is exactly the kind of drift this repo's whole `radio-cat-rs`
> extraction effort exists to prevent — divergent bugfixes are strictly
> worse than either app lacking a fix outright, because they are silent.

That was argued over ~400 lines. The `ui` crates are in the same state, much
larger:

| File | `ts570d` | `ft991a` |
|---|---|---|
| `layout.rs` | 1055 | 1048 |
| `terminal.rs` | 2636 | 3543 |
| `control.rs` | 1711 | 9189 |
| `lib.rs` | 141 | 214 |

Not inferred from line counts: `ft991a/ui/src/layout.rs`'s own header says
it was *"ported near-verbatim from `ts570d/ui/src/layout.rs`"*, and a `diff`
reports 1142 changed lines across 1467.

Reading the two files side by side shows the three states the seam has to
account for, all present today:

- **`format_hz()` and `mini_bar()` are byte-identical** in both repos.
  Copies, maintained twice, with nothing radio-specific in either.
- **`smeter_bar()` is the same drawing with a different scale.** Identical
  glyph vocabulary (`▐ █ ░ ▌`) and identical structure; `ts570d` takes a
  `u16` over 0–30 with the width hardcoded to 20, `ft991a` a `u8` over
  0–255 with `width` as a parameter. The *rendering* is one function. The
  *scale* is a property of the radio.
- **`smeter_label()` exists only in `ts570d`.** The FT-991A TUI shows a bar
  with no S-unit called out. Nothing records whether that was decided or
  merely never done.

The middle case is the instructive one, and it sharpens revision 3's rule of
thumb. A widget parameter that carries **radio data** (`MeterSet`'s range,
so one bar serves both meters) belongs in the shared widget. A parameter
that carries **one radio's taste** does not. Revision 3 said only the
latter; stated that bluntly it would have pushed `smeter_bar` back into both
apps and preserved the duplication it was written to remove.

### But a UI is not one thing

`cat-rigctl` was extractable because an external standard fully specifies
it. A console is not one such artifact. It decomposes into layers that sit
at very different distances from the radio:

| Layer | Varies by radio? | Example |
|---|---|---|
| Leaf widget | No | waterfall pass, S-meter, knob, bar meter |
| Radio-concept component | No | VFO readout, meter rail, band grid, memory list, settings-descriptor panel |
| Layout / hierarchy | **Yes** | where the waterfall sits, what the left rail holds |
| Feature set / menu topology | **Yes** | `ft991a`'s `control.rs` is 5.4× `ts570d`'s |

Sharing everything fails on the bottom two rows — that is why the first
draft was rejected. Sharing nothing fails on the top two — a wgpu waterfall
pass has no radio opinion whatsoever, and neither does `format_hz`.

### Why the shared set must serve both renderers

Revision 3 scoped it to `cat-ui-egui` and deferred the terminal equivalent
behind the divergence audit. ADR 0013 makes that untenable: if the shared
widget set exists only for the GPU renderer, then every widget added to it
widens the TUI/GUI gap *by construction*, and the parity rule is fighting
the crate graph rather than resting on it.

The audit does not disappear. It stops being a **gate** and becomes the
**mechanism** — see Decision §3.

## Decision

**`cat-ui` exists and serves both renderers. It is scoped to base widgets
and radio-concept components. Layout and features stay in each radio's own
`ui`/`gui` crate.**

### Crates

- **`cat-ui`** — renderer-agnostic, and the largest of the three. Formatting
  and presentation logic for radio concepts: frequency and S-unit
  formatting, meter scaling against `MeterSet` ranges, band plans, the
  spectrum ring buffer, and the two-rate discipline (spectrum frames are
  push at ~60 fps; CAT state is request/response and slow — they live in
  separate lanes so a renderer cannot block a waterfall behind a menu read).
  No `egui`, no `ratatui`, no `wgpu`. Headlessly testable, which none of the
  current UI code is.
- **`cat-ui-egui`** — the GPU widget set: the waterfall as a custom `wgpu`
  render pass via `egui_wgpu`'s callback, spectrum plot, analog S-meter,
  rotary tuning knob, bar meters driven by `MeterSet`, VFO readout, band
  and mode grids, and one generic renderer for ADR 0010 §4's
  `SettingDescriptor` lists.
- **`cat-ui-ratatui`** — the terminal widget set, covering the same concept
  list: coarse spectrum and half-block waterfall, S-meter bar with S-unit
  label, meter rail, VFO readout, band and mode grids, menu column builders,
  the connection/error/disconnected panels, and the same generic
  `SettingDescriptor` renderer. `format_hz`, `mini_bar` and `smeter_bar`
  are the first three functions it absorbs.

The concept list is deliberately the same for both renderer crates. Where a
concept cannot be carried at useful fidelity in a terminal, ADR 0013 §2(a)
governs — a coarser rendering, not an omission.

### What stays radio-specific

Each app's `ui`/`gui` crate owns, and `cat-ui` must never contain:

- Layout and information hierarchy — which panels exist, where, how big.
- Feature set and menu topology.
- Keybindings and interaction conventions.
- Visual identity.

A widget in either renderer crate takes normalized data (`SpectrumFrame`,
`MeterSet`, `SettingDescriptor`) and draws it. It never decides whether it
should be on screen, or where.

### The divergence audit is the extraction, not a prerequisite for it

Revision 3 held `cat-ui-ratatui` behind an audit of the 1142 changed lines,
on the grounds that each difference is either a bugfix one app has and the
other lacks, or a genuine radio-specific difference belonging in layout —
and telling those apart is the real work.

That reasoning stands. The sequencing does not. Performed as a standalone
audit it is a large read-only exercise producing a document; performed as
the extraction it is the same question asked one function at a time, with a
compiler checking each answer and a shared implementation as the artifact.
`format_hz` takes minutes. `smeter_bar` forces exactly one real decision
(the scale is `MeterSet` data). `control.rs`'s 5.4× asymmetry is mostly
menu topology, which this ADR leaves in the apps and which therefore never
needs reconciling at all.

**Order within the work is fixed: `cat-ui` first.** The renderer-agnostic
crate is where `format_hz`, meter scaling and band plans live, so extracting
it settles most of the audit before either renderer crate is written.

### Migrating the existing TUIs is in scope

Revision 3 left `ts570d/ui` and `ft991a/ui` untouched. They are now migrated
onto `cat-ui` + `cat-ui-ratatui` as part of this decision, per app, in the
app's own repo, keeping each app's layout and feature set exactly as it
stands. **A migration that changes what an operator sees has overreached** —
except where it closes a gap the audit identifies as a missing bugfix, which
is recorded explicitly in that app's `docs/renderer-parity.md`.

### Explicitly out of scope for this ADR

- **Retiring either renderer.** ADR 0013: both ship, for every radio.
- **A visual design language.** Layout, spacing and colour are design
  process outputs, not architecture.
- **Audio widgets** (AF scope, AF FFT). Blocked on the audio-stream design
  ADR 0010 leaves out of scope; absent from both renderers, so ADR 0013's
  parity rule is trivially satisfied.
- **Reconciling `control.rs`.** Menu topology is radio-specific by this
  ADR's own seam, so the largest single divergence is out of scope by
  construction, not by deferral.

## Consequences

**Good.**

- The wgpu waterfall pass, the S-meter, the knob and the bar meters are
  written **once per renderer**, for every radio present and future.
  `ic7100`'s TUI and GUI are both layout, not a third and fourth widget set.
- `format_hz` and `mini_bar` stop existing twice. `smeter_bar` stops
  existing twice *and* gains the `MeterSet` scale that makes one
  implementation correct for both radios.
- `cat-ui` is headlessly testable, so meter scaling and frequency
  formatting get unit tests for the first time.
- ADR 0013's parity rule rests on the crate graph instead of on discipline:
  a capability rendered in one crate has an obvious home in the other.
- `ft991a` stays 5.4× richer than `ts570d` without either distorting the
  other — the abstraction stops below the layer where they differ.
- ADR 0010 §4's delegated settings get exactly one renderer per medium.

**Costs and risks.**

- **The seam is a judgement call and will be tested.** The first time a
  radio wants a *slightly* different S-meter, the pressure will be to add a
  configuration knob rather than a widget to that app. Rule of thumb for
  reviewers, in the sharpened form above: a parameter carrying **radio
  data** belongs in the shared widget; a parameter carrying **one radio's
  preference** belongs in that radio's crate.
- **Migrating two shipping TUIs carries regression risk that revision 3
  avoided by not doing it.** Both apps have real users and neither `ui`
  crate is well covered by tests. The migration is per app, behind review,
  and its acceptance bar is that the operator sees no change.
- `wgpu` and a GPU toolchain enter this repository's CI.
- Three UI crates instead of two, and a cross-repo version pin now covers
  UI work — a `cat-ui` fix needs a release and a bump in each app rather
  than shipping with the app.
- **The audit's hard cases are now discovered during extraction rather than
  before it.** That is the deliberate trade: cheaper overall, but a
  genuinely irreconcilable difference surfaces mid-task rather than in a
  planning document. Such a case is escalated, not resolved by widening the
  shared widget.
