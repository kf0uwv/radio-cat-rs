# Findings — architect

Planning pass: 2026-07-16. Extraction (task A) and TCP/UDP/server (task B)
are now authorized by explicit user go-ahead. This file records what was
learned reading `ts570d`'s actual current code (as of commit `1585e1e Add
CatSession abstraction for network-transport readiness`, branch
`refactor/generic-cat-framework`) rather than relying on this repository's
ADRs' paraphrase of it, plus the concrete decisions made for the four
questions the architect was asked to resolve.

## 1. Monoio / async-runtime binding — resolved

See [ADR 0002](../../docs/adr/0002-async-runtime-binding-for-transport-crates.md).
Decision: **keep** `monoio` / `#[async_trait(?Send)]`, extend it to
`cat-transport-core`, `cat-transport-serial`, `cat-transport-tcp`,
`cat-transport-udp`, `cat-server`. Do not adopt the GAT/associated-future
redesign now. Explicit revisit trigger recorded: Windows serial support, or a
consumer needing a non-monoio runtime.

`cat-framework`/`cat-client` are unaffected: `cat-framework` has zero async
code today; `cat-client` needs `async-trait` but not `monoio` as a real
dependency (only as a dev-dependency, for `#[monoio::test]`).

## 2. Cargo workspace shape and cross-repo dependency mechanism

**Workspace (this repo):** a root `Cargo.toml` with `[workspace]`,
`[workspace.package]`, and `[workspace.dependencies]`, matching `ts570d`'s own
versions for eventual compatibility: `monoio = "0.2.3"`, `async-trait = "0.1"`,
`thiserror = "1.0.61"`, `tracing = "0.1.40"`, `libc = "0.2.153"`,
`nix = "0.27.1"`, edition `2021`, `rust-version = "1.75"`, `resolver = "2"`.
Members are added incrementally as each dispatch-queue task lands — not all
seven declared up front — matching ADR 0001's own "not a requirement to build
out all seven crates immediately" guidance. The **first** subagent to run
(`cat_framework`, Task 1) creates this root `Cargo.toml`, since nothing exists
yet for it to extend.

**How `ts570d` will eventually depend on these crates (decision, not
executed — `ts570d` migration is out of scope of this planning pass, see
"Explicit follow-on" below):** a path dependency across two independent git
repositories on the same filesystem (e.g.
`cat-framework = { path = "../radio-cat-rs/cat-framework" }`) is fragile —
it only resolves on a machine where both repos happen to be checked out as
siblings at that exact relative location, which breaks for any fresh clone of
`ts570d` anywhere else (the normal case for two separate GitHub repositories).
Decision: `ts570d` should depend on these crates via **git dependencies**:

```toml
cat-framework = { git = "https://github.com/kf0uwv/radio-cat-rs", branch = "main" }
cat-client = { git = "https://github.com/kf0uwv/radio-cat-rs", branch = "main" }
cat-transport-core = { git = "https://github.com/kf0uwv/radio-cat-rs", branch = "main" }
cat-transport-serial = { git = "https://github.com/kf0uwv/radio-cat-rs", branch = "main" }
```

`radio-cat-rs` already has an `origin` remote configured at exactly this URL
(`git remote -v` confirms it), so once this work is committed and pushed, the
git dependency resolves for any clone of `ts570d`, anywhere — not just this
machine. Until pushed, a `file://`-scheme git URL pointing at this repo's
local `.git` directory would also resolve for `cargo` on this machine (`cargo`
shells out to `git`, and `git`'s own `file://` transport needs no GitHub);
that's a valid same-machine fallback but not the target end state.
Pin strategy: floating `branch = "main"` while both repos co-evolve pre-1.0;
move to `tag =` / pinned `rev =` once these crates stabilize enough to
version. **This is a decision for the record, not an action taken now** — see
"Explicit follow-on" below.

## 3. Code-motion mechanics: clean copy-and-adapt, not history-preserving move

Decision: subagents write fresh files in the new crate layout (Read the
`ts570d` source, Write the adapted version here) rather than using
`git subtree`/`git filter-repo` to graft `ts570d`'s file history into this
repo's git graph. Reasons:

- `ts570d`'s own history for these files is untouched and fully browsable
  forever in the `ts570d` repository — nothing is destroyed by not replaying
  it here.
- At least one extraction (`cat-client`, see finding 5 below) requires real
  code changes, not a byte-identical move — a history-preserving import would
  carry forward code that's about to be edited anyway, with no benefit.
- Grafting foreign commit history into an otherwise-independent repository via
  `git subtree`/`filter-repo` is real operational overhead (rewriting this
  repo's own graph, reconciling unrelated histories) for a benefit — line
  blame across the repo boundary — that citation-by-hash already achieves
  well enough: every new crate's first commit message should cite the exact
  `ts570d` commit (`1585e1e`) and file paths it was extracted from, so anyone
  who needs the original history can jump to it in `ts570d`'s own repo.

## 4. Explicit follow-on (not in this dispatch queue, not dropped)

Migrating `ts570d` itself onto these new crates (deleting its local
`framework`/`serial::io_uring`/`radio::RadioClient` code and pointing at the
git dependencies above) is **out of scope of this planning pass**. It is a
separate, later planning pass, gated on this repository's crates actually
existing and being pushed. `ts570d` and `ft991a` were not touched by this
planning pass, per instruction.

## 5. `cat-client` is not a pure move — `RadioClient` is not actually generic yet

ADR 0001 describes `cat-client` as "the radio-independent core of what
`ts570d`'s `radio::RadioClient<S: CatSession>` does today." Reading
`radio/src/client.rs` directly shows this is not quite accurate as a
description of *today's* code — it is accurate as a description of the
*target*:

```rust
use crate::ts570d_radio::{Ts570dCommandId, TS570D_COMMAND_TABLE};
use crate::{RadioError, RadioResult};

pub struct RadioClient<S: CatSession> { pub(crate) session: S }

impl<S> RadioClient<S> where S: CatSession<Error = TransportError> {
    fn validate_code(code: &str) -> RadioResult<&'static CommandDefinition<Ts570dCommandId>> {
        TS570D_COMMAND_TABLE.find(code)...
    }
    // query/query_with_param/set all return RadioResult<T>
}
```

`RadioClient` hardcodes `Ts570dCommandId`/`TS570D_COMMAND_TABLE` (a concrete
radio's command table, not a generic `C: CommandId`/`CommandTable<C>`) and
returns `radio::RadioError` — a TS-570D-flavored error enum that mixes truly
generic variants (`UnknownCommand`, `CommandNotReadable`, `CommandNotWritable`,
`Transport`) with radio-specific ones (`InvalidMode`, `FrequencyOutOfRange`).
Extracting `cat-client` therefore requires **design work, not a mechanical
move**: parameterizing the client over `C: CommandId` / `&'static
CommandTable<C>`, and introducing a new generic client error type (e.g.
`ClientError<E>` with `UnknownCommand`/`CommandNotReadable`/
`CommandNotWritable`/`Transport(E)` variants) that `ts570d`'s `RadioError`
would later wrap or convert from, once `ts570d` migrates (see finding 4).
This is called out explicitly in the Task 3 dispatch below and in ADR 0001's
amendments — it is a real gap between the ADR's target description and the
code as it exists, not a silent redesign.

## 6. `cat-transport-serial`'s real scope includes `ts570d/serial/`, not just `framework/src/session.rs`

See ADR 0001 Amendment 3. `framework/src/session.rs`'s `SerialCatSession<T:
Transport>` is already fully generic over `Transport` — it has no hardware
code in it. The actual io_uring RS-232 implementation (`SerialConfig`,
`SerialPort`, `impl Transport for SerialPort`, termios/`nix`/`libc` plumbing)
lives in `ts570d`'s separate `serial` crate (`serial/src/io_uring.rs`, 658
lines, plus `serial/src/lib.rs`, 42 lines). The user's own background-reading
list for this planning pass did not name this crate, but ADR 0001's own text
already requires it ("`SerialCatSession<T: Transport>`, reproducing `ts570d`'s
existing read-until-`;` framing over **a real serial port** (io_uring on
Linux today)") — without it, `cat-transport-serial` would be an empty shell:
a generic wrapper with no concrete transport to wrap. Task 2 below includes
both sources.

## 7. `cat-transport-core`'s dependency-on-nothing claim was wrong

See ADR 0001 Amendment 2. `CatSession::execute` returns
`framework::cat::ResponseDisposition` (itself carrying `ProtocolErrorKind`),
reused deliberately per `ts570d` ADR 0005 rather than duplicated. That type
belongs to `cat-framework`'s scope, so `cat-transport-core` must depend on
`cat-framework` (one-way, and `cat-framework` still has zero dependencies of
its own — the DAG property survives). Recorded as an ADR 0001 amendment
rather than silently building it differently from what's written down.

## 8. `framework::state_machine` (`ApplicationStateMachine`/`State`) is out of scope

Not named anywhere in ADR 0001's `cat-framework` responsibility list; not CAT
protocol machinery; and as far as this reading found, unused anywhere in
`ts570d` outside its own re-export in `framework/src/lib.rs` (`grep` for
`ApplicationStateMachine`/`state_machine::` outside that file turned up
nothing). Excluded from the Task 1 dispatch below. Recorded explicitly per
instruction not to silently drop something — it stays in `ts570d`, unmoved,
and its purpose (dead scaffolding? planned for future use?) is a question for
a separate decision, not this one.

## 9. `errors.rs` must be split, not moved whole

`framework/src/errors.rs` defines both `FrameworkError`/`FrameworkResult`
(used only by `state_machine.rs`, excluded per finding 8) and `TransportError`
(used by `Transport`/`CatSession`, required by `cat-transport-core`). Task 1
takes neither (cat-framework's own `CatFrameworkError<E>`, defined directly in
`cat.rs`, is self-contained and needs nothing from `errors.rs`). Task 2 takes
only `TransportError` into `cat-transport-core`. `FrameworkError`/
`FrameworkResult` stay behind in `ts570d`, alongside `state_machine.rs`. Note
for whoever eventually touches `ts570d` in the follow-on migration (finding
4): `CatFrameworkError<E>` (cat-framework's real dispatch error) and
`FrameworkError` (the state-machine-only error) are confusingly similarly
named today; not this planning pass's problem to fix, but worth flagging so
the two aren't conflated during a future move.

## 10. Wire-framing coordination needed between Task 4 (transports) and Task 5 (server)

`cat-server`'s charter (`.claude/agents/cat_server.md`) states server-side
TCP/UDP listener code lives in `cat-server` itself, not in
`cat-transport-tcp`/`cat-transport-udp` (those crates own the client-facing
session framing). But a client-side `TcpCatSession` and a server-side TCP
listener must speak the *same* wire framing to interoperate at all — the
length-prefix format `cat-transport-tcp` invents in Task 4 is the format
`cat-server`'s TCP listener must parse in Task 5, and likewise for UDP's
envelope/dedup format. This is not a crate dependency (per the charter,
`cat-server` doesn't depend on `cat-transport-tcp`/`-udp` for its listener
code) but it is a **coordination requirement**: Task 4's `progress.md` must
document the exact byte-level framing (length-prefix width/endianness for
TCP; envelope field layout, request-ID size, and dedup-cache key/eviction
policy for UDP) precisely enough for the `cat_server` agent to implement a
wire-compatible listener in Task 5 without re-deriving the format
independently. Sequenced accordingly in the dispatch queue.

## 11. Windows serial backend (2026-07-19) — ADR 0002's revisit trigger fired

Full decision record: [ADR 0004](../../docs/adr/0004-windows-serial-backend.md).
This section is the supporting research read directly for that decision,
kept here per this file's existing convention of recording what was learned
from the real code rather than an ADR's paraphrase of it.

**What was actually read, and why it mattered:**

- `ft991a/ui/src/terminal.rs` + `ft991a/src/main.rs` (read in full): a
  single sequential loop under `#[monoio::main]`. No `monoio::spawn`
  anywhere in this repo — `poll_radio_state` awaits 10 sequential CAT round
  trips, `draw_frame`, a blocking `crossterm::event::poll(10ms)` (already
  synchronous today), then `monoio::time::sleep(5ms)`. Exactly one live
  task; nothing else could ever be starved by that task blocking.
- `ts570d/ui/src/terminal.rs` (read in full): genuinely concurrent.
  `run()` spawns a `radio_task` via `monoio::spawn` alongside a `ui_task`,
  linked by `Rc<RefCell<VecDeque<T>>>` channels, specifically so key events
  stay responsive during a slow poll or a 107-step diagnostic run (the
  module's own doc comment says so explicitly). This is real cooperative
  concurrency on one OS thread — if either task blocks that thread
  synchronously, so does the other.
- This asymmetry between the two consuming repos is the crux of the
  decision: a design that's fine for `ft991a` today (naive blocking Win32
  calls disguised as `async fn`) would silently defeat `ts570d`'s
  responsiveness design the moment `ts570d` targets Windows — and
  `cat-transport-serial`, as shared infrastructure, has no visibility into
  which consumer is using it, so it cannot assume the `ft991a` shape is
  the only one that matters. This ruled out "blocking-in-async-fn" as the
  crate's general mechanism, even though it would have been the simplest
  thing that works for the one consumer currently being built against.
- `cat-transport-serial/src/{io_uring.rs,lib.rs,session.rs}` (read in
  full): confirmed `SerialCatSession<T: Transport>` is already fully
  generic and platform-agnostic — it needed zero changes. Confirmed the
  exact shape of `SerialConfig`/`Parity`/`FlowControl` (currently defined
  inline in `io_uring.rs`, moved to a new shared `config.rs` per ADR 0004
  §2 rather than duplicated) and the exact `READ_TIMEOUT`
  production/test-constant split that Task 7's `SetCommTimeouts` design
  reuses rather than reinventing.
- `docs/adr/0003-modem-control-lines.md` + `cat-transport-core/src/
  modem.rs`: confirmed `ModemControlLines` methods are plain sync `fn`s
  ("direct `ioctl(2)` calls with no I/O wait") — this precedent is what
  justifies calling `EscapeCommFunction`/`GetCommModemStatus` directly and
  synchronously on Windows too, rather than routing them through the
  worker thread the way `Transport::read`/`write` are.

**Why option 3 (a third async-runtime crate) was rejected, restated
precisely:** the actual requirement — "let a background OS thread report
one I/O completion back to whatever is polling this future" — does not
need a scheduler or reactor, only a `Future`-compatible completion value.
A hand-rolled `Mutex<Option<T>> + Option<Waker>` primitive (the same shape
as `futures::channel::oneshot`) satisfies it completely, at the cost of
~50 lines of `std`-only code instead of a new dependency. Its correctness
is guaranteed by `std::task::Waker`'s own contract (`wake()` must be safe
from any thread — that is the entire reason `Waker` exists as opposed to a
plain closure), not by anything specific to an executor. Importantly,
`monoio` itself is never in the picture on the Windows side at all (it
cannot compile there — io_uring is a Linux kernel interface, not a library
choice), so there was no need to empirically verify anything against
`monoio`'s own `Waker` implementation, which simplified this decision
considerably once made explicit.

**What this leaves for `ft991a`/`ts570d` themselves (not authorized, not
dispatched, informational only):** since `#[monoio::main]` cannot exist on
Windows, each application needs its own Windows entry point eventually.
`ft991a`'s single-sequential-loop shape only needs a ~30-line hand-rolled
thread-parking `block_on` (no new crate) with `std::thread::sleep`
replacing `monoio::time::sleep`. `ts570d`'s concurrent two-task shape would
need `std::thread::spawn` replacing `monoio::spawn` to preserve its
responsiveness property. Recorded in ADR 0004 §1 for whoever eventually
plans that work in each of those repos — not touched by this session, per
this repo's own "`ts570d`/`ft991a` not touched" ground rule.

## Agent-roster note: `cat-client` is `cat_framework`'s task, not `cat_transport`'s

Worth flagging since it could otherwise look like a slip: the established
roster (`CLAUDE.md`'s subagent table, `.claude/agents/cat_framework.md`'s own
"You work exclusively in the `cat-framework/` and `cat-client/` directories")
assigns `cat-client` to the `cat_framework` agent, not `cat_transport`.
`.claude/agents/cat_transport.md` never mentions `cat-client` in its scope at
all. The dispatch queue below follows the roster's established lane
boundaries rather than building `cat-client` under `cat_transport`.

## Task 13 — capability model gate: **PASS, with one recorded strain** (2026-08-28)

ADR 0010 conceded `RadioCapabilities` "is a guess until three radios are
actually described by it." Two are now described, as `#[cfg(test)]`
fixtures in `cat-framework/src/capabilities_fixtures.rs`. Every value is
read out of the app repos and cited at its point of use; neither app was
modified.

### No escape hatch was needed

Two of ADR 0010's three forbidden hatches are **structurally impossible**
here rather than merely avoided: `RadioCapabilities` is `Copy` and
`const`-constructible, and neither `serde_json::Value` nor `HashMap` is
either, so adding one would fail to compile. A test builds
`const RADIOS: [RadioCapabilities; 2]` to keep that true.

The third — matching on `model` — is not structurally preventable, so
`every_question_is_answered_from_data_not_from_the_model_string` blanks the
model name and then asks every question a real consumer asks (which handles
to open, whether to draw a waterfall, how to scale the S-meter, which mode
buttons to render, whether a width selector is meaningful). It is written
as a loop over both radios, which it could not be if any answer depended on
knowing which radio it was.

### What the FT-991A actually stressed

| | TS-570D | FT-991A |
|---|---|---|
| Handles | 1 (CAT + Keying shared) | 3 (CAT, Keying, Audio; none shared) |
| Modes | 8 | 14 |
| Meters | 4 | 7 |
| S-meter range | 0-30 | 0-255 |
| Memory | 0-99, unnamed | 1-117, named |
| Menu entries | 52 | 152 |
| Coverage | 500 kHz - 60 MHz | 30 kHz - 470 MHz |
| Signal | `IfTap` (CN4, 73.05 MHz, inverted) | `None` |

The endpoint model earned its keep in both directions: the TS-570D needed
`shareable_with` to say one handle does two jobs, and the FT-991A needed it
*empty* to say three handles do not. A global "shares its port" flag on the
set would have expressed neither.

Two findings worth keeping:

- **The S-meter ranges differ by 8.5x for the same physical quantity.**
  This is the concrete fix for the duplication ADR 0011 found, where
  `smeter_bar` existed twice with each radio's scale hardcoded. Raw 15 is
  mid-scale on one radio and under 6% on the other.
- **Both radios put `6` on the wire for their sixth mode** — FSK on the
  TS-570D, RTTY-LSB on the FT-991A. A shared table of wire codes would have
  been a table of coincidences. This is why `ModeDescriptor` carries a
  normalized `ModeId` and a per-radio `label` but no encoding at all.

### The strain: `MenuCapability`

`MenuCapability` is an item count and a writability flag. That answers "is
there a menu, and how big," which is what a handshake and a status display
need. It does **not** describe a menu well enough to render one: the
FT-991A's `EX_MENU_TABLE` is 152 heterogeneous typed entries and the
TS-570D's 52 are a different shape again.

This is a pass, not a failure, because menu *topology* is radio-specific by
ADR 0011's own seam and belongs in each app's crate. It is recorded because
the pressure to widen it is predictable: the first GUI menu screen will
want `&'static [MenuItemDescriptor]` here, which would pull 152
FT-991A-shaped entries into the generic library and rebuild the coupling
this model exists to remove.

**If it is ever genuinely needed, the right shape already exists**:
`cat-signal`'s `SettingDescriptor` list, published by the radio and
rendered generically — not a second bespoke menu type. Recorded so the
next person reaches for that instead.

### Not closed by this task

ADR 0010 asked for **three** radios. The IC-7100 remains undescribed and
its manual is still outstanding, so the model has been tested against a
shared-handle HF radio and a multi-handle HF/VHF/UHF one, but not against a
binary CI-V radio. ADR 0009's `CatWireFormat` work suggests CI-V will
stress the *wire* layer rather than this one, but that is a prediction, not
a result.

## TUI divergence audit (2026-08-28) — **no silent bugfix drift**

ADR 0011 rev 4 made this audit a condition of `cat-ui-ratatui`, on the
grounds that each of the 1142 changed lines between `ts570d/ui/src/layout.rs`
and `ft991a/ui/src/layout.rs` is "either a bugfix one app has and the other
lacks, or a genuine radio-specific difference," and that telling them apart
is the real work.

Done function by function. **Every divergence is legitimate.** The failure
mode the audit existed to catch — a fix living in one app and silently
absent from the other — does not appear in these files.

### Shared functions

| Function | Divergence | Verdict |
|---|---|---|
| `draw_disconnected` | none: 47 lines byte-identical | **pure duplication** |
| `draw_errors` | the concrete display type only | **duplication blocked by a type** |
| `draw_header` | the title string only | **duplication with a parameter** |
| `build_menu_column` | key type `&'static str` vs `char` | **duplication blocked by a type** |
| `format_hz`, `mini_bar` | none | already extracted (Task 18) |
| `smeter_bar` | 0-30 vs 0-255 scale | radio data; solved by `MeterReading` |
| `draw_diag_warning_panel` | one line of copy about a keyer memory | genuine radio difference |
| `split_areas` | status pane 7 rows vs 4 | genuine layout difference |
| `draw_control_panel` | 37% shared; FT-991A menu topology | genuine, and ADR 0011 leaves it in the app |

The four marked **duplication** are now `cat-ui-ratatui::session`. None
needed a behaviour change to extract; three were blocked only by a concrete
type or a literal.

### Functions unique to one app

Ten only in `ts570d`, six only in `ft991a`. Most are genuinely
radio-specific (`agc_label`, `nr_label`, `bc_label` for TS-570D DSP;
`draw_profile_list`, `draw_ex_sub_group_menu`, `rts_label` for FT-991A).

Two findings among them:

- **`smeter_label` exists only in `ts570d`.** The FT-991A TUI shows an
  S-meter bar with **no S-unit called out at all** — an operator reads a bar
  and no number. This is a *feature gap*, not a bugfix gap, and closing it
  is an app-side change needing a `docs/renderer-parity.md` row either way
  (ADR 0013). `cat_ui::format_smeter_label` makes it a one-line fix
  whenever it is wanted.
- **`draw_diag_warning_panel` is `pub` in one app and private in the
  other.** Harmless, and worth noticing as the kind of drift that
  accumulates unremarked.

### What this means for the migration

The audit was the risk. It is discharged: migrating `ts570d/ui` and
`ft991a/ui` onto the shared widgets is now a mechanical substitution with a
clear acceptance bar — the operator sees no change — rather than a
reconciliation of two divergent behaviours.
