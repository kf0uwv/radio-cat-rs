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

//! Refuse rigctl `T 1` while a console holds the CW transmit arm.
//!
//! ADR 0023 decision 8. A console that has armed CW has clamped power and
//! is the one party allowed to key the radio; WSJT-X keying it through
//! rigctl at the same moment would transmit outside every rule the arm
//! enforces. So while [`NativeShared::cw_blocks_external_ptt`] is true --
//! from the arm request until a restore has been verified -- `transmit`
//! is refused and never reaches the radio. `receive` is always allowed:
//! un-keying is never the dangerous direction.
//!
//! A wrapper of its own rather than a line in [`crate::cached::Cached`],
//! because rigctl is wrapped in `Cached` on Linux only. This one goes on
//! both platforms wherever consoles are served.

use std::sync::Arc;

use async_trait::async_trait;
use cat_framework::capabilities::{ModeId, RadioCapabilities};

use crate::native_bridge::NativeShared;
use crate::RigctlRadio;

/// A rigctl radio that cannot be keyed while CW is armed.
pub struct PttGate<R: RigctlRadio> {
    inner: R,
    shared: Arc<NativeShared>,
}

impl<R: RigctlRadio> PttGate<R> {
    pub fn new(inner: R, shared: Arc<NativeShared>) -> Self {
        Self { inner, shared }
    }

    /// The wrapped radio.
    pub fn inner(&self) -> &R {
        &self.inner
    }
}

#[async_trait(?Send)]
impl<R: RigctlRadio> RigctlRadio for PttGate<R> {
    type Mode = R::Mode;
    type Error = R::Error;

    async fn transmit(&mut self) -> Result<(), Self::Error> {
        if self.shared.cw_blocks_external_ptt() {
            tracing::warn!("rigctl T 1 refused: a console holds the CW transmit arm");
            return Err(R::unsupported());
        }
        self.inner.transmit().await
    }

    async fn receive(&mut self) -> Result<(), Self::Error> {
        self.inner.receive().await
    }

    async fn get_transmitting(&mut self) -> Result<bool, Self::Error> {
        self.inner.get_transmitting().await
    }

    async fn get_vfo_a_hz(&mut self) -> Result<u64, Self::Error> {
        self.inner.get_vfo_a_hz().await
    }

    async fn set_vfo_a_hz(&mut self, hz: u64) -> Result<(), Self::Error> {
        self.inner.set_vfo_a_hz(hz).await
    }

    async fn get_mode(&mut self) -> Result<Self::Mode, Self::Error> {
        self.inner.get_mode().await
    }

    async fn set_mode(&mut self, mode: Self::Mode) -> Result<(), Self::Error> {
        self.inner.set_mode(mode).await
    }

    async fn get_split(&mut self) -> Result<bool, Self::Error> {
        self.inner.get_split().await
    }

    async fn set_split(&mut self, on: bool) -> Result<(), Self::Error> {
        self.inner.set_split(on).await
    }

    async fn get_rit_hz(&mut self) -> Result<i32, Self::Error> {
        self.inner.get_rit_hz().await
    }

    async fn get_xit_hz(&mut self) -> Result<i32, Self::Error> {
        self.inner.get_xit_hz().await
    }

    fn unsupported() -> Self::Error {
        R::unsupported()
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

    fn mode_from_id(id: ModeId) -> Option<Self::Mode> {
        R::mode_from_id(id)
    }

    fn capabilities() -> Option<&'static RadioCapabilities> {
        R::capabilities()
    }
}
