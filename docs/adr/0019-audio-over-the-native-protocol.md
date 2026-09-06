# ADR 0019 — Audio over the native protocol

Status: Accepted (2026-09-02)

## Context

The protocol carried spectrum frames and no audio. A server could offer
its sound cards to a console (ADR 0018) and then had to refuse to attach
one, because there was nowhere for the samples to go. The AF panels on a
remote console were dark, and the refusal message had to explain which
half was missing — which is honest, and is not the same as working.

## Decision

A third frame kind, `FrameKind::Audio`, carrying a `cat_signal::AudioFrame`.

### The server runs the DSP

The samples are on the machine with the sound card in it. Sending raw PCM
would move the FFT to a console that has no audio hardware in the picture
at all, and cost more to do it: 48 kHz of `f32` is about 190 KB/s against
roughly 145 KB/s of finished frames at the pump rate.

It also keeps the audio path the same shape as the spectrum path, where
the server likewise computes and the console likewise draws.

### Both halves travel in one payload

An `AudioFrame` is a scope trace and an AF spectrum computed from the
**same block**, sharing a sequence number. That is the entire reason the
type is a pair rather than two streams: a console must not be able to draw
a waveform and a spectrum that disagree about what the radio was doing.
Two frame kinds could not promise it, so there is one payload and one
sequence number, and a test asserts the number survives.

### `AudioFrame` moved into `cat-signal`

It is a data pair with no I/O, and it now has to be named by `cat-native`.
Leaving it in `cat-signal-audio` would have forced the protocol crate to
depend on a crate that can link cpal in order to declare a struct.
`cat-signal-audio` re-exports it, so callers did not change.

`AudioTap` moved with it, for a sharper reason: a server *capturing*
audio needs exactly the same abstraction as a console *drawing* it, and
the trait had been living in the terminal UI crate. A server that had to
depend on a TUI to name it would be an odd shape indeed.

### Two opt-ins, not one

`Hello` gained an `audio` flag beside `spectrum`, and `Connection::connect`
now takes a `Streams { spectrum, audio }` rather than a bare bool —
`connect(addr, true, false)` says nothing about which is which, and a
second bool in the same position is the argument nobody gets right.

They are independent because a console with AF panels and no waterfall is
an ordinary way to run, and should not be sent 2048 bins thirty times a
second to throw away. `#[serde(default)]` on the new field means a client
built before audio existed asks for none and is sent none.

## Consequences

### A stalled capture must not look live

The pump does not resend a frame whose sequence the client already has,
and `RemoteAudio` drops one too. This matters more for audio than for
spectrum: an operator watches a scope trace precisely to see whether audio
is *moving*, so a repeated trace reads as a steady tone — the opposite of
what has happened.

### "No source" and "silence" stay different answers

Silence is a signal; a quiet band sounds exactly like it. A server with
nothing attached publishes `None` rather than a flat trace, so a console
shows its AF panels as absent rather than drawing a straight line an
operator would go looking for a cable to explain.

### A frame kind is two edits, not one

Adding `Audio` to the enum did not add it to `FrameKind::from_u8`, so the
server sent frames the client rejected as an unknown kind — and the
symptom appeared on the *receiver* as a broken stream while the sender
looked fine. Found by a socket-level test, not by the codec's own unit
tests, which were happy throughout. There is now a test asserting every
variant round-trips through `from_u8`.
