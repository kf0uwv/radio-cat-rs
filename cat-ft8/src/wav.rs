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

//! 16-bit PCM WAV files (`std` feature): mono output in the format WSJT-X
//! reads and writes for a slot (12 kHz, 180 000 samples), and input.

use std::io::{self, Read, Write};

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

/// A 16-bit PCM WAV file's contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wav {
    /// Samples per second (per channel).
    pub sample_rate: u32,
    /// Channels; `samples` interleaves them.
    pub channels: u16,
    /// The samples, interleaved by channel.
    pub samples: std::vec::Vec<i16>,
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, std::string::String::from(msg))
}

/// Reads a RIFF/WAVE file of 16-bit PCM samples (format tag 1, or
/// `WAVE_FORMAT_EXTENSIBLE` with the PCM subtype), any rate and channel
/// count. Chunks other than `fmt ` and `data` are skipped. A `data` chunk
/// that claims more bytes than the file holds (as some streaming writers
/// leave it) is read to the end of the file. Other encodings are an
/// [`io::ErrorKind::InvalidData`] error naming what was found.
pub fn read_wav<R: Read>(r: R) -> io::Result<Wav> {
    read_wav_prefix(r, usize::MAX)
}

/// As [`read_wav`], but reads at most `max_frames` frames (samples per
/// channel) and nothing after them: the rest of the file is not read.
pub fn read_wav_prefix<R: Read>(mut r: R, max_frames: usize) -> io::Result<Wav> {
    let mut head = [0u8; 12];
    if read_full(&mut r, &mut head)? < 12 || &head[..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return Err(invalid("not a RIFF/WAVE file"));
    }
    let u16_at = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let mut format: Option<(u32, u16)> = None;
    loop {
        let mut chunk = [0u8; 8];
        if read_full(&mut r, &mut chunk)? < 8 {
            return Err(invalid("WAV file has no data chunk"));
        }
        let len = u64::from(u32_at(&chunk, 4));
        match &chunk[..4] {
            b"fmt " => {
                let mut body = std::vec![0u8; len.min(64) as usize];
                let got = read_full(&mut r, &mut body)?;
                skip(&mut r, len - got as u64 + (len & 1))?;
                if got < 16 {
                    return Err(invalid("WAV fmt chunk too short"));
                }
                let body = &body[..got];
                let mut tag = u16_at(body, 0);
                let channels = u16_at(body, 2);
                let rate = u32_at(body, 4);
                let bits = u16_at(body, 14);
                if tag == 0xfffe && body.len() >= 26 {
                    tag = u16_at(body, 24); // the subformat GUID's first two bytes
                }
                if tag != 1 {
                    return Err(invalid(&std::format!(
                        "WAV encoding {tag:#06x} is not supported (16-bit PCM only)"
                    )));
                }
                if bits != 16 {
                    return Err(invalid(&std::format!(
                        "{bits}-bit WAV samples are not supported (16-bit PCM only)"
                    )));
                }
                if channels == 0 {
                    return Err(invalid("WAV file with no channels"));
                }
                format = Some((rate, channels));
            }
            b"data" => {
                let (sample_rate, channels) =
                    format.ok_or_else(|| invalid("WAV data chunk before its fmt chunk"))?;
                let want = (max_frames as u64)
                    .saturating_mul(u64::from(channels))
                    .saturating_mul(2)
                    .min(len);
                let mut bytes = std::vec::Vec::new();
                r.by_ref().take(want).read_to_end(&mut bytes)?;
                let samples = bytes
                    .chunks_exact(2)
                    .map(|p| i16::from_le_bytes([p[0], p[1]]))
                    .collect();
                return Ok(Wav {
                    sample_rate,
                    channels,
                    samples,
                });
            }
            // Chunks are padded to an even length.
            _ => skip(&mut r, len + (len & 1))?,
        }
    }
}

/// Reads until `buf` is full or the input ends; returns the bytes read.
fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// Reads and discards `n` bytes (fewer if the input ends).
fn skip<R: Read>(r: &mut R, n: u64) -> io::Result<()> {
    io::copy(&mut r.by_ref().take(n), &mut io::sink())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::string::ToString;

    #[test]
    fn read_back_what_was_written() {
        let mut buf = std::vec::Vec::new();
        write_wav(&mut buf, &[0.0, 0.5, -0.5, 1.0], 12_000).unwrap();
        let wav = read_wav(&buf[..]).unwrap();
        assert_eq!((wav.sample_rate, wav.channels), (12_000, 1));
        assert_eq!(wav.samples, [0, 16_384, -16_384, 32_767]);
    }

    /// A WAV file from parts: format tag, channels, rate, bits, an extra
    /// chunk before `data` (odd-sized), and the data bytes.
    fn build(
        tag: u16,
        channels: u16,
        rate: u32,
        bits: u16,
        extra: bool,
        data: &[u8],
    ) -> std::vec::Vec<u8> {
        let mut b = std::vec::Vec::new();
        b.extend_from_slice(b"RIFF\0\0\0\0WAVE");
        if tag == 0xfffe {
            b.extend_from_slice(b"fmt ");
            b.extend_from_slice(&40u32.to_le_bytes());
            b.extend_from_slice(&tag.to_le_bytes());
        } else {
            b.extend_from_slice(b"fmt ");
            b.extend_from_slice(&16u32.to_le_bytes());
            b.extend_from_slice(&tag.to_le_bytes());
        }
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        b.extend_from_slice(&(channels * 2).to_le_bytes());
        b.extend_from_slice(&bits.to_le_bytes());
        if tag == 0xfffe {
            b.extend_from_slice(&22u16.to_le_bytes());
            b.extend_from_slice(&bits.to_le_bytes());
            b.extend_from_slice(&4u32.to_le_bytes());
            b.extend_from_slice(&1u16.to_le_bytes()); // PCM subtype
            b.extend_from_slice(&[0; 14]);
        }
        if extra {
            b.extend_from_slice(b"LIST");
            b.extend_from_slice(&3u32.to_le_bytes());
            b.extend_from_slice(b"abc\0"); // 3 bytes and a pad byte
        }
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(data);
        b
    }

    #[test]
    fn extensible_format_extra_chunks_and_stereo() {
        let data = [1u8, 0, 2, 0, 3, 0, 4, 0];
        let wav = read_wav(&build(0xfffe, 2, 48_000, 16, true, &data)[..]).unwrap();
        assert_eq!((wav.sample_rate, wav.channels), (48_000, 2));
        assert_eq!(wav.samples, [1, 2, 3, 4]);
    }

    #[test]
    fn a_data_chunk_longer_than_the_file_is_read_to_the_end() {
        let mut b = build(1, 1, 12_000, 16, false, &[5, 0, 6, 0, 7]);
        let n = b.len();
        b[n - 9..n - 5].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(read_wav(&b[..]).unwrap().samples, [5, 6]);
    }

    #[test]
    fn a_prefix_reads_only_the_frames_asked_for() {
        let data: std::vec::Vec<u8> = (0u8..40).collect();
        let b = build(1, 2, 12_000, 16, true, &data);
        let wav = read_wav_prefix(&b[..], 3).unwrap();
        assert_eq!(wav.samples.len(), 6); // 3 frames of 2 channels
        assert_eq!(wav.samples[0], i16::from_le_bytes([0, 1]));
        // Nothing past the frames asked for is read.
        let mut reader = &b[..];
        read_wav_prefix(&mut reader, 3).unwrap();
        assert_eq!(reader.len(), 40 - 12);
    }

    #[test]
    fn unsupported_files_are_rejected_with_a_reason() {
        let err = |b: &[u8]| read_wav(b).unwrap_err().to_string();
        assert!(err(b"hello").contains("RIFF"));
        assert!(err(&build(3, 1, 12_000, 32, false, &[0; 8])).contains("0x0003"));
        assert!(err(&build(1, 1, 12_000, 24, false, &[0; 6])).contains("24-bit"));
        let mut no_data = build(1, 1, 12_000, 16, false, &[]);
        no_data.truncate(no_data.len() - 8);
        assert!(err(&no_data).contains("no data"));
    }

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
