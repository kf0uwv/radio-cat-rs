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

//! Answering a client from what the server already knows.
//!
//! # The problem this removes
//!
//! Measured on a TS-570D over 9600-baud serial:
//!
//! | command | median | what it did |
//! |---|---|---|
//! | `\chk_vfo` | 0 ms | answered locally |
//! | `T 0` | 4 ms | a DTR ioctl, no wire |
//! | `t` | 319 ms | read `IF;` from the radio |
//! | `f` | 305 ms | read `FA;` from the radio |
//!
//! The pump was **already** polling the radio and publishing frequency,
//! mode, split and TX into [`NativeShared`]. A client then asked for the
//! frequency and the server sent `FA;` and waited 305 ms for a value it
//! had refreshed 100 ms earlier. Two paths to the same data on one slow
//! link: the console read a cache, rigctl went to the wire.
//!
//! That is what made this bench's occasional two-second serial stalls
//! fatal rather than merely slow. Hamlib's rig timeout is well under a
//! second, so a stalled poll made WSJT-X abandon or retry a transmission.
//!
//! `FA;` reports the dial *setting*. Nothing in it describes RF — that is
//! what an IF tap is for — so there is nothing to be gained by paying wire
//! latency for it.
//!
//! # Set-then-read
//!
//! A client that sets a frequency and reads it straight back must not be
//! told the old one because the next poll has not happened yet. Every set
//! here patches the cache as well as the radio.
//!
//! # Redundant sets
//!
//! A client re-sending the frequency the radio is already on is pure wire
//! time. Skipped — but only when the cache is **fresh** as well as
//! matching, because "the radio is already there" and "the radio was
//! there before the link stalled" are different claims and only one of
//! them justifies not writing.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::native_bridge::NativeShared;
use crate::RigctlRadio;

/// How fresh a *measurement* must be for a matching set to be skipped.
///
/// The number that matters is not how long an operator takes to reach the
/// dial; it is the blind window between the last poll and the set, during
/// which a front-panel move is invisible. Skip inside that window and the
/// radio stays where the operator put it while the client is told `RPRT
/// 0` and goes on believing it moved -- it then logs, and transmits on,
/// a frequency the radio is not tuned to.
///
/// So this is deliberately just over one fast poll (500 ms), not a
/// human-scale figure: the skip is only taken when the pump is actively
/// polling at display rate, which is exactly when a panel move would have
/// been caught within the window. When the pump has backed off to its
/// idle rate no set qualifies and every one goes to the radio, which is
/// the safe direction for an optimisation to fail in.
pub const FRESH_ENOUGH_TO_SKIP: Duration = Duration::from_millis(600);

/// How stale the cache may be and still answer a client read.
///
/// The pump keeps the last good state through a failed read so a console
/// does not flicker, which means a radio that is switched off or
/// unplugged leaves a plausible frequency in the cache indefinitely.
/// Serving `f` from that forever would remove the only signal Hamlib has
/// that rig control is gone. Past this bound the read goes back to the
/// wire and returns the real error.
///
/// Sized at rather more than the idle poll period (ten fast polls, so
/// five seconds), because missing one idle poll on a serial link is
/// ordinary and missing three in a row is not.
pub const MAX_READ_AGE: Duration = Duration::from_secs(12);

/// A radio that answers reads from the server's cache.
pub struct Cached<R: RigctlRadio> {
    inner: R,
    shared: Arc<NativeShared>,
}

impl<R: RigctlRadio> Cached<R> {
    pub fn new(inner: R, shared: Arc<NativeShared>) -> Self {
        Self { inner, shared }
    }

    /// The cache, only if it is fresh enough to decide against writing.
    ///
    /// A measurement, never a value some earlier set assumed: see
    /// `NativeShared::measured_state`.
    fn fresh(&self) -> Option<cat_native::RadioState> {
        self.shared.measured_state(FRESH_ENOUGH_TO_SKIP)
    }
}

#[async_trait(?Send)]
impl<R> RigctlRadio for Cached<R>
where
    R: RigctlRadio,
{
    type Mode = R::Mode;
    type Error = R::Error;

    async fn get_vfo_a_hz(&mut self) -> Result<u64, Self::Error> {
        match self.shared.recent_state(MAX_READ_AGE) {
            Some(state) => Ok(state.vfo_a_hz),
            // Nothing polled yet, or nothing polled lately: a client is
            // better served by a slow answer -- or a real error -- than
            // by a wrong one. See `MAX_READ_AGE`.
            None => self.inner.get_vfo_a_hz().await,
        }
    }

    async fn set_vfo_a_hz(&mut self, hz: u64) -> Result<(), Self::Error> {
        if self.fresh().is_some_and(|s| s.vfo_a_hz == hz) {
            return Ok(());
        }
        self.inner.set_vfo_a_hz(hz).await?;
        self.shared.patch_state(|s| s.vfo_a_hz = hz);
        Ok(())
    }

    async fn get_mode(&mut self) -> Result<Self::Mode, Self::Error> {
        // Served from the cache only where the radio has published an
        // exact `ModeId` -> `Self::Mode` mapping. A radio that has not
        // implemented `mode_from_id`, or one whose current mode has no
        // counterpart, falls through to the wire rather than being
        // reported as an approximation.
        // A *measurement*, unlike the frequency read. `set_vfo_a_hz` can
        // correct the cached frequency it wrote, so reading that back is
        // right; `set_mode` cannot -- the crossing only runs one way --
        // so it invalidates instead, and serving the entry it invalidated
        // would report the mode the radio was in before the set, forever.
        // The coupling this buys is that any set makes the next `m` go to
        // the wire, once, until the following poll measures the radio.
        if let Some(state) = self.shared.measured_state(MAX_READ_AGE) {
            if let Some(mode) = R::mode_from_id(state.mode) {
                return Ok(mode);
            }
        }
        self.inner.get_mode().await
    }

    async fn set_mode(&mut self, mode: Self::Mode) -> Result<(), Self::Error> {
        // The cache cannot be patched with the new mode -- the crossing
        // runs `ModeId` -> `Self::Mode` only, never back -- so the entry
        // is invalidated instead of corrected: bumping the generation
        // marks it assumed, and `get_mode` then falls through to the wire
        // until the next poll measures it. Doing nothing at all would let
        // a poll already in flight publish the pre-set mode as a fresh
        // measurement, and `m` would report the old mode indefinitely.
        self.inner.set_mode(mode).await?;
        self.shared.patch_state(|_| {});
        Ok(())
    }

    async fn get_transmitting(&mut self) -> Result<bool, Self::Error> {
        // Deliberately not cached. A server that drives PTT itself
        // answers from the line state it set -- a `Cell<bool>`, measured
        // at 0.3 ms through this decorator against the emulator -- so
        // there is nothing here for a cache to save.
        //
        // The fallback, where something else is keying and `IF;` is the
        // only source, does cost a full round trip. Caching *that* would
        // be a mistake of a different kind: a client polls `t` to learn
        // when a transmission ended, and the cache learns that from a
        // pump which has deliberately backed off to its idle rate
        // precisely because the radio is transmitting. Answering "still
        // transmitting" from a five-second-old reading would hold a
        // client in TX after the radio had dropped.
        self.inner.get_transmitting().await
    }

    async fn transmit(&mut self) -> Result<(), Self::Error> {
        self.inner.transmit().await?;
        self.shared.patch_state(|s| s.transmitting = true);
        Ok(())
    }

    async fn receive(&mut self) -> Result<(), Self::Error> {
        self.inner.receive().await?;
        self.shared.patch_state(|s| s.transmitting = false);
        Ok(())
    }

    fn unsupported() -> Self::Error {
        R::unsupported()
    }

    async fn get_split(&mut self) -> Result<bool, Self::Error> {
        // From the cache, like the dial: split is carried on every state
        // the pump publishes, and a client polling it during a QSO should
        // not be paying for a round trip to learn something the console
        // already knows.
        match self.shared.recent_state(MAX_READ_AGE) {
            Some(state) => Ok(state.split),
            None => self.inner.get_split().await,
        }
    }

    async fn set_split(&mut self, on: bool) -> Result<(), Self::Error> {
        self.inner.set_split(on).await?;
        self.shared.patch_state(|s| s.split = on);
        Ok(())
    }

    fn hamlib_mode_name(mode: Self::Mode) -> &'static str {
        R::hamlib_mode_name(mode)
    }

    fn hamlib_mode_from_name(name: &str) -> Option<Self::Mode> {
        R::hamlib_mode_from_name(name)
    }

    fn freq_range_hz() -> (u64, u64) {
        R::freq_range_hz()
    }

    fn capabilities() -> Option<&'static cat_framework::capabilities::RadioCapabilities> {
        // Forwarded, not defaulted. `\dump_state` is generated from this,
        // and the default `None` sends Hamlib the placeholder table: a
        // 10 Hz tuning step, one 2400 Hz filter, and +/-1200 Hz of RIT and
        // XIT for a radio that may have neither. Hamlib would then offer
        // the operator an RIT control whose every command comes back
        // `RPRT -1`.
        R::capabilities()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_framework::capabilities::*;
    use cat_native::{ModeId, RadioState};
    use std::cell::RefCell;
    use std::rc::Rc;

    const MODES: &[ModeDescriptor] = &[ModeDescriptor {
        id: ModeId::Usb,
        label: "USB",
        kind: ModeKind::Ssb,
        sideband: Some(Sideband::Upper),
        default_bandwidth_hz: 2400,
    }];
    const ENDPOINTS: &[EndpointDescriptor] = &[EndpointDescriptor {
        role: EndpointRole::Cat,
        required: true,
        shareable_with: &[],
    }];
    static RADIO: RadioCapabilities = RadioCapabilities {
        model: "Cache Test Radio",
        endpoints: EndpointSet::new(ENDPOINTS),
        vfos: VfoCapability {
            count: 2,
            split: true,
            rit_hz: None,
            xit_hz: None,
        },
        modes: MODES,
        tuning_steps_hz: &[10],
        rx_range: FrequencyRange::new(500_000, 60_000_000),
        filters: FilterCapability {
            if_shift_hz: None,
            widths_hz: None,
            notch: false,
        },
        meters: MeterSet::new(&[]),
        memory: None,
        menu: None,
        signal: SignalSupport::None,
    };

    /// What the fake radio was actually asked to do.
    #[derive(Default)]
    struct Log {
        split: bool,
        gets: usize,
        mode_gets: usize,
        sets: Vec<u64>,
        /// The radio ignores writes -- memory mode, or panel LOCK.
        deaf: bool,
        hz: u64,
    }

    struct FakeRadio(Rc<RefCell<Log>>);

    #[async_trait(?Send)]
    impl RigctlRadio for FakeRadio {
        type Mode = ModeId;
        type Error = std::io::Error;

        async fn get_vfo_a_hz(&mut self) -> Result<u64, Self::Error> {
            let mut log = self.0.borrow_mut();
            log.gets += 1;
            Ok(log.hz)
        }

        async fn set_vfo_a_hz(&mut self, hz: u64) -> Result<(), Self::Error> {
            let mut log = self.0.borrow_mut();
            log.sets.push(hz);
            if !log.deaf {
                log.hz = hz;
            }
            Ok(())
        }

        async fn get_mode(&mut self) -> Result<Self::Mode, Self::Error> {
            self.0.borrow_mut().mode_gets += 1;
            Ok(ModeId::Usb)
        }
        async fn set_mode(&mut self, _mode: Self::Mode) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn get_transmitting(&mut self) -> Result<bool, Self::Error> {
            Ok(false)
        }
        async fn get_split(&mut self) -> Result<bool, Self::Error> {
            Ok(self.0.borrow().split)
        }
        async fn set_split(&mut self, on: bool) -> Result<(), Self::Error> {
            self.0.borrow_mut().split = on;
            Ok(())
        }
        async fn transmit(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn receive(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
        fn unsupported() -> Self::Error {
            std::io::Error::other("unsupported")
        }
        fn hamlib_mode_name(_mode: Self::Mode) -> &'static str {
            "USB"
        }
        fn hamlib_mode_from_name(_name: &str) -> Option<Self::Mode> {
            Some(ModeId::Usb)
        }
        fn freq_range_hz() -> (u64, u64) {
            (30_000, 30_000_000)
        }
        fn capabilities() -> Option<&'static RadioCapabilities> {
            Some(&RADIO)
        }
        fn mode_from_id(id: ModeId) -> Option<Self::Mode> {
            Some(id)
        }
    }

    /// The same radio without a `ModeId` mapping, which is the default.
    struct Unmapped(Rc<RefCell<Log>>);

    #[async_trait(?Send)]
    impl RigctlRadio for Unmapped {
        fn unsupported() -> Self::Error {
            std::io::Error::other("unsupported")
        }
        type Mode = ModeId;
        type Error = std::io::Error;
        async fn get_vfo_a_hz(&mut self) -> Result<u64, Self::Error> {
            FakeRadio(Rc::clone(&self.0)).get_vfo_a_hz().await
        }
        async fn set_vfo_a_hz(&mut self, hz: u64) -> Result<(), Self::Error> {
            FakeRadio(Rc::clone(&self.0)).set_vfo_a_hz(hz).await
        }
        async fn get_mode(&mut self) -> Result<Self::Mode, Self::Error> {
            FakeRadio(Rc::clone(&self.0)).get_mode().await
        }
        async fn set_mode(&mut self, mode: Self::Mode) -> Result<(), Self::Error> {
            FakeRadio(Rc::clone(&self.0)).set_mode(mode).await
        }
        async fn get_transmitting(&mut self) -> Result<bool, Self::Error> {
            Ok(false)
        }
        async fn transmit(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
        async fn receive(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
        fn hamlib_mode_name(_mode: Self::Mode) -> &'static str {
            "USB"
        }
        fn hamlib_mode_from_name(_name: &str) -> Option<Self::Mode> {
            Some(ModeId::Usb)
        }
        fn freq_range_hz() -> (u64, u64) {
            (30_000, 30_000_000)
        }
    }

    fn state_at(hz: u64) -> RadioState {
        RadioState {
            vfo_a_hz: hz,
            vfo_b_hz: hz,
            mode: ModeId::Usb,
            split: false,
            transmitting: false,
            memory_channel: None,
            if_shift_hz: None,
            filter_width_hz: None,
            meters: Vec::new(),
            levels: None,
        }
    }

    fn rig(log: &Rc<RefCell<Log>>) -> (Cached<FakeRadio>, Arc<NativeShared>) {
        let shared = NativeShared::new(&RADIO);
        (
            Cached::new(FakeRadio(Rc::clone(log)), Arc::clone(&shared)),
            shared,
        )
    }

    #[monoio::test(driver = "legacy")]
    async fn a_warm_read_never_touches_the_radio() {
        let log = Rc::new(RefCell::new(Log::default()));
        let (mut cached, shared) = rig(&log);
        shared.publish_now(Some(state_at(14_074_000)));

        assert_eq!(cached.get_vfo_a_hz().await.unwrap(), 14_074_000);
        assert_eq!(log.borrow().gets, 0);
    }

    #[monoio::test(driver = "legacy")]
    async fn a_cold_read_goes_to_the_radio() {
        let log = Rc::new(RefCell::new(Log::default()));
        log.borrow_mut().hz = 14_074_000;
        let (mut cached, _shared) = rig(&log);

        assert_eq!(cached.get_vfo_a_hz().await.unwrap(), 14_074_000);
        assert_eq!(log.borrow().gets, 1);
    }

    #[monoio::test(driver = "legacy")]
    async fn a_read_after_a_set_sees_the_set() {
        let log = Rc::new(RefCell::new(Log::default()));
        let (mut cached, shared) = rig(&log);
        shared.publish_now(Some(state_at(14_070_000)));

        cached.set_vfo_a_hz(14_074_000).await.unwrap();
        assert_eq!(cached.get_vfo_a_hz().await.unwrap(), 14_074_000);
        assert_eq!(log.borrow().gets, 0);
    }

    #[monoio::test(driver = "legacy")]
    async fn a_redundant_set_against_a_fresh_measurement_is_skipped() {
        let log = Rc::new(RefCell::new(Log::default()));
        let (mut cached, shared) = rig(&log);
        shared.publish_now(Some(state_at(14_074_000)));

        cached.set_vfo_a_hz(14_074_000).await.unwrap();
        assert!(log.borrow().sets.is_empty());
    }

    #[monoio::test(driver = "legacy")]
    async fn a_repeated_set_is_not_skipped_against_the_value_it_assumed() {
        // The recovery path. The radio ACKed the first set without taking
        // it, so the cache holds a frequency nobody measured. An operator
        // (or a client) re-sending the same frequency must reach the
        // radio, or the one action that could recover it is swallowed.
        let log = Rc::new(RefCell::new(Log::default()));
        log.borrow_mut().deaf = true;
        log.borrow_mut().hz = 14_070_000;
        let (mut cached, shared) = rig(&log);
        shared.publish_now(Some(state_at(14_070_000)));

        cached.set_vfo_a_hz(14_074_000).await.unwrap();
        cached.set_vfo_a_hz(14_074_000).await.unwrap();

        assert_eq!(log.borrow().sets, vec![14_074_000, 14_074_000]);
    }

    #[monoio::test(driver = "legacy")]
    async fn a_set_against_a_stale_measurement_is_not_skipped() {
        // Outside the window the pump has not looked lately, so a
        // front-panel move would be invisible. The write goes through.
        let log = Rc::new(RefCell::new(Log::default()));
        let (mut cached, shared) = rig(&log);
        shared.publish_now(Some(state_at(14_074_000)));
        std::thread::sleep(FRESH_ENOUGH_TO_SKIP + Duration::from_millis(50));

        cached.set_vfo_a_hz(14_074_000).await.unwrap();
        assert_eq!(log.borrow().sets, vec![14_074_000]);
    }

    #[monoio::test(driver = "legacy")]
    async fn a_set_with_a_cold_cache_is_not_skipped() {
        let log = Rc::new(RefCell::new(Log::default()));
        let (mut cached, _shared) = rig(&log);
        cached.set_vfo_a_hz(14_074_000).await.unwrap();
        assert_eq!(log.borrow().sets, vec![14_074_000]);
    }

    #[monoio::test(driver = "legacy")]
    async fn a_failed_set_does_not_patch_the_cache() {
        // `?` on the inner call, so the patch is never reached. Guarding
        // the reverse -- cache says moved, radio did not.
        struct Failing;
        #[async_trait(?Send)]
        impl RigctlRadio for Failing {
            fn unsupported() -> Self::Error {
                std::io::Error::other("unsupported")
            }
            type Mode = ModeId;
            type Error = std::io::Error;
            async fn get_vfo_a_hz(&mut self) -> Result<u64, Self::Error> {
                unreachable!()
            }
            async fn set_vfo_a_hz(&mut self, _hz: u64) -> Result<(), Self::Error> {
                Err(std::io::Error::other("no radio"))
            }
            async fn get_mode(&mut self) -> Result<Self::Mode, Self::Error> {
                unreachable!()
            }
            async fn set_mode(&mut self, _m: Self::Mode) -> Result<(), Self::Error> {
                unreachable!()
            }
            async fn get_transmitting(&mut self) -> Result<bool, Self::Error> {
                unreachable!()
            }
            async fn transmit(&mut self) -> Result<(), Self::Error> {
                unreachable!()
            }
            async fn receive(&mut self) -> Result<(), Self::Error> {
                unreachable!()
            }
            fn hamlib_mode_name(_m: Self::Mode) -> &'static str {
                "USB"
            }
            fn hamlib_mode_from_name(_n: &str) -> Option<Self::Mode> {
                None
            }
            fn freq_range_hz() -> (u64, u64) {
                (0, 0)
            }
        }

        let shared = NativeShared::new(&RADIO);
        shared.publish_now(Some(state_at(14_070_000)));
        let mut cached = Cached::new(Failing, Arc::clone(&shared));

        assert!(cached.set_vfo_a_hz(14_074_000).await.is_err());
        assert_eq!(shared.cached_state().unwrap().vfo_a_hz, 14_070_000);
    }

    #[monoio::test(driver = "legacy")]
    async fn a_warm_mode_read_is_served_from_the_cache() {
        let log = Rc::new(RefCell::new(Log::default()));
        let (mut cached, shared) = rig(&log);
        shared.publish_now(Some(state_at(14_074_000)));
        assert_eq!(cached.get_mode().await.unwrap(), ModeId::Usb);
        assert_eq!(log.borrow().mode_gets, 0);
    }

    #[monoio::test(driver = "legacy")]
    async fn a_mode_read_after_a_mode_set_goes_to_the_radio() {
        // `set_mode` cannot correct the cache -- the `ModeId` crossing
        // only runs one way -- so it invalidates. Serving the entry it
        // invalidated would report the pre-set mode indefinitely.
        let log = Rc::new(RefCell::new(Log::default()));
        let (mut cached, shared) = rig(&log);
        shared.publish_now(Some(state_at(14_074_000)));

        cached.set_mode(ModeId::Lsb).await.unwrap();
        cached.get_mode().await.unwrap();
        assert_eq!(log.borrow().mode_gets, 1);
    }

    #[monoio::test(driver = "legacy")]
    async fn a_radio_without_a_mapping_always_reads_mode_from_the_wire() {
        // The default `mode_from_id` returns `None`. Reporting an
        // approximation would tell a client the radio is in a mode it is
        // not in, which is worse than a slow answer.
        let log = Rc::new(RefCell::new(Log::default()));
        let shared = NativeShared::new(&RADIO);
        shared.publish_now(Some(state_at(14_074_000)));
        let mut cached = Cached::new(Unmapped(Rc::clone(&log)), Arc::clone(&shared));
        cached.get_mode().await.unwrap();
        assert_eq!(log.borrow().mode_gets, 1);
    }

    #[monoio::test(driver = "legacy")]
    async fn split_is_read_from_the_cache_and_a_set_patches_it() {
        // Split is on every state the pump publishes. A client polling it
        // during a QSO should not pay for a round trip to learn what the
        // console already knows, and a client that sets it must read back
        // what it set rather than the value from before.
        let log = Rc::new(RefCell::new(Log::default()));
        let (mut cached, shared) = rig(&log);
        shared.publish_now(Some(state_at(14_074_000)));
        assert!(!cached.get_split().await.unwrap());

        cached.set_split(true).await.unwrap();
        assert!(
            cached.get_split().await.unwrap(),
            "a read after a set must see the set"
        );
    }

    #[test]
    fn capabilities_are_forwarded_rather_than_defaulted() {
        // `\dump_state` is generated from these. The default `None` would
        // advertise +/-1200 Hz of RIT and XIT this radio does not have.
        let real = <Cached<FakeRadio> as RigctlRadio>::capabilities();
        assert!(real.is_some());
        assert_eq!(real.unwrap().vfos.rit_hz, None);
        assert_eq!(
            <Cached<FakeRadio> as RigctlRadio>::freq_range_hz(),
            FakeRadio::freq_range_hz()
        );
    }
}
