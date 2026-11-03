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
//! On a microcontroller whose `f32` `max` and `min` are instructions, such
//! as thumbv8m.main with its FPU (Cortex-M33: `VMAXNM`, `VMINNM`), build
//! with `--cfg cat_ft8_minmaxnm`: the subtraction then clamps its input
//! without a branch, the form measured on the Cortex-M33. Stable Rust
//! cannot tell such a target from one where they are library calls
//! (thumbv7em), so the crate does not set it. Scoped to the target, in the
//! application's `.cargo/config.toml`:
//!
//! ```toml
//! [target.thumbv8m.main-none-eabihf]
//! rustflags = ["--cfg", "cat_ft8_minmaxnm"]
//! ```
//!
//! Without it the branch form is used there: the results are the same, bit
//! for bit; its speed on the Cortex-M33 was not measured. A `RUSTFLAGS`
//! environment variable replaces the rustflags of the configuration
//! files, so a build that sets it must repeat the cfg. x86-64 and AArch64
//! take the branch-free form without it.
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
//! The decoder uses `f32` `+ − × ÷` and comparisons, integer arithmetic,
//! in-crate polynomial helpers and static tables, each in a fixed order.
//! For the same input samples (bit patterns), configuration and starting
//! call-table state, its output — order, payloads, text, frequency, time
//! offset, SNR and report — is identical for any feed block sizes and any
//! thread count on the split API, on every platform with IEEE-754
//! binary32 round-to-nearest arithmetic **and without flush-to-zero or
//! denormals-are-zero** (subnormal values can reach the FFT before the
//! logarithm's floor; a target that flushes them may differ). Tests pin
//! hashes of the decode output for seeded synthetic slots.
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
//! `cat-ft8-decode FILE.wav [--config host|small[,passes=N][,osd=C:T|off]] [--raw] [--profile]`
//! reads a 12 kHz mono 16-bit PCM WAV file. Its first 15 s are the slot: a
//! shorter file is decoded as a partial slot, and samples after 15 s are
//! ignored (and not read). Other sample rates and channel counts are
//! errors (exit status 1; usage errors exit with 2; `--help` prints the
//! usage on standard output and exits with 0). `--config host` (the
//! default) is `decode::DecodeConfig::HOST` over 100–3100 Hz with 128
//! decodes (three passes, OSD-1 at 10:21); `--config small` is `SMALL`
//! over 200–2900 Hz with 64 decodes (one pass, no OSD-1).
//! `,passes=N` (1–3) sets the configuration's pass count: after each pass
//! but the last, its decodes are subtracted from the audio and the
//! residual is decoded again (`decode::Decoder::decode_passes`).
//! `,osd=C:T` sets the ordered-statistics decoding
//! (`decode::DecodeConfig::with_osd` with `decode::OsdConfig::new(C, T)`;
//! `osd=off` or `osd=0:T` turns it off); the output format is the same.
//!
//! Output is one JSON object per line, one per distinct decode whose
//! payload unpacked, in the order found: pass by pass, and in candidate
//! order within a pass:
//!
//! ```text
//! {"freq": 1000.1, "dt": 0.00, "snr": -9, "message": "CQ K1ABC FN42", "pass": 1, "candidate": 0, "hard_errors": 0, "rounds": 0, "score": 9732}
//! ```
//!
//! - `freq`: tone 0 frequency, Hz, one decimal;
//! - `dt`: start relative to the nominal 0.5 s, seconds, two decimals;
//! - `snr`: signal-to-noise ratio in 2500 Hz against the local noise,
//!   dB, rounded to an integer;
//! - `message`: the text (hashed calls the file did not name in full
//!   appear as `<...>`);
//! - `pass`: the pass that found it, from 1 (always 1 with one pass);
//! - `candidate`, `hard_errors`, `rounds`, `score`: the fields of
//!   `decode::Decode` of the same names (`candidate` is the index in its
//!   own pass's search, so `pass` and `candidate` together identify it).
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
//! `decode::DecodeReport` count under its field name (the slot's report,
//! `decode::PassesReport::report`: the search, list and rejection counts
//! are totals over every pass, the input counters those of the slot's
//! audio; stage times are summed over passes too). With more than one
//! pass configured it also has `subtract_seconds` (in `total`),
//! `residual_bytes` (in `peak_bytes`; the subtraction's working storage is
//! the spectrogram's buffer, idle between passes, so it adds nothing),
//! `passes`, `passes_run`,
//! `passes_end` (why no further pass ran: `"all_passes"`,
//! `"nothing_new"`, `"list_full"`, `"subtraction_cap"` or `"stopped"`),
//! and per pass N `passN_feed_seconds`,
//! `passN_sync_seconds`, `passN_candidates_seconds`,
//! `passN_subtract_seconds` (N ≥ 2) and the pass's own counts
//! `passN_<count>` (`subtracted`, the search and list counts of
//! `decode::PassReport`, `earlier_pass_duplicates`, `bp_rounds`).
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
//! WSJT-X and are used here as facts. For the decoder, `ft8_lib`'s
//! spectrogram (2× oversampling in time and frequency), sync scoring,
//! soft-bit normalisation and belief-propagation decoder were consulted;
//! the decoder was written from the QEX paper and the literature cited in
//! the ADR, and no `ft8_lib` function was translated. Signal subtraction
//! was written from the QEX paper's description and the waveform
//! definition, without reference to `ft8_lib`'s or WSJT-X's source. See
//! the crate's `NOTICE` file.

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
        "DEGREES",
        "VAR_EDGES",
        "EDGE_VARS",
        "LEVEL_POWER",
        "OSD_ROWS",
    ];
    const NOT_COUNTED: &[&str] = &[
        "GENERATOR", // compile-time input to GENERATOR_ROWS only
        // Compile-time input to the decoder's DEGREES and VAR_EDGES;
        // linked only if Codeword::satisfies_parity_checks is used.
        "PARITY_CHECKS",
        "COSTAS_POSITIONS", // frame-layout constants, folded into code
        "DATA_BLOCKS",
        "STAGES", // FFT stage radices, folded into code
        "ONES",   // Gray bit masks for soft bits, folded into code
        "ALL",    // the decoder's profiling stages, folded into code
        "STEPS",  // the decode command's profile names (not in the library)
        // The contents of DIGIT_REV as a constant, for compile-time checks.
        "DIGIT_REV_TABLE",
        "LAST_BOUNDS", // the FFT's last-stage segments, folded into code
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
                        if rest.starts_with("fn ") {
                            continue;
                        }
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
            + size_of_val(&crate::decode::bp::DEGREES)
            + size_of_val(&crate::decode::bp::VAR_EDGES)
            + size_of_val(&crate::decode::bp::EDGE_VARS)
            + size_of_val(&crate::decode::tables::LEVEL_POWER)
            + size_of_val(&crate::decode::osd::OSD_ROWS);
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
