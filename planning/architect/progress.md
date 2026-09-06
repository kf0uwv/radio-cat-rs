# Progress — architect

## 2026-07-16 — Planning pass: extraction + TCP/UDP/server authorized

Status: **planning complete, no code written** (per architect's constraints —
no Rust/Cargo, no `cargo init`, no edits to `ts570d`/`ft991a`).

Delivered:
- `docs/adr/0002-async-runtime-binding-for-transport-crates.md` — new ADR
  resolving the monoio open item ADR 0001 carried forward: retain `monoio`/
  `#[async_trait(?Send)]`, extend to all transport crates + `cat-server`,
  explicit revisit trigger recorded (Windows serial support, or a non-monoio
  runtime consumer).
- `docs/adr/0001-scope-and-crate-boundaries.md` — amended (not rewritten)
  with an "Amendments" section: monoio item resolved (pointer to ADR 0002);
  `cat-transport-core` dependency corrected (depends on `cat-framework`, not
  nothing, for `ResponseDisposition`/`ProtocolErrorKind` reuse);
  `cat-transport-serial` scope clarified to include `ts570d/serial/`'s
  concrete io_uring implementation; `framework::state_machine` scoped out
  entirely.
- `docs/adr/README.md` — ADR 0002 row added; repository status updated to
  reflect extraction being authorized and the dispatch queue existing.
- `planning/architect/findings.md` — full reasoning: monoio decision,
  workspace/Cargo-dependency-mechanism decision (git dependency, `origin`
  already configured, `file://` fallback noted), code-motion decision (clean
  copy-and-adapt, not git-subtree/filter-repo, with commit-hash citation as
  the provenance substitute), plus real findings from reading `ts570d`'s
  actual code that go beyond ADR 0001's paraphrase (§5 `cat-client`/
  `RadioClient` isn't generic yet; §6 `cat-transport-serial` needs
  `ts570d/serial/`; §7 the dependency correction; §8 `state_machine`
  exclusion; §9 `errors.rs` split; §10 wire-framing coordination between
  Task 4 and Task 5).
- `planning/architect/task_plan.md` — full dispatch queue: Task 1
  (`cat_framework`: `cat-framework`) → Task 2 (`cat_transport`:
  `cat-transport-core` + `cat-transport-serial`) → Task 3 (`cat_framework`:
  `cat-client`, flagged as design work not a pure move) → Task 4a
  (`cat_transport`: `cat-transport-tcp`) → Task 4b (`cat_transport`:
  `cat-transport-udp`) → Task 5 (`cat_server`: `cat-server`). Ordering
  rationale and cross-task dependencies recorded per task.

Not done, and explicitly not silently dropped:
- Migrating `ts570d` itself onto these crates once they exist — a separate,
  later planning pass (see `findings.md` §4). `ts570d`/`ft991a` were not
  touched.
- No task has been dispatched yet. Per the standing "one task at a time,
  review before the next" workflow, Task 1 is next, pending user review of
  this planning pass.

## Next action (superseded by the entry below for the Windows planning pass;
kept for history)

Await review of ADR 0002, the ADR 0001 amendments, and the task_plan.md
dispatch queue. On approval, dispatch Task 1 to the `cat_framework` agent.

## 2026-07-19 — Planning pass: Windows serial backend (ADR 0002's revisit trigger fired)

Status: **planning complete, no code written** (same constraints as every
prior pass — no Rust/Cargo, no edits to `ts570d`/`ft991a`).

Trigger: the user wants real Windows COM-port control of a physical FT-991A
from a native Windows `ft991a` build (and eventually `ts570d`), with
explicit direction to keep `monoio`/io_uring unchanged for Linux and add a
genuinely separate Windows backend inside `cat-transport-serial` — not a
runtime-agnostic redesign touching the Linux path.

Read before deciding (not touched, reference only): `ft991a/ui/src/
terminal.rs`, `ft991a/src/main.rs` (single sequential loop, no
`monoio::spawn`), `ts570d/ui/src/terminal.rs` (genuine concurrent two-task
design via `monoio::spawn` + channels, for UI responsiveness during slow
polls), plus this repo's own `cat-transport-serial/src/{io_uring.rs,lib.rs,
session.rs}`, `cat-transport-core/src/{transport.rs,modem.rs,errors.rs}`,
and ADR 0002/0003 in full.

Delivered:
- `docs/adr/0004-windows-serial-backend.md` — new ADR. Async-execution
  decision: a dedicated background OS thread doing blocking Win32
  `ReadFile`/`WriteFile`, paired with a small hand-rolled single-slot
  completion primitive (not blocking-in-async-fn as the general mechanism,
  not a third async-runtime crate) — reasoning tied directly to `ts570d`'s
  concurrent two-task architecture, which a naive blocking implementation
  would silently break on a future Windows port even though it would be
  harmless for `ft991a`'s simpler single-loop shape today. Crate/module
  structure: same `cat-transport-serial` crate, same public type names
  (`SerialPort`/`SerialConfig`/`Parity`/`FlowControl`), `#[cfg(target_os =
  "windows")]`-gated internals in a new `windows.rs` alongside the existing
  `io_uring.rs`, with `SerialConfig`/`Parity`/`FlowControl` extracted into a
  new shared, ungated `config.rs` (behavior-preserving move, not a
  duplication) — not a new crate. Full `SerialConfig` ↔ `DCB` field mapping
  table (every field maps cleanly; one deliberate cross-platform
  consistency choice flagged — reusing Linux's validated baud-rate set on
  Windows even though `DCB.BaudRate` itself is more permissive). Win32
  dependency: `windows-sys`, target-gated exactly like the existing
  Linux-gated `monoio` entry. `ModemControlLines` maps to direct,
  synchronous `EscapeCommFunction`/`GetCommModemStatus` calls, mirroring
  ADR 0003's "no I/O wait" precedent exactly.
- `docs/adr/0002-...md` — small appended "Amendment" section (not a
  rewrite) pointing at ADR 0004 as the resolution of the revisit trigger
  ADR 0002 itself named.
- `docs/adr/README.md` — ADR 0004 row added; a short paragraph added
  pointing at the new dispatch queue.
- `planning/architect/findings.md` §11 — the supporting research: what was
  read in both consuming repos and why the asymmetry between `ft991a`'s and
  `ts570d`'s UI architectures drove the async-execution decision; why
  option 3 (a third runtime) was rejected in concrete terms; explicit note
  that `monoio` is never in the picture on the Windows side at all, which
  is what makes the hand-rolled completion primitive's correctness rest on
  `std::task::Waker`'s ordinary contract rather than on `monoio`
  internals.
- `planning/architect/task_plan.md` — Tasks 6, 7, 8 appended (`cat_transport`
  agent, sequential): Task 6 extracts `config.rs` and adds the portable
  `oneshot.rs` completion primitive (real `cargo test` coverage, since it
  has no OS dependency); Task 7 implements Windows `SerialPort::open`/`DCB`
  configuration/`SetCommTimeouts` only; Task 8 implements
  `Transport`/`ModemControlLines` plus the worker thread. Verification
  boundary stated explicitly and differently from every prior task in this
  file: this sandbox is Linux-only and cannot execute Windows binaries, so
  Tasks 7–8's "done when" is `cargo check --target x86_64-pc-windows-gnu`
  compiling cleanly, not `cargo test` — matching the user's own statement
  that they will validate against real hardware.

Not done, and explicitly not silently dropped:
- No task dispatched yet — same one-task-at-a-time, review-before-next
  workflow as every prior pass. Task 6 is next, pending review of ADR 0004
  and this task_plan.md addition.
- `ft991a`'s and `ts570d`'s own Windows entry-point work (replacing
  `#[monoio::main]`, since `monoio` cannot compile on Windows at all) is
  recorded as a needed follow-on in ADR 0004 §1 but is explicitly out of
  scope for this repository's dispatch queue — a future planning pass in
  each of those repos, gated on Task 8 landing here and on those repos'
  own architects picking it up, not something this session touched or
  authorized.

## Next action

Await review of ADR 0004 and the Task 6–8 additions to `task_plan.md`. On
approval, dispatch Task 6 to the `cat_transport` agent.

## 2026-08-27 — ADRs 0010-0013 Accepted; Task 10 executed

User sign-off ("accept and start"). Status flipped to **Accepted** on
`radio-cat-rs` ADRs 0010 (capability model + normalized signal), 0011 rev 4
(`cat-ui` for both renderers), 0012 (native MSVC), 0013 (renderer parity),
and `ts570d` ADR 0008 (GPU `gui` crate). Both ADR indexes updated. The
`ts570d` and `ft991a` `CLAUDE.md` pending-amendment blocks are now in force
as direction — with the explicit caveat, written into both, that Rules 1-7
still describe and govern the current code because none of the migration
has been written; what acceptance forbids is *new* code entrenching the
superseded framing.

**Task 10 (`release_workflow`) executed** — see
`planning/release_workflow/{findings,progress}.md`. Config and docs
complete; both `windows-check` jobs now run `cargo check` + `cargo test` on
`windows-latest`. ADR 0012's caveat 1 (GPU crates under `cargo-xwin`)
closed affirmative by measurement, caveat 2 (Microsoft licence) narrowed to
developer machines and left for the user, caveat 3 (the local check cannot
run tests) newly recorded. One stale-scope bug found and fixed: `ft991a`'s
CI had excluded `server` from Windows verification for a month after the
upstream gap that justified it had closed.

**Outstanding on Task 10:** no `windows-latest` run has happened — these
repos are not pushed from here. That run is where ADR 0006 §4's
never-executed Windows tests finally execute.

**Next:** Task 11 (`cat-framework::capabilities`), unblocked.

## 2026-08-28 — Tasks 12-18 complete; queue stops at the widget sets

All library work in the planning pass is done and pushed to
`feat/msvc-windows-target-adr-0012` (PR #1). `ts570d` and `ft991a` PRs are
merged.

| Task | Outcome |
|---|---|
| 12 `cat-signal` | Types + trait + `FakeSpectrumSource`. Low-frequency-first invariant asserted, not documented. |
| 13 **GATE** | **PASS**, one strain recorded (`MenuCapability`). See `findings.md`. |
| 14 ADR 0014 | RTL-SDR source: worker thread, newest-wins backpressure, WinUSB story. |
| 15 `cat-signal-rtlsdr` | DSP fully tested; device layer behind a default-off feature, built by CI. |
| 16 native protocol | Handshake, capability-validated commands, binary spectrum frames. |
| 17 `cat-rigctl` | `\dump_state` generated from capabilities; **verified against live Hamlib 4.6.5**. |
| 18 `cat-ui` | Renderer-agnostic. Seam drawn one notch sharper than planned — see below. |

**Verification:** Linux 336 passed / 0 failed; Windows 11 MSVC 262 / 0, on
real hardware (`radiombf`) as well as CI.

### Decisions taken during implementation, for review

1. **`cat-ui` does not contain `mini_bar`/`smeter_bar`.** The extraction
   list named them, but they return a `String` of block characters — a
   terminal rendering, not renderer-agnostic logic. What is shared is the
   *fraction*; the glyphs belong in `cat-ui-ratatui` (Task 20) and the
   rectangle in `cat-ui-egui` (Task 19). Including them would have made the
   renderer-agnostic crate a terminal crate the GPU renderer worked around.

2. **`cat-signal-rtlsdr`'s device layer is behind a default-off feature.**
   Building it needs libusb and librtlsdr headers. The cost — code CI does
   not compile — is named in ADR 0014 §5 and mitigated by a CI step that
   builds `--features device` explicitly.

3. **ADR 0014 was corrected during implementation.** It was drafted against
   an `rtlsdr_read_async`-shaped API; the `rtlsdr` crate exposes a single
   device with blocking `read_sync`. Found by compiling, not by reading.
   The worker-thread reasoning is unaffected.

4. **`RigctlRadio::capabilities()` defaults to `None`.** An unmigrated
   radio keeps its historical `\dump_state` reply byte-for-byte, pinned by
   a test. The compatibility layer must not change under radios that did
   not ask for anything.

### Corrections to earlier claims in this planning pass

- Task 17's commit message states "Linux 322 passed". The actual figure was
  **298**. The tests and the verification were real; the number in that
  message was not.
- Task 17's live-Hamlib tests were described as skipping "loudly" when
  `rigctl` is absent. `eprintln!` from a passing test is captured by the
  harness, so on Windows they passed silently without running — the exact
  thing that commit called worse than no test. Fixed in Task 18's commit:
  the skip is a hard failure when `CI` is set, and the Linux job installs
  `libhamlib-utils`.

### Where this stops, and why

**Tasks 19 (`cat-ui-egui`) and 20 (`cat-ui-ratatui`) are not started.**
Both are widget sets, and a widget set is a design artifact as much as a
code one: what a meter rail looks like, how a band grid is arranged, what
the absent-capability state shows. `ts570d/.claude/agents/designer.md`
exists to answer those, and its brief runs in parallel precisely so this
moment does not become a blocked one.

The library work they build on is finished and verified, so the design
track is now the critical path.

### Still outstanding, and user-owned

- **A live capture against the CN4 tap.** ADR 0010's orientation claim is
  asserted in software against a synthetic tone; only real hardware proves
  the physical tap is wired as assumed.
- **A calibrated `trim_hz`** measured against WWV.
- **An IC-7100 manual.** ADR 0010 asked for three radios described; two
  are. Nothing has been tested against a binary CI-V radio.
- **App-side migration** (`ts570d`/`ft991a` onto the native protocol and
  `cat-ui`, deleting both `rigctl_radio.rs`), which is its own per-repo
  pass and was read-only to this queue throughout.

## 2026-09-02 — remote device enumeration landed

Done, verified live against the emulator + `ts570d server`:

- `cat-native`: `Command::ReadDevices` / `AttachDevice`,
  `ServerMessage::Devices { lists }`, `publish_devices`, client
  `read_devices()`/`attach_device()`, `testing::StubHost` + `serve_stub`.
- `cat-signal`: `DeviceDirectory` (list + attach in one trait).
- `cat-rigctl`: `NativeShared::with_devices`; an attach is answered from
  the directory and never queued to the radio's CAT link.
- `ts570d`: `ServerDevices` in the wiring layer; `IfSelection` makes the
  server's IF source swappable at runtime; GUI SOURCE tab is a real
  picker over the *server's* list.
- ADR 0018; parity rows for the TUI's `--server` gap and remote audio.

Two bugs found on the way, neither of which had a failing test:

1. **`ServerMessage` is internally tagged**, and serde cannot tag a
   newtype variant holding a sequence. `Devices(Vec<..>)` failed at
   *serialization*, so the only symptom was the server dropping the
   connection — indistinguishable from a network fault. Now a struct
   variant, with a round-trip test that fails saying so.
2. **`server/src/spectrum.rs` still asked for 96 kHz**, the impossible
   RTL-SDR rate fixed everywhere else weeks ago. It survived because it
   hand-built the pipeline instead of calling `cat_signal_rtlsdr::open`,
   and because rtl_tcp is a socket that serves any rate asked of it. The
   server now goes through `open`, which validates first.

## 2026-09-02 — audio over the native protocol (ADR 0019)

The last recorded gap is closed. `FrameKind::Audio` carries a whole
`AudioFrame`; `ts570d server --acc2-audio` publishes the ACC2 pair; a
console attaches one of the server's sound cards from the picker and draws
both AF panels from it.

Verified full-stack: emulator -> `ts570d server` -> `ts570d --server` on
one screen showing 17.5k waterfall glyphs, 750 AF braille glyphs, both AF
panels LIVE, `:f 14.074` moving the real radio, and the server's own
device list on the SOURCE tab.

Two moves the design forced, both improvements on their own:

- **`AudioFrame` and `AudioTap` moved down.** The frame is a data pair and
  now has to be named by `cat-native`, which must not depend on a crate
  that can link cpal. The trait moved for a sharper reason: a server
  *capturing* audio needs the same abstraction as a console *drawing* it,
  and it had been living in the terminal UI crate.
- **`Streams { spectrum, audio }` replaced the bare bool.** Two bools in a
  row is the argument order nobody gets right, and the two are genuinely
  independent opt-ins.

One bug, found by a socket-level test and invisible to the codec's own:
**adding a variant to `FrameKind` did not add it to `from_u8`**, so the
server sent frames the client rejected as unknown — and the symptom landed
on the receiver as a broken stream while the sender looked fine. Now
pinned by a test that walks every variant.

## 2026-09-02 — the GUI's AF panels (parity closed)

Both renderers now draw both AF panels, from a local source or a remote
one. Verified by rendering: a still with a 800 Hz tone shows the wave, the
FFT hump, and both amber filter marks; `NO_AUDIO=1` shows the empty state
keeping its size, its zero line and its marks.

The arithmetic moved to `cat_ui::af` rather than being written twice — the
fixed 3 kHz axis, the 40 dB window below peak, the passband, the
resolution cap. Two consoles showing the same radio with different bar
heights would leave an operator unable to trust either.

Three things the work turned up:

- **`passband_for` is now capability-derived.** The TUI's version read a
  TS-570D table in the `radio` crate, which the GUI may not depend on.
  Deriving it from the published `ModeDescriptor` is both shareable and
  more correct: the mark is the bandwidth *that* radio declared.
- **The server never published an `Installation`.** `serve_one` always
  built the session with the default, so `CapabilitiesWire.installation`
  was empty on every connection — the SOURCE tab's attached list could
  not fill, and a console could not tell "nothing wired" from "nothing
  yet". `RadioHost::installation` now asks the host, per connection, so an
  attach mid-session reaches the next console.
- **A first test asserted something false.** I wrote that a fixed window
  keeps a quiet panel low; it does not — the window is anchored to the
  peak, so a quiet panel reads flat and *high*. The true guarantee is that
  a small spread stays small. I corrected the test, not the code, and said
  so in the test.

## 2026-09-02 — the console stack ported to the other radios

**`ft991a` has a GUI, and it is not a copy.** The measurement that decided
the shape: `ts570d/gui/src/app.rs` was 1442 lines with *one*
radio-specific mention in it. So the console moved into `cat-ui-egui` and
each app supplies a window — `ts570d/gui` went from ~1700 lines to 296,
and `ft991a/gui` is 300 including its demo fixture.

The lift is pixel-identical: the offscreen still before and after compares
with no differing region at all.

**The FT-991A's own features arrive without an FT-991A console existing.**
Its still shows fourteen modes, 151 menu items, five meters, a FILTER
control and a NOTCH the Kenwood does not get, memory numbered from 1, and
**no SPECTRUM workspace** — because the radio declares
`SignalSupport::None`. All derived from the capability document.

**Both listeners on both radios**, verified answering at once: rigctl
returning `14000000`/`USB` on one port while the native protocol
identified the radio on another.

Two things found on the way:

- **`demo_state` hardcoded "connected to Kenwood TS-570D".** Harmless
  while one radio had a console; a still of an FT-991A captioned with a
  Kenwood the moment a second did. Now reads the model it was handed.
- **`ft991a` could not be patched onto this checkout.** It pinned tag
  `v0.3.0`, and a `[patch]` only substitutes across a *matching* version,
  so every patch silently went unused and the build failed on missing
  modules rather than on the real cause. Switched to path dependencies,
  which say the same thing without the indirection.

**`ic7100` was not started, and should not be.** Its own ADR 0001 records
three preconditions: `CivFormat` is unwritten and behind an unmerged,
untagged branch; the CI-V reference guide is not in the repository; and
that ADR is explicit that inferred web research must not stand in for it.
A CI-V driver written from guesses is how a radio ends up keyed when
nobody asked.

## 2026-09-02 — the terminal console shared too

`ft991a` now draws the option-3 console. The measurement that decided the
shape, again: `ts570d/ui/src/console.rs` was 1805 lines with **two**
radio-specific parts — a meter table and a mode-label lookup — and the
capability document already answers both.

So `cat-ui-ratatui::console` and `cat_ui::display::RadioDisplay` are
shared, and `ft991a/ui` reaches them through a ~40-line conversion from
its own `Ft991aDisplay`. Its console shows what an FT-991A is: its own
meters, MEMORY 1–117, MENU 151, and **no SPECTRUM tab**.

Three things worth keeping:

- **The label lookup is gone, not moved.** Parsing "CW" back into a mode
  works for one radio's spelling. `RadioDisplay` carries `mode_id` beside
  the label now, and the passband reads the width the radio published.
- **`MeterReading::from_wire`.** Both renderers had written meter scaling
  out separately — one path now, so a 0-30 S-unit table and an
  uncalibrated 0-255 cannot be drawn to each other's scale.
- **`mode_id` had to be filled or the passband marks would have vanished
  silently.** Nothing would have failed; the AF FFT would just have lost
  its amber edges. Filled in both the poll loop and the still renderer.

**Pre-existing and not mine, but worth fixing:** both apps call
`tracing_subscriber::fmt()` writing to stdout, and the log lines land on
top of the alternate screen while the console draws. Verified pre-existing
by stashing the change and reproducing it against `ft991a`'s old layout.
It makes both consoles look broken at startup and is a small fix — routing
the subscriber to a file or to stderr — but it changes logging behaviour
for two apps, so it is reported rather than done in passing.

## 2026-09-03 — the GPU console follows the radio too

Layout and palette both. `ts570d-gui` and `ft991a-gui` now resolve the
arrangement their server published and paint it in that radio's colours.

**The look is grounded in the hardware.** The TS-570D is amber on charcoal
because that is its LCD on its case; the FT-991A is blue and cyan because
that is its TFT. With both on a bench, which console is which is legible
before a label is read.

**The closed vocabulary was the wrong call and is corrected.**
`PanelKind::Custom(name)` plus a painter registry, with the base building
blocks public. A console without the painter draws the panel's *name* —
"no widget for ft991a.clarifier" is a different message from a blank
rectangle, and a pure network console will always be in that position
because it has not linked any radio's crate.

Three things found while doing it:

- **The spectrum drew twice** once a layout gave it a panel of its own:
  the workspace's SPECTRUM tab drew a second waterfall of the same signal,
  which reads as two receivers.
- **The status line drew twice**, because the command row had carried it
  back when it was the only row at the bottom.
- **Cells do not translate one-for-one between renderers.** The readout
  needed five cells rather than three for the GPU console's taller strip.
  The units are right — proportions match — but a panel's density is not
  the same in both, and a layout has to fit the roomier one.

**The waterfall keeps Turbo deliberately.** Its ramp encodes magnitude, so
recolouring it to match a front panel would cost signal legibility for an
aesthetic gain. Worth revisiting only if an operator asks.

## 2026-09-03 — the retune camera move, and a meter that agrees

**Clicking a signal is a camera move now, not a repaint.** The waterfall's
history already knew its own frequencies and could be redrawn from any
vantage point (`rebuild`); what was missing was the continuity — the fact
that this picture is the previous picture seen from somewhere else.

`cat_ui::retune` eases the view's centre from the old dial to the new and
dips the span on the way, a dolly toward the signal rather than a cut.
`rebuild_onto` reprojects the whole history at every step, so a carrier the
operator clicked curves into the middle of the screen instead of
teleporting there. It ends at the ordinary span: a console left zoomed in
would need a control to get back out, and inventing one to undo an
animation says the animation went too far.

Two things fixed while wiring it:

- **The waterfall scrolled on every repaint**, pushing the same row over
  and over, so a source that had stopped went on looking live. It advances
  on a new frame now — the same rule the audio path already had.
- **The image advanced only while its panel was visible**, so switching
  tabs put a hole in the history an operator would later scroll back to.

**The emulator's S-meter now reads the band it is transmitting.** It
answered a constant 10, so a console could show a carrier filling its
waterfall while the needle sat at S5 and nothing anywhere would notice —
which made the emulator useless for the one thing a console most needs
tested, that its instruments agree with each other.

It reads the peak inside a 3 kHz passband from the same `Band` the tap and
the ACC2 audio render, mapped through *this radio's own* `SUnitScale`
table rather than a formula. Verified over CAT: sweeping 20 m gives 8–27
raw with peaks on the actual emitters, CW keying visible as 24/27
alternation and SSB moving with its speech envelope.

Two errors of mine on the way:

- The meter first went in the tap's client loop, so the needle only moved
  for an operator who happened to have a waterfall open. A radio's meter
  is a property of the radio; it has its own thread now.
- `raw_for_dbm` folded every non-finite value to zero, so an infinite
  signal read as a dead band — the wrong way round for anything that might
  be a fault. NaN reads zero; an infinity pegs.

## 2026-09-05 — the IC-7100 is complete

Radio, emulator, client, server and both consoles. Verified end to end:
rigctl answers WSJT-X (`14100000`/`USB`), the console protocol publishes
this radio's own layout, and both consoles draw it — amber on black,
MEMORY 1–99, MENU 112, no spectrum, bands from 160 m to **70 cm**, and
**DV** among the modes.

Four things in the shared stack had quietly assumed ASCII, and a binary
protocol found all four by being run rather than read:

- **`CatClient` returned `String`** — `from_utf8_lossy` destroys a CI-V
  frame.
- **The broker did the same**, so every rigctl read answered `RPRT -1`.
- **`SerialCatSession` read until `;`** — hardcoded, so it waited forever
  for a byte CI-V never sends. `FrameScanner` had existed since ADR 0009
  and nothing had used it.
- **`cat-rigctl`/`cat-server::build` were pinned to `AsciiLineFormat`.**

And one thing that was wrong for **every** radio: the terminal console's
band and mode rows were hardcoded to the TS-570D's nine HF bands and six
modes, while the GUI derived both from capabilities. An FT-991A was
showing six of its fourteen modes. Both derive now.

Worth naming for next time: the byte-order trap in `civ` (frequency is
little-endian, everything else big-endian) was found by writing
`examples/wire` and *reading the bytes against the manual* — not by a
test. An assertion checks that the bytes match what the test author
expected; it cannot check that the expectation matches what the
manufacturer documented.
