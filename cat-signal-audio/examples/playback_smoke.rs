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

//! One second of a 700 Hz tone on a sound-card output.
//!
//! ```text
//! cargo run -p cat-signal-audio --example playback_smoke --features device [-- audio:<name>]
//! ```
//!
//! Prints the negotiated format, feeds the tone through the non-blocking
//! queue the way an application would (small blocks, topping up), waits
//! for it to drain, and prints what the queue saw. Whether anything was
//! *heard* is for the person at the speaker to say.

use std::f64::consts::PI;
use std::time::{Duration, Instant};

use cat_signal_audio::{output_devices, AudioPlayback, PlaybackConfig};

fn main() {
    let list = output_devices();
    println!(
        "outputs: {:?}",
        list.devices.iter().map(|d| &d.spec).collect::<Vec<_>>()
    );
    let spec = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "audio:".to_string());
    let player = match AudioPlayback::open(&spec, PlaybackConfig::default()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("could not open {spec:?}: {e}");
            std::process::exit(1);
        }
    };
    let f = player.format().clone();
    println!("opened {:?}: {:?}", player.label(), f);

    let rate = f64::from(f.sample_rate_hz);
    let total = f.sample_rate_hz as usize; // one second
    let ramp = (0.005 * rate) as usize;
    let tone: Vec<f32> = (0..total)
        .map(|n| {
            let env = (n.min(total - 1 - n) as f64 / ramp as f64).min(1.0);
            (0.3 * env * (2.0 * PI * 700.0 * n as f64 / rate).sin()) as f32
        })
        .collect();

    let started = Instant::now();
    let mut sent = 0;
    while sent < tone.len() {
        let end = (sent + 960).min(tone.len());
        match player.write(&tone[sent..end]) {
            Ok(n) => sent += n,
            Err(e) => {
                eprintln!("write failed: {e}");
                std::process::exit(1);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while player.queued() > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(200));
    println!(
        "wrote {sent} samples in {:.2} s; queued now {}; underruns {}",
        started.elapsed().as_secs_f64(),
        player.queued(),
        player.underruns()
    );
    if player.queued() > 0 {
        eprintln!("the device did not drain the queue: is it really playing?");
        std::process::exit(2);
    }
}
