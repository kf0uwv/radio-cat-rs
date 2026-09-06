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

//! The join between a capture callback and the reader thread: a bounded
//! ring that a producer writes without ever blocking and a consumer reads
//! as an ordinary [`Read`].
//!
//! # Why this exists at all
//!
//! [`AudioStream::from_reader`](crate::AudioStream::from_reader) already
//! takes any `Read` carrying the wire's format, and the reader thread it
//! spawns already does the DSP and the newest-wins handoff. A sound card
//! does not offer a `Read`: it offers a callback that a real-time audio
//! thread invokes, and which must return *now*. This type is the whole of
//! the adaptation, and it is deliberately outside the `device` feature so
//! that the part with the invariants is compiled and tested in every
//! build.
//!
//! # The producer must never block, so the ring drops
//!
//! A capture callback that blocks does not merely add latency: on ALSA it
//! is being called from a high-priority thread that owns the device, and
//! stalling it makes the *driver* drop samples, where nobody can see or
//! count them. So [`PcmRing::push`] never waits. When the ring is full it
//! discards the **oldest** audio and counts what it discarded.
//!
//! That is the same policy as the frame slot one layer up
//! (`docs/adr/0017-acc2-audio-source.md` §4) and for the same reason: a
//! queue between a real-time producer and a consumer that has fallen
//! behind does not add latency once, it adds it forever. Note that this
//! ring should never actually overflow in practice — the reader thread
//! does a 1024-point FFT per 21 ms of audio and is not the bottleneck —
//! and `overruns()` being non-zero is therefore a signal worth surfacing,
//! not routine.
//!
//! # Whole samples, always
//!
//! Every push and every drop is a whole number of samples, so the byte
//! stream a reader sees can never shift by one byte. That is the same
//! failure `cat-signal-rtlsdr`'s `rtl_tcp` module carries a hand-rolled
//! partial-byte carry to prevent, and the reason it cannot happen here is
//! structural rather than careful.
//!
//! # The end of the stream carries a reason
//!
//! [`RingReader::read`] never returns `Ok(0)`. End-of-stream is always an
//! [`io::Error`] carrying why, because
//! [`stream::describe`](crate::stream) maps a plain EOF to "the peer
//! closed the connection" — true of a socket, and a lie about a USB codec
//! that was unplugged.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use crate::pcm::{self, BYTES_PER_SAMPLE};

/// A bounded, drop-oldest ring of wire-format PCM.
///
/// Shared by `Arc`: the capture callback holds one clone and pushes, the
/// [`RingReader`] handed to the reader thread holds another and reads.
pub struct PcmRing {
    state: Mutex<RingState>,
    ready: Condvar,
    /// Capacity in **bytes**, always even.
    capacity: usize,
    /// Samples thrown away because the consumer was behind.
    overruns: AtomicU64,
}

#[derive(Default)]
struct RingState {
    bytes: VecDeque<u8>,
    /// Why the stream ended, once it has. Terminal, like the frame slot's.
    closed: Option<String>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Matching `stream::lock`: a panic on one side must not turn every
    // later call on the other into a second panic. What is behind this
    // mutex is a byte queue and a reason string, neither of which has an
    // invariant a panic could have left broken.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl PcmRing {
    /// A ring holding at most `capacity_samples` samples.
    ///
    /// One sample is one mono frame of the wire format, so the capacity in
    /// milliseconds is `capacity_samples * 1000 / sample_rate_hz`.
    /// A capacity of zero would make every push an overrun, so it is
    /// raised to one sample.
    pub fn new(capacity_samples: usize) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(RingState::default()),
            ready: Condvar::new(),
            capacity: capacity_samples.max(1) * BYTES_PER_SAMPLE,
            overruns: AtomicU64::new(0),
        })
    }

    /// Push one block of normalized audio. **Never blocks.**
    ///
    /// Samples outside `-1.0..=1.0` are clamped, never wrapped, exactly as
    /// on the transmit path: a wrapped sample turns a loud positive peak
    /// into a loud negative one, which on a *display* is a spurious
    /// wideband transient that looks like a real signal.
    pub fn push(&self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        let mut state = lock(&self.state);
        if state.closed.is_some() {
            return;
        }

        // A single block larger than the whole ring: only its newest tail
        // can survive, and the rest is discarded here rather than pushed
        // and immediately dropped again. Newest wins at both ends.
        let capacity_samples = self.capacity / BYTES_PER_SAMPLE;
        let skipped = samples.len().saturating_sub(capacity_samples);
        let kept = &samples[skipped..];

        let incoming = kept.len() * BYTES_PER_SAMPLE;
        let would_be = state.bytes.len() + incoming;
        let evicted = if would_be > self.capacity {
            // Drop the oldest, in whole samples. `capacity`, `incoming` and
            // `bytes.len()` are all even, so this is too, and the byte
            // stream cannot shift by one. `incoming <= capacity` after the
            // trim above, so this never exceeds what is buffered.
            let excess = would_be - self.capacity;
            state.bytes.drain(..excess);
            excess / BYTES_PER_SAMPLE
        } else {
            0
        };
        if skipped + evicted > 0 {
            self.overruns
                .fetch_add((skipped + evicted) as u64, Ordering::Relaxed);
        }

        for &sample in kept {
            let [lo, hi] = pcm::to_i16(sample).to_le_bytes();
            state.bytes.push_back(lo);
            state.bytes.push_back(hi);
        }
        // One waiter, because there is exactly one reader thread.
        self.ready.notify_one();
    }

    /// End the stream, giving the reason a console will be shown.
    ///
    /// Terminal and idempotent: the first reason wins, so a device error
    /// is not overwritten by the "stopped by the consumer" that follows it
    /// when the capture handle is then dropped.
    pub fn close(&self, reason: impl Into<String>) {
        let mut state = lock(&self.state);
        if state.closed.is_none() {
            state.closed = Some(reason.into());
        }
        // `notify_all`, not `notify_one`: every waiter must learn the
        // stream is over or one of them waits forever on a producer that
        // has gone.
        self.ready.notify_all();
    }

    /// Samples discarded because the consumer was behind.
    ///
    /// Published as the read-only `samples_dropped` setting. Non-zero here
    /// means the *capture* side lost audio, which is a different fault
    /// from `frames_dropped` (the console being slow) and has a different
    /// fix.
    pub fn overruns(&self) -> u64 {
        self.overruns.load(Ordering::Relaxed)
    }

    /// Whether the ring is still accepting audio.
    pub fn is_open(&self) -> bool {
        lock(&self.state).closed.is_none()
    }

    /// The consumer end, to hand to
    /// [`AudioStream::from_reader`](crate::AudioStream::from_reader).
    pub fn reader(self: &Arc<Self>) -> RingReader {
        RingReader {
            ring: Arc::clone(self),
        }
    }

    /// Bytes currently buffered. Test and diagnostics only.
    #[cfg(test)]
    fn buffered(&self) -> usize {
        lock(&self.state).bytes.len()
    }
}

/// The blocking `Read` half of a [`PcmRing`].
///
/// Moved into the reader thread by
/// [`AudioStream::from_reader`](crate::AudioStream::from_reader), which is
/// why it is a separate type: the ring itself stays shared with the
/// capture callback.
pub struct RingReader {
    ring: Arc<PcmRing>,
}

impl Read for RingReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let mut state = lock(&self.ring.state);
        loop {
            if !state.bytes.is_empty() {
                let n = state.bytes.len().min(out.len());
                for (slot, byte) in out[..n].iter_mut().zip(state.bytes.drain(..n)) {
                    *slot = byte;
                }
                return Ok(n);
            }
            if let Some(reason) = &state.closed {
                // Deliberately an error and never `Ok(0)`: see the module
                // header. A plain EOF would reach the console as "the peer
                // closed the connection".
                return Err(io::Error::other(reason.clone()));
            }
            state = self
                .ring
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(from: i16, count: usize) -> Vec<f32> {
        (0..count).map(|i| pcm::from_i16(from + i as i16)).collect()
    }

    fn decode(bytes: &[u8]) -> Vec<i16> {
        bytes
            .chunks_exact(2)
            .map(|p| i16::from_le_bytes([p[0], p[1]]))
            .collect()
    }

    #[test]
    fn what_goes_in_comes_out_in_order_and_in_wire_format() {
        let ring = PcmRing::new(64);
        ring.push(&ramp(100, 4));
        let mut reader = ring.reader();
        let mut buf = [0u8; 8];
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(decode(&buf), vec![100, 101, 102, 103]);
        assert_eq!(ring.overruns(), 0);
    }

    #[test]
    fn a_reader_blocks_until_the_producer_pushes() {
        // The property `read_exact` on the reader thread depends on: an
        // empty ring is "not yet", not "end of stream". A spinning reader
        // would burn a core per stream.
        let ring = PcmRing::new(64);
        let mut reader = ring.reader();
        let producer = Arc::clone(&ring);
        let handle = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            producer.push(&ramp(7, 2));
        });
        let started = std::time::Instant::now();
        let mut buf = [0u8; 4];
        reader.read_exact(&mut buf).unwrap();
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(40),
            "the read returned before the producer pushed"
        );
        assert_eq!(decode(&buf), vec![7, 8]);
        handle.join().unwrap();
    }

    #[test]
    fn a_full_ring_discards_the_oldest_audio_and_counts_it() {
        // Newest-wins, one layer below the frame slot and for the same
        // reason. The live edge is what a console needs to draw.
        let ring = PcmRing::new(4);
        ring.push(&ramp(1, 4)); // 1 2 3 4
        ring.push(&ramp(5, 2)); // pushes 1 and 2 out
        assert_eq!(ring.overruns(), 2);

        let mut reader = ring.reader();
        let mut buf = [0u8; 8];
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(decode(&buf), vec![3, 4, 5, 6]);
    }

    #[test]
    fn a_block_larger_than_the_ring_leaves_the_newest_samples() {
        let ring = PcmRing::new(4);
        ring.push(&ramp(1, 10));
        assert_eq!(ring.buffered(), 8);
        assert_eq!(ring.overruns(), 6);

        let mut reader = ring.reader();
        let mut buf = [0u8; 8];
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(decode(&buf), vec![7, 8, 9, 10]);
    }

    #[test]
    fn every_push_and_every_drop_is_a_whole_number_of_samples() {
        // The structural reason a byte stream out of this ring can never
        // shift by one and swap the halves of every sample after an
        // overrun -- the bug `rtl_tcp` carries a partial-byte carry to
        // avoid.
        let ring = PcmRing::new(5);
        for n in 1..40 {
            ring.push(&ramp(n as i16, n % 7 + 1));
            assert_eq!(ring.buffered() % BYTES_PER_SAMPLE, 0, "at n = {n}");
        }
    }

    #[test]
    fn the_end_of_the_stream_is_an_error_carrying_its_reason() {
        // Never `Ok(0)`: a plain EOF is mapped by `stream::describe` to
        // "the peer closed the connection", which is a lie about an
        // unplugged sound card.
        let ring = PcmRing::new(64);
        ring.close("audio device stopped: DeviceNotAvailable");
        let mut reader = ring.reader();
        let mut buf = [0u8; 4];
        let error = reader.read_exact(&mut buf).unwrap_err();
        assert_ne!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert!(
            error.to_string().contains("DeviceNotAvailable"),
            "unhelpful reason: {error}"
        );
    }

    #[test]
    fn buffered_audio_is_delivered_before_the_end_is_reported() {
        // Matching the frame slot's rule: the last audio a radio sent
        // before the link dropped is still worth drawing.
        let ring = PcmRing::new(64);
        ring.push(&ramp(1, 2));
        ring.close("stopped");
        let mut reader = ring.reader();
        let mut buf = [0u8; 4];
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(decode(&buf), vec![1, 2]);
        assert!(reader.read_exact(&mut buf).is_err());
    }

    #[test]
    fn closing_keeps_the_first_reason() {
        // A device error must not be overwritten by the "stopped by the
        // consumer" that follows when the handle is then dropped.
        let ring = PcmRing::new(64);
        ring.close("audio device stopped: DeviceNotAvailable");
        ring.close("capture stopped by the consumer");
        let mut reader = ring.reader();
        let error = reader.read_exact(&mut [0u8; 2]).unwrap_err();
        assert!(error.to_string().contains("DeviceNotAvailable"));
        assert!(!ring.is_open());
    }

    #[test]
    fn closing_wakes_a_blocked_reader() {
        // What makes dropping a capture handle actually stop the reader
        // thread. Without it the worker sits in `read_exact` forever and
        // the thread leaks, which is the mistake ADR 0016's consequences
        // record for `cat-transport-rfc2217`.
        let ring = PcmRing::new(64);
        let mut reader = ring.reader();
        let closer = Arc::clone(&ring);
        let handle = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            closer.close("capture stopped by the consumer");
        });
        let error = reader.read_exact(&mut [0u8; 2]).unwrap_err();
        assert!(error.to_string().contains("stopped by the consumer"));
        handle.join().unwrap();
    }

    #[test]
    fn a_push_after_the_close_is_ignored_rather_than_resurrecting_the_stream() {
        let ring = PcmRing::new(64);
        ring.close("stopped");
        ring.push(&ramp(1, 4));
        assert_eq!(ring.buffered(), 0);
    }

    #[test]
    fn the_ring_feeds_the_real_pipeline_end_to_end() {
        // The claim this whole module exists to support: a capture
        // callback's samples come out of `AudioStream` as frames, through
        // the same `from_reader` seam and the same reader thread the
        // socket path uses.
        use crate::{AudioPipelineConfig, AudioStream};

        let ring = PcmRing::new(48_000);
        let mut stream = AudioStream::from_reader(ring.reader(), AudioPipelineConfig::default());

        let producer = Arc::clone(&ring);
        let feeder = std::thread::spawn(move || {
            for block in 0..40 {
                let samples: Vec<f32> = (0..512)
                    .map(|i| {
                        let t = (block * 512 + i) as f64 / 48_000.0;
                        (0.5 * (std::f64::consts::TAU * 1_000.0 * t).sin()) as f32
                    })
                    .collect();
                producer.push(&samples);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            producer.close("test source finished");
        });

        let frame = stream.blocking_next_frame().expect("a frame from the ring");
        assert_eq!(frame.scope.samples.len(), 1024);
        assert_eq!(frame.scope.sample_rate_hz, 48_000);
        // A 1 kHz tone at 46.875 Hz per bin lands in bin 21, and it should
        // be the loudest thing in the frame.
        let peak =
            frame
                .spectrum
                .bins
                .iter()
                .enumerate()
                .fold(
                    (0usize, f32::MIN),
                    |best, (i, &v)| {
                        if v > best.1 {
                            (i, v)
                        } else {
                            best
                        }
                    },
                );
        assert_eq!(peak.0, 21, "1 kHz should land in bin 21");
        feeder.join().unwrap();
    }
}
