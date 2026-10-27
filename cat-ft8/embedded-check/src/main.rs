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
//! `feed_i16`, `skip`, `decode` with each check-node rule, `DecodeList`,
//! `finish`), with every buffer in statics. Linking at all proves the core needs neither an
//! allocator nor `std`; `check.sh` then rejects any soft-double helper
//! (`__aeabi_d*`, `__*df*`) in the image. Only code reachable from here is
//! linked, so a new floating-point entry point must be called here too.

#![no_std]
#![no_main]

use core::hint::black_box;
use core::panic::PanicInfo;
use core::ptr::addr_of_mut;

use cat_ft8::decode::{
    Band, Candidate, CandidateScratch, Decode, DecodeConfig, DecodeList, Decoder, FrontEnd,
    CANDIDATES, SMALL_DECODES,
};
use cat_ft8::encode;
use cat_ft8::message::{unpack, CallTable};
use cat_ft8::waveform::{cos_turns, sin_turns, synthesize, Placement, Synth};

// Decoder buffers live in statics (`.bss`), as on a real target. The
// small configuration (200–2900 Hz, 8-bit levels) is the one sized for
// this class of part.
static mut FRONT: FrontEnd = FrontEnd::new();
static mut LEVELS: [u8; Band::SMALL.spectrogram_len()] = [0; Band::SMALL.spectrogram_len()];
static mut SYNC_SCRATCH: [i32; Decoder::sync_scratch_len(Band::SMALL)] =
    [0; Decoder::sync_scratch_len(Band::SMALL)];
static mut CANDIDATE_LIST: [Candidate; CANDIDATES] = [Candidate {
    score: 0,
    start_row: 0,
    base: 0,
}; CANDIDATES];
static mut SCRATCH: CandidateScratch = CandidateScratch::new();
static mut DECODES: [Decode; SMALL_DECODES] = [Decode::EMPTY; SMALL_DECODES];

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
    // The decoder: a synthesized slot fed in blocks (with `skip` for the
    // silence before and after), decoded with each check-node rule.
    // SAFETY: `_start` is the only code that runs and it is not
    // re-entered, so these are the only references to the statics.
    let (front, levels, sync_scratch, cands, scratch, decodes) = unsafe {
        (
            &mut *addr_of_mut!(FRONT),
            &mut *addr_of_mut!(LEVELS),
            &mut *addr_of_mut!(SYNC_SCRATCH),
            &mut *addr_of_mut!(CANDIDATE_LIST),
            &mut *addr_of_mut!(SCRATCH),
            &mut *addr_of_mut!(DECODES),
        )
    };
    let mut rows = 0;
    let mut list = DecodeList::new(decodes);
    for config in [DecodeConfig::SMALL, DecodeConfig::HOST] {
        let Ok(mut dec) = Decoder::new(
            &mut *front,
            &mut *levels,
            &mut *sync_scratch,
            &mut *cands,
            Band::SMALL,
            black_box(config),
        ) else {
            continue;
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
        rows += report.rows_filled + report.candidates_kept + report.all_zero_rejected;
        list.finish(&mut table, |d, text| {
            sink += d.snr_db + d.freq_hz;
            rows += text.map_or(0, str::len);
        });
    }

    black_box((sink, text_len, rows));
    loop {
        core::hint::spin_loop();
    }
}
