# ADR 0020 — Shared parts, per-radio arrangement, authored by the server

Status: Accepted (2026-09-02)

## Context

The console had become one console. Capability-derived, so the right tabs
appeared and a radio's own meters were the meters it showed — and
identical in arrangement for every rig, because arrangement was code.

That gets a long way and stops exactly where it matters. A TS-570D with an
SDR on its IF tap wants the waterfall dominating the screen; somebody
bought that dongle to see the band. An FT-991A has **no spectrum over CAT
at all** and 151 menu items, and wants that same space given to what it
does have. Both are the same components. Neither is the same console.

Deriving that difference from a capability set means encoding one
designer's taste in a renderer and calling it inference.

## Decision

**Shared: the parts. Per radio: the arrangement. Authored by the server.**

- `cat-layout` — a closed `PanelKind` vocabulary, a `Node` tree of splits,
  and one solver. Pure data: no rendering, no I/O, and no dependency on
  either renderer, because a layout is published by a server and read by a
  console and must sit below both. It cannot live in `cat-ui` either —
  `cat-ui` depends on `cat-native`, and `cat-native` is what carries a
  layout on the wire.
- `CapabilitiesWire::layout` — published in the handshake, beside what the
  radio is. A console is told how to arrange itself in the same breath as
  what it is arranging.
- `RadioHost::layout` — the server's to answer, because the server is what
  knows the rig.
- Each radio crate has a `console_layout` module beside `capabilities`,
  and it is prose about that radio as much as it is code.

**The vocabulary is shared, with a named way out.** A radio choosing its
arrangement composes from parts that already work — that is how three
consoles avoid growing three subtly different S-meters. But a rig
sometimes has a feature no other rig has, and telling its operator to do
without because the vocabulary is closed would be the wrong trade.

So `PanelKind::Custom(name)` exists, the base building blocks are public
(`meter_bar`, `af`, `waterfall`, `spectrum_map`, and egui itself), and a
radio's crate registers a painter under a name it prefixes with the rig.
**A pure network console has not linked that crate and will not have the
painter** — it draws the panel's name rather than a blank rectangle,
because "no widget for ft991a.clarifier" is a different message from a
widget that failed, with a different fix.

## Consequences

### Two radios, two consoles, neither derived from the other

At 120x40 the TS-570D asks for `Spectrum` 72x21 with the workspace beneath
it; the FT-991A places no spectrum at all and gives the workspace 70x34,
with a taller meter rail for its five meters and its band and mode buttons
moved into the right rail where fourteen modes fit.

### Silence means "not asked", not "no opinion"

`layout: None` falls back to the arrangement the console has always had.
An older server has not declined to specify one; it has never been asked.
A console that showed an empty screen for that would break every existing
deployment on the day this shipped.

### A fixed size that cannot be met is dropped, not shrunk

The solver gives a `Fixed(20)` panel twenty units or nothing. Twenty is
what an AF FFT needs for 150 Hz per cell; nineteen is not a smaller AF FFT
but a wrong one. A small terminal loses its rightmost panel and keeps a
working radio.

### A panel a build cannot draw is a gap

`PanelKind` is `#[non_exhaustive]`, and a renderer skips what it does not
know. Drawing something else where a panel was asked for is worse than
leaving a hole, because a hole is visibly a hole.

### The console must use the *server's* capability document

Found the hard way: the TUI built its own from the local static
declaration, so the layout arrived and was ignored. In `--server` mode the
authoritative document is the far end's — it is the one carrying the
layout that server authored, and the one that knows what the bench has
wired.

### The look is the radio's too

A layout says where the panels go; `cat_layout::Theme` says what they look
like, and it is the radio's for the same reason. An operator in front of a
TS-570D is looking at an **amber LCD on a charcoal panel**; an FT-991A is
a **colour TFT, blue and white**. A console that matches its rig is one
whose readout can be found without translating between two visual
languages — and with two rigs on a bench, which console is which is
legible before a single label is read.

Six colours by role, and the renderer derives its shades by mixing, so a
radio describes its front panel rather than enumerating twenty tokens.
**Structure, type scale and spacing stay the design system's**: a radio
picks its palette and cannot restyle a component into something another
radio's operator would not recognise.

The waterfall keeps its Turbo ramp regardless. That ramp is a measurement
scale — it encodes magnitude — and recolouring it to match a front panel
would make weak signals harder to see. An aesthetic gain is not worth that.

### The GPU console resolves layouts in cells, then scales

Both renderers resolve the same spec in character cells; the GPU console
multiplies by its own measured cell size. Asking for a rail "22 points
wide" would give three characters, and the two consoles would not have the
same proportions.

One thing this exposed: the same cell count does not always suit both,
because a panel's content differs in density between them. The readout
needed five cells rather than three, for the GPU console's taller strip.

## Not yet done

`cat-ui-ratatui` has no custom-widget registry yet — the escape hatch is
egui-only, so a terminal console meeting `PanelKind::Custom` skips it
rather than naming it.
