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

//! An [`IqSource`] that speaks `rtl_tcp`.
//!
//! `rtl_tcp` is the protocol `librtlsdr` ships for putting a dongle on the
//! network: a twelve-byte greeting, then unsigned 8-bit interleaved I/Q
//! forever, with five-byte commands going the other way.
//!
//! It is worth implementing for a reason beyond convenience. A tap that
//! speaks it is interchangeable with a real dongle *and* with the
//! emulator's virtual one, so the console, the DSP and every correction
//! run identically against both. A bespoke wire format between the
//! emulator and the control program would test the bespoke format.
//!
//! Unlike the `device` module this needs no C toolchain, so it is **not**
//! behind a feature flag: it compiles and is tested everywhere.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};

use rustfft::num_complex::Complex32;

use crate::IqSource;

/// Commands `rtl_tcp` accepts. The values are librtlsdr's, not ours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RtlTcpCommand {
    SetFrequency = 0x01,
    SetSampleRate = 0x02,
    SetGainMode = 0x03,
    SetGain = 0x04,
}

/// The greeting a server sends on connect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dongle {
    /// Four ASCII bytes; a real dongle sends `RTL0`.
    pub magic: [u8; 4],
    pub tuner_id: u32,
    pub gain_count: u32,
}

#[derive(Debug)]
pub enum RtlTcpError {
    Io(std::io::Error),
    /// The greeting was absent or not `RTL0`.
    NotADongle([u8; 4]),
    Closed,
}

impl std::fmt::Display for RtlTcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RtlTcpError::Io(e) => write!(f, "rtl_tcp io error: {e}"),
            RtlTcpError::NotADongle(m) => {
                write!(f, "peer is not an rtl_tcp server (magic {m:?})")
            }
            RtlTcpError::Closed => write!(f, "rtl_tcp connection closed"),
        }
    }
}

impl std::error::Error for RtlTcpError {}

impl From<std::io::Error> for RtlTcpError {
    fn from(e: std::io::Error) -> Self {
        RtlTcpError::Io(e)
    }
}

/// A dongle on the other end of a socket.
pub struct RtlTcpSource {
    stream: TcpStream,
    dongle: Dongle,
    /// Leftover byte when a read ends mid-sample. IQ is pairs, and a
    /// stream does not respect that: dropping the odd byte would swap I
    /// and Q for every sample after the first short read, which sounds
    /// like nothing and looks like a spectrum mirrored about DC.
    partial: Option<u8>,
}

impl RtlTcpSource {
    /// Connect and read the greeting.
    pub fn connect<A: ToSocketAddrs>(addr: A) -> Result<Self, RtlTcpError> {
        let mut stream = TcpStream::connect(addr)?;
        stream.set_nodelay(true)?;
        let mut greeting = [0u8; 12];
        stream.read_exact(&mut greeting)?;
        let magic = [greeting[0], greeting[1], greeting[2], greeting[3]];
        if &magic != b"RTL0" {
            return Err(RtlTcpError::NotADongle(magic));
        }
        Ok(Self {
            stream,
            dongle: Dongle {
                magic,
                tuner_id: u32::from_be_bytes(greeting[4..8].try_into().unwrap()),
                gain_count: u32::from_be_bytes(greeting[8..12].try_into().unwrap()),
            },
            partial: None,
        })
    }

    pub fn dongle(&self) -> Dongle {
        self.dongle
    }

    /// Send a five-byte command: one byte of opcode, four big-endian.
    pub fn command(&mut self, command: RtlTcpCommand, value: u32) -> Result<(), RtlTcpError> {
        let mut frame = [0u8; 5];
        frame[0] = command as u8;
        frame[1..].copy_from_slice(&value.to_be_bytes());
        self.stream.write_all(&frame)?;
        self.stream.flush()?;
        Ok(())
    }

    /// Retune the dongle.
    ///
    /// For an IF tap this is set once and left: the SDR sits on the fixed
    /// intermediate frequency while the radio's LO does the tuning.
    pub fn set_frequency_hz(&mut self, hz: u32) -> Result<(), RtlTcpError> {
        self.command(RtlTcpCommand::SetFrequency, hz)
    }

    pub fn set_sample_rate_hz(&mut self, hz: u32) -> Result<(), RtlTcpError> {
        self.command(RtlTcpCommand::SetSampleRate, hz)
    }
}

impl IqSource for RtlTcpSource {
    type Error = RtlTcpError;

    fn read(&mut self, wanted: usize) -> Result<Vec<Complex32>, Self::Error> {
        let mut bytes = Vec::with_capacity(wanted * 2);
        if let Some(b) = self.partial.take() {
            bytes.push(b);
        }
        let mut buf = vec![0u8; wanted * 2];
        while bytes.len() < wanted * 2 {
            let n = self.stream.read(&mut buf)?;
            if n == 0 {
                return Err(RtlTcpError::Closed);
            }
            bytes.extend_from_slice(&buf[..n]);
        }
        // Keep any odd trailing byte for next time rather than discarding
        // it -- see `partial`.
        if bytes.len() % 2 == 1 {
            self.partial = bytes.pop();
        }
        Ok(bytes
            .chunks_exact(2)
            .map(|pair| {
                // Unsigned 8-bit, offset 127, scaled to roughly -1..1 --
                // the conversion every rtl_tcp consumer does.
                Complex32::new(
                    (f32::from(pair[0]) - 127.5) / 127.5,
                    (f32::from(pair[1]) - 127.5) / 127.5,
                )
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// A server that greets, then sends `body` forever.
    fn serve(magic: &'static [u8; 4], body: Vec<u8>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut greeting = [0u8; 12];
            greeting[..4].copy_from_slice(magic);
            greeting[4..8].copy_from_slice(&5u32.to_be_bytes());
            greeting[8..12].copy_from_slice(&29u32.to_be_bytes());
            if stream.write_all(&greeting).is_err() {
                return;
            }
            loop {
                if stream.write_all(&body).is_err() {
                    return;
                }
            }
        });
        port
    }

    #[test]
    fn a_greeting_identifies_the_dongle() {
        let port = serve(b"RTL0", vec![127; 64]);
        let source = RtlTcpSource::connect(("127.0.0.1", port)).unwrap();
        assert_eq!(&source.dongle().magic, b"RTL0");
        assert_eq!(source.dongle().tuner_id, 5);
        assert_eq!(source.dongle().gain_count, 29);
    }

    #[test]
    fn something_that_is_not_a_dongle_is_refused_rather_than_decoded() {
        // Pointing this at the wrong port would otherwise read HTTP as IQ
        // and render it as a spectrum, which looks like noise and is very
        // hard to recognise as a misconfiguration.
        let port = serve(b"HTTP", vec![0; 64]);
        assert!(matches!(
            RtlTcpSource::connect(("127.0.0.1", port)),
            Err(RtlTcpError::NotADongle(_))
        ));
    }

    #[test]
    fn samples_are_centred_on_zero_the_way_the_dongle_means_them() {
        // 127/128 straddle the midpoint; a consumer that forgot the offset
        // would see a large DC component and put a spike at the centre of
        // every spectrum.
        let port = serve(b"RTL0", vec![127, 128]);
        let mut source = RtlTcpSource::connect(("127.0.0.1", port)).unwrap();
        let iq = source.read(4).unwrap();
        assert_eq!(iq.len(), 4);
        for s in iq {
            assert!(s.re.abs() < 0.01, "I not centred: {}", s.re);
            assert!(s.im.abs() < 0.01, "Q not centred: {}", s.im);
        }
    }

    #[test]
    fn a_read_that_lands_mid_sample_does_not_swap_i_and_q_forever_after() {
        // A stream does not respect sample boundaries. Discarding the odd
        // trailing byte swaps I and Q for every sample that follows, which
        // renders as a spectrum mirrored about DC -- plausible-looking and
        // very hard to trace back.
        let port = serve(b"RTL0", vec![200, 60]);
        let mut source = RtlTcpSource::connect(("127.0.0.1", port)).unwrap();
        for _ in 0..8 {
            // Odd counts force the boundary to land mid-pair repeatedly.
            for sample in source.read(3).unwrap() {
                assert!(sample.re > 0.0, "I and Q swapped: {sample:?}");
                assert!(sample.im < 0.0, "I and Q swapped: {sample:?}");
            }
        }
    }

    #[test]
    fn a_command_is_five_bytes_big_endian() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut greeting = [0u8; 12];
            greeting[..4].copy_from_slice(b"RTL0");
            stream.write_all(&greeting).unwrap();
            let mut cmd = [0u8; 5];
            stream.read_exact(&mut cmd).unwrap();
            cmd
        });
        let mut source = RtlTcpSource::connect(("127.0.0.1", port)).unwrap();
        source.set_frequency_hz(73_050_000).unwrap();
        let cmd = handle.join().unwrap();
        assert_eq!(cmd[0], RtlTcpCommand::SetFrequency as u8);
        assert_eq!(u32::from_be_bytes(cmd[1..].try_into().unwrap()), 73_050_000);
    }

    #[test]
    fn a_server_that_hangs_up_is_reported_and_not_spun_on() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut greeting = [0u8; 12];
            greeting[..4].copy_from_slice(b"RTL0");
            let _ = stream.write_all(&greeting);
        });
        let mut source = RtlTcpSource::connect(("127.0.0.1", port)).unwrap();
        assert!(matches!(source.read(16), Err(RtlTcpError::Closed)));
    }
}
