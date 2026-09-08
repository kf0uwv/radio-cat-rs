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

//! Carrying a radio's receive audio to a console somewhere else.
//!
//! The invariant worth defending here is the one `AudioFrame` exists for:
//! the scope trace and the AF spectrum come from the *same samples* and
//! share a sequence number. A wire format that let them drift would let a
//! console draw a waveform and a spectrum that disagree about what the
//! radio was doing, which looks like a working display and is not one.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cat_native::testing::{StubHost, STUB_RADIO};
use cat_native::{
    decode_audio_payload, encode_audio_payload, Connection, RadioHost, RadioState, Streams,
};
use cat_signal::{AudioFrame, AudioScopeFrame, AudioSpectrumFrame};

fn frame(sequence: u64) -> AudioFrame {
    AudioFrame {
        scope: AudioScopeFrame {
            sample_rate_hz: 48_000,
            // Asymmetric on purpose: a scope trace reversed or halved by a
            // codec bug would still look like a waveform, and a symmetric
            // fixture would hide it.
            samples: (0..64).map(|i| (i as f32) / 64.0 - 0.5).collect(),
            sequence,
        },
        spectrum: AudioSpectrumFrame {
            start_hz: 0,
            span_hz: 4_000,
            bins: (0..85).map(|i| -100.0 + i as f32).collect(),
            sequence,
        },
    }
}

/// A radio whose audio can be changed from a test.
struct Host {
    audio: Mutex<Option<AudioFrame>>,
}

impl Host {
    fn new(audio: Option<AudioFrame>) -> Arc<Self> {
        Arc::new(Self {
            audio: Mutex::new(audio),
        })
    }
}

impl RadioHost for Host {
    fn capabilities(&self) -> &'static cat_framework::capabilities::RadioCapabilities {
        &STUB_RADIO
    }

    fn state(&self) -> RadioState {
        StubHost::new().state()
    }

    fn audio(&self) -> Option<AudioFrame> {
        self.audio.lock().unwrap().clone()
    }

    fn apply(&self, _command: &cat_native::Command) -> Result<(), String> {
        Ok(())
    }
}

fn serve_audio(host: Arc<Host>) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        let _ = cat_native::serve(listener, host);
    });
    addr
}

/// Poll until an audio frame arrives, or give up.
fn wait_for_audio(conn: &mut Connection) -> Option<AudioFrame> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let _ = conn.poll(Some(Duration::from_millis(50)));
        if let Some(frame) = conn.take_audio() {
            return Some(frame);
        }
    }
    None
}

#[test]
fn both_halves_survive_the_wire_unchanged() {
    let original = frame(11);
    let back = decode_audio_payload(&encode_audio_payload(&original))
        .expect("what we encoded must decode");
    assert_eq!(back, original);
}

#[test]
fn the_two_halves_keep_the_sequence_that_ties_them_together() {
    // The whole reason this is one payload. A console checks the trace
    // against the spectrum by this number; two frame kinds could not
    // promise it.
    let back = decode_audio_payload(&encode_audio_payload(&frame(4242))).unwrap();
    assert_eq!(back.scope.sequence, back.spectrum.sequence);
    assert_eq!(back.sequence(), 4242);
}

#[test]
fn a_truncated_payload_is_refused_rather_than_half_drawn() {
    // Half a scope is a waveform the radio never produced. Better to draw
    // nothing than something plausible and false.
    let full = encode_audio_payload(&frame(1));
    for cut in [0, 10, 27, full.len() / 2, full.len() - 1] {
        assert!(
            decode_audio_payload(&full[..cut]).is_none(),
            "{cut} bytes decoded into a frame"
        );
    }
}

#[test]
fn a_console_that_asked_for_audio_receives_it() {
    let host = Host::new(Some(frame(1)));
    let mut conn = Connection::connect(
        serve_audio(host).as_str(),
        Streams {
            spectrum: false,
            audio: true,
            max_fps: None,
        },
    )
    .unwrap();

    let got = wait_for_audio(&mut conn).expect("no audio arrived");
    assert_eq!(got.scope.samples.len(), 64);
    assert_eq!(got.spectrum.bins.len(), 85);
}

#[test]
fn a_console_that_did_not_ask_is_sent_none() {
    // Not merely ignored on arrival: a console drawing meters over a slow
    // link should not be paying for audio it will throw away.
    let host = Host::new(Some(frame(1)));
    let mut conn = Connection::connect(serve_audio(host).as_str(), Streams::spectrum()).unwrap();

    let deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < deadline {
        let _ = conn.poll(Some(Duration::from_millis(50)));
        assert!(
            conn.take_audio().is_none(),
            "audio was sent to a client that declined it"
        );
    }
}

#[test]
fn asking_for_audio_does_not_imply_asking_for_spectrum() {
    // The two opt-ins are independent. A console with AF panels and no
    // waterfall is an ordinary way to run, and it should not be sent
    // 2048 bins thirty times a second to ignore.
    let host = Host::new(Some(frame(1)));
    let mut conn = Connection::connect(
        serve_audio(host).as_str(),
        Streams {
            spectrum: false,
            audio: true,
            max_fps: None,
        },
    )
    .unwrap();

    assert!(wait_for_audio(&mut conn).is_some());
    assert!(
        conn.take_spectrum().is_none(),
        "spectrum arrived for a client that asked only for audio"
    );
}

#[test]
fn a_stalled_capture_is_not_redrawn_as_though_it_were_live() {
    // The server holds one frame forever. Resending it would make a dead
    // capture look like a steady tone -- the one thing a scope must not
    // do, because an operator watches it precisely to see whether audio
    // is moving.
    let host = Host::new(Some(frame(7)));
    let mut conn = Connection::connect(
        serve_audio(Arc::clone(&host)).as_str(),
        Streams {
            spectrum: false,
            audio: true,
            max_fps: None,
        },
    )
    .unwrap();

    assert_eq!(wait_for_audio(&mut conn).unwrap().sequence(), 7);

    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        let _ = conn.poll(Some(Duration::from_millis(50)));
        assert!(conn.take_audio().is_none(), "the same frame was sent twice");
    }

    // A genuinely new block does arrive.
    *host.audio.lock().unwrap() = Some(frame(8));
    assert_eq!(
        wait_for_audio(&mut conn).unwrap().sequence(),
        8,
        "a new frame never arrived, so the don't-resend rule is blocking real audio"
    );
}

#[test]
fn a_host_with_no_audio_is_simply_quiet() {
    // A bench with an SDR on the IF tap and nothing on the audio pair is
    // an ordinary bench, not a fault.
    let host = Host::new(None);
    let mut conn = Connection::connect(
        serve_audio(host).as_str(),
        Streams {
            spectrum: false,
            audio: true,
            max_fps: None,
        },
    )
    .unwrap();

    let deadline = Instant::now() + Duration::from_millis(400);
    while Instant::now() < deadline {
        let _ = conn.poll(Some(Duration::from_millis(50)));
        assert!(conn.take_audio().is_none());
    }
}

#[test]
fn a_client_built_before_audio_existed_asks_for_none() {
    // The `#[serde(default)]` that keeps an old client working. Its
    // `Hello` has no `audio` field at all, and must deserialize to "no"
    // rather than failing the handshake.
    let old: cat_native::ClientMessage =
        serde_json::from_str(r#"{"type":"hello","version":1,"spectrum":true}"#)
            .expect("an old hello must still parse");
    match old {
        cat_native::ClientMessage::Hello {
            spectrum, audio, ..
        } => {
            assert!(spectrum);
            assert!(
                !audio,
                "an old client must not be sent audio it cannot decode"
            );
        }
        other => panic!("parsed as {other:?}"),
    }
}

#[test]
fn every_frame_kind_can_be_read_back_off_the_wire() {
    // The discriminant and the parser are written out separately, and a
    // kind added to one and forgotten in the other is invisible until
    // something tries to send it -- at which point the *receiver* reports
    // a broken stream and the sender looks fine. That is what happened
    // when audio was added, so it is pinned here.
    use cat_native::FrameKind;
    for kind in [FrameKind::Control, FrameKind::Spectrum, FrameKind::Audio] {
        assert_eq!(
            FrameKind::from_u8(kind as u8),
            Some(kind),
            "{kind:?} has a discriminant the decoder does not accept"
        );
    }
}
