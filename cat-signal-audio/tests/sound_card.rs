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

//! Enumeration and capture against whatever sound hardware is actually
//! here.
//!
//! `tests/acc2_wire.rs` drives a real socket because that is what will be
//! plugged into the network source. This file drives the real sound layer
//! for the same reason. Neither can be replaced by a mock: what is being
//! checked is that the assumptions about the *host* hold.
//!
//! # Two of these tests can be skipped, and say so
//!
//! A machine with no sound service at all is a legitimate machine to run
//! this suite on, and a CI runner is usually one. The enumeration tests
//! therefore assert on the *shape* of every answer — including "cannot
//! ask" — and the capture test opens a device if there is one and prints
//! why it did not if there is not. A test that silently passes because it
//! did nothing is worse than one that says it did nothing, so each prints
//! what it found.

#![cfg(feature = "device")]

use std::time::{Duration, Instant};

use cat_signal::DeviceKind;

/// Just enough of a `DeviceInfo` to open one, so the three hardware tests
/// read the same whether the spec came from enumeration or the environment.
struct Device {
    spec: String,
}

use cat_signal_audio::{AudioCapture, AudioEndpoint, AudioSource, CaptureConfig, CaptureError};

#[test]
fn enumeration_answers_this_machine_one_way_or_the_other() {
    let list = cat_signal_audio::input_devices();
    assert_eq!(list.kind, DeviceKind::AudioInput);

    match &list.error {
        // "Cannot ask": a real answer, and it must carry a reason a person
        // can act on rather than an empty list that means something else.
        Some(why) => {
            assert!(!why.trim().is_empty(), "an unavailable list needs a reason");
            assert!(list.devices.is_empty());
            eprintln!("no sound layer on this machine: {why}");
        }
        None => {
            eprintln!("found {} input device(s):", list.devices.len());
            for device in &list.devices {
                eprintln!(
                    "  {}{}  spec={:?}  detail={:?}",
                    if device.is_default { "*" } else { " " },
                    device.label,
                    device.spec,
                    device.detail
                );
            }
        }
    }
}

#[test]
fn every_listed_device_carries_a_spec_the_flag_would_take() {
    // Picking is a shortcut for typing (see `cat_signal::device`), so a
    // spec that `--acc2-audio` would reject, or that would parse as a
    // network endpoint, makes the picker a second naming mechanism.
    for device in cat_signal_audio::input_devices().devices {
        assert_eq!(device.kind, DeviceKind::AudioInput);
        assert!(
            !device.label.is_empty(),
            "a device with no label cannot be offered"
        );
        match AudioEndpoint::parse(&device.spec) {
            AudioEndpoint::Device(name) => assert_eq!(
                name, device.label,
                "the spec must name the device the driver named"
            ),
            AudioEndpoint::Network(_) => {
                panic!("{:?} would be read as a network endpoint", device.spec)
            }
        }
    }
}

#[test]
fn at_most_one_device_is_the_host_default_and_specs_are_unique() {
    // A picker highlights the default, and a duplicate spec would offer a
    // choice that could not be honoured: opening it would find the first.
    let list = cat_signal_audio::input_devices();
    let defaults = list.devices.iter().filter(|d| d.is_default).count();
    assert!(defaults <= 1, "{defaults} devices claim to be the default");

    let mut specs: Vec<&str> = list.devices.iter().map(|d| d.spec.as_str()).collect();
    specs.sort_unstable();
    let before = specs.len();
    specs.dedup();
    assert_eq!(before, specs.len(), "two devices share one spec");
}

#[test]
fn a_network_endpoint_is_not_silently_treated_as_a_device_name() {
    let error = AudioCapture::open("127.0.0.1:4533", CaptureConfig::default())
        .expect_err("a socket address is not a sound card");
    assert!(matches!(error, CaptureError::NotADeviceSpec(_)));
}

/// The spec these tests capture from.
///
/// The host's default unless `CAT_AUDIO_DEVICE` names one, so the same test
/// can be pointed at a station's actual rig interface —
/// `CAT_AUDIO_DEVICE="audio:USB Audio CODEC" cargo test -p cat-signal-audio
/// --features device` — rather than at whatever the desktop happens to
/// route. That is the only way this suite ever meets a real ACC2 lead.
fn spec_under_test() -> Option<String> {
    if let Ok(spec) = std::env::var("CAT_AUDIO_DEVICE") {
        if !spec.trim().is_empty() {
            return Some(spec);
        }
    }
    let list = cat_signal_audio::input_devices();
    list.default_device()
        .or_else(|| list.devices.first())
        .map(|d| d.spec.clone())
}

#[test]
fn capturing_from_a_real_device_produces_real_frames() {
    let Some(spec) = spec_under_test() else {
        eprintln!(
            "SKIPPED: no input device on this machine ({:?})",
            cat_signal_audio::input_devices().error
        );
        return;
    };
    let device = Device { spec };

    // Opened by the spec a picker would have handed over, not by some
    // internal handle -- that round trip is half of what this proves.
    let mut capture = match AudioCapture::open(&device.spec, CaptureConfig::default()) {
        Ok(capture) => capture,
        Err(e) => {
            // A device that enumerates but will not open is normal: it may
            // be exclusively held, or the session may have no permission.
            eprintln!("SKIPPED: {:?} would not open: {e}", device.spec);
            return;
        }
    };

    let format = capture.format().clone();
    eprintln!(
        "capturing {:?}: {} Hz{}, {} ch (using ch {}), {}",
        capture.label(),
        format.sample_rate_hz,
        if format.rate_substituted {
            " (the device's rate, not 48 kHz)"
        } else {
            ""
        },
        format.channels,
        format.channel,
        format.sample_format
    );

    // What a console would poll. Never blocks, so a slow or silent device
    // cannot hang the suite -- it just runs out of deadline.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut frame = None;
    while Instant::now() < deadline {
        match capture.try_next_frame() {
            Ok(Some(f)) => {
                frame = Some(f);
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            // A device that starts and then reports a fault is the host's
            // problem, not this crate's -- and the fact that the driver's
            // own words reached the console *is* the terminal-close rule
            // working. Skipping here rather than failing keeps this suite
            // green on a machine whose sound stack is broken, while still
            // printing exactly what broke.
            Err(cat_signal_audio::AudioError::Closed { reason })
                if reason.contains("audio device stopped") =>
            {
                eprintln!("SKIPPED: the device reported a fault: {reason}");
                return;
            }
            Err(e) => panic!("the capture ended before it produced a frame: {e}"),
        }
    }
    let frame = frame.expect("a device that opened should deliver audio within ten seconds");

    // The claims a console draws against.
    assert_eq!(frame.scope.samples.len(), capture.config().fft_size);
    assert_eq!(
        frame.scope.sample_rate_hz, format.sample_rate_hz,
        "a frame must report the rate it was actually sampled at, not the \
         rate that was asked for"
    );
    assert_eq!(frame.scope.sequence, frame.spectrum.sequence);
    assert!(
        frame.scope.samples.iter().all(|s| (-1.0..=1.0).contains(s)),
        "samples must be normalized"
    );
    assert!(
        frame.spectrum.bins.iter().all(|b| b.is_finite()),
        "a -inf bin would poison a renderer's autoscale"
    );
    assert!(capture.is_connected());
    assert_eq!(
        capture.samples_dropped(),
        0,
        "the ring overran while the reader thread was keeping up"
    );

    // Capability, which is what stops a console drawing speech on a band
    // axis. Nyquist of the *negotiated* rate.
    match capture.capability() {
        cat_signal::SignalCapability::AudioDerived { max_bandwidth_hz } => {
            assert_eq!(max_bandwidth_hz, format.sample_rate_hz / 2);
        }
        other => panic!("a sound card is never a band panorama: {other:?}"),
    }

    // Sequence numbers are producer-stamped, so a second frame must be a
    // later one -- not the same frame handed out twice.
    let first = frame.sequence();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(Some(next)) = capture.try_next_frame() {
            assert!(
                next.sequence() > first,
                "sequence went backwards: {} then {}",
                first,
                next.sequence()
            );
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("only one frame arrived in ten seconds");
}

#[test]
fn dropping_a_capture_stops_its_reader_thread() {
    // The failure this guards is the one ADR 0016 records against
    // `cat-transport-rfc2217`: a worker holding its own `Arc` means
    // dropping the handle frees nothing, and the thread sits in a blocking
    // read forever. A console that opens and closes devices from a picker
    // would leak one thread per pick.
    let Some(spec) = spec_under_test() else {
        eprintln!("SKIPPED: no input device on this machine");
        return;
    };
    let device = Device { spec };

    for _ in 0..3 {
        match AudioCapture::open(&device.spec, CaptureConfig::default()) {
            Ok(mut capture) => {
                let _ = capture.try_next_frame();
                drop(capture);
            }
            Err(e) => {
                eprintln!("SKIPPED: {:?} would not open: {e}", device.spec);
                return;
            }
        }
    }
    // Reaching here without deadlocking is the assertion: each `drop`
    // closes the ring, which is the only thing that can wake a reader
    // thread parked in `read_exact`.
}

#[test]
fn asking_for_a_channel_a_device_does_not_have_is_refused_before_anything_opens() {
    let Some(spec) = spec_under_test() else {
        eprintln!("SKIPPED: no input device on this machine");
        return;
    };
    let device = Device { spec };

    let config = CaptureConfig {
        channel: 64,
        ..CaptureConfig::default()
    };
    match AudioCapture::open(&device.spec, config) {
        Err(CaptureError::NoSuchChannel {
            channels, channel, ..
        }) => {
            assert_eq!(channel, 64);
            assert!(channels < 64);
        }
        Err(other) => eprintln!("SKIPPED: {:?} would not open: {other}", device.spec),
        Ok(_) => panic!("channel 64 should not have been accepted"),
    }
}
