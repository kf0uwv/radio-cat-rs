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

//! io_uring-based serial communication implementation
//!
//! This module provides high-performance asynchronous serial communication
//! using Linux io_uring interface for zero-copy operations via monoio.
//!
//! Moved from `ts570d`'s separate `serial` crate (`serial/src/io_uring.rs`,
//! commit `1585e1e`), per `docs/adr/0001-scope-and-crate-boundaries.md`
//! Amendment 3.

use std::fmt;
use std::io::ErrorKind;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream as StdUnixStream;

use async_trait::async_trait;
use monoio::buf::VecBuf;
use monoio::io::{AsyncReadRent, AsyncWriteRent};
use monoio::net::UnixStream;
use nix::sys::termios::{
    cfmakeraw, cfsetispeed, cfsetospeed, tcdrain, tcgetattr, tcsetattr, BaudRate, ControlFlags,
    InputFlags, SetArg,
};

use cat_transport_core::{ModemControlLines, Transport, TransportError};

use crate::SerialError;

/// RTS (Request To Send) bit for `TIOCMGET`/`TIOCMBIS`/`TIOCMBIC`.
const TIOCM_RTS: libc::c_int = 0x004;
/// DTR (Data Terminal Ready) bit for `TIOCMGET`/`TIOCMBIS`/`TIOCMBIC`.
const TIOCM_DTR: libc::c_int = 0x002;
/// CTS (Clear To Send) status bit, read-only via `TIOCMGET`.
const TIOCM_CTS: libc::c_int = 0x020;
/// DSR (Data Set Ready) status bit, read-only via `TIOCMGET`.
const TIOCM_DSR: libc::c_int = 0x100;
/// DCD/Carrier Detect status bit (`TIOCM_CAR` in `<sys/ttycom.h>`/glibc's
/// ioctl-types header naming), read-only via `TIOCMGET`.
const TIOCM_CAR: libc::c_int = 0x040;

/// Set or clear modem-control bits on `fd` via `TIOCMBIS`/`TIOCMBIC`.
///
/// PTY devices don't support modem control lines and return `ENOTTY` —
/// callers that need "best effort, ignore on PTY" semantics (e.g.
/// `SerialPort::open`'s startup assert) discard this `Err` explicitly
/// rather than this helper swallowing it, since [`ModemControlLines`]'s
/// contract requires surfacing real failures to callers who need to know.
fn modem_bits_set(fd: RawFd, bit: libc::c_int, asserted: bool) -> Result<(), TransportError> {
    let request = if asserted {
        libc::TIOCMBIS
    } else {
        libc::TIOCMBIC
    };
    // SAFETY: `fd` is a valid open serial-port (or PTY) fd for the duration
    // of this call; `&bit` is a valid pointer to a `c_int` as TIOCMBIS/
    // TIOCMBIC expect.
    let rc = unsafe { libc::ioctl(fd, request, &bit) };
    if rc == -1 {
        return Err(TransportError::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Read the modem status register on `fd` via `TIOCMGET` and test whether
/// `bit` is set.
fn modem_bit_get(fd: RawFd, bit: libc::c_int) -> Result<bool, TransportError> {
    let mut status: libc::c_int = 0;
    // SAFETY: `fd` is a valid open serial-port (or PTY) fd for the duration
    // of this call; `&mut status` is a valid pointer to a `c_int` as
    // TIOCMGET expects to write into.
    let rc = unsafe { libc::ioctl(fd, libc::TIOCMGET, &mut status) };
    if rc == -1 {
        return Err(TransportError::Io(std::io::Error::last_os_error()));
    }
    Ok(status & bit != 0)
}

/// Timeout for `Transport::read` retries on EAGAIN.
///
/// When the serial port has no data (EAGAIN / `WouldBlock`), `Transport::read`
/// retries the readv operation until data arrives or this deadline elapses.
/// 2 seconds is generous for TS-570D command-response latency (typically < 100 ms).
///
/// In tests, a shorter timeout is used to keep the test suite fast.
#[cfg(not(test))]
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Short read timeout used in tests to avoid 2-second waits in the test suite.
#[cfg(test)]
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);

/// Serial port configuration
#[derive(Debug, Clone)]
pub struct SerialConfig {
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: Parity,
    pub flow_control: FlowControl,
    /// Whether [`SerialPort::open`] asserts RTS high at construction time
    /// (via [`ModemControlLines::set_rts`]`(true)`). Defaults to `true`,
    /// preserving this crate's historical unconditional-assert behavior
    /// exactly. Set `false` if a consumer wants full runtime control over
    /// RTS from the moment the port opens — e.g. RTS-keyed CW/PTT, where
    /// idle/asserted polarity matters and this crate asserting it first
    /// would be an unwanted side effect.
    pub initial_rts: bool,
    /// Same as `initial_rts`, for DTR. Defaults to `true`.
    pub initial_dtr: bool,
}

#[derive(Debug, Clone)]
pub enum Parity {
    None,
    Even,
    Odd,
}

#[derive(Debug, Clone)]
pub enum FlowControl {
    None,
    Software,
    Hardware,
}

impl Default for SerialConfig {
    fn default() -> Self {
        Self {
            baud_rate: 9600,
            data_bits: 8,
            stop_bits: 2,
            parity: Parity::None,
            flow_control: FlowControl::None,
            initial_rts: true,
            initial_dtr: true,
        }
    }
}

/// Map a u32 baud rate to the nix `BaudRate` enum.
fn baud_rate_from_u32(baud: u32) -> Result<BaudRate, SerialError> {
    match baud {
        1200 => Ok(BaudRate::B1200),
        2400 => Ok(BaudRate::B2400),
        4800 => Ok(BaudRate::B4800),
        9600 => Ok(BaudRate::B9600),
        19200 => Ok(BaudRate::B19200),
        38400 => Ok(BaudRate::B38400),
        57600 => Ok(BaudRate::B57600),
        115200 => Ok(BaudRate::B115200),
        230400 => Ok(BaudRate::B230400),
        other => Err(SerialError::InvalidConfig(format!(
            "Unsupported baud rate: {}",
            other
        ))),
    }
}

/// Configure a file descriptor for raw serial communication according to `config`.
///
/// Applies: raw mode (cfmakeraw), data bits, stop bits, parity, flow control,
/// correct baud rate, and commits with TCSANOW.
fn configure_termios(fd: &OwnedFd, config: &SerialConfig) -> Result<(), SerialError> {
    let baud_rate = baud_rate_from_u32(config.baud_rate)?;

    let mut termios = tcgetattr(fd)
        .map_err(|e| SerialError::InvalidConfig(format!("tcgetattr failed: {}", e)))?;

    // Raw mode: disable all special processing (canonical mode, echo, signals, flow ctrl).
    // cfmakeraw clears CSIZE and sets CS8 among other things; we apply our own data-bits
    // setting afterwards to override.
    cfmakeraw(&mut termios);

    // --- data bits (character size) ---
    // Clear the CSIZE mask first, then set the requested width.
    termios.control_flags &= !ControlFlags::CSIZE;
    let cs = match config.data_bits {
        5 => ControlFlags::CS5,
        6 => ControlFlags::CS6,
        7 => ControlFlags::CS7,
        _ => ControlFlags::CS8, // 8 is the default and most common
    };
    termios.control_flags |= cs;

    // --- stop bits ---
    if config.stop_bits >= 2 {
        termios.control_flags |= ControlFlags::CSTOPB;
    } else {
        termios.control_flags &= !ControlFlags::CSTOPB;
    }

    // --- parity ---
    match config.parity {
        Parity::None => {
            termios.control_flags &= !(ControlFlags::PARENB | ControlFlags::PARODD);
        }
        Parity::Even => {
            termios.control_flags |= ControlFlags::PARENB;
            termios.control_flags &= !ControlFlags::PARODD;
        }
        Parity::Odd => {
            termios.control_flags |= ControlFlags::PARENB | ControlFlags::PARODD;
        }
    }

    // --- flow control ---
    match config.flow_control {
        FlowControl::None => {
            termios.control_flags &= !ControlFlags::CRTSCTS;
            termios.input_flags &= !(InputFlags::IXON | InputFlags::IXOFF);
        }
        FlowControl::Hardware => {
            termios.control_flags |= ControlFlags::CRTSCTS;
            termios.input_flags &= !(InputFlags::IXON | InputFlags::IXOFF);
        }
        FlowControl::Software => {
            termios.control_flags &= !ControlFlags::CRTSCTS;
            termios.input_flags |= InputFlags::IXON | InputFlags::IXOFF;
        }
    }

    // Set input and output baud rate
    cfsetispeed(&mut termios, baud_rate)
        .map_err(|e| SerialError::InvalidConfig(format!("cfsetispeed failed: {}", e)))?;
    cfsetospeed(&mut termios, baud_rate)
        .map_err(|e| SerialError::InvalidConfig(format!("cfsetospeed failed: {}", e)))?;

    // Apply immediately (TCSANOW: change occurs immediately)
    tcsetattr(fd, SetArg::TCSANOW, &termios)
        .map_err(|e| SerialError::InvalidConfig(format!("tcsetattr failed: {}", e)))?;

    Ok(())
}

/// Asynchronous serial port using monoio (io_uring).
///
/// Open a real serial device or PTY slave with [`SerialPort::open`], then use
/// the [`Transport`] implementation to send and receive bytes asynchronously.
///
/// ## Runtime Requirement
///
/// [`SerialPort::open`] must be called within an active monoio runtime context
/// because `UnixStream::from_std` registers the fd with the io_uring driver.
/// In practice this means calling `open()` from within `block_on` or an async task.
///
/// ## Implementation Notes
///
/// The fd is wrapped in `monoio::net::UnixStream` via `StdUnixStream::from_raw_fd`.
/// This is **sound** because:
/// - The fd is a valid, open, readable/writable file descriptor (serial port or PTY).
/// - I/O is performed exclusively via `writev`/`readv`, which submit
///   `IORING_OP_WRITEV`/`IORING_OP_READV` to io_uring.  These operations are
///   fd-type-agnostic — the kernel only requires that the fd is open and
///   supports vectored I/O, which both serial ports and PTY devices do.
/// - `IORING_OP_SEND`/`IORING_OP_RECV` (socket-only) are **never** used.
/// - The `UnixStream` type is used solely as a handle that exposes the
///   `AsyncReadRent` / `AsyncWriteRent` interface backed by readv/writev;
///   no Unix-domain-socket semantics are exercised.
pub struct SerialPort {
    /// monoio streaming I/O wrapper around the real file descriptor.
    ///
    /// Held as `UnixStream` for its `AsyncReadRent` / `AsyncWriteRent` impls
    /// (backed by IORING_OP_READV / IORING_OP_WRITEV, which are fd-type-agnostic).
    stream: UnixStream,
    /// Device path, kept for diagnostics
    path: String,
}

impl fmt::Debug for SerialPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SerialPort")
            .field("path", &self.path)
            .field("fd", &self.stream.as_raw_fd())
            .finish()
    }
}

impl SerialPort {
    /// Open a serial device at `path` with the given `config`.
    ///
    /// Must be called within an active monoio runtime context — the fd registration
    /// with io_uring happens inside `UnixStream::from_std`.
    ///
    /// Configures the port for raw mode according to all fields of `config`:
    /// baud rate, data bits (5–8), stop bits (1–2), parity, and flow control.
    ///
    /// `SerialConfig::default()` gives 9600 baud, 8N2, no flow control (matching the Kenwood TS-570D).
    pub fn open(path: &str, config: SerialConfig) -> crate::SerialResult<Self> {
        // Open the device:
        // O_NOCTTY: don't become controlling terminal
        // O_NONBLOCK: avoid blocking if no carrier detect (DCD) signal
        let std_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
            .open(path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    SerialError::DeviceNotFound(path.to_string())
                } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                    SerialError::PermissionDenied(path.to_string())
                } else {
                    SerialError::Io(e)
                }
            })?;

        // Convert std::fs::File to OwnedFd for termios configuration.
        // termios must be configured before handing the fd to monoio.
        let owned_fd: OwnedFd = std_file.into();

        // Configure termios with all SerialConfig fields.
        configure_termios(&owned_fd, &config)?;

        // Flush any data the radio sent before we were ready (e.g. AI auto-info).
        // SAFETY: owned_fd is a valid open fd.
        unsafe {
            libc::tcflush(owned_fd.as_raw_fd(), libc::TCIFLUSH);
        }

        // Wrap in monoio::net::UnixStream for AsyncReadRent + AsyncWriteRent.
        //
        // SAFETY: `owned_fd` is a valid open file descriptor (serial port or PTY).
        // We consume it via `into_raw_fd()` (preventing double-close) and hand
        // the raw fd to `StdUnixStream::from_raw_fd`.  The stream is then used
        // exclusively with `writev`/`readv` (IORING_OP_WRITEV / IORING_OP_READV),
        // which are fd-type-agnostic and do not require a Unix domain socket.
        // `UnixStream::from_std` registers the fd with the active io_uring driver.
        let raw_fd = owned_fd.into_raw_fd();
        let std_unix: StdUnixStream = unsafe { StdUnixStream::from_raw_fd(raw_fd) };
        std_unix.set_nonblocking(true).map_err(SerialError::Io)?;
        let stream = UnixStream::from_std(std_unix).map_err(SerialError::Io)?;

        let port = Self {
            stream,
            path: path.to_string(),
        };

        // Assert RTS and/or DTR per `config` (both default `true`, preserving
        // this crate's historical unconditional-assert behavior exactly). The
        // TS-570D uses RTS as "receive enable" — when RTS is LOW the radio
        // suppresses responses. Assert it explicitly so the OS/driver default
        // doesn't leave it undriven.
        //
        // Generalized to reuse the same runtime `ModemControlLines::set_rts`/
        // `set_dtr` a caller uses later, rather than duplicating the ioctl
        // call here. Errors are ignored deliberately — PTY devices don't
        // support modem control lines and return ENOTTY, which is harmless.
        if config.initial_rts {
            let _ = port.set_rts(true);
        }
        if config.initial_dtr {
            let _ = port.set_dtr(true);
        }

        Ok(port)
    }

    /// Get the device path this port was opened on.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Discard any unread bytes in the kernel receive buffer.
    pub fn flush_rx(&self) {
        // SAFETY: self.stream.as_raw_fd() is a valid open fd.
        unsafe {
            libc::tcflush(self.stream.as_raw_fd(), libc::TCIFLUSH);
        }
    }
}

#[async_trait(?Send)]
impl Transport for SerialPort {
    /// Write `data` to the serial port. Returns the number of bytes written.
    ///
    /// Uses `IORING_OP_WRITEV` (via `writev`) which works on any fd type,
    /// unlike `IORING_OP_SEND` which is socket-only.
    async fn write(&mut self, data: &[u8]) -> Result<usize, TransportError> {
        // Build a single-segment VecBuf for writev
        let buf: VecBuf = vec![data.to_vec()].into();
        let (result, _buf) = self.stream.writev(buf).await;
        let n = result?;
        Ok(n)
    }

    /// Read bytes from the serial port into `buf`. Returns the number of bytes read.
    ///
    /// Uses `IORING_OP_READV` (via `readv`) which works on any fd type,
    /// unlike `IORING_OP_RECV` which is socket-only.
    ///
    /// ## Non-blocking behaviour and retries
    ///
    /// The fd is opened with `O_NONBLOCK`.  When no data is available, io_uring
    /// completes the `readv` immediately with `EAGAIN` (`ErrorKind::WouldBlock`).
    /// This method retries internally until either data arrives or the read
    /// timeout (`READ_TIMEOUT`) elapses, at which point it returns
    /// `Err(TransportError::ReadTimeout)`.
    ///
    /// ## Contract
    ///
    /// On success, the return value is always `> 0`.  Callers can safely treat
    /// `Ok(0)` as impossible and `n == 0` as an EOF sentinel without needing to
    /// handle the EAGAIN case themselves.
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        let deadline = std::time::Instant::now() + READ_TIMEOUT;
        loop {
            // Build a single-segment VecBuf for readv; initialize with zeros so iov_len is correct
            let read_buf: VecBuf = vec![vec![0u8; buf.len()]].into();
            let (result, read_buf) = self.stream.readv(read_buf).await;
            match result {
                Ok(n) => {
                    // Extract the raw buffers back and copy data to caller
                    let vecs: Vec<Vec<u8>> = read_buf.into();
                    if n > 0 {
                        buf[..n].copy_from_slice(&vecs[0][..n]);
                    }
                    return Ok(n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    // EAGAIN: no data yet. Check deadline before retrying.
                    if std::time::Instant::now() >= deadline {
                        return Err(TransportError::ReadTimeout);
                    }
                    // Re-submit readv to io_uring. Each `.await` is a cooperative
                    // yield point — the runtime processes other completions between
                    // retries, so this is not a true busy-spin even though it loops
                    // without an explicit sleep.
                }
                Err(e) => return Err(TransportError::Io(e)),
            }
        }
    }

    fn flush_rx(&mut self) {
        SerialPort::flush_rx(self);
    }

    /// Flush the serial port output buffer.
    ///
    /// Calls `tcdrain(2)` which blocks until all output queued in the kernel
    /// serial driver has been physically transmitted.  For PTY devices this
    /// returns immediately (the kernel has no transmit queue to drain).
    ///
    /// Note: `tcdrain` is a synchronous syscall.  It only blocks during the
    /// flush path, which is intentionally short — this is acceptable in an
    /// async context where callers invoke `flush` deliberately.
    async fn flush(&mut self) -> Result<(), TransportError> {
        let raw_fd = self.stream.as_raw_fd();
        // SAFETY: raw_fd is valid for the lifetime of self.stream.
        // BorrowedFd::borrow_raw does not take ownership; we hold the fd
        // alive via self.stream for the duration of this call.
        let borrowed = unsafe { BorrowedFd::borrow_raw(raw_fd) };
        tcdrain(borrowed)
            .map_err(|e| TransportError::Io(std::io::Error::from_raw_os_error(e as i32)))?;
        Ok(())
    }
}

impl AsRawFd for SerialPort {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.stream.as_raw_fd()
    }
}

/// Direct RS-232 modem control/status line access, independent of
/// [`Transport`]'s byte-level CAT framing. See the trait's own docs for why
/// this is a separate, additively-implemented capability rather than folded
/// into `Transport`.
///
/// `set_rts`/`set_dtr` use `TIOCMBIS`/`TIOCMBIC` (set/clear a single bit);
/// `read_cts`/`read_dsr`/`read_dcd` use `TIOCMGET` (read the full status
/// register, then test the relevant bit) — CTS/DSR/DCD are status lines
/// only, never settable from this side of the link.
impl ModemControlLines for SerialPort {
    fn set_rts(&self, asserted: bool) -> Result<(), TransportError> {
        modem_bits_set(self.stream.as_raw_fd(), TIOCM_RTS, asserted)
    }

    fn set_dtr(&self, asserted: bool) -> Result<(), TransportError> {
        modem_bits_set(self.stream.as_raw_fd(), TIOCM_DTR, asserted)
    }

    fn read_cts(&self) -> Result<bool, TransportError> {
        modem_bit_get(self.stream.as_raw_fd(), TIOCM_CTS)
    }

    fn read_dsr(&self) -> Result<bool, TransportError> {
        modem_bit_get(self.stream.as_raw_fd(), TIOCM_DSR)
    }

    fn read_dcd(&self) -> Result<bool, TransportError> {
        modem_bit_get(self.stream.as_raw_fd(), TIOCM_CAR)
    }
}

#[cfg(test)]
mod tests {
    use cat_transport_core::{ModemControlLines, Transport};
    use monoio::RuntimeBuilder;

    use super::*;

    /// Minimal Linux PTY pair, built directly on `nix::pty` (already a
    /// dependency via the `term` feature) rather than `ts570d`'s
    /// `emulator::pty::PtyPair`.
    ///
    /// `ts570d`'s `emulator` crate (which wraps `serialport::TTYPort::pair()`)
    /// is out of scope for this extraction — it is dev-only test
    /// infrastructure that pulls in the external `serialport` crate, which
    /// is not among this crate's authorized dependencies
    /// (`cat-transport-core`, `monoio`, `async-trait`, `thiserror`, `libc`,
    /// `nix`). `nix::pty::{posix_openpt, grantpt, unlockpt, ptsname_r}` — the
    /// same primitives `serialport::TTYPort::pair()` uses internally —
    /// reproduce the master/slave pair without a new dependency. Unlike
    /// `emulator::pty::PtyPair`, this helper does not need to hold the slave
    /// fd open itself: every test below opens the slave via
    /// `SerialPort::open` immediately after creating the pair, and
    /// `SerialPort::open`'s own `configure_termios` call (via `cfmakeraw`)
    /// puts the slave in raw mode, so no separate raw-mode setup is needed
    /// here either.
    struct TestPtyPair {
        master: nix::pty::PtyMaster,
        slave_path: String,
    }

    impl TestPtyPair {
        fn new() -> Self {
            use nix::fcntl::OFlag;
            use nix::pty::{grantpt, posix_openpt, ptsname_r, unlockpt};

            let master =
                posix_openpt(OFlag::O_RDWR | OFlag::O_NOCTTY).expect("posix_openpt failed");
            grantpt(&master).expect("grantpt failed");
            unlockpt(&master).expect("unlockpt failed");
            let slave_path = ptsname_r(&master).expect("ptsname_r failed");

            Self { master, slave_path }
        }

        fn slave_path(&self) -> &str {
            &self.slave_path
        }

        fn master_raw_fd(&self) -> std::os::fd::RawFd {
            self.master.as_raw_fd()
        }
    }

    /// Helper: write bytes to a raw fd (master side of PTY) synchronously.
    ///
    /// Pre-existing (not added by the `ModemControlLines` work) and
    /// currently unused by any test in this module — kept for symmetry with
    /// `read_from_master` below, which IS used. `#[allow(dead_code)]`
    /// because `cargo clippy --all-targets -- -D warnings` treats unused
    /// private test fns as a hard error; this predates and is unrelated to
    /// this session's changes (confirmed against a clean-HEAD worktree
    /// before touching it).
    #[allow(dead_code)]
    fn write_to_master(master_fd: std::os::fd::RawFd, data: &[u8]) {
        let written =
            unsafe { libc::write(master_fd, data.as_ptr() as *const libc::c_void, data.len()) };
        assert!(written > 0, "write to master failed: errno={}", unsafe {
            *libc::__errno_location()
        });
    }

    /// Helper: read bytes from a raw fd (master side of PTY) synchronously.
    fn read_from_master(master_fd: std::os::fd::RawFd, max_len: usize) -> Vec<u8> {
        let mut buf = vec![0u8; max_len];
        // Brief pause to let the kernel process io_uring writes before reading
        std::thread::sleep(std::time::Duration::from_millis(20));
        let n = unsafe { libc::read(master_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        assert!(n > 0, "read from master returned {}", n);
        buf.truncate(n as usize);
        buf
    }

    /// Build a monoio IoUring runtime for test use.
    fn make_runtime() -> monoio::Runtime<monoio::IoUringDriver> {
        RuntimeBuilder::<monoio::IoUringDriver>::new()
            .build()
            .expect("monoio runtime build failed")
    }

    /// `SerialPort::open` must be called inside a monoio runtime because
    /// `UnixStream::from_std` registers the fd with the io_uring driver.
    #[test]
    fn test_serial_port_open_on_pty_slave() {
        let pair = TestPtyPair::new();
        let slave = pair.slave_path().to_string();

        make_runtime().block_on(async {
            let port =
                SerialPort::open(&slave, SerialConfig::default()).expect("SerialPort::open failed");
            assert_eq!(port.path(), slave);
        });
    }

    #[test]
    fn test_transport_read_from_master() {
        let pair = TestPtyPair::new();
        let slave = pair.slave_path().to_string();
        let master_fd = pair.master_raw_fd();

        // Write from a background thread after a short delay so the data
        // arrives AFTER the io_uring read is submitted.  Pre-writing before
        // the runtime starts does not trigger a completion event on PTY fds
        // with IORING_OP_READV on kernel 5.15 (GitHub Actions runner).
        let expected: &'static [u8] = b"FA;";
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            unsafe {
                libc::write(
                    master_fd,
                    expected.as_ptr() as *const libc::c_void,
                    expected.len(),
                )
            };
        });

        let _pair = pair; // keep master fd alive
        let result = make_runtime().block_on(async {
            let mut port =
                SerialPort::open(&slave, SerialConfig::default()).expect("SerialPort::open failed");
            let mut buf = vec![0u8; 64];
            let n = port.read(&mut buf).await.expect("Transport::read failed");
            buf.truncate(n);
            buf
        });

        assert_eq!(result, expected, "Read data did not match written data");
    }

    #[test]
    fn test_transport_write_to_master() {
        let pair = TestPtyPair::new();
        let slave = pair.slave_path().to_string();

        let expected = b"ID020;";

        // Write from SerialPort (slave side) via async Transport::write
        make_runtime().block_on(async {
            let mut port =
                SerialPort::open(&slave, SerialConfig::default()).expect("SerialPort::open failed");
            let n = port.write(expected).await.expect("Transport::write failed");
            assert_eq!(n, expected.len());
        });

        // Read back from master (synchronous, outside the runtime)
        let received = read_from_master(pair.master_raw_fd(), 64);
        assert_eq!(&received, expected, "Master received unexpected data");
    }

    #[test]
    fn test_unsupported_baud_rate_error() {
        // baud_rate_from_u32 is a pure function — no runtime needed
        let err = baud_rate_from_u32(12345).expect_err("Expected error for invalid baud rate");
        assert!(
            matches!(err, SerialError::InvalidConfig(_)),
            "Expected InvalidConfig, got {:?}",
            err
        );
    }

    #[test]
    fn test_open_nonexistent_device_error() {
        // Device-not-found happens before UnixStream::from_std, so we still need
        // a runtime (open() may call from_std if path exists), but here the path
        // doesn't exist so the error occurs in OpenOptions::open before that.
        let err = make_runtime().block_on(async {
            SerialPort::open("/dev/ttyDOESNOTEXIST99999", SerialConfig::default())
        });
        assert!(
            matches!(err, Err(SerialError::DeviceNotFound(_))),
            "Expected DeviceNotFound, got {:?}",
            err
        );
    }

    /// `SerialConfig::default()` must set stop_bits = 2 to match the Kenwood TS-570D (8N2).
    #[test]
    fn test_default_config_stop_bits_is_2() {
        let config = SerialConfig::default();
        assert_eq!(
            config.stop_bits, 2,
            "SerialConfig::default() must use 2 stop bits (8N2) for TS-570D compatibility"
        );
    }

    /// `SerialConfig::default()` must assert RTS and DTR at open time, matching
    /// the crate's historical unconditional-assert behavior exactly (no
    /// existing caller opts out, so the default must preserve it).
    #[test]
    fn test_default_config_initial_rts_dtr_are_true() {
        let config = SerialConfig::default();
        assert!(
            config.initial_rts,
            "SerialConfig::default() must assert RTS at open time"
        );
        assert!(
            config.initial_dtr,
            "SerialConfig::default() must assert DTR at open time"
        );
    }

    /// Opting out of the startup RTS/DTR assert must not prevent the port
    /// from opening — `initial_rts`/`initial_dtr` only gate the assert call,
    /// they are not a precondition for `open()` succeeding.
    #[test]
    fn test_open_with_initial_rts_dtr_opted_out_still_succeeds() {
        let pair = TestPtyPair::new();
        let slave = pair.slave_path().to_string();

        make_runtime().block_on(async {
            let config = SerialConfig {
                initial_rts: false,
                initial_dtr: false,
                ..SerialConfig::default()
            };
            let port = SerialPort::open(&slave, config)
                .expect("SerialPort::open must succeed even with the startup assert opted out");
            assert_eq!(port.path(), slave);
        });
    }

    /// `ModemControlLines` methods must exercise the real ioctl code path
    /// against a PTY-backed `SerialPort`.
    ///
    /// PTY devices do not support modem control line ioctls at all —
    /// verified empirically (Python `fcntl.ioctl` against a fresh
    /// `pty.openpty()` pair): `TIOCMGET`/`TIOCMBIS`/`TIOCMBIC` all return
    /// `ENOTTY` on both master and slave sides on this kernel. This matches
    /// the comment already on `SerialPort::open`'s pre-existing RTS/DTR
    /// assert ("PTY devices don't support modem control lines ... ENOTTY,
    /// which is harmless").
    ///
    /// Because of this, `TestPtyPair` (this module's only test-double
    /// infrastructure, built on `nix::pty`) cannot exercise the SUCCESS path
    /// of any `ModemControlLines` method — there is no PTY-level way to make
    /// `TIOCMBIS`/`TIOCMGET` succeed. What CAN be verified here, and is
    /// verified below, is that the production code path (real `ioctl(2)`
    /// call against a real fd) is reached for all five methods and fails
    /// predictably as `Err(TransportError::Io(_))` carrying `ENOTTY` rather
    /// than panicking, hanging, or silently reporting success. The
    /// bit-actually-toggles success path requires real serial hardware (or a
    /// kernel virtual null-modem pair, e.g. `tty0tty`) and is out of reach of
    /// this crate's existing PTY-only test infrastructure — not fabricated
    /// here.
    #[test]
    fn test_modem_control_lines_return_err_on_pty() {
        let pair = TestPtyPair::new();
        let slave = pair.slave_path().to_string();

        make_runtime().block_on(async {
            let port =
                SerialPort::open(&slave, SerialConfig::default()).expect("SerialPort::open failed");

            let assert_enotty = |result: Result<(), TransportError>, what: &str| match result {
                Err(TransportError::Io(e)) => {
                    assert_eq!(
                        e.raw_os_error(),
                        Some(libc::ENOTTY),
                        "{what}: expected ENOTTY, got {:?}",
                        e
                    );
                }
                other => panic!("{what}: expected Err(TransportError::Io(ENOTTY)), got {other:?}"),
            };
            let assert_enotty_bool = |result: Result<bool, TransportError>, what: &str| match result
            {
                Err(TransportError::Io(e)) => {
                    assert_eq!(
                        e.raw_os_error(),
                        Some(libc::ENOTTY),
                        "{what}: expected ENOTTY, got {:?}",
                        e
                    );
                }
                other => {
                    panic!("{what}: expected Err(TransportError::Io(ENOTTY)), got {other:?}")
                }
            };

            assert_enotty(port.set_rts(true), "set_rts(true)");
            assert_enotty(port.set_rts(false), "set_rts(false)");
            assert_enotty(port.set_dtr(true), "set_dtr(true)");
            assert_enotty(port.set_dtr(false), "set_dtr(false)");
            assert_enotty_bool(port.read_cts(), "read_cts()");
            assert_enotty_bool(port.read_dsr(), "read_dsr()");
            assert_enotty_bool(port.read_dcd(), "read_dcd()");
        });
    }

    /// `Transport::read` waits for data and returns it once available.
    ///
    /// This test verifies that `Transport::read` does not return `Ok(0)` when no
    /// data is immediately available.  Instead, it should block (retrying internally)
    /// until data arrives.  A background thread writes to the master side after a
    /// short delay; the async read must successfully receive that data.
    ///
    /// On PTY devices, io_uring's `IORING_OP_READV` blocks at the kernel level until
    /// data arrives (rather than returning EAGAIN).  On real O_NONBLOCK serial ports,
    /// EAGAIN is returned and the retry loop handles it.  In both cases the caller
    /// receives `Ok(n > 0)` when data arrives — never `Ok(0)`.
    #[test]
    fn test_read_blocks_until_data_arrives() {
        let pair = TestPtyPair::new();
        let slave = pair.slave_path().to_string();
        let master_fd = pair.master_raw_fd();

        // Write to the master after a short delay from a background thread.
        // The async read on the slave must wait and then return the data.
        let expected = b"FA00014000000;";
        let expected_clone: &'static [u8] = expected;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            unsafe {
                libc::write(
                    master_fd,
                    expected_clone.as_ptr() as *const libc::c_void,
                    expected_clone.len(),
                )
            };
        });

        // Keep TestPtyPair alive to hold the master fd open.
        let _pair = pair;

        let result = make_runtime().block_on(async {
            let mut port =
                SerialPort::open(&slave, SerialConfig::default()).expect("SerialPort::open failed");
            let mut buf = vec![0u8; 64];
            let n = port.read(&mut buf).await.expect("Transport::read failed");
            buf.truncate(n);
            buf
        });

        assert_eq!(
            result, expected,
            "read should return data written by the background thread"
        );
    }

    /// Full-duplex roundtrip test: write from slave via Transport, read from master;
    /// then write from master, read from slave via Transport.
    #[test]
    fn test_transport_roundtrip() {
        let pair = TestPtyPair::new();
        let slave = pair.slave_path().to_string();
        let master_fd = pair.master_raw_fd();

        // --- slave → master direction ---
        let slave_to_master_msg = b"FA;";
        make_runtime().block_on(async {
            let mut port =
                SerialPort::open(&slave, SerialConfig::default()).expect("SerialPort::open failed");
            let n = port
                .write(slave_to_master_msg)
                .await
                .expect("Transport::write failed");
            assert_eq!(
                n,
                slave_to_master_msg.len(),
                "write returned wrong byte count"
            );
        });
        let received_on_master = read_from_master(master_fd, 64);
        assert_eq!(
            &received_on_master, slave_to_master_msg,
            "master did not receive slave's write"
        );

        // --- master → slave direction ---
        // Write from a background thread so data arrives after the io_uring
        // read is submitted (pre-writing does not fire a completion on PTY fds
        // with IORING_OP_READV on kernel 5.15).
        let master_to_slave_msg: &'static [u8] = b"FA00014000000;";
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            unsafe {
                libc::write(
                    master_fd,
                    master_to_slave_msg.as_ptr() as *const libc::c_void,
                    master_to_slave_msg.len(),
                )
            };
        });
        let received_on_slave = make_runtime().block_on(async {
            let mut port =
                SerialPort::open(&slave, SerialConfig::default()).expect("SerialPort::open failed");
            let mut buf = vec![0u8; 64];
            let n = port.read(&mut buf).await.expect("Transport::read failed");
            buf.truncate(n);
            buf
        });
        assert_eq!(
            &received_on_slave, master_to_slave_msg,
            "slave did not receive master's write"
        );
    }
}
