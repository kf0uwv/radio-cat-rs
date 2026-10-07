# 22. A PTT release that does not wait for the queue: owned release-only modem lines

Date: 2026-10-06

## Status

**Accepted** (user-approved 2026-10-06). Amends [ADR 0003](0003-modem-control-lines.md).
Supersedes, for the de-keying path only, the "no `Arc`, no `dup`" rule in
`planning/cat_server/task_plan.md` (task-job design, v2).

## Context

`cat-server`'s broker owns the physical session and serialises all access
through one worker. A radio's software keys PTT on DTR by handing the
broker a task job (`submit_task`); the worker runs it with a borrow of the
session's modem lines, in queue order. That is right for **asserting** PTT:
do not key before the command that set the mode has landed.

It is wrong for **releasing** PTT. While a CAT exchange is in flight the
worker holds an exclusive borrow of the session until the read returns or
times out (2 s read timeout, 5 s broker timeout). A queued un-key waits
that long while the radio transmits. Racing an "urgent" lane inside the
worker cannot help: the urgent branch still needs the session while the
exclusive borrow is live.

The task-job design rejected handing anyone a line handle that outlives the
session borrow, because a *captured raw fd* can be reused after the port
closes -- this process also opens sound cards, an RTL-SDR and sockets, so a
stale `TIOCMBIC` could hit a different device.

## Decision

1. `Transport::release_lines` and `CatSession::release_lines` return
   `Option<Box<dyn ModemControlLines + Send + Sync>>`, defaulting to `None`;
   `SerialCatSession` forwards. Implementations return an **owned
   duplicate**, not a borrowed or raw handle:
   - Linux `SerialPort`: `fcntl(F_DUPFD_CLOEXEC)` into an `OwnedFd`.
   - Windows `SerialPort`: `DuplicateHandle` into an `OwnedHandle`.
   - `Rfc2217Port`: a clone of its `Arc` shared state.
   An owned duplicate holds its own reference to the open file description,
   so its number cannot be reused while it is held. That removes the reason
   for the original rule. A modem-line change never touches the byte
   stream, so running one out of queue order cannot corrupt a frame.
2. `cat-server` takes the release lines **once**, in `build`, before the
   session moves into the broker, into one shared `ReleaseLines` type used
   by both the Linux and Windows backends. The worker holds a closer: when
   the worker is dropped, the duplicate is dropped (closed). A release holds
   the same lock while it runs, so close and release cannot overlap.
3. `BrokerHandle::release_now_bypassing_queue` (and the same on
   `BrokerCatSession`) runs a task immediately, on the caller's thread,
   against the release lines. **Only de-keying / safe-state actions may use
   it.** With no release lines it falls back to the ordered queue and logs a
   warning once per broker. Asserts stay on `submit_task`.

## Consequences

- A release completes in the time of one ioctl, regardless of what the
  worker is doing.
- While a broker exists, the port has one extra open handle. Closing the
  primary no longer drops DTR via last-close on its own; the broker's
  closer drops the duplicate when the worker ends, so last-close still
  happens when the broker goes away.
- `ModemControlLines` gains a second way in (an owned box beside the
  borrowed `modem_lines`). ADR 0003's "only reachable through a concrete
  type" consequence was already relaxed by `modem_lines`; this extends the
  same optional-accessor pattern.
