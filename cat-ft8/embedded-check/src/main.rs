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

//! Calls the core with no allocator and no `std`: `encode` (pack, CRC,
//! LDPC, tones), `unpack`, and every public floating-point entry point: `Placement::slot`, `synthesize` (at a negative start, into a small
//! buffer), `Synth::new`, `fill`, `mix`, `skip` and `fill_phase`, plus
//! `sin_turns`/`cos_turns`; and the decoder (`Decoder::new`, `feed`,
//! `feed_i16`, `skip`, `decode`, `DecodeList`, `finish`) in one
//! configuration, with every buffer in statics: the small configuration by
//! default, the host configuration with the `host` feature, none with
//! `encoder-only` (a baseline for the decoder's size); `check.sh` builds
//! every image. The check-node rule is a run-time field of the
//! configuration, so both rules are linked in either decoder image. Linking at all proves the core
//! needs neither an allocator nor `std`; `check.sh` then rejects any
//! soft-double helper (`__aeabi_d*`, `__*df*`) in the image. Only code
//! reachable from here is linked, so a new floating-point entry point must
//! be called here too.

#![no_std]
#![no_main]

use core::hint::black_box;
use core::panic::PanicInfo;
#[cfg(not(feature = "encoder-only"))]
use core::ptr::addr_of_mut;

#[cfg(not(feature = "encoder-only"))]
use cat_ft8::decode::{
    Candidate, CandidateScratch, Decode, DecodeList, Decoder, FrontEnd, CANDIDATES,
};
use cat_ft8::encode;
use cat_ft8::message::{unpack, CallTable};
use cat_ft8::waveform::{cos_turns, sin_turns, synthesize, Placement, Synth};

// The configuration of this image.
#[cfg(not(any(feature = "host", feature = "encoder-only")))]
mod setup {
    use cat_ft8::decode::{Band, DecodeConfig, SMALL_DECODES};
    /// 200–2900 Hz: the band sized for this class of part.
    pub const BAND: Band = Band::SMALL;
    pub const CONFIG: DecodeConfig = DecodeConfig::SMALL;
    pub const DECODES: usize = SMALL_DECODES;
}
#[cfg(all(feature = "host", not(feature = "encoder-only")))]
mod setup {
    use cat_ft8::decode::{Band, DecodeConfig, HOST_DECODES};
    /// 100–3100 Hz.
    pub const BAND: Band = Band::DEFAULT;
    pub const CONFIG: DecodeConfig = DecodeConfig::HOST;
    pub const DECODES: usize = HOST_DECODES;
}

// Decoder buffers live in statics (`.bss`), as on a real target.
#[cfg(not(feature = "encoder-only"))]
mod buffers {
    use super::setup::{BAND, DECODES};
    use super::*;
    pub static mut FRONT: FrontEnd = FrontEnd::new();
    pub static mut LEVELS: [u8; BAND.spectrogram_len()] = [0; BAND.spectrogram_len()];
    pub static mut SYNC_SCRATCH: [i32; Decoder::sync_scratch_len(BAND)] =
        [0; Decoder::sync_scratch_len(BAND)];
    pub static mut CANDIDATE_LIST: [Candidate; CANDIDATES] = [Candidate {
        score: 0,
        start_row: 0,
        base: 0,
    }; CANDIDATES];
    pub static mut SCRATCH: CandidateScratch = CandidateScratch::new();
    pub static mut DECODES_LIST: [Decode; DECODES] = [Decode::EMPTY; DECODES];
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

/// Entry point. Inputs and outputs go through `black_box` so nothing is
/// evaluated at compile time or optimised away.
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut table: CallTable<16> = CallTable::new();
    let mut sink = 0.0f32;
    let mut text_len = 0;
    if let Ok(enc) = encode(black_box("CQ K1ABC FN42"), &mut table) {
        if let Ok(text) = unpack(&enc.payload, &mut table) {
            text_len = text.len();
        }
        let mut block = [0.0f32; 480];
        // Block rendering: the whole waveform through `fill`.
        if let Ok(mut synth) = Synth::new(&enc.symbols, black_box(1500.0), black_box(0.5)) {
            loop {
                let n = synth.fill(&mut block);
                sink += black_box(&block)[0];
                if n < block.len() {
                    break;
                }
            }
        }
        // `skip`, `mix` and `fill_phase` on a second generator.
        if let Ok(mut synth) = Synth::new(&enc.symbols, black_box(812.5), black_box(0.25)) {
            synth.skip(black_box(1_000));
            synth.mix(&mut block);
            let mut phase = [0u32; 64];
            let mut env = [0.0f32; 64];
            synth.fill_phase(&mut phase, &mut env);
            sink += black_box(&block)[1] + env[63] * (sin_turns(phase[63]) + cos_turns(phase[0]));
        }
        // Whole-buffer path: a slot placement from a run-time dt, then
        // `synthesize` at a negative start into a small buffer.
        if let Ok(mut placement) = Placement::slot(black_box(1500.0), black_box(-0.6)) {
            placement.amplitude = black_box(0.5);
            let mut small = [0.0f32; 256];
            if synthesize(&enc.symbols, &placement, &mut small).is_ok() {
                sink += black_box(&small)[255];
            }
        }
    }
    #[cfg_attr(feature = "encoder-only", allow(unused_mut))]
    let mut rows = 0;
    #[cfg(not(feature = "encoder-only"))]
    decode_a_slot(&mut table, &mut sink, &mut rows);
    black_box((sink, text_len, rows));
    loop {
        core::hint::spin_loop();
    }
}

/// The decoder: a synthesized slot fed in blocks (with `skip` for the
/// silence before and after), decoded and finished with this image's
/// configuration.
/// Not inlined, so `check.sh --stack` reports the decoder path's own
/// frame apart from `_start`'s.
#[cfg(not(feature = "encoder-only"))]
#[inline(never)]
fn decode_a_slot(table: &mut CallTable<16>, sink: &mut f32, rows: &mut usize) {
    use buffers::*;
    // SAFETY: `_start` is the only code that runs, it is not re-entered,
    // and it calls this once, so these are the only references to the
    // statics.
    let (front, levels, sync_scratch, cands, scratch, decodes) = unsafe {
        (
            &mut *addr_of_mut!(FRONT),
            &mut *addr_of_mut!(LEVELS),
            &mut *addr_of_mut!(SYNC_SCRATCH),
            &mut *addr_of_mut!(CANDIDATE_LIST),
            &mut *addr_of_mut!(SCRATCH),
            &mut *addr_of_mut!(DECODES_LIST),
        )
    };
    let mut list = DecodeList::new(decodes);
    let Ok(mut dec) = Decoder::new(
        front,
        levels,
        sync_scratch,
        cands,
        setup::BAND,
        setup::CONFIG,
    ) else {
        return;
    };
    dec.skip(black_box(6_000));
    let mut feed_block = [0.0f32; 480];
    if let Ok(mut synth) = Synth::new(&[3; 79], black_box(1500.0), black_box(0.25)) {
        loop {
            let n = synth.fill(&mut feed_block);
            dec.feed(&feed_block[..n]);
            if n < feed_block.len() {
                break;
            }
        }
    }
    dec.feed_i16(black_box(&[0i16; 64]));
    dec.skip(usize::MAX);
    let report = dec.decode(scratch, &mut list);
    *rows += report.rows_filled + report.candidates_kept + report.all_zero_rejected;
    list.finish(table, |d, text| {
        *sink += d.snr_db + d.freq_hz;
        *rows += text.map_or(0, str::len);
    });
}
