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

//! The rigctld TCP accept loop and command dispatch, generic over
//! [`crate::RigctlRadio`].
//!
//! Ported from `ft991a`'s current `server/src/rigctl.rs`, which was
//! validated against a real Hamlib client (`rigctl -m 2`, model 2 = "NET
//! rigctl" — the same `netrigctl.c` backend WSJT-X's "Hamlib NET rigctl"
//! rig type uses) after a real WSJT-X instance reported a connection
//! timeout against an earlier version of this module. The command subset
//! below (the short single-letter commands `f`/`F`/`m`/`M`/`t`/`T`/`v`,
//! plus `\dump_state` and `\chk_vfo`) is what `netrigctl.c` actually
//! issues — this is not the same wire text as interactive `rigctl`'s
//! long-form `get_freq`/`set_freq`/... commands, which a human types at a
//! REPL, not what `netrigctl.c` sends automatically. Two bugs were found
//! and fixed in the FT-991A-specific predecessor of this module, and both
//! fixes are preserved here: (1) `\dump_state`'s capability tail was two
//! fields short (missing `has_get_parm`/`has_set_parm`), which left
//! `netrigctl_open()` (in Hamlib's `rigs/dummy/netrigctl.c`) blocked
//! waiting for data that never arrived — the exact field layout was
//! cross-checked against that file's parser and `tests/rigctl_parse.c`'s
//! `dump_state` writer, both fetched from the `Hamlib/Hamlib` GitHub repo;
//! (2) `F` (set frequency) only parsed a bare integer, but
//! `netrigctl_set_freq()` always sends `freq_t` (a C `double`) formatted
//! as `%f`, e.g. `F 14074000.000000`. Mode/range bitmasks are deliberately
//! permissive (`-1`, "any mode") rather than an attempt at Hamlib's exact
//! per-mode bit assignments, specifically to avoid a wrong narrow mask
//! silently rejecting operations WSJT-X needs.
//!
//! Also out of scope, deliberately: split VFO (`s`/`S`) and VFO selection
//! (`V`) have no [`crate::RigctlRadio`] method to back them, so they are
//! not implemented rather than silently faked — `RPRT -1` (this bridge's
//! one generic failure code — see [`RPRT_ERR`]) tells the client honestly
//! that the command isn't supported, rather than reporting success for
//! something that didn't happen.
//!
//! Delegates to a radio-specific [`crate::RigctlRadio`] implementation —
//! this module never constructs a raw radio wire frame itself; that is
//! `cat_server::BrokerCatSession` plus a radio crate's own typed client's
//! job, entirely below this abstraction.

use std::io;

use monoio::io::{AsyncReadRent, AsyncWriteRentExt};
use monoio::net::{TcpListener, TcpStream};

use cat_server::{BrokerCatSession, BrokerHandle, ClientId};

use crate::RigctlRadio;

/// Accept loop, mirroring `cat_server::tcp::serve`'s shape: binding is the
/// caller's responsibility, one task per accepted connection, runs until
/// `accept()` itself fails. `make_radio` builds one `R: RigctlRadio` per
/// accepted connection from a fresh [`BrokerCatSession`] tagged with that
/// connection's [`ClientId`] — this is the one seam where a caller's
/// concrete radio type plugs into otherwise fully generic dispatch logic.
pub(crate) async fn serve<R, F>(
    listener: TcpListener,
    handle: BrokerHandle,
    make_radio: F,
) -> io::Result<()>
where
    R: RigctlRadio + 'static,
    F: Fn(BrokerCatSession) -> R + Clone + 'static,
{
    let mut next_client_id: u64 = 0;
    loop {
        let (stream, _peer_addr) = listener.accept().await?;
        let client_id = ClientId::from_raw(next_client_id);
        next_client_id = next_client_id.wrapping_add(1);
        let handle = handle.clone();
        let make_radio = make_radio.clone();
        monoio::spawn(handle_connection(stream, handle, client_id, make_radio));
    }
}

async fn handle_connection<R, F>(
    mut stream: TcpStream,
    handle: BrokerHandle,
    client_id: ClientId,
    make_radio: F,
) where
    R: RigctlRadio,
    F: Fn(BrokerCatSession) -> R,
{
    let mut radio = make_radio(BrokerCatSession::new(handle, client_id));
    let mut reader = LineReader::new();

    loop {
        let line = match reader.read_line(&mut stream).await {
            Ok(Some(line)) => line,
            Ok(None) | Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.eq_ignore_ascii_case("q") {
            break;
        }

        let response = dispatch(&mut radio, trimmed).await;
        let (result, _buf) = stream.write_all(response.into_bytes()).await;
        if result.is_err() {
            break;
        }
    }
}

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

/// Buffers partial reads and splits them into `\n`-terminated lines (with
/// an optional trailing `\r` trimmed) — rigctld's protocol is line-based
/// text, not `cat-transport-tcp`'s length-prefixed binary framing, so
/// neither that crate's frame codec nor `cat-server::tcp`'s accept loop
/// apply here; this is minimal line-buffering built directly on monoio's
/// owned-buffer `AsyncReadRentExt::read`.
struct LineReader {
    buf: Vec<u8>,
}

impl LineReader {
    fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Returns `Ok(Some(line))` (no trailing `\n`/`\r`) for one complete
    /// line, `Ok(None)` on a clean disconnect with no partial line pending,
    /// or `Err` on any I/O failure. A partial line still in the buffer when
    /// the peer disconnects is returned once as a final "line" (mirrors
    /// how a real terminal client's last unterminated command would still
    /// be worth attempting) rather than silently discarded.
    async fn read_line(&mut self, stream: &mut TcpStream) -> io::Result<Option<String>> {
        loop {
            if let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
                let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
                line.pop(); // trailing '\n'
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
            }

            let chunk = vec![0u8; 4096];
            let (result, chunk) = stream.read(chunk).await;
            let n = result?;
            if n == 0 {
                if self.buf.is_empty() {
                    return Ok(None);
                }
                let line = std::mem::take(&mut self.buf);
                return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small in-crate fake `RigctlRadio`, mirroring how
    /// `cat-server::test_fixtures` builds a small fake `CommandTable`
    /// rather than importing a concrete radio's command set — this
    /// verifies the generic dispatch/dump_state logic in isolation, with
    /// no dependency on either app's `radio` crate.
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

    #[monoio::test(driver = "legacy")]
    async fn dispatch_f_reports_current_frequency() {
        let mut radio = FakeRadio::new();
        assert_eq!(dispatch(&mut radio, "f").await, "14250000\n");
    }

    #[monoio::test(driver = "legacy")]
    async fn dispatch_f_reports_error_when_the_radio_fails() {
        let mut radio = FakeRadio::failing();
        assert_eq!(dispatch(&mut radio, "f").await, RPRT_ERR);
    }

    #[monoio::test(driver = "legacy")]
    async fn dispatch_capital_f_sets_frequency() {
        let mut radio = FakeRadio::new();
        assert_eq!(dispatch(&mut radio, "F 14250000").await, RPRT_OK);
        assert_eq!(radio.vfo_hz, 14_250_000);
    }

    #[monoio::test(driver = "legacy")]
    async fn dispatch_capital_f_accepts_the_decimal_form_hamlib_actually_sends() {
        let mut radio = FakeRadio::new();
        assert_eq!(dispatch(&mut radio, "F 14074000.000000").await, RPRT_OK);
        assert_eq!(radio.vfo_hz, 14_074_000);
    }

    #[monoio::test(driver = "legacy")]
    async fn dispatch_capital_f_rejects_non_numeric_argument() {
        let mut radio = FakeRadio::new();
        assert_eq!(dispatch(&mut radio, "F not-a-number").await, RPRT_ERR);
    }

    #[monoio::test(driver = "legacy")]
    async fn dispatch_m_reports_mode_and_placeholder_passband() {
        let mut radio = FakeRadio::new();
        assert_eq!(dispatch(&mut radio, "m").await, "USB\n0\n");
    }

    #[monoio::test(driver = "legacy")]
    async fn dispatch_capital_m_sets_mode() {
        let mut radio = FakeRadio::new();
        assert_eq!(dispatch(&mut radio, "M LSB 0").await, RPRT_OK);
        assert_eq!(radio.mode, FakeMode::Lsb);
    }

    #[monoio::test(driver = "legacy")]
    async fn dispatch_capital_m_rejects_unknown_mode_name() {
        let mut radio = FakeRadio::new();
        assert_eq!(dispatch(&mut radio, "M BOGUS 0").await, RPRT_ERR);
    }

    #[monoio::test(driver = "legacy")]
    async fn dispatch_t_reports_ptt_off() {
        let mut radio = FakeRadio::new();
        assert_eq!(dispatch(&mut radio, "t").await, "0\n");
    }

    #[monoio::test(driver = "legacy")]
    async fn dispatch_capital_t_one_transmits() {
        let mut radio = FakeRadio::new();
        assert_eq!(dispatch(&mut radio, "T 1").await, RPRT_OK);
        assert!(radio.transmitting);
    }

    #[monoio::test(driver = "legacy")]
    async fn dispatch_capital_t_zero_receives() {
        let mut radio = FakeRadio::new();
        radio.transmitting = true;
        assert_eq!(dispatch(&mut radio, "T 0").await, RPRT_OK);
        assert!(!radio.transmitting);
    }

    #[monoio::test(driver = "legacy")]
    async fn dispatch_v_reports_vfo_a() {
        let mut radio = FakeRadio::new();
        assert_eq!(dispatch(&mut radio, "v").await, "VFOA\n");
    }

    #[monoio::test(driver = "legacy")]
    async fn dispatch_chk_vfo_reports_zero() {
        let mut radio = FakeRadio::new();
        assert_eq!(dispatch(&mut radio, "\\chk_vfo").await, "0\n");
    }

    #[monoio::test(driver = "legacy")]
    async fn dispatch_unknown_command_is_rprt_err() {
        let mut radio = FakeRadio::new();
        assert_eq!(dispatch(&mut radio, "bogus").await, RPRT_ERR);
    }

    #[monoio::test(driver = "legacy")]
    async fn dispatch_dump_state_ends_every_list_with_a_zero_sentinel_row() {
        let mut radio = FakeRadio::new();
        let state = dispatch(&mut radio, "\\dump_state").await;
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
}
