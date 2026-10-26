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

//! Mono 16-bit PCM WAV output (`std` feature): the format WSJT-X reads and
//! writes for a slot (12 kHz, 180 000 samples).

use std::io::{self, Write};

/// Writes `samples` (full scale ±1.0, clipped beyond) as a mono 16-bit PCM
/// WAV at `sample_rate`. Returns how many samples were clipped.
pub fn write_wav<W: Write>(mut w: W, samples: &[f32], sample_rate: u32) -> io::Result<usize> {
    let too_big = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "too many samples for a WAV file",
        )
    };
    let data_len = samples
        .len()
        .checked_mul(2)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(too_big)?;
    let riff_len = data_len.checked_add(36).ok_or_else(too_big)?;
    let byte_rate = sample_rate.checked_mul(2).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "sample rate too high for a WAV file",
        )
    })?;
    let mut header = std::vec::Vec::with_capacity(44);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&riff_len.to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    header.extend_from_slice(&1u16.to_le_bytes()); // PCM
    header.extend_from_slice(&1u16.to_le_bytes()); // mono
    header.extend_from_slice(&sample_rate.to_le_bytes());
    header.extend_from_slice(&byte_rate.to_le_bytes());
    header.extend_from_slice(&2u16.to_le_bytes()); // block align
    header.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    header.extend_from_slice(b"data");
    header.extend_from_slice(&data_len.to_le_bytes());
    w.write_all(&header)?;

    let mut clipped = 0;
    let mut body = std::vec::Vec::with_capacity(samples.len() * 2);
    for &x in samples {
        let v = (x * 32767.0).round();
        if !(-32768.0..=32767.0).contains(&v) {
            clipped += 1;
        }
        let v = v.clamp(-32768.0, 32767.0) as i16;
        body.extend_from_slice(&v.to_le_bytes());
    }
    w.write_all(&body)?;
    Ok(clipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_samples() {
        let mut buf = std::vec::Vec::new();
        let clipped = write_wav(&mut buf, &[0.0, 1.0, -2.0], 12_000).unwrap();
        assert_eq!(clipped, 1);
        assert_eq!(buf.len(), 44 + 6);
        assert_eq!(&buf[..4], b"RIFF");
        assert_eq!(&buf[24..28], &12_000u32.to_le_bytes());
        assert_eq!(&buf[44..], &[0, 0, 0xff, 0x7f, 0x00, 0x80]);
    }

    #[test]
    fn rejects_an_overflowing_sample_rate() {
        let mut buf = std::vec::Vec::new();
        assert!(write_wav(&mut buf, &[0.0], u32::MAX).is_err());
        assert!(buf.is_empty());
    }
}
