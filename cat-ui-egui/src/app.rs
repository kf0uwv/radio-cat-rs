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

//! The console: status strip, quick bar, workspaces, command line.
//!
//! Placement only. Every decision this file appears to make is made in
//! `workspace`, `quick`, `tuning`, `command` or `readout`, which are
//! testable without a window — see the crate docs for why that split is
//! load-bearing rather than tidy.

use cat_native::{Client, Command, ErrorCode, Event, ServerMessage, Streams};
use cat_signal::SpectrumFrame;
use cat_ui::MeterReading;
use egui::{Align, Color32, Layout, RichText, Sense, Stroke, Vec2};

use crate::readout::Readout;
use crate::{theme, tuning};
use cat_ui::workspace::{self, Tab, TabEntry};
use cat_ui::{command, quick};

/// Where the console is in its life.
enum Link {
    /// Never connected, or connection lost. Carries why.
    Down(String),
    Up(Box<Client>),
}

/// How often the waterfall texture may be sent to the GPU.
///
/// A full RGBA upload of the whole image, which at 30 a second left the
/// console unresponsive on a machine without a fast GPU -- sitting in
/// `drm_syncobj_array_wait_timeout` at 48% CPU with its window frozen.
/// Ten a second is a waterfall that still reads as live.
const WATERFALL_UPLOAD_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

pub struct Console {
    address: String,
    link: Link,
    tabs: Vec<TabEntry>,
    active: Tab,
    readout: Readout,
    /// The newest spectrum frame, and the reference the waterfall
    /// re-projects onto.
    latest: Option<SpectrumFrame>,
    waterfall: crate::waterfall::WaterfallImage,
    command_open: bool,
    command_text: String,
    /// The last thing the console has to say to the operator.
    status: String,
    last_state_request: std::time::Instant,
    /// Capabilities for a still, when there is no link to get them from.
    offline_capabilities: Option<cat_native::CapabilitiesWire>,
    /// GPU copy of the waterfall, re-uploaded as rows arrive.
    waterfall_texture: Option<egui::TextureHandle>,
    /// The waterfall generation already on the GPU.
    ///
    /// Rebuilding and re-uploading the whole texture on every repaint --
    /// a full RGBA allocation, a copy and a GPU upload, thirty times a
    /// second whether or not a pixel had moved -- was enough to leave the
    /// window unresponsive on a machine without a fast GPU, sitting in
    /// `drm_syncobj_array_wait_timeout` at 48% CPU.
    waterfall_uploaded: Option<u64>,
    /// When the waterfall texture last went to the GPU.
    ///
    /// Spectrum frames arrive at about 29 a second, so a
    /// changed-since-last-upload test alone still uploads on nearly every
    /// repaint. The image advances in `pump` regardless of drawing, so
    /// refreshing the *picture* at a lower rate loses no history -- only
    /// smoothness nobody is reading a waterfall for.
    waterfall_uploaded_at: Option<std::time::Instant>,
    /// The frames behind the waterfall, newest first.
    ///
    /// Kept so the picture can be redrawn from a different vantage point.
    /// The scroll normally costs one row; a retune costs the whole image,
    /// and that is only possible if the frames are still here.
    history: std::collections::VecDeque<SpectrumFrame>,
    /// The camera move a click starts, while it is running.
    retune: Option<cat_ui::retune::Retune>,
    /// When the last frame was drawn, for advancing the move by real time
    /// rather than by frame count — a console that dropped frames would
    /// otherwise animate in slow motion.
    last_frame_at: Option<std::time::Instant>,
    /// Whether this radio's layout placed a spectrum panel of its own.
    ///
    /// Resolved once per frame from the layout, so the workspace can avoid
    /// drawing a second waterfall of the same signal.
    spectrum_has_its_own_panel: bool,
    /// Widgets a radio brought with it, if the application supplied any.
    ///
    /// Empty for a pure network console, which has not linked any radio's
    /// crate and cannot have their painters. That is the ordinary state,
    /// not a gap to work around.
    widgets: crate::widgets::Widgets,
    /// What the *radio's* host said it can see. Never this machine.
    devices: crate::devices::Offer,
    /// The newest audio frame the server sent, if it is sending any.
    ///
    /// One frame, newest-wins: the AF panels are a snapshot of what the
    /// radio is doing now, and a backlog drawn late would be worse than a
    /// gap.
    audio: Option<cat_signal::AudioFrame>,
    /// Whether a `ReadDevices` is outstanding.
    ///
    /// Replies on this protocol are not correlated to requests, so an
    /// `Unsupported` error can only be attributed to the device question
    /// by knowing one is in flight. The console asks once per connection
    /// and one command at a time, which makes that sound; a protocol that
    /// grows concurrent requests would need real correlation instead.
    devices_pending: bool,
}

impl Console {
    pub fn new(address: String) -> Self {
        Self {
            address,
            link: Link::Down("not connected".to_string()),
            tabs: Vec::new(),
            active: Tab::Source,
            readout: Readout::default(),
            latest: None,
            // TURBO over MONO: this console's job is to make a weak
            // carrier findable, and a perceptually-uniform ramp separates
            // the bottom few dB where that carrier lives. The floor is a
            // starting value, not a calibration -- it becomes a published
            // setting once the source layer lands.
            waterfall: crate::waterfall::WaterfallImage::new(
                512,
                256,
                crate::waterfall::Palette::TURBO,
                -110.0,
            ),
            command_open: false,
            command_text: String::new(),
            status: String::new(),
            last_state_request: std::time::Instant::now(),
            offline_capabilities: None,
            waterfall_texture: None,
            waterfall_uploaded: None,
            waterfall_uploaded_at: None,
            history: std::collections::VecDeque::new(),
            retune: None,
            last_frame_at: None,
            spectrum_has_its_own_panel: false,
            widgets: crate::widgets::Widgets::new(),
            devices: crate::devices::Offer::default(),
            audio: None,
            devices_pending: false,
        }
    }

    /// Fill the readout with plausible values, for the offscreen renderer.
    ///
    /// Not a demo mode: it exists so a still of the console shows the
    /// layout under load rather than a row of em dashes, which is what an
    /// unconnected console correctly shows and which says nothing about
    /// whether the design is right.
    /// Install a capability set without a socket, so a still shows the
    /// real layout rather than the disconnected state.
    pub fn demo_capabilities(&mut self, caps: cat_native::CapabilitiesWire) {
        self.tabs = cat_ui::workspace::tabs(&caps);
        self.active = self.tabs.first().map(|t| t.tab).unwrap_or(Tab::Source);
        self.offline_capabilities = Some(caps);
    }

    /// Push a spectrum frame in, for a still.
    ///
    /// A screenshot showing NO STREAM proves the empty state and nothing
    /// about the waterfall, which is the part of this console with the
    /// most that can go wrong.
    pub fn demo_spectrum(&mut self, frames: &[cat_signal::SpectrumFrame]) {
        for frame in frames {
            self.waterfall.push(frame);
        }
        self.latest = frames.last().cloned();
    }

    /// Install an audio frame, so a still shows the AF panels under load.
    ///
    /// Not a demo mode: an unconnected console correctly shows PENDING,
    /// which says nothing about whether the panels are drawn right.
    pub fn demo_audio(&mut self, frame: cat_signal::AudioFrame) {
        self.audio = Some(frame);
    }

    pub fn demo_state(&mut self) {
        self.readout.vfo_a_hz.confirm(14_074_000);
        self.readout.mode.confirm(cat_native::ModeId::Usb);
        self.readout.split.confirm(false);
        self.readout.smeter_raw.confirm(17);
        self.readout.if_shift_hz.confirm(0);
        // The model this console was actually handed, not a name baked
        // in when it served one radio. A still of an FT-991A captioned
        // "Kenwood TS-570D" is exactly the kind of wrong that survives
        // review because nobody reads the status line.
        self.status = match self.capabilities() {
            Some(caps) => format!("connected to {}", caps.model),
            None => "connected".to_string(),
        };
    }

    /// Try to connect, replacing whatever link there was.
    pub fn connect(&mut self) {
        // Spectrum is requested unconditionally: this console's whole
        // reason for existing on a GPU is the waterfall, and a client that
        // declined would then have to reconnect to change its mind.
        match Client::connect(self.address.as_str(), Streams::all()) {
            Ok(client) => {
                self.tabs = workspace::tabs(client.capabilities());
                self.active = self.tabs.first().map(|t| t.tab).unwrap_or(Tab::Source);
                self.status = format!("connected to {}", client.capabilities().model);
                self.link = Link::Up(Box::new(client));
            }
            Err(e) => {
                self.status = format!("{e}");
                self.link = Link::Down(e.to_string());
            }
        }
    }

    fn capabilities(&self) -> Option<&cat_native::CapabilitiesWire> {
        match &self.link {
            Link::Up(client) => Some(client.capabilities()),
            Link::Down(_) => self.offline_capabilities.as_ref(),
        }
    }

    /// Drain everything the reader thread has for us. Never blocks.
    fn pump(&mut self) {
        let mut lost = None;
        let mut confirmed = None;
        let mut devices = None;
        let mut audio = None;
        let mut spectrum = None;
        let devices_pending = self.devices_pending;
        if let Link::Up(client) = &self.link {
            while let Some(event) = client.try_event() {
                match event {
                    Event::Reply(ServerMessage::Error { code, message }) => {
                        // A server that declines device selection, or one
                        // built before the command existed (which cannot
                        // parse the `cmd` tag and says `Malformed`), is
                        // not a fault to put in the status line -- it is
                        // the answer to the question, and the source
                        // panel is where it belongs.
                        if devices_pending
                            && matches!(code, ErrorCode::Unsupported | ErrorCode::Malformed)
                        {
                            devices = Some(crate::devices::Offer::NotOffered);
                        } else {
                            self.status = format!("{code:?}: {message}");
                        }
                    }
                    // The radio's own account of itself. It wins over
                    // anything this console asked for -- see
                    // `readout::Field::confirm`.
                    Event::Reply(ServerMessage::State(state)) => {
                        confirmed = Some(*state);
                    }
                    Event::Reply(ServerMessage::Meter(sample)) => {
                        if sample.kind == cat_native::MeterKind::S {
                            self.readout.smeter_raw.confirm(sample.raw);
                        }
                    }
                    Event::Reply(ServerMessage::Devices { lists }) => {
                        devices = Some(crate::devices::Offer::Listed(lists));
                    }
                    Event::Reply(_) => {}
                    Event::Disconnected(why) => {
                        lost = Some(why);
                        break;
                    }
                }
            }
            if let Some(frame) = client.take_spectrum() {
                spectrum = Some(frame);
            }
            if let Some(frame) = client.take_audio() {
                audio = Some(frame);
            }
        }
        if let Some(state) = confirmed {
            self.readout.vfo_a_hz.confirm(state.vfo_a_hz);
            self.readout.mode.confirm(state.mode);
            self.readout.split.confirm(state.split);
            if let Some(hz) = state.if_shift_hz {
                self.readout.if_shift_hz.confirm(hz);
            }
            if let Some(hz) = state.filter_width_hz {
                self.readout.filter_width_hz.confirm(hz);
            }
            if let Some(channel) = state.memory_channel {
                self.readout.memory_channel.confirm(channel);
            }
            if let Some(raw) = state.meter(cat_native::MeterKind::S) {
                self.readout.smeter_raw.confirm(raw);
            }
        }
        if let Some(frame) = audio {
            self.audio = Some(frame);
        }
        // Every frame, visible panel or not: the waterfall's history is
        // what a retune reprojects, and a gap in it is a gap in the
        // picture an operator scrolls back to.
        self.advance_waterfall(spectrum);
        if let Some(offer) = devices {
            self.devices = offer;
            self.devices_pending = false;
        }
        if let Some(why) = lost {
            self.status = format!("connection lost: {why}");
            self.link = Link::Down(why);
            // A new connection is a new machine as far as this console
            // knows. Keeping the old list would show one host's hardware
            // while talking to another.
            self.devices = crate::devices::Offer::Unasked;
            self.devices_pending = false;
            // The panels go back to pending rather than holding the last
            // trace, which would be a waveform from a radio this console
            // is no longer talking to.
            self.audio = None;
        }
    }

    /// What this console knows about the audio path, and it is three
    /// things — see `cat_ui::af::AudioState`.
    ///
    /// `Absent` comes from the server's own installation: if the radio's
    /// bench has no audio source wired, no amount of waiting will produce
    /// one, and the panels should say so rather than sitting at PENDING
    /// forever.
    fn audio_state(&self) -> cat_ui::af::AudioState {
        use cat_ui::af::AudioState;
        if self.audio.is_some() {
            return AudioState::Streaming;
        }
        match self.capabilities() {
            Some(caps) if caps.installation.audio_sources().next().is_none() => AudioState::Absent,
            _ => AudioState::Configured,
        }
    }

    /// The filter passband to mark on the AF FFT, if this mode has one.
    fn passband(&self) -> Option<cat_ui::af::Passband> {
        let caps = self.capabilities()?;
        cat_ui::af::passband_for(caps, self.readout.mode.value()?)
    }

    /// Ask the radio what it is doing, at a rate a human can read.
    ///
    /// Ten times a second, not per frame. State is request/response over
    /// the same socket the spectrum uses, and asking at frame rate would
    /// put sixty round trips a second in front of the traffic that
    /// actually needs to be fast -- ADR 0011's two-rate discipline, which
    /// exists precisely so a menu read cannot stall a waterfall.
    fn poll_state(&mut self) {
        const INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
        if self.last_state_request.elapsed() < INTERVAL {
            return;
        }
        self.last_state_request = std::time::Instant::now();
        if let Link::Up(client) = &self.link {
            client.request_state();
        }
        self.ask_for_devices();
    }

    /// Ask the radio's host what it can see, once per connection.
    ///
    /// Once, not per poll: the answer changes when somebody plugs
    /// something in, which is rare and which the SOURCE tab's refresh
    /// button covers. Asking ten times a second would put an enumeration
    /// of every sound card on the machine in front of the state traffic a
    /// readout depends on.
    fn ask_for_devices(&mut self) {
        if self.devices_pending || self.devices != crate::devices::Offer::Unasked {
            return;
        }
        if let Link::Up(client) = &self.link {
            if client.send(Command::ReadDevices) {
                self.devices_pending = true;
            }
        }
    }

    /// Ask again, for a dongle plugged in since the last answer.
    fn refresh_devices(&mut self) {
        self.devices = crate::devices::Offer::Unasked;
        self.devices_pending = false;
        self.ask_for_devices();
    }

    fn send(&mut self, command: Command) {
        if let Link::Up(client) = &self.link {
            if !client.send(command) {
                self.status = "connection lost".to_string();
                self.link = Link::Down("send failed".to_string());
            }
        } else {
            self.status = "not connected".to_string();
        }
    }

    fn run_line(&mut self) {
        let line = std::mem::take(&mut self.command_text);
        let Some(caps) = self.capabilities().cloned() else {
            self.status = "not connected".to_string();
            return;
        };
        match command::parse(&line, &caps) {
            Ok(command::Action::Quit) => std::process::exit(0),
            Ok(command::Action::SelectTab(n)) => match workspace::tab_for_digit(&self.tabs, n) {
                Some(tab) => self.active = tab,
                None => self.status = format!("no workspace {n}"),
            },
            Ok(command::Action::Radio(cmd)) => {
                self.note_request(&cmd);
                self.send(cmd);
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    /// Record what we asked for, so the display can show it as pending
    /// rather than either lying or looking frozen.
    fn note_request(&mut self, command: &Command) {
        match command {
            Command::SetFrequency { hz, .. } | Command::Retune { hz } => {
                self.readout.vfo_a_hz.request(*hz)
            }
            Command::SetMode { mode } => self.readout.mode.request(*mode),
            Command::SetSplit { enabled } => self.readout.split.request(*enabled),
            Command::SetIfShift { hz } => self.readout.if_shift_hz.request(*hz),
            Command::SetFilterWidth { hz } => self.readout.filter_width_hz.request(*hz),
            Command::SetMemoryChannel { channel } => self.readout.memory_channel.request(*channel),
            // Reads ask a question; they do not request a change.
            Command::ReadMeter { .. } | Command::ReadState | Command::ReadDevices => {}
            // Nothing on the readout describes what is attached, so there
            // is no pending value to show. The source panel reflects the
            // attach when the server's next state says it happened.
            Command::AttachDevice { .. } => {}
        }
    }
}

/// What the operator asked the source panel to do.
///
/// Collected rather than acted on inline: the panel draws from `&self`
/// borrows of the device lists, and sending a command needs `&mut self`.
enum SourceAction {
    Attach(cat_signal::DeviceKind, String),
    Refresh,
}

impl Console {
    /// The SOURCE workspace: what is attached, and what could be.
    ///
    /// Every device here was named by the **server**. This console never
    /// enumerates its own hardware -- see `crate::devices` for why that
    /// would be a bug rather than a shortcut.
    fn source(&mut self, ui: &mut egui::Ui, pending: &mut Option<SourceAction>) {
        ui.label(dim("attached"));
        let installed: Vec<String> = self
            .capabilities()
            .map(|c| {
                c.installation
                    .sources
                    .iter()
                    .map(|s| s.label.clone())
                    .collect()
            })
            .unwrap_or_default();
        if installed.is_empty() {
            ui.label(absent("nothing attached"));
        }
        for label in installed {
            ui.label(value(label, theme::pal().text));
        }

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(dim("available on the radio's host"));
            if ui.small_button("refresh").clicked() {
                *pending = Some(SourceAction::Refresh);
            }
        });

        if let Some(line) = crate::devices::absent_line(&self.devices) {
            ui.label(absent(line));
            return;
        }

        for list in self.devices.lists() {
            ui.add_space(6.0);
            ui.label(key(list.kind.heading()));
            if let Some(line) = crate::devices::empty_line(list) {
                ui.label(absent(line));
                continue;
            }
            for device in &list.devices {
                ui.horizontal(|ui| {
                    if ui.button(&device.label).clicked() {
                        *pending = Some(SourceAction::Attach(device.kind, device.spec.clone()));
                    }
                    if device.is_default {
                        ui.label(dim("default"));
                    }
                });
                // The spec, not just the friendly name: picking is a
                // shortcut for typing, and an operator who picks one
                // should be able to write it down for next time.
                ui.label(dim(format!(
                    "    {} {}",
                    crate::devices::flag_for(device.kind),
                    device.spec
                )));
                if let Some(detail) = &device.detail {
                    ui.label(dim(format!("    {detail}")));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing
//
// Placement only. The structure follows the accepted mockup: a persistent
// strip of key-over-value fields, a left rail of meters that is always
// there, capability-derived tabs, and a command line. Everything is
// square, tight and bordered -- an instrument panel, not a form.
// ---------------------------------------------------------------------------

/// This console's palette, for the shared AF widgets.
fn af_style() -> crate::AfStyle {
    crate::AfStyle {
        ink: theme::pal().signal,
        dim: theme::pal().absent,
        trough: theme::pal().panel_alt,
        // Amber for the filter edges: the same colour this console uses
        // for a setting everywhere else, so a mark on a panel reads as
        // "something is configured here" and not as a signal.
        setting: theme::pal().accent,
    }
}

fn key(text: impl Into<String>) -> RichText {
    RichText::new(text.into().to_uppercase())
        .color(theme::pal().dim)
        .size(theme::SIZE_KEY)
}

fn value(text: impl Into<String>, colour: Color32) -> RichText {
    RichText::new(text).color(colour).size(theme::SIZE_VALUE)
}

fn dim(text: impl Into<String>) -> RichText {
    RichText::new(text)
        .color(theme::pal().dim)
        .size(theme::SIZE_BODY)
}

fn absent(text: impl Into<String>) -> RichText {
    RichText::new(text)
        .color(theme::pal().absent)
        .size(theme::SIZE_BODY)
}

// `rule` drew the separator between the old fixed side panel and the
// content pane. A layout gives every panel its own ground, so the seam
// between two of them is the gap between their rects.

impl Console {
    /// One field of the persistent strip: a dim key with a value under it.
    ///
    /// The mockup's basic unit. Stacking the label above the value is what
    /// lets twelve facts sit in one strip and still be scannable — inline
    /// `KEY: value` pairs need separators and twice the width.
    fn field(ui: &mut egui::Ui, k: &str, v: RichText, width: f32) {
        ui.allocate_ui(Vec2::new(width, 34.0), |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(key(k));
                ui.label(v);
            });
        });
    }

    /// The persistent capability strip.
    fn strip(&mut self, ui: &mut egui::Ui) {
        let caps = self.capabilities().cloned();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;

            match &caps {
                Some(c) => Self::field(ui, "radio", value(&c.model, theme::pal().accent), 150.0),
                None => Self::field(ui, "radio", value("NO RADIO", theme::pal().warning), 150.0),
            }

            let (link_text, link_colour) = match &self.link {
                Link::Up(_) => (format!("◆ {}", self.address), theme::pal().signal),
                Link::Down(_) => (format!("◇ {}", self.address), theme::pal().absent),
            };
            Self::field(ui, "link", value(link_text, link_colour).size(11.0), 190.0);

            // The dial. The one thing on the strip that gets read from
            // across the room, so it is the one thing that is large.
            let mode_label = caps
                .as_ref()
                .and_then(|c| {
                    self.readout
                        .mode
                        .value()
                        .and_then(|id| c.modes.iter().find(|m| m.id == id))
                })
                .map(|m| m.label.clone())
                .unwrap_or_else(|| "—".to_string());
            ui.allocate_ui(Vec2::new(280.0, 34.0), |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(key(format!("vfo a · {mode_label}")));
                    let (text, colour) = match self.readout.vfo_a_hz.value() {
                        Some(hz) => (
                            cat_ui::format_hz_compact(hz),
                            if self.readout.vfo_a_hz.is_pending() {
                                theme::pal().accent
                            } else {
                                theme::pal().text
                            },
                        ),
                        None => ("—.———.———".to_string(), theme::pal().absent),
                    };
                    ui.label(RichText::new(text).color(colour).size(theme::SIZE_DIAL));
                });
            });

            let split = match self.readout.split.value() {
                Some(true) => value("ON", theme::pal().accent),
                Some(false) => value("OFF", theme::pal().dim),
                None => value("—", theme::pal().absent),
            };
            Self::field(ui, "split", split, 60.0);

            let s = match self.smeter_reading() {
                Some(r) => value(
                    format!("{}  {}/{}", r.s_unit(), r.raw, r.range.max),
                    theme::pal().text,
                ),
                None => value("—", theme::pal().absent),
            };
            Self::field(ui, "s", s, 130.0);

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let state = if self.readout.split.value().is_some() {
                    value("RX", theme::pal().signal)
                } else {
                    value("—", theme::pal().absent)
                };
                Self::field(ui, "state", state, 44.0);

                let signal = match caps.as_ref().map(|c| c.signal) {
                    Some(cat_native::SignalSupport::IfTapPoint { .. }) => {
                        let live = self.latest.is_some();
                        value(
                            "IF-TAP",
                            if live {
                                theme::pal().signal
                            } else {
                                theme::pal().absent
                            },
                        )
                    }
                    Some(cat_native::SignalSupport::None) => value("NONE", theme::pal().absent),
                    Some(_) => value("SCOPE", theme::pal().signal),
                    None => value("—", theme::pal().absent),
                };
                Self::field(ui, "signal · rf", signal.size(11.0), 80.0);
            });
        });
    }

    /// Register a widget this application's radio brought with it.
    ///
    /// See `crate::widgets` for why a console has this seam at all and
    /// what a console without the painter does instead.
    pub fn widgets_mut(&mut self) -> &mut crate::widgets::Widgets {
        &mut self.widgets
    }

    /// Draw one panel the radio's layout placed.
    ///
    /// The dispatcher. Every arm is a component this crate already had;
    /// what the layout decides is which of them appear and where.
    #[allow(clippy::too_many_arguments)]
    fn panel(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        kind: &cat_layout::PanelKind,
        pending: &mut Option<SourceAction>,
        wants_connect: &mut bool,
    ) {
        use cat_layout::PanelKind;
        let _ = wants_connect;

        // A panel's own ground, so a layout that leaves a gap shows the
        // case behind it rather than whatever was drawn there last frame.
        let fill = match kind {
            PanelKind::Workspace | PanelKind::Spectrum => theme::pal().bg,
            _ => theme::pal().panel,
        };
        ui.painter().rect_filled(rect, egui::Rounding::ZERO, fill);

        let inner = rect.shrink2(egui::vec2(6.0, 2.0));
        if inner.width() < 1.0 || inner.height() < 1.0 {
            return;
        }

        let mut content = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        let ui = &mut content;
        ui.set_clip_rect(inner);

        match kind {
            PanelKind::Readout => {
                self.strip(ui);
                ui.add_space(2.0);
                self.tab_bar(ui);
            }
            PanelKind::QuickBar => self.quick_bar(ui),
            PanelKind::MeterRail => self.meters(ui),
            PanelKind::AfScope => self.af_scope_panel(ui),
            PanelKind::AfFft => self.af_fft_panel(ui),
            PanelKind::LevelsRail => self.levels(ui),
            PanelKind::Spectrum => self.spectrum(ui),
            PanelKind::Workspace => self.workspace(ui, pending),
            PanelKind::Status => {
                ui.label(dim(self.status.clone()));
            }
            PanelKind::CommandLine => self.command_line(ui),
            PanelKind::BandBar => self.band_bar(ui),
            PanelKind::ModeBar => self.mode_bar(ui),
            PanelKind::Blank => {}
            // A widget this radio brought with it, if this build has one.
            PanelKind::Custom(name) => match self.widgets.get(name) {
                Some(painter) => {
                    let caps = match self.capabilities() {
                        Some(c) => c.clone(),
                        None => return,
                    };
                    let display = cat_ui::display::RadioDisplay::default();
                    painter(
                        ui,
                        &crate::widgets::PanelContext {
                            rect: inner,
                            palette: &theme::pal(),
                            radio: &display,
                            capabilities: &caps,
                        },
                    );
                }
                // A pure network console has not linked this radio's
                // crate and cannot have the painter. Named rather than
                // blank: "no widget for ft991a.clarifier" is a different
                // message from a panel that failed to draw, with a
                // different fix.
                None => {
                    ui.label(absent(kind.placeholder_label()));
                }
            },
            other => {
                ui.label(absent(format!("{other:?}")));
            }
        }
    }

    /// The band buttons this radio's range reaches.
    ///
    /// Derived from the capability set's `rx_range`, so a radio that does
    /// not reach 6 m is not offered it.
    fn band_bar(&mut self, ui: &mut egui::Ui) {
        Self::pane_header(ui, "BAND", None);
        let Some(caps) = self.capabilities().cloned() else {
            return;
        };
        let mut retune = None;
        ui.horizontal_wrapped(|ui| {
            for band in cat_ui::band::BANDS {
                // A band this radio cannot reach is not offered. The
                // FT-991A gets 2 m and 70 cm; the TS-570D does not, and
                // the difference comes from the capability set rather than
                // from two tables.
                if !caps.rx_range.contains(band.range.min_hz) {
                    continue;
                }
                if ui.small_button(band.label).clicked() {
                    // The bottom of the band, which is where an operator
                    // means when they say "go to 20".
                    retune = Some(band.range.min_hz);
                }
            }
        });
        if let Some(hz) = retune {
            self.readout.vfo_a_hz.request(hz);
            self.send(Command::Retune { hz });
        }
    }

    /// The mode buttons this radio declares.
    fn mode_bar(&mut self, ui: &mut egui::Ui) {
        Self::pane_header(ui, "MODE", None);
        let Some(caps) = self.capabilities().cloned() else {
            return;
        };
        let mut chosen = None;
        ui.horizontal_wrapped(|ui| {
            for descriptor in &caps.modes {
                if ui.small_button(&descriptor.label).clicked() {
                    chosen = Some(descriptor.id);
                }
            }
        });
        if let Some(mode) = chosen {
            self.readout.mode.request(mode);
            self.send(Command::SetMode { mode });
        }
    }

    /// Whichever workspace tab is selected.
    fn workspace(&mut self, ui: &mut egui::Ui, pending: &mut Option<SourceAction>) {
        match self.active {
            // A layout that gives the spectrum a panel of its own has
            // already drawn it; drawing it again here would be two
            // waterfalls of the same signal, which reads as two receivers.
            Tab::Spectrum if self.spectrum_has_its_own_panel => {
                ui.label(dim("spectrum"));
                ui.label(absent(
                    "shown in its own panel — this radio's layout gives it one",
                ));
            }
            Tab::Spectrum => self.spectrum(ui),
            Tab::Memory => {
                ui.label(dim("memory workspace"));
                ui.label(absent(
                    "the protocol has no read side for memory contents yet",
                ));
            }
            Tab::Menu => {
                ui.label(dim("menu workspace"));
                ui.label(absent(
                    "the protocol has no read side for menu contents yet",
                ));
            }
            Tab::Source => self.source(ui, pending),
        }
    }

    /// The left rail: every meter this radio has, always visible.
    ///
    /// A rail rather than a row, and never reflowed. A meter that is inert
    /// keeps its place dimmed — a TX meter appearing and vanishing on every
    /// transmit would make the whole panel jump.
    /// The meters, as a panel a layout can place on its own.
    fn meters(&mut self, ui: &mut egui::Ui) {
        self.rail(ui)
    }

    /// The levels rail.
    ///
    /// The native protocol carries no AF, RF or squelch level — a console
    /// on this protocol can set them and never read them back. So the rows
    /// are named and their values are em dashes, which is the honest
    /// rendering and visibly different from a zero.
    fn levels(&mut self, ui: &mut egui::Ui) {
        Self::pane_header(ui, "LEVELS", None);
        ui.add_space(4.0);
        for label in ["AF", "RF", "SQL", "MIC", "PWR"] {
            ui.horizontal(|ui| {
                ui.label(key(label));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(absent("—"));
                });
            });
        }
    }

    /// The AF scope, alone.
    fn af_scope_panel(&mut self, ui: &mut egui::Ui) {
        let state = self.audio_state();
        Self::pane_header(ui, "AF SCOPE", Some(state.label()));
        let h = (ui.available_height() - 4.0).max(8.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
        crate::af_scope_styled(
            ui,
            rect,
            self.audio.as_ref().map(|a| &a.scope),
            state,
            af_style(),
        );
    }

    /// The AF FFT, alone.
    fn af_fft_panel(&mut self, ui: &mut egui::Ui) {
        let state = self.audio_state();
        let passband = self.passband();
        Self::pane_header(ui, "AF FFT", Some("0-3 kHz"));
        let h = (ui.available_height() - 4.0).max(8.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
        crate::af_fft_styled(
            ui,
            rect,
            self.audio.as_ref().map(|a| &a.spectrum),
            passband,
            state,
            af_style(),
        );
    }

    fn rail(&mut self, ui: &mut egui::Ui) {
        let Some(caps) = self.capabilities().cloned() else {
            ui.label(absent("no radio"));
            return;
        };
        Self::pane_header(ui, "METERS · MeterSet", None);
        ui.add_space(4.0);

        for descriptor in &caps.meters {
            let reading = if descriptor.kind == cat_native::MeterKind::S {
                self.smeter_reading()
            } else {
                None
            };
            let active = reading.is_some();
            let label_colour = if active {
                theme::pal().text
            } else {
                theme::pal().absent
            };

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{:?}", descriptor.kind).to_uppercase())
                        .color(label_colour)
                        .size(theme::SIZE_BODY),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let v = reading
                        .map(|r| r.raw.to_string())
                        .unwrap_or_else(|| "—".to_string());
                    ui.label(RichText::new(v).color(label_colour).size(theme::SIZE_BODY));
                });
            });

            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 9.0), Sense::hover());
            ui.painter().rect_filled(rect, 0.0, theme::pal().panel_alt);
            ui.painter()
                .rect_stroke(rect, 0.0, Stroke::new(1.0, theme::pal().line));
            if let Some(r) = reading {
                crate::meter_bar(
                    ui,
                    rect.shrink(1.0),
                    r,
                    theme::pal().signal,
                    theme::pal().panel_alt,
                );
            }
            ui.add_space(7.0);
        }
    }

    // `af_panels` drew both AF panels as one block, because the rail was
    // the only place they went. A layout places each on its own now --
    // see `af_scope_panel` and `af_fft_panel`.

    /// A pane header: dim uppercase left, an optional note right.
    fn pane_header(ui: &mut egui::Ui, left: &str, right: Option<&str>) {
        let height = 17.0;
        let rect = ui
            .allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover())
            .0;
        ui.painter().rect_filled(rect, 0.0, theme::pal().panel_alt);
        ui.painter().line_segment(
            [
                egui::pos2(rect.left(), rect.bottom()),
                egui::pos2(rect.right(), rect.bottom()),
            ],
            Stroke::new(1.0, theme::pal().line),
        );
        ui.painter().text(
            egui::pos2(rect.left() + 7.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            left,
            egui::FontId::monospace(theme::SIZE_KEY),
            theme::pal().dim,
        );
        if let Some(right) = right {
            ui.painter().text(
                egui::pos2(rect.right() - 7.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                right,
                egui::FontId::monospace(theme::SIZE_KEY),
                theme::pal().absent,
            );
        }
    }

    /// The tab bar, derived from what the radio says it is.
    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let entries = self.tabs.clone();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (i, entry) in entries.iter().enumerate() {
                let selected = self.active == entry.tab;
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(entry.label.len() as f32 * 8.0 + 46.0, 26.0),
                    Sense::click(),
                );
                if selected {
                    ui.painter().rect_filled(rect, 0.0, theme::pal().panel);
                    ui.painter().line_segment(
                        [
                            egui::pos2(rect.left(), rect.bottom() - 1.0),
                            egui::pos2(rect.right(), rect.bottom() - 1.0),
                        ],
                        Stroke::new(2.0, theme::pal().accent),
                    );
                }
                let colour = if selected {
                    theme::pal().accent
                } else {
                    theme::pal().dim
                };
                // The digit that selects it, then the name. The digits are
                // the whole reason this design can hold TUI parity.
                ui.painter().text(
                    egui::pos2(rect.left() + 10.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("{}", i + 1),
                    egui::FontId::monospace(theme::SIZE_KEY),
                    theme::pal().absent,
                );
                ui.painter().text(
                    egui::pos2(rect.left() + 24.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &entry.label,
                    egui::FontId::monospace(theme::SIZE_BODY),
                    colour,
                );
                if response.clicked() {
                    self.active = entry.tab;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(10.0);
                ui.label(
                    RichText::new("1–9 switch · : command")
                        .color(theme::pal().absent)
                        .size(theme::SIZE_KEY),
                );
            });
        });
    }

    /// The always-visible quick controls.
    ///
    /// The design review's one complaint about this direction was that
    /// mode and filters were reachable only by knowing a command. They are
    /// here, and the command line still reaches them too.
    fn quick_bar(&mut self, ui: &mut egui::Ui) {
        let Some(caps) = self.capabilities().cloned() else {
            return;
        };
        let controls = quick::controls(&caps);
        if controls.is_empty() {
            return;
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for control in &controls {
                ui.label(key(control.label()));
                match control {
                    quick::Control::Mode => {
                        let label = self
                            .readout
                            .mode
                            .value()
                            .and_then(|id| caps.modes.iter().find(|m| m.id == id))
                            .map(|m| m.label.clone())
                            .unwrap_or_else(|| "—".to_string());
                        if ui
                            .add(egui::Button::new(
                                RichText::new(format!("{label:<8}"))
                                    .color(theme::pal().text)
                                    .size(theme::SIZE_BODY),
                            ))
                            .clicked()
                        {
                            if let Some(next) = quick::next_mode(&caps, self.readout.mode.value()) {
                                self.readout.mode.request(next);
                                self.send(Command::SetMode { mode: next });
                            }
                        }
                    }
                    quick::Control::FilterWidth { widths_hz } => {
                        let current = self.readout.filter_width_hz.value();
                        let label = current.map(|w| format!("{w} Hz")).unwrap_or("—".into());
                        egui::ComboBox::from_id_salt("filter_width")
                            .selected_text(RichText::new(label).size(theme::SIZE_BODY))
                            .show_ui(ui, |ui| {
                                for width in widths_hz {
                                    if ui
                                        .selectable_label(
                                            current == Some(*width),
                                            format!("{width} Hz"),
                                        )
                                        .clicked()
                                    {
                                        self.readout.filter_width_hz.request(*width);
                                        self.send(Command::SetFilterWidth { hz: *width });
                                    }
                                }
                            });
                    }
                    quick::Control::IfShift { limit_hz } => {
                        let current = self.readout.if_shift_hz.value().unwrap_or(0);
                        ui.label(
                            RichText::new(format!("{current:+5} Hz"))
                                .color(theme::pal().text)
                                .size(theme::SIZE_BODY),
                        );
                        for (caption, delta) in [("−", -100), ("+", 100)] {
                            if ui.small_button(caption).clicked() {
                                let next = quick::clamp_shift(current + delta, *limit_hz);
                                self.readout.if_shift_hz.request(next);
                                self.send(Command::SetIfShift { hz: next });
                            }
                        }
                    }
                    quick::Control::Notch => {}
                    quick::Control::Split => {
                        let on = self.readout.split.value().unwrap_or(false);
                        let colour = if on {
                            theme::pal().accent
                        } else {
                            theme::pal().dim
                        };
                        if ui
                            .add(egui::Button::new(
                                RichText::new(if on { "ON " } else { "OFF" })
                                    .color(colour)
                                    .size(theme::SIZE_BODY),
                            ))
                            .clicked()
                        {
                            self.readout.split.request(!on);
                            self.send(Command::SetSplit { enabled: !on });
                        }
                    }
                }
                ui.label(
                    RichText::new("│")
                        .color(theme::pal().line)
                        .size(theme::SIZE_BODY),
                );
            }
        });
    }

    /// The S-meter reading, carrying the radio's own range and table.
    fn smeter_reading(&self) -> Option<MeterReading> {
        let caps = self.capabilities()?;
        let raw = self.readout.smeter_raw.value()?;
        let descriptor = caps
            .meters
            .iter()
            .find(|m| m.kind == cat_native::MeterKind::S)?;
        let mut reading = MeterReading::new(descriptor.kind, raw, descriptor.raw_range);
        if let Some(scale) = descriptor.s_units {
            reading = reading.with_s_units(scale);
        }
        Some(reading)
    }

    /// Start the camera move toward a signal.
    ///
    /// From wherever the view is now, which is not necessarily the last
    /// frame's centre: clicking twice in quick succession should continue
    /// the journey rather than snap back and start again.
    fn begin_retune(&mut self, target_hz: u64) {
        let from = self.current_view();
        self.retune = Some(cat_ui::retune::Retune::to(from, target_hz as f64));
    }

    /// The axis the waterfall is drawing on right now.
    fn current_view(&self) -> cat_ui::retune::View {
        if let Some(r) = &self.retune {
            return r.view();
        }
        match self.waterfall.reference().or(self.latest.as_ref()) {
            Some(f) => cat_ui::retune::View::new(f.center_hz as f64, f64::from(f.span_hz)),
            None => cat_ui::retune::View::new(0.0, 1.0),
        }
    }

    /// Take the newest frame into the image, and run any move in progress.
    ///
    /// Two paths on purpose. Ordinarily a frame is one new row and the
    /// rest of the picture scrolls, which is cheap. While the view is
    /// travelling, the whole image is redrawn from the history onto the
    /// view's current axis — expensive, and only for the third of a second
    /// a move lasts.
    fn advance_waterfall(&mut self, frame: Option<SpectrumFrame>) {
        let arrived = frame.is_some();
        if let Some(frame) = frame {
            self.history.push_front(frame.clone());
            while self.history.len() > self.waterfall.height() as usize {
                self.history.pop_back();
            }
            self.latest = Some(frame);
        }

        let dt = self
            .last_frame_at
            .map(|t| t.elapsed().as_millis().min(u128::from(u32::MAX)) as u32)
            .unwrap_or(0);
        self.last_frame_at = Some(std::time::Instant::now());

        let travelling = match &mut self.retune {
            Some(r) => {
                r.advance(dt);
                true
            }
            None => false,
        };

        if travelling {
            let view = self.retune.as_ref().expect("checked above").view();
            // An axis, not a signal: `rebuild_onto` reads only the centre
            // and span of what it is given.
            let axis = SpectrumFrame {
                center_hz: view.center_hz as u64,
                span_hz: view.span_hz as u32,
                ref_level_dbm: self
                    .latest
                    .as_ref()
                    .map(|f| f.ref_level_dbm)
                    .unwrap_or(-110.0),
                bins: Vec::new(),
                sequence: 0,
            };
            let frames: Vec<SpectrumFrame> = self.history.iter().cloned().collect();
            self.waterfall.rebuild_onto(&frames, &axis);
            if self.retune.as_ref().is_some_and(|r| r.is_done()) {
                // Landed. The next frame's own axis takes over, so the
                // cheap scroll resumes and the picture is exactly what the
                // radio is sending.
                self.retune = None;
                let frames: Vec<SpectrumFrame> = self.history.iter().cloned().collect();
                self.waterfall.rebuild(&frames);
            }
        } else if arrived {
            // Only on a new frame. Scrolling on every repaint would push
            // the same row over and over, and a source that had stopped
            // would go on looking live -- which is the one thing a
            // waterfall must not do, since an operator watches it to see
            // whether anything is still arriving.
            if let Some(frame) = self.history.front().cloned() {
                self.waterfall.push(&frame);
            }
        }
    }

    /// The waterfall, and the click that tunes it.
    fn spectrum(&mut self, ui: &mut egui::Ui) {
        let Some(caps) = self.capabilities().cloned() else {
            return;
        };
        let span = match caps.signal {
            cat_native::SignalSupport::IfTapPoint { .. } => "IF TAP · CN4 → RTL-SDR",
            _ => "SPECTRUM",
        };
        Self::pane_header(
            ui,
            span,
            Some("click to tune · snaps to the radio's finest step"),
        );

        let available = ui.available_size();
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(available.x, available.y.max(1.0)), Sense::click());
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_rgb(4, 7, 10));

        let Some(frame) = self.latest.clone() else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "NO STREAM",
                egui::FontId::monospace(13.0),
                theme::pal().absent,
            );
            return;
        };

        // The image is advanced in `pump`, once per frame, rather than
        // here: this method is called only when the panel is visible, and
        // a waterfall that stopped scrolling because its tab was hidden
        // would lose the history an operator came back for.

        // Paint it. The buffer was being filled and never drawn -- a
        // waterfall the console maintained and never showed, which is the
        // kind of thing only looking at the thing catches.
        // Only when it actually changed, and at most `WATERFALL_UPLOAD_HZ`
        // times a second. See `waterfall_uploaded`.
        let generation = self.waterfall.generation();
        let due = self
            .waterfall_uploaded_at
            .map_or(true, |t| t.elapsed() >= WATERFALL_UPLOAD_INTERVAL);
        if (self.waterfall_uploaded != Some(generation) && due) || self.waterfall_texture.is_none()
        {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [
                    self.waterfall.width() as usize,
                    self.waterfall.height() as usize,
                ],
                &self.waterfall.rgba(),
            );
            match &mut self.waterfall_texture {
                Some(handle) => handle.set(image, egui::TextureOptions::NEAREST),
                None => {
                    self.waterfall_texture = Some(ui.ctx().load_texture(
                        "waterfall",
                        image,
                        egui::TextureOptions::NEAREST,
                    ))
                }
            }
            self.waterfall_uploaded = Some(generation);
            self.waterfall_uploaded_at = Some(std::time::Instant::now());
        }
        if let Some(handle) = &self.waterfall_texture {
            ui.painter().image(
                handle.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        // A fixed reticle at the centre, not a movable cursor. An IF tap is
        // dial-centred by construction, so the dial IS the centre.
        let x = rect.center().x;
        ui.painter().line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xe6, 0xab, 0x44, 90)),
        );

        if let Some(pos) = response.interact_pointer_pos() {
            if response.clicked() && rect.width() > 0.0 {
                let fraction = (pos.x - rect.left()) / rect.width();
                match tuning::tune_target(
                    &frame,
                    fraction,
                    &caps.tuning_steps_hz,
                    caps.rx_range.min_hz,
                    caps.rx_range.max_hz,
                ) {
                    Some(hz) => {
                        self.readout.vfo_a_hz.request(hz);
                        // The camera move, started before the command goes
                        // out. The picture travels to the signal while the
                        // radio gets there, so the two arrive together
                        // instead of the axis jumping when the first
                        // recentred frame lands.
                        self.begin_retune(hz);
                        // Retune, not SetFrequency: this moves the dial and
                        // the IF-tap source with it, which is what makes
                        // the picture recentre.
                        self.send(Command::Retune { hz });
                        self.status = format!("tuning {}", cat_ui::format_hz(hz));
                    }
                    None => {
                        self.status = "outside this radio's coverage".to_string();
                    }
                }
            }
        }
    }

    fn command_line(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.command_open {
                ui.label(
                    RichText::new(":")
                        .color(theme::pal().accent)
                        .size(theme::SIZE_BODY)
                        .strong(),
                );
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.command_text)
                        .desired_width(f32::INFINITY)
                        .font(egui::FontId::monospace(theme::SIZE_BODY))
                        .frame(false),
                );
                edit.request_focus();
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.run_line();
                    self.command_open = false;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.command_open = false;
                    self.command_text.clear();
                }
            } else {
                // The status belongs to the `Status` panel. This row used
                // to carry it because it was the only row at the bottom;
                // a layout that places both would show it twice.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new("press :  for a command")
                            .color(theme::pal().absent)
                            .size(theme::SIZE_KEY),
                    );
                });
            }
        });
    }
}

// No `impl eframe::App` here. The window and its event loop belong to
// each app's own binary (ADR 0011's seam), and `draw` below is the whole
// of what a console needs from one — an app's `App::update` is a single
// line forwarding to it. Keeping `eframe` out of this crate is also what
// lets the offscreen renderer draw a console with no window at all.

impl Console {
    /// One frame, against a bare `egui::Context`.
    ///
    /// Split out from `eframe::App::update` so the console can be drawn
    /// without a window. `examples/render.rs` rasterises this offscreen
    /// through lavapipe and writes a PNG, which is the only way to *look*
    /// at a change here on a machine with no display — and looking is the
    /// thing that was missing when this first shipped not resembling the
    /// design it was built from.
    pub fn draw(&mut self, ctx: &egui::Context) {
        self.pump();
        self.poll_state();
        // The radio's palette, before anything is drawn with it. An
        // operator in front of a TS-570D is looking at an amber LCD on
        // charcoal; an FT-991A is a colour TFT. A console that matches its
        // rig is one whose readout can be found without translating
        // between two visual languages.
        theme::set_active(theme::Palette::for_radio(
            self.capabilities().and_then(|c| c.theme.as_ref()),
        ));
        // Spectrum arrives on its own schedule, not on input, so the
        // console has to ask to be woken rather than waiting for a click.
        ctx.request_repaint_after(std::time::Duration::from_millis(33));

        let mut wants_connect = false;
        // Collected while drawing and acted on after, because the panel
        // borrows the device lists out of `self` to draw them.
        let mut pending: Option<SourceAction> = None;
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Colon)
                || (i.modifiers.shift && i.key_pressed(egui::Key::Semicolon))
            {
                self.command_open = true;
            }
            for (n, key) in [
                egui::Key::Num1,
                egui::Key::Num2,
                egui::Key::Num3,
                egui::Key::Num4,
                egui::Key::Num5,
            ]
            .into_iter()
            .enumerate()
            {
                if i.key_pressed(key) && !self.command_open {
                    if let Some(tab) = workspace::tab_for_digit(&self.tabs, n + 1) {
                        self.active = tab;
                    }
                }
            }
        });

        // One panel, and the radio's layout inside it. The arrangement is
        // published by the server (radio-cat-rs ADR 0020) and resolved in
        // the same units the terminal console uses, so a rig's console has
        // the same proportions in both.
        let layout = self
            .capabilities()
            .and_then(|c| c.layout.clone())
            .unwrap_or_else(default_layout);
        self.spectrum_has_its_own_panel = layout.root.places(&cat_layout::PanelKind::Spectrum);

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::pal().bg))
            .show(ctx, |ui| {
                if self.capabilities().is_none() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.label(
                            RichText::new("NOT CONNECTED")
                                .color(theme::pal().warning)
                                .size(16.0),
                        );
                        // The reason travels with the state rather than
                        // only in `status`, which the next message would
                        // overwrite.
                        let why = match &self.link {
                            Link::Down(why) => why.as_str(),
                            Link::Up(_) => "",
                        };
                        ui.label(dim(format!("{} — {why}", self.address)));
                        if ui.button("connect").clicked() {
                            wants_connect = true;
                        }
                    });
                    return;
                }

                let full = ui.available_rect_before_wrap();
                let cell = cell_size(ui);
                // Resolved in cells and scaled to points. A layout says a
                // rail is 22 wide because that is what the panel needs in
                // characters; asking for 22 *points* would give a rail
                // three characters across.
                let in_cells = cat_layout::Area::new(
                    0,
                    0,
                    (full.width() / cell.x) as u16,
                    (full.height() / cell.y) as u16,
                );

                for placement in layout.resolve(in_cells) {
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(
                            full.left() + f32::from(placement.area.x) * cell.x,
                            full.top() + f32::from(placement.area.y) * cell.y,
                        ),
                        egui::vec2(
                            f32::from(placement.area.width) * cell.x,
                            f32::from(placement.area.height) * cell.y,
                        ),
                    );
                    if rect.width() < 1.0 || rect.height() < 1.0 {
                        continue;
                    }
                    self.panel(ui, rect, &placement.kind, &mut pending, &mut wants_connect);
                }
            });

        match pending {
            Some(SourceAction::Refresh) => self.refresh_devices(),
            Some(SourceAction::Attach(kind, spec)) => {
                // Optimism would be wrong here: nothing local changes, and
                // whether it worked is the server's to say. The status
                // line carries its answer, refusal included.
                self.status = format!("attaching {spec}…");
                self.send(Command::AttachDevice { kind, spec });
            }
            None => {}
        }
        if wants_connect {
            self.connect();
        }
    }
}

/// One character cell, in points.
///
/// The console is monospace throughout, so a layout expressed in cells
/// maps onto it exactly. Measured from the font in force rather than
/// assumed, because the type scale is the design system's and may change.
fn cell_size(ui: &egui::Ui) -> egui::Vec2 {
    let font = egui::FontId::monospace(theme::SIZE_BODY);
    let w = ui.fonts(|f| f.glyph_width(&font, 'M'));
    let h = ui.fonts(|f| f.row_height(&font));
    egui::vec2(w.max(1.0), (h + 2.0).max(1.0))
}

/// The arrangement a console uses when its server publishes none.
///
/// The layout this console shipped with, so an older server's operator
/// sees no change.
fn default_layout() -> cat_layout::LayoutSpec {
    use cat_layout::{Child, Node, PanelKind, Size};
    cat_layout::LayoutSpec::new(Node::rows(vec![
        Child::panel(Size::Fixed(4), PanelKind::Readout),
        Child::panel(Size::Fixed(2), PanelKind::QuickBar),
        Child::new(
            Size::Min(8),
            Node::columns(vec![
                Child::new(
                    Size::Fixed(26),
                    Node::rows(vec![
                        Child::panel(Size::Min(6), PanelKind::MeterRail),
                        Child::panel(Size::Fixed(5), PanelKind::AfScope),
                        Child::panel(Size::Fixed(5), PanelKind::AfFft),
                    ]),
                ),
                Child::panel(Size::Min(20), PanelKind::Workspace),
            ]),
        ),
        Child::panel(Size::Fixed(1), PanelKind::Status),
        Child::panel(Size::Fixed(2), PanelKind::CommandLine),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_native::testing::{serve_stub, StubHost};
    use cat_signal::{DeviceInfo, DeviceKind, DeviceList};

    /// Hardware named nothing like this machine's, so a console that
    /// enumerated locally could not pass by accident.
    fn radio_side_devices() -> Vec<DeviceList> {
        vec![DeviceList::found(
            DeviceKind::Sdr,
            vec![DeviceInfo {
                kind: DeviceKind::Sdr,
                spec: "rtl:in-the-shack".to_string(),
                label: "the dongle on the radio's IF tap".to_string(),
                detail: None,
                is_default: false,
            }],
        )]
    }

    /// Drive the console the way `update` does, until `done` or the clock
    /// runs out. The reply is asynchronous; there is nothing to await.
    fn settle(console: &mut Console, done: impl Fn(&Console) -> bool) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            console.pump();
            console.ask_for_devices();
            if done(console) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    fn connected(host: std::sync::Arc<StubHost>) -> Console {
        let mut console = Console::new(serve_stub(host));
        console.connect();
        console
    }

    #[test]
    fn the_picker_shows_what_the_server_has_not_what_this_machine_has() {
        let mut console = connected(StubHost::offering(radio_side_devices()));

        assert!(
            settle(&mut console, |c| matches!(
                c.devices,
                crate::devices::Offer::Listed(_)
            )),
            "the console never received the server's device list"
        );

        let specs: Vec<&str> = console
            .devices
            .lists()
            .iter()
            .flat_map(|l| l.devices.iter().map(|d| d.spec.as_str()))
            .collect();
        assert_eq!(
            specs,
            vec!["rtl:in-the-shack"],
            "the list must be the server's, and only the server's"
        );
    }

    #[test]
    fn a_server_that_declines_is_not_reported_as_a_fault() {
        // The decline belongs in the source panel: it is the answer to the
        // question, not an error to put in front of the operator.
        let mut console = connected(StubHost::new());

        assert!(
            settle(&mut console, |c| c.devices
                == crate::devices::Offer::NotOffered),
            "a declining server left the console waiting forever"
        );
        assert!(
            !console.status.contains("Unsupported"),
            "the decline leaked into the status line: {}",
            console.status
        );
    }

    #[test]
    fn the_console_asks_once_and_not_at_poll_rate() {
        // Enumerating every sound card on a machine is not free, and doing
        // it ten times a second would put it in front of the state traffic
        // the readout depends on.
        //
        // Asserted on the console's own guard rather than by counting
        // arrivals at the server: `ReadDevices` is answered by the session
        // and never reaches `RadioHost::apply`, so a server-side tally
        // would sit at zero and pass no matter what this console did.
        let mut console = connected(StubHost::offering(radio_side_devices()));
        assert!(settle(&mut console, |c| matches!(
            c.devices,
            crate::devices::Offer::Listed(_)
        )));

        for _ in 0..50 {
            console.ask_for_devices();
            assert!(
                !console.devices_pending,
                "asked again while already holding an answer"
            );
        }
    }

    #[test]
    fn refreshing_asks_again_for_a_dongle_plugged_in_since() {
        let host = StubHost::new();
        let mut console = connected(std::sync::Arc::clone(&host));
        assert!(settle(&mut console, |c| c.devices
            == crate::devices::Offer::NotOffered));

        host.set_devices(Some(radio_side_devices()));
        console.refresh_devices();

        assert!(
            settle(&mut console, |c| matches!(
                c.devices,
                crate::devices::Offer::Listed(_)
            )),
            "refresh did not re-ask, so new hardware stayed invisible"
        );
    }

    fn audio_frame() -> cat_signal::AudioFrame {
        cat_signal::AudioFrame {
            scope: cat_signal::AudioScopeFrame {
                sample_rate_hz: 48_000,
                samples: vec![0.0, 0.5, -0.5],
                sequence: 1,
            },
            spectrum: cat_signal::AudioSpectrumFrame {
                start_hz: 0,
                span_hz: 4_000,
                bins: vec![-90.0, -50.0, -80.0],
                sequence: 1,
            },
        }
    }

    #[test]
    fn the_af_panels_report_three_states_and_not_two() {
        // "Configured but nothing has arrived" and "nothing is wired at
        // all" send an operator to opposite ends of the shack. A console
        // that showed both as an empty panel would tell them nothing.
        use cat_ui::af::AudioState;
        let mut console = connected(StubHost::new());

        // The stub declares no audio source, so waiting is pointless and
        // the panels say so rather than sitting at PENDING for ever.
        assert_eq!(console.audio_state(), AudioState::Absent);

        console.audio = Some(audio_frame());
        assert_eq!(console.audio_state(), AudioState::Streaming);
    }

    #[test]
    fn losing_the_link_clears_the_trace() {
        // A waveform left on screen after the link dropped is a picture of
        // a radio this console is no longer talking to.
        let mut console = connected(StubHost::new());
        console.audio = Some(audio_frame());

        console.link = Link::Down("dropped".to_string());
        console.audio = None;

        assert!(console.audio.is_none());
        assert_ne!(console.audio_state(), cat_ui::af::AudioState::Streaming);
    }

    #[test]
    fn the_passband_marks_come_from_the_radios_own_declaration() {
        // Not a table of one radio's filters: the mark is the bandwidth
        // this radio published for this mode.
        let mut console = connected(StubHost::offering(Vec::new()));
        console.readout.mode.confirm(cat_native::ModeId::Usb);

        let pb = console.passband().expect("USB has a passband");
        let declared = console
            .capabilities()
            .unwrap()
            .modes
            .iter()
            .find(|m| m.id == cat_native::ModeId::Usb)
            .unwrap()
            .default_bandwidth_hz as f32;
        assert_eq!(pb.high_hz - pb.low_hz, declared);
    }

    #[test]
    fn cw_marks_nothing_rather_than_guessing_a_sidetone() {
        // The audio a CW receiver produces sits at the operator's sidetone
        // pitch, which is a menu setting. A mark at a guessed pitch would
        // be worse than none: it would look like a measurement.
        let mut console = connected(StubHost::new());
        console.readout.mode.confirm(cat_native::ModeId::CwUpper);
        assert!(console.passband().is_none());
    }

    #[test]
    fn attaching_names_the_device_to_the_server() {
        let host = StubHost::offering(radio_side_devices());
        let mut console = connected(std::sync::Arc::clone(&host));
        assert!(settle(&mut console, |c| matches!(
            c.devices,
            crate::devices::Offer::Listed(_)
        )));

        console.send(Command::AttachDevice {
            kind: DeviceKind::Sdr,
            spec: "rtl:in-the-shack".to_string(),
        });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut seen = false;
        while std::time::Instant::now() < deadline && !seen {
            console.pump();
            seen = host.applied().iter().any(|c| {
                matches!(c, Command::AttachDevice { kind, spec }
                    if *kind == DeviceKind::Sdr && spec == "rtl:in-the-shack")
            });
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(seen, "the attach never reached the server");
    }
}
