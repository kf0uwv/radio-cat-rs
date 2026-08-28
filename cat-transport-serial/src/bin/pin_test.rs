// Copyright 2026 Matt Franklin
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! RS-232 null-modem cable pin tester.
//!
//! Per `docs/adr/0006-windows-network-transport.md` (Deliverable 3): this
//! has nothing to do with any radio's CAT protocol -- it only exercises
//! [`cat_transport_serial::SerialPort`]'s [`Transport`] (TXD/RXD byte
//! loopback) and [`ModemControlLines`] (RTS/DTR/CTS/DSR/DCD) implementations
//! directly, so it lives here rather than in any radio application's own
//! repo, and both `ft991a` and `ts570d` can package/ship the same binary.
//!
//! Generalizes `ts570d`'s original `src/bin/pin_test.rs`, which hand-rolled
//! raw `libc`/`ioctl` calls against two file descriptors instead of reusing
//! this crate's abstractions (`SerialPort`/`ModemControlLines` did not exist
//! there as a shared transport-level capability yet). Same seven checks,
//! same semantics (TXD->RXD byte loopback both directions; RTS->CTS both
//! directions, hard failure; DTR->DSR both directions and DTR->DCD one
//! direction, warn-only since the TS-570D's own null-modem cable convention
//! does not require those lines) -- now expressed entirely through
//! `SerialPort`/`Transport`/`ModemControlLines`, so it is genuinely
//! cross-platform (same public types on Linux and Windows, per ADR 0004)
//! with zero platform-specific code in this file itself. Only `main`'s
//! entry point differs per platform, for the same reason ADR 0004 §1
//! documents for every other consumer of this crate's Windows backend:
//! `#[monoio::main]` cannot exist on Windows (no `monoio`), so a minimal
//! thread-parking `block_on` drives the same async body there instead.
//!
//! Usage:
//!   pin-test <source-port> <dest-port>
//!
//! Example (Linux):   pin-test /dev/ttyUSB0 /dev/ttyUSB1
//! Example (Windows): pin-test COM3 COM4

use cat_transport_core::{ModemControlLines, Transport};
use cat_transport_serial::{FlowControl, Parity, SerialConfig, SerialPort};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Pass,
    Fail(String),
    Warn(String),
}

struct TestResult {
    label: &'static str,
    outcome: Outcome,
}

/// Configuration used to open both ports: 9600 8N2, no flow control, and
/// -- unlike every other `SerialPort::open` caller in this workspace --
/// RTS/DTR are deliberately left deasserted at open time
/// (`initial_rts`/`initial_dtr: false`), since this tool manages both lines
/// itself, one bit at a time, and needs a known-clear baseline before each
/// modem-control check.
fn pin_test_config() -> SerialConfig {
    SerialConfig {
        baud_rate: 9600,
        data_bits: 8,
        stop_bits: 2,
        parity: Parity::None,
        flow_control: FlowControl::None,
        initial_rts: false,
        initial_dtr: false,
    }
}

/// Write two known bytes on `tx` and confirm they arrive unchanged on `rx`.
async fn test_txd_rxd(label: &'static str, tx: &mut SerialPort, rx: &mut SerialPort) -> TestResult {
    rx.flush_rx();

    let to_send = [0xAAu8, 0x55u8];
    if let Err(e) = tx.write(&to_send).await {
        return TestResult {
            label,
            outcome: Outcome::Fail(format!("write failed: {e}")),
        };
    }
    if let Err(e) = tx.flush().await {
        return TestResult {
            label,
            outcome: Outcome::Fail(format!("flush failed: {e}")),
        };
    }

    let mut got = [0u8; 2];
    let mut filled = 0;
    while filled < got.len() {
        match rx.read(&mut got[filled..]).await {
            Ok(n) => filled += n,
            Err(e) => {
                return TestResult {
                    label,
                    outcome: Outcome::Fail(format!("read failed (timeout or error): {e}")),
                }
            }
        }
    }

    let outcome = if got == to_send {
        Outcome::Pass
    } else {
        Outcome::Fail(format!(
            "data mismatch: sent {to_send:02X?}, got {got:02X?}"
        ))
    };
    TestResult { label, outcome }
}

/// Signature of a [`ModemControlLines`] setter (`set_rts`/`set_dtr`).
type SetLine<'a> = &'a dyn Fn(bool) -> Result<(), cat_transport_core::TransportError>;
/// Signature of a [`ModemControlLines`] getter (`read_cts`/`read_dsr`/`read_dcd`).
type ReadLine<'a> = &'a dyn Fn() -> Result<bool, cat_transport_core::TransportError>;

/// Assert `set_line` on one port, confirm `read_line` observes it (and a
/// clear baseline before, and a clear state after) on the other port's
/// corresponding status line. `warn_only` downgrades a failure to a
/// [`Outcome::Warn`] for lines the TS-570D's own cabling convention does not
/// require (DTR->DSR, DTR->DCD).
fn test_modem_bit(
    label: &'static str,
    set_line: SetLine,
    read_line: ReadLine,
    warn_only: bool,
) -> TestResult {
    let make_outcome = |msg: String| {
        if warn_only {
            Outcome::Warn(msg)
        } else {
            Outcome::Fail(msg)
        }
    };

    if let Err(e) = set_line(false) {
        return TestResult {
            label,
            outcome: make_outcome(format!("clear (baseline) failed: {e}")),
        };
    }
    std::thread::sleep(Duration::from_millis(50));

    match read_line() {
        Ok(true) => {
            return TestResult {
                label,
                outcome: make_outcome("baseline check failed: line already asserted".to_string()),
            }
        }
        Err(e) => {
            return TestResult {
                label,
                outcome: make_outcome(format!("read (baseline) failed: {e}")),
            }
        }
        Ok(false) => {}
    }

    if let Err(e) = set_line(true) {
        return TestResult {
            label,
            outcome: make_outcome(format!("set failed: {e}")),
        };
    }
    std::thread::sleep(Duration::from_millis(50));

    match read_line() {
        Ok(false) => {
            let suffix = if warn_only {
                " -- not required by every radio's cabling convention"
            } else {
                ""
            };
            return TestResult {
                label,
                outcome: make_outcome(format!(
                    "no response -- line not asserted after set{suffix}"
                )),
            };
        }
        Err(e) => {
            return TestResult {
                label,
                outcome: make_outcome(format!("read (after set) failed: {e}")),
            }
        }
        Ok(true) => {}
    }

    if let Err(e) = set_line(false) {
        return TestResult {
            label,
            outcome: make_outcome(format!("clear (after test) failed: {e}")),
        };
    }
    std::thread::sleep(Duration::from_millis(50));

    match read_line() {
        Ok(true) => TestResult {
            label,
            outcome: make_outcome("line still asserted after clearing".to_string()),
        },
        Err(e) => TestResult {
            label,
            outcome: make_outcome(format!("read (after clear) failed: {e}")),
        },
        Ok(false) => TestResult {
            label,
            outcome: Outcome::Pass,
        },
    }
}

fn print_result(r: &TestResult) {
    const LABEL_WIDTH: usize = 22;
    match &r.outcome {
        Outcome::Pass => println!("  {:<LABEL_WIDTH$} ... PASS", r.label),
        Outcome::Fail(msg) => {
            println!("  {:<LABEL_WIDTH$} ... FAIL", r.label);
            println!("      detail: {msg}");
        }
        Outcome::Warn(msg) => println!("  {:<LABEL_WIDTH$} ... WARN  ({msg})", r.label),
    }
}

/// The whole test run, platform-neutral: opens both ports, runs all seven
/// checks, prints results, and returns the process exit code.
async fn run(port_a: &str, port_b: &str) -> i32 {
    println!("Pin test: {port_a} -> {port_b}");
    println!();

    let mut a = match SerialPort::open(port_a, pin_test_config()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error opening {port_a}: {e}");
            return 1;
        }
    };
    let mut b = match SerialPort::open(port_b, pin_test_config()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error opening {port_b}: {e}");
            return 1;
        }
    };

    let mut results = Vec::new();
    results.push(test_txd_rxd("TXD->RXD (A->B)", &mut a, &mut b).await);
    results.push(test_txd_rxd("TXD->RXD (B->A)", &mut b, &mut a).await);
    results.push(test_modem_bit(
        "RTS->CTS (A->B)",
        &|v| a.set_rts(v),
        &|| b.read_cts(),
        false,
    ));
    results.push(test_modem_bit(
        "RTS->CTS (B->A)",
        &|v| b.set_rts(v),
        &|| a.read_cts(),
        false,
    ));
    results.push(test_modem_bit(
        "DTR->DSR (A->B)",
        &|v| a.set_dtr(v),
        &|| b.read_dsr(),
        true,
    ));
    results.push(test_modem_bit(
        "DTR->DSR (B->A)",
        &|v| b.set_dtr(v),
        &|| a.read_dsr(),
        true,
    ));
    results.push(test_modem_bit(
        "DTR->DCD (A->B)",
        &|v| a.set_dtr(v),
        &|| b.read_dcd(),
        true,
    ));

    for r in &results {
        print_result(r);
    }

    let passed = results
        .iter()
        .filter(|r| r.outcome == Outcome::Pass)
        .count();
    let failed = results
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Fail(_)))
        .count();
    let warned = results
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Warn(_)))
        .count();

    println!();
    println!("Result: {passed} passed, {failed} failed, {warned} warnings");

    i32::from(failed > 0)
}

enum ParsedArgs {
    Ports(String, String),
    PortsAndLoop(String, String, String),
}

fn parse_args() -> Option<ParsedArgs> {
    let args: Vec<String> = std::env::args().collect();
    match args.len() {
        3 => Some(ParsedArgs::Ports(args[1].clone(), args[2].clone())),
        4 => Some(ParsedArgs::PortsAndLoop(
            args[1].clone(),
            args[2].clone(),
            args[3].clone(),
        )),
        _ => {
            eprintln!("Usage: {} <source-port> <dest-port>", args[0]);
            eprintln!(
                "Example: {} /dev/ttyUSB0 /dev/ttyUSB1  (or COM3 COM4 on Windows)",
                args[0]
            );
            None
        }
    }
}

#[cfg(target_os = "linux")]
#[monoio::main(timer_enabled = true)]
async fn main() {
    match parse_args() {
        Some(ParsedArgs::Ports(port_a, port_b)) => {
            std::process::exit(run(&port_a, &port_b).await);
        }
        Some(ParsedArgs::PortsAndLoop(port_a, port_b, loop_count_str)) => {
            let loop_count: u32 = loop_count_str.parse().unwrap();
            // Repeat count only -- no per-iteration state depends on the
            // index itself.
            for _ in 0..loop_count {
                let e = run(&port_a, &port_b).await;
                monoio::time::sleep(Duration::from_millis(100)).await;
                if e != 0 {
                    std::process::exit(e);
                }
            }
            std::process::exit(0);
        }
        None => {
            std::process::exit(1);
        }
    }
}

/// Minimal thread-parking `block_on`, needed only because `#[monoio::main]`
/// cannot exist on Windows (no `monoio`) -- see this file's module doc and
/// `docs/adr/0004-windows-serial-backend.md` §1, the same shape sketched
/// there for `ft991a`'s eventual Windows entry point, applied here to this
/// binary's own `main` instead.
#[cfg(target_os = "windows")]
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    struct ThreadWaker(std::thread::Thread);
    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = Box::pin(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::park(),
        }
    }
}

#[cfg(target_os = "windows")]
fn main() {
    // Mirrors the Linux `main` above's three-way `ParsedArgs` dispatch --
    // this used to destructure a plain 2-tuple, from before `PortsAndLoop`
    // existed, and was left stale when that variant was added (this
    // platform's `main` is never exercised by a Linux build, so nothing
    // caught the mismatch until now). No `monoio::time::sleep` here since
    // `monoio` doesn't exist on Windows at all -- `std::thread::sleep` is
    // the direct equivalent for this blocking `main`.
    match parse_args() {
        Some(ParsedArgs::Ports(port_a, port_b)) => {
            std::process::exit(block_on(run(&port_a, &port_b)));
        }
        Some(ParsedArgs::PortsAndLoop(port_a, port_b, loop_count_str)) => {
            let loop_count: u32 = loop_count_str.parse().unwrap();
            for _ in 0..loop_count {
                let e = block_on(run(&port_a, &port_b));
                std::thread::sleep(Duration::from_millis(100));
                if e != 0 {
                    std::process::exit(e);
                }
            }
            std::process::exit(0);
        }
        None => {
            std::process::exit(1);
        }
    }
}
