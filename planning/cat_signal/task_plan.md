# cat_signal — task plan

## Task (2026-09-01)

Build the **client half** of the ACC2 audio pair: a source that connects to
a radio's TCP audio endpoint (`ts570d`'s `--acc2-audio`) and produces the
audio-domain frames a console draws, plus a transmit path for PKD.

## The wire (fixed; not ours to redesign)

- One TCP connection, full duplex.
- 48 kHz, mono, signed 16-bit little-endian PCM, both directions.
- Server -> client is ANO (radio receive audio, ACC2 pin 3).
- Client -> server is PKD (transmit audio into the radio, ACC2 pin 11).
- Server paces to real time. Stream is continuous: a receiving radio always
  sends a noise floor; a transmitting radio sends digital silence.

## Plan

1. Read `cat-signal/src/{lib,audio}.rs`, `cat-signal-rtlsdr/**`, ADR 0014,
   ADR 0016 Consequences. (done)
2. Decide crate layout, FFT size/window, scope window, peer-loss policy.
3. New crate `cat-signal-audio`:
   - `pcm.rs`  — i16le <-> normalized f32, pure.
   - `dsp.rs`  — `AudioPipeline`: one block of samples -> `AudioFrame`
                 (`AudioScopeFrame` + `AudioSpectrumFrame`). Pure.
   - `stream.rs` — worker thread + single-slot latest-frame handoff +
                 `AudioStream` + `AudioTransmitter`.
   - `lib.rs`  — `AudioSource` trait, `AudioFrame`, errors, settings.
4. Tests: unit (pure DSP/PCM) + integration over a real loopback socket
   against a fake ACC2 server speaking the exact wire.
5. ADR 0017 + row in `docs/adr/README.md`.
6. Verify: `cargo test --workspace`, clippy `-D warnings`, `cargo fmt`.

## Constraints honoured

- No tokio. No monoio in this crate at all (plain `std::net` + `std::thread`,
  like `cat-transport-rfc2217`), so nothing target-gates.
- **No cross-thread wake**: the handoff is `Mutex` + `Condvar`, and the async
  `next_frame` blocks on the condvar rather than registering a waker. The
  ADR 0016 monoio `sync`-feature trap therefore does not apply — by design,
  not by luck.
- MSRV 1.75.
- Do not touch `cat-ui`, `cat-ui-ratatui`, `cat-native`, or `ts570d`.
- Do not modify `cat-signal`'s frame types.

---

## Task (2026-09-02) — a sound card as a second ACC2 audio source

ADR 0017 §1 named this as the revisit trigger for the `AudioSource` trait
("a sound-card capture via `cpal` is the obvious candidate") and listed
sound-card capture under *Explicitly out of scope*. It is now in scope.

### What is being added

1. A **default-off `device` feature** on `cat-signal-audio` pulling in
   `cpal`, exactly as `cat-signal-rtlsdr` does for librtlsdr (ADR 0014 §5).
2. **Capture** from a named input device, feeding the *existing* pipeline
   through the *existing* `AudioStream::from_reader` seam.
3. **Enumeration**: `input_devices() -> cat_signal::DeviceList`, present
   with and without the feature.
4. An **amendment** to ADR 0017 (new section, not a rewrite) and a refreshed
   row in `docs/adr/README.md`.

### Plan

1. Read `cat-signal/src/device.rs` (read-only — another agent owns it),
   `cat-signal-rtlsdr`'s `device` feature and `devices()`, ADR 0014 §5,
   ADR 0016 Consequences, and my own `cat-signal-audio`. (done)
2. Probe `cpal` on this machine: version, MSRV, real ALSA enumeration,
   whether `cpal::Stream` is `Send`. (done — see findings §9-§12)
3. `src/device.rs` (always compiled): the `audio:` spec scheme, the
   `AudioEndpoint` parser, `input_devices()` + its feature-off stub.
4. `src/ring.rs` (always compiled): a byte ring with a blocking `Read`,
   drop-oldest-whole-sample overrun policy, and a *reasoned* close. This is
   what turns a cpal callback into the `Read` `from_reader` already takes,
   and it is pure, so it is testable and tested without the feature.
5. `src/capture.rs` (feature-gated): format negotiation, channel
   extraction, `AudioCapture`, `AudioSource` impl, `Drop`.
6. Tests: pure ones in `ring.rs`/`capture.rs`/`device.rs` that run in the
   default build where they can; `tests/device_capture.rs` for the
   feature build, including a real open of this machine's default input.
7. ADR 0017 amendment + README row.
8. Verify: `cargo test --workspace`, `cargo test -p cat-signal-audio
   --features device`, both clippy runs, `cargo fmt --all`, and a real
   enumeration on this machine.

### Boundaries honoured

- Only `cat-signal-audio/**`, `docs/adr/0017-*`, `docs/adr/README.md` and
  `planning/cat_signal/**` are written. `cat-ui`, `cat-ui-ratatui`,
  `cat-signal-rtlsdr` and `cat-signal/src/device.rs` are **read only**.
- No tokio, no monoio, MSRV 1.75 (which is why `cpal` is pinned to 0.16 —
  see findings §10). No commits, no branches.
