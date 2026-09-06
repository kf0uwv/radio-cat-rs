# 16. `cat-transport-rfc2217`: a serial port, with its modem control lines, over TCP

Date: 2026-09-01

## Status

**Accepted.** Implemented as `cat-transport-rfc2217`. First consumer:
`ts570d`'s emulator (that repo's ADR 0009).

## Context

Every transport in this workspace moves CAT bytes. None of them moves the
**wires**, and for one family of station the wires are where the behaviour
lives.

`ts570d`'s operator keys PTT from RS-232 DTR through an opto-isolator onto
the radio's ACC2 PTT pin. On that station: DTR keys the transmitter, RTS is
the radio's receive-enable (the radio withholds CAT responses while it is
low), and CTS reports that the radio's COM port is alive. `ModemControlLines`
already exists here precisely to expose that, and
`cat-transport-serial::SerialPort` implements it with `TIOCMBIS`/`TIOCMBIC`/
`TIOCMGET`.

The problem is testing it. `ts570d`'s emulator hosts its CAT port on a
pseudo-terminal, and **Linux ptys implement no modem-control ioctls at all**
— all three fail with `ENOTTY` on both ends. So the one signal the interface
is built on could not be observed by any virtual radio, and because
`SerialPort::open` swallows the result (`let _ = port.set_dtr(true)`), the
failure was silent rather than loud.

Something has to carry the lines. The options were an invented side-channel,
an out-of-tree kernel module (`tty0tty`), or a protocol that already exists.

## Decision

### Implement RFC 2217

RFC 2217, the Telnet Com Port Control Option, is what the world ships for
putting a serial port with its modem lines on a network. `ser2net` speaks
it; Moxa, Digi and USR device servers speak it. So a virtual radio and a
real device server become interchangeable to everything upstream, and a
consuming application gains remote-rig-over-a-device-server as a genuine
feature rather than a test-only path.

This is ADR 0014's `rtl_tcp` reasoning applied again: prefer the protocol
real hardware already speaks over a private one, even when the private one
would be less code.

### It is a `Transport`, not a `CatSession`

An RFC 2217 endpoint *is* a serial port; TCP changes only how the bytes and
the line states arrive. So this crate supplies `Rfc2217Port: Transport +
ModemControlLines` and stops there. `SerialCatSession<Rfc2217Port>` then
supplies read-until-`;` framing exactly as it does for a local port, and its
existing blanket `impl<T: Transport + ModemControlLines> ModemControlLines
for SerialCatSession<T>` forwards the lines through the framing layer for
free.

A second `CatSession` here would have meant a second copy of the framing
logic, free to drift from the first.

### Both directions of the protocol live in one module

`codec` is pure: Telnet framing and the Com Port option, client and server,
no I/O. `server::Rfc2217Peer` is the device-server half of one connection,
also with no I/O — a host program supplies only its sockets.

A device server that hand-rolled its own half would agree with the client
right up until one of them was edited, and the failure would surface as a
radio that quietly stopped keying rather than as a test going red.

### A reader thread, not a request/response worker

`cat-transport-tcp`'s Windows backend hands each `execute()` to a worker
thread as one write-then-read unit. That shape cannot work here.
`ModemControlLines` is **synchronous**, and a serial link is quiet for long
stretches: a worker blocked in a read waiting for a radio with nothing to
say would have `set_dtr` queued behind it, and the PTT key would be
swallowed by an idle receive.

So the socket is split. A reader thread owns the read half for the port's
whole life; writes go out through a mutex-guarded write half from whichever
side wants them. Neither waits on the other.

### Line state is a cache

`read_cts`/`read_dsr`/`read_dcd` answer from the last `NOTIFY-MODEMSTATE`
received. That is RFC 2217's own model — the option is notification-driven
and has no "read the lines now" round trip. Because the reader thread is
always reading, the cache tracks the peer continuously rather than going
stale between CAT commands, which is what lets a tool that sends no CAT
traffic at all (`ts570d-line status`) work against a remote port.

## Consequences

- A pty-hosted virtual radio can now be given real DTR/RTS/CTS, so
  PTT-line software becomes testable. `ts570d` ADR 0009 is the first use.
- Applications get remote serial over `ser2net`/device servers for free,
  through the framing layer they already use.
- **`Rfc2217Port` must implement `Drop`, and does.** The reader thread holds
  its own `Arc` to the shared state, so letting the last port fall out of
  scope frees nothing: the socket stays open and the peer never sees a
  disconnect. On a DTR-keyed station that is not a leak but *a transmitter
  left keyed by a program that has exited*. `Drop` shuts the socket down in
  both directions. This was found by a test, not by review.
- The crate is plain `std::net` + `std::thread` and target-gates nothing, so
  unlike `cat-transport-serial` it compiles and its tests run everywhere.
- **A monoio-based caller must enable monoio's `sync` feature**, and this is
  a genuine trap. The reader thread wakes the suspended task from another
  OS thread; monoio's waker panics on that unless `sync` is on
  (*"waker can only be sent across threads when `sync` feature enabled"*).
  This crate cannot enforce it — it has no monoio dependency at all, which
  is the design working as intended — and **no test in this workspace can
  catch a caller getting it wrong**, because the tests here use
  `futures::executor::block_on`, whose waker is thread-safe. It was found by
  running `ts570d`'s TUI, not by running any suite; the guard now lives in
  `ts570d/tests/rfc2217_under_monoio.rs`, next to the runtime and the flag.
  Worth remembering that `cat-transport-tcp`'s Windows backend uses the same
  primitive and has never tripped this, because there is no monoio on
  Windows — so this is the first cross-thread wake into a monoio task in
  the workspace.
- Not implemented, and decoded-but-ignored: line-state notification,
  flow-control suspend/resume, `PURGE-DATA`. Nothing in this workspace uses
  them; they reach the caller as events rather than being dropped.
