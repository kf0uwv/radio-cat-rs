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

//! A virtual CN4 tap, end to end: synthetic IQ over `rtl_tcp`, through the
//! real pipeline, out as a spectrum.
//!
//! # The test this workspace did not have
//!
//! The inversion correction has always been checked by handing the
//! pipeline a synthetic tone and asserting it comes out the other side.
//! That checks the arithmetic and not the claim. The claim is about
//! **hardware**: a TS-570D's LO1 is high-side, so the tapped spectrum
//! arrives mirrored, and the pipeline exists to undo that.
//!
//! Here the IQ is mirrored on the way in, exactly as the radio mirrors it,
//! and the assertion is that a signal above the dial appears above the
//! centre. Generating un-mirrored IQ and then testing the un-mirroring
//! would be a test of nothing — the two errors cancel and everything
//! passes.
//!
//! Real hardware still has the last word on whether CN4 is wired as
//! assumed. This proves the software path.

use std::io::Write;
use std::net::TcpListener;

use cat_signal::synthetic::{Band, Emission, Emitter};
use cat_signal::{IfTapConfig, SpectrumSource};
use cat_signal_rtlsdr::{RtlSdrSource, RtlTcpSource};

const DIAL_HZ: u64 = 14_074_000;
const RATE_HZ: u32 = 96_000;
const FFT: usize = 2048;

/// Serve `band` as an rtl_tcp dongle would, mirrored per `inverted`.
fn serve(band: Band, inverted: bool) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut greeting = [0u8; 12];
        greeting[..4].copy_from_slice(b"RTL0");
        if stream.write_all(&greeting).is_err() {
            return;
        }
        let mut t = 0.0f64;
        loop {
            let bytes = band.iq_bytes(DIAL_HZ, RATE_HZ, FFT, t, inverted);
            if stream.write_all(&bytes).is_err() {
                return;
            }
            t += FFT as f64 / f64::from(RATE_HZ);
        }
    });
    port
}

fn source(port: u16, inverted: bool) -> RtlSdrSource<RtlTcpSource> {
    let iq = RtlTcpSource::connect(("127.0.0.1", port)).expect("connect to the virtual dongle");
    let mut source = RtlSdrSource::new(
        iq,
        RATE_HZ,
        FFT,
        IfTapConfig {
            if_center_hz: 73_050_000,
            inverted,
            trim_hz: 0,
        },
    );
    source.retune(DIAL_HZ);
    source
}

fn frame(source: &mut RtlSdrSource<RtlTcpSource>) -> cat_signal::SpectrumFrame {
    futures::executor::block_on(source.next_frame()).expect("a frame")
}

/// The bin holding the strongest signal.
fn peak_bin(frame: &cat_signal::SpectrumFrame) -> usize {
    frame
        .bins
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap()
        .0
}

/// One carrier, 20 kHz above the dial.
fn one_carrier_above() -> Band {
    Band::empty(-110.0, 3).with(Emitter::new(DIAL_HZ + 20_000, Emission::Cw, -50.0))
}

#[test]
fn a_signal_above_the_dial_appears_above_the_centre() {
    // The claim, tested against IQ that is mirrored the way the hardware
    // mirrors it.
    let port = serve(one_carrier_above(), true);
    let mut source = source(port, true);
    let f = frame(&mut source);
    let peak = peak_bin(&f);
    assert!(
        peak > FFT / 2,
        "a carrier 20 kHz above the dial landed at bin {peak} of {FFT} -- below centre"
    );
}

#[test]
fn the_correction_is_load_bearing_and_not_decoration() {
    // The same mirrored IQ, with the correction switched off, must put the
    // carrier on the WRONG side. If this passes with the peak still above
    // centre then `inverted` is doing nothing and the test above proves
    // nothing either.
    let port = serve(one_carrier_above(), true);
    let mut source = source(port, false);
    let f = frame(&mut source);
    let peak = peak_bin(&f);
    assert!(
        peak < FFT / 2,
        "with the correction off, the carrier should be mirrored to bin < {} but was at {peak}",
        FFT / 2
    );
}

#[test]
fn the_peak_lands_where_the_carrier_actually_is() {
    // Not merely on the right side: at the right frequency. 20 kHz of a
    // 96 kHz span is 20.8% above centre.
    let port = serve(one_carrier_above(), true);
    let mut source = source(port, true);
    let f = frame(&mut source);
    let hz = f.bin_frequency_hz(peak_bin(&f)).unwrap();
    let error = (hz - (DIAL_HZ + 20_000) as f64).abs();
    assert!(
        error < 500.0,
        "carrier reported at {hz:.0} Hz, expected {} (out by {error:.0} Hz)",
        DIAL_HZ + 20_000
    );
}

#[test]
fn a_frame_is_centred_on_the_dial_and_spans_the_sample_rate() {
    // What makes the console's click-to-tune arithmetic correct. The IF
    // never appears -- a consumer must not learn this radio has one.
    let port = serve(one_carrier_above(), true);
    let mut source = source(port, true);
    let f = frame(&mut source);
    assert_eq!(f.center_hz, DIAL_HZ);
    assert_eq!(f.span_hz, RATE_HZ);
    assert_eq!(f.bins.len(), FFT);
    let (low, high) = f.range_hz();
    assert!(low < DIAL_HZ as f64 && (DIAL_HZ as f64) < high);
}

#[test]
fn trim_moves_the_whole_picture_and_nothing_else() {
    // The WWV calibration. A station's crystal error is a fixed Hz offset
    // because the SDR never retunes, so it shifts the axis rather than
    // scaling it.
    let port = serve(one_carrier_above(), true);
    let iq = RtlTcpSource::connect(("127.0.0.1", port)).unwrap();
    let mut source = RtlSdrSource::new(
        iq,
        RATE_HZ,
        FFT,
        IfTapConfig {
            if_center_hz: 73_050_000,
            inverted: true,
            trim_hz: 1_200,
        },
    );
    source.retune(DIAL_HZ);
    let f = frame(&mut source);
    assert_eq!(f.center_hz, DIAL_HZ + 1_200);
}

#[test]
fn a_busy_band_still_resolves_separate_signals() {
    // A carrier either side of the dial, so a mirrored axis cannot pass by
    // symmetry the way a single centred signal would.
    let band = Band::empty(-110.0, 11)
        .with(Emitter::new(DIAL_HZ - 30_000, Emission::Cw, -55.0))
        .with(Emitter::new(DIAL_HZ + 10_000, Emission::Cw, -45.0));
    let port = serve(band, true);
    let mut source = source(port, true);
    let f = frame(&mut source);

    let mid = FFT / 2;
    let below = f.bins[..mid]
        .iter()
        .cloned()
        .fold(f32::NEG_INFINITY, f32::max);
    let above = f.bins[mid..]
        .iter()
        .cloned()
        .fold(f32::NEG_INFINITY, f32::max);
    // The stronger one is above the dial, so the upper half must win.
    assert!(
        above > below,
        "the louder carrier is above the dial; got {above} above vs {below} below"
    );
}
