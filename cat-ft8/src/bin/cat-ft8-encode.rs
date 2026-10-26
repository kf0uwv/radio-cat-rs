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

//! `cat-ft8-encode "<message>" [--freq 1500] [--dt 0.0] --out slot.wav`
//!
//! Writes one 15 s, 12 kHz, mono 16-bit WAV slot with the message starting
//! at 0.5 s + dt, as an FT8 transmission does, peaking at half full scale.
//! Prints the message type, payload bits and channel symbols on stdout.
//! (The `noisy_slot` example adds white noise at a given SNR.)

use std::fs::File;
use std::io::BufWriter;
use std::process::ExitCode;

use cat_ft8::message::NoHashes;
use cat_ft8::waveform::{synthesize, Placement};
use cat_ft8::{encode, wav, SAMPLE_RATE, SLOT_SAMPLES};

const USAGE: &str = "usage: cat-ft8-encode \"<message>\" [--freq HZ] [--dt S] --out FILE.wav";

struct Args {
    message: String,
    freq: f32,
    dt: f32,
    out: String,
}

fn parse() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut a = Args {
        message: String::new(),
        freq: 1500.0,
        dt: 0.0,
        out: String::new(),
    };
    while let Some(arg) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--freq" => a.freq = val("--freq")?.parse().map_err(|e| format!("--freq: {e}"))?,
            "--dt" => a.dt = val("--dt")?.parse().map_err(|e| format!("--dt: {e}"))?,
            "--out" => a.out = val("--out")?,
            "-h" | "--help" => return Err(USAGE.into()),
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n{USAGE}")),
            s if a.message.is_empty() => a.message = s.to_string(),
            s => return Err(format!("unexpected argument {s}\n{USAGE}")),
        }
    }
    if a.message.is_empty() || a.out.is_empty() {
        return Err(USAGE.into());
    }
    Ok(a)
}

fn run(a: &Args) -> Result<(), String> {
    let enc = encode(&a.message, &mut NoHashes).map_err(|e| format!("{}: {e}", a.message))?;
    let mut placement = Placement::slot(a.freq, a.dt).map_err(|e| e.to_string())?;
    placement.amplitude = 0.5;
    let mut slot = vec![0.0f32; SLOT_SAMPLES];
    synthesize(&enc.symbols, &placement, &mut slot).map_err(|e| e.to_string())?;

    let file = File::create(&a.out).map_err(|e| format!("{}: {e}", a.out))?;
    wav::write_wav(BufWriter::new(file), &slot, SAMPLE_RATE)
        .map_err(|e| format!("{}: {e}", a.out))?;

    let bits: String = (0..77)
        .map(|i| char::from(b'0' + enc.payload.bit(i)))
        .collect();
    let syms: String = enc.symbols.iter().map(|&s| char::from(b'0' + s)).collect();
    println!("type {}", enc.payload.message_type());
    println!("bits77 {bits}");
    println!("symbols {syms}");
    Ok(())
}

fn main() -> ExitCode {
    match parse().and_then(|a| run(&a)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
