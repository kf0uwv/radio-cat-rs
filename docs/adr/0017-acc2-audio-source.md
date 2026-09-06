# 17. `cat-signal-audio`: the client half of a radio's audio pair

Date: 2026-09-01

## Status

**Accepted.** Implemented as `cat-signal-audio`. First server: `ts570d`'s
emulator (`--acc2-audio <addr>`, that repo's ADR 0009). Closes the
"streaming audio" item `ts570d`'s ADR 0008 left undesigned on the consumer
side, and the "audio-derived spectrum" item
[ADR 0014](0014-rtlsdr-spectrum-source.md) listed as out of scope pending an
audio-stream design.

**Amended 2026-09-02** — see the amendment at the end of this file. A local
sound card is now a second source, behind a default-off `device` feature;
everything below stands unchanged.

## Context

[ADR 0010](0010-capability-model-and-normalized-signal-source.md) §4 named
`AudioDerived` as a signal capability and gave it settings, and
[ADR 0015](0015-model-facts-versus-installation-facts.md) §6 gave it two
frame types — `AudioScopeFrame` and `AudioSpectrumFrame` — deliberately
*not* `SpectrumFrame`, so that a few kHz of speech cannot be drawn across a
band axis as though it were a panorama.

Nothing produced them. `ts570d`'s emulator now serves a radio's ACC2 audio
pins over a socket, and nothing consumed that either. This is the missing
consumer.

### The wire, which is fixed

One TCP connection, full duplex, 48 kHz mono signed 16-bit little-endian PCM
in both directions, paced to real time by the server exactly as the CN4 tap
paces its IQ.

- **Server to client** is ANO, the radio's receive audio (ACC2 pin 3).
- **Client to server** is PKD, transmit audio into the radio (ACC2 pin 11).

There is no greeting, no framing and no handshake — unlike `rtl_tcp`, which
`cat-signal-rtlsdr` speaks, this is bytes from the first one. The stream is
continuous in both directions: a receiving radio always sends *something*,
because there is a noise floor, and a **transmitting** radio sends digital
silence. So a run of zeroes is a normal frame, and a gap is a broken link.

This ADR does not design that wire. It designs the thing that plugs into it.

### Three constraints from the rest of the workspace

- **`cat-signal` is dependency-light and pure** — `async-trait` and `serde`,
  nothing else, with a comment in its manifest saying `cat-signal-rtlsdr` is
  a separate crate precisely so that consuming a `SpectrumFrame` never drags
  in libusb.
- **Tokio is banned** and `monoio` is Linux-only and target-gated
  ([ADR 0002](0002-async-runtime-binding-for-transport-crates.md),
  [ADR 0004](0004-windows-serial-backend.md)).
- **A cross-thread wake into a monoio task panics** unless the *caller*
  enables monoio's `sync` feature ([ADR 0016](0016-rfc2217-transport.md)'s
  Consequences). That was found by running `ts570d`'s TUI, not by any test
  in this workspace, and no test here can catch a caller getting it wrong.

## Decision

### 1. A new crate, `cat-signal-audio`; `cat-signal` is not edited at all

A module inside `cat-signal` would have added `rustfft`, `std::net` and a
thread to the crate every consumer of a frame type depends on — including
`cat-ui`, whose entire dependency on `cat-signal` is that it needs to name
`AudioScopeFrame` and `AudioSpectrumFrame` in a widget signature. That is
the libusb argument verbatim, and it does not become weaker because an FFT
is smaller than a USB stack.

So the frame types stay exactly where they are, `cat-signal`'s manifest is
untouched, and everything new — the pipeline, the socket, the thread, the
transmit path, and the `AudioSource` trait — lives in `cat-signal-audio`.
A console renders frames with a dependency on `cat-signal` alone; only the
application that opens the link depends on this crate.

**The `AudioSource` trait lives here rather than in `cat-signal`**, even
though it would cost `cat-signal` nothing (it already has `async-trait`).
There is exactly one implementation, and a trait with one implementation in
a shared crate is a guess about the second one. Promoting it is a two-line
move the day a second audio source lands — a sound-card capture via `cpal`
is the obvious candidate — and that is the revisit trigger.

`cat-signal-audio` is plain `std::net` + `std::thread` like
`cat-transport-rfc2217`, so it target-gates nothing and its tests run on
every platform.

### 2. The reader thread does the DSP, and the slot holds finished frames

This is the one place this design deliberately departs from
[ADR 0014](0014-rtlsdr-spectrum-source.md) §2, which put raw IQ in the slot
and ran the FFT in the frame pump.

Doing that here would be a bug. If the console's rate gated the socket
reads, **the kernel receive buffer would become the unbounded queue**, and
the audio drawn would fall further and further behind real time. Moving a
queue out of our process and into the kernel does not make it not a queue;
it makes it a queue nobody can see or measure.

So the worker reads *and* windows *and* transforms, and hands over a
finished `AudioFrame`. The socket is drained at real time whatever the
console is doing, and staleness is bounded to one block.

The FFT is cheap here — 1024 points, 47 times a second — so this is about
where the queue lives, not about CPU.

### 3. FFT size 1024, Hann, and the scope window is the FFT block

- **Window: Hann**, the same as `cat-signal-rtlsdr`'s RF pipeline. Same
  reasoning, and the more important reason is that two pipelines in one
  console that answer "how wide is that signal" differently are a support
  problem. Selectable windows remain a `SettingDescriptor` away.
- **FFT size: 1024 by default**, settable from 256 to 4096, the same list
  the RF source offers.
- **The scope window is the FFT block**, and that is the decision most
  worth knowing about.

At 48 kHz, 1024 samples is 21.3 ms of audio, 46.875 Hz per bin, and 46.9
frames per second: a readable scope window, useful frequency resolution and
a comfortable console frame rate, all from one number. Raising `fft_size`
buys resolution and spends time resolution *and* frame rate. That trade is
real, it is the same trade the RF source exposes, and it is visible through
`AudioScopeFrame::window_ms()`.

The alternative — an independent scope block size — means two buffers, two
rates and two things to explain, in exchange for decoupling a knob nobody
has asked to set separately. If someone does ask, the shape that answers it
is a `scope_ms` setting that decimates or extends the captured block, and
nothing here forecloses it.

**Both frames come from the same samples and carry the same sequence
number.** A console cannot show a trace and a spectrum that disagree about
what the radio was doing.

Two arithmetic details, because both are silently wrong-able:

- The input is real, so only the first `N/2` bins carry information and
  bins `1..N/2` are **doubled** to account for the mirrored energy
  discarded with the upper half. Without that, every level in the display
  is 6 dB pessimistic — invisible until someone compares it to a real
  meter. A test asserts a full-scale sine reads 0 dB.
- Samples are scaled by **32767, not 32768**, and clamped. With 32768 a
  genuinely clipped capture sitting at +32767 reads 0.99997 and
  `AudioScopeFrame::is_clipping()` — the whole point of the field — never
  fires.

Reported span defaults to **4 kHz**, truncated from the full 24 kHz
half-spectrum. That is the useful part of a communications receiver's
audio; the other 20 kHz is empty and drawing it spends five sixths of the
display on nothing. The frame reports the span its bins *actually* cover,
not the span that was requested, so `bin_width_hz()` on the frame agrees
with the FFT that produced it.

### 4. Latest-frame backpressure, as ADR 0014 §3, and it matters more here

One slot, newest wins, drops counted and published as a read-only
`frames_dropped` setting.

The argument is stronger in the audio domain than it was for a waterfall. A
queue between a real-time producer and a slow consumer does not add latency
once — **it adds it forever**, because the producer never slows down. A
console showing audio from 400 ms ago, then 800 ms, then two seconds, is
worse than useless on a transmit monitor, and it degrades in a way that
looks like the radio misbehaving rather than the console.

Unlike `SpectrumFrame`, whose `sequence` `cat-signal-rtlsdr` stamps on
delivery, **`AudioFrame`'s sequence is stamped by the producer**. A dropped
frame therefore shows up as a gap in `sequence` as well as in the counter,
so a consumer can detect loss without reading a setting.

### 5. No cross-thread wake, deliberately

The handoff is `Mutex` + `Condvar`. **No waker is ever registered and no
task is ever woken from another OS thread**: `next_frame` blocks the calling
thread inside the condvar wait. This is the property `cat-signal-rtlsdr`
has, and it is why ADR 0016's monoio `sync`-feature trap has never applied
to it. Copying the shape carries the property over. **This crate requires no
feature flag from any caller.**

The price is named rather than hidden: `next_frame` *blocks*, and on a
monoio executor that is the executor thread — the same thread driving the
CAT session — for up to one block (21 ms at the defaults).

So the crate also offers **`try_next_frame`, which never blocks**, and the
documentation says plainly that a console driven by a render loop should
call it. A TUI ticking at 30 Hz polls; it does not await.

Rejected: a `futures` channel with a cross-thread wake. It would make
`next_frame` a genuine await, and it would put every monoio-based caller one
forgotten feature flag away from a panic that no test in this workspace can
catch. A design that needs a warning in its own crate docs is worse than one
that does not need the warning.

### 6. Losing the peer: terminal, with the last frame delivered first

When the socket ends or errors, the worker records why, wakes every waiter,
and stops. Then:

- **Any frame still in the slot is delivered before the error.** The last
  audio a radio sent before the link dropped is still worth drawing.
- After that the stream is **terminal**: every call returns the same
  `Closed { reason }` forever, rather than blocking on a producer that has
  gone. `is_connected()` and a read-only `connected` setting expose it, so
  an operator whose display went blank can tell "the radio stopped" from
  "the display stopped".
- **There is no reconnect inside the source.** Reconnect policy needs to
  know whether the radio is *meant* to be there — is this a restart, a
  deliberate shutdown, or a failed station? — and only the application
  knows. A source that silently reconnected would hide a station fault
  behind a gap in the waterfall.

**The two directions fail independently.** A dead PKD write does not end the
ANO stream, because they are separate syscalls on one socket and a half-open
link is a real state. A dead microphone must not look like a dead receiver.

`Drop` on the receive side sets a stop flag and shuts down **only the read
half**, which is what unblocks the reader thread — `cat-transport-rfc2217`
learned in ADR 0016 that a worker holding its own `Arc` means dropping the
handle frees nothing. Read-only, rather than the whole socket, because a
cloned transmitter may still be sending audio and killing that from the
receive side's destructor would be a surprise. The connection closes for
real when the last handle drops, and a test asserts the server sees it.

Note the contrast with ADR 0016's version of this hazard: there, a leaked
port left *a transmitter keyed*. Here it cannot, because —

### 7. The transmit path carries audio and keys nothing

`AudioTransmitter::send` puts samples on the radio's PKD pin. There is no
VOX, no PTT, no CAT command and no DTR anywhere in this crate. On the
station this was built for, keying is DTR through an opto-isolator
(`ts570d`'s SN-6), and it stays that way; audio arriving at a *receiving*
radio's PKD pin does nothing at all, which is the correct and safe outcome
of getting the order wrong.

This is said in the crate docs, the type docs and here, because a transmit
path that *looks* like it keys is the kind of thing someone tries once, into
an antenna.

Two smaller decisions inside it:

- **`send` blocks, and that is the right backpressure for this direction.**
  The peer consumes at real time, so a caller pushing faster than 48 000
  samples per second fills the socket buffer and waits. This is deliberately
  the *opposite* of the receive side's policy, for the same underlying
  reason: stale receive audio is worthless, and unsent transmit audio is
  not — dropping it would put a gap on the air.
- **Out-of-range samples are clamped and counted, never wrapped.** A wrapped
  sample turns a loud positive peak into a loud negative one: broadband
  splatter that the operator's monitor cannot hear. `send` returns how many
  samples it had to clamp, which is the only honest overdrive indicator a
  transmit path has — the operator's audio chain is upstream of this crate,
  and a console that can say "142 samples clipped" can tell them to turn it
  down.

`AudioTransmitter` is `Clone` and `Send`, so it can be handed to whatever
thread is capturing a microphone or generating a digital-mode tone; writes
from several clones are serialized so blocks never interleave mid-sample.

### 8. Tests drive a socket, because that is what will be plugged in

`dsp` and `pcm` are pure and tested by handing them arrays. None of that
proves anything about the wire. So `tests/acc2_wire.rs` runs a fake radio on
loopback speaking exactly the format above — no greeting, real-time pacing,
full duplex, reading PKD on its own thread — and every frame it asserts on
came off a socket.

The backpressure claim in particular is tested rather than asserted in prose:
each block is stamped with its own index, the consumer sleeps 200 ms, and
the frame it collects afterwards must be from the **live edge** of the
stream, not from block two. A queueing implementation fails it by a factor
of a hundred.

This crate cannot depend on `ts570d` — a consumer never becomes a
dependency — so the wire is re-stated in that file. If the two ever
disagree, that is a real defect, and that is where it should surface.

### Explicitly out of scope

- **Sound-card capture and playback.** `cpal` or equivalent, so a console
  can send the operator's microphone rather than a generated buffer. The
  `AudioTransmitter` API is the seam; it takes normalized `f32` blocks
  precisely so the thing producing them can be anything.
- **Modem/decoder work.** CW, PSK31 and FT8 decoders consume
  `AudioScopeFrame` samples and are not this crate's business.
- **Resampling.** The wire is 48 kHz and the frames report the wire's rate.
  A radio that streamed some other rate would work; converting between rates
  is a feature nobody has asked for.
- **Audio *routing*** — playing received audio out of a speaker. That is an
  application concern and drags in an output device.
- **Reconnect and stream health policy**, per §6.
- **Choosing the defaults by measurement.** Sensible defaults now; they are
  `SettingDescriptor`s, so tuning them is a settings change.

## Consequences

**Good.**

- `cat-signal` stays pure. `cat-ui` gains an audio renderer's data types
  with no new dependency, and nothing that merely *draws* a frame acquires
  an FFT, a socket or a thread.
- The console side of `ts570d`'s ACC2 pair exists, so the emulator's audio
  endpoint stops being a server with no client.
- **No caller needs monoio's `sync` feature because of this crate**, and
  that is by construction rather than by luck. The trap ADR 0016 documented
  cannot be walked into here.
- A slow console degrades to a lower frame rate with a visible counter and
  a visible sequence gap, at the live edge of the stream — never to growing
  latency, and never to an invisible queue in the kernel.
- The transmit path is honest about keying nothing, in three places.

**Costs and risks.**

- **`next_frame` blocks the calling thread.** On a monoio executor that is
  the executor. The mitigation is `try_next_frame` and documentation, and
  documentation is a weaker guarantee than a type. A console author who
  awaits `next_frame` in a monoio task will see CAT latency, not a panic —
  which is a milder failure than ADR 0016's, but a quieter one.
- **A thread per stream**, and it is detached rather than joined on drop:
  a worker blocked on a peer that has gone silent without closing would
  otherwise hang the console's shutdown. It ends on its next syscall.
- **Latest-frame is a policy, and someone will want the other one.**
  Recording received audio needs every sample, and this source will not give
  it to them. That is a recording sink on the worker side — a different
  feature, and not a reason to make the live path unbounded. The same
  sentence appears in ADR 0014 for the same reason.
- **The wire is re-stated in this repository's tests**, so it can drift from
  `ts570d`'s emulator. It is a dozen lines and a format with no options,
  which is the least bad version of that risk.
- **Nothing here has met a real radio.** The done-when for that is audio
  from an actual ACC2 jack through an actual sound interface; no fake server
  can prove the level, the impedance or which pin is which. This proves the
  software path, exactly as ADR 0014 §"done-when" said of the CN4 tap.

---

## Amendment (2026-09-02): a sound card is the second source

**Status: Accepted.** Additive. Nothing above is withdrawn — the socket
source, its wire, its backpressure policy, its terminal-close rule and its
transmit path are all unchanged, and `stream.rs` was not edited to make this
work. What changes is that "Sound-card capture", listed above under
*Explicitly out of scope*, is now in scope and implemented, and §1's
prediction — "a sound-card capture via `cpal` is the obvious candidate" for
a second `AudioSource` — has come true.

### 9. Why now, and what the revisit trigger actually decided

§1 said the `AudioSource` trait would stay in this crate until a second
implementation landed, and that promoting it to `cat-signal` would be a
two-line move on that day. That day is today, and the answer is **still no**.

`cat-signal` gained a `device` module — `DeviceKind`, `DeviceInfo`,
`DeviceList` — so a console can render sound cards and SDRs through one
picker without linking a driver for either. That is the vocabulary a
renderer needs. `AudioSource` is not: a renderer never calls `next_frame`,
only the application that opened the link does, and that application already
depends on this crate. Moving the trait would put an `async_trait` in
`cat-signal`'s public surface for the benefit of code that also depends on
`cat-signal-audio` anyway. The two implementations now live side by side in
the crate that owns them, and `AudioCapture` implements the trait, so
generic code needs no branch.

### 10. The `device` feature, default off — the same trade as ADR 0014 §5

`cpal` needs the platform's sound library: ALSA headers on Linux, and a
build that is not free on any of them. `cat-signal-audio` is depended on by
anything that wants to *draw* audio, and making all of that acquire a C
toolchain for a sound card the operator may not own is the same poor trade
[ADR 0014](0014-rtlsdr-spectrum-source.md) §5 refused for libusb.

**The cost is the same one too, and it is named rather than hidden:** code
behind a default-off feature is code CI does not compile, which is exactly
how the Windows defects in `planning/release_workflow/findings.md` §7
survived. Three mitigations, in descending order of how much they are worth:

1. **The gate is thin, deliberately.** The spec grammar, the enumeration
   *answer* (including the feature-off one), the ring buffer with all its
   invariants, the format-negotiation rule and the ring-capacity arithmetic
   are all **outside** the feature and all tested in the default build.
   What the gate hides is `cpal` calls, not logic. 16 of the 23 new unit
   tests run with the feature off.
2. The Linux CI job builds `--features device` explicitly, as it already
   does for `cat-signal-rtlsdr`.
3. `tests/sound_card.rs` runs against whatever sound hardware is actually
   present, and prints what it found or why it skipped — a test that
   silently passes because it did nothing is worse than one that says so.

A version note that is a decision, not a detail: **`cpal` is pinned to
0.16**, not the current 0.18. 0.17 raised its MSRV to 1.77 and 0.18 to 1.85;
this workspace declares `rust-version = "1.75"`. An MSRV that is only true
while a feature is off is not an MSRV.

### 11. Enumeration answers three things, not two

```rust
pub fn input_devices() -> cat_signal::DeviceList
```

`cat-signal`'s `DeviceList` keeps "found nothing" and "could not ask" apart,
because they send an operator to different places — one to their cabling,
the other to their build. This crate produces all three answers:

- a list, possibly empty, when the host answered;
- `unavailable("could not enumerate audio input devices: …")` when the sound
  service is not running or the driver failed;
- `unavailable("this build cannot see sound cards: rebuild … --features
  device")` when the feature is off.

That last one is why **`input_devices` is compiled in every build** and only
its body is gated. `cat-signal-rtlsdr` gates its whole `device` module, so
`devices()` does not exist at all without the feature; that is fine for a
diagnostic binary and wrong for a console picker, which is compiled once and
has to be able to explain an empty list. This is the one place this design
deliberately departs from that precedent.

### 12. `audio:<name>`, because one flag now takes two kinds of thing

`DeviceInfo::spec` is *exactly* the string `--acc2-audio` takes — that is
`cat-signal`'s rule, and the reason picking from a list is a shortcut for
typing rather than a second naming mechanism. But the flag already took a
network endpoint, and `hw:1,0`, `plughw:0`, `default` and `127.0.0.1:4533`
cannot be told apart by inspection without a heuristic that would have to do
DNS at parse time to be sure of itself.

So the grammar is explicit, and it is the shape `rtl:<index>` already has:

| Spec | Means |
|------|-------|
| `audio:<name>` | the local input device the host calls `<name>` |
| `audio:` | whatever the host considers its default input |
| anything else | a network endpoint, exactly as before |

Existing `--acc2-audio 127.0.0.1:4533` invocations are unaffected.
`AudioEndpoint::parse` lives in this crate, and **without** the `device`
feature, so that there is one grammar rather than one per console and so a
build that cannot capture can still say "that is a sound card, and this
build has no sound-card support" instead of trying to resolve it as a
hostname.

**The name in a spec is the driver's name for the device, not an ALSA id.**
`cpal` identifies a device by the string `DeviceTrait::name` returns and
offers no other handle — its ALSA backend keeps the openable PCM id
(`plughw:0`) private and reports the card's name ("HDA Intel"). Opening
therefore means enumerating and matching that name, which is also what makes
the same code correct on WASAPI and CoreAudio, where ALSA ids do not exist.
A consequence worth stating: two devices a driver gives the same name are
listed **once**, because a spec that resolved to the first while claiming to
be the second would be a choice that could not be honoured.

### 13. The capture reaches the existing pipeline through the seam that existed

`AudioStream::from_reader<R: Read + Send + 'static>` was already there. A
sound card does not offer a `Read`; it offers a callback that a real-time
thread invokes and that must return *now*. `ring::PcmRing` is the whole of
the adaptation:

```text
cpal callback ──push──▶ PcmRing ──Read──▶ AudioStream::from_reader
 (real-time thread)     (bounded,          (reader thread: DSP,
                         drop-oldest)       newest-wins frame slot)
```

Everything downstream — the DSP, the frame slot, the sequence stamping, the
terminal-close rule, `try_next_frame` — is the *same code* the socket path
uses, and therefore the same code the 15 wire tests already cover. Nothing
in `stream.rs` changed.

Three properties this ring has to have, and why:

- **It never blocks the producer.** A capture callback that waits does not
  merely add latency: on ALSA it is a high-priority thread that owns the
  device, and stalling it makes the *driver* drop samples, where nobody can
  see or count them. So a full ring discards its **oldest** audio and counts
  it — §4's policy, one layer down, for §4's reason. `samples_dropped` is a
  read-only setting, separate from `frames_dropped` because the two faults
  have different fixes: frames dropped means the console is slow and the
  display is merely at a lower rate, samples dropped means audio was lost
  before the DSP ever saw it.
- **Every push and every drop is a whole number of samples**, so the byte
  stream cannot shift by one and swap the halves of every sample after an
  overrun. That is the `rtl_tcp` bug §8's tests were written for, and here
  it is prevented structurally rather than carefully.
- **The end of the stream is never a plain EOF.** `stream::describe` maps
  `UnexpectedEof` to "the peer closed the connection", which is true of a
  socket and a lie about a USB codec that was unplugged. So the ring's
  `Read` always ends with an error carrying the driver's own words, and an
  unplugged card reads *"audio device stopped: DeviceNotAvailable"* — still
  terminal, still no reconnect, exactly as §6 requires and for §6's reason.

**§5's property survives intact.** `cpal`'s callback thread touches one
mutex and one condvar. No waker is registered and no task is woken from
another OS thread, so this crate still requires no feature flag from any
caller and ADR 0016's monoio `sync` trap still cannot be walked into here.

Dropping an `AudioCapture` closes the ring, and that is the *only* thing
that can wake a reader thread parked in `read_exact`. This is ADR 0016's
lesson again — a worker holding its own `Arc` means dropping the handle
frees nothing — and a console whose picker opens and closes devices would
otherwise leak a thread per pick.

### 14. What happens to a device that is not 48 kHz mono, because none of them are

The machine this was written on offers 44.1 kHz stereo f32 by default. The
rules, in full, and the reason each is not the alternative:

- **Sample rate: ask for 48 kHz, take the device's own rate if it has not
  got it, and tell the pipeline.** `AudioPipelineConfig::sample_rate_hz` is
  set to the *negotiated* rate before the stream starts, so every frame's
  `sample_rate_hz`, `window_ms()`, `bin_width_hz()` and the source's
  `max_bandwidth_hz` derive from what the card is actually doing. A 44.1 kHz
  card drawn as though it were 48 kHz would put every audio frequency 8.8%
  out — a 1000 Hz CW note reading 1088 Hz — which is exactly the kind of
  quiet wrongness this ADR's §3 arithmetic notes exist to prevent.
  `CaptureFormat::rate_substituted` says plainly that this happened, so a
  console can put it in a status bar.

  **Rejected: resampling.** It is still out of scope, for the reason given
  above: the frames report the wire's rate, and nobody has asked to convert
  between rates. **Rejected: refusing the device.** The card works
  perfectly well; refusing it would be a console declining to show audio it
  can see, in the name of a number it could simply report correctly.

- **Channels: take one, do not average.** The device's *own* channel count
  is requested — the driver is never asked to convert — and channel 0 (or
  `CaptureConfig::channel`) is extracted here. Asking ALSA's plug layer for
  one channel from a stereo card hands the decision to a layer nobody can
  see: it may take channel 0 or it may mix, and either way it is invisible.
  Averaging is worse still, because on a rig feed wired to one input it
  halves the level by 6 dB *and* mixes the other channel's noise into the
  spectrum, both silently. Taking the wrong channel gives silence, which an
  operator can see and fix in one keystroke. `channels` and `channel` are
  read-only settings so a console can show "2 ch, using ch 0".

- **Format: accept all of them, convert, and accept the 16-bit floor.**
  Every `cpal` sample format is converted to normalized `f32` and then
  quantized to 16 bits by the ring, because the ring feeds `from_reader`,
  whose format is the wire's. On a 24-bit or f32 card that is a −96 dBFS
  quantization floor — some 40 dB below any receiver's audio noise floor —
  and what it buys is one pipeline, one set of numbers, and a device capture
  that behaves identically to the socket capture the existing tests already
  cover. That is the trade, and it is the right way round for a display; it
  would not be for a recorder, which is a different feature (§4's last
  paragraph, again).

### Consequences of this amendment

**Good.**

- A console can capture a radio's receive audio from the sound card an ACC2
  lead is actually plugged into, not only from an emulator's socket. The
  done-when ADR 0014 and §8 both name — "nothing here has met a real radio"
  — is now reachable without writing any more code.
- A picker can offer sound cards and SDRs through one list and one flag,
  because both crates return `cat-signal`'s vocabulary and both specs are
  what their flag takes.
- The default build is unchanged: no new dependency, no new thread, no
  target gating, and the 42 tests that existed still pass unmodified.
- The failure modes are the ones already designed. An unplugged card is a
  terminal close with the driver's own words; a slow console drops frames at
  the live edge; a stalled reader thread is impossible because the producer
  cannot block.

**Costs and risks.**

- **A default-off feature is code CI does not compile.** Mitigated as §10
  says, and mitigated *partially*: the `cpal` calls themselves are only
  compiled by the explicit `--features device` job, and only exercised at
  all on a machine with a sound card.
- **`cpal` is pinned behind its current major.** 0.16 is what MSRV 1.75
  allows. When the workspace's MSRV rises past 1.85, this pin should be
  revisited; until then, raising it silently breaks a promise the manifest
  makes.
- **Enumeration omits a device that is busy.** `cpal`'s ALSA backend opens
  each card to list it, so a card another process holds exclusively does not
  appear. That is observable on the development machine: `HDA Intel` is
  listed when nothing else has it and absent while a PipeWire-routed capture
  is running. It is the driver's answer to "can this be used", which is the
  right question, but it means a list is a snapshot rather than an
  inventory.
- **Not every card `cpal` lists can be driven by `cpal`.** On the
  development machine, opening the raw `HDA Intel` card starts and then
  immediately fails inside cpal's ALSA backend with *"get_htstamp `0.0` was
  earlier than get_trigger_htstamp"* — a driver/backend quirk, not a defect
  here. What that proves is that the terminal-close path works against real
  hardware: the driver's own sentence reached the consumer as an
  `AudioError::Closed`, which is precisely what §6 asks for. The PipeWire
  and PulseAudio routes on the same machine capture correctly at 48 kHz.
- **Still nothing has met a real radio.** A sound card capturing a desktop's
  microphone proves the software path end to end and proves nothing about
  level, impedance, or which ACC2 pin is which. The done-when is unchanged.
