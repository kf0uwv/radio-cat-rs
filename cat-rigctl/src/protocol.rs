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
    let mut s = String::new();
    // Protocol marker, rig model (0 = generic/unknown), ITU region (0 =
    // unspecified) — the three fixed header lines every `dump_state` reply
    // starts with.
    s.push_str("0\n0\n0\n");

    let (min_hz, max_hz) = R::freq_range_hz();

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
    // Tuning steps: `modes step_size`, terminated by an all-zero row. `10`
    // Hz is a conservative, commonly-supported fine step.
    s.push_str("-1 10\n0 0\n");
    // Filters: `modes width`, terminated by an all-zero row. `2400` Hz is a
    // conservative, universally-legal SSB bandwidth — a placeholder, not a
    // per-mode table, since this bridge doesn't resolve real filter widths
    // (module docs).
    s.push_str("-1 2400\n0 0\n");

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
    s.push_str("1200\n0\n1200\n0\n0\n0\n0x0\n0x0\n0x0\n0x0\n0x0\n0x0\n");
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
