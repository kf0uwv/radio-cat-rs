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

//! The sample codec: signed 16-bit little-endian PCM to and from the
//! normalized `-1.0..=1.0` floats [`AudioScopeFrame`] promises.
//!
//! Pure, hardware-free, and separated from the socket for the same reason
//! `cat-signal-rtlsdr`'s `dsp` module is: this is the arithmetic that is
//! easy to get *silently* wrong, and it should be testable without a
//! radio, a socket, or a thread.
//!
//! [`AudioScopeFrame`]: cat_signal::AudioScopeFrame
//!
//! # Why 32767 and not 32768
//!
//! Dividing by 32768 is the tidier-looking choice and it breaks the one
//! thing this scaling has to support. `AudioScopeFrame::is_clipping()` asks
//! whether any sample reached full scale (`peak() >= 1.0`); with a divisor
//! of 32768 a genuinely clipped capture sitting at +32767 reads 0.99997 and
//! the indicator never lights. Scaling by 32767 and clamping makes **both**
//! full-scale codes read exactly ±1.0, keeps every sample inside the
//! documented range, and round-trips exactly against [`to_i16`].

/// Bytes per sample on the wire. Mono, signed 16-bit.
pub const BYTES_PER_SAMPLE: usize = 2;

/// Full-scale magnitude. See the module header for why this is 32767.
const FULL_SCALE: f32 = 32_767.0;

/// One little-endian sample pair to a normalized float.
#[inline]
pub fn from_i16(sample: i16) -> f32 {
    (f32::from(sample) / FULL_SCALE).clamp(-1.0, 1.0)
}

/// A normalized float to one wire sample.
///
/// Out-of-range input is **clamped, never wrapped**. A wrapped sample flips
/// a loud positive peak to a loud negative one, which on a transmit path is
/// broadband splatter that sounds, to the operator, like nothing at all.
#[inline]
pub fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * FULL_SCALE).round() as i16
}

/// Decode a whole block of wire bytes.
///
/// `bytes.len()` must be even; a trailing odd byte is impossible here
/// because every read is a `read_exact` of `2 * block` bytes (see
/// [`crate::stream`]), and it is discarded rather than silently shifting
/// every sample that follows.
pub fn decode(bytes: &[u8], out: &mut Vec<f32>) {
    out.clear();
    out.reserve(bytes.len() / BYTES_PER_SAMPLE);
    out.extend(
        bytes
            .chunks_exact(BYTES_PER_SAMPLE)
            .map(|p| from_i16(i16::from_le_bytes([p[0], p[1]]))),
    );
}

/// Encode a block for transmission, returning how many samples had to be
/// clamped.
///
/// The count is returned rather than logged because it is the only honest
/// overdrive indicator a transmit path has: the operator's audio chain is
/// upstream of this crate, and a console that can say "142 samples clipped"
/// can tell them to turn it down.
pub fn encode(samples: &[f32], out: &mut Vec<u8>) -> usize {
    out.clear();
    out.reserve(samples.len() * BYTES_PER_SAMPLE);
    let mut clipped = 0;
    for &s in samples {
        if !(-1.0..=1.0).contains(&s) {
            clipped += 1;
        }
        out.extend_from_slice(&to_i16(s).to_le_bytes());
    }
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_signal::AudioScopeFrame;

    #[test]
    fn full_scale_reads_as_clipping_in_both_directions() {
        // The reason the divisor is 32767. With 32768 the positive
        // full-scale code reads 0.99997 and `is_clipping()` -- the whole
        // point of the field -- silently never fires.
        let frame = AudioScopeFrame {
            sample_rate_hz: 48_000,
            samples: vec![from_i16(i16::MAX), from_i16(i16::MIN)],
            sequence: 1,
        };
        assert_eq!(frame.samples[0], 1.0);
        assert_eq!(frame.samples[1], -1.0);
        assert!(frame.is_clipping());
    }

    #[test]
    fn every_sample_lands_inside_the_documented_range() {
        for code in [i16::MIN, -1, 0, 1, i16::MAX] {
            let v = from_i16(code);
            assert!((-1.0..=1.0).contains(&v), "{code} decoded to {v}");
        }
    }

    #[test]
    fn silence_is_silence() {
        // A transmitting radio sends digital silence on ANO. That must be
        // an ordinary, valid frame -- not an error, and not a DC offset.
        let mut out = Vec::new();
        decode(&[0u8; 64], &mut out);
        assert_eq!(out.len(), 32);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn samples_are_little_endian() {
        // The one byte-order mistake that produces something plausible:
        // big-endian decoding of quiet audio looks like loud noise, which
        // a renderer will happily draw.
        let mut out = Vec::new();
        decode(&[0x00, 0x40], &mut out); // 0x4000 = +16384 LE
        assert!((out[0] - 0.5).abs() < 0.001, "got {}", out[0]);
    }

    #[test]
    fn encoding_round_trips_through_the_wire_format() {
        let original = [0.0f32, 0.5, -0.5, 1.0, -1.0];
        let mut bytes = Vec::new();
        assert_eq!(encode(&original, &mut bytes), 0);
        let mut back = Vec::new();
        decode(&bytes, &mut back);
        for (a, b) in original.iter().zip(&back) {
            assert!((a - b).abs() < 1e-4, "{a} round-tripped to {b}");
        }
    }

    #[test]
    fn a_hot_sample_is_clamped_and_counted_never_wrapped() {
        // Wrapping turns a loud positive peak into a loud negative one:
        // broadband splatter on the air that is inaudible in the monitor.
        let mut bytes = Vec::new();
        let clipped = encode(&[2.0, -2.0, 0.25], &mut bytes);
        assert_eq!(clipped, 2);
        let mut back = Vec::new();
        decode(&bytes, &mut back);
        assert_eq!(back[0], 1.0);
        assert_eq!(back[1], -1.0);
        assert!(back[2] > 0.0);
    }

    #[test]
    fn a_trailing_odd_byte_is_dropped_not_shifted_into_the_next_sample() {
        let mut out = Vec::new();
        decode(&[0x00, 0x40, 0x11], &mut out);
        assert_eq!(out.len(), 1);
        assert!((out[0] - 0.5).abs() < 0.001);
    }
}
