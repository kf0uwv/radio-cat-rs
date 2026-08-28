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

//! The rigctld wire protocol itself — command dispatch, `\dump_state`, and
//! line buffering — with zero I/O of its own, so it is identical on every
//! platform. `crate::rigctl` (Linux, `monoio`-based) and
//! `crate::rigctl_windows` (`std`-based, genuine OS threads) each own their
//! own accept loop and byte-level reading, but both call straight through
//! to [`dispatch`] and both buffer incoming bytes with [`LineSplitter`] —
//! extracted here so the wire-format logic, and in particular
//! [`MAX_LINE_LEN`]'s enforcement (previously the site of a real regression
//! — see that constant's doc), exists in exactly one place instead of two
//! platform-specific copies that could silently drift apart.
//!
//! Moved out of `crate::rigctl` verbatim (this module's `dispatch`/
//! `dump_state`/`RPRT_*` are byte-for-byte what `crate::rigctl` used to
//! define directly) when this crate grew a Windows backend — see
//! `docs/adr/0006-windows-network-transport.md`'s follow-up note.

use crate::RigctlRadio;

/// A rigctld error report line: `RPRT <code>`. `-1` is used for every
/// failure here (this bridge does not attempt to reproduce Hamlib's
/// specific per-cause negative error codes — WSJT-X's own error handling
/// only distinguishes zero from non-zero).
pub(crate) const RPRT_OK: &str = "RPRT 0\n";
pub(crate) const RPRT_ERR: &str = "RPRT -1\n";

/// Dispatch one rigctld command line against `radio`, returning the full
/// response text (already newline-terminated). Generic over any
/// [`RigctlRadio`] implementation, so this works against a real radio
/// crate's typed client in production and a small in-crate fake in tests
/// without any test-specific branching.
///
/// `radio`'s `Error` type is never displayed here — rigctld's `RPRT -1`
/// convention carries no error text on the wire, so every failure
/// collapses to the same generic report regardless of cause.
pub(crate) async fn dispatch<R: RigctlRadio>(radio: &mut R, line: &str) -> String {
    let mut parts = line.split_whitespace();
    let Some(cmd) = parts.next() else {
        return RPRT_ERR.to_string();
    };
    let args: Vec<&str> = parts.collect();

    match cmd {
        "f" => match radio.get_vfo_a_hz().await {
            Ok(hz) => format!("{hz}\n"),
            Err(_) => RPRT_ERR.to_string(),
        },
        "F" => {
            // Hamlib's `netrigctl_set_freq()` sends `freq_t` (a C `double`)
            // formatted as `%f`, e.g. `F 14074000.000000` — never a bare
            // integer — so this must parse as a float first (confirmed
            // against a real Hamlib `rigctl -m 2` client, which sent
            // exactly this and got `RPRT -1` back before this fix).
            let Some(hz) = args
                .first()
                .and_then(|s| s.parse::<f64>().ok())
                .map(|hz| hz.round() as u64)
            else {
                return RPRT_ERR.to_string();
            };
            match radio.set_vfo_a_hz(hz).await {
                Ok(()) => RPRT_OK.to_string(),
                Err(_) => RPRT_ERR.to_string(),
            }
        }
        "m" => match radio.get_mode().await {
            // Passband is always reported as `0` ("use the rig's current
            // default") rather than a real bandwidth — see module docs on
            // why filter-width resolution is out of scope for this bridge.
            Ok(mode) => format!("{}\n0\n", R::hamlib_mode_name(mode)),
            Err(_) => RPRT_ERR.to_string(),
        },
        "M" => {
            let Some(mode_name) = args.first() else {
                return RPRT_ERR.to_string();
            };
            match R::hamlib_mode_from_name(mode_name) {
                Some(mode) => match radio.set_mode(mode).await {
                    Ok(()) => RPRT_OK.to_string(),
                    Err(_) => RPRT_ERR.to_string(),
                },
                None => RPRT_ERR.to_string(),
            }
        }
        "t" => match radio.get_transmitting().await {
            Ok(false) => "0\n".to_string(),
            Ok(true) => "1\n".to_string(),
            Err(_) => RPRT_ERR.to_string(),
        },
        "T" => {
            let result = match args.first() {
                Some(&"1") => radio.transmit().await,
                Some(&"0") => radio.receive().await,
                _ => return RPRT_ERR.to_string(),
            };
            match result {
                Ok(()) => RPRT_OK.to_string(),
                Err(_) => RPRT_ERR.to_string(),
            }
        }
        // No VFO-B/split concept on `RigctlRadio` today — always report
        // the single VFO this bridge controls (see module docs).
        "v" => "VFOA\n".to_string(),
        "\\chk_vfo" => "0\n".to_string(),
        "\\dump_state" => dump_state::<R>(),
        _ => RPRT_ERR.to_string(),
    }
}

/// The `\dump_state` capability handshake Hamlib's `netrigctl.c` client
/// sends once, right after connecting. Frequency range drawn from
/// `R::freq_range_hz()` (real per-radio values, supplied by the caller's
/// [`RigctlRadio`] impl) — unlike the invented mode/vfo/antenna bitmasks
/// below, which stay generic placeholders for every radio (see module
/// docs).
fn dump_state<R: RigctlRadio>() -> String {
    match R::capabilities() {
        Some(caps) => dump_state_from_capabilities(caps),
        // No capabilities published: the historical placeholder tail,
        // byte-for-byte. Existing radios must not change behaviour just
        // because this function grew a second branch.
        None => {
            let (min_hz, max_hz) = R::freq_range_hz();
            dump_state_text(min_hz, max_hz, "-1 10\n0 0\n", "-1 2400\n0 0\n", 1200, 1200)
        }
    }
}

/// Generate the `\dump_state` reply from a radio's own description.
///
/// ADR 0010 §6: "a radio gains rigctl support by describing itself, not by
/// writing a bridge." Everything here that used to be a hand-written
/// placeholder — the frequency range, the tuning-step list, the filter
/// widths, the RIT/XIT limits — now comes from the capability model.
///
/// What is deliberately **not** generated: the mode/vfo/antenna bitmasks
/// stay `-1` (all bits set). Hamlib's per-mode bit assignments are a fixed
/// external vocabulary that does not map cleanly onto `ModeId`, and being
/// permissive there is what the existing, field-tested behaviour does.
/// Narrowing it would be a behaviour change to the compatibility layer,
/// which is precisely what this task must not do.
pub(crate) fn dump_state_from_capabilities(
    caps: &cat_framework::capabilities::RadioCapabilities,
) -> String {
    let mut steps = String::new();
    for step in caps.tuning_steps_hz {
        steps.push_str(&format!("-1 {step}\n"));
    }
    if steps.is_empty() {
        steps.push_str("-1 10\n");
    }
    steps.push_str("0 0\n");

    let mut filters = String::new();
    if let Some(widths) = caps.filters.widths_hz {
        for width in widths {
            filters.push_str(&format!("-1 {width}\n"));
        }
    }
    if filters.is_empty() {
        // A radio with no CAT-selectable widths still has to say
        // something here; a conservative, universally-legal SSB bandwidth
        // is what the placeholder always used.
        filters.push_str("-1 2400\n");
    }
    filters.push_str("0 0\n");

    // Hamlib wants a single magnitude for each. The capability model
    // carries them as symmetric limits already.
    let rit = caps.vfos.rit_hz.unwrap_or(0).unsigned_abs();
    let xit = caps.vfos.xit_hz.unwrap_or(0).unsigned_abs();

    dump_state_text(
        caps.rx_range.min_hz,
        caps.rx_range.max_hz,
        &steps,
        &filters,
        rit,
        xit,
    )
}

/// The `\dump_state` reply's fixed shape, with the variable parts injected.
///
/// One function so that both the generated and the placeholder paths
/// produce **structurally identical** replies, differing only in values.
/// The field count is the thing that must never drift (see below), and it
/// is now impossible for the two paths to disagree about it.
fn dump_state_text(
    min_hz: u64,
    max_hz: u64,
    tuning_steps: &str,
    filters: &str,
    rit_hz: u32,
    xit_hz: u32,
) -> String {
    let mut s = String::new();
    // Protocol marker, rig model (0 = generic/unknown), ITU region (0 =
    // unspecified) — the three fixed header lines every `dump_state` reply
    // starts with.
    s.push_str("0\n0\n0\n");

    // RX range list: one row of `start end modes low_power high_power vfo
    // ant`, terminated by an all-zero sentinel row. `modes`/`vfo`/`ant` use
    // `-1` (all bits set) rather than an attempt at Hamlib's exact per-mode
    // bit assignments — deliberately permissive, see module docs.
    s.push_str(&format!(
        "{min_hz} {max_hz} -1 -1 -1 -1 -1\n0 0 0 0 0 0 0\n"
    ));
    // TX range list — same shape, terminated the same way.
    s.push_str(&format!(
        "{min_hz} {max_hz} -1 -1 -1 -1 -1\n0 0 0 0 0 0 0\n"
    ));
    s.push_str(tuning_steps);
    s.push_str(filters);

    // Fixed capability tail: max RIT/XIT/IF-shift (Hz), announces bitmask,
    // preamp levels list (dB, zero-terminated), attenuator levels list (dB,
    // zero-terminated), then six zero (hex) capability bitmasks —
    // `has_get_func`/`has_set_func`/`has_get_level`/`has_set_level`/
    // `has_get_parm`/`has_set_parm`. This bridge does not claim any Hamlib
    // "func"/"level"/"parm" capabilities beyond plain freq/mode/ptt, so all
    // six are `0`. All six are required: `netrigctl_open()` (Hamlib's
    // `rigs/dummy/netrigctl.c`) reads exactly this many capability lines
    // before returning for protocol version 0, and blocks waiting for the
    // rest if fewer are sent (confirmed against a real WSJT-X/Hamlib
    // client, which timed out here when only four were sent).
    s.push_str(&format!(
        "{rit_hz}\n0\n{xit_hz}\n0\n0\n0\n0x0\n0x0\n0x0\n0x0\n0x0\n0x0\n"
    ));
    s
}

/// Maximum buffered line length, in bytes, before a client that never
/// sends `\n` is treated as misbehaving and disconnected. Without this
/// bound, [`LineSplitter`] would grow its buffer without limit for such a
/// client — an unbounded-memory-growth DoS. Restored here after this
/// crate's initial extraction from `ts570d`/`ft991a` silently dropped it
/// (found by an independent post-migration review, reproduced live: a raw
/// socket sending 600 bytes with no newline hung the connection forever
/// with no error, where the pre-extraction code in both apps closed it
/// with `InvalidData` after 512 bytes) — restored as ts570d's original fix
/// had it (512 bytes), not reinvented. Enforced identically on both
/// platforms since both `crate::rigctl` and `crate::rigctl_windows` share
/// this one `LineSplitter`.
pub(crate) const MAX_LINE_LEN: usize = 512;

/// Buffers partial reads and splits them into `\n`-terminated lines (with
/// an optional trailing `\r` trimmed) — rigctld's protocol is line-based
/// text, not `cat-transport-tcp`'s length-prefixed binary framing, so
/// neither that crate's frame codec nor `cat-server`'s accept loops apply
/// here. Pure buffering, no I/O of its own — `crate::rigctl`'s async
/// `AsyncReadRent`-based reader and `crate::rigctl_windows`'s blocking
/// `std::io::Read`-based reader each feed it bytes from whichever
/// platform's transport they're built on, but both get identical
/// line-splitting/length-limit behavior from this one implementation.
pub(crate) struct LineSplitter {
    buf: Vec<u8>,
}

impl LineSplitter {
    pub(crate) fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Append newly-read bytes to the buffer.
    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Pop one complete line out of the buffer (trailing `\n`/`\r`
    /// trimmed), if one is present.
    pub(crate) fn try_take_line(&mut self) -> Option<String> {
        let pos = self.buf.iter().position(|&b| b == b'\n')?;
        let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
        line.pop(); // trailing '\n'
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        Some(String::from_utf8_lossy(&line).into_owned())
    }

    /// Whether the buffer holds more than [`MAX_LINE_LEN`] bytes with no
    /// newline in sight yet — the caller should treat this as a misbehaving
    /// client and disconnect.
    pub(crate) fn is_over_limit(&self) -> bool {
        self.buf.len() > MAX_LINE_LEN
    }

    /// On a clean disconnect: if a partial line remains in the buffer,
    /// return it once as a final "line" (mirrors how a real terminal
    /// client's last unterminated command would still be worth attempting)
    /// rather than silently discarding it.
    pub(crate) fn take_final_partial_line(&mut self) -> Option<String> {
        if self.buf.is_empty() {
            return None;
        }
        let line = std::mem::take(&mut self.buf);
        Some(String::from_utf8_lossy(&line).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `RigctlRadio` impl mirroring `crate::rigctl`'s own test fake —
    /// duplicated here (not shared via `pub(crate)` across a `#[cfg(test)]`
    /// boundary, which modules can't reach into each other for) since each
    /// module's tests need a concrete `R` to call `dispatch`/`dump_state`
    /// against and this one is small.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FakeMode {
        Usb,
        Lsb,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct FakeError;

    struct FakeRadio {
        vfo_hz: u64,
        mode: FakeMode,
        transmitting: bool,
        fail_next: bool,
    }

    impl FakeRadio {
        fn new() -> Self {
            Self {
                vfo_hz: 14_250_000,
                mode: FakeMode::Usb,
                transmitting: false,
                fail_next: false,
            }
        }

        fn failing() -> Self {
            Self {
                fail_next: true,
                ..Self::new()
            }
        }
    }

    #[async_trait::async_trait(?Send)]
    impl RigctlRadio for FakeRadio {
        type Mode = FakeMode;
        type Error = FakeError;

        async fn get_vfo_a_hz(&mut self) -> Result<u64, Self::Error> {
            if self.fail_next {
                return Err(FakeError);
            }
            Ok(self.vfo_hz)
        }

        async fn set_vfo_a_hz(&mut self, hz: u64) -> Result<(), Self::Error> {
            if self.fail_next {
                return Err(FakeError);
            }
            self.vfo_hz = hz;
            Ok(())
        }

        async fn get_mode(&mut self) -> Result<Self::Mode, Self::Error> {
            if self.fail_next {
                return Err(FakeError);
            }
            Ok(self.mode)
        }

        async fn set_mode(&mut self, mode: Self::Mode) -> Result<(), Self::Error> {
            if self.fail_next {
                return Err(FakeError);
            }
            self.mode = mode;
            Ok(())
        }

        async fn get_transmitting(&mut self) -> Result<bool, Self::Error> {
            if self.fail_next {
                return Err(FakeError);
            }
            Ok(self.transmitting)
        }

        async fn transmit(&mut self) -> Result<(), Self::Error> {
            if self.fail_next {
                return Err(FakeError);
            }
            self.transmitting = true;
            Ok(())
        }

        async fn receive(&mut self) -> Result<(), Self::Error> {
            if self.fail_next {
                return Err(FakeError);
            }
            self.transmitting = false;
            Ok(())
        }

        fn hamlib_mode_name(mode: Self::Mode) -> &'static str {
            match mode {
                FakeMode::Usb => "USB",
                FakeMode::Lsb => "LSB",
            }
        }

        fn hamlib_mode_from_name(name: &str) -> Option<Self::Mode> {
            match name.to_ascii_uppercase().as_str() {
                "USB" => Some(FakeMode::Usb),
                "LSB" => Some(FakeMode::Lsb),
                _ => None,
            }
        }

        fn freq_range_hz() -> (u64, u64) {
            (30_000, 56_000_000)
        }
    }

    // `dispatch`/`dump_state` tests: plain `#[tokio::test]`-free async
    // tests would need a runtime; these have no I/O at all, so a
    // synchronous block_on-free poll via `futures::executor` is
    // unnecessary — `#[test]` + a tiny inline poll is enough, but to match
    // this workspace's existing convention of driving `async fn` tests
    // through a real (minimal) executor rather than hand-rolled polling,
    // use `cat_server::block_on`, which has no OS dependency of its own
    // and is exactly this crate's intended reuse of that primitive.
    fn run<F: std::future::Future>(fut: F) -> F::Output {
        cat_server::block_on::block_on(fut)
    }

    #[test]
    fn dispatch_f_reports_current_frequency() {
        let mut radio = FakeRadio::new();
        assert_eq!(run(dispatch(&mut radio, "f")), "14250000\n");
    }

    #[test]
    fn dispatch_f_reports_error_when_the_radio_fails() {
        let mut radio = FakeRadio::failing();
        assert_eq!(run(dispatch(&mut radio, "f")), RPRT_ERR);
    }

    #[test]
    fn dispatch_capital_f_sets_frequency() {
        let mut radio = FakeRadio::new();
        assert_eq!(run(dispatch(&mut radio, "F 14250000")), RPRT_OK);
        assert_eq!(radio.vfo_hz, 14_250_000);
    }

    #[test]
    fn dispatch_capital_f_accepts_the_decimal_form_hamlib_actually_sends() {
        let mut radio = FakeRadio::new();
        assert_eq!(run(dispatch(&mut radio, "F 14074000.000000")), RPRT_OK);
        assert_eq!(radio.vfo_hz, 14_074_000);
    }

    #[test]
    fn dispatch_capital_f_rejects_non_numeric_argument() {
        let mut radio = FakeRadio::new();
        assert_eq!(run(dispatch(&mut radio, "F not-a-number")), RPRT_ERR);
    }

    #[test]
    fn dispatch_m_reports_mode_and_placeholder_passband() {
        let mut radio = FakeRadio::new();
        assert_eq!(run(dispatch(&mut radio, "m")), "USB\n0\n");
    }

    #[test]
    fn dispatch_capital_m_sets_mode() {
        let mut radio = FakeRadio::new();
        assert_eq!(run(dispatch(&mut radio, "M LSB 0")), RPRT_OK);
        assert_eq!(radio.mode, FakeMode::Lsb);
    }

    #[test]
    fn dispatch_capital_m_rejects_unknown_mode_name() {
        let mut radio = FakeRadio::new();
        assert_eq!(run(dispatch(&mut radio, "M BOGUS 0")), RPRT_ERR);
    }

    #[test]
    fn dispatch_t_reports_ptt_off() {
        let mut radio = FakeRadio::new();
        assert_eq!(run(dispatch(&mut radio, "t")), "0\n");
    }

    #[test]
    fn dispatch_capital_t_one_transmits() {
        let mut radio = FakeRadio::new();
        assert_eq!(run(dispatch(&mut radio, "T 1")), RPRT_OK);
        assert!(radio.transmitting);
    }

    #[test]
    fn dispatch_capital_t_zero_receives() {
        let mut radio = FakeRadio::new();
        radio.transmitting = true;
        assert_eq!(run(dispatch(&mut radio, "T 0")), RPRT_OK);
        assert!(!radio.transmitting);
    }

    #[test]
    fn dispatch_v_reports_vfo_a() {
        let mut radio = FakeRadio::new();
        assert_eq!(run(dispatch(&mut radio, "v")), "VFOA\n");
    }

    #[test]
    fn dispatch_chk_vfo_reports_zero() {
        let mut radio = FakeRadio::new();
        assert_eq!(run(dispatch(&mut radio, "\\chk_vfo")), "0\n");
    }

    #[test]
    fn dispatch_unknown_command_is_rprt_err() {
        let mut radio = FakeRadio::new();
        assert_eq!(run(dispatch(&mut radio, "bogus")), RPRT_ERR);
    }

    #[test]
    fn dispatch_dump_state_ends_every_list_with_a_zero_sentinel_row() {
        let mut radio = FakeRadio::new();
        let state = run(dispatch(&mut radio, "\\dump_state"));
        let lines: Vec<&str> = state.lines().collect();
        // 3 header lines + rx range (1 row + 1 sentinel) + tx range (1 + 1)
        // + tuning steps (1 + 1) + filters (1 + 1) + 12 tail lines (max
        // rit/xit/ifshift, announces, preamp, attenuator, then 6 capability
        // bitmasks: has_get/set_func, has_get/set_level, has_get/set_parm).
        assert_eq!(lines.len(), 3 + 2 + 2 + 2 + 2 + 12);
        assert_eq!(lines[4], "0 0 0 0 0 0 0");
        assert_eq!(lines[6], "0 0 0 0 0 0 0");
        assert_eq!(lines[8], "0 0");
        assert_eq!(lines[10], "0 0");
        assert!(state.starts_with("0\n0\n0\n30000 56000000"));
    }

    #[test]
    fn hamlib_mode_round_trips_for_every_supported_mode() {
        for mode in [FakeMode::Usb, FakeMode::Lsb] {
            let name = FakeRadio::hamlib_mode_name(mode);
            assert_eq!(
                FakeRadio::hamlib_mode_from_name(name),
                Some(mode),
                "mode {mode:?} -> {name} did not round-trip"
            );
        }
    }

    #[test]
    fn line_splitter_returns_none_until_a_newline_arrives() {
        let mut s = LineSplitter::new();
        s.feed(b"f");
        assert_eq!(s.try_take_line(), None);
        s.feed(b"\n");
        assert_eq!(s.try_take_line(), Some("f".to_string()));
    }

    #[test]
    fn line_splitter_trims_trailing_carriage_return() {
        let mut s = LineSplitter::new();
        s.feed(b"f\r\n");
        assert_eq!(s.try_take_line(), Some("f".to_string()));
    }

    #[test]
    fn line_splitter_splits_multiple_buffered_lines() {
        let mut s = LineSplitter::new();
        s.feed(b"f\nm\n");
        assert_eq!(s.try_take_line(), Some("f".to_string()));
        assert_eq!(s.try_take_line(), Some("m".to_string()));
        assert_eq!(s.try_take_line(), None);
    }

    /// Regression guard for a real bug found by an independent post-hoc
    /// review after this crate's initial extraction from `ts570d`/`ft991a`:
    /// the extraction silently dropped `MAX_LINE_LEN`, leaving the line
    /// buffer grow without limit for a client that never sends `\n`.
    #[test]
    fn is_over_limit_trips_once_the_buffer_exceeds_max_line_len_with_no_newline() {
        let mut s = LineSplitter::new();
        s.feed(&vec![b'x'; MAX_LINE_LEN]);
        assert!(!s.is_over_limit());
        s.feed(b"x");
        assert!(s.is_over_limit());
    }

    #[test]
    fn take_final_partial_line_returns_the_remainder_once_on_clean_eof() {
        let mut s = LineSplitter::new();
        s.feed(b"partial");
        assert_eq!(s.take_final_partial_line(), Some("partial".to_string()));
        assert_eq!(s.take_final_partial_line(), None);
    }
}

#[cfg(test)]
mod capability_dump_state_tests {
    use super::*;
    use cat_framework::capabilities::*;

    const MODES: &[ModeDescriptor] = &[ModeDescriptor {
        id: ModeId::Lsb,
        label: "LSB",
        kind: ModeKind::Ssb,
        sideband: Some(Sideband::Lower),
        default_bandwidth_hz: 2400,
    }];
    const METERS: &[MeterDescriptor] = &[MeterDescriptor {
        kind: MeterKind::S,
        raw_range: RawRange::new(0, 30),
        active_on_transmit: false,
    }];
    const ENDPOINTS: &[EndpointDescriptor] = &[EndpointDescriptor {
        role: EndpointRole::Cat,
        required: true,
        shareable_with: &[],
    }];

    static RICH: RadioCapabilities = RadioCapabilities {
        model: "Rich Radio",
        endpoints: EndpointSet::new(ENDPOINTS),
        vfos: VfoCapability {
            count: 2,
            split: true,
            rit_hz: Some(9999),
            xit_hz: Some(9999),
        },
        modes: MODES,
        tuning_steps_hz: &[10, 100, 1_000],
        rx_range: FrequencyRange::new(500_000, 60_000_000),
        filters: FilterCapability {
            if_shift_hz: Some(1_000),
            widths_hz: Some(&[500, 2_400]),
            notch: false,
        },
        meters: MeterSet::new(METERS),
        memory: None,
        menu: None,
        signal: cat_signal::SignalCapability::None,
    };

    static BARE: RadioCapabilities = RadioCapabilities {
        model: "Bare Radio",
        endpoints: EndpointSet::new(ENDPOINTS),
        vfos: VfoCapability {
            count: 1,
            split: false,
            rit_hz: None,
            xit_hz: None,
        },
        modes: MODES,
        tuning_steps_hz: &[],
        rx_range: FrequencyRange::new(1_800_000, 30_000_000),
        filters: FilterCapability {
            if_shift_hz: None,
            widths_hz: None,
            notch: false,
        },
        meters: MeterSet::new(METERS),
        memory: None,
        menu: None,
        signal: cat_signal::SignalCapability::None,
    };

    /// The tail Hamlib counts: everything after the zero-terminated
    /// tuning-step and filter lists.
    fn capability_tail(dump: &str) -> Vec<&str> {
        let lines: Vec<&str> = dump.lines().collect();
        lines[lines.len() - 12..].to_vec()
    }

    // -----------------------------------------------------------------
    // ADR 0005 regression #1: the capability tail's field count.
    //
    // A reply short by even one line makes Hamlib's netrigctl_open()
    // block forever waiting for the rest, and nothing about the symptom
    // points at the cause. It was found once, against a live client, and
    // it is exactly the kind of thing a hand-maintained string regrows.
    // -----------------------------------------------------------------

    #[test]
    fn the_capability_tail_has_exactly_twelve_fields() {
        for caps in [&RICH, &BARE] {
            let dump = dump_state_from_capabilities(caps);
            let tail = capability_tail(&dump);
            assert_eq!(
                tail.len(),
                12,
                "netrigctl_open() reads exactly 12 capability lines for \
                 protocol version 0 and blocks if fewer arrive; {} sent {:?}",
                caps.model,
                tail
            );
            // The last six are the func/level/parm bitmasks, all hex zero.
            assert_eq!(&tail[6..], &["0x0"; 6]);
        }
    }

    #[test]
    fn generated_and_placeholder_replies_have_identical_structure() {
        // The two paths may differ in VALUES but never in SHAPE. Before
        // this refactor there was one hand-written string; now there are
        // two callers, and a field-count drift between them would
        // reproduce the original bug in a new way.
        struct Placeholder;
        #[async_trait::async_trait(?Send)]
        impl crate::RigctlRadio for Placeholder {
            type Mode = ();
            type Error = ();
            async fn get_vfo_a_hz(&mut self) -> Result<u64, ()> {
                Ok(0)
            }
            async fn set_vfo_a_hz(&mut self, _hz: u64) -> Result<(), ()> {
                Ok(())
            }
            async fn get_mode(&mut self) -> Result<(), ()> {
                Ok(())
            }
            async fn set_mode(&mut self, _m: ()) -> Result<(), ()> {
                Ok(())
            }
            async fn get_transmitting(&mut self) -> Result<bool, ()> {
                Ok(false)
            }
            async fn transmit(&mut self) -> Result<(), ()> {
                Ok(())
            }
            async fn receive(&mut self) -> Result<(), ()> {
                Ok(())
            }
            fn hamlib_mode_name(_m: ()) -> &'static str {
                "USB"
            }
            fn hamlib_mode_from_name(_n: &str) -> Option<()> {
                Some(())
            }
            fn freq_range_hz() -> (u64, u64) {
                (500_000, 60_000_000)
            }
        }

        let placeholder = dump_state::<Placeholder>();
        let generated = dump_state_from_capabilities(&BARE);

        assert_eq!(capability_tail(&placeholder).len(), 12);
        assert_eq!(capability_tail(&generated).len(), 12);
        // Same header shape, same sentinel rows.
        assert!(placeholder.starts_with("0\n0\n0\n"));
        assert!(generated.starts_with("0\n0\n0\n"));
        assert_eq!(
            placeholder.matches("0 0 0 0 0 0 0\n").count(),
            generated.matches("0 0 0 0 0 0 0\n").count()
        );
    }

    #[test]
    fn a_radio_that_publishes_nothing_keeps_its_historical_reply() {
        // The compatibility layer's first duty. A radio that has not been
        // migrated must see byte-for-byte what it saw before.
        struct Unmigrated;
        #[async_trait::async_trait(?Send)]
        impl crate::RigctlRadio for Unmigrated {
            type Mode = ();
            type Error = ();
            async fn get_vfo_a_hz(&mut self) -> Result<u64, ()> {
                Ok(0)
            }
            async fn set_vfo_a_hz(&mut self, _hz: u64) -> Result<(), ()> {
                Ok(())
            }
            async fn get_mode(&mut self) -> Result<(), ()> {
                Ok(())
            }
            async fn set_mode(&mut self, _m: ()) -> Result<(), ()> {
                Ok(())
            }
            async fn get_transmitting(&mut self) -> Result<bool, ()> {
                Ok(false)
            }
            async fn transmit(&mut self) -> Result<(), ()> {
                Ok(())
            }
            async fn receive(&mut self) -> Result<(), ()> {
                Ok(())
            }
            fn hamlib_mode_name(_m: ()) -> &'static str {
                "USB"
            }
            fn hamlib_mode_from_name(_n: &str) -> Option<()> {
                Some(())
            }
            fn freq_range_hz() -> (u64, u64) {
                (500_000, 60_000_000)
            }
        }

        assert_eq!(
            dump_state::<Unmigrated>(),
            "0\n0\n0\n\
             500000 60000000 -1 -1 -1 -1 -1\n0 0 0 0 0 0 0\n\
             500000 60000000 -1 -1 -1 -1 -1\n0 0 0 0 0 0 0\n\
             -1 10\n0 0\n\
             -1 2400\n0 0\n\
             1200\n0\n1200\n0\n0\n0\n0x0\n0x0\n0x0\n0x0\n0x0\n0x0\n"
        );
    }

    // -----------------------------------------------------------------
    // Generated content.
    // -----------------------------------------------------------------

    #[test]
    fn the_frequency_range_comes_from_the_radios_own_coverage() {
        let dump = dump_state_from_capabilities(&BARE);
        assert!(dump.contains("1800000 30000000 -1 -1 -1 -1 -1\n"));
        assert!(!dump.contains("500000 60000000"));
    }

    #[test]
    fn every_tuning_step_the_radio_supports_is_reported() {
        let dump = dump_state_from_capabilities(&RICH);
        for step in ["-1 10\n", "-1 100\n", "-1 1000\n"] {
            assert!(dump.contains(step), "missing step row {step:?}");
        }
    }

    #[test]
    fn every_selectable_filter_width_is_reported() {
        let dump = dump_state_from_capabilities(&RICH);
        assert!(dump.contains("-1 500\n"));
        assert!(dump.contains("-1 2400\n"));
    }

    #[test]
    fn rit_and_xit_limits_come_from_the_radio_not_a_constant() {
        assert!(dump_state_from_capabilities(&RICH).contains("9999\n0\n9999\n"));
        // A radio with no RIT reports zero rather than inheriting the old
        // hardcoded 1200.
        assert!(dump_state_from_capabilities(&BARE).contains("0\n0\n0\n0\n0\n0\n0x0"));
    }

    #[test]
    fn a_radio_with_no_steps_or_widths_still_sends_well_formed_lists() {
        // Empty lists would desynchronize the reply. BARE has neither, so
        // this is the case that would break a client if it were mishandled.
        let dump = dump_state_from_capabilities(&BARE);
        assert!(
            dump.contains("-1 10\n0 0\n"),
            "must fall back to a step row"
        );
        assert!(
            dump.contains("-1 2400\n0 0\n"),
            "must fall back to a width row"
        );
        assert_eq!(capability_tail(&dump).len(), 12);
    }
}

#[cfg(test)]
mod hamlib_interop_regression_tests {
    use super::*;
    use futures::executor::block_on;

    /// Records what it was asked to do, so a wire-level command can be
    /// checked against the value that actually reached the radio.
    struct Recorder {
        last_hz: u64,
    }

    #[async_trait::async_trait(?Send)]
    impl crate::RigctlRadio for Recorder {
        type Mode = ();
        type Error = ();
        async fn get_vfo_a_hz(&mut self) -> Result<u64, ()> {
            Ok(self.last_hz)
        }
        async fn set_vfo_a_hz(&mut self, hz: u64) -> Result<(), ()> {
            self.last_hz = hz;
            Ok(())
        }
        async fn get_mode(&mut self) -> Result<(), ()> {
            Ok(())
        }
        async fn set_mode(&mut self, _m: ()) -> Result<(), ()> {
            Ok(())
        }
        async fn get_transmitting(&mut self) -> Result<bool, ()> {
            Ok(false)
        }
        async fn transmit(&mut self) -> Result<(), ()> {
            Ok(())
        }
        async fn receive(&mut self) -> Result<(), ()> {
            Ok(())
        }
        fn hamlib_mode_name(_m: ()) -> &'static str {
            "USB"
        }
        fn hamlib_mode_from_name(_n: &str) -> Option<()> {
            Some(())
        }
        fn freq_range_hz() -> (u64, u64) {
            (500_000, 60_000_000)
        }
    }

    // -----------------------------------------------------------------
    // ADR 0005 regression #2: `F` carries a %f-formatted double.
    //
    // Hamlib's netrigctl_set_freq() sends freq_t (a C double) as `%f` --
    // never a bare integer. Parsing it as an integer returned RPRT -1 to
    // a real `rigctl -m 2` client. WSJT-X setting frequency is the single
    // most-used path through this bridge, so this is the regression that
    // matters most.
    // -----------------------------------------------------------------

    #[test]
    fn set_frequency_accepts_the_float_hamlib_actually_sends() {
        let mut radio = Recorder { last_hz: 0 };
        let reply = block_on(dispatch(&mut radio, "F 14074000.000000"));
        assert_eq!(reply, RPRT_OK);
        assert_eq!(radio.last_hz, 14_074_000);
    }

    #[test]
    fn set_frequency_still_accepts_a_bare_integer() {
        // Hand-typed `rigctl` sessions and older clients send this form.
        let mut radio = Recorder { last_hz: 0 };
        assert_eq!(block_on(dispatch(&mut radio, "F 7100000")), RPRT_OK);
        assert_eq!(radio.last_hz, 7_100_000);
    }

    #[test]
    fn a_fractional_hz_rounds_rather_than_truncating() {
        let mut radio = Recorder { last_hz: 0 };
        block_on(dispatch(&mut radio, "F 14074000.700000"));
        assert_eq!(radio.last_hz, 14_074_001);
    }

    #[test]
    fn a_non_numeric_frequency_is_refused_without_touching_the_radio() {
        let mut radio = Recorder { last_hz: 42 };
        assert_eq!(block_on(dispatch(&mut radio, "F nonsense")), RPRT_ERR);
        assert_eq!(block_on(dispatch(&mut radio, "F")), RPRT_ERR);
        assert_eq!(
            radio.last_hz, 42,
            "a rejected command must not mutate state"
        );
    }
}

/// Live interoperability against a real Hamlib client.
///
/// Task 17's acceptance bar is "verified against a live Hamlib client", and
/// nothing short of running one proves it: both of ADR 0005's bugs
/// (`\dump_state`'s field count, `F`'s `%f` float) passed every unit test
/// this crate had at the time and still failed against `rigctl`. The unit
/// tests above encode what we *learned*; these prove it against the thing
/// that taught us.
///
/// The accept loop here is plain `std::net` rather than either platform
/// backend. That is deliberate: what is under test is the wire protocol —
/// [`dispatch`] and [`LineSplitter`], which are shared by both backends and
/// have no I/O of their own — not the accept loop, which has its own
/// coverage. Using `std::net` keeps this test runnable on any platform
/// without a runtime.
///
/// Skipped, loudly, when `rigctl` is not installed. A test that silently
/// passes when it did not run is worse than no test.
#[cfg(test)]
mod live_hamlib_tests {
    use super::*;
    use futures::executor::block_on;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};

    struct FakeRadio {
        hz: u64,
        transmitting: bool,
    }

    #[async_trait::async_trait(?Send)]
    impl crate::RigctlRadio for FakeRadio {
        type Mode = ();
        type Error = ();
        async fn get_vfo_a_hz(&mut self) -> Result<u64, ()> {
            Ok(self.hz)
        }
        async fn set_vfo_a_hz(&mut self, hz: u64) -> Result<(), ()> {
            self.hz = hz;
            Ok(())
        }
        async fn get_mode(&mut self) -> Result<(), ()> {
            Ok(())
        }
        async fn set_mode(&mut self, _m: ()) -> Result<(), ()> {
            Ok(())
        }
        async fn get_transmitting(&mut self) -> Result<bool, ()> {
            Ok(self.transmitting)
        }
        async fn transmit(&mut self) -> Result<(), ()> {
            self.transmitting = true;
            Ok(())
        }
        async fn receive(&mut self) -> Result<(), ()> {
            self.transmitting = false;
            Ok(())
        }
        fn hamlib_mode_name(_m: ()) -> &'static str {
            "USB"
        }
        fn hamlib_mode_from_name(_n: &str) -> Option<()> {
            Some(())
        }
        fn freq_range_hz() -> (u64, u64) {
            (500_000, 60_000_000)
        }
        fn capabilities() -> Option<&'static cat_framework::capabilities::RadioCapabilities> {
            Some(&CAPS)
        }
    }

    use cat_framework::capabilities::*;
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
    }];
    const ENDPOINTS: &[EndpointDescriptor] = &[EndpointDescriptor {
        role: EndpointRole::Cat,
        required: true,
        shareable_with: &[],
    }];
    static CAPS: RadioCapabilities = RadioCapabilities {
        model: "Interop Test Radio",
        endpoints: EndpointSet::new(ENDPOINTS),
        vfos: VfoCapability {
            count: 2,
            split: true,
            rit_hz: Some(9999),
            xit_hz: Some(9999),
        },
        modes: MODES,
        tuning_steps_hz: &[10, 100, 1_000],
        rx_range: FrequencyRange::new(500_000, 60_000_000),
        filters: FilterCapability {
            if_shift_hz: Some(1_000),
            widths_hz: Some(&[500, 2_400]),
            notch: false,
        },
        meters: MeterSet::new(METERS),
        memory: None,
        menu: None,
        signal: cat_signal::SignalCapability::None,
    };

    /// Whether a real Hamlib client is available to test against.
    ///
    /// **On CI this does not skip — it fails.** `eprintln!` from a passing
    /// test is captured by the harness and shown to nobody, so a "loud"
    /// skip is only loud when someone runs with `--nocapture`. On a
    /// developer machine without Hamlib that is an acceptable trade; in CI
    /// it would mean this crate's most important tests quietly stopped
    /// running the day the runner image changed, which is precisely the
    /// failure mode ADR 0012 was written about.
    fn have_rigctl() -> bool {
        if std::process::Command::new("rigctl")
            .arg("--version")
            .output()
            .is_ok()
        {
            return true;
        }
        assert!(
            std::env::var_os("CI").is_none(),
            "rigctl is not installed, but CI is set. These are the live \
             Hamlib interop tests -- both of ADR 0005's bugs passed every \
             unit test and still failed against a real client, so CI must \
             not go green without them. Install libhamlib-utils."
        );
        eprintln!("SKIPPED: rigctl not installed; install libhamlib-utils to run this");
        false
    }

    /// Serve exactly one connection, then return.
    fn serve_one() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut writer = stream.try_clone().expect("clone");
            let mut reader = BufReader::new(stream);
            let mut radio = FakeRadio {
                hz: 14_074_000,
                transmitting: false,
            };
            let mut splitter = LineSplitter::new();
            let mut chunk = String::new();
            while reader.read_line(&mut chunk).unwrap_or(0) > 0 {
                splitter.feed(chunk.as_bytes());
                chunk.clear();
                while let Some(Ok(line)) = splitter.try_take_line().map(Ok::<_, ()>) {
                    let reply = block_on(dispatch(&mut radio, &line));
                    if writer.write_all(reply.as_bytes()).is_err() {
                        return;
                    }
                    let _ = writer.flush();
                }
            }
        });
        port
    }

    fn rigctl(port: u16, args: &[&str]) -> String {
        let out = std::process::Command::new("rigctl")
            .args(["-m", "2", "-r", &format!("127.0.0.1:{port}")])
            .args(args)
            .output()
            .expect("run rigctl");
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    #[test]
    fn a_real_hamlib_client_completes_the_capability_handshake() {
        if !have_rigctl() {
            eprintln!("SKIPPED: rigctl not installed; install libhamlib-utils to run this");
            return;
        }
        // netrigctl_open() reads `\dump_state` before it will answer
        // anything. If the capability tail is short by a line, this hangs
        // rather than failing -- which is precisely how ADR 0005's bug
        // presented. Getting a frequency back at all proves the handshake
        // completed.
        let port = serve_one();
        let out = rigctl(port, &["f"]);
        assert!(
            out.contains("14074000"),
            "Hamlib did not complete the handshake; got {out:?}"
        );
    }

    #[test]
    fn a_real_hamlib_client_sets_frequency_the_way_it_actually_formats_it() {
        if !have_rigctl() {
            eprintln!("SKIPPED: rigctl not installed; install libhamlib-utils to run this");
            return;
        }
        // This is ADR 0005's second bug end to end. Hamlib formats freq_t
        // as %f on the wire; we never write that string ourselves here --
        // Hamlib does, which is the entire point of running it.
        let port = serve_one();
        let out = rigctl(port, &["F", "7100000", "f"]);
        assert!(
            out.contains("7100000"),
            "frequency did not round-trip through a real client; got {out:?}"
        );
    }

    #[test]
    fn a_real_hamlib_client_reads_ptt_state() {
        if !have_rigctl() {
            eprintln!("SKIPPED: rigctl not installed; install libhamlib-utils to run this");
            return;
        }
        let port = serve_one();
        let out = rigctl(port, &["t"]);
        assert!(out.contains('0'), "expected PTT off; got {out:?}");
    }

    #[test]
    fn the_generated_dump_state_is_what_a_client_receives() {
        if !have_rigctl() {
            eprintln!("SKIPPED: rigctl not installed; install libhamlib-utils to run this");
            return;
        }
        // Proves the capability-generated tail -- not the placeholder --
        // is what went over the wire, by asking for a value only the
        // generated path produces.
        let port = serve_one();
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream.write_all(b"\\dump_state\n").expect("write");
        stream.flush().unwrap();
        let mut reply = String::new();
        let mut reader = BufReader::new(stream);
        for _ in 0..24 {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            reply.push_str(&line);
        }
        assert!(
            reply.contains("-1 1000\n"),
            "generated tuning steps missing"
        );
        assert!(
            reply.contains("-1 500\n"),
            "generated filter widths missing"
        );
        assert!(reply.contains("9999\n"), "generated RIT limit missing");
    }
}
