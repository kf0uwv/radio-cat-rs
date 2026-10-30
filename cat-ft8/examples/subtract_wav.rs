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

//! Decodes one slot, subtracts every decode from it and writes what is
//! left:
//!
//! ```text
//! cargo run --release -p cat-ft8 --example subtract_wav -- IN.wav OUT.wav [--config host|small] [--seg N] [--no-drift]
//! ```
//!
//! IN.wav is 12 kHz mono 16-bit PCM; its first 15 s are the slot (sample 0
//! is the slot's start), as for `cat-ft8-decode`. The single-pass decoder
//! runs once; then each decode, in output order, is subtracted with
//! `decode::subtract`, and the residual (the same length as the input read,
//! at most 15 s) is written to OUT.wav. One JSON object per decode goes to
//! standard output: the decode (`message`, `freq`, `dt`, `snr`) and the
//! subtraction's estimates — `start` (slot sample of the waveform's first
//! sample), `fine_dt` (s, before rounding to a sample), `fine_freq` (Hz,
//! at the middle), `drift` (Hz/s), `samples` changed, `energy` removed,
//! and the suppression estimate `left_db`: the signal estimated to be left,
//! in dB relative to the energy removed. It is the sum of two parts, also
//! given: `residue_db`, what the reference still sees in the residual (the
//! model's misfit: a fade, drift or waveform it did not follow; other
//! signals add to it), and `residue_noise_db`, the error the noise is
//! expected to leave in the amplitude estimate (other signals count as
//! noise there). Both are measured only along the reference waveform (its
//! cosine and sine per segment): a mismatch orthogonal to it is invisible
//! to `left_db`. A signal from another generator whose ramps start a
//! sample or two away from this crate's, for example, leaves a residue
//! that `left_db` does not report, so a comparison with the true slot is
//! the real test.
//! Messages go to standard error.
//!
//! A test of subtraction against signals from another FT8 generator: mix
//! known signals into a slot, run this, and compare OUT.wav with the slot
//! without them.

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::process::ExitCode;

use cat_ft8::decode::{
    subtract, Band, Candidate, CandidateScratch, Decode, DecodeConfig, DecodeList, Decoder,
    FrontEnd, SubtractConfig, CANDIDATES, HOST_DECODES, SMALL_DECODES, SUBTRACT_SCRATCH_BYTES,
};
use cat_ft8::message::CallTable;
use cat_ft8::{wav, SAMPLE_RATE, SLOT_SAMPLES};
use serde_json::json;

const USAGE: &str =
    "usage: subtract_wav IN.wav OUT.wav [--config host|small] [--seg N] [--no-drift]";

struct Args {
    input: String,
    output: String,
    band: Band,
    config: DecodeConfig,
    decodes: usize,
    subtract: SubtractConfig,
}

fn parse() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut files = Vec::new();
    let mut a = Args {
        input: String::new(),
        output: String::new(),
        band: Band::DEFAULT,
        config: DecodeConfig::HOST,
        decodes: HOST_DECODES,
        subtract: SubtractConfig::DEFAULT,
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--config" => match it.next().as_deref() {
                Some("host") => {}
                Some("small") => {
                    a.band = Band::SMALL;
                    a.config = DecodeConfig::SMALL;
                    a.decodes = SMALL_DECODES;
                }
                _ => return Err(format!("--config takes host or small\n{USAGE}")),
            },
            "--seg" => {
                let n = it
                    .next()
                    .and_then(|s| s.parse().ok())
                    .ok_or(format!("--seg takes a number of samples\n{USAGE}"))?;
                a.subtract = a.subtract.with_segment(n);
            }
            "--no-drift" => a.subtract = a.subtract.with_drift(false),
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n{USAGE}")),
            _ => files.push(arg),
        }
    }
    let [input, output]: [String; 2] = files.try_into().map_err(|_| USAGE.to_string())?;
    a.input = input;
    a.output = output;
    Ok(a)
}

fn run(a: &Args) -> Result<(), String> {
    let file = File::open(&a.input).map_err(|e| format!("{}: {e}", a.input))?;
    let w = wav::read_wav_prefix(BufReader::new(file), SLOT_SAMPLES)
        .map_err(|e| format!("{}: {e}", a.input))?;
    if w.sample_rate != SAMPLE_RATE || w.channels != 1 {
        return Err(format!(
            "{}: {} Hz, {} channels; 12 kHz mono is needed",
            a.input, w.sample_rate, w.channels
        ));
    }
    // The decoder's own scaling of 16-bit samples (full scale ±1.0).
    let mut audio: Vec<f32> = w
        .samples
        .iter()
        .map(|&s| f32::from(s) * (1.0 / 32_768.0))
        .collect();

    let mut front = Box::new(FrontEnd::new());
    let mut levels = vec![0u8; a.band.spectrogram_len()];
    let mut sync = vec![0i32; Decoder::sync_scratch_len(a.band)];
    let mut candidates = vec![Candidate::default(); CANDIDATES];
    let mut decoder = Decoder::new(
        &mut front,
        &mut levels,
        &mut sync,
        &mut candidates,
        a.band,
        a.config,
    )
    .map_err(|e| e.to_string())?;
    decoder.feed_i16(&w.samples);
    let mut entries = vec![Decode::EMPTY; a.decodes];
    let mut list = DecodeList::new(&mut entries);
    decoder.decode(&mut Box::new(CandidateScratch::new()), &mut list);
    let mut table: CallTable<64> = CallTable::new();
    let mut decodes: Vec<(Decode, String)> = Vec::new();
    list.finish(&mut table, |d, text| {
        let text = match text {
            Ok(t) => t.to_string(),
            Err(e) => format!("(not unpacked: {e})"),
        };
        decodes.push((*d, text));
    });

    let mut scratch = vec![0u8; SUBTRACT_SCRATCH_BYTES];
    for (d, text) in &decodes {
        let s = subtract(&mut audio, d, 0, &a.subtract, &mut scratch).map_err(|e| e.to_string())?;
        let line = json!({
            "message": text,
            "freq": d.freq_hz,
            "dt": d.dt,
            "snr": d.snr_db,
            "start": s.start,
            "fine_dt": s.dt,
            "fine_freq": s.freq_hz,
            "drift": s.drift_hz_per_s,
            "samples": s.samples,
            "energy": s.energy,
            "left_db": s.left_db(),
            "residue_db": s.residue_db(),
            "residue_noise_db": s.residue_noise_db(),
        });
        println!("{line}");
    }
    let out = File::create(&a.output).map_err(|e| format!("{}: {e}", a.output))?;
    let clipped = wav::write_wav(BufWriter::new(out), &audio, SAMPLE_RATE)
        .map_err(|e| format!("{}: {e}", a.output))?;
    eprintln!(
        "{} decodes subtracted; {} samples written to {}{}",
        decodes.len(),
        audio.len(),
        a.output,
        if clipped > 0 {
            format!(" ({clipped} clipped)")
        } else {
            String::new()
        }
    );
    Ok(())
}

fn main() -> ExitCode {
    match parse() {
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
        Ok(a) => match run(&a) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("subtract_wav: {e}");
                ExitCode::FAILURE
            }
        },
    }
}
