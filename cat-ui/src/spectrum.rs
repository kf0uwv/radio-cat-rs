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

//! The waterfall's scrollback.

use cat_signal::SpectrumFrame;
use std::collections::VecDeque;

/// A fixed-capacity ring of recent spectrum frames, newest first.
///
/// Every waterfall needs this and none of it is renderer-specific: the GPU
/// renderer uploads rows to a scrolling texture, the terminal renderer
/// draws half-blocks, and both need the same bounded history with the same
/// drop policy.
///
/// **Bounded by construction.** A waterfall runs for hours at 60 fps; an
/// unbounded history is an out-of-memory bug with a long fuse, and the one
/// thing that must never be "fixed later".
#[derive(Debug, Clone)]
pub struct SpectrumHistory {
    frames: VecDeque<SpectrumFrame>,
    capacity: usize,
    dropped: u64,
}

impl SpectrumHistory {
    /// A history holding at most `capacity` frames. A capacity of zero is
    /// promoted to one — a history that stores nothing would silently make
    /// a waterfall blank.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            frames: VecDeque::with_capacity(capacity),
            capacity,
            dropped: 0,
        }
    }

    /// Add a frame, evicting the oldest if full.
    pub fn push(&mut self, frame: SpectrumFrame) {
        if self.frames.len() == self.capacity {
            self.frames.pop_back();
            self.dropped += 1;
        }
        self.frames.push_front(frame);
    }

    /// The most recent frame, if any.
    pub fn latest(&self) -> Option<&SpectrumFrame> {
        self.frames.front()
    }

    /// Frames newest first — the order a waterfall draws them, with the
    /// current spectrum at the top.
    pub fn iter(&self) -> impl Iterator<Item = &SpectrumFrame> {
        self.frames.iter()
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Frames evicted for age since this history was created.
    pub fn evicted(&self) -> u64 {
        self.dropped
    }

    /// Frames missing between the two most recent, from their sequence
    /// numbers.
    ///
    /// `SpectrumFrame::sequence` exists so a gap is detectable rather than
    /// silently rendered as continuous time. A waterfall with unmarked
    /// gaps lies about how long ago something happened.
    pub fn gap_before_latest(&self) -> u64 {
        let mut it = self.frames.iter();
        match (it.next(), it.next()) {
            (Some(newest), Some(previous)) => newest
                .sequence
                .saturating_sub(previous.sequence)
                .saturating_sub(1),
            _ => 0,
        }
    }

    /// Drop everything. Used when the source changes and old rows would
    /// be at a different centre frequency or span.
    pub fn clear(&mut self) {
        self.frames.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(sequence: u64) -> SpectrumFrame {
        SpectrumFrame {
            center_hz: 14_074_000,
            span_hz: 48_000,
            ref_level_dbm: -20.0,
            bins: vec![-110.0, -40.0],
            sequence,
        }
    }

    #[test]
    fn history_is_bounded_and_evicts_the_oldest() {
        let mut history = SpectrumHistory::new(3);
        for i in 1..=10 {
            history.push(frame(i));
        }
        assert_eq!(history.len(), 3);
        assert_eq!(history.capacity(), 3);
        assert_eq!(history.evicted(), 7);
    }

    #[test]
    fn frames_come_back_newest_first() {
        // The order a waterfall draws: current spectrum at the top.
        let mut history = SpectrumHistory::new(4);
        history.push(frame(1));
        history.push(frame(2));
        history.push(frame(3));
        let sequences: Vec<u64> = history.iter().map(|f| f.sequence).collect();
        assert_eq!(sequences, vec![3, 2, 1]);
        assert_eq!(history.latest().unwrap().sequence, 3);
    }

    #[test]
    fn a_zero_capacity_history_still_holds_the_current_frame() {
        // Storing nothing would make a waterfall blank with no error.
        let mut history = SpectrumHistory::new(0);
        history.push(frame(1));
        assert_eq!(history.len(), 1);
        assert!(history.latest().is_some());
    }

    #[test]
    fn a_gap_in_sequence_numbers_is_detectable() {
        let mut history = SpectrumHistory::new(4);
        history.push(frame(1));
        history.push(frame(2));
        assert_eq!(history.gap_before_latest(), 0);

        history.push(frame(9));
        assert_eq!(history.gap_before_latest(), 6);
    }

    #[test]
    fn an_empty_or_single_frame_history_reports_no_gap() {
        let mut history = SpectrumHistory::new(4);
        assert_eq!(history.gap_before_latest(), 0);
        history.push(frame(100));
        assert_eq!(history.gap_before_latest(), 0);
    }

    #[test]
    fn out_of_order_sequences_do_not_underflow() {
        // Defensive: a source restart could re-issue low sequence numbers,
        // and an underflow here would report a gap of 18 quintillion.
        let mut history = SpectrumHistory::new(4);
        history.push(frame(9));
        history.push(frame(1));
        assert_eq!(history.gap_before_latest(), 0);
    }

    #[test]
    fn clearing_drops_rows_that_no_longer_share_a_frequency_axis() {
        let mut history = SpectrumHistory::new(4);
        history.push(frame(1));
        history.clear();
        assert!(history.is_empty());
        assert!(history.latest().is_none());
    }
}
