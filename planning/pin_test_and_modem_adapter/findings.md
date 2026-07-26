# Findings: pin-test tool + NoModemControlLines (Deliverables 3 and 4)

Folded into docs/adr/0006-windows-network-transport.md §6/§7 (grouped with
the Windows network transport work per the task's own "your call" on ADR
placement, since all three deliverables share the same
SerialPort/ModemControlLines/CatSession foundation).

- Read ts570d/src/bin/pin_test.rs in full: hand-rolled raw libc/ioctl
  RS-232 pin tester (TIOCMBIS/TIOCMBIC/TIOCMGET, raw termios), predates
  this workspace's SerialPort/ModemControlLines abstractions. Rebuilt as
  cat-transport-serial/src/bin/pin_test.rs, a [[bin]] (name = "pin-test"),
  same seven checks, built entirely on SerialPort/Transport/
  ModemControlLines -- zero platform-specific code in the file itself.
- Read ft991a/src/main.rs's TcpClientSession (honest-error ModemControlLines
  adapter around TcpCatSession) in full. Generalized as
  cat_transport_core::NoModemControlLines<S> (cat-transport-core/src/
  modem.rs): transparent CatSession delegation + an unconditional
  ModemControlLines impl returning a named error from every method.
  Deliberately does NOT also solve CatSession error-type remapping
  (orthogonal, orphan-rule-constrained) -- see the ADR for the full
  reasoning and the exact composition pattern ft991a/ts570d should use.

See docs/adr/0006-windows-network-transport.md for the full design record.
