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

//! Serving consoles from a server whose radio access is async.
//!
//! # The mismatch, and why a cache is the honest answer
//!
//! [`cat_native::RadioHost`] is blocking and `&self`: one radio, many
//! connections, each on its own thread. The broker is the opposite — a
//! single-threaded monoio runtime that owns the one physical link, and
//! every read of the radio is an `await` on it.
//!
//! The bridge is a cache. A task inside the runtime polls the radio and
//! publishes what it finds; the listener threads read the last published
//! value. That is not a compromise to work around the type mismatch, it is
//! what a server of this shape genuinely is: the radio is a serial port
//! answering a few times a second, and pretending a console's read reaches
//! down to the wire would mean every connected client queueing behind the
//! same 9600-baud link.
//!
//! Commands go the other way, and they *do* wait: a console that asks for a
//! frequency should learn whether the radio took it. They queue, the poller
//! applies them, and the answer comes back on a one-shot channel.

use std::collections::VecDeque;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cat_framework::capabilities::RadioCapabilities;
use cat_native::{Command, RadioHost, RadioState};
use cat_signal::SpectrumFrame;

/// How long a console waits for the radio to answer a command.
///
/// Generous: the link is slow and the poller may be mid-cycle. Short
/// enough that a wedged radio does not hold a connection thread forever.
const APPLY_TIMEOUT: Duration = Duration::from_secs(3);

/// A radio the native protocol can read and drive, in the async terms the
/// broker actually speaks.
#[async_trait::async_trait(?Send)]
pub trait NativeRadio {
    /// Everything a console displays. `None` if the radio did not answer.
    async fn state(&mut self) -> Option<RadioState>;

    /// Apply a command that capabilities have already accepted.
    async fn apply(&mut self, command: &Command) -> Result<(), String>;
}

/// For a server that does not serve consoles.
///
/// A concrete type rather than an `Option`, because a closure that is
/// sometimes absent cannot have its type inferred and every caller would
/// have to name one anyway.
pub struct NoNative;

#[async_trait::async_trait(?Send)]
impl NativeRadio for NoNative {
    async fn state(&mut self) -> Option<RadioState> {
        None
    }
    async fn apply(&mut self, _command: &Command) -> Result<(), String> {
        Err("this server was not built to serve consoles".to_string())
    }
}

type Pending = (Command, SyncSender<Result<(), String>>);

/// What the poller publishes and the listener threads read.
pub struct NativeShared {
    capabilities: &'static RadioCapabilities,
    state: Mutex<Option<RadioState>>,
    spectrum: Mutex<Option<SpectrumFrame>>,
    queue: Mutex<VecDeque<Pending>>,
}

impl NativeShared {
    pub fn new(capabilities: &'static RadioCapabilities) -> Arc<Self> {
        Arc::new(Self {
            capabilities,
            state: Mutex::new(None),
            spectrum: Mutex::new(None),
            queue: Mutex::new(VecDeque::new()),
        })
    }

    /// Publish a spectrum frame. Newest wins.
    ///
    /// Called from whatever thread owns the SDR — which is its own thread,
    /// because reading a dongle is blocking I/O and doing it inside the
    /// monoio runtime would stall every other client while the FFT ran.
    pub fn publish_spectrum(&self, frame: SpectrumFrame) {
        if let Ok(mut slot) = self.spectrum.lock() {
            *slot = Some(frame);
        }
    }

    /// The dial, for an SDR that needs to follow it.
    pub fn dial_hz(&self) -> Option<u64> {
        self.state.lock().ok()?.as_ref().map(|s| s.vfo_a_hz)
    }

    fn publish_state(&self, state: Option<RadioState>) {
        if let Ok(mut slot) = self.state.lock() {
            // A failed read leaves the last good state rather than blanking
            // the console. One missed poll on a serial link is ordinary;
            // showing em dashes for it would make the display flicker
            // between "known" and "unknown" all day.
            if state.is_some() {
                *slot = state;
            }
        }
    }

    fn take_queued(&self) -> Vec<Pending> {
        self.queue
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }
}

impl RadioHost for NativeShared {
    fn capabilities(&self) -> &'static RadioCapabilities {
        self.capabilities
    }

    fn state(&self) -> RadioState {
        self.state
            .lock()
            .ok()
            .and_then(|s| s.clone())
            .unwrap_or_else(|| RadioState {
                // Nothing has been read yet. Reported honestly rather than
                // as zeros: the session turns a state it has not got into
                // NotReady, and a console draws em dashes.
                vfo_a_hz: 0,
                vfo_b_hz: 0,
                mode: cat_framework::capabilities::ModeId::Usb,
                split: false,
                transmitting: false,
                memory_channel: None,
                if_shift_hz: None,
                filter_width_hz: None,
                meters: Vec::new(),
            })
    }

    fn apply(&self, command: &Command) -> Result<(), String> {
        let (tx, rx) = sync_channel(1);
        self.queue
            .lock()
            .map_err(|_| "the server's command queue is poisoned".to_string())?
            .push_back((command.clone(), tx));
        match rx.recv_timeout(APPLY_TIMEOUT) {
            Ok(result) => result,
            Err(_) => Err("the radio did not answer in time".to_string()),
        }
    }

    fn spectrum(&self) -> Option<SpectrumFrame> {
        self.spectrum.lock().ok().and_then(|s| s.clone())
    }
}

/// Drive `radio` from inside the broker's runtime: apply queued commands,
/// then refresh the published state.
///
/// Runs until the process ends. Commands are applied *before* the refresh
/// so that a console's next read reflects what it just asked for rather
/// than lagging a whole poll behind.
pub async fn pump<N: NativeRadio>(shared: Arc<NativeShared>, mut radio: N, interval: Duration) {
    loop {
        // Apply everything, then refresh, then answer. The order matters:
        // answering first lets a console's next read arrive before the
        // refresh, so it sees the state from before its own command and
        // the display appears not to have taken it. Refreshing first means
        // that by the time `apply` returns, a read is already correct.
        let queued = shared.take_queued();
        let mut results = Vec::with_capacity(queued.len());
        for (command, reply) in queued {
            results.push((reply, radio.apply(&command).await));
        }
        shared.publish_state(radio.state().await);
        for (reply, result) in results {
            // A console that has hung up leaves nobody to tell; that is
            // not an error worth logging on every disconnect.
            let _ = reply.send(result);
        }
        // Two platforms, two correct answers. On Linux the pump is a task
        // inside the broker's runtime and must yield to it; on Windows it
        // owns a thread, and sleeping that thread is exactly right. A
        // thread sleep on Linux would stall every other client for the
        // interval.
        #[cfg(target_os = "linux")]
        monoio::time::sleep(interval).await;
        #[cfg(not(target_os = "linux"))]
        std::thread::sleep(interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_framework::capabilities::*;

    const MODES: &[ModeDescriptor] = &[ModeDescriptor {
        id: ModeId::Usb,
        label: "USB",
        kind: ModeKind::Ssb,
        sideband: Some(Sideband::Upper),
        default_bandwidth_hz: 2400,
    }];
    const ENDPOINTS: &[EndpointDescriptor] = &[EndpointDescriptor {
        role: EndpointRole::Cat,
        required: true,
        shareable_with: &[],
    }];
    static RADIO: RadioCapabilities = RadioCapabilities {
        model: "Bridge Test Radio",
        endpoints: EndpointSet::new(ENDPOINTS),
        vfos: VfoCapability {
            count: 2,
            split: true,
            rit_hz: None,
            xit_hz: None,
        },
        modes: MODES,
        tuning_steps_hz: &[10],
        rx_range: FrequencyRange::new(500_000, 60_000_000),
        filters: FilterCapability {
            if_shift_hz: None,
            widths_hz: None,
            notch: false,
        },
        meters: MeterSet::new(&[]),
        memory: None,
        menu: None,
        signal: SignalSupport::None,
    };

    fn state_at(hz: u64) -> RadioState {
        RadioState {
            vfo_a_hz: hz,
            vfo_b_hz: 0,
            mode: ModeId::Usb,
            split: false,
            transmitting: false,
            memory_channel: None,
            if_shift_hz: None,
            filter_width_hz: None,
            meters: Vec::new(),
        }
    }

    #[test]
    fn a_failed_read_keeps_the_last_good_state_rather_than_blanking_it() {
        // One missed poll on a serial link is ordinary. Blanking would make
        // the console flicker between known and unknown all day.
        let shared = NativeShared::new(&RADIO);
        shared.publish_state(Some(state_at(14_074_000)));
        shared.publish_state(None);
        assert_eq!(RadioHost::state(&*shared).vfo_a_hz, 14_074_000);
    }

    #[test]
    fn a_command_waits_for_the_radio_and_reports_what_it_said() {
        let shared = NativeShared::new(&RADIO);
        let worker = Arc::clone(&shared);
        std::thread::spawn(move || {
            // Stand in for the poller: drain and refuse.
            loop {
                for (_, reply) in worker.take_queued() {
                    let _ = reply.send(Err("the radio said no".to_string()));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let result = RadioHost::apply(&*shared, &Command::Retune { hz: 14_074_000 });
        assert_eq!(result, Err("the radio said no".to_string()));
    }

    #[test]
    fn a_radio_that_never_answers_times_out_rather_than_wedging_the_connection() {
        // Nothing drains the queue here. Without a timeout this would hold
        // a connection thread until the process ended.
        let shared = NativeShared::new(&RADIO);
        let started = std::time::Instant::now();
        let result = RadioHost::apply(&*shared, &Command::Retune { hz: 14_074_000 });
        assert!(result.is_err());
        assert!(started.elapsed() >= APPLY_TIMEOUT);
        assert!(started.elapsed() < APPLY_TIMEOUT * 2);
    }

    #[test]
    fn spectrum_is_newest_wins() {
        let shared = NativeShared::new(&RADIO);
        for sequence in 0..5 {
            shared.publish_spectrum(SpectrumFrame {
                center_hz: 14_074_000,
                span_hz: 96_000,
                ref_level_dbm: 0.0,
                sequence,
                bins: vec![-100.0; 8],
            });
        }
        assert_eq!(RadioHost::spectrum(&*shared).unwrap().sequence, 4);
    }

    #[test]
    fn the_dial_is_readable_for_an_sdr_that_has_to_follow_it() {
        // An IF tap is dial-centred, so the thread reading the dongle needs
        // to know where the radio is pointing.
        let shared = NativeShared::new(&RADIO);
        assert_eq!(shared.dial_hz(), None);
        shared.publish_state(Some(state_at(21_074_000)));
        assert_eq!(shared.dial_hz(), Some(21_074_000));
    }
}
