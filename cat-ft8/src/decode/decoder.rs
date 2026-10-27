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

//! The decoder (ADR 0024, amendment "single-pass decoder"): configuration,
//! the borrowed workspace, the split API and the merge.
//!
//! A slot is decoded in three steps, which [`Decoder::decode`] runs in
//! order on one thread:
//!
//! 1. [`Decoder::find_candidates`] searches the spectrogram for sync
//!    candidates and lists them in a fixed order (score descending, then
//!    start row, then base bin).
//! 2. [`Decoder::decode_candidate`] decodes one candidate. It takes
//!    `&self` and a [`CandidateScratch`] that it fully re-initialises, so
//!    it is a pure function of the workspace and the candidate index:
//!    callers may run candidates on any number of threads, one scratch
//!    each, in any order.
//! 3. [`DecodeList::push`] takes the results in candidate-index order (an
//!    index out of order is an error): failures are counted, a payload
//!    already in the list is a duplicate (the lowest index is kept), and
//!    decodes beyond the list's capacity are counted as truncated.
//!
//! [`DecodeList::finish`] then unpacks the decodes, in list order, through
//! a callback, using the caller's [`CallHashes`]; [`DecodeList::finish_payloads`]
//! returns the decodes without unpacking. Neither the decoder nor the list
//! writes a hash table before `finish`.
//!
//! For the same input samples, configuration and starting call-table
//! state, the output is identical for any feed block sizes and any thread
//! count (see the crate's determinism promise).
//!
//! ```
//! use cat_ft8::decode::{
//!     Band, Candidate, CandidateScratch, Decode, DecodeConfig, DecodeList, Decoder, FrontEnd,
//!     CANDIDATES, HOST_DECODES,
//! };
//! use cat_ft8::message::NoHashes;
//! use cat_ft8::waveform::{synthesize, Placement};
//! use cat_ft8::{encode, SLOT_SAMPLES};
//!
//! // A slot with one signal (no noise, for brevity).
//! let enc = encode("CQ K1ABC FN42", &mut NoHashes).unwrap();
//! let mut slot = vec![0.0f32; SLOT_SAMPLES];
//! let mut at = Placement::slot(1_500.0, 0.2).unwrap();
//! at.amplitude = 0.1;
//! synthesize(&enc.symbols, &at, &mut slot).unwrap();
//!
//! // Caller-owned buffers (statics on a microcontroller).
//! let band = Band::DEFAULT;
//! let mut front = Box::new(FrontEnd::new());
//! let mut levels = vec![0u8; band.spectrogram_len()];
//! let mut sync = vec![0i32; Decoder::sync_scratch_len(band)];
//! let mut candidates = vec![Candidate::default(); CANDIDATES];
//! let mut decoder =
//!     Decoder::new(&mut front, &mut levels, &mut sync, &mut candidates, band, DecodeConfig::HOST)
//!         .unwrap();
//! for block in slot.chunks(1_920) {
//!     decoder.feed(block);
//! }
//! let mut entries = vec![Decode::EMPTY; HOST_DECODES];
//! let mut list = DecodeList::new(&mut entries);
//! let report = decoder.decode(&mut Box::new(CandidateScratch::new()), &mut list);
//! assert_eq!(report.decoded, 1);
//! list.finish(&mut NoHashes, |d, text| {
//!     assert_eq!(text, Ok("CQ K1ABC FN42"));
//!     assert!((d.freq_hz - 1_500.0).abs() < 1.0 && (d.dt - 0.2).abs() < 0.02);
//! });
//! ```

use super::bp::{decode_soft, BpFailure, BpScratch, CheckRule};
use super::frontend::BIN_HZ;
use super::frontend::{Band, FrontEnd, Spectrogram};
use super::snr;
use super::soft::soft_bits_at;
use super::sync::{self, Candidate, SyncReport, ROW_SECONDS};
use crate::ldpc::N;
use crate::message::{unpack, CallHashes, MessageType, Payload};
use crate::Error;

/// Recommended candidate capacity (hosts and small targets).
pub const CANDIDATES: usize = 200;
/// Recommended decode-list capacity on a host.
pub const HOST_DECODES: usize = 128;
/// Recommended decode-list capacity on a small target.
pub const SMALL_DECODES: usize = 64;

/// Decoder settings. Start from [`DecodeConfig::HOST`] or
/// [`DecodeConfig::SMALL`] and change fields with the `with_*` setters
/// (the type is non-exhaustive, so later settings are not breaking
/// changes).
///
/// ```
/// use cat_ft8::decode::{CheckRule, DecodeConfig};
/// const MINE: DecodeConfig = DecodeConfig::HOST.with_rule(CheckRule::MinSum).with_candidates(100);
/// assert_eq!(MINE.candidates, 100);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct DecodeConfig {
    /// The check-node rule of belief propagation.
    pub rule: CheckRule,
    /// Message-passing rounds before a candidate is given up.
    pub max_rounds: u8,
    /// Mean magnitude the soft bits are scaled to (the reliability scale
    /// belief propagation sees); finite and positive.
    pub llr_scale: f32,
    /// Lowest sync score a candidate may have.
    pub min_score: i32,
    /// Candidates kept by the sync search (1–65 535); the candidate buffer
    /// needs at least this many entries.
    pub candidates: usize,
}

impl DecodeConfig {
    /// Hosts: sum-product, 30 rounds, LLR scale 4, 200 candidates.
    pub const HOST: DecodeConfig = DecodeConfig {
        rule: CheckRule::SumProduct,
        max_rounds: 30,
        llr_scale: 4.0,
        min_score: 0,
        candidates: CANDIDATES,
    };

    /// Small targets: as [`DecodeConfig::HOST`] but with normalised
    /// min-sum, which costs about a third as much per round. Provisional
    /// until measured on such a target.
    pub const SMALL: DecodeConfig = DecodeConfig {
        rule: CheckRule::MinSum,
        ..DecodeConfig::HOST
    };

    /// With check-node rule `rule`.
    pub const fn with_rule(self, rule: CheckRule) -> Self {
        DecodeConfig { rule, ..self }
    }

    /// With at most `max_rounds` message-passing rounds.
    pub const fn with_max_rounds(self, max_rounds: u8) -> Self {
        DecodeConfig { max_rounds, ..self }
    }

    /// With soft bits scaled to mean magnitude `llr_scale`.
    pub const fn with_llr_scale(self, llr_scale: f32) -> Self {
        DecodeConfig { llr_scale, ..self }
    }

    /// With minimum sync score `min_score`.
    pub const fn with_min_score(self, min_score: i32) -> Self {
        DecodeConfig { min_score, ..self }
    }

    /// With a candidate capacity of `candidates`.
    pub const fn with_candidates(self, candidates: usize) -> Self {
        DecodeConfig { candidates, ..self }
    }

    fn check(&self) -> Result<(), Error> {
        let scale_ok = self.llr_scale.is_finite() && self.llr_scale > 0.0;
        let count_ok = (1..=usize::from(u16::MAX)).contains(&self.candidates);
        if scale_ok && count_ok {
            Ok(())
        } else {
            Err(Error::InvalidConfig)
        }
    }
}

impl Default for DecodeConfig {
    fn default() -> Self {
        DecodeConfig::HOST
    }
}

/// One decoded message, without text.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Decode {
    /// The 77-bit payload.
    pub payload: Payload,
    /// Frequency of tone 0, Hz.
    pub freq_hz: f32,
    /// Start relative to the nominal 0.5 s, seconds.
    pub dt: f32,
    /// Signal-to-noise ratio in 2500 Hz, dB, against the noise near the
    /// signal (25–50 Hz either side). Where the receiver's passband is
    /// not flat this can differ from figures taken against a band-wide
    /// noise reference, most near the passband's edges.
    pub snr_db: f32,
    /// The candidate's sync score.
    pub score: i32,
    /// The candidate's index in the sync list.
    pub candidate: u16,
    /// Codeword bits that differ from the channel's hard decisions.
    pub hard_errors: u8,
    /// Message-passing rounds used.
    pub rounds: u8,
}

impl Decode {
    /// Placeholder contents for storage (for example a `static` array).
    pub const EMPTY: Decode = Decode {
        payload: Payload::ZERO,
        freq_hz: 0.0,
        dt: 0.0,
        snr_db: 0.0,
        score: 0,
        candidate: 0,
        hard_errors: 0,
        rounds: 0,
    };

    /// The payload's message type.
    pub fn message_type(&self) -> MessageType {
        self.payload.message_type()
    }
}

impl Default for Decode {
    fn default() -> Self {
        Decode::EMPTY
    }
}

/// The stages of one candidate's decode, reported in this order by
/// [`Decoder::decode_candidate_staged`]. It exists for timing: a profiler
/// books the time since the previous report to the stage reported.
/// Adding a stage is a breaking change on purpose, so that every match
/// on it is revisited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Sub-grid refinement of the candidate's position.
    Refine,
    /// Soft bits at the refined position.
    SoftBits,
    /// Belief propagation, the CRC and the all-zero test.
    BeliefPropagation,
    /// Re-encoding the decoded payload and estimating the SNR (decodes
    /// only).
    Snr,
}

impl Stage {
    /// Every stage, in the order they run.
    pub const ALL: [Stage; 4] = [
        Stage::Refine,
        Stage::SoftBits,
        Stage::BeliefPropagation,
        Stage::Snr,
    ];

    /// The stage's position in [`Stage::ALL`].
    pub const fn index(self) -> usize {
        match self {
            Stage::Refine => 0,
            Stage::SoftBits => 1,
            Stage::BeliefPropagation => 2,
            Stage::Snr => 3,
        }
    }

    /// A short snake-case name (`refine`, `soft_bits`,
    /// `belief_propagation`, `snr`).
    pub const fn name(self) -> &'static str {
        match self {
            Stage::Refine => "refine",
            Stage::SoftBits => "soft_bits",
            Stage::BeliefPropagation => "belief_propagation",
            Stage::Snr => "snr",
        }
    }
}

/// The result of one candidate.
pub type CandidateResult = Result<Decode, BpFailure>;

/// Working storage for one candidate (soft bits, belief-propagation
/// messages and the noise histogram, about 5.1 KB). One per worker; every
/// candidate re-initialises it.
#[derive(Clone)]
pub struct CandidateScratch {
    llr: [f32; N],
    bp: BpScratch,
    hist: [u16; 256],
}

impl CandidateScratch {
    /// Zeroed storage (usable in a `static` initialiser).
    pub const fn new() -> Self {
        CandidateScratch {
            llr: [0.0; N],
            bp: BpScratch::new(),
            hist: [0; 256],
        }
    }
}

/// Working storage only: its contents are not printed.
impl core::fmt::Debug for CandidateScratch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CandidateScratch").finish_non_exhaustive()
    }
}

/// The spectrogram's summary, the configuration and the last search; no
/// buffers are printed.
impl core::fmt::Debug for Decoder<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Decoder")
            .field("spectrogram", &self.spec)
            .field("config", &self.config)
            .field("sync", &self.sync)
            .finish_non_exhaustive()
    }
}

/// The decodes in the list (not the unused capacity) and the counts.
impl core::fmt::Debug for DecodeList<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecodeList")
            .field("decodes", &self.finish_payloads())
            .field("capacity", &self.entries.len())
            .field("pushed", &self.next)
            .field("truncated", &self.truncated)
            .field("duplicates", &self.duplicates)
            .field("all_zero", &self.all_zero)
            .field("crc_failed", &self.crc_failed)
            .field("not_converged", &self.not_converged)
            .finish()
    }
}

impl Default for CandidateScratch {
    fn default() -> Self {
        Self::new()
    }
}

/// What a decode did, never silently truncated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct DecodeReport {
    /// Sync local maxima at or above the minimum score.
    pub candidates_found: usize,
    /// Of those, kept (and tried).
    pub candidates_kept: usize,
    /// Of those, dropped because the candidate list was full.
    pub candidates_dropped: usize,
    /// Decodes: those in the list plus [`DecodeReport::decodes_truncated`].
    /// Exact (distinct payloads) unless truncation happened; then an upper
    /// bound, since a truncated payload is not kept and a repeat of it
    /// counts as truncated again.
    pub decoded: usize,
    /// Decodes not stored because the list was full (see
    /// [`DecodeReport::decoded`]).
    pub decodes_truncated: usize,
    /// Candidates that decoded a payload already in the list.
    pub duplicates: usize,
    /// Candidates that converged on the all-zero codeword (rejected).
    pub all_zero_rejected: usize,
    /// Candidates that converged on a codeword whose CRC failed.
    pub crc_failed: usize,
    /// Candidates without a codeword within the round limit.
    pub not_converged: usize,
    /// Spectrogram rows filled (184 for a whole slot).
    pub rows_filled: usize,
    /// Non-finite input samples replaced by 0.0.
    pub sanitised_samples: usize,
    /// Input samples clamped to ±2¹⁶.
    pub clamped_samples: usize,
    /// Samples skipped (counted as zeros).
    pub skipped_samples: usize,
    /// Spectrogram levels at the bottom of the range.
    pub floored_levels: usize,
    /// Spectrogram levels at the top of the range.
    pub top_levels: usize,
}

/// The decoder: a spectrogram over the caller's buffers, the sync scratch
/// and candidate list (also the caller's), and the configuration.
///
/// A `Decoder` holds a slot's rows as they are fed, so it must live across
/// the whole slot: from [`Decoder::start_slot`] (or `new`) through the
/// last `feed` to the decode. One decoder serves every slot.
pub struct Decoder<'a> {
    spec: Spectrogram<'a>,
    scratch: &'a mut [i32],
    candidates: &'a mut [Candidate],
    config: DecodeConfig,
    sync: SyncReport,
}

impl<'a> Decoder<'a> {
    /// A decoder for `band` at the start of a slot. `levels` needs
    /// [`Band::spectrogram_len`] entries, `sync_scratch`
    /// [`Decoder::sync_scratch_len`] and `candidates` at least
    /// `config.candidates` ([`Error::BufferTooSmall`] otherwise);
    /// [`Error::InvalidConfig`] for a bad configuration.
    pub fn new(
        front: &'a mut FrontEnd,
        levels: &'a mut [u8],
        sync_scratch: &'a mut [i32],
        candidates: &'a mut [Candidate],
        band: Band,
        config: DecodeConfig,
    ) -> Result<Self, Error> {
        config.check()?;
        if sync_scratch.len() < Self::sync_scratch_len(band) || candidates.len() < config.candidates
        {
            return Err(Error::BufferTooSmall);
        }
        Ok(Decoder {
            spec: Spectrogram::new(front, levels, band)?,
            scratch: sync_scratch,
            candidates,
            config,
            sync: SyncReport::default(),
        })
    }

    /// Entries the sync scratch needs for `band` (usable to size a
    /// `static`).
    pub const fn sync_scratch_len(band: Band) -> usize {
        sync::scratch_len(band.base_bins())
    }

    /// The configuration.
    pub fn config(&self) -> &DecodeConfig {
        &self.config
    }

    /// The spectrogram (read-only).
    pub fn spectrogram(&self) -> &Spectrogram<'a> {
        &self.spec
    }

    /// Starts a new slot (see [`Spectrogram::start_slot`]); the candidate
    /// list is emptied.
    pub fn start_slot(&mut self) {
        self.spec.start_slot();
        self.sync = SyncReport::default();
    }

    /// Feeds 12 kHz samples (full scale ±1.0); see [`Spectrogram::feed`].
    ///
    /// Feeding after [`Decoder::find_candidates`] leaves the candidate list
    /// as it was (stale against the new rows) until [`Decoder::start_slot`]
    /// or another search.
    pub fn feed(&mut self, samples: &[f32]) -> usize {
        self.spec.feed(samples)
    }

    /// Feeds 16-bit samples; see [`Spectrogram::feed_i16`].
    pub fn feed_i16(&mut self, samples: &[i16]) -> usize {
        self.spec.feed_i16(samples)
    }

    /// Advances over missing samples; see [`Spectrogram::skip`].
    pub fn skip(&mut self, n: usize) -> usize {
        self.spec.skip(n)
    }

    /// Rows complete so far (0..=184).
    pub fn rows_filled(&self) -> usize {
        self.spec.rows_filled()
    }

    /// Step 1: searches the rows filled so far and lists the candidates.
    pub fn find_candidates(&mut self) -> SyncReport {
        let out = &mut self.candidates[..self.config.candidates];
        // The buffer sizes were checked in `new`, so this cannot fail.
        self.sync = sync::find_candidates(&self.spec, self.config.min_score, self.scratch, out)
            .unwrap_or_default();
        self.sync
    }

    /// The candidates of the last search, in order.
    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates[..self.sync.kept]
    }

    /// Step 2: decodes candidate `index` of the last search, re-initialising
    /// `scratch`. `None` if there is no such candidate.
    ///
    /// The candidate's position is refined to a fraction of a grid step
    /// from the sync scores of its neighbours; soft bits, frequency, time
    /// offset and SNR are taken at the refined position.
    pub fn decode_candidate(
        &self,
        index: usize,
        scratch: &mut CandidateScratch,
    ) -> Option<CandidateResult> {
        self.decode_candidate_staged(index, scratch, |_| {})
    }

    /// [`Decoder::decode_candidate`], calling `done` as each [`Stage`]
    /// ends, in order (the SNR stage only for a decode), so a caller can
    /// time the stages. The result is the same.
    pub fn decode_candidate_staged(
        &self,
        index: usize,
        scratch: &mut CandidateScratch,
        mut done: impl FnMut(Stage),
    ) -> Option<CandidateResult> {
        let c = *self.candidates().get(index)?;
        let CandidateScratch { llr, bp, hist } = scratch;
        let (frac_row, frac_bin) = sync::refine(&self.spec, &c);
        done(Stage::Refine);
        soft_bits_at(
            &self.spec,
            &c,
            frac_row,
            frac_bin,
            self.config.llr_scale,
            llr,
        );
        done(Stage::SoftBits);
        let decoded = decode_soft(llr, self.config.rule, self.config.max_rounds, bp);
        done(Stage::BeliefPropagation);
        let result = decoded.map(|d| {
            let base = (self.spec.band().first_bin() + usize::from(c.base)) as f32 + frac_bin;
            let tones = crate::encode_payload(&d.payload);
            let snr_db = snr::estimate(&self.spec, &c, frac_row, frac_bin, &tones, hist);
            done(Stage::Snr);
            Decode {
                payload: d.payload,
                freq_hz: base * BIN_HZ,
                dt: c.dt() + frac_row * ROW_SECONDS,
                snr_db,
                score: c.score,
                candidate: index as u16,
                hard_errors: d.hard_errors,
                rounds: d.rounds,
            }
        });
        Some(result)
    }

    /// Steps 1–3 on one thread: [`Decoder::find_candidates`], then
    /// [`Decoder::decode_candidate`] for every candidate in order, each
    /// result pushed into `list` (emptied first). Returns the report.
    pub fn decode(
        &mut self,
        scratch: &mut CandidateScratch,
        list: &mut DecodeList<'_>,
    ) -> DecodeReport {
        self.find_candidates();
        list.clear();
        for i in 0..self.sync.kept {
            if let Some(r) = self.decode_candidate(i, scratch) {
                // In order by construction, so this cannot fail.
                let _ = list.push(i, &r);
            }
        }
        self.report(list)
    }

    /// The report for the last search and `list`.
    pub fn report(&self, list: &DecodeList<'_>) -> DecodeReport {
        DecodeReport {
            candidates_found: self.sync.found,
            candidates_kept: self.sync.kept,
            candidates_dropped: self.sync.dropped,
            decoded: list.len + list.truncated,
            decodes_truncated: list.truncated,
            duplicates: list.duplicates,
            all_zero_rejected: list.all_zero,
            crc_failed: list.crc_failed,
            not_converged: list.not_converged,
            rows_filled: self.spec.rows_filled(),
            sanitised_samples: self.spec.sanitised_samples(),
            clamped_samples: self.spec.clamped_samples(),
            skipped_samples: self.spec.skipped_samples(),
            floored_levels: self.spec.floored_levels(),
            top_levels: self.spec.top_levels(),
        }
    }
}

/// The decodes of a slot, in candidate-index order, over the caller's
/// storage (its length is the capacity).
pub struct DecodeList<'a> {
    entries: &'a mut [Decode],
    len: usize,
    next: usize,
    truncated: usize,
    duplicates: usize,
    all_zero: usize,
    crc_failed: usize,
    not_converged: usize,
}

impl<'a> DecodeList<'a> {
    /// An empty list over `entries`.
    pub fn new(entries: &'a mut [Decode]) -> Self {
        DecodeList {
            entries,
            len: 0,
            next: 0,
            truncated: 0,
            duplicates: 0,
            all_zero: 0,
            crc_failed: 0,
            not_converged: 0,
        }
    }

    /// Empties the list and its counts.
    pub fn clear(&mut self) {
        let entries = core::mem::take(&mut self.entries);
        *self = DecodeList::new(entries);
    }

    /// Step 3: the result of candidate `index`, which must be the next
    /// index (0 after `new` or `clear`), and a decode must carry that index;
    /// [`Error::OutOfOrder`] otherwise (nothing is changed).
    pub fn push(&mut self, index: usize, result: &CandidateResult) -> Result<(), Error> {
        let carried = match result {
            Ok(d) => usize::from(d.candidate) == index,
            Err(_) => true,
        };
        if index != self.next || !carried {
            return Err(Error::OutOfOrder);
        }
        self.next += 1;
        match result {
            Err(BpFailure::AllZero) => self.all_zero += 1,
            Err(BpFailure::Crc) => self.crc_failed += 1,
            Err(BpFailure::NotConverged) => self.not_converged += 1,
            Ok(d) => {
                // Truncated payloads are not kept: a repeat of one counts
                // as another truncated decode.
                if self.entries[..self.len]
                    .iter()
                    .any(|e| e.payload == d.payload)
                {
                    self.duplicates += 1;
                } else if self.len < self.entries.len() {
                    self.entries[self.len] = *d;
                    self.len += 1;
                } else {
                    self.truncated += 1;
                }
            }
        }
        Ok(())
    }

    /// Pushes `results[i]` as candidate `i`, in order, after any results
    /// already pushed (so `results` continues from [`DecodeList::pushed`]).
    pub fn merge(&mut self, results: &[CandidateResult]) -> Result<(), Error> {
        for r in results {
            self.push(self.next, r)?;
        }
        Ok(())
    }

    /// Results pushed so far.
    pub fn pushed(&self) -> usize {
        self.next
    }

    /// The decodes, without unpacking.
    pub fn finish_payloads(&self) -> &[Decode] {
        &self.entries[..self.len]
    }

    /// Unpacks every decode in order with `hashes`, calling `out` with the
    /// decode and its text, or the unpack error for a payload this crate
    /// cannot render (the decode is still reported).
    pub fn finish<H: CallHashes + ?Sized>(
        &self,
        hashes: &mut H,
        mut out: impl FnMut(&Decode, Result<&str, Error>),
    ) {
        for d in self.finish_payloads() {
            match unpack(&d.payload, hashes) {
                Ok(text) => out(d, Ok(text.as_str())),
                Err(e) => out(d, Err(e)),
            }
        }
    }
}

#[cfg(test)]
mod tests;
