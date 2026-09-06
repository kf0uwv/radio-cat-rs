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
use cat_signal::{DeviceInfo, DeviceKind, DeviceList};
use rustfft::num_complex::Complex32;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

fn check_sample_rate(hz: u32) -> Result<(), DeviceError> {
    if crate::is_valid_sample_rate(hz) {
        return Ok(());
    }
    Err(DeviceError::Configure(format!(
        "sample rate {hz} Hz is not one an RTL2832U can be set to; \
         it accepts {}-{} Hz and {}-{} Hz and nothing between or below",
        crate::SAMPLE_RATE_BANDS[0].0,
        crate::SAMPLE_RATE_BANDS[0].1,
        crate::SAMPLE_RATE_BANDS[1].0,
        crate::SAMPLE_RATE_BANDS[1].1,
    )))
}

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
        // Checked here rather than left to the driver, which answers an
        // out-of-range rate with `errno -22` and the word "Unknown".
        //
        // This is not a nicety. `ts570d`'s emulator served its IF output at
        // 96 kHz for months and every test passed, because `rtl_tcp` is a
        // socket and a socket will carry any rate you like. The first time
        // a real dongle was asked for 96 kHz it refused, and the emulator
        // turned out to have been emulating a device that cannot exist.
        // A named error at the boundary is what turns that into a sentence
        // instead of a number.
        check_sample_rate(sample_rate_hz)?;
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
                //
                // A read error ends the loop and is not carried out of it:
                // the end is signalled by `running` going false and
                // `ready` waking every waiter, which is what a consumer
                // already watches. Threading the error through would need
                // somewhere to put it that outlives this thread.
                while let Ok(bytes) = device.read_sync(READ_CHUNK_BYTES) {
                    let samples = to_complex(&bytes);
                    let mut held = worker_slot.buffer.lock().expect("slot poisoned");
                    if held.is_some() {
                        // Newest wins; count what the consumer missed.
                        worker_slot.dropped.fetch_add(1, Ordering::Relaxed);
                    }
                    *held = Some(samples);
                    worker_slot.ready.notify_one();
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

// ---------------------------------------------------------------------------
// Enumeration
// ---------------------------------------------------------------------------

/// The SDRs plugged into this machine.
///
/// Returns `cat_signal::DeviceList` so a console can render SDRs and sound
/// cards through one picker without linking a driver for either — see that
/// type's module doc.
///
/// [`DeviceInfo::spec`](cat_signal::DeviceInfo::spec) is `rtl:<index>`,
/// which is exactly what an endpoint argument takes. An RTL-SDR has no
/// filesystem name on any platform — libusb claims it, so neither the tty
/// nor the sound layer ever sees it — and librtlsdr addresses it by index.
/// That is why the scheme exists and why the same string is correct on
/// Linux and Windows alike.
///
/// **No default is ever marked.** librtlsdr has no notion of one, and
/// inventing "index 0 is the default" would be a claim the driver does not
/// make — on a two-dongle station the wrong one is a plausible pick.
pub fn devices() -> DeviceList {
    let count = rtlsdr::get_device_count();
    if count <= 0 {
        // Not an error: zero dongles is a successful answer to the
        // question, and it means "plug one in" rather than "this build
        // cannot look".
        return DeviceList::found(DeviceKind::Sdr, Vec::new());
    }

    let devices = (0..count)
        .map(|index| {
            let name = rtlsdr::get_device_name(index);
            // The USB strings carry the serial, which is the only way to
            // tell two identical dongles apart. Best-effort: a device that
            // will not answer still belongs in the list, because it is
            // still plugged in and still openable by index.
            let detail = rtlsdr::get_device_usb_strings(index)
                .ok()
                .map(|s| format!("{} {} serial {}", s.manufacturer, s.product, s.serial))
                .filter(|d| !d.trim().is_empty());

            DeviceInfo {
                kind: DeviceKind::Sdr,
                spec: format!("rtl:{index}"),
                label: if name.is_empty() {
                    format!("RTL-SDR #{index}")
                } else {
                    name
                },
                detail,
                is_default: false,
            }
        })
        .collect();

    DeviceList::found(DeviceKind::Sdr, devices)
}

#[cfg(test)]
mod enumeration_tests {
    use super::*;

    #[test]
    fn enumerating_with_no_hardware_is_an_answer_and_not_a_failure() {
        // This runs on a machine with no dongle, which is the case worth
        // pinning: "none plugged in" must not surface as "cannot look",
        // because the two send an operator to different places.
        let list = devices();
        assert_eq!(list.kind, DeviceKind::Sdr);
        assert!(
            list.is_available(),
            "enumeration itself must succeed even with nothing attached: {:?}",
            list.error
        );
    }

    #[test]
    fn every_listed_device_carries_a_spec_the_command_line_would_take() {
        // Picking is a shortcut for typing; a spec a flag would reject
        // would make the picker a second, incompatible naming scheme.
        for d in devices().devices {
            assert!(d.spec.starts_with("rtl:"), "{:?}", d.spec);
            assert!(
                d.spec.trim_start_matches("rtl:").parse::<u32>().is_ok(),
                "an SDR is addressed by index: {:?}",
                d.spec
            );
            assert!(
                !d.label.is_empty(),
                "a device with no label cannot be picked"
            );
        }
    }

    #[test]
    fn no_sdr_is_claimed_to_be_the_default() {
        // librtlsdr has no notion of one. Marking index 0 would be a claim
        // the driver does not make, and on a two-dongle station it would be
        // a plausible-looking wrong pick.
        assert!(devices().devices.iter().all(|d| !d.is_default));
    }
}

#[cfg(test)]
mod sample_rate_tests {
    use super::*;

    #[test]
    fn the_rate_the_emulator_served_for_months_is_not_a_real_one() {
        // The bug this check exists for. 96 kHz went unquestioned because
        // `rtl_tcp` is a socket and a socket carries any rate; the hardware
        // it was pretending to be cannot be set below 225 kHz.
        assert!(!crate::is_valid_sample_rate(96_000));
        let err = check_sample_rate(96_000).unwrap_err();
        let text = format!("{err}");
        assert!(text.contains("96000"), "{text}");
        assert!(
            text.contains("225001"),
            "and says what it would accept: {text}"
        );
    }

    #[test]
    fn both_bands_are_accepted_and_the_gap_between_them_is_not() {
        for hz in [225_001, 240_000, 300_000, 900_001, 2_048_000, 3_200_000] {
            assert!(crate::is_valid_sample_rate(hz), "{hz} is a real rate");
        }
        // The divider gap. A rate in here looks plausible and is refused by
        // the silicon.
        for hz in [0, 48_000, 225_000, 300_001, 600_000, 900_000, 3_200_001] {
            assert!(!crate::is_valid_sample_rate(hz), "{hz} is not settable");
        }
    }
}
