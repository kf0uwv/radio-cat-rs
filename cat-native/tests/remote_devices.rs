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

//! Asking the radio's host what it can see, over a real socket.
//!
//! The bug the whole feature exists to prevent is a console enumerating
//! *its own* hardware and offering it as the radio's: an operator on a
//! laptop picking their laptop microphone as the radio's receive audio.
//! That looks entirely correct on screen and is wrong, so these tests are
//! about who is being asked, not merely that an answer arrives.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cat_framework::capabilities::*;
use cat_native::client::ClientError;
use cat_native::{
    serve, Command, Connection, ErrorCode, RadioHost, RadioState, ServerMessage, Streams,
};
use cat_signal::{DeviceInfo, DeviceKind, DeviceList};

const MODES: &[ModeDescriptor] = &[
    ModeDescriptor {
        id: ModeId::Lsb,
        label: "LSB",
        kind: ModeKind::Ssb,
        sideband: Some(Sideband::Lower),
        default_bandwidth_hz: 2400,
    },
    ModeDescriptor {
        id: ModeId::Usb,
        label: "USB",
        kind: ModeKind::Ssb,
        sideband: Some(Sideband::Upper),
        default_bandwidth_hz: 2400,
    },
];

const METERS: &[MeterDescriptor] = &[
    MeterDescriptor {
        kind: MeterKind::S,
        raw_range: RawRange::new(0, 30),
        active_on_transmit: false,
        s_units: Some(SUnitScale::TS570D),
    },
    MeterDescriptor {
        kind: MeterKind::Swr,
        raw_range: RawRange::new(0, 30),
        active_on_transmit: true,
        s_units: None,
    },
];

const ENDPOINTS: &[EndpointDescriptor] = &[EndpointDescriptor {
    role: EndpointRole::Cat,
    required: true,
    shareable_with: &[],
}];

static RADIO: RadioCapabilities = RadioCapabilities {
    model: "Dummy Radio",
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

/// The radio's host. Its devices are deliberately named nothing like a
/// real machine's, so a test cannot pass by accident if a client ever
/// enumerated locally instead of asking.
struct Host {
    /// `None` = this host declines device selection entirely.
    devices: Mutex<Option<Vec<DeviceList>>>,
    attached: Mutex<Vec<(DeviceKind, String)>>,
    busy: AtomicBool,
}

fn radio_side_devices() -> Vec<DeviceList> {
    vec![
        DeviceList::found(
            DeviceKind::AudioInput,
            vec![DeviceInfo {
                kind: DeviceKind::AudioInput,
                spec: "audio:shack-codec-acc2".to_string(),
                label: "ACC2 USB codec (in the shack)".to_string(),
                detail: None,
                is_default: true,
            }],
        ),
        DeviceList::found(
            DeviceKind::Sdr,
            vec![DeviceInfo {
                kind: DeviceKind::Sdr,
                spec: "rtl:shack-dongle".to_string(),
                label: "RTL-SDR on the IF tap".to_string(),
                detail: None,
                is_default: false,
            }],
        ),
    ]
}

impl Host {
    fn offering(devices: Option<Vec<DeviceList>>) -> Arc<Self> {
        Arc::new(Self {
            devices: Mutex::new(devices),
            attached: Mutex::new(Vec::new()),
            busy: AtomicBool::new(false),
        })
    }
}

impl RadioHost for Host {
    fn capabilities(&self) -> &'static RadioCapabilities {
        &RADIO
    }

    fn state(&self) -> RadioState {
        RadioState {
            vfo_a_hz: 14_074_000,
            vfo_b_hz: 7_074_000,
            mode: ModeId::Usb,
            split: false,
            transmitting: false,
            memory_channel: None,
            if_shift_hz: Some(0),
            filter_width_hz: None,
            meters: Vec::new(),
        }
    }

    fn devices(&self) -> Option<Vec<DeviceList>> {
        self.devices.lock().unwrap().clone()
    }

    fn apply(&self, command: &Command) -> Result<(), String> {
        if let Command::AttachDevice { kind, spec } = command {
            if self.busy.load(Ordering::Relaxed) {
                return Err("device or resource busy".to_string());
            }
            self.attached.lock().unwrap().push((*kind, spec.clone()));
        }
        Ok(())
    }
}

fn serve_host(host: Arc<Host>) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let _ = serve(listener, host);
    });
    port
}

fn connect(port: u16) -> Connection {
    Connection::connect(("127.0.0.1", port), Streams::none()).unwrap()
}

#[test]
fn a_console_is_told_what_the_radios_host_can_see() {
    let host = Host::offering(Some(radio_side_devices()));
    let mut client = connect(serve_host(Arc::clone(&host)));

    let lists = client.read_devices().unwrap().expect("host offers devices");

    let specs: Vec<String> = lists
        .iter()
        .flat_map(|l| l.devices.iter().map(|d| d.spec.clone()))
        .collect();
    assert!(
        specs.contains(&"audio:shack-codec-acc2".to_string()),
        "the radio's own audio device did not reach the console: {specs:?}"
    );
    assert!(
        specs.contains(&"rtl:shack-dongle".to_string()),
        "the radio's own SDR did not reach the console: {specs:?}"
    );
}

#[test]
fn declining_the_question_is_not_the_same_as_having_nothing() {
    // The distinction this feature turns on. A console that read these two
    // the same way would tell an operator to plug something in when the
    // real answer is that this server cannot help them from here.
    let declines = Host::offering(None);
    let mut client = connect(serve_host(declines));
    assert_eq!(
        client.read_devices().unwrap(),
        None,
        "a host that never publishes must not look like a host with empty slots"
    );

    let empty = Host::offering(Some(vec![DeviceList::found(DeviceKind::Sdr, Vec::new())]));
    let mut client = connect(serve_host(empty));
    let lists = client.read_devices().unwrap().expect("offered, just empty");
    assert_eq!(lists.len(), 1);
    assert!(lists[0].devices.is_empty());
    assert!(
        lists[0].is_available(),
        "an empty answer is still an answer, and reads differently from no answer"
    );
}

#[test]
fn the_reason_a_group_could_not_be_enumerated_survives_the_wire() {
    // `DeviceList`'s third outcome is the one most easily flattened away
    // by a serialization round trip, and it is the one that tells an
    // operator their build lacks a backend rather than their cable is out.
    let host = Host::offering(Some(vec![DeviceList::unavailable(
        DeviceKind::AudioInput,
        "no sound backend compiled into this server",
    )]));
    let mut client = connect(serve_host(host));

    let lists = client.read_devices().unwrap().unwrap();
    assert!(!lists[0].is_available());
    assert_eq!(
        lists[0].error.as_deref(),
        Some("no sound backend compiled into this server"),
    );
}

#[test]
fn attaching_names_a_device_on_the_radios_machine() {
    let host = Host::offering(Some(radio_side_devices()));
    let mut client = connect(serve_host(Arc::clone(&host)));

    client
        .attach_device(DeviceKind::Sdr, "rtl:shack-dongle")
        .unwrap();

    assert_eq!(
        *host.attached.lock().unwrap(),
        vec![(DeviceKind::Sdr, "rtl:shack-dongle".to_string())],
        "the attach reached the radio's host, with the spec unmodified"
    );
}

#[test]
fn a_refused_attach_carries_the_hosts_own_words() {
    // "device or resource busy" names the other program holding the
    // dongle. A tidied-up "attach failed" would send an operator to check
    // their cabling instead of their process list.
    let host = Host::offering(Some(radio_side_devices()));
    host.busy.store(true, Ordering::Relaxed);
    let mut client = connect(serve_host(Arc::clone(&host)));

    let err = client
        .attach_device(DeviceKind::Sdr, "rtl:shack-dongle")
        .unwrap_err();

    match err {
        ClientError::Unexpected(ServerMessage::Error { message, .. }) => {
            assert_eq!(message, "device or resource busy");
        }
        other => panic!("expected the host's refusal, got {other:?}"),
    }
    assert!(host.attached.lock().unwrap().is_empty());
}

#[test]
fn a_dongle_plugged_in_after_the_server_started_still_appears() {
    // Enumerated per call, not cached at startup. An operator who plugs in
    // a dongle and sees nothing has no way to tell that from a broken one.
    let host = Host::offering(Some(vec![DeviceList::found(DeviceKind::Sdr, Vec::new())]));
    let mut client = connect(serve_host(Arc::clone(&host)));
    assert!(client.read_devices().unwrap().unwrap()[0]
        .devices
        .is_empty());

    *host.devices.lock().unwrap() = Some(radio_side_devices());

    let lists = client.read_devices().unwrap().unwrap();
    assert!(
        lists.iter().any(|l| !l.devices.is_empty()),
        "the same connection never saw the new hardware"
    );
}

#[test]
fn an_older_server_reads_as_declining_rather_than_as_a_fault() {
    // A server built before this command existed cannot deserialize the
    // `cmd` tag and answers `Malformed`. From a console's side that is the
    // same situation as a server that declines: there is no list to show.
    // Asserted at the mapping, since an old binary cannot be linked here.
    let malformed = ServerMessage::Error {
        code: ErrorCode::Malformed,
        message: "unknown variant `read_devices`".to_string(),
    };
    let json = serde_json::to_string(&malformed).unwrap();
    let back: ServerMessage = serde_json::from_str(&json).unwrap();
    assert!(matches!(
        back,
        ServerMessage::Error {
            code: ErrorCode::Malformed,
            ..
        }
    ));

    // And the shape an old server actually chokes on is a `cmd` tag it has
    // no variant for, which is what makes `Malformed` the reply.
    let sent = serde_json::to_string(&Command::ReadDevices).unwrap();
    assert!(sent.contains("read_devices"), "{sent}");
}

#[test]
fn a_client_that_never_said_hello_gets_no_device_list() {
    // Device specs describe the radio operator's machine. They are not
    // secret, but they are also not something to hand to a socket that has
    // not identified itself as speaking this protocol.
    let host = Host::offering(Some(radio_side_devices()));
    let port = serve_host(host);

    use std::io::{Read, Write};
    let mut raw = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let payload =
        serde_json::to_vec(&cat_native::ClientMessage::Command(Command::ReadDevices)).unwrap();
    raw.write_all(&cat_native::encode_frame(
        cat_native::FrameKind::Control,
        &payload,
    ))
    .unwrap();
    raw.flush().unwrap();

    let mut buf = [0u8; 4096];
    let n = raw.read(&mut buf).unwrap();
    let (_, body, _) = cat_native::decode_frame(&buf[..n]).unwrap();
    let reply: ServerMessage = serde_json::from_slice(body).unwrap();
    match reply {
        ServerMessage::Error { code, .. } => assert_eq!(code, ErrorCode::NotReady),
        other => panic!("a device list leaked before the handshake: {other:?}"),
    }
}

#[test]
fn the_device_reply_can_actually_be_tagged() {
    // `ServerMessage` is internally tagged, and serde refuses to tag a
    // newtype variant whose contents serialize as a sequence. The failure
    // is at *serialization*, on the server, so the only symptom a client
    // sees is the connection dropping -- which reads like a network fault
    // and is not one. Pinned here so the next sequence-shaped variant
    // fails with a message that says what is wrong.
    let message = ServerMessage::Devices {
        lists: vec![DeviceList::found(
            DeviceKind::Sdr,
            vec![DeviceInfo {
                kind: DeviceKind::Sdr,
                spec: "rtl:0".to_string(),
                label: "a dongle".to_string(),
                detail: None,
                is_default: false,
            }],
        )],
    };

    let json = serde_json::to_string(&message).expect("must serialize under the type tag");
    let back: ServerMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(back, message, "round trip changed the message");
}
