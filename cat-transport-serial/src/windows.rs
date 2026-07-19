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

//! Windows COM-port serial backend.
//!
//! Per `docs/adr/0004-windows-serial-backend.md` §1/§2/§3/§4 and
//! `planning/architect/task_plan.md`'s Task 7/Task 8: this module
//! implements the full Windows `SerialPort` -- `open`, `DCB` configuration
//! (`configure_dcb`), `SetCommTimeouts` (Task 7), plus (Task 8) the
//! background worker thread that drives blocking, non-overlapped
//! `ReadFile`/`WriteFile`, `impl Transport for SerialPort`, and `impl
//! ModemControlLines for SerialPort`.
//!
//! Built directly against `windows-sys` (the official Microsoft FFI-bindings
//! crate; no `winapi`, no `Win32_System_IO`/`OVERLAPPED`-based *async* I/O
//! design -- ADR 0004 §3 deliberately avoids reimplementing IOCP-based
//! overlapped I/O on top of this). `windows-sys` generates raw `extern
//! "system"` bindings with no ergonomic wrapper: in particular `DCB`'s
//! packed boolean/2-bit sub-fields (`fBinary`, `fParity`, `fOutxCtsFlow`,
//! `fRtsControl`, ...) are exposed as one opaque `_bitfield: u32`, not named
//! accessor methods -- confirmed by inspecting the locally-vendored
//! `windows-sys` 0.52.0 source
//! (`src/Windows/Win32/Devices/Communication/mod.rs`), which has no
//! generated `fParity`/`set_fParity`-style methods anywhere in the crate.
//! [`dcb_bits`] below hand-rolls the documented `winbase.h` bit layout for
//! exactly the sub-fields this module needs to set.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::mpsc;
use std::thread;

use async_trait::async_trait;
use windows_sys::Win32::Devices::Communication::{
    EscapeCommFunction, GetCommModemStatus, GetCommState, PurgeComm, SetCommState, SetCommTimeouts,
    CLRDTR, CLRRTS, COMMTIMEOUTS, DCB, EVENPARITY, MS_CTS_ON, MS_DSR_ON, MS_RLSD_ON, NOPARITY,
    ODDPARITY, ONESTOPBIT, PURGE_RXCLEAR, SETDTR, SETRTS, TWOSTOPBITS,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND,
    GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING,
};

use cat_transport_core::{ModemControlLines, Transport, TransportError};

use crate::config::{FlowControl, Parity, SerialConfig};
use crate::oneshot;
use crate::timeouts::READ_TIMEOUT;
use crate::{SerialError, SerialResult};

/// Hand-rolled `DCB._bitfield` layout (`winbase.h`'s `_DCB` packed-bitfield
/// members), since `windows-sys` exposes the whole thing as one opaque
/// `u32` with no named accessors (see this module's doc comment).
///
/// Bit order and widths per the documented Win32 `DCB` struct layout
/// (LSB-first): `fBinary`(1) `fParity`(1) `fOutxCtsFlow`(1)
/// `fOutxDsrFlow`(1) `fDtrControl`(2) `fDsrSensitivity`(1)
/// `fTXContinueOnXoff`(1) `fOutX`(1) `fInX`(1) `fErrorChar`(1) `fNull`(1)
/// `fRtsControl`(2) `fAbortOnError`(1) `fDummy2`(17, reserved/unused here).
mod dcb_bits {
    pub(super) const F_BINARY: u32 = 1 << 0;
    pub(super) const F_PARITY: u32 = 1 << 1;
    pub(super) const F_OUTX_CTS_FLOW: u32 = 1 << 2;
    pub(super) const F_OUTX_DSR_FLOW: u32 = 1 << 3;
    pub(super) const F_DTR_CONTROL_SHIFT: u32 = 4;
    pub(super) const F_DSR_SENSITIVITY: u32 = 1 << 6;
    pub(super) const F_TX_CONTINUE_ON_XOFF: u32 = 1 << 7;
    pub(super) const F_OUT_X: u32 = 1 << 8;
    pub(super) const F_IN_X: u32 = 1 << 9;
    pub(super) const F_ERROR_CHAR: u32 = 1 << 10;
    pub(super) const F_NULL: u32 = 1 << 11;
    pub(super) const F_RTS_CONTROL_SHIFT: u32 = 12;
    pub(super) const F_ABORT_ON_ERROR: u32 = 1 << 14;

    /// `fDtrControl`/`fRtsControl` values (each a 2-bit sub-field). Not
    /// exposed as named constants by `windows-sys` (only real API surface
    /// gets generated bindings; these are documented bit *meanings*, not
    /// linkable symbols), so hand-rolled here per `winbase.h`.
    pub(super) const DTR_CONTROL_ENABLE: u32 = 1;
    pub(super) const RTS_CONTROL_ENABLE: u32 = 1;
    pub(super) const RTS_CONTROL_HANDSHAKE: u32 = 2;

    /// Set or clear a single-bit sub-field identified by `mask`.
    pub(super) fn set_bit(bitfield: &mut u32, mask: u32, value: bool) {
        if value {
            *bitfield |= mask;
        } else {
            *bitfield &= !mask;
        }
    }

    /// Set a 2-bit sub-field (`fDtrControl`/`fRtsControl`) at `shift` to `value` (0..=3).
    pub(super) fn set_2bit(bitfield: &mut u32, shift: u32, value: u32) {
        let mask = 0b11 << shift;
        *bitfield = (*bitfield & !mask) | ((value << shift) & mask);
    }
}

/// A raw Win32 `HANDLE`, newtyped so it can be stored in [`SerialPort`] and
/// (in Task 8) handed to a worker thread.
///
/// # Safety (`unsafe impl Send`)
///
/// A Win32 `HANDLE` to an open serial port is not inherently thread-affine
/// the way some Windows objects are (e.g. certain UI handles tied to the
/// thread that created them) -- `ReadFile`/`WriteFile`/`EscapeCommFunction`/
/// `GetCommModemStatus`/`CloseHandle` are all documented as callable from
/// any thread against the same `HANDLE` value. Sending a `HANDLE` to another
/// thread and using it there is therefore safe *as long as access is
/// externally synchronized* -- this type carries no synchronization of its
/// own, and correctness rests entirely on the design this crate builds on
/// top of it (ADR 0004 §1/§4): a single background worker thread owns this
/// `HANDLE` for `ReadFile`/`WriteFile` (Task 8), while `ModemControlLines`
/// methods (`EscapeCommFunction`/`GetCommModemStatus`, also Task 8) call
/// directly on the calling thread against the same `HANDLE` -- a pattern the
/// ADR itself justifies ("`EscapeCommFunction`/`GetCommModemStatus` on the
/// calling thread do not race meaningfully against a concurrent blocking
/// `ReadFile` on the worker thread"). This newtype only asserts the
/// primitive fact that the raw handle value itself is safe to move across a
/// thread boundary; it does not by itself guarantee synchronized access --
/// that guarantee comes from the single-owner worker-thread pattern the
/// surrounding `SerialPort` design enforces, not from this type.
#[derive(Debug, Clone, Copy)]
struct RawHandle(HANDLE);

// SAFETY: see the doc comment on `RawHandle` above.
unsafe impl Send for RawHandle {}

/// A request sent to [`SerialPort`]'s background worker thread over an
/// `std::sync::mpsc` channel, per ADR 0004 §1's design: a dedicated
/// `std::thread` owns the raw `HANDLE` and performs blocking, non-overlapped
/// `ReadFile`/`WriteFile` in a loop, replying via one half of the
/// `oneshot.rs` completion primitive (ADR 0004 dispatch queue Task 6).
enum WorkerRequest {
    /// Read up to `len` bytes. Replies with `Ok(bytes)` where `bytes.len()`
    /// is always `> 0` -- a zero-byte `ReadFile` completion (the configured
    /// `ReadTotalTimeoutConstant`, `set_comm_timeouts` above, elapsed with
    /// no data) is translated to `Err(TransportError::ReadTimeout)` inside
    /// the worker, before it ever replies, so `Transport::read`'s "`Ok(n)`
    /// is always `> 0`" contract holds exactly as it does on Linux (see
    /// `io_uring.rs`'s own doc comment on this same contract).
    Read {
        len: usize,
        reply: oneshot::CompletionTx<Result<Vec<u8>, TransportError>>,
    },
    /// Write `data` in full. Replies with `Ok(bytes_written)` or an I/O
    /// error.
    Write {
        data: Vec<u8>,
        reply: oneshot::CompletionTx<Result<usize, TransportError>>,
    },
}

/// The worker thread body: owns `handle` for its whole lifetime, performing
/// blocking, non-overlapped `ReadFile`/`WriteFile` calls driven by
/// `WorkerRequest`s received over `rx`. Exits its loop (ending the thread)
/// when `rx.recv()` returns `Err` -- i.e. [`SerialPort`]'s `Drop` impl has
/// dropped the request sender, per ADR 0004 §1's explicit shutdown
/// sequencing.
fn worker_loop(handle: RawHandle, rx: mpsc::Receiver<WorkerRequest>) {
    while let Ok(request) = rx.recv() {
        match request {
            WorkerRequest::Read { len, reply } => {
                reply.send(worker_read(handle, len));
            }
            WorkerRequest::Write { data, reply } => {
                reply.send(worker_write(handle, &data));
            }
        }
    }
}

/// Perform one blocking, non-overlapped `ReadFile` call for up to `len`
/// bytes. See [`WorkerRequest::Read`] for the zero-bytes-means-timeout
/// mapping.
fn worker_read(handle: RawHandle, len: usize) -> Result<Vec<u8>, TransportError> {
    let mut buf = vec![0u8; len];
    let mut bytes_read: u32 = 0;
    // SAFETY: `handle.0` is a valid open serial-port handle for the
    // lifetime of the worker thread (owned by `SerialPort`, closed only
    // after this thread is joined -- see `Drop for SerialPort`); `buf` is a
    // valid, writable buffer of `len` bytes for the duration of this call;
    // `&mut bytes_read` is a valid `*mut u32`; `lpOverlapped` is
    // `null_mut()` -- this is deliberately non-overlapped (blocking) I/O,
    // confined to this dedicated worker thread, per ADR 0004 §3.
    let ok = unsafe {
        ReadFile(
            handle.0,
            buf.as_mut_ptr(),
            len as u32,
            &mut bytes_read,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(TransportError::Io(std::io::Error::last_os_error()));
    }
    let n = bytes_read as usize;
    if n == 0 {
        return Err(TransportError::ReadTimeout);
    }
    buf.truncate(n);
    Ok(buf)
}

/// Perform one blocking, non-overlapped `WriteFile` call for the whole of
/// `data`.
///
/// A write that does not complete in full within the configured
/// `WriteTotalTimeoutConstant` (`set_comm_timeouts` above) still returns
/// success from `WriteFile` itself, but with `lpNumberOfBytesWritten` less
/// than the requested length -- the well-established Win32 serial-I/O
/// behavior (matching `pyserial`'s own `serialwin32.py` handling, which this
/// crate has no direct access to re-verify against real hardware in this
/// sandboxed environment, but is the documented/standard interpretation):
/// this short write is treated as `Err(TransportError::WriteTimeout)`
/// rather than `Ok(partial_n)`, giving `WriteTotalTimeoutConstant` a real,
/// first caller on this platform (ADR 0004 §3).
fn worker_write(handle: RawHandle, data: &[u8]) -> Result<usize, TransportError> {
    let mut bytes_written: u32 = 0;
    // SAFETY: same reasoning as `worker_read` above -- `data` is a valid,
    // live buffer for the duration of this call; `lpOverlapped` is
    // `null_mut()` (blocking, non-overlapped I/O).
    let ok = unsafe {
        WriteFile(
            handle.0,
            data.as_ptr(),
            data.len() as u32,
            &mut bytes_written,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(TransportError::Io(std::io::Error::last_os_error()));
    }
    let n = bytes_written as usize;
    if n < data.len() {
        return Err(TransportError::WriteTimeout);
    }
    Ok(n)
}

/// Build the [`TransportError`] used when the worker thread is unexpectedly
/// gone -- either because sending a [`WorkerRequest`] failed (the worker's
/// `mpsc::Receiver` was dropped, i.e. its thread already exited -- most
/// plausibly via a panic, since [`SerialPort`]'s ordinary shutdown path
/// (`Drop`) always joins the thread before this could otherwise be
/// observed) or because the paired `CompletionRx` resolved to
/// `Err(`[`oneshot::Canceled`]`)` (the `CompletionTx` was dropped without
/// sending -- same underlying cause). `TransportError` has no dedicated
/// "worker thread gone" variant -- ADR 0004 explicitly expects no new
/// `SerialError` variant to be needed for the open/configure path, and the
/// same minimal-surface-change spirit applies here; `Io` with a synthetic
/// `BrokenPipe` error models "the other end of this request/reply channel
/// is gone" the same way it would for a genuinely broken pipe.
fn worker_gone_error() -> TransportError {
    TransportError::Io(std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        "cat-transport-serial: Windows worker thread is no longer running",
    ))
}

/// Windows COM-port serial port.
///
/// Open with [`SerialPort::open`]. Implements [`cat_transport_core::
/// Transport`] (`read`/`write` via the background worker thread; `flush`/
/// `flush_rx` directly and synchronously, per ADR 0004 §3) and
/// [`cat_transport_core::ModemControlLines`] (also direct and synchronous,
/// per ADR 0004 §4) -- see this module's doc comment.
pub struct SerialPort {
    handle: RawHandle,
    /// Device path this port was opened on (the caller-supplied form, e.g.
    /// `"COM3"` -- not the `\\.\`-prefixed form actually passed to
    /// `CreateFileW`), kept for diagnostics/parity with the Linux type.
    path: String,
    /// Sender half of the worker thread's request channel. `Some` for the
    /// entire lifetime of an open `SerialPort` -- only ever taken (and
    /// dropped) by `Drop::drop`, so the worker's `recv()` observes
    /// disconnection at exactly the right moment (ADR 0004 §1's shutdown
    /// sequencing: drop the sender first, THEN join the thread, THEN
    /// `CloseHandle`).
    request_tx: Option<mpsc::Sender<WorkerRequest>>,
    /// The worker thread's `JoinHandle`. `Some` for the entire lifetime of
    /// an open `SerialPort` -- only ever taken (and joined) by `Drop::drop`.
    worker: Option<thread::JoinHandle<()>>,
}

impl std::fmt::Debug for SerialPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SerialPort")
            .field("path", &self.path)
            .field("handle", &self.handle.0)
            .finish()
    }
}

impl SerialPort {
    /// Open a COM port at `path` (e.g. `"COM3"`) with the given `config`.
    ///
    /// `path` is prefixed with `\\.\` if not already present -- required
    /// for COM10 and above, harmless for COM1-9 (ADR 0004 §3).
    ///
    /// Configures the port per every field of `config` (`configure_dcb`)
    /// and sets non-overlapped read/write timeouts (`SetCommTimeouts`)
    /// before returning. Spawns the background worker thread that drives
    /// `ReadFile`/`WriteFile` (ADR 0004 §1), then asserts RTS/DTR per
    /// `config.initial_rts`/`config.initial_dtr` (both default `true`) via
    /// [`ModemControlLines`], identical sequencing to the Linux
    /// implementation (`io_uring.rs`'s `SerialPort::open`).
    pub fn open(path: &str, config: SerialConfig) -> SerialResult<Self> {
        let full_path = if path.starts_with(r"\\.\") {
            path.to_string()
        } else {
            format!(r"\\.\{path}")
        };
        let wide_path: Vec<u16> = OsStr::new(&full_path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // SAFETY: `wide_path` is a valid, NUL-terminated, live UTF-16 buffer
        // for the duration of this call (it outlives the call on the stack
        // above). `lpSecurityAttributes` is `null` (default security, no
        // inheritance) and `hTemplateFile` is `null_mut()` (unused without
        // `OPEN_ALWAYS`/`CREATE_*`), both valid per `CreateFileW`'s
        // documented contract for `OPEN_EXISTING`.
        let handle = unsafe {
            CreateFileW(
                wide_path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0, // dwShareMode: exclusive access, matching Linux's default (no O_SHLOCK-equivalent request)
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            return Err(map_create_file_error(path));
        }

        if let Err(e) = configure_dcb(handle, &config) {
            // SAFETY: `handle` is the valid, open handle returned by the
            // `CreateFileW` call above; nothing else has taken ownership of
            // it yet on this early-return path.
            unsafe { CloseHandle(handle) };
            return Err(e);
        }

        if let Err(e) = set_comm_timeouts(handle) {
            // SAFETY: same as above.
            unsafe { CloseHandle(handle) };
            return Err(e);
        }

        let raw_handle = RawHandle(handle);
        let (request_tx, request_rx) = mpsc::channel();
        let worker = thread::spawn(move || worker_loop(raw_handle, request_rx));

        let port = Self {
            handle: raw_handle,
            path: path.to_string(),
            request_tx: Some(request_tx),
            worker: Some(worker),
        };

        // Assert RTS and/or DTR per `config` (both default `true`,
        // preserving this crate's historical unconditional-assert behavior
        // exactly -- see the identical comment on the Linux implementation,
        // `io_uring.rs`'s `SerialPort::open`). Errors are ignored
        // deliberately, mirroring Linux's "harmless on a device that
        // doesn't support it" handling.
        if config.initial_rts {
            let _ = port.set_rts(true);
        }
        if config.initial_dtr {
            let _ = port.set_dtr(true);
        }

        Ok(port)
    }

    /// Get the device path this port was opened on (parity with the Linux
    /// `SerialPort::path`).
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The sender half of the worker thread's request channel.
    ///
    /// # Panics
    ///
    /// Only if called after `Drop::drop` has already run (impossible
    /// through the public API -- `&self`/`&mut self` methods cannot be
    /// called on an already-dropped value), so this can never actually
    /// panic in practice; the `expect` documents that invariant rather than
    /// papering over a real `None` case with a silently-wrong fallback.
    fn request_tx(&self) -> &mpsc::Sender<WorkerRequest> {
        self.request_tx
            .as_ref()
            .expect("SerialPort::request_tx is only None after Drop::drop")
    }
}

#[async_trait(?Send)]
impl Transport for SerialPort {
    /// Send a [`WorkerRequest::Write`] to the worker thread and `.await`
    /// the reply. See [`worker_write`] for the byte-count/timeout mapping.
    async fn write(&mut self, data: &[u8]) -> Result<usize, TransportError> {
        let (tx, rx) = oneshot::channel();
        self.request_tx()
            .send(WorkerRequest::Write {
                data: data.to_vec(),
                reply: tx,
            })
            .map_err(|_| worker_gone_error())?;
        rx.await.map_err(|_| worker_gone_error())?
    }

    /// Send a [`WorkerRequest::Read`] for `buf.len()` bytes to the worker
    /// thread and `.await` the reply, copying the result into `buf`. See
    /// [`WorkerRequest::Read`] for the "`Ok(n)` is always `> 0`" contract
    /// this preserves exactly.
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        let (tx, rx) = oneshot::channel();
        self.request_tx()
            .send(WorkerRequest::Read {
                len: buf.len(),
                reply: tx,
            })
            .map_err(|_| worker_gone_error())?;
        let bytes = rx.await.map_err(|_| worker_gone_error())??;
        let n = bytes.len();
        buf[..n].copy_from_slice(&bytes);
        Ok(n)
    }

    /// `PurgeComm(handle, PURGE_RXCLEAR)`, called directly and
    /// synchronously -- NOT routed through the worker thread. Mirrors
    /// [`ModemControlLines`]'s own "no I/O wait, direct call on the calling
    /// thread" reasoning (ADR 0004 §3/§4): discarding unread receive-buffer
    /// bytes is not a blocking I/O wait the way `ReadFile`/`WriteFile` are.
    fn flush_rx(&mut self) {
        // SAFETY: `self.handle.0` is a valid open serial-port handle for
        // the lifetime of `self`.
        unsafe {
            PurgeComm(self.handle.0, PURGE_RXCLEAR);
        }
    }

    /// `FlushFileBuffers(handle)`, called directly and synchronously inside
    /// this `async fn`'s body -- NOT routed through the worker thread. This
    /// is a deliberate, narrow exception to the read/write worker-thread
    /// design, mirroring `io_uring.rs`'s own `tcdrain`-inside-async-body
    /// exception exactly (ADR 0004 §3): `FlushFileBuffers` is a short,
    /// synchronous Win32 call, and blocking briefly here is acceptable in
    /// an async context where callers invoke `flush` deliberately -- the
    /// same justification `io_uring.rs` gives for `tcdrain`.
    async fn flush(&mut self) -> Result<(), TransportError> {
        // SAFETY: `self.handle.0` is a valid open serial-port handle for
        // the lifetime of `self`.
        let ok = unsafe { FlushFileBuffers(self.handle.0) };
        if ok == 0 {
            return Err(TransportError::Io(std::io::Error::last_os_error()));
        }
        Ok(())
    }
}

/// Direct, synchronous [`EscapeCommFunction`] call on the calling thread --
/// NOT routed through the worker thread or the completion primitive, per
/// ADR 0004 §4 (mirroring the Linux `ioctl`-based [`ModemControlLines`]
/// implementation's identical "no I/O wait" reasoning, `io_uring.rs`).
fn escape_comm_function(handle: RawHandle, function: u32) -> Result<(), TransportError> {
    // SAFETY: `handle.0` is a valid open serial-port handle for the
    // lifetime of the `SerialPort` this is called through.
    let ok = unsafe { EscapeCommFunction(handle.0, function) };
    if ok == 0 {
        return Err(TransportError::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Direct, synchronous [`GetCommModemStatus`] call on the calling thread,
/// testing whether `bit` is set in the returned modem status -- NOT routed
/// through the worker thread, same reasoning as [`escape_comm_function`].
fn modem_status_bit(handle: RawHandle, bit: u32) -> Result<bool, TransportError> {
    let mut status: u32 = 0;
    // SAFETY: `handle.0` is a valid open serial-port handle; `&mut status`
    // is a valid `*mut u32`.
    let ok = unsafe { GetCommModemStatus(handle.0, &mut status) };
    if ok == 0 {
        return Err(TransportError::Io(std::io::Error::last_os_error()));
    }
    Ok(status & bit != 0)
}

/// Direct RS-232 modem control/status line access, independent of
/// [`Transport`]'s byte-level CAT framing -- see the trait's own docs for
/// why this is a separate, additively-implemented capability rather than
/// folded into `Transport`.
///
/// `set_rts`/`set_dtr` use `EscapeCommFunction` (`SETRTS`/`CLRRTS`/
/// `SETDTR`/`CLRDTR`); `read_cts`/`read_dsr`/`read_dcd` use
/// `GetCommModemStatus`, testing `MS_CTS_ON`/`MS_DSR_ON`/`MS_RLSD_ON` --
/// the Windows equivalents of Linux's `TIOCMBIS`/`TIOCMBIC`/`TIOCMGET`
/// (`io_uring.rs`).
impl ModemControlLines for SerialPort {
    fn set_rts(&self, asserted: bool) -> Result<(), TransportError> {
        escape_comm_function(self.handle, if asserted { SETRTS } else { CLRRTS })
    }

    fn set_dtr(&self, asserted: bool) -> Result<(), TransportError> {
        escape_comm_function(self.handle, if asserted { SETDTR } else { CLRDTR })
    }

    fn read_cts(&self) -> Result<bool, TransportError> {
        modem_status_bit(self.handle, MS_CTS_ON)
    }

    fn read_dsr(&self) -> Result<bool, TransportError> {
        modem_status_bit(self.handle, MS_DSR_ON)
    }

    fn read_dcd(&self) -> Result<bool, TransportError> {
        modem_status_bit(self.handle, MS_RLSD_ON)
    }
}

impl Drop for SerialPort {
    /// Drop the request sender (the worker's `recv()` then returns `Err`,
    /// ending its loop), join the worker thread, then `CloseHandle` -- this
    /// exact sequencing, per ADR 0004 §1, is load-bearing for clean
    /// shutdown: joining before dropping the sender would hang forever (the
    /// worker would still be blocked in `recv()` with nothing left to wake
    /// it), and `CloseHandle`ing before the worker thread has actually
    /// exited would let its in-flight `ReadFile`/`WriteFile` call race a
    /// closed handle.
    fn drop(&mut self) {
        // Drop the sender FIRST (not `worker.join()` first) -- see the
        // ordering rationale above. `Option::take` leaves `None` behind,
        // dropping the real `mpsc::Sender` immediately (not merely a
        // clone), which is what lets the worker's `recv()` observe
        // disconnection.
        self.request_tx.take();

        if let Some(worker) = self.worker.take() {
            // A worker-thread panic (e.g. a genuinely unexpected Win32
            // failure mid-request) surfaces as a `Canceled`/broken-pipe
            // `TransportError` to whichever in-flight caller was awaiting
            // it (see `worker_gone_error`) -- `join`'s own `Err` here (the
            // panic payload) is deliberately swallowed rather than
            // panicking again inside `Drop::drop`, per the ordinary Rust
            // guidance against panicking during unwind/drop.
            let _ = worker.join();
        }

        // SAFETY: `self.handle.0` is the handle opened by `SerialPort::
        // open`; the worker thread (the only other user of it) has just
        // been joined above, so no other thread can be mid-`ReadFile`/
        // `WriteFile` against it here.
        unsafe {
            CloseHandle(self.handle.0);
        }
    }
}

/// Map the last Win32 error (via `GetLastError`) after a failed
/// `CreateFileW` call to this crate's [`SerialError`], per ADR 0004 §3's
/// exact table: `ERROR_FILE_NOT_FOUND`/`ERROR_PATH_NOT_FOUND` ->
/// `DeviceNotFound`; `ERROR_ACCESS_DENIED` -> `PermissionDenied`; else ->
/// `Io`.
fn map_create_file_error(path: &str) -> SerialError {
    // SAFETY: `GetLastError` takes no arguments and has no preconditions;
    // called immediately after the failing `CreateFileW`, before any other
    // Win32 call could overwrite the thread-local last-error value.
    let code = unsafe { GetLastError() };
    match code {
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => {
            SerialError::DeviceNotFound(path.to_string())
        }
        ERROR_ACCESS_DENIED => SerialError::PermissionDenied(path.to_string()),
        other => SerialError::Io(std::io::Error::from_raw_os_error(other as i32)),
    }
}

/// `GetCommState`/mutate `DCB`/`SetCommState` -- the Windows analog of
/// `io_uring.rs`'s `configure_termios`. Implements every
/// [`SerialConfig`] field per `docs/adr/0004-windows-serial-backend.md` §5's
/// mapping table exactly; see inline comments below for the row-by-row
/// correspondence.
fn configure_dcb(handle: HANDLE, config: &SerialConfig) -> SerialResult<()> {
    // Start from the driver's current DCB (as `configure_termios` starts
    // from `tcgetattr`'s current termios) rather than a zeroed struct, so
    // fields this function doesn't touch (`XonLim`/`XoffLim`/`DCBlength`/
    // `wReserved*`) keep sane driver-supplied defaults.
    let mut dcb: DCB = unsafe { std::mem::zeroed() };
    dcb.DCBlength = std::mem::size_of::<DCB>() as u32;
    // SAFETY: `handle` is a valid open serial-port handle (an invariant of
    // this function's only caller, `SerialPort::open`, which calls this
    // immediately after a successful `CreateFileW`); `&mut dcb` is a valid,
    // properly-sized `*mut DCB` (`DCBlength` set just above, as
    // `GetCommState` requires).
    if unsafe { GetCommState(handle, &mut dcb) } == 0 {
        return Err(SerialError::Io(std::io::Error::last_os_error()));
    }

    // --- baud_rate: u32 (ADR 0004 §5 row 1) ---
    // Reuse the same validated rate set Linux's `baud_rate_from_u32` uses
    // (`crate::baud`), rejecting anything it would reject with the same
    // `SerialError::InvalidConfig` -- a deliberate cross-platform
    // consistency choice, not a Win32 requirement (Windows' `DCB.BaudRate`
    // would otherwise accept an arbitrary `u32`).
    dcb.BaudRate = crate::baud::validate_baud_rate(config.baud_rate)?;

    // --- data_bits: u8 (5-8) -> DCB.ByteSize (ADR 0004 §5 row 2) ---
    // Mirrors `configure_termios`'s exact fallback shape (any value other
    // than 5/6/7 becomes 8, the default and most common), for identical
    // behavior on out-of-range input on both platforms.
    dcb.ByteSize = match config.data_bits {
        5 => 5,
        6 => 6,
        7 => 7,
        _ => 8,
    };

    // --- stop_bits: u8 (1 or 2) -> DCB.StopBits (ADR 0004 §5 row 3) ---
    // ONESTOPBIT(0) / TWOSTOPBITS(2); `ONE5STOPBITS`(1) is unreachable
    // through this shared `u8` field on either platform, per the ADR.
    dcb.StopBits = if config.stop_bits >= 2 {
        TWOSTOPBITS
    } else {
        ONESTOPBIT
    };

    // --- parity: Parity -> DCB.Parity + fParity (ADR 0004 §5 row 4) ---
    // NOPARITY(0)/EVENPARITY(2)/ODDPARITY(1), plus `fParity = 1` to enable
    // checking whenever parity is not `None` (mirroring Linux's PARENB,
    // cleared only for `Parity::None`).
    let (dcb_parity, f_parity) = match config.parity {
        Parity::None => (NOPARITY, false),
        Parity::Even => (EVENPARITY, true),
        Parity::Odd => (ODDPARITY, true),
    };
    dcb.Parity = dcb_parity;
    dcb_bits::set_bit(&mut dcb._bitfield, dcb_bits::F_PARITY, f_parity);

    // --- flow_control: FlowControl (ADR 0004 §5 row 5) ---
    // Hardware: fOutxCtsFlow=1, fRtsControl=RTS_CONTROL_HANDSHAKE.
    // Software: fInX=1, fOutX=1 (XonChar/XoffChar default 0x11/0x13,
    //   matching Linux's default VSTART/VSTOP -- set unconditionally below,
    //   harmless when unused).
    // None: all off, fRtsControl=RTS_CONTROL_ENABLE.
    let (outx_cts, in_x, out_x, rts_control) = match config.flow_control {
        FlowControl::None => (false, false, false, dcb_bits::RTS_CONTROL_ENABLE),
        FlowControl::Hardware => (true, false, false, dcb_bits::RTS_CONTROL_HANDSHAKE),
        FlowControl::Software => (false, true, true, dcb_bits::RTS_CONTROL_ENABLE),
    };
    dcb_bits::set_bit(&mut dcb._bitfield, dcb_bits::F_OUTX_CTS_FLOW, outx_cts);
    dcb_bits::set_bit(&mut dcb._bitfield, dcb_bits::F_IN_X, in_x);
    dcb_bits::set_bit(&mut dcb._bitfield, dcb_bits::F_OUT_X, out_x);
    dcb_bits::set_2bit(
        &mut dcb._bitfield,
        dcb_bits::F_RTS_CONTROL_SHIFT,
        rts_control,
    );
    dcb.XonChar = 0x11;
    dcb.XoffChar = 0x13;

    // --- fDtrControl: not a distinct row in ADR 0004 §5's table (the
    // table's only DTR-related rows, `initial_dtr`, are about Task 8's
    // `EscapeCommFunction(SETDTR/CLRDTR)` post-open calls, not
    // configure-time DCB state). Set to DTR_CONTROL_ENABLE (manual/software
    // control) unconditionally so those later `EscapeCommFunction` calls
    // actually take effect -- if left at DTR_CONTROL_DISABLE the line would
    // stay low regardless of `EscapeCommFunction`; if HANDSHAKE the driver
    // would override manual control. This mirrors Linux, where DTR is never
    // touched by `configure_termios`/`CRTSCTS` at all and is purely
    // ioctl-controlled (`TIOCMBIS`/`TIOCMBIC`) after open.
    dcb_bits::set_2bit(
        &mut dcb._bitfield,
        dcb_bits::F_DTR_CONTROL_SHIFT,
        dcb_bits::DTR_CONTROL_ENABLE,
    );

    // --- Raw/binary mode: the Windows analog of `configure_termios`'s
    // unconditional `cfmakeraw` call, not driven by any `SerialConfig`
    // field. `fBinary = 1` is required for ordinary byte-oriented (as
    // opposed to line-oriented/EOF-character-sensitive) serial I/O -- the
    // Win32 documentation states Windows does not support non-binary
    // (`fBinary = 0`) mode for serial communication at all. The remaining
    // bits below have no `SerialConfig`-field or Linux-termios equivalent
    // and are set to their conservative/no-special-processing values,
    // matching `cfmakeraw`'s general "disable all special processing"
    // intent: no DSR-based flow control, no DSR-based input gating, no
    // auto-resume-on-XOFF, no error-character substitution, no NUL-byte
    // discarding, no abort-on-error.
    dcb_bits::set_bit(&mut dcb._bitfield, dcb_bits::F_BINARY, true);
    dcb_bits::set_bit(&mut dcb._bitfield, dcb_bits::F_OUTX_DSR_FLOW, false);
    dcb_bits::set_bit(&mut dcb._bitfield, dcb_bits::F_DSR_SENSITIVITY, false);
    dcb_bits::set_bit(&mut dcb._bitfield, dcb_bits::F_TX_CONTINUE_ON_XOFF, false);
    dcb_bits::set_bit(&mut dcb._bitfield, dcb_bits::F_ERROR_CHAR, false);
    dcb_bits::set_bit(&mut dcb._bitfield, dcb_bits::F_NULL, false);
    dcb_bits::set_bit(&mut dcb._bitfield, dcb_bits::F_ABORT_ON_ERROR, false);

    // SAFETY: `handle` is the same valid open handle validated by the
    // `GetCommState` call above; `&dcb` is a valid, fully-initialized
    // `*const DCB` (every field either copied from `GetCommState`'s output
    // or explicitly set above).
    if unsafe { SetCommState(handle, &dcb) } == 0 {
        return Err(SerialError::Io(std::io::Error::last_os_error()));
    }

    Ok(())
}

/// `SetCommTimeouts`, per `docs/adr/0004-windows-serial-backend.md` §3.
///
/// `ReadIntervalTimeout = 0`, `ReadTotalTimeoutMultiplier = 0`,
/// `ReadTotalTimeoutConstant = READ_TIMEOUT.as_millis()` (this exact
/// combination means: a single blocking `ReadFile` call waits up to
/// `READ_TIMEOUT` and returns whatever arrived, possibly zero bytes -- Task
/// 8's worker thread maps a successful zero-byte `ReadFile` to
/// `Err(TransportError::ReadTimeout)`, preserving `Transport::read`'s
/// existing "`Ok(n)` is always `> 0`" contract exactly, per the ADR).
///
/// `WriteTotalTimeoutConstant`: chosen as 5000ms (5s), matching ADR 0004
/// §3's own suggested starting point ("a generous bound (e.g. 5s) to fail a
/// hung/removed-device write rather than hang forever"). No real Windows
/// hardware is available in this sandboxed environment to empirically tune
/// this value against; 5s is generous relative to typical CAT
/// command-response latency (< 100ms, per `crate::timeouts`' own doc
/// comment) while still failing a genuinely hung/removed device in bounded
/// time rather than never. `WriteTotalTimeoutMultiplier = 0` (a fixed
/// constant timeout, not scaled per byte -- CAT command frames are short,
/// a handful to a few dozen bytes, so a per-byte multiplier adds nothing
/// meaningful here).
fn set_comm_timeouts(handle: HANDLE) -> SerialResult<()> {
    let timeouts = COMMTIMEOUTS {
        ReadIntervalTimeout: 0,
        ReadTotalTimeoutMultiplier: 0,
        // `READ_TIMEOUT` is a small fixed constant (2000ms production /
        // 100ms test) -- `as_millis()` (`u128`) always fits in `u32` here.
        ReadTotalTimeoutConstant: READ_TIMEOUT.as_millis() as u32,
        WriteTotalTimeoutMultiplier: 0,
        WriteTotalTimeoutConstant: 5000,
    };
    // SAFETY: `handle` is a valid open serial-port handle (same invariant
    // as `configure_dcb`, called immediately after it succeeds); `&timeouts`
    // is a valid `*const COMMTIMEOUTS` for the duration of this call.
    if unsafe { SetCommTimeouts(handle, &timeouts) } == 0 {
        return Err(SerialError::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}
