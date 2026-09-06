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

//! Does a real dongle produce real spectrum?
//!
//! ```sh
//! cargo run -p cat-signal-rtlsdr --features device --example capture -- [index] [centre_hz]
//! ```
//!
//! The companion to `enumerate`: that one answers "is it plugged in", this
//! one answers "does the whole path work" — device, worker thread,
//! backpressure slot, FFT, and the `IfTapConfig` correction — without a
//! console in the way. When a waterfall looks wrong, this says which side
//! of the console the problem is on.

use cat_signal::{IfTapConfig, SpectrumSource};
use cat_signal_rtlsdr::{device::RtlSdrDevice, RtlSdrSource};

/// The TS-570D's first IF, which is what this crate exists to look at.
const DEFAULT_CENTRE_HZ: u64 = 73_050_000;
const SAMPLE_RATE_HZ: u32 = 240_000;
const FFT: usize = 2048;

fn main() {
    let mut args = std::env::args().skip(1);
    let index: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let centre: u64 = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_CENTRE_HZ);

    println!(
        "opening rtl:{index} at {:.3} MHz, {SAMPLE_RATE_HZ} S/s",
        centre as f64 / 1e6
    );
    let device = match RtlSdrDevice::open(index, centre, SAMPLE_RATE_HZ) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("could not open rtl:{index}: {e}");
            std::process::exit(1);
        }
    };

    // `inverted: false` here, unlike a real CN4 tap: this is a bare dongle
    // on whatever it is connected to, not a TS-570D's high-side-injected
    // first IF, so un-mirroring would be correcting a distortion nothing
    // applied.
    let mut source = RtlSdrSource::new(
        device,
        SAMPLE_RATE_HZ,
        FFT,
        IfTapConfig {
            if_center_hz: centre,
            inverted: false,
            trim_hz: 0,
        },
    );
    source.retune(centre);

    for n in 0..5 {
        match futures::executor::block_on(source.next_frame()) {
            Ok(frame) => {
                let floor = frame.bins.iter().copied().fold(f32::INFINITY, f32::min);
                let peak = frame.bins.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let peak_bin = frame
                    .bins
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let peak_hz = frame.bin_frequency_hz(peak_bin).unwrap_or(0.0);
                println!(
                    "frame {n}: seq {} centre {:.3} MHz span {} kHz, {} bins, \
                     floor {floor:.1} dBm peak {peak:.1} dBm at {:.3} MHz",
                    frame.sequence,
                    frame.center_hz as f64 / 1e6,
                    frame.span_hz / 1000,
                    frame.bins.len(),
                    peak_hz / 1e6,
                );
            }
            Err(e) => {
                eprintln!("frame {n}: {e}");
                std::process::exit(1);
            }
        }
    }
    println!("\nthe device path works: frames arrived, corrected, low-frequency-first");
}
