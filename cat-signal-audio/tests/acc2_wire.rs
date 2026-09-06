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

//! The ACC2 audio pair, end to end, over a real socket.
//!
//! # Why this file exists rather than more unit tests
//!
//! Everything in `dsp` and `pcm` can be proved by handing a function an
//! array, and all of it is. None of that proves anything about the thing
//! that will actually be plugged in: a full-duplex TCP connection carrying
//! 48 kHz signed 16-bit little-endian PCM, paced to real time by a radio at
//! the other end.
//!
//! So the fake server here speaks exactly that wire and nothing else — no
//! greeting, no framing, no length prefix, no handshake — because that is
//! what `ts570d`'s emulator serves on `--acc2-audio`. Every frame these
//! tests assert on came off a socket.
//!
//! This crate cannot depend on `ts570d` (a consumer never becomes a
//! dependency), so the wire is re-stated here. If the two ever disagree,
//! that is a real defect and this is where it should surface.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cat_signal::{SettingValue, SignalCapability, Unit};
use cat_signal_audio::{AudioError, AudioFrame, AudioPipelineConfig, AudioSource, AudioStream};

const RATE: u32 = 48_000;
const BLOCK: usize = 1_024;
/// One block of 1024 samples at 48 kHz, to the nearest microsecond.
const REAL_TIME_BLOCK: Duration = Duration::from_micros(21_333);

// ---------------------------------------------------------------------
// A fake radio that speaks the ACC2 audio wire.
// ---------------------------------------------------------------------

struct Serve {
    /// Delay between blocks. `REAL_TIME_BLOCK` is what a radio does.
    pace: Duration,
    /// Stop sending and hang up after this many blocks.
    limit: Option<u64>,
    /// Write in pieces of this many bytes, to force reads that land
    /// mid-sample.
    chunk: Option<usize>,
}

impl Default for Serve {
    fn default() -> Self {
        Self {
            pace: REAL_TIME_BLOCK,
            limit: None,
            chunk: None,
        }
    }
}

struct Radio {
    port: u16,
    /// Blocks successfully written to the socket.
    written: Arc<AtomicU64>,
    /// Everything the client sent on PKD.
    received: Arc<Mutex<Vec<u8>>>,
    /// Set when the client's transmit direction reached end-of-stream.
    pkd_ended: Arc<AtomicBool>,
}

impl Radio {
    fn connect(
        &self,
        config: AudioPipelineConfig,
    ) -> (AudioStream, cat_signal_audio::AudioTransmitter) {
        AudioStream::connect(("127.0.0.1", self.port), config).expect("connect to the fake radio")
    }
}

/// Serve `block(index)` bytes forever (or until `opts.limit`), reading
/// whatever the client sends on PKD.
fn serve(block: impl Fn(u64) -> Vec<u8> + Send + 'static, opts: Serve) -> Radio {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let written = Arc::new(AtomicU64::new(0));
    let received = Arc::new(Mutex::new(Vec::new()));
    let pkd_ended = Arc::new(AtomicBool::new(false));

    let radio = Radio {
        port,
        written: Arc::clone(&written),
        received: Arc::clone(&received),
        pkd_ended: Arc::clone(&pkd_ended),
    };

    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream.set_nodelay(true).unwrap();

        // The link is full duplex, so the radio reads PKD on its own
        // thread. A server that only read between writes would deadlock
        // against a client doing the same thing, and would prove nothing
        // about a full-duplex wire.
        let reader = stream.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 4_096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => received.lock().unwrap().extend_from_slice(&buf[..n]),
                }
            }
            pkd_ended.store(true, Ordering::Release);
        });

        let mut stream = stream;
        let mut index = 0u64;
        loop {
            if opts.limit.is_some_and(|n| index >= n) {
                let _ = stream.shutdown(Shutdown::Write);
                return;
            }
            let bytes = block(index);
            let ok = match opts.chunk {
                None => stream.write_all(&bytes).is_ok(),
                Some(size) => {
                    let mut ok = true;
                    for piece in bytes.chunks(size) {
                        if stream.write_all(piece).is_err() {
                            ok = false;
                            break;
                        }
                        std::thread::sleep(Duration::from_micros(200));
                    }
                    ok
                }
            };
            if !ok {
                return;
            }
            written.fetch_add(1, Ordering::Release);
            index += 1;
            if !opts.pace.is_zero() {
                std::thread::sleep(opts.pace);
            }
        }
    });

    radio
}

/// One block of a sine at `hz`, as wire bytes.
fn tone(hz: f64, amplitude: f32) -> impl Fn(u64) -> Vec<u8> + Send + 'static {
    move |index| {
        let start = index as usize * BLOCK;
        let mut out = Vec::with_capacity(BLOCK * 2);
        for i in 0..BLOCK {
            let p = std::f64::consts::TAU * hz * (start + i) as f64 / f64::from(RATE);
            let s = (amplitude * p.sin() as f32).clamp(-1.0, 1.0);
            out.extend_from_slice(&((s * 32_767.0).round() as i16).to_le_bytes());
        }
        out
    }
}

/// One block of digital silence — what a *transmitting* radio sends.
fn silence(_index: u64) -> Vec<u8> {
    vec![0u8; BLOCK * 2]
}

/// A block whose every sample is its own block index, so a frame can say
/// exactly which moment of the stream it came from.
fn counted(index: u64) -> Vec<u8> {
    let code = (index % 30_000) as i16;
    code.to_le_bytes()
        .iter()
        .copied()
        .cycle()
        .take(BLOCK * 2)
        .collect()
}

/// Recover the block index a `counted` frame was made from.
fn block_index(frame: &AudioFrame) -> i64 {
    (frame.scope.samples[0] * 32_767.0).round() as i64
}

fn peak_bin(frame: &AudioFrame) -> usize {
    frame
        .spectrum
        .bins
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap()
        .0
}

fn next(stream: &mut AudioStream) -> AudioFrame {
    futures::executor::block_on(stream.next_frame()).expect("a frame off the socket")
}

// ---------------------------------------------------------------------
// Receive: ANO, the radio's audio, off a real socket.
// ---------------------------------------------------------------------

#[test]
fn a_tone_off_the_socket_arrives_in_both_domains() {
    let radio = serve(tone(1_000.0, 0.5), Serve::default());
    let (mut stream, _tx) = radio.connect(AudioPipelineConfig::default());

    let frame = next(&mut stream);

    // Time domain: the trace is the audio, at the wire's own rate.
    assert_eq!(frame.scope.sample_rate_hz, RATE);
    assert_eq!(frame.scope.samples.len(), BLOCK);
    assert!(
        (frame.scope.peak() - 0.5).abs() < 0.02,
        "a half-scale tone read {} on the trace",
        frame.scope.peak()
    );
    assert!(!frame.scope.is_clipping());

    // Frequency domain: at the frequency it was actually generated at.
    let hz = frame.spectrum.audio_frequency_hz(peak_bin(&frame)).unwrap();
    assert!(
        (hz - 1_000.0).abs() < frame.spectrum.bin_width_hz() * 2.0,
        "1 kHz off the wire was reported at {hz:.0} Hz"
    );

    // ...and the two halves describe the same instant.
    assert_eq!(frame.scope.sequence, frame.spectrum.sequence);
}

#[test]
fn a_transmitting_radio_sends_silence_and_that_is_a_frame_not_a_fault() {
    // The wire is continuous in both directions: a transmitting radio sends
    // digital silence on ANO. A source that treated a run of zeroes as a
    // dead link would drop the console's audio display every time the
    // operator keyed up.
    let radio = serve(silence, Serve::default());
    let (mut stream, _tx) = radio.connect(AudioPipelineConfig::default());

    for _ in 0..3 {
        let frame = next(&mut stream);
        assert_eq!(frame.scope.peak(), 0.0);
        assert!(frame.spectrum.bins.iter().all(|b| b.is_finite()));
        assert!(frame.spectrum.bins.iter().all(|b| *b <= -100.0));
    }
    assert!(stream.is_connected());
}

#[test]
fn frames_arrive_at_the_rate_the_radio_paces_them() {
    // The property no array-fed test can show: the server paces to real
    // time and the client tracks it, rather than free-running through a
    // buffer. Five 1024-sample blocks at 48 kHz is ~85 ms of audio.
    let radio = serve(tone(1_000.0, 0.5), Serve::default());
    let (mut stream, _tx) = radio.connect(AudioPipelineConfig::default());

    next(&mut stream);
    let started = Instant::now();
    for _ in 0..4 {
        next(&mut stream);
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed > Duration::from_millis(40),
        "four blocks arrived in {elapsed:?}; the client is not tracking real time"
    );
    assert!(
        elapsed < Duration::from_millis(600),
        "four blocks took {elapsed:?}; the client is falling behind the radio"
    );
}

#[test]
fn a_peer_that_writes_three_bytes_at_a_time_still_produces_correct_audio() {
    // TCP does not respect sample boundaries. `cat-signal-rtlsdr`'s
    // `rtl_tcp` module carries a hand-rolled partial-byte carry because a
    // read landing mid-sample swapped I and Q for every sample after it,
    // and the only symptom was a mirrored spectrum. Odd-sized writes are
    // what found it.
    let radio = serve(
        tone(1_000.0, 0.5),
        Serve {
            pace: Duration::ZERO,
            chunk: Some(3),
            ..Default::default()
        },
    );
    let (mut stream, _tx) = radio.connect(AudioPipelineConfig::default());

    for _ in 0..3 {
        let frame = next(&mut stream);
        let hz = frame.spectrum.audio_frequency_hz(peak_bin(&frame)).unwrap();
        assert!(
            (hz - 1_000.0).abs() < frame.spectrum.bin_width_hz() * 3.0,
            "awkward chunking shifted the tone to {hz:.0} Hz"
        );
        assert!((frame.scope.peak() - 0.5).abs() < 0.05);
    }
}

// ---------------------------------------------------------------------
// Backpressure: the newest frame wins.
// ---------------------------------------------------------------------

#[test]
fn a_slow_consumer_sees_recent_audio_not_a_backlog() {
    // The claim the whole design rests on. A queue between a real-time
    // producer and a slow consumer does not add latency once -- it adds it
    // forever, because the producer never slows down.
    //
    // Each block here is stamped with its own index, so a frame says
    // exactly which moment of the stream it came from. A console that had
    // been served from a queue would come back holding block 1.
    let radio = serve(
        counted,
        Serve {
            // 20x real time, so a short sleep covers many blocks without
            // the test itself taking a second.
            pace: Duration::from_millis(1),
            ..Default::default()
        },
    );
    let (mut stream, _tx) = radio.connect(AudioPipelineConfig::default());

    let first = next(&mut stream);
    let first_index = block_index(&first);

    // The console goes away to do something else -- draw, handle a key,
    // wait on CAT.
    std::thread::sleep(Duration::from_millis(200));

    let served_before_we_looked = radio.written.load(Ordering::Acquire) as i64;
    let second = next(&mut stream);
    let second_index = block_index(&second);

    assert!(
        second_index - first_index > 50,
        "only {} blocks passed during a 200 ms sleep -- either the radio is \
         not streaming, or frames were queued for us rather than dropped",
        second_index - first_index
    );
    // The load-bearing assertion: we came back to the LIVE EDGE, not to
    // block first_index + 1.
    assert!(
        second_index + 25 >= served_before_we_looked,
        "came back holding block {second_index} while the radio had sent \
         {served_before_we_looked} -- that is a backlog, not the newest frame"
    );
    // Frames were dropped, and both the counter and the sequence say so.
    assert!(
        stream.frames_dropped() > 20,
        "only {} frames dropped; nothing was actually overwritten",
        stream.frames_dropped()
    );
    assert!(
        second.sequence() - first.sequence() > 1,
        "sequence went {} -> {} with no gap, so nothing was dropped",
        first.sequence(),
        second.sequence()
    );

    // ...and having caught up, the next frame is adjacent, not another
    // jump: the slot holds one frame, never a queue that drains slowly.
    let third = next(&mut stream);
    assert!(
        block_index(&third) - second_index < 25,
        "after catching up the stream jumped again, from {second_index} to {}",
        block_index(&third)
    );
}

#[test]
fn frames_dropped_is_visible_to_an_operator() {
    // A counter nobody can see does not turn "the audio feels laggy" into
    // a diagnosis. ADR 0014 section 3's point, restated in the audio domain.
    let radio = serve(
        counted,
        Serve {
            pace: Duration::from_millis(1),
            ..Default::default()
        },
    );
    let (mut stream, _tx) = radio.connect(AudioPipelineConfig::default());
    next(&mut stream);
    std::thread::sleep(Duration::from_millis(100));
    next(&mut stream);

    let settings = stream.settings();
    let dropped = settings.find("frames_dropped").expect("a dropped counter");
    assert_eq!(dropped.access, cat_signal::Access::ReadOnly);
    match dropped.value {
        SettingValue::Int { value, .. } => assert!(value > 0, "counter stuck at {value}"),
        ref other => panic!("expected an integer count, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Transmit: PKD, audio into the radio.
// ---------------------------------------------------------------------

#[test]
fn transmit_audio_reaches_the_radio_as_the_bytes_the_wire_specifies() {
    let radio = serve(silence, Serve::default());
    let (_stream, transmit) = radio.connect(AudioPipelineConfig::default());

    let clipped = transmit.send(&[0.0, 0.5, -0.5, 1.0]).unwrap();
    assert_eq!(clipped, 0);

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let got = radio.received.lock().unwrap().clone();
        if got.len() >= 8 {
            // Signed 16-bit little-endian, exactly: 0, +16384, -16384,
            // full scale. Hard-coded rather than computed, because a test
            // that re-derives the encoding cannot catch the encoding
            // changing.
            assert_eq!(&got[..8], &[0x00, 0x00, 0x00, 0x40, 0x00, 0xC0, 0xFF, 0x7F]);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "PKD audio never arrived: {got:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn hot_transmit_audio_is_clamped_and_counted_never_wrapped() {
    // A wrapped sample turns a loud positive peak into a loud negative one:
    // splatter on the air that the operator's monitor cannot hear.
    let radio = serve(silence, Serve::default());
    let (_stream, transmit) = radio.connect(AudioPipelineConfig::default());

    assert_eq!(transmit.send(&[2.0, -3.0, 0.1]).unwrap(), 2);

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let got = radio.received.lock().unwrap().clone();
        if got.len() >= 6 {
            assert_eq!(i16::from_le_bytes([got[0], got[1]]), i16::MAX);
            assert_eq!(i16::from_le_bytes([got[2], got[3]]), -i16::MAX);
            break;
        }
        assert!(Instant::now() < deadline, "PKD audio never arrived");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn silence_on_pkd_is_data_and_flows_alongside_incoming_audio() {
    // Full duplex, both ways at once: the client sends silence on PKD while
    // the radio is sending audio on ANO, and neither blocks the other.
    let radio = serve(tone(1_000.0, 0.5), Serve::default());
    let (mut stream, transmit) = radio.connect(AudioPipelineConfig::default());

    for _ in 0..3 {
        transmit.send_silence(BLOCK).unwrap();
        let frame = next(&mut stream);
        assert!(
            frame.scope.peak() > 0.4,
            "receive stalled while transmitting"
        );
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    while radio.received.lock().unwrap().len() < BLOCK * 2 * 3 {
        assert!(Instant::now() < deadline, "PKD silence never arrived");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(radio.received.lock().unwrap().iter().all(|b| *b == 0));
}

#[test]
fn closing_transmit_leaves_receive_running() {
    // The two directions of one connection finish independently. A dead
    // microphone must not look like a dead receiver.
    let radio = serve(tone(1_000.0, 0.5), Serve::default());
    let (mut stream, transmit) = radio.connect(AudioPipelineConfig::default());

    next(&mut stream);
    transmit.close().unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    while !radio.pkd_ended.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline, "the radio never saw PKD end");
        std::thread::sleep(Duration::from_millis(5));
    }

    for _ in 0..3 {
        assert!(next(&mut stream).scope.peak() > 0.4);
    }
    assert!(stream.is_connected());
}

// ---------------------------------------------------------------------
// Losing the peer.
// ---------------------------------------------------------------------

#[test]
fn a_radio_that_goes_away_mid_stream_ends_the_stream_and_says_why() {
    let radio = serve(
        tone(1_000.0, 0.5),
        Serve {
            pace: Duration::from_millis(1),
            limit: Some(6),
            ..Default::default()
        },
    );
    let (mut stream, _tx) = radio.connect(AudioPipelineConfig::default());

    let mut frames = 0;
    let reason = loop {
        match futures::executor::block_on(stream.next_frame()) {
            Ok(_) => {
                frames += 1;
                assert!(frames < 50, "the stream never ended");
            }
            Err(AudioError::Closed { reason }) => break reason,
            Err(other) => panic!("unexpected error: {other}"),
        }
    };

    // Every frame the radio managed to send is delivered before the end is
    // reported: the last audio before a link dropped is still worth drawing.
    assert!(frames >= 1, "the last good frames were thrown away");
    assert!(
        reason.contains("closed") || reason.contains("connection"),
        "unhelpful reason: {reason}"
    );
    assert!(!stream.is_connected());

    // Terminal: it keeps saying the same thing rather than blocking
    // forever on a producer that has gone.
    for _ in 0..3 {
        match futures::executor::block_on(stream.next_frame()) {
            Err(AudioError::Closed { reason: r }) => assert_eq!(r, reason),
            other => panic!("expected the same terminal error, got {other:?}"),
        }
    }
}

#[test]
fn dropping_both_halves_closes_the_connection() {
    // `cat-transport-rfc2217` learned this one the hard way (ADR 0016):
    // the worker thread holds its own `Arc`, so letting the handles fall
    // out of scope frees nothing unless `Drop` unblocks the read.
    let radio = serve(tone(1_000.0, 0.5), Serve::default());
    let (mut stream, transmit) = radio.connect(AudioPipelineConfig::default());
    next(&mut stream);

    drop(stream);
    drop(transmit);

    let deadline = Instant::now() + Duration::from_secs(5);
    while !radio.pkd_ended.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "the radio still has an open connection to a program that has gone"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

// ---------------------------------------------------------------------
// Settings, over a live link.
// ---------------------------------------------------------------------

#[test]
fn changing_the_fft_size_takes_effect_without_dropping_the_link() {
    // The settings a console writes are read by the reader thread at the
    // top of each block. If that handoff were broken the symptom would be
    // a knob that silently does nothing.
    let radio = serve(
        tone(1_000.0, 0.5),
        Serve {
            pace: Duration::from_millis(1),
            ..Default::default()
        },
    );
    let (mut stream, _tx) = radio.connect(AudioPipelineConfig::default());
    assert_eq!(next(&mut stream).scope.samples.len(), 1_024);

    stream
        .apply(
            "fft_size",
            SettingValue::Enum {
                value: 3, // "2048"
                options: &["256", "512", "1024", "2048", "4096"],
            },
        )
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let frame = next(&mut stream);
        if frame.scope.samples.len() == 2_048 {
            // The scope window follows the FFT size, and says so.
            assert!((frame.scope.window_ms() - 42.67).abs() < 0.01);
            break;
        }
        assert!(Instant::now() < deadline, "the FFT size never changed");
    }
    assert!(stream.is_connected());
}

#[test]
fn narrowing_the_span_narrows_the_spectrum_it_reports() {
    let radio = serve(
        tone(1_000.0, 0.5),
        Serve {
            pace: Duration::from_millis(1),
            ..Default::default()
        },
    );
    let (mut stream, _tx) = radio.connect(AudioPipelineConfig::default());
    assert_eq!(next(&mut stream).spectrum.bins.len(), 86);

    stream
        .apply(
            "span_hz",
            SettingValue::Int {
                value: 2_000,
                min: 100,
                max: 24_000,
                step: 100,
                unit: Unit::Hz,
            },
        )
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let frame = next(&mut stream);
        if frame.spectrum.bins.len() == 43 {
            assert!(frame.spectrum.span_hz >= 2_000 && frame.spectrum.span_hz < 2_100);
            // The tone is still where it was; a narrower span is a
            // truncation of the same FFT, never a resampling of it.
            let hz = frame.spectrum.audio_frequency_hz(peak_bin(&frame)).unwrap();
            assert!((hz - 1_000.0).abs() < frame.spectrum.bin_width_hz() * 2.0);
            break;
        }
        assert!(Instant::now() < deadline, "the span never changed");
    }
}

#[test]
fn a_live_link_still_reports_itself_as_audio_only() {
    let radio = serve(tone(1_000.0, 0.5), Serve::default());
    let (mut stream, _tx) = radio.connect(AudioPipelineConfig::default());
    next(&mut stream);

    let capability = stream.capability();
    assert!(
        !capability.is_band_panorama(),
        "an audio stream must never be drawable as a band waterfall"
    );
    assert_eq!(
        capability,
        SignalCapability::AudioDerived {
            max_bandwidth_hz: 24_000
        }
    );

    let settings = stream.settings();
    assert_eq!(
        settings.find("connected").map(|d| d.value.clone()),
        Some(SettingValue::Bool(true))
    );
}
