# cat_signal — progress

## 2026-09-01 — ACC2 audio client (ADR 0017). Complete.

### Built

New crate `cat-signal-audio` (added to the workspace members list). No
existing file was edited except `Cargo.toml` (one member line) and
`docs/adr/README.md` (one table row). **`cat-signal` is untouched.**

| File | What |
|------|------|
| `cat-signal-audio/Cargo.toml` | `cat-signal` + `async-trait` + `rustfft` + `thiserror`. No monoio, no tokio, nothing target-gated. |
| `src/pcm.rs` | i16 LE <-> normalized f32. Scaled by 32767 and clamped, so `is_clipping()` fires on both full-scale codes. |
| `src/dsp.rs` | `AudioPipeline`: one block -> `AudioFrame` (scope + AF spectrum, same samples, same sequence). Hann, 1024 default, one-sided doubling, -200 dB floor, optional spectrum-only averaging. |
| `src/stream.rs` | Reader thread doing blocking I/O **and** the DSP; single-slot newest-wins handoff; `AudioStream` (`AudioSource` impl, `try_next_frame`, `Drop`) and `AudioTransmitter` (PKD). |
| `src/lib.rs` | `AudioSource` trait, `AudioFrame`, `AudioError`. |
| `tests/acc2_wire.rs` | A fake radio on loopback speaking the exact wire; 15 tests. |
| `docs/adr/0017-acc2-audio-source.md` | The decisions and their reasoning. |

### Verification

- `cargo test --workspace`: **655 passed, 0 failed, 2 ignored.** (The 572
  baseline in the brief predates concurrent work in `cat-ui*` and
  `cat-transport-rfc2217`; `cat-signal-audio` contributes 42 — 26 lib, 15
  integration, 1 doctest.)
- `cargo clippy --workspace --all-targets -- -D warnings`: clean **except**
  one pre-existing error in `cat-ui-ratatui/src/af.rs:152`
  (`manual_range_contains`), a file another agent is editing right now and
  which this agent must not touch. `--exclude cat-ui-ratatui` is clean.
- `cargo fmt --all -- --check`: clean. (The `--all` run made no changes to
  the concurrently-edited `cat-ui*` files — their mtimes predate it.)

### Not done, and why

- **MSRV not verified by a 1.75 toolchain**: only `stable` is installed
  here. Every API used was checked by hand against 1.75 (`is_some_and` 1.70,
  `Duration::is_zero` 1.53, `let`-`else` 1.65, `f32::clamp` 1.50); nothing
  newer is used.
- **No real radio.** No fake server can prove level, impedance or which ACC2
  pin is which. This proves the software path, as ADR 0014 said of CN4.

## 2026-09-02 — sound-card capture + enumeration (ADR 0017 amendment). Complete.

### Built

All new work is inside `cat-signal-audio/`, plus one row in
`docs/adr/README.md` and an appended section in `docs/adr/0017-*`.
**`cat-signal`, `cat-ui*` and `cat-signal-rtlsdr` were not modified** — only
read. `stream.rs`, `dsp.rs` and `pcm.rs` are unchanged: the capture reaches
the existing pipeline through the `AudioStream::from_reader` seam that was
already there.

| File | What |
|------|------|
| `Cargo.toml` | Default-off `device` feature -> `cpal = "0.16"`, `default-features = false`. Pinned at 0.16 because 0.17 wants rustc 1.77 and 0.18 wants 1.85; the workspace declares 1.75. |
| `src/device.rs` (new, **always compiled**) | The `audio:<name>` spec grammar, `AudioEndpoint::parse`, `device_spec`, and `input_devices()` — the real one behind the feature, an `unavailable(...)` one naming `--features device` without it. |
| `src/ring.rs` (new, **always compiled**) | `PcmRing` + `RingReader`: bounded, drop-oldest, never blocks the producer, whole samples only, and a *reasoned* close so an unplugged card never reads as "the peer closed the connection". |
| `src/capture.rs` (new, feature-gated) | `AudioCapture` (`AudioSource` impl, `Drop`), `CaptureConfig`, `CaptureFormat`, `CaptureError`, format negotiation, channel extraction, `enumerate()`. |
| `src/lib.rs` | Module wiring, re-exports, and a header that now describes two sources rather than one. |
| `examples/enumerate.rs` (new) | Builds **with and without** the feature — the without case is the interesting one. |
| `tests/sound_card.rs` (new, feature-gated) | 7 tests against this machine's real sound layer. `CAT_AUDIO_DEVICE=audio:<name>` points them at a station's actual interface. |

### Judgment calls

1. **`input_devices()` is compiled in every build**, unlike
   `cat-signal-rtlsdr::device::devices`. A picker is compiled once and must
   be able to say *why* a list is empty.
2. **`audio:` scheme** on the spec, because `--acc2-audio` already took
   `host:port` and the two cannot be separated without DNS at parse time.
   `AudioEndpoint::parse` ships here so there is one grammar, not one per
   console, and it parses without the feature.
3. **Negotiated rate is written into the pipeline**, so a 44.1 kHz card
   reports 44.1 kHz everywhere rather than being drawn 8.8% out. No
   resampler; ADR 0017 already scoped that out.
4. **One channel, not an average.** The device's own channel count is
   requested and channel 0 (configurable) extracted here, so no invisible
   ALSA plug-layer decision and no silent 6 dB.
5. **i16 in the ring**, so the device path is byte-identical to the socket
   path the 15 wire tests already cover. Costs a -96 dBFS floor on an f32
   card, ~40 dB below any receiver's noise floor.

### Verification

- `cargo test --workspace`: **687 passed, 0 failed, 2 ignored.** (Baseline
  in the brief was 658; `cat-signal-audio` contributes +17 of the increase,
  the rest is other agents' concurrent work in `cat-ui*`.)
- `cargo test -p cat-signal-audio --features device`: **48 lib + 15
  acc2_wire + 7 sound_card + 2 doc = 72 passed, 0 failed.** Default build:
  42 + 15 + 0 + 2 = 59.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
  (The `cat-ui-ratatui/src/af.rs` error noted on 2026-09-01 is gone — the
  owning agent fixed it.)
- `cargo clippy -p cat-signal-audio --features device --all-targets --
  -D warnings`: clean.
- `cargo fmt --all`: applied; `--check` clean. It touched only
  `cat-signal-audio` files.
- **Real enumeration on this machine** (`cargo run --example enumerate
  --features device`), stable across three runs:

  ```
  AUDIO INPUT
  * default      spec: audio:default     2 ch, 44100 Hz default, f32, 48 kHz available
    pipewire     spec: audio:pipewire    2 ch, 44100 Hz default, f32, 48 kHz available
    pulse        spec: audio:pulse       2 ch, 44100 Hz default, f32, 48 kHz available
    HDA Intel    spec: audio:HDA Intel   2 ch, 44100 Hz default, f32, 48 kHz available
  ```

  Without the feature: `unavailable: this build cannot see sound cards:
  rebuild cat-signal-audio with --features device ...`

- **Real capture**: `audio:default` and `audio:pulse` both open and deliver
  frames — 48000 Hz, 2 ch (using ch 0), f32.

### Not done, and why

- **The raw `audio:HDA Intel` card starts and then fails inside cpal's ALSA
  backend** on this machine: *"get_htstamp `0.0` was earlier than
  get_trigger_htstamp"*. A backend/driver quirk, not this crate. It did
  prove the terminal-close path against real hardware — the driver's own
  sentence arrived as `AudioError::Closed`. The capture test treats a
  device-reported fault as a loud SKIP so a broken host sound stack cannot
  redden this suite.
- **Enumeration is a snapshot, not an inventory.** cpal opens each card to
  list it, so a card another process holds exclusively is omitted. Observed:
  `HDA Intel` disappears from the list while a PipeWire-routed capture is
  running in the same process.
- **MSRV still not checked by a 1.75 toolchain** (only `stable` here).
  Every new API was checked by hand: `io::Error::other` 1.74, let-else 1.65,
  inline format args 1.58, `strip_prefix` 1.45; `cpal` 0.16 declares 1.70.
- **No real radio, still.** A desktop sound card proves the software path
  and proves nothing about level, impedance or which ACC2 pin is which.
