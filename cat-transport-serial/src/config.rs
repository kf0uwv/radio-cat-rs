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

//! Shared, platform-neutral serial port configuration types.
//!
//! `SerialConfig`/`Parity`/`FlowControl` are pure data — no platform-specific
//! code — so they live in this ungated module instead of being duplicated
//! between the Linux `io_uring` backend and a future Windows backend. Moved
//! out of `io_uring.rs` verbatim (same fields, same `Default` impl, same doc
//! comments) per `docs/adr/0004-windows-serial-backend.md` §2 / §"Consequences"
//! ("existing Linux tests are expected to pass unchanged").

/// Serial port configuration
#[derive(Debug, Clone)]
pub struct SerialConfig {
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: Parity,
    pub flow_control: FlowControl,
    /// Whether [`crate::SerialPort::open`] asserts RTS high at construction
    /// time (via [`cat_transport_core::ModemControlLines::set_rts`]`(true)`).
    /// Defaults to `true`, preserving this crate's historical
    /// unconditional-assert behavior exactly. Set `false` if a consumer
    /// wants full runtime control over RTS from the moment the port opens —
    /// e.g. RTS-keyed CW/PTT, where idle/asserted polarity matters and this
    /// crate asserting it first would be an unwanted side effect.
    pub initial_rts: bool,
    /// Same as `initial_rts`, for DTR. **Defaults to `false`.**
    ///
    /// # Why this default is not `true`
    ///
    /// On a great many amateur stations DTR *is* the PTT line. Asserting
    /// it at open time keys the transmitter -- into whatever load and on
    /// whatever frequency the radio happens to be on -- as a side effect
    /// of a program starting up.
    ///
    /// This defaulted to `true` until 2026-09-09, to "preserve historical
    /// behaviour". On that day a diagnostic script opened `/dev/ttyUSB0`
    /// on a TS-570D whose PTT is DTR-keyed, and keyed the radio; the same
    /// default is what a caller who has not thought about modem lines
    /// gets. `ts570d` passes `false` at all six of its open sites and
    /// documents why in `port_guard.rs`; `ft991a` and `ic7100` passed
    /// nothing and were silently getting the asserting behaviour.
    ///
    /// The two failures are not comparable. If a station genuinely needs
    /// DTR high -- some interfaces take their power from it -- the cost of
    /// this default is that CAT does not work until someone sets it, which
    /// is visible in the first second and fixed by one field. The cost of
    /// the other default being wrong is an unattended transmission.
    ///
    /// So: opt *in* to asserting DTR, and say why where you do.
    pub initial_dtr: bool,
}

#[derive(Debug, Clone)]
pub enum Parity {
    None,
    Even,
    Odd,
}

#[derive(Debug, Clone)]
pub enum FlowControl {
    None,
    Software,
    Hardware,
}

impl Default for SerialConfig {
    fn default() -> Self {
        Self {
            baud_rate: 9600,
            data_bits: 8,
            stop_bits: 2,
            parity: Parity::None,
            flow_control: FlowControl::None,
            initial_rts: true,
            // See the field's own doc: DTR is PTT on many stations.
            initial_dtr: false,
        }
    }
}
