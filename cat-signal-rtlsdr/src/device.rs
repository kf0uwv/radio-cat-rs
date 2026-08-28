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

//! The librtlsdr worker thread. Behind the `device` feature.
//!
//! Per `docs/adr/0014-rtlsdr-spectrum-source.md` §2: `rtlsdr_read_async`
//! blocks until cancelled, so a dedicated `std::thread` owns the device
//! handle and everything librtlsdr touches. Only owned sample buffers
//! cross back, so nothing `!Send` crosses the thread boundary and the
//! workspace's `?Send` binding (ADR 0002) is untouched.
//!
//! Backpressure is ADR 0014 §3: a slot holding the **newest** buffer,
//! overwritten rather than queued. A waterfall consumer that has fallen
//! behind wants the current spectrum, not a stale queued one, and an
//! unbounded queue turns a slow consumer into unbounded memory. Blocking
//! the worker instead would stall the USB read loop and make librtlsdr
//! drop samples at the driver level, where nobody can see it.

use crate::IqSource;
use rustfft::num_complex::Complex32;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// What can go wrong talking to a dongle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceError {
    /// No device at the requested index.
    NotFound(u32),
    /// Opened, but the driver is wrong for our purposes.
    ///
    /// The common Windows case: the dongle still has its DVB-T driver and
    /// needs WinUSB (conventionally via Zadig). ADR 0014 §4 requires this
    /// be a specific, actionable error rather than a generic "no device" —
    /// we deliberately do **not** rebind the driver ourselves.
    DriverNotUsable(String),
    Open(String),
    Configure(String),
    /// The worker stopped; the device was probably unplugged.
    Stopped,
}

impl std::fmt::Display for DeviceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceError::NotFound(i) => write!(f, "no RTL-SDR at index {i}"),
            DeviceError::DriverNotUsable(m) => write!(
                f,
                "RTL-SDR found but not usable ({m}). On Windows the dongle \
                 must be bound to WinUSB (use Zadig); the stock DVB-T \
                 driver cannot be used for IQ capture."
            ),
            DeviceError::Open(m) => write!(f, "could not open RTL-SDR: {m}"),
            DeviceError::Configure(m) => write!(f, "could not configure RTL-SDR: {m}"),
            DeviceError::Stopped => write!(f, "RTL-SDR worker stopped (device unplugged?)"),
        }
    }
}

impl std::error::Error for DeviceError {}

/// The single-slot handoff between the worker thread and the frame pump.
struct Slot {
    buffer: Mutex<Option<Vec<Complex32>>>,
    ready: Condvar,
    dropped: AtomicU64,
    running: AtomicBool,
}

/// An [`IqSource`] fed by a librtlsdr worker thread.
pub struct RtlSdrDevice {
    slot: Arc<Slot>,
    _worker: std::thread::JoinHandle<()>,
}

impl RtlSdrDevice {
    /// Open the dongle at `index`, park it on `if_center_hz`, and start
    /// reading.
    ///
    /// The frequency set here is the **only** one ever written to the
    /// device. Nothing in `retune` touches it — see the crate header.
    pub fn open(index: u32, if_center_hz: u64, sample_rate_hz: u32) -> Result<Self, DeviceError> {
        let slot = Arc::new(Slot {
            buffer: Mutex::new(None),
            ready: Condvar::new(),
            dropped: AtomicU64::new(0),
            running: AtomicBool::new(true),
        });

        // The device is opened INSIDE the worker thread, and the outcome
        // reported back over this channel.
        //
        // `RTLSDRDevice` wraps a raw pointer from a C library and carries
        // no thread-safety guarantee we should rely on. Opening it here and
        // moving it in would need that guarantee; opening it there needs
        // nothing. The cost is one channel and a wait; the benefit is that
        // the device handle is created, used and dropped on exactly one
        // thread, which is also what makes the `?Send` story trivial.
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), DeviceError>>();
        let worker_slot = Arc::clone(&slot);

        let worker = std::thread::Builder::new()
            .name("rtlsdr-iq".into())
            .spawn(move || {
                let mut device = match rtlsdr::open(index as i32) {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = tx.send(Err(map_open_error(index, e)));
                        worker_slot.running.store(false, Ordering::Release);
                        worker_slot.ready.notify_all();
                        return;
                    }
                };

                // The ONLY frequency ever written to this device. Nothing
                // in `retune` touches it -- see the crate header and ADR
                // 0014 section 6.
                let configured = device
                    .set_sample_rate(sample_rate_hz)
                    .map_err(|e| DeviceError::Configure(format!("sample rate: {e:?}")))
                    .and_then(|()| {
                        device
                            .set_center_freq(if_center_hz as u32)
                            .map_err(|e| DeviceError::Configure(format!("centre frequency: {e:?}")))
                    })
                    .and_then(|()| {
                        device
                            .set_tuner_gain_mode(false)
                            .map_err(|e| DeviceError::Configure(format!("gain mode: {e:?}")))
                    })
                    .and_then(|()| {
                        device
                            .reset_buffer()
                            .map_err(|e| DeviceError::Configure(format!("buffer reset: {e:?}")))
                    });

                if let Err(e) = configured {
                    let _ = tx.send(Err(e));
                    worker_slot.running.store(false, Ordering::Release);
                    worker_slot.ready.notify_all();
                    return;
                }

                if tx.send(Ok(())).is_err() {
                    return; // the opener gave up
                }
                drop(tx);

                // `read_sync` blocks. That is the whole reason this thread
                // exists: called from a monoio task it would wedge the
                // executor that is also driving the CAT session, so a
                // spectrum source would stall the radio it annotates.
                loop {
                    match device.read_sync(READ_CHUNK_BYTES) {
                        Ok(bytes) => {
                            let samples = to_complex(&bytes);
                            let mut held = worker_slot.buffer.lock().expect("slot poisoned");
                            if held.is_some() {
                                // Newest wins; count what the consumer missed.
                                worker_slot.dropped.fetch_add(1, Ordering::Relaxed);
                            }
                            *held = Some(samples);
                            worker_slot.ready.notify_one();
                        }
                        Err(_) => break,
                    }
                }

                worker_slot.running.store(false, Ordering::Release);
                worker_slot.ready.notify_all();
            })
            .map_err(|e| DeviceError::Open(format!("worker thread: {e}")))?;

        match rx.recv() {
            Ok(Ok(())) => Ok(Self {
                slot,
                _worker: worker,
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(DeviceError::Stopped),
        }
    }
}

/// One USB transfer's worth of samples. 16384 bytes is 8192 IQ pairs --
/// comfortably more than the largest FFT this crate offers, so a single
/// read always yields a whole frame.
const READ_CHUNK_BYTES: usize = 16_384;

/// librtlsdr delivers unsigned 8-bit IQ centred on 127.5.
fn to_complex(bytes: &[u8]) -> Vec<Complex32> {
    bytes
        .chunks_exact(2)
        .map(|p| {
            Complex32::new(
                (f32::from(p[0]) - 127.5) / 127.5,
                (f32::from(p[1]) - 127.5) / 127.5,
            )
        })
        .collect()
}

fn map_open_error(index: u32, e: impl std::fmt::Debug) -> DeviceError {
    let text = format!("{e:?}");
    // librtlsdr cannot tell us "wrong driver" directly; on Windows a
    // DVB-T-bound dongle enumerates and then fails to claim its interface.
    if text.contains("Access") || text.contains("claim") || text.contains("busy") {
        DeviceError::DriverNotUsable(text)
    } else if text.contains("NoDevice") || text.contains("not found") {
        DeviceError::NotFound(index)
    } else {
        DeviceError::Open(text)
    }
}

impl IqSource for RtlSdrDevice {
    type Error = DeviceError;

    fn read(&mut self, _wanted: usize) -> Result<Vec<Complex32>, Self::Error> {
        let mut slot = self.slot.buffer.lock().expect("slot poisoned");
        loop {
            if let Some(buffer) = slot.take() {
                return Ok(buffer);
            }
            if !self.slot.running.load(Ordering::Acquire) {
                return Err(DeviceError::Stopped);
            }
            slot = self.slot.ready.wait(slot).expect("slot poisoned");
        }
    }

    fn frames_dropped(&self) -> u64 {
        self.slot.dropped.load(Ordering::Relaxed)
    }
}
