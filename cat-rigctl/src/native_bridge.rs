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

/// How a host is asked what it has wired. See [`NativeShared::set_installation`].
pub type InstallationFn = Arc<dyn Fn() -> cat_framework::installation::Installation + Send + Sync>;

/// What the poller publishes and the listener threads read.
pub struct NativeShared {
    capabilities: &'static RadioCapabilities,
    /// How many consoles are watching.
    ///
    /// The pump polls the radio to keep this cache warm. At display rate
    /// on a 9600-baud link that is most of the link's capacity, and every
    /// other client -- WSJT-X asking for frequency and PTT -- queues
    /// behind it. Measured on a TS-570D: 383 ms median per rigctl request
    /// with the pump running and nobody looking at a console.
    ///
    /// So the pump asks. Nobody watching means nothing needs a fresh
    /// state, and the link belongs to whoever does.
    consoles: std::sync::atomic::AtomicUsize,
    state: Mutex<Option<RadioState>>,
    spectrum: Mutex<Option<SpectrumFrame>>,
    /// The newest audio frame, on the same newest-wins terms as spectrum.
    audio: Mutex<Option<cat_signal::AudioFrame>>,
    queue: Mutex<VecDeque<Pending>>,
    /// What this machine can see, if the application offered a directory.
    ///
    /// `None` declines the question, and a client is told exactly that.
    /// This bridge deliberately does not know how to enumerate anything:
    /// sound cards and SDRs are the application's business, and a server
    /// wired for CAT only should not grow a cpal dependency to say "no".
    devices: Option<Arc<dyn cat_signal::DeviceDirectory>>,
    /// The arrangement this radio's console should use, if the
    /// application authored one.
    layout: Mutex<Option<cat_layout::LayoutSpec>>,
    /// The palette this radio's console should use.
    theme: Mutex<Option<cat_layout::Theme>>,
    /// How to ask the application what this bench has wired.
    ///
    /// A closure, not a value, because it changes underneath: a console
    /// can attach a source at runtime, and a stored answer would then tell
    /// the *next* console what was there when the server started. Called
    /// once per connection, at handshake.
    installation: Mutex<Option<InstallationFn>>,
}

impl NativeShared {
    pub fn new(capabilities: &'static RadioCapabilities) -> Arc<Self> {
        Arc::new(Self {
            capabilities,
            consoles: std::sync::atomic::AtomicUsize::new(0),
            state: Mutex::new(None),
            spectrum: Mutex::new(None),
            audio: Mutex::new(None),
            queue: Mutex::new(VecDeque::new()),
            devices: None,
            installation: Mutex::new(None),
            layout: Mutex::new(None),
            theme: Mutex::new(None),
        })
    }

    /// The same, for a server that can tell clients what its machine has.
    ///
    /// A console is not usually on the radio's machine, so its own sound
    /// cards are not the radio's. This is how the far end gets a truthful
    /// answer instead of a picker full of the operator's laptop.
    pub fn with_devices(
        capabilities: &'static RadioCapabilities,
        devices: Arc<dyn cat_signal::DeviceDirectory>,
    ) -> Arc<Self> {
        Arc::new(Self {
            capabilities,
            consoles: std::sync::atomic::AtomicUsize::new(0),
            state: Mutex::new(None),
            spectrum: Mutex::new(None),
            audio: Mutex::new(None),
            queue: Mutex::new(VecDeque::new()),
            devices: Some(devices),
            installation: Mutex::new(None),
            layout: Mutex::new(None),
            theme: Mutex::new(None),
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

    /// Publish the arrangement this radio's console should use.
    ///
    /// Set by the wiring layer from what the radio's own crate authored.
    /// A server that sets nothing leaves consoles to their own default,
    /// which is what an older server means by saying nothing.
    pub fn set_layout(&self, layout: cat_layout::LayoutSpec) {
        if let Ok(mut slot) = self.layout.lock() {
            *slot = Some(layout);
        }
    }

    /// Publish the palette this radio's console should use.
    pub fn set_theme(&self, theme: cat_layout::Theme) {
        if let Ok(mut slot) = self.theme.lock() {
            *slot = Some(theme);
        }
    }

    /// Say how to find out what this bench has wired.
    ///
    /// Asked afresh for every connection, so a console that connects after
    /// somebody attached a source is told about it. One already connected
    /// learns from the frames themselves — the handshake happens once.
    pub fn set_installation(&self, installation: InstallationFn) {
        if let Ok(mut slot) = self.installation.lock() {
            *slot = Some(installation);
        }
    }

    /// Publish an audio frame. Newest wins.
    ///
    /// Called from whatever thread owns the sound card, for the same
    /// reason as `publish_spectrum`: capturing audio is blocking I/O and
    /// the FFT is real work, and doing either inside the broker's runtime
    /// would stall every client for the duration.
    pub fn publish_audio(&self, frame: cat_signal::AudioFrame) {
        if let Ok(mut slot) = self.audio.lock() {
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

impl NativeShared {
    /// How many consoles are attached right now.
    pub fn consoles(&self) -> usize {
        self.consoles.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// How long to wait before polling the radio again.
    ///
    /// Display rate only while a console is watching AND the radio is
    /// receiving. **Transmitting backs off even with a console attached**:
    /// nothing about the dial, mode or split changes mid-transmission, and
    /// the link is shared with whatever is keying. A client polling PTT
    /// during a transmission that queues behind a state refresh -- on a
    /// 9600-baud link that occasionally stalls for two seconds -- can time
    /// out and abort the transmission it was watching. A slightly stale
    /// S-meter is a much smaller cost than a truncated transmission.
    pub fn poll_interval(&self, watching: Duration, idle: Duration) -> Duration {
        let transmitting = self
            .state
            .lock()
            .ok()
            .and_then(|s| s.as_ref().map(|s| s.transmitting))
            .unwrap_or(false);
        if self.consoles() > 0 && !transmitting {
            watching
        } else {
            idle
        }
    }
}

impl RadioHost for NativeShared {
    fn capabilities(&self) -> &'static RadioCapabilities {
        self.capabilities
    }

    fn console_attached(&self) {
        self.consoles
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn console_detached(&self) {
        // `fetch_update` rather than `fetch_sub`: an unbalanced detach
        // would wrap a `usize` to enormous and pin the pump at display
        // rate forever, which is the failure this whole mechanism exists
        // to avoid.
        let _ = self.consoles.fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |n| Some(n.saturating_sub(1)),
        );
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
                // Nor these. See the note above: an unread block is
                // reported as absent, not as somebody's defaults.
                levels: None,
            })
    }

    fn devices(&self) -> Option<Vec<cat_signal::DeviceList>> {
        self.devices.as_ref().map(|d| d.list())
    }

    fn apply(&self, command: &Command) -> Result<(), String> {
        // Handled here, not queued. The queue goes to the radio over CAT,
        // and a sound card is not something a Kenwood command set has an
        // opinion about — an attach sent down it would wait out the
        // timeout and then report that the radio did not answer, which is
        // true and entirely beside the point.
        if let Command::AttachDevice { kind, spec } = command {
            return match &self.devices {
                Some(directory) => directory.attach(*kind, spec),
                None => Err("this server does not offer device selection".to_string()),
            };
        }
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

    fn layout(&self) -> Option<cat_layout::LayoutSpec> {
        self.layout.lock().ok().and_then(|l| l.clone())
    }

    fn theme(&self) -> Option<cat_layout::Theme> {
        self.theme.lock().ok().and_then(|t| *t)
    }

    fn installation(&self) -> cat_framework::installation::Installation {
        self.installation
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(|f| f()))
            .unwrap_or_default()
    }

    fn audio(&self) -> Option<cat_signal::AudioFrame> {
        self.audio.lock().ok().and_then(|a| a.clone())
    }
}

/// Drive `radio` from inside the broker's runtime: apply queued commands,
/// then refresh the published state.
///
/// Runs until the process ends. Commands are applied *before* the refresh
/// so that a console's next read reflects what it just asked for rather
/// than lagging a whole poll behind.
/// How much slower to poll when no console is attached.
///
/// Not "never": a console that connects should find a state already
/// there, and a five-second-old dial is a better first frame than a
/// blank one. Slow enough that the link is effectively free for the
/// clients that are actually asking.
const IDLE_MULTIPLIER: u32 = 10;

pub async fn pump<N: NativeRadio>(shared: Arc<NativeShared>, mut radio: N, interval: Duration) {
    // Poll at display rate only while a console is watching. See
    // `NativeShared::consoles`.
    let idle = interval * IDLE_MULTIPLIER;
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
        let wait = if shared.consoles() > 0 {
            interval
        } else {
            idle
        };
        monoio::time::sleep(wait).await;
        #[cfg(not(target_os = "linux"))]
        let wait = if shared.consoles() > 0 {
            interval
        } else {
            idle
        };
        std::thread::sleep(wait);
    }
}

#[cfg(test)]
mod tests {
    /// A receiving radio. `RadioState` has no `Default`, deliberately --
    /// a blank one would be a radio at DC in an invented mode.
    fn rx_state(transmitting: bool) -> RadioState {
        RadioState {
            vfo_a_hz: 14_074_000,
            vfo_b_hz: 14_074_000,
            mode: ModeId::Usb,
            split: false,
            transmitting,
            memory_channel: None,
            if_shift_hz: None,
            filter_width_hz: None,
            meters: Vec::new(),
        }
    }

    #[test]
    fn a_transmitting_radio_is_not_polled_at_display_rate() {
        // A PTT poll that queues behind a state refresh, on a link that
        // occasionally stalls for two seconds, can time out and abort the
        // transmission it was watching. Reported from the bench as "the
        // signal is terminating without being sent fully".
        let fast = Duration::from_millis(200);
        let slow = Duration::from_secs(2);
        let shared = NativeShared::new(&RADIO);
        shared.console_attached();

        shared.publish_state(Some(rx_state(false)));
        assert_eq!(shared.poll_interval(fast, slow), fast, "receiving: keep up");

        shared.publish_state(Some(rx_state(true)));
        assert_eq!(
            shared.poll_interval(fast, slow),
            slow,
            "transmitting: leave the link to whatever is keying"
        );
    }

    #[test]
    fn nobody_watching_backs_off_whatever_the_radio_is_doing() {
        let fast = Duration::from_millis(200);
        let slow = Duration::from_secs(2);
        let shared = NativeShared::new(&RADIO);
        shared.publish_state(Some(rx_state(false)));
        assert_eq!(shared.poll_interval(fast, slow), slow);
    }

    #[test]
    fn a_server_with_nobody_watching_reports_no_consoles() {
        // The whole point: an idle server must be able to tell, so the
        // pump can leave a slow serial link to the clients that are
        // actually asking for something.
        let shared = NativeShared::new(&RADIO);
        assert_eq!(shared.consoles(), 0);
    }

    #[test]
    fn consoles_are_counted_up_and_down() {
        let shared = NativeShared::new(&RADIO);
        shared.console_attached();
        shared.console_attached();
        assert_eq!(shared.consoles(), 2);
        shared.console_detached();
        assert_eq!(shared.consoles(), 1);
        shared.console_detached();
        assert_eq!(shared.consoles(), 0);
    }

    #[test]
    fn an_unbalanced_detach_cannot_wrap_the_count() {
        // `fetch_sub` on a usize at zero wraps to enormous, which would
        // pin the pump at display rate forever -- the exact failure this
        // mechanism exists to avoid, and it would look like the feature
        // simply not working.
        let shared = NativeShared::new(&RADIO);
        shared.console_detached();
        shared.console_detached();
        assert_eq!(shared.consoles(), 0);
        shared.console_attached();
        assert_eq!(shared.consoles(), 1, "and it still counts up afterwards");
    }

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
