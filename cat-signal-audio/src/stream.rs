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

//! The socket, the reader thread, and the single-slot handoff.
//!
//! The shape is `cat-signal-rtlsdr`'s `device` module (ADR 0014 §2-3): a
//! dedicated `std::thread` doing the blocking I/O, and one slot holding the
//! newest frame, overwritten rather than queued.
//!
//! # One difference from `cat-signal-rtlsdr`, and it is the important one
//!
//! `RtlSdrSource` keeps **raw IQ** in the slot and runs the FFT in the
//! frame pump. Doing that here would be a bug. If the console's rate gated
//! the socket reads, the kernel receive buffer would become the unbounded
//! queue, and the audio drawn would fall further behind real time forever —
//! the exact failure the latest-frame policy exists to prevent. Moving the
//! queue from our process into the kernel does not make it not a queue.
//!
//! So the worker reads **and** does the DSP, and the slot holds finished
//! [`AudioFrame`]s. The socket is drained at real time whatever the console
//! is doing, and staleness is bounded to one block.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use cat_signal::{
    Access, SettingDescriptor, SettingGroup, SettingValue, SignalCapability, SpectrumSettings, Unit,
};

use crate::dsp::{AudioPipeline, AudioPipelineConfig};
use crate::pcm::{self, BYTES_PER_SAMPLE};
use crate::{AudioError, AudioFrame, AudioSource};

/// FFT sizes offered as a setting. Mirrors `cat-signal-rtlsdr`'s list so a
/// generic settings panel behaves identically in both domains.
const FFT_SIZES: &[&str] = &["256", "512", "1024", "2048", "4096"];

/// Everything the reader thread and the console share.
struct Shared {
    slot: Mutex<Slot>,
    ready: Condvar,
    /// Frames produced but never collected, because the console was behind.
    dropped: AtomicU64,
    /// Set by [`AudioStream`]'s `Drop`, so the worker stops even if the
    /// peer is still sending.
    stop: AtomicBool,
    /// Read by the worker at the top of every block, so a setting written
    /// from the console side takes effect without tearing the link down.
    params: Mutex<AudioPipelineConfig>,
}

#[derive(Default)]
struct Slot {
    /// The newest frame, or none collected yet.
    frame: Option<AudioFrame>,
    /// Why the stream ended, once it has. Terminal.
    ended: Option<String>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic in the worker must not turn every later call into a second
    // panic in the console. The data behind this mutex is a frame and a
    // reason string; neither has an invariant a panic could have broken.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Wraps a shared socket so the worker can read from it while a
/// [`AudioTransmitter`] writes to it.
///
/// `&TcpStream` implements both `Read` and `Write`, so one `Arc` serves
/// both directions and no `try_clone` (or its per-platform failure modes)
/// is needed.
struct SharedSocket(Arc<TcpStream>);

impl Read for SharedSocket {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        (&*self.0).read(buf)
    }
}

/// A radio's audio endpoint, as a source of [`AudioFrame`]s.
///
/// Construct with [`connect`](Self::connect) for the real thing, or
/// [`from_reader`](Self::from_reader) to drive the same pipeline from any
/// byte source.
pub struct AudioStream {
    shared: Arc<Shared>,
    /// Present only for a socket-backed stream; used by `Drop` to unblock
    /// the worker's read.
    socket: Option<Arc<TcpStream>>,
    _worker: std::thread::JoinHandle<()>,
}

impl AudioStream {
    /// Connect to a radio's audio endpoint.
    ///
    /// Returns the receive side and the transmit side of the **same** TCP
    /// connection. A caller that wants only receive can drop the
    /// transmitter; that leaves the write half open and idle, which is
    /// exactly right, because an idle PKD pin does nothing.
    pub fn connect<A: ToSocketAddrs>(
        addr: A,
        config: AudioPipelineConfig,
    ) -> Result<(Self, AudioTransmitter), AudioError> {
        let socket = TcpStream::connect(addr).map_err(AudioError::Connect)?;
        // Audio is a latency-sensitive stream of small blocks; Nagle would
        // coalesce transmit blocks and add delay for no benefit.
        socket.set_nodelay(true).map_err(AudioError::Connect)?;
        let socket = Arc::new(socket);

        let stream = Self::spawn(
            SharedSocket(Arc::clone(&socket)),
            config,
            Some(Arc::clone(&socket)),
        );
        let transmitter = AudioTransmitter {
            inner: Arc::new(TransmitInner {
                socket,
                buffer: Mutex::new(Vec::new()),
            }),
        };
        Ok((stream, transmitter))
    }

    /// Drive the pipeline from any byte source carrying the same wire
    /// format.
    ///
    /// There is no transmit path here — a `Read` has no return direction —
    /// so this is for tests, for replaying a capture, and for embedding the
    /// pipeline behind some other kind of link. The worker ends at EOF.
    pub fn from_reader<R: Read + Send + 'static>(reader: R, config: AudioPipelineConfig) -> Self {
        Self::spawn(reader, config, None)
    }

    fn spawn<R: Read + Send + 'static>(
        reader: R,
        config: AudioPipelineConfig,
        socket: Option<Arc<TcpStream>>,
    ) -> Self {
        let shared = Arc::new(Shared {
            slot: Mutex::new(Slot::default()),
            ready: Condvar::new(),
            dropped: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            params: Mutex::new(config),
        });
        let worker_shared = Arc::clone(&shared);
        let worker = std::thread::Builder::new()
            .name("cat-audio-rx".into())
            .spawn(move || run(reader, worker_shared, config))
            .expect("spawn the audio reader thread");
        Self {
            shared,
            socket,
            _worker: worker,
        }
    }

    /// Take the newest frame if one is waiting, without blocking.
    ///
    /// **This is what a console's render loop should call.** It never
    /// blocks, so it cannot stall a monoio executor that is also driving
    /// the CAT session; `Ok(None)` simply means no new block has arrived
    /// since the last call.
    pub fn try_next_frame(&mut self) -> Result<Option<AudioFrame>, AudioError> {
        let mut slot = lock(&self.shared.slot);
        if let Some(frame) = slot.frame.take() {
            return Ok(Some(frame));
        }
        match &slot.ended {
            Some(reason) => Err(AudioError::Closed {
                reason: reason.clone(),
            }),
            None => Ok(None),
        }
    }

    /// Block until a frame arrives or the stream ends.
    ///
    /// The synchronous form of [`AudioSource::next_frame`], for a caller
    /// that is not in an async context at all.
    pub fn blocking_next_frame(&mut self) -> Result<AudioFrame, AudioError> {
        let mut slot = lock(&self.shared.slot);
        loop {
            // Drain before reporting the end: the last good frame a radio
            // sent before the link dropped is still worth drawing.
            if let Some(frame) = slot.frame.take() {
                return Ok(frame);
            }
            if let Some(reason) = &slot.ended {
                return Err(AudioError::Closed {
                    reason: reason.clone(),
                });
            }
            slot = self
                .shared
                .ready
                .wait(slot)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Frames produced but never collected, because the console was behind.
    ///
    /// Also published as the read-only `frames_dropped` setting. A number a
    /// user can see is what turns "the audio feels laggy" into "you are
    /// dropping 80% of frames."
    pub fn frames_dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }

    /// Whether the stream is still live.
    ///
    /// Once this is false it stays false: there is no reconnect here (ADR
    /// 0017 §6). An application that wants one calls
    /// [`connect`](Self::connect) again.
    pub fn is_connected(&self) -> bool {
        lock(&self.shared.slot).ended.is_none()
    }

    /// The pipeline configuration currently in force.
    pub fn config(&self) -> AudioPipelineConfig {
        *lock(&self.shared.params)
    }
}

impl Drop for AudioStream {
    fn drop(&mut self) {
        // The worker holds its own `Arc` to the shared state, so letting
        // this fall out of scope frees nothing on its own -- the thread
        // would sit in `read_exact` forever. `cat-transport-rfc2217` learned
        // this the same way (ADR 0016's Consequences).
        self.shared.stop.store(true, Ordering::Release);
        if let Some(socket) = &self.socket {
            // Read only. A cloned `AudioTransmitter` may still be sending
            // audio into a radio, and killing that from the receive side's
            // destructor would be a surprise. The connection closes for
            // real when the last handle drops.
            let _ = socket.shutdown(Shutdown::Read);
        }
        // Not joined. A worker blocked in a read on a peer that has gone
        // silent without closing would hang the console's shutdown, and a
        // detached thread that ends on its next syscall costs nothing.
    }
}

/// The reader thread: blocking read, DSP, newest-wins handoff.
fn run<R: Read>(mut reader: R, shared: Arc<Shared>, config: AudioPipelineConfig) {
    let mut pipeline = AudioPipeline::new(config);
    let mut bytes: Vec<u8> = Vec::new();
    let mut samples: Vec<f32> = Vec::new();

    loop {
        if shared.stop.load(Ordering::Acquire) {
            finish(&shared, "closed by the consumer".to_string());
            return;
        }

        let wanted = *lock(&shared.params);
        if wanted != pipeline.config() {
            pipeline.reconfigure(wanted);
        }
        let block = pipeline.block_size();
        if block == 0 {
            finish(&shared, "block size of zero".to_string());
            return;
        }

        bytes.resize(block * BYTES_PER_SAMPLE, 0);
        // `read_exact`, so a read landing mid-sample is unrepresentable.
        // `cat-signal-rtlsdr`'s `rtl_tcp` module carries a hand-rolled
        // partial-byte carry for exactly this, and the bug it fixed --
        // I and Q swapped for every sample after one short read -- was
        // invisible except as a mirrored spectrum.
        if let Err(e) = reader.read_exact(&mut bytes) {
            finish(&shared, describe(&e));
            return;
        }

        pcm::decode(&bytes, &mut samples);
        if let Some(frame) = pipeline.process(&samples) {
            let mut slot = lock(&shared.slot);
            if slot.frame.is_some() {
                // Newest wins. Count what the console missed.
                shared.dropped.fetch_add(1, Ordering::Relaxed);
            }
            slot.frame = Some(frame);
            shared.ready.notify_one();
        }
    }
}

fn finish(shared: &Arc<Shared>, reason: String) {
    let mut slot = lock(&shared.slot);
    if slot.ended.is_none() {
        slot.ended = Some(reason);
    }
    // `notify_all`, not `notify_one`: every waiter must learn the stream is
    // over, or one of them waits forever on a producer that has gone.
    shared.ready.notify_all();
}

fn describe(e: &std::io::Error) -> String {
    match e.kind() {
        // What a peer that hung up cleanly looks like from `read_exact`.
        std::io::ErrorKind::UnexpectedEof => "the peer closed the connection".to_string(),
        _ => e.to_string(),
    }
}

#[async_trait(?Send)]
impl AudioSource for AudioStream {
    type Error = AudioError;

    async fn next_frame(&mut self) -> Result<AudioFrame, Self::Error> {
        // No waker is registered and no task is woken from another thread:
        // this blocks the calling thread on a condvar, which is what keeps
        // ADR 0016's monoio `sync`-feature trap out of this crate entirely.
        // A render loop should call `try_next_frame` instead.
        self.blocking_next_frame()
    }

    fn capability(&self) -> SignalCapability {
        // Nyquist, not the current `span_hz`: this is what the source
        // *could* show, and it is the number that tells a console this can
        // never be a band panorama.
        SignalCapability::AudioDerived {
            max_bandwidth_hz: self.config().sample_rate_hz / 2,
        }
    }

    fn settings(&self) -> SpectrumSettings {
        let config = self.config();
        let nyquist = i64::from(config.sample_rate_hz / 2);
        SpectrumSettings::new(vec![
            SettingDescriptor {
                key: "sample_rate_hz",
                label: "Sample rate",
                group: SettingGroup::Source,
                access: Access::ReadOnly,
                value: SettingValue::Int {
                    value: i64::from(config.sample_rate_hz),
                    min: 8_000,
                    max: 192_000,
                    step: 1,
                    unit: Unit::Sps,
                },
            },
            // Read-only and deliberately visible: an operator whose audio
            // display has gone blank needs to be able to tell "the radio
            // stopped" from "the display stopped".
            SettingDescriptor {
                key: "connected",
                label: "Connected",
                group: SettingGroup::Source,
                access: Access::ReadOnly,
                value: SettingValue::Bool(self.is_connected()),
            },
            SettingDescriptor {
                key: "fft_size",
                label: "FFT size",
                group: SettingGroup::Display,
                access: Access::ReadWrite,
                value: SettingValue::Enum {
                    value: fft_size_index(config.fft_size),
                    options: FFT_SIZES,
                },
            },
            SettingDescriptor {
                key: "span_hz",
                label: "AF span",
                group: SettingGroup::Display,
                access: Access::ReadWrite,
                value: SettingValue::Int {
                    value: i64::from(config.span_hz),
                    min: 100,
                    max: nyquist,
                    step: 100,
                    unit: Unit::Hz,
                },
            },
            SettingDescriptor {
                key: "averaging",
                label: "Spectrum averaging",
                group: SettingGroup::Display,
                access: Access::ReadWrite,
                value: SettingValue::Float {
                    value: f64::from(config.averaging),
                    min: 0.0,
                    max: 0.95,
                    unit: Unit::None,
                },
            },
            SettingDescriptor {
                key: "frames_dropped",
                label: "Frames dropped",
                group: SettingGroup::Display,
                access: Access::ReadOnly,
                value: SettingValue::Int {
                    value: self.frames_dropped() as i64,
                    min: 0,
                    max: i64::MAX,
                    step: 1,
                    unit: Unit::None,
                },
            },
        ])
    }

    fn apply(&mut self, key: &str, value: SettingValue) -> Result<(), Self::Error> {
        let descriptor = self
            .settings()
            .find(key)
            .cloned()
            .ok_or_else(|| AudioError::UnknownSetting(key.to_string()))?;

        if descriptor.access == Access::ReadOnly {
            return Err(AudioError::ReadOnly(descriptor.key));
        }
        if !descriptor.value.same_kind_as(&value) {
            return Err(AudioError::WrongKind(descriptor.key));
        }
        if !value.is_valid() {
            return Err(AudioError::OutOfRange(descriptor.key));
        }

        let mut params = lock(&self.shared.params);
        match (descriptor.key, &value) {
            ("fft_size", SettingValue::Enum { value: v, .. }) => {
                params.fft_size = FFT_SIZES[*v as usize].parse().expect("static table");
            }
            ("span_hz", SettingValue::Int { value: v, .. }) => {
                params.span_hz = *v as u32;
            }
            ("averaging", SettingValue::Float { value: v, .. }) => {
                params.averaging = *v as f32;
            }
            _ => return Err(AudioError::UnknownSetting(key.to_string())),
        }
        Ok(())
    }
}

fn fft_size_index(size: usize) -> u16 {
    FFT_SIZES
        .iter()
        .position(|s| s.parse::<usize>() == Ok(size))
        .unwrap_or(2) as u16
}

/// The transmit half: audio into the radio's PKD pin.
///
/// # This keys nothing
///
/// Sending audio here does **not** put the radio into transmit. There is no
/// VOX, no PTT and no CAT command anywhere in this type. On the station
/// this was built for, keying is DTR through an opto-isolator, and audio
/// arriving at a receiving radio's PKD pin does nothing at all.
///
/// Said this plainly because a transmit path that *looks* like it keys is
/// the kind of thing someone tries once, into an antenna.
///
/// `Clone` and `Send`, so a console can hand one to whatever thread is
/// capturing the operator's microphone or generating a digital-mode tone.
/// Writes from several clones are serialized, so blocks never interleave
/// mid-sample.
#[derive(Clone)]
pub struct AudioTransmitter {
    inner: Arc<TransmitInner>,
}

struct TransmitInner {
    socket: Arc<TcpStream>,
    /// The encode buffer, reused, and the write lock in one: holding it
    /// across the `write_all` is what stops two clones interleaving.
    buffer: Mutex<Vec<u8>>,
}

impl AudioTransmitter {
    /// Send one block of normalized audio, returning how many samples had
    /// to be clamped.
    ///
    /// A non-zero return is the honest overdrive indicator: the operator's
    /// audio chain is upstream of this crate, and a console that can say
    /// "142 samples clipped" can tell them to turn it down. Out-of-range
    /// input is clamped and never wrapped.
    ///
    /// **Blocking, and paced by the radio.** The peer consumes at real
    /// time, so a caller that pushes faster than 48 000 samples per second
    /// will fill the socket buffer and then block here. That is the correct
    /// backpressure for a transmit path — dropping transmit audio would put
    /// a gap on the air — and it is the opposite of the receive side's
    /// policy for the same reason: stale receive audio is worthless, and
    /// unsent transmit audio is not.
    pub fn send(&self, samples: &[f32]) -> Result<usize, AudioError> {
        let mut buffer = lock(&self.inner.buffer);
        let clipped = pcm::encode(samples, &mut buffer);
        (&*self.inner.socket)
            .write_all(&buffer)
            .map_err(|e| AudioError::Closed {
                reason: format!("transmit failed: {e}"),
            })?;
        Ok(clipped)
    }

    /// Send `samples` of digital silence.
    ///
    /// What a client sends while it has nothing to say. The stream is
    /// continuous in both directions, so silence is data.
    pub fn send_silence(&self, samples: usize) -> Result<(), AudioError> {
        let mut buffer = lock(&self.inner.buffer);
        buffer.clear();
        buffer.resize(samples * BYTES_PER_SAMPLE, 0);
        (&*self.inner.socket)
            .write_all(&buffer)
            .map_err(|e| AudioError::Closed {
                reason: format!("transmit failed: {e}"),
            })
    }

    /// Close the transmit direction, leaving receive alone.
    ///
    /// The peer sees end-of-stream on PKD and keeps sending ANO. The two
    /// directions of this connection fail and finish independently, because
    /// a half-open link is a real state and pretending otherwise would turn
    /// a dead microphone into a dead receiver.
    pub fn close(&self) -> Result<(), AudioError> {
        self.inner
            .socket
            .shutdown(Shutdown::Write)
            .map_err(|e| AudioError::Closed {
                reason: format!("transmit shutdown failed: {e}"),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader that yields `blocks` copies of a fixed byte block, then
    /// ends. No socket: this exercises the worker and the slot alone.
    struct Canned {
        block: Vec<u8>,
        remaining: usize,
        cursor: usize,
    }

    impl Read for Canned {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.remaining == 0 {
                return Ok(0);
            }
            let available = &self.block[self.cursor..];
            let n = available.len().min(buf.len());
            buf[..n].copy_from_slice(&available[..n]);
            self.cursor += n;
            if self.cursor == self.block.len() {
                self.cursor = 0;
                self.remaining -= 1;
            }
            Ok(n)
        }
    }

    fn tone_bytes(len: usize, hz: f64) -> Vec<u8> {
        let samples: Vec<f32> = (0..len)
            .map(|i| {
                let p = std::f64::consts::TAU * hz * i as f64 / 48_000.0;
                0.5 * p.sin() as f32
            })
            .collect();
        let mut out = Vec::new();
        pcm::encode(&samples, &mut out);
        out
    }

    fn canned(blocks: usize) -> AudioStream {
        AudioStream::from_reader(
            Canned {
                block: tone_bytes(1024, 1_000.0),
                remaining: blocks,
                cursor: 0,
            },
            AudioPipelineConfig::default(),
        )
    }

    #[test]
    fn a_frame_arrives_with_both_domains_populated() {
        let mut stream = canned(4);
        let frame = stream.blocking_next_frame().unwrap();
        assert_eq!(frame.scope.samples.len(), 1024);
        assert_eq!(frame.scope.sample_rate_hz, 48_000);
        assert_eq!(frame.spectrum.bins.len(), 86);
        assert_eq!(frame.sequence(), frame.spectrum.sequence);
    }

    #[test]
    fn the_end_of_the_stream_is_reported_after_the_last_good_frame() {
        // Draining before reporting matters: the last audio a radio sent
        // before the link dropped is still worth drawing.
        let mut stream = canned(2);
        assert!(stream.blocking_next_frame().is_ok());
        let mut ended = false;
        for _ in 0..3 {
            if let Err(AudioError::Closed { reason }) = stream.blocking_next_frame() {
                assert!(reason.contains("closed"), "unhelpful reason: {reason}");
                ended = true;
                break;
            }
        }
        assert!(ended, "the stream never reported its end");
        assert!(!stream.is_connected());
    }

    #[test]
    fn the_closed_state_is_terminal_and_keeps_saying_the_same_thing() {
        let mut stream = canned(1);
        let mut reason = None;
        for _ in 0..4 {
            if let Err(AudioError::Closed { reason: r }) = stream.blocking_next_frame() {
                reason = Some(r);
                break;
            }
        }
        let first = reason.expect("the stream should have ended");
        for _ in 0..3 {
            match stream.blocking_next_frame() {
                Err(AudioError::Closed { reason }) => assert_eq!(reason, first),
                other => panic!("expected the same terminal error, got {other:?}"),
            }
        }
    }

    #[test]
    fn try_next_frame_never_blocks() {
        // The call a render loop makes. `Ok(None)` means "nothing new",
        // which is an ordinary answer and not an error.
        let mut stream = AudioStream::from_reader(
            Canned {
                block: tone_bytes(1024, 1_000.0),
                remaining: usize::MAX,
                cursor: 0,
            },
            AudioPipelineConfig::default(),
        );
        let mut got = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while got < 3 && std::time::Instant::now() < deadline {
            let started = std::time::Instant::now();
            let polled = stream
                .try_next_frame()
                .expect("the canned reader never ends");
            // The claim under test: the call itself returns immediately
            // whether or not there was anything to collect.
            assert!(
                started.elapsed() < std::time::Duration::from_millis(50),
                "try_next_frame blocked for {:?}",
                started.elapsed()
            );
            if polled.is_some() {
                got += 1;
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        assert_eq!(got, 3, "never saw frames through the non-blocking path");
    }

    #[test]
    fn settings_expose_the_audio_knobs_and_refuse_the_read_only_ones() {
        let mut stream = canned(2);
        let settings = stream.settings();
        assert!(settings.is_writable("fft_size"));
        assert!(settings.is_writable("span_hz"));
        assert!(settings.is_writable("averaging"));
        assert!(!settings.is_writable("sample_rate_hz"));
        assert!(!settings.is_writable("frames_dropped"));
        assert!(!settings.is_writable("connected"));

        assert!(matches!(
            stream.apply(
                "sample_rate_hz",
                SettingValue::Int {
                    value: 44_100,
                    min: 8_000,
                    max: 192_000,
                    step: 1,
                    unit: Unit::Sps
                }
            ),
            Err(AudioError::ReadOnly("sample_rate_hz"))
        ));
        assert!(matches!(
            stream.apply("span_hz", SettingValue::Bool(true)),
            Err(AudioError::WrongKind("span_hz"))
        ));
        assert!(matches!(
            stream.apply(
                "span_hz",
                SettingValue::Int {
                    value: 999_999,
                    min: 100,
                    max: 24_000,
                    step: 100,
                    unit: Unit::Hz
                }
            ),
            Err(AudioError::OutOfRange("span_hz"))
        ));
        assert!(matches!(
            stream.apply("nope", SettingValue::Bool(true)),
            Err(AudioError::UnknownSetting(_))
        ));
    }

    #[test]
    fn this_source_can_never_be_mistaken_for_a_band_panorama() {
        // `cat-signal`'s whole reason for `AudioDerived`.
        let stream = canned(1);
        let capability = stream.capability();
        assert!(!capability.is_band_panorama());
        assert!(capability.is_present());
        assert_eq!(
            capability,
            SignalCapability::AudioDerived {
                max_bandwidth_hz: 24_000
            }
        );
    }
}
