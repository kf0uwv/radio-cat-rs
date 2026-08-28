# 15. Separate what a radio *model* can do from what an *installation* has wired

Date: 2026-08-28

## Status

**Proposed** — drafted for architect + user review. No code has been
written. Amends [ADR 0010](0010-capability-model-and-normalized-signal-source.md)
§1 and §3, whose claim that "every field is answerable statically per
model" is true of most of `RadioCapabilities` and false of two of its
fields.

## Context

ADR 0010 §1 states:

> Every field is answerable statically per model, so the handshake costs no
> round trip to the radio.

That claim is load-bearing. It is why `RadioCapabilities` is `Copy` and
`const`-constructible, why a radio crate can declare its capabilities as a
constant, and why `cat-framework`'s own test asserts a whole
`RadioCapabilities` can be built as a `const`.

It is also, for two fields, not true — and Task 13's gate did not catch it
because the fixture written to pass the gate quietly encoded the same
mistake.

### The tell was already in the fixture

`capabilities_fixtures.rs` describes the TS-570D with:

```rust
signal: SignalCapability::IfTap(IfTapConfig {
    if_center_hz: 73_050_000,
    inverted: true,
    trim_hz: 0,
}),
```

and a comment conceding that `trim_hz` "is a per-station measurement, not a
property of the model." That comment is correct, and it is an admission
that a per-station value is sitting inside a per-model constant. The
zero is not a real trim; it is a placeholder standing in for a fact the
type cannot express.

The larger problem is one level up: the constant asserts that a TS-570D
**has** an IF tap at all. It does not. Every TS-570D has a CN4 header on
its TX-RX unit; whether an SDR is connected to it is a fact about one
bench, on one day.

### The hardware that made it concrete

The user has built an interface box carrying, on one assembly: the CAT
serial port (whose DTR line also keys PTT through ACC2), RF audio in and
out through ACC2 to a **USB sound device inside the box**, and the SDR on
the CN4 tap.

The same radio model, with and without that box, differs in:

| | Bare TS-570D | With the box |
|---|---|---|
| Endpoints | one serial handle, or none | serial handle **and** a USB audio device |
| `SignalCapability` | `None` | `IfTap { .. }` |
| `AudioDerived` | absent | available |

Nothing in that table is a property of the *model*. Every row is a
property of the *installation*. And `RadioCapabilities` currently has no
way to say so, which is why the fixture had to pick one and assert it.

`ts570d` ADR 0008 anticipated exactly this and got the timing wrong:

> The TS-570D has no USB codec at all — any audio would come from a
> soundcard on ACC2 — so it will report `AudioDerived` absent **for the
> foreseeable future**.

The foreseeable future was three weeks.

### Why `IfTapConfig` is the clearest evidence

`IfTapConfig` has three fields and they do not belong to the same world:

- `if_center_hz: 73_050_000` — a property of the **radio's circuitry**. The
  TS-570D's first IF. True of every one ever built.
- `inverted: true` — likewise. LO1 is high-side injection (73.05–103.05
  MHz), so the tapped spectrum is mirrored on every TS-570D.
- `trim_hz` — a property of **one dongle's crystal**, measured against WWV
  by one operator.

Two model facts and one station fact in a three-field struct. That is the
whole problem in miniature.

## Decision

**Split the capability model along the seam that already exists in the
data: what the model can do, and what this installation has wired.**

### 1. `RadioCapabilities` keeps only model facts, and keeps its `const`

Modes, meters and their ranges, tuning steps, coverage, filters, memory,
menu, VFOs — all genuinely static per model, all staying exactly as they
are. ADR 0010 §1's claim becomes true rather than mostly true.

Its endpoint field changes meaning: it describes the endpoint topology the
model **supports** — which roles exist, and which may share a handle —
not which are currently connected. The TS-570D supporting `Cat` + `Keying`
on one port is a fact about its RS-232C wiring, and stays here.

Its signal field describes what the model can **accept**: for the TS-570D,
that an IF tap is possible, at 73.05 MHz, inverted. That is circuitry.

### 2. A new `Installation` carries the per-station facts

Runtime data, not `const`, resolved when a server starts against real
hardware:

- Which endpoint roles are actually connected, and on which handles.
- Whether a spectrum source is attached, and its configuration —
  including `trim_hz`, which finally lives somewhere it can be written and
  persisted rather than being a zero in a constant.
- Whether an audio device is present.

### 3. The handshake publishes both, resolved

A client asks one question — "what can I do right now?" — and the answer
is the model's capabilities narrowed by what is installed. A console does
not want to know that a TS-570D *could* take an IF tap; it wants to know
whether to draw a waterfall.

`cat-server`'s `CapabilitiesWire` already converts a `&'static
RadioCapabilities` into owned data per connection (ADR 0010 §6). That is
exactly the right place for the resolution to happen, and it means this
decision does not change the protocol's shape — only what fills it.

### 4. `IfTapConfig` splits along its own seam

`if_center_hz` and `inverted` are model facts and stay with the radio.
`trim_hz` becomes installation data. This is what stops a per-station
calibration from being a placeholder zero inside a `const`.

### 5. A station has *several* signal sources at once, not one

Added 2026-08-28 after the console design surfaced it, and it is the part of
this decision that actually blocks work.

`RadioCapabilities.signal` is a **single** `SignalCapability`. The
installation described above has **two sources live simultaneously** — an
`IfTap` on CN4 and an `AudioDerived` path through the USB sound device —
and one enum field cannot say so. Downstream, `ConsoleState::for_radio`
derives exactly one `SpectrumLane` from that one field, so there is no lane
for audio at all: not an absent one, no place for one to exist.

So `Installation` carries a **set** of sources, not a field. A consumer asks
"which sources can drive a band panorama?" and gets zero, one or several —
which is also what makes the IC-7100's likely shape (a native scope *and*
an audio path) expressible without another amendment.

`SpectrumLane::available: bool` needs the same widening for a different
reason: audio has **three** states, not two — no endpoint at all,
configured but not streaming, and streaming. The middle one is the state
this station is in today, since the transport design does not exist, and a
`bool` cannot express it. This is the same shape as `CatLane.pending`
being a bare count: a type that answers a narrower question than the UI
must ask.

### 6. Enforce "never a band panorama" in the type system, not a comment

`SpectrumFrame` is frequency-domain and carries `center_hz`. Pushing an AF
spectrum through it would make `bin_frequency_hz()` return audio Hz that
are indistinguishable from RF Hz, and `retune()` meaningless — the exact
confusion `AudioDerived { max_bandwidth_hz }` was introduced to prevent.

Today that prevention is a method (`is_band_panorama()`) plus a doc comment
asking consumers to check it. A separate frame type for audio-domain data
makes the compiler do it instead, and no consumer can forget. A
time-domain scope needs its own shape regardless: ADR 0010 §4 gives
`AudioDerived` only FFT-flavoured descriptors (`input_device`, `fft_size`,
`window`, `averaging`) and nothing for a timebase or a level, because a
scope was never modelled.

### Explicitly out of scope for this ADR

- **Persisting an `Installation`.** Where the trim, the device paths and
  the audio device selection are stored between runs is a configuration
  design, not this one.
- **Discovering hardware automatically.** Whether the server probes for a
  dongle or is told about one is a separate question.
- **Audio transport.** Still deferred by ADR 0010. This decision lets an
  installation *report* an audio device; streaming it remains undesigned.
- **Re-running Task 13's gate.** The gate's verdict stands — the model fits
  two radios. What it did not test is this seam, because the fixture
  encoded the conflation rather than exposing it.

## Consequences

**Good.**

- ADR 0010 §1's central claim becomes true without qualification, and the
  `const` test stops guarding a partial truth.
- `trim_hz` gets a home it can actually be written to. Today the only place
  it exists at rest is a zero in a constant with a comment apologising for
  it.
- A radio's description stops changing when someone unplugs a dongle. The
  same TS-570D reports the same `RadioCapabilities` on every bench, which
  is what makes it a description of the *model*.
- The absent-spectrum state the console design treats as first-class gets a
  real source: `Installation` says the tap is not connected, rather than a
  constant having to lie in one direction or the other.
- It generalizes. The IC-7100 will bring the same question — an installed
  option is neither a model fact nor an accident — and the seam is already
  drawn.

**Costs and risks.**

- **It is a second type where there was one**, and every consumer that
  currently takes `&RadioCapabilities` must be looked at to decide whether
  it wants model facts, installed facts, or the resolved view. Most want
  the resolved view.
- **The audio panels cannot be built until §5 lands.** That is the one hard
  ordering constraint this ADR creates: the console design has AF
  instruments with no lane to read from.
- **Task 16's `CapabilitiesWire` and Task 13's fixtures both need
  revising**, and the fixtures are the acceptance gate's evidence. They
  should be revised to *demonstrate* the seam — the TS-570D described once
  as a model, then twice as installations, bare and with the box.
- **The boundary will be argued.** Is a fitted filter a model fact or an
  installed one? The rule this ADR offers: if two units of the same model
  can disagree about it, it is installation data. A fitted filter is
  therefore installation data, and so is anything else optional.
- Nothing in the queue is blocked by this, but the app-side migration
  should not start until it is settled, or it will be written twice.
