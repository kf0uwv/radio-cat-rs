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

//! The protocol, end to end, over a real TCP socket.
//!
//! `NativeSession` is I/O-free by design and has always been tested by
//! handing it messages directly. That proves the state machine and nothing
//! about the wire: until now **no test had ever put a byte of this
//! protocol on a socket**. Framing, partial reads, two frame kinds
//! interleaved on one stream and the newest-wins spectrum rule are all
//! properties of the transport, not the session, and all four are things a
//! GUI will depend on from its first frame.
//!
//! The server side here is deliberately a plain thread rather than the
//! real `cat-server` — that crate pulls monoio and every transport, which
//! is exactly what the client must not need.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use cat_framework::capabilities::*;
use cat_native::{
    decode_frame, encode_frame, ClientMessage, Command, Connection, FrameError, FrameKind,
    NativeSession, ServerMessage,
};
use cat_signal::SpectrumFrame;

const MODES: &[ModeDescriptor] = &[ModeDescriptor {
    id: ModeId::Usb,
    label: "USB",
    kind: ModeKind::Ssb,
    sideband: Some(Sideband::Upper),
    default_bandwidth_hz: 2400,
}];

const METERS: &[MeterDescriptor] = &[MeterDescriptor {
    kind: MeterKind::S,
    raw_range: RawRange::new(0, 30),
    active_on_transmit: false,
    s_units: Some(SUnitScale::TS570D),
}];

const ENDPOINTS: &[EndpointDescriptor] = &[EndpointDescriptor {
    role: EndpointRole::Cat,
    required: true,
    shareable_with: &[],
}];

static RADIO: RadioCapabilities = RadioCapabilities {
    model: "Socket Test Radio",
    endpoints: EndpointSet::new(ENDPOINTS),
    vfos: VfoCapability {
        count: 2,
        split: true,
        rit_hz: Some(9999),
        xit_hz: Some(9999),
    },
    modes: MODES,
    tuning_steps_hz: &[10, 100],
    rx_range: FrequencyRange::new(500_000, 60_000_000),
    filters: FilterCapability {
        if_shift_hz: Some(1_000),
        widths_hz: None,
        notch: false,
    },
    meters: MeterSet::new(METERS),
    memory: None,
    menu: None,
    signal: SignalSupport::IfTapPoint {
        if_center_hz: 73_050_000,
        inverted: true,
    },
};

/// What the fake server should do beyond answering control messages.
#[derive(Clone, Copy, PartialEq)]
enum Extra {
    Nothing,
    /// Push `n` spectrum frames immediately after the handshake, with
    /// ascending sequence numbers.
    SpectrumBurst(u32),
}

fn spectrum_frame(sequence: u64) -> SpectrumFrame {
    SpectrumFrame {
        center_hz: 14_074_000,
        span_hz: 48_000,
        ref_level_dbm: -100.0,
        sequence,
        bins: vec![-90.0, -80.0, -70.0, -60.0],
    }
}

/// Serve exactly one connection with a real `NativeSession`, then stop.
fn serve(extra: Extra) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut session = NativeSession::new(&RADIO);
        let mut pending: Vec<u8> = Vec::new();
        let mut buf = [0u8; 8192];
        let mut greeted = false;

        loop {
            let n = match stream.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            pending.extend_from_slice(&buf[..n]);

            let mut consumed = 0;
            while let Ok((kind, payload, used)) = decode_frame(&pending[consumed..]) {
                if kind == FrameKind::Control {
                    let message: ClientMessage =
                        serde_json::from_slice(payload).expect("client sent valid JSON");
                    let reply = session.handle(message);
                    let bytes = serde_json::to_vec(&reply).unwrap();
                    if stream
                        .write_all(&encode_frame(FrameKind::Control, &bytes))
                        .is_err()
                    {
                        return;
                    }
                    if !greeted {
                        greeted = true;
                        if let Extra::SpectrumBurst(count) = extra {
                            // Only if the client actually asked for them --
                            // the session is the authority on that.
                            if session.wants_spectrum() {
                                for i in 0..count {
                                    let payload = cat_native::encode_spectrum_payload(
                                        &spectrum_frame(u64::from(i)),
                                    );
                                    let _ = stream
                                        .write_all(&encode_frame(FrameKind::Spectrum, &payload));
                                }
                            }
                        }
                    }
                    let _ = stream.flush();
                }
                consumed += used;
            }
            pending.drain(..consumed);
        }
    });
    port
}

#[test]
fn a_connection_that_exists_has_already_been_told_what_the_radio_is() {
    // There is no "connected but not yet handshaken" state to forget to
    // check: `connect` either returns capabilities or returns an error.
    let port = serve(Extra::Nothing);
    let conn = Connection::connect(("127.0.0.1", port), false).expect("connect");
    assert_eq!(conn.capabilities().model, "Socket Test Radio");
    assert_eq!(conn.capabilities().modes.len(), 1);
    assert_eq!(conn.capabilities().rx_range.max_hz, 60_000_000);
}

#[test]
fn the_radios_s_unit_table_survives_the_socket() {
    // The reason `SUnitScale` is a fixed-size array: it needs no owned
    // mirror, so a remote console reads the same S-units as a local one
    // instead of quietly falling back to an interpolated scale.
    let port = serve(Extra::Nothing);
    let conn = Connection::connect(("127.0.0.1", port), false).expect("connect");
    let s = conn.capabilities().meters[0]
        .s_units
        .expect("the table crossed the wire");
    assert_eq!(s.label(24), "S9+10");
}

#[test]
fn the_if_tap_orientation_survives_the_socket() {
    // A client that lost `inverted` would mirror every signal about the
    // dial and look entirely plausible doing it.
    let port = serve(Extra::Nothing);
    let conn = Connection::connect(("127.0.0.1", port), false).expect("connect");
    match conn.capabilities().signal {
        SignalSupport::IfTapPoint {
            if_center_hz,
            inverted,
        } => {
            assert_eq!(if_center_hz, 73_050_000);
            assert!(inverted);
        }
        other => panic!("expected an IF tap, got {other:?}"),
    }
}

#[test]
fn a_command_gets_its_own_reply_back() {
    let port = serve(Extra::Nothing);
    let mut conn = Connection::connect(("127.0.0.1", port), false).expect("connect");
    let reply = conn
        .command(Command::SetFrequency {
            vfo: 0,
            hz: 14_074_000,
        })
        .expect("command");
    assert_eq!(reply, ServerMessage::Ack);
}

#[test]
fn a_command_the_radio_cannot_do_is_refused_over_the_wire_too() {
    // Capability validation is the session's job, but a client has to be
    // able to *see* the refusal, with a code rather than English prose.
    let port = serve(Extra::Nothing);
    let mut conn = Connection::connect(("127.0.0.1", port), false).expect("connect");
    let reply = conn
        .command(Command::SetFrequency {
            vfo: 0,
            hz: 450_000_000,
        })
        .expect("a refusal is still a reply");
    match reply {
        ServerMessage::Error { code, .. } => {
            assert_eq!(code, cat_native::ErrorCode::OutOfRange)
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_client_that_declines_spectrum_receives_not_one_byte_of_it() {
    // Asserted in-process already. Asserted here on an actual socket,
    // because "receives no frames" is a claim about the wire.
    let port = serve(Extra::SpectrumBurst(8));
    let mut conn = Connection::connect(("127.0.0.1", port), false).expect("connect");
    conn.ping().expect("ping");
    assert!(conn.take_spectrum().is_none());
    assert!(conn
        .poll(Some(Duration::from_millis(150)))
        .unwrap()
        .is_none());
}

#[test]
fn a_burst_of_spectrum_frames_collapses_to_the_newest() {
    // The rule the waterfall depends on. A backlog of stale frames is
    // worse than none: the UI would spend its catching-up drawing
    // spectra that are no longer true.
    let port = serve(Extra::SpectrumBurst(8));
    let mut conn = Connection::connect(("127.0.0.1", port), true).expect("connect");
    // Give the burst time to arrive, then read it all in one go.
    std::thread::sleep(Duration::from_millis(100));
    let frame = conn
        .poll(Some(Duration::from_millis(500)))
        .expect("poll")
        .expect("a frame arrived");
    assert_eq!(
        frame.sequence, 7,
        "kept a stale frame instead of the newest"
    );
    assert_eq!(frame.bins.len(), 4);
    assert_eq!(frame.center_hz, 14_074_000);
    // And nothing is left queued behind it.
    assert!(conn.take_spectrum().is_none());
}

#[test]
fn a_reply_arriving_behind_spectrum_frames_is_not_lost() {
    // The bug this shape exists to prevent: `poll` decoding a control
    // frame while looking for spectrum, and dropping it. A GUI polling
    // for frames in its draw loop would lose command replies at random,
    // which is the worst kind of bug to go looking for later.
    let port = serve(Extra::SpectrumBurst(4));
    let mut conn = Connection::connect(("127.0.0.1", port), true).expect("connect");
    std::thread::sleep(Duration::from_millis(100));

    // Drain the burst with a poll, then ask a question.
    let _ = conn.poll(Some(Duration::from_millis(200))).expect("poll");
    let reply = conn
        .command(Command::SetMode { mode: ModeId::Usb })
        .unwrap();
    assert_eq!(reply, ServerMessage::Ack);
}

#[test]
fn a_frame_split_across_two_reads_still_decodes() {
    // Every real socket does this eventually, and a decoder that assumed
    // whole frames would work perfectly on loopback and fail in the shack.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut session = NativeSession::new(&RADIO);
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).expect("read hello");
        let (_, payload, _) = decode_frame(&buf[..n]).expect("hello framed");
        let hello: ClientMessage = serde_json::from_slice(payload).unwrap();
        let reply = serde_json::to_vec(&session.handle(hello)).unwrap();
        let framed = encode_frame(FrameKind::Control, &reply);

        // Deliberately split mid-payload, with a pause, so the client must
        // hold partial bytes across two reads.
        let split = framed.len() / 2;
        stream.write_all(&framed[..split]).unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(60));
        stream.write_all(&framed[split..]).unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(200));
    });

    let conn = Connection::connect(("127.0.0.1", port), false).expect("connect");
    assert_eq!(conn.capabilities().model, "Socket Test Radio");
}

#[test]
fn a_server_that_hangs_up_is_reported_rather_than_hung_on() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        drop(stream);
    });
    let err = Connection::connect(("127.0.0.1", port), false)
        .err()
        .expect("connecting to a server that hangs up must fail");
    // Whether this arrives as a clean EOF or as RST depends on whether the
    // peer had unread data queued -- timing, not anything a caller can act
    // on. Both are `Closed`; this test is what established that they need
    // to be.
    assert!(
        matches!(err, cat_native::ClientError::Closed),
        "expected Closed, got {err:?}"
    );
}

#[test]
fn garbage_on_the_wire_is_a_frame_error_and_not_a_panic() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        // Frame kind 99 does not exist.
        let _ = stream.write_all(&[99, 0, 0, 0, 1, b'x']);
        let _ = stream.flush();
        std::thread::sleep(Duration::from_millis(200));
    });
    let err = Connection::connect(("127.0.0.1", port), false)
        .err()
        .expect("an unknown frame kind must not be accepted");
    assert!(
        matches!(
            err,
            cat_native::ClientError::Frame(FrameError::UnknownKind(99))
        ),
        "got {err:?}"
    );
}

#[test]
fn the_threaded_client_hands_a_frame_loop_what_it_needs_without_blocking() {
    // The shape a GUI uses: capabilities up front, spectrum newest-wins,
    // replies on a channel, and nothing that waits on a socket.
    let port = serve(Extra::SpectrumBurst(4));
    let client = cat_native::Client::connect(("127.0.0.1", port), true).expect("connect");
    assert_eq!(client.capabilities().model, "Socket Test Radio");

    assert!(client.send(Command::SetMode { mode: ModeId::Usb }));

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut saw_reply = false;
    let mut saw_spectrum = false;
    while std::time::Instant::now() < deadline && !(saw_reply && saw_spectrum) {
        if let Some(cat_native::Event::Reply(ServerMessage::Ack)) = client.try_event() {
            saw_reply = true;
        }
        if client.take_spectrum().is_some() {
            saw_spectrum = true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(saw_reply, "no command reply reached the frame loop");
    assert!(saw_spectrum, "no spectrum frame reached the frame loop");
}

#[test]
fn a_stream_is_a_stream_even_when_the_test_writes_it_by_hand() {
    // Guards the framing contract itself, independent of either side's
    // implementation: encode two frames back to back and decode both out
    // of one buffer, the way a socket delivers them.
    let a = encode_frame(FrameKind::Control, b"{}");
    let b = encode_frame(FrameKind::Spectrum, &[1, 2, 3]);
    let mut stream = a.clone();
    stream.extend_from_slice(&b);

    let (kind, payload, used) = decode_frame(&stream).unwrap();
    assert_eq!(kind, FrameKind::Control);
    assert_eq!(payload, b"{}");
    let (kind, payload, _) = decode_frame(&stream[used..]).unwrap();
    assert_eq!(kind, FrameKind::Spectrum);
    assert_eq!(payload, &[1, 2, 3]);
}

/// A frame arriving one byte at a time must never decode early.
#[test]
fn a_partial_frame_is_incomplete_and_not_an_error() {
    let framed = encode_frame(FrameKind::Control, b"{\"type\":\"ping\"}");
    for cut in 0..framed.len() {
        assert_eq!(
            decode_frame(&framed[..cut]).unwrap_err(),
            FrameError::Incomplete,
            "decoded a frame from {cut} of {} bytes",
            framed.len()
        );
    }
    assert!(decode_frame(&framed).is_ok());
}

/// Nothing here should need a live server to be worth running.
#[test]
fn connecting_to_nothing_fails_promptly() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    assert!(Connection::connect(("127.0.0.1", port), false).is_err());
}

/// Keeps `TcpStream` imported meaningfully if the above ever changes.
#[allow(dead_code)]
fn _type_check(_: TcpStream) {}
