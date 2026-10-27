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

//! FT8, implemented from the published protocol description: S. Franke,
//! B. Somerville and J. Taylor, "The FT4 and FT8 Communication Protocols",
//! QEX, July/August 2020.
//!
//! See `docs/adr/0024-cat-ft8.md`.
//!
//! This release is the encoder, message unpacking, and a single-pass
//! decoder ([`decode`]; stages listed in its module docs).
//!
//! | Stage | Module | Does |
//! |-------|--------|------|
//! | 1 | [`message`] | text ↔ 77-bit payload (`i3.n3` message types), callsign hashes |
//! | 2 | [`crc`] | CRC-14 → 91 bits |
//! | 3 | [`ldpc`] | (174,91) LDPC parity → 174-bit codeword |
//! | 4 | [`symbols`] | Gray-coded 8-FSK tones and three 7×7 Costas arrays → 79 symbols |
//! | 5 | [`waveform`] | continuous-phase GFSK (BT = 2) into a caller's `f32` buffer at 12 kHz |
//! | – | [`decode`] | 12 kHz audio → spectrogram → sync candidates → soft bits → belief propagation → payloads, with frequency, time offset and SNR |
//!
//! # `no_std` core
//!
//! The crate is `#![no_std]` in every build. The core allocates nothing
//! (fixed-size arrays and caller-provided buffers), starts no threads, uses
//! no `f64`, and has no dependencies, so it runs on microcontrollers as
//! well as hosts. All floating-point arithmetic is `f32` `+ − × ÷` and
//! in-crate polynomial approximations (`log2`, `exp2`); the pulse shape,
//! sine, FFT twiddles, window and level-to-power table are static tables.
//! The `std` feature (default) adds WAV input and output (module `wav`),
//! the `cat-ft8-encode` and `cat-ft8-decode` binaries and
//! `std::error::Error`. The `profile` feature builds the measurement
//! examples and makes the decoder's stages public for them; it does not
//! change results.
//!
//! # Determinism
//!
//! Packing, CRC, LDPC and tone mapping are integer-only. The waveform uses
//! only `f32` `+ − ×` (Rust never fuses them into FMA), static tables and
//! integer phase. Given IEEE-754 binary32 arithmetic with round-to-nearest
//! — every target with SSE2 or a hardware single-precision FPU, and
//! soft-float targets, but not x87-only targets such as i586 — the same
//! symbols, frequency, amplitude and position give bit-identical samples on
//! every platform. Tests pin hashes of synthesized audio.
//!
//! # Example
//!
//! ```
//! use cat_ft8::{encode, message::NoHashes, waveform::{synthesize, Placement}};
//!
//! let enc = encode("CQ K1ABC FN42", &mut NoHashes).unwrap();
//! assert_eq!(&enc.symbols[..7], &[3, 1, 4, 0, 6, 5, 2]);
//!
//! // A whole 15 s slot (on a small target, render in blocks with
//! // `waveform::Synth` instead of holding the slot in memory).
//! let mut slot = vec![0.0f32; cat_ft8::SLOT_SAMPLES];
//! synthesize(&enc.symbols, &Placement::slot(1500.0, 0.0).unwrap(), &mut slot).unwrap();
//! ```
//!
//! # Decode command output (format version 0)
//!
//! `cat-ft8-decode FILE.wav [--config host|small] [--raw] [--profile]`
//! reads a 12 kHz mono 16-bit PCM WAV file. Its first 15 s are the slot: a
//! shorter file is decoded as a partial slot, and samples after 15 s are
//! ignored (and not read). Other sample rates and channel counts are
//! errors (exit status 1; usage errors exit with 2; `--help` prints the
//! usage on standard output and exits with 0). `--config host` (the
//! default) is `decode::DecodeConfig::HOST` over 100–3100 Hz with 128
//! decodes; `--config small` is `SMALL` over 200–2900 Hz with 64 decodes.
//!
//! Output is one JSON object per line, in candidate order, one per
//! distinct decode whose payload unpacked:
//!
//! ```text
//! {"freq": 1000.1, "dt": 0.00, "snr": -9, "message": "CQ K1ABC FN42", "candidate": 0, "hard_errors": 0, "rounds": 0, "score": 9732}
//! ```
//!
//! - `freq`: tone 0 frequency, Hz, one decimal;
//! - `dt`: start relative to the nominal 0.5 s, seconds, two decimals;
//! - `snr`: signal-to-noise ratio in 2500 Hz against the local noise,
//!   dB, rounded to an integer;
//! - `message`: the text (hashed calls the file did not name in full
//!   appear as `<...>`);
//! - `candidate`, `hard_errors`, `rounds`, `score`: the fields of
//!   `decode::Decode` of the same names.
//!
//! `--raw` adds a line for each decode whose payload did not unpack, with
//! `"payload"` (the 77 bits as 10 bytes, most significant first, in hex),
//! `"type"` (`"i3"`, or `"0.n3"` when i3 is 0), `"unpacked": false` and
//! `"error"` in place of `"message"`.
//!
//! `--profile` adds one last line, `{"profile": {...}}`, with `"format": 0`,
//! `"config"`, `"band_lo_hz"`, `"band_hi_hz"`, `"decode_capacity"`, stage
//! times in keys ending `_seconds` (`read`, `feed`, `sync`, `candidates` —
//! split into `refine`, `soft_bits`, `belief_propagation` and `snr` —
//! `finish`, and `total` from the first feed to the end of `finish`,
//! single-threaded), memory in keys ending `_bytes` (each
//! buffer, `peak_bytes` their sum, `static_table_bytes`), and every
//! `decode::DecodeReport` count under its field name.
//!
//! The format is version 0 and may change until a release; a change will
//! raise the version reported in the profile line.
//!
//! # Provenance
//!
//! Implemented from the QEX 2020 paper. WSJT-X source code (GPLv3) is not
//! used; WSJT-X's `ft8sim` and `jt9` serve only as black-box test oracles
//! (`tests/data/`). `ft8_lib` (MIT) was consulted (its CRC, message
//! packing, encoder and waveform code); the LDPC generator and parity-check
//! tables, the callsign-hash multiplier and the callsign alphabet order were
//! taken from it, and the waveform's first/last-tone edge extension
//! follows it. The tables are interoperability constants that originate in
//! WSJT-X and are used here as facts. See the crate's `NOTICE` file.

#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "std")]
extern crate std;

pub mod crc;
pub mod decode;
mod error;
pub mod ldpc;
mod ldpc_tables;
pub mod message;
pub mod symbols;
#[cfg(feature = "std")]
pub mod wav;
pub mod waveform;

pub use error::Error;
pub use ldpc::Codeword;
pub use message::{pack, unpack, CallHashes, MessageText, MessageType, Payload};

/// Audio sample rate of the synthesized waveform, Hz (the WSJT-X slot format).
pub const SAMPLE_RATE: u32 = 12_000;
/// Samples per channel symbol at [`SAMPLE_RATE`] (0.160 s).
pub const SAMPLES_PER_SYMBOL: usize = 1_920;
/// Channel symbols per transmission (3 × 7 Costas + 58 data).
pub const SYMBOL_COUNT: usize = 79;
/// Tone spacing and keying rate, Hz (h = 1, T = 0.160 s).
pub const TONE_SPACING_HZ: f32 = 6.25;
/// Samples in one waveform: 79 × 1920 = 151 680 (12.64 s).
pub const WAVEFORM_SAMPLES: usize = SYMBOL_COUNT * SAMPLES_PER_SYMBOL;
/// Samples in one 15 s slot at [`SAMPLE_RATE`].
pub const SLOT_SAMPLES: usize = 15 * SAMPLE_RATE as usize;
/// Nominal start of a transmission after the slot boundary, seconds.
pub const NOMINAL_START_S: f32 = 0.5;

/// Bytes of lookup tables that encoding, unpacking and synthesis link into
/// static data: the CRC byte table, the LDPC generator row masks, the two
/// character sets, the Costas array, the Gray maps, and the waveform's
/// pulse and sine tables.
/// Not counted: the generator's byte form (used only at compile time) and
/// the parity-check table (linked only if
/// `Codeword::satisfies_parity_checks` is called). A test ties every term
/// to the size of the actual item.
pub const STATIC_TABLE_BYTES: usize = core::mem::size_of::<[u16; 256]>()
    + core::mem::size_of::<[u128; ldpc::M]>()
    + core::mem::size_of::<[u8; 42]>()
    + core::mem::size_of::<[u8; 38]>()
    + core::mem::size_of::<[u8; 7]>()
    + 2 * core::mem::size_of::<[u8; 8]>()
    + waveform::TABLE_BYTES;

#[cfg(test)]
mod tests {
    extern crate std;

    use core::mem::size_of_val;
    use std::vec::Vec;

    /// Tables counted in `STATIC_TABLE_BYTES` (encoder) or in
    /// `decode::TABLE_BYTES` (decoder), and arrays deliberately not
    /// counted (with why). Every array `static`/`const` in `src/` must be
    /// in one list, so a new table cannot go uncounted.
    const COUNTED: &[&str] = &[
        "CRC_TABLE",
        "GENERATOR_ROWS",
        "FREE_TEXT",
        "CALL38",
        "COSTAS",
        "GRAY_MAP",
        "GRAY_UNMAP",
        "PULSE_HALF",
        "QUARTER_SINE",
    ];
    const DECODE_COUNTED: &[&str] = &[
        "TWIDDLES",
        "SPLIT_TWIDDLES",
        "HANN_HALF",
        "DIGIT_REV",
        "CHECKS",
        "DEGREES",
        "LEVEL_POWER",
    ];
    const NOT_COUNTED: &[&str] = &[
        "GENERATOR", // compile-time input to GENERATOR_ROWS only
        // Compile-time input to the decoder's CHECKS; linked only if
        // Codeword::satisfies_parity_checks is used.
        "PARITY_CHECKS",
        "COSTAS_POSITIONS", // frame-layout constants, folded into code
        "DATA_BLOCKS",
        "STAGES", // FFT stage radices, folded into code
        "ONES",   // Gray bit masks for soft bits, folded into code
        "ALL",    // the decoder's profiling stages, folded into code
    ];

    #[test]
    fn every_array_item_is_accounted_for() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = std::vec![dir];
        let mut found = Vec::new();
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("tests.rs") {
                    // Skip the body of each `mod tests { ... }` by brace depth.
                    let mut depth: i32 = 0;
                    for line in std::fs::read_to_string(&p).unwrap().lines() {
                        let l = line.trim_start();
                        let braces = l.matches('{').count() as i32 - l.matches('}').count() as i32;
                        if depth > 0 || l.starts_with("mod tests {") {
                            depth += braces;
                            continue;
                        }
                        let l = l
                            .strip_prefix("pub(crate) ")
                            .or(l.strip_prefix("pub "))
                            .unwrap_or(l);
                        let Some(rest) = l.strip_prefix("static ").or(l.strip_prefix("const "))
                        else {
                            continue;
                        };
                        let Some((name, ty)) = rest.split_once(": ") else {
                            continue;
                        };
                        if ty.starts_with('[') || ty.starts_with("&[") {
                            found.push(std::string::String::from(name));
                        }
                    }
                }
            }
        }
        for name in &found {
            assert!(
                COUNTED.contains(&name.as_str())
                    || DECODE_COUNTED.contains(&name.as_str())
                    || NOT_COUNTED.contains(&name.as_str()),
                "array item {name} is not in STATIC_TABLE_BYTES's lists"
            );
        }
        for name in COUNTED.iter().chain(DECODE_COUNTED).chain(NOT_COUNTED) {
            assert!(
                found.iter().any(|f| f == name),
                "{name} listed but not found"
            );
        }
    }

    #[test]
    fn static_table_bytes_matches_the_tables() {
        let parts = size_of_val(&crate::crc::CRC_TABLE)
            + size_of_val(&crate::ldpc::GENERATOR_ROWS)
            + size_of_val(crate::message::FREE_TEXT)
            + size_of_val(crate::message::CALL38)
            + size_of_val(&crate::symbols::COSTAS)
            + size_of_val(&crate::symbols::GRAY_MAP)
            + size_of_val(&crate::symbols::GRAY_UNMAP)
            + crate::waveform::table_bytes_of_statics();
        assert_eq!(super::STATIC_TABLE_BYTES, parts);
    }

    #[test]
    fn decode_table_bytes_matches_the_tables() {
        let parts = crate::decode::fft::table_bytes_of_statics()
            + size_of_val(&crate::decode::bp::CHECKS)
            + size_of_val(&crate::decode::bp::DEGREES)
            + size_of_val(&crate::decode::tables::LEVEL_POWER);
        assert_eq!(crate::decode::TABLE_BYTES, parts);
    }
}

/// A message after every stage up to the channel symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Encoded {
    /// The 77-bit payload.
    pub payload: Payload,
    /// Tone index (0–7) of each of the 79 channel symbols.
    pub symbols: [u8; SYMBOL_COUNT],
}

/// Packs `text` and runs CRC, LDPC and tone mapping.
///
/// Full callsigns seen while packing are offered to `hashes` (see
/// [`CallHashes`]); pass [`message::NoHashes`] when nothing is to be kept.
pub fn encode<H: CallHashes + ?Sized>(text: &str, hashes: &mut H) -> Result<Encoded, Error> {
    let payload = pack(text, hashes)?;
    Ok(Encoded {
        payload,
        symbols: encode_payload(&payload),
    })
}

/// CRC, LDPC and tone mapping for an already packed payload.
pub fn encode_payload(payload: &Payload) -> [u8; SYMBOL_COUNT] {
    let a91 = crc::append_crc(payload);
    let codeword = ldpc::encode(a91);
    symbols::map(&codeword)
}
