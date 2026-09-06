# ADR 0018 — A console asks the radio's host what it has

Status: Accepted (2026-09-02)

## Context

A console is not usually on the radio's machine. The GUI is a network
client by construction (`ts570d` ADR 0008 §3), and the TUI can be one.

Both need to offer an operator a choice of signal hardware — sound-card
and SDR names are per-host and unguessable, so nobody types
`plughw:CARD=Codec,DEV=0` from memory — and until now the only way a
console could produce that list was to enumerate **its own** machine.

That is not a smaller version of the right answer. It is a wrong one that
looks right: an operator running the GUI on a laptop would be offered
their laptop's microphone as the radio's ACC2 receive audio. The picker
would populate, the attach would succeed, a waveform would appear, and
every layer below would agree. Nothing downstream can detect it.

`CapabilitiesWire::installation` (ADR 0015) already tells a console what
the radio's bench has *wired and streaming*. It does not answer what
*could* be attached, which is a different question asked at a different
time — a dongle sitting unplugged in a drawer is in neither answer, and
one plugged in after the server started is in the second and not yet the
first.

## Decision

The native protocol carries the question.

- `Command::ReadDevices` → `ServerMessage::Devices { lists }`, one
  `cat_signal::DeviceList` per kind, each keeping its own three-way
  outcome: found some, found none, could not ask.
- `Command::AttachDevice { kind, spec }` → `Ack`, or `Error` carrying the
  host's own refusal text.
- `NativeSession::publish_devices` mirrors `publish_state`, so the session
  stays I/O-free and testable without a socket on any platform.
- `RadioHost::devices()` defaults to `None`, which **declines** the
  question. A server that never looked must not answer with an empty list:
  that would be a claim about its machine it is in no position to make.
- `cat_signal::DeviceDirectory` is the seam an application implements. It
  pairs `list` with `attach` deliberately — the two halves have to agree
  about what a `spec` string means, and one trait makes that structural
  rather than a convention two crates must separately remember.

`spec` strings are passed back verbatim from a `DeviceInfo` the server
sent. A client never composes one: device naming is the host's namespace,
and a client that built a spec would be guessing about a machine it cannot
see.

## Consequences

### Four states, and they must look different

"Haven't asked", "the server declines", "the server looked and found
nothing", and "here they are" have four different next actions. Folding
the middle two into an empty list is the specific mistake this design
exists to prevent: *plug something in* and *this server cannot help you
from here* send an operator to opposite ends of the shack.

### An older server reads as declining, not as a fault

A server built before this command cannot deserialize the `cmd` tag and
answers `Malformed`. The client reports that as "not offered", the same as
an explicit decline. From a console's side the two situations are
identical — there is no list — and neither deserves an error dialog.

### Attach opens the device where somebody is waiting

`ServerDevices::attach` opens the source before handing it to the tap
thread. A busy dongle therefore refuses the attach with the driver's own
words while the operator is still looking at the picker, rather than two
seconds later on a background thread with nobody listening. It also means
the device is never open twice.

### Audio is offered and cannot yet be attached remotely

*(Superseded 2026-09-02 by ADR 0019, which added audio frames. An
`AttachDevice` for a sound card now opens it on the radio's machine and
the samples reach the console. The paragraph is kept because the reasoning
still applies to any future device kind whose output the protocol cannot
carry: offer the list, refuse the attach, and say which half is missing —
rather than reporting success for a panel that would stay dark.)*

The protocol carries spectrum frames and no audio. A server that opened a
sound card on an `AttachDevice` would capture audio with nowhere to send
it and report success for a panel that would stay dark, so it refuses and
says which half is missing. The list is still worth serving: it tells an
operator what to put after `--acc2-audio` on the radio's own machine.

### The raw-CAT console cannot use any of this

`ts570d --server <addr>` speaks `cat-transport-tcp`'s raw framing to
`--raw-tcp-port` — a Kenwood byte pipe with no capability concept. It
reports honestly that it cannot ask (`ConsoleSources::remote()`) and will
keep doing so unless it gains a native-protocol mode. That is a real
limit, recorded rather than papered over.
