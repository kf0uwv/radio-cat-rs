# 13. Renderer parity: the TUI and the GUI expose the same capabilities, and the TUI is permanent

Date: 2026-08-27

## Status

**Accepted** (2026-08-27) — user sign-off. Binding on all UI work from this date.
Amends [ADR 0011](0011-cat-ui-base-widgets-radio-specific-layout.md), which
deferred `cat-ui-ratatui`, and `ts570d`'s ADR 0008, which described the TUI
as "the headless/SSH path."

## Context

A GPU GUI is arriving beside a shipping ratatui TUI (`ts570d` ADR 0008).
The well-known failure mode is not that anyone decides to abandon the older
renderer — it is that nobody decides anything. New features land where they
are most fun to build, the older renderer accumulates gaps, and eighteen
months later someone observes that the TUI is missing half the product and
proposes retiring it. The decision gets made by drift.

`ts570d` ADR 0008 already records that the user rejected retiring the TUI.
That is a decision about the *end state*. It does not, by itself, prevent
the drift, because every individual instance of drift looks reasonable.

**The TUI is not a lesser path.** It is the only renderer that works over a
plain SSH session on a headless shack machine, on hardware with no GPU or no
display server, over a link too thin for a waterfall but perfectly adequate
for CAT control, and inside a minimal container. The GUI's network model
(ADR 0008 §2) makes *remote* operation work, but it still requires a
GPU-capable client at one end. The TUI requires a terminal.

### The drift has already started, at a smaller scale

The two existing TUIs are one renderer, in one fleet, and they are already
asymmetric. `ts570d/ui/src/layout.rs:37` has `smeter_label()`, mapping the
raw meter reading to `S0`…`S9+30`. `ft991a/ui/src/layout.rs` has no
equivalent — an operator reads a bar with no S-unit called out. Whether
that was a decision or an omission is not recorded anywhere, which is the
point: nobody can tell, so nobody can fix it with confidence.

That is radio-to-radio drift, between two repos, with no shared code path.
Renderer-to-renderer drift inside a single app will be faster, because the
temptation is larger and the two renderers sit in adjacent directories.

## Decision

### 1. Parity is on capabilities, not on pixels

**Every capability in `RadioCapabilities` that one renderer lets an operator
read or change, the other renderer must also let them read or change.**

Presentation, information density, layout and interaction idiom may differ
completely — that is exactly what ADR 0011 leaves to each app, and a
terminal that imitated a GPU console would be a worse terminal. The
obligation is on *reachability of the capability*, nothing more.

### 2. "Within reason" — the three grounds for an exception

An exception is legitimate on exactly these grounds:

**(a) The medium cannot carry the data at useful fidelity.** A 60 fps GPU
waterfall is not a terminal artifact. But note carefully what this licenses:
an exception on **fidelity**, not on **presence**. The terminal gets a
coarse spectrum at a low frame rate, or a half-block waterfall, or — at
minimum — a visible affordance saying a spectrum source is connected and
where to see it. Silent absence is not an exception under this ground; it
is the gap this ADR exists to prevent.

**(b) The interaction has no terminal equivalent.** Continuous drag on a
rotary knob is a pointer gesture. The terminal reaches the same underlying
parameter by keys and steps. The *parameter* is still reachable; only the
gesture is not.

**(c) The feature is genuinely in progress in one renderer.** Time-bounded
and tracked. This is the ground that lets the GUI land something first.

**Development cost is explicitly not a ground.** It is the exception that
would swallow the rule, since it applies to everything.

### 3. Exceptions are recorded where a reviewer will see them

Each app carries `docs/renderer-parity.md`: one row per exception, naming
the capability, the renderer that lacks it, the ground (a/b/c), and for
ground (c) the tracking item that will close it.

**A feature that ships in one renderer with neither a counterpart nor a row
in that table is a review failure**, not a follow-up. The table is the
deliverable that makes this ADR enforceable rather than aspirational; an
empty table means parity is complete, which is a meaningful thing to be able
to assert.

### 4. The mechanism, not just the rule

Parity is a discipline problem only if the two renderers are built on
different foundations. Under ADR 0011 they are not: `cat-ui` holds the
renderer-agnostic presentation logic, `RadioCapabilities` holds the feature
list, and both renderer crates consume the same normalized types. "Does the
TUI handle this capability?" becomes a question with a mechanical answer.

**This is the load-bearing reason ADR 0011 revision 4 promotes
`cat-ui-ratatui` from deferred to in-scope.** A shared GPU widget set with
no terminal counterpart does not merely permit the asymmetry this ADR
forbids — it builds it into the crate graph, and every widget added to
`cat-ui-egui` widens the gap by construction.

### 5. The TUI is permanent

Neither renderer is retired without a superseding ADR. Both ship in every
release, for every radio in the fleet, including `ic7100` when it is
scaffolded.

### Explicitly out of scope for this ADR

- **Which features exist at all.** Per-radio, and ADR 0011's business.
- **Feature parity *between radios*.** `ft991a`'s `control.rs` is
  legitimately 5.4× `ts570d`'s. This ADR is about renderers within one app.
- **Visual design.** Parity of capability says nothing about how either
  renderer looks.
- **Audio panels** (AF scope, AF FFT). Blocked upstream on the audio-stream
  design ADR 0010 defers, and therefore absent from both renderers — parity
  is trivially held and stays that way until that work is decided.

## Consequences

**Good.**

- The TUI cannot decay into a legacy path by accumulation of individually
  reasonable omissions, which is the only way it realistically would.
- Headless, SSH, no-GPU and thin-link operation stay first-class rather than
  becoming "the old way."
- New features are pushed through the renderer-agnostic layer first, which
  is where they belong anyway. A feature that is hard to express in `cat-ui`
  is usually a feature with layout assumptions baked into it — parity
  surfaces that at design time instead of at port time.
- Reviewers get one concrete question to ask about every UI change, instead
  of a vague sense that the TUI is falling behind.

**Costs and risks.**

- **Every feature costs two implementations.** Real, and accepted. `cat-ui`
  reduces it to two thin renderings of one piece of logic, but does not
  reduce it to one.
- **The rule can pace the GUI to the terminal.** This is the genuine risk,
  and ground (c) is the deliberate release valve: the GUI may land first as
  long as the debt is visible and owned. If (c) rows start accumulating
  without closing, the rule is being used to defer work rather than to
  sequence it, and that is the signal to revisit this ADR.
- **A stale exception table is worse than no table**, because it asserts
  something false. It needs pruning as part of the same review that adds to
  it.
- ADR 0011's deferred TUI divergence audit stops being optional. It was a
  cost the fleet could carry indefinitely; under this ADR it is on the
  critical path to a maintainable second renderer.
