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

//! The console's view of a radio.
//!
//! Moved here when a second radio needed the same console. It was already
//! radio-generic in shape — a dial, a mode, meters, gains — and keeping a
//! copy per radio would have meant two structs that agreed until one was
//! edited.

/// What a console displays, filled by whatever is talking to the radio.
///
/// One struct for every radio, because a console draws the same things
/// whatever is on the other end of the wire: a dial, a mode, meters,
/// gains, the receiver's switches. A radio fills the fields it can read
/// and leaves the rest at their defaults — an FT-991A's poll fills
/// fifteen of these and a TS-570D's fills most of them, and the console
/// draws what it was given without knowing which radio it came from.
///
/// # Not read yet is not zero
///
/// Fields the ribbon shows are `Option`, and that distinction is the
/// point: `None` draws a dimmed placeholder, where a `0` would be a
/// confident claim about a radio nobody has asked.
#[derive(Debug, Clone)]
pub struct RadioDisplay {
    // --- Primary (from IF / get_information) ---
    pub vfo_a_hz: u64,
    pub vfo_b_hz: u64,
    pub mode: String,
    /// The same mode, as the shared vocabulary names it.
    ///
    /// Beside the label rather than instead of it: the label is what an
    /// operator reads and is the radio's own ("CW-R", "DATA-U"), while
    /// this is what code decides with. Parsing the label back into a mode
    /// — which is what the console used to do — breaks the moment a radio
    /// spells one differently, and every radio spells at least one
    /// differently.
    pub mode_id: Option<cat_framework::capabilities::ModeId>,
    pub tx: bool,
    pub rit: bool,
    pub xit: bool,
    pub rit_xit_offset_hz: i32,
    pub split: bool,
    pub scan: bool,
    pub memory_channel: u8,
    pub memory_mode: bool,

    // --- Meters ---
    pub smeter: u16,

    // --- Gains / levels ---
    /// Whether the rail's settings were actually read from the radio.
    ///
    /// **False means every field below is a placeholder, not a reading.**
    /// The console protocol's `RadioState` carries the dial, mode, split,
    /// TX, memory channel, IF shift, filter width and meters -- and none
    /// of AF, RF, SQL, MIC, PWR, AGC, NB, NR, PRE, ATT, PROC, VOX or LOCK.
    /// A console attached over the network therefore knows none of them.
    ///
    /// Before this flag they were drawn from `Default`, so a network
    /// console displayed `AF 200` at a radio reading `AG034` and `PRE off`
    /// at a radio with its preamp on -- confidently, and indistinguishably
    /// from a real reading. A dash is the honest rendering; the direct
    /// serial console, which polls every one of these, sets this true.
    pub levels_known: bool,
    pub af_gain: u8,
    pub rf_gain: u8,
    pub squelch: u8,
    pub mic_gain: u8,
    pub power_pct: u8,
    pub agc: u8,

    // --- Receiver features ---
    pub noise_blanker: bool,
    pub noise_reduction: u8,
    pub preamp: bool,
    pub attenuator: bool,
    pub speech_processor: bool,
    pub beat_cancel: u8,

    // --- Transmit ---
    pub vox: bool,
    pub antenna: u8,

    // --- VFO routing ---
    pub rx_vfo: u8,
    pub tx_vfo: u8,

    // --- Tone ---
    pub ctcss: bool,
    pub freq_lock: bool,
    pub fine_step: bool,

    // --- Filter, for the quick-settings ribbon ---
    //
    // `Option` rather than a default, and that distinction is the point.
    // These are the three cells the design puts in the ribbon that the
    // *GUI* cannot fill: the native protocol has no field for filter width,
    // IF shift or notch (designer findings B2), so a network console can
    // send them and never read them back. The TUI talks CAT directly and
    // `Ts570dState` carries all three, so here they are genuinely
    // knowable -- and `None` means "not read yet", which the ribbon draws
    // as a dimmed placeholder rather than as a confident zero.
    pub filter_width_hz: Option<u16>,
    pub if_shift_hz: Option<i16>,
    pub notch: Option<bool>,

    // --- Poll errors (from most recent poll cycle) ---
    pub poll_errors: Vec<String>,

    // --- Connection health ---
    /// `false` when the radio has been unresponsive for 3 consecutive poll cycles.
    pub connected: bool,

    /// `true` from startup until the first successful poll cycle completes.
    /// Used to show "Connecting..." instead of "CONNECTION LOST" on startup.
    pub initializing: bool,

    // --- Optional capabilities ---
    /// `true` when the port this console is connected through actually has
    /// DTR/RTS handshake lines, so the `[P]` PTT-line item is offered.
    ///
    /// Decided once, from `radio::PttLine::ptt_line_available`, before the
    /// radio task starts — not re-derived per poll cycle, so the menu cannot
    /// flicker when a probe happens to land mid-CAT-command. The radio task
    /// sends a fresh `RadioDisplay` every cycle and does not know about this
    /// field; `terminal::ui_task` restamps it. See `docs/adr/0010`.
    pub ptt_line_available: bool,
}

impl Default for RadioDisplay {
    fn default() -> Self {
        Self {
            vfo_a_hz: 14_000_000,
            vfo_b_hz: 14_100_000,
            mode: "USB".to_string(),
            // No mode read yet. The label above is a placeholder a console
            // draws before the first poll; this stays `None` so nothing
            // derives a passband from a guess.
            mode_id: None,
            tx: false,
            rit: false,
            xit: false,
            rit_xit_offset_hz: 0,
            split: false,
            scan: false,
            memory_channel: 0,
            memory_mode: false,
            smeter: 0,
            levels_known: false,
            af_gain: 200,
            rf_gain: 255,
            squelch: 0,
            mic_gain: 50,
            power_pct: 100,
            agc: 2,
            noise_blanker: false,
            noise_reduction: 0,
            preamp: false,
            attenuator: false,
            speech_processor: false,
            beat_cancel: 0,
            vox: false,
            antenna: 1,
            rx_vfo: 0,
            tx_vfo: 0,
            ctcss: false,
            freq_lock: false,
            fine_step: false,
            filter_width_hz: None,
            if_shift_hz: None,
            notch: None,
            poll_errors: Vec::new(),
            connected: true,
            initializing: true,
            ptt_line_available: false,
        }
    }
}
