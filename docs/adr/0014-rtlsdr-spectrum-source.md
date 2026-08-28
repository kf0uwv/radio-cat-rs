# 14. `cat-signal-rtlsdr`: a worker thread, a bounded latest-frame channel, and a Windows driver story

Date: 2026-08-28

## Status

**Proposed** — drafted for architect + user review. No code has been
written. Resolves the four items
[ADR 0010](0010-capability-model-and-normalized-signal-source.md) §5
deferred, and the `cat-signal-rtlsdr` requirement
[ADR 0012](0012-native-msvc-windows-target.md) §4 named.

## Context

[ADR 0010](0010-capability-model-and-normalized-signal-source.md) §5
specified the *behaviour* of an RTL-SDR spectrum source — read IQ, window,
FFT, magnitude, apply `IfTapConfig`, emit a `SpectrumFrame` — and
explicitly deferred four decisions:

1. the librtlsdr binding,
2. whether the FFT runs on a worker thread or in the frame pump,
3. backpressure policy when a consumer falls behind,
4. and, added by ADR 0012 §4, the Windows driver story.

The working spike is `ts570d/if-panadapter-bridge.py`, which drove gqrx's
`LNB_LO` over its remote-control port. It proved the arithmetic
(`LNB_LO = dial − 73.05 MHz` with IQ swap on) and, more importantly, proved
that **the dongle never retunes**: it stays parked on the IF, so its
crystal error is a fixed Hz offset rather than one that scales with
frequency. That is why `IfTapConfig::trim_hz` is a single calibrated
number and no ppm correction exists anywhere in this design.

Three constraints come from the rest of the workspace.

- **Tokio is banned**, and `cat-transport-core` is bound to `monoio` on
  Linux only ([ADR 0002](0002-async-runtime-binding-for-transport-crates.md),
  [ADR 0004](0004-windows-serial-backend.md)). `cat-signal`'s
  `SpectrumSource` is `#[async_trait(?Send)]` to match.
- **`cat-signal` has no dependencies beyond `async-trait`.** Keeping libusb
  out of every consumer of a `SpectrumFrame` is the entire reason this is a
  separate crate.
- **librtlsdr is a C library with a blocking, callback-driven read loop.**
  `rtlsdr_read_async` does not return until cancelled. It cannot be polled,
  and it has no async interface to adapt.

That last point is the crux: an inherently blocking C callback loop has to
meet a `!Send`, single-threaded async trait.

## Decision

### 1. Binding: `rtl-sdr` crate, wrapped, not exposed

Use the `rtl-sdr` crate (safe bindings over librtlsdr) behind a private
module. No librtlsdr type appears in this crate's public API.

Rejected: writing our own `bindgen` layer (real work, no benefit — the
existing bindings are thin and the C API is small), and shelling out to
`rtl_tcp` (an extra process, an extra socket, and it re-introduces exactly
the "some other program owns the dongle" coupling that retiring the gqrx
bridge was meant to remove).

### 2. A worker thread owns the dongle; the FFT runs there

**Not in the frame pump.** A dedicated `std::thread` **opens and owns** the
device handle, loops on a blocking read, and converts each transfer to
normalized IQ. The handle is created, used and dropped on that one thread
and never moved across a boundary — `RTLSDRDevice` wraps a raw pointer from
a C library, and relying on any thread-safety guarantee for it would be
borrowing trouble for no gain. It sends finished `SpectrumFrame`s to the async side.

Three reasons, in order of weight:

- **librtlsdr forces it.** Its read call blocks — `rtlsdr_read_async` until
  cancelled, and the `rtlsdr` crate's `read_sync` per transfer (which is
  what the binding actually exposes, so the implementation loops on it).
  Either way, calling it from a `monoio` task would wedge that task's
  entire executor thread — and on Linux that executor is also driving the CAT session, so a
  spectrum source would stall the radio it is supposed to be annotating.
- **The FFT is the expensive part.** At 2.4 MS/s a 2048-point FFT runs
  hundreds of times a second. Doing that on the executor thread is the same
  stall with a different cause.
- **It confines the `!Send` boundary.** The worker owns everything
  librtlsdr touches; the async side only ever sees an owned
  `SpectrumFrame`. Nothing `!Send` crosses the thread boundary, so the
  house `?Send` binding is untouched.

FFT via `rustfft`. Window: Hann, fixed for now — a selectable window is a
`SettingDescriptor` away if it is ever wanted, which is the point of the
delegated-settings design.

### 3. Backpressure: keep the newest frame, drop the rest, and say so

A bounded channel of **capacity 1** holding the most recent frame. When the
worker produces a frame and the slot is full, it **overwrites** it.

A waterfall is a live instrument. A consumer that fell behind wants the
current spectrum, not a queued one from 400 ms ago, and an unbounded queue
turns a slow consumer into unbounded memory growth plus rendering that
drifts steadily further from reality. Blocking the worker is worse still:
it would stall the USB read loop and cause librtlsdr to drop samples at the
driver level, where nobody can see it happen.

**Dropped frames are counted and exposed** as a read-only
`SettingDescriptor` (`frames_dropped`, group `Display`). `SpectrumFrame`
already carries a `sequence`, so a consumer can detect gaps itself — but a
number a user can see is what turns "the waterfall feels laggy" into "you
are dropping 80% of frames."

### 4. Windows: WinUSB via Zadig, vendored libusb, documented not automated

Per ADR 0012 §4 this is a platform port, not a `cfg` detail.

- **Driver.** The dongle ships with a DVB-T driver that must be replaced
  with WinUSB, conventionally using Zadig. This **cannot be automated by
  us** and must not be attempted: it rebinds a USB device driver
  system-wide, and doing that silently on a user's machine would be
  hostile. It is a documented setup step, and the source returns a
  specific, actionable error when it sees the wrong driver rather than a
  generic "no device."
- **libusb.** Linux gets it from the system package manager via
  `pkg-config`. Windows has no such path, so libusb is built from source by
  `vcpkg` and linked statically — one self-contained `.exe`, no DLL to
  ship beside it.
- **Gating.** `#[cfg(target_os = "windows")]` at the acquisition and
  linkage layer only. The FFT, the `IfTapConfig` correction, the channel,
  and the whole `SpectrumSource` implementation are platform-independent
  and must not be duplicated per platform.

### 5. The device layer is behind a default-off `device` feature

Added during implementation (Task 15), because it is a real decision and
not an implementation detail.

The DSP pipeline — window, FFT, magnitude, FFT shift, inversion, trim — is
always compiled and always tested. The librtlsdr worker is opt-in, because
building it needs libusb and librtlsdr headers, and making every consumer
of this workspace acquire a C toolchain to compile a crate they may not use
is a poor trade.

**The cost is named rather than hidden:** code behind a default-off feature
is code CI does not compile, and that is precisely how the Windows defects
in `planning/release_workflow/findings.md` §7 survived for as long as they
did. The mitigation is that the Linux CI job builds `--features device`
explicitly. It is a weaker guarantee than the Windows job's `check` +
`test`, and it is the honest one available: no CI runner has a dongle
plugged into it.

The split also buys the thing that matters most here — the corrections that
are easy to get *silently* wrong are testable with a synthetic tone rather
than a radio on the bench. A test asserts that a signal above the dial
renders to the right, that inversion is not a no-op, that DC lands in the
centre bin, and that trim is a constant offset at 3.5 MHz and at 28 MHz.

### 6. The correction, stated once

```text
center_hz = dial_hz + trim_hz          // the SDR is never retuned
bins      = if inverted { fft.reverse() } else { fft }
```

`retune(dial_hz)` changes **only** the reported centre. It issues no USB
control transfer. A test asserts that `retune` does not touch the device,
because the day someone "fixes" this by calling `rtlsdr_set_center_freq` is
the day the trim calibration silently stops meaning anything.

### Explicitly out of scope for this ADR

- **Other SDR hardware** (Airspy, SDRplay, HackRF). The `SpectrumSource`
  trait is the seam; each is its own crate. `DirectSdr` exists for them.
- **Audio-derived spectrum.** Blocked on the audio-stream design ADR 0010
  leaves out of scope.
- **Automatic `trim_hz` calibration.** A user measures it once against WWV.
  Automating it means detecting and identifying a known carrier, which is a
  real signal-processing feature, not a footnote.
- **Choosing the FFT size or averaging defaults by measurement.** Sensible
  defaults now; they are `SettingDescriptor`s, so tuning them is a settings
  change and not a code change.

## Consequences

**Good.**

- The blocking C loop is confined to one thread that owns everything it
  touches, so the workspace's `?Send` binding and its ban on tokio are
  untouched.
- A slow consumer degrades to a lower frame rate with a visible counter,
  rather than to unbounded memory or invisible driver-level sample loss.
- `if-panadapter-bridge.py` retires. gqrx stops being a required component,
  and the IF correction stops being duplicated between a Python script and
  the product.
- The Windows driver requirement is designed in rather than discovered
  during packaging — which is exactly what ADR 0012 §4 asked for.

**Costs and risks.**

- **A thread and a C library enter the process.** librtlsdr's error
  handling is C-shaped: device-gone conditions surface as return codes in a
  callback, and turning them into a clean `SpectrumSource::Error` without
  leaking a half-dead worker needs care.
- **`vcpkg` becomes a Windows build dependency**, and static libusb linkage
  is the least-travelled path in this design. If it proves unworkable, the
  fallback is shipping `libusb-1.0.dll` alongside the binary.
- **Capacity-1 overwrite is a policy, and someone will want the other
  one.** Recording averaged frames over time needs every frame. That is a
  different feature — a recording sink on the worker side — and not a
  reason to make the live path unbounded.
- **The done-when needs hardware.** ADR 0010's orientation claim can only
  be closed by a real capture against the CN4 tap: a signal above the dial
  must appear to the right. No fake source can verify that, and no CI job
  can either.
