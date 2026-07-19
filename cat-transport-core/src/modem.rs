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

//! Optional capability trait for direct RS-232 modem control/status line
//! access, independent of byte-level CAT framing.

use crate::errors::TransportError;

/// Optional capability for a serial-backed Transport/CatSession: direct
/// control of RS-232 modem control lines (RTS, DTR) and status lines
/// (CTS, DSR, DCD), independent of byte-level CAT framing.
///
/// Not every transport has physical modem control lines — TCP/UDP sessions
/// have none — so this is a separate, additively-implemented trait, never
/// folded into the base Transport/CatSession traits. A consumer bounds its
/// own methods on `S: CatSession + ModemControlLines` rather than requiring
/// it universally.
///
/// All methods are plain sync fns, not #[async_trait] — these are direct
/// ioctl(2) calls with no I/O wait, matching the precedent already set by
/// Transport::flush_rx/CatSession::flush_rx (both plain sync fns on
/// otherwise-async traits, for the same reason).
pub trait ModemControlLines {
    fn set_rts(&self, asserted: bool) -> Result<(), TransportError>;
    fn set_dtr(&self, asserted: bool) -> Result<(), TransportError>;
    fn read_cts(&self) -> Result<bool, TransportError>;
    fn read_dsr(&self) -> Result<bool, TransportError>;
    fn read_dcd(&self) -> Result<bool, TransportError>;
}
