# cat_signal — findings

## 1. `cat-signal` is pure and must stay that way

`cat-signal/Cargo.toml` depends on `async-trait` + `serde` and nothing else,
with a comment saying the separate `cat-signal-rtlsdr` crate exists precisely
so "consuming a `SpectrumFrame` never drags in libusb". The same argument
applies verbatim to an FFT, a socket and a thread. `cat-ui` renders
`AudioScopeFrame`/`AudioSpectrumFrame` and needs *only* those types.

=> New crate `cat-signal-audio`. `cat-signal` is not edited at all, so
`cargo test -p cat-signal`'s count is unchanged by this work.

## 2. How `cat-signal-rtlsdr` hands frames over (the pattern to mirror)

`cat-signal-rtlsdr/src/device.rs`:

- `Slot { buffer: Mutex<Option<..>>, ready: Condvar, dropped: AtomicU64,
  running: AtomicBool }`, shared by `Arc`.
- Worker thread does the blocking read; if the slot is already full it bumps
  `dropped` and **overwrites** — newest wins (ADR 0014 §3).
- The consumer side (`IqSource::read`) blocks on `Condvar::wait`.

**Crucially: no waker is ever registered and no task is ever woken from
another thread.** The `async fn next_frame` simply blocks the calling thread
inside the condvar wait. That is why ADR 0016's monoio-`sync` panic does not
apply to `cat-signal-rtlsdr`, and copying the shape carries the property
over.

## 3. One real difference from `cat-signal-rtlsdr`: where the DSP runs

`RtlSdrSource` keeps raw IQ in the slot and runs the FFT in the frame pump.
For a *socket* source that would be a bug: if the consumer's rate gates the
socket reads, the kernel receive buffer becomes the unbounded queue, and the
audio a console draws falls further behind real time forever — the exact
failure the task names.

=> In `cat-signal-audio` the worker does read **and** DSP, and the slot holds
finished `AudioFrame`s. The socket is drained at real time no matter what the
consumer does, and staleness is bounded to one block.

## 4. Sequence numbers are producer-side here

`SpectrumPipeline` increments `sequence` when a frame is *consumed* through
the pump, so an rtlsdr consumer sees no gap on a drop and must read
`frames_dropped`. Audio frames are stamped on the worker, so a dropped frame
shows up as a gap in `sequence` as well as in the counter. `AudioScopeFrame`
and its paired `AudioSpectrumFrame` share one sequence number.

## 5. Scaling: 32767, not 32768

Normalizing i16 by 32768 makes full-scale positive read 0.99997, so
`AudioScopeFrame::is_clipping()` (`peak() >= 1.0`) would never fire on a
genuinely clipped capture. Scaling by 32767 and clamping makes both
full-scale codes read exactly ±1.0, and round-trips exactly with the
transmit conversion `(x.clamp(-1,1) * 32767.0).round() as i16`.

## 6. Framing: `read_exact`, so the `rtl_tcp` I/Q-swap bug cannot recur

`rtl_tcp.rs` carries a hard-won `partial: Option<u8>` for reads landing
mid-sample. Reading a whole block with `read_exact` into a byte buffer of
exactly `2 * block` bytes makes a mid-sample boundary unrepresentable. A test
still drives a peer that writes 3 bytes at a time, because that is what
found the original bug.

## 7. Peer loss

`ts570d`'s emulator can exit, or the station's audio bridge can be
restarted. Decision: the stream is **terminal** on peer loss — the last good
frame is still delivered, then every call returns `Closed { reason }`
forever. No auto-reconnect inside the source: reconnect policy needs to know
whether the radio is *meant* to be there, which only the application knows,
and a silently reconnecting audio source hides a station fault behind a
gap in the waterfall.

The two directions fail independently (a dead PKD write does not kill ANO),
because they are separate syscalls on one socket and a half-open link is a
real state.

## 8. Transmit is audio only

`ts570d` SN-6: that station keys via DTR. Writing PKD samples moves audio
into the radio's ACC2 pin 11 and **keys nothing** — no VOX, no PTT, no CAT.
Said in the type docs, the crate docs and the ADR, because a transmit path
that looks like it keys is the kind of thing someone tests into an antenna.

---

## 2026-09-02 — sound-card capture

## 9. `cat-signal-rtlsdr`'s `device` feature is the precedent, and it is narrower than ours

`cat-signal-rtlsdr` gates its whole `device` **module** on the feature, so
without it `devices()` does not exist at all. That is not allowed here: the
task requires `input_devices()` to compile and answer `unavailable` in the
default build, because a console's picker is compiled once and must be able
to say *why* the list is empty. So the module is always compiled and only
its body is gated.

## 10. `cpal` version is an MSRV decision, not a taste one

Available: 0.15.3/0.16.0 declare `rust-version = 1.70`; 0.17.x wants 1.77;
0.18.x wants **1.85**. The workspace declares `rust-version = "1.75"` and
`cat-signal-audio` inherits it. 0.18.2 is what happens to be in the local
cargo cache, but pinning it would make the crate's own declared MSRV false
the moment anyone enables the feature.

=> `cpal = "0.16"`, MSRV 1.70, verified to build against the ALSA 1.2.15.3
headers on this machine. `default-features = false` costs nothing: on Linux
cpal 0.16 has **no** default features (jack/asio/oboe are all opt-in), so
enabling `device` pulls ALSA only.

## 11. cpal's ALSA device *name* is not an ALSA PCM id

`cpal-0.16.0/src/host/alsa/enumerate.rs` builds each non-builtin `Device`
with `name: card_name` ("HDA Intel") and a **private** `pcm_id`
("plughw:0"). `pcm_id` has no accessor. So `DeviceInfo::spec` cannot be an
ALSA device string like `hw:1,0` — cpal could not reopen it.

The only string that re-identifies a cpal device, on every backend, is
`DeviceTrait::name()`. So the spec carries that, and opening means
enumerating and matching the name. This is also what makes the same code
correct on WASAPI and CoreAudio, where ALSA-style ids do not exist at all.

## 12. `--acc2-audio` already takes `host:port`, so the device spec needs a scheme

`hw:1,0`, `plughw:0`, `default` and `127.0.0.1:4533` cannot be told apart by
inspection without a heuristic that does DNS at parse time. `cat-signal`'s
`DeviceInfo::spec` doc requires the spec be *exactly* what the flag takes,
so the flag needs an unambiguous grammar:

- `audio:<device name>` — a local sound card (`audio:` alone = host default)
- anything else — the TCP endpoint, unchanged.

`rtl:<index>` is the same shape, so a console parses all three the same way.
`AudioEndpoint::parse` lives in this crate so no console invents a second
grammar; it is available **without** the `device` feature, because parsing a
spec is not the same as being able to open one.

## 13. `cpal::Stream` is `Send` here, so no extra thread is needed

Compile-checked against cpal 0.16 on this machine. The ALSA backend already
runs its own high-priority callback thread; adding one of ours purely to own
a handle would be a second thread for nothing. `AudioCapture` therefore owns
the `cpal::Stream` directly. On a backend where `Stream` is `!Send`,
`AudioCapture` simply becomes `!Send` too — which the `?Send` house binding
(ADR 0002) already tolerates.

## 14. A device disappearing must not report "the peer closed the connection"

`stream::describe` maps `UnexpectedEof` to that sentence, which is true of a
socket and false of a USB codec being unplugged. So the ring's `Read` never
returns `Ok(0)`: it always ends with an `io::Error` carrying the real
reason, which `describe` passes straight through. An unplugged card reads
"audio device stopped: DeviceNotAvailable ...", and the stream is terminal
exactly as ADR 0017 §6 requires.

## 15. Real sound cards are not 48 kHz mono, and refusing them would be wrong

This machine's default input offers 44.1 kHz stereo f32.

- **Rate**: ask for 48 kHz if the device supports it; otherwise take the
  device's own default rate and set the *pipeline's* `sample_rate_hz` to it.
  No resampler (ADR 0017 already scopes resampling out), and nothing is
  misleading, because every frame reports the rate it was actually sampled
  at and `window_ms()`/`bin_width_hz()`/`max_bandwidth_hz` all derive from
  that number.
- **Channels**: prefer a mono config; if the device is multi-channel, take
  **one channel** (default 0, selectable) rather than averaging. Averaging
  a rig feed wired to one input halves the level by 6 dB *silently*;
  picking the wrong channel gives silence, which the operator can see and
  fix. `channels` and `channel` are read-only settings so a console can
  show "2 ch, using ch 0".
- **Format**: every cpal sample format is converted to normalized f32 and
  then quantized to i16 for the ring, because the ring feeds
  `AudioStream::from_reader`, whose wire is i16. The cost is a -96 dBFS
  quantization floor on a 24-bit or f32 card — 40 dB below any receiver's
  audio noise floor, and it buys one pipeline, one set of numbers, and a
  device capture that behaves identically to the socket capture the 15
  existing wire tests already cover.
