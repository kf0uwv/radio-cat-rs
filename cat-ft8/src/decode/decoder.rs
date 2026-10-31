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

use core::ops::ControlFlow;

use super::bp::{decode_soft, BpFailure, BpScratch, CheckRule};
use super::frontend::BIN_HZ;
use super::frontend::{Band, FrontEnd, Spectrogram};
use super::snr;
use super::soft::soft_bits_at;
use super::subtract::{subtract_unreported, SubtractConfig, MIN_SCRATCH_BYTES};
use super::sync::{self, Candidate, SyncReport, ROW_SECONDS};
use crate::ldpc::N;
use crate::message::{unpack, CallHashes, MessageType, Payload};
use crate::Error;

/// Recommended candidate buffer capacity (hosts and small targets): the
/// largest `DecodeConfig::candidates` the measurements cover. The presets
/// use fewer.
pub const CANDIDATES: usize = 200;
// The recommended buffer holds every preset's candidates.
const _: () = assert!(
    DecodeConfig::HOST.candidate_capacity() <= CANDIDATES
        && DecodeConfig::SMALL.candidate_capacity() <= CANDIDATES
);
/// Recommended decode-list capacity on a host. Below what `HOST`'s three
/// passes could list in the worst case ([`DecodeConfig::max_decodes`],
/// 200): validated up to 40 signals per slot; decodes beyond it are
/// counted as truncated and end the passes. Size the list from
/// [`DecodeConfig::max_decodes`] where it must never truncate.
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
    /// belief propagation sees): above 0 and at most
    /// [`DecodeConfig::MAX_LLR_SCALE`].
    pub llr_scale: f32,
    /// Lowest sync score a candidate may have.
    pub min_score: i32,
    /// Candidates kept by the sync search (1–65 535); the candidate buffer
    /// needs at least this many entries.
    pub candidates: usize,
    /// Decoding passes, 1–3, run by [`Decoder::decode_passes`]: after each
    /// pass but the last, that pass's decodes are subtracted from the audio
    /// and the residual is decoded again. 1 is the single-pass decoder.
    /// [`Decoder::decode`] always runs one pass. More than one needs the
    /// slot's audio as a writable buffer (720 000 bytes for a whole slot)
    /// and the subtraction's stack (see [`Decoder::decode_passes`]).
    pub passes: u8,
    /// Candidates kept by the sync search in every pass after the first
    /// (1–65 535); the candidate buffer needs at least this many entries
    /// too ([`DecodeConfig::candidate_capacity`]).
    pub later_candidates: usize,
    /// How each decode is subtracted between passes.
    pub subtract: SubtractConfig,
    /// A deterministic work cap: [`Decoder::decode_passes`] starts a
    /// further pass only if the subtractions it needs, added to those
    /// already made, are at most this many (`None`, the default: no cap).
    /// The passes that run are exactly those an uncapped decode runs
    /// first, so a capped slot's output is a prefix of the uncapped one's
    /// and as reproducible.
    pub max_subtractions: Option<u16>,
}

impl DecodeConfig {
    /// Hosts: sum-product, 30 rounds, LLR scale 4, 100 candidates; three
    /// passes ([`Decoder::decode_passes`]) with 50 candidates in each
    /// further pass and two-symbol (3 840-sample) subtraction segments.
    ///
    /// 100 candidates decode the same messages as 200 on the measured
    /// sets for much less belief-propagation work; in further passes 50
    /// decode all but 4 of the 42 437 messages 200 do (the `crowded`,
    /// `wide` and `fading` sets, three passes; a figure to re-measure
    /// whenever the further-pass count changes). Validated up to 40
    /// signals per slot; for busier bands raise them with
    /// [`DecodeConfig::with_candidates`] and
    /// [`DecodeConfig::with_later_candidates`]. [`Decoder::decode`] runs
    /// the first pass only.
    pub const HOST: DecodeConfig = DecodeConfig {
        rule: CheckRule::SumProduct,
        max_rounds: 30,
        llr_scale: 4.0,
        min_score: 0,
        candidates: 100,
        passes: 3,
        later_candidates: 50,
        subtract: SubtractConfig::DEFAULT.with_segment(2 * crate::SAMPLES_PER_SYMBOL),
        max_subtractions: None,
    };

    /// Small targets: normalised min-sum, which costs about a third as
    /// much per round as sum-product, 75 candidates and one pass (a small
    /// target cannot hold the slot's residual); otherwise as
    /// [`DecodeConfig::HOST`].
    ///
    /// Fewer candidates trim the cost further for a small target's budget.
    /// Validated up to 40 signals per slot; for busier bands raise it with
    /// [`DecodeConfig::with_candidates`]. Provisional until measured on
    /// such a target.
    pub const SMALL: DecodeConfig = DecodeConfig {
        rule: CheckRule::MinSum,
        candidates: 75,
        passes: 1,
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

    /// With `passes` decoding passes (1–3). More than one runs only through
    /// [`Decoder::decode_passes`] (or [`Decoder::decode_further_passes`]),
    /// which needs the audio buffer and
    /// [`SUBTRACT_STACK_BYTES`](super::SUBTRACT_STACK_BYTES) of stack plus
    /// call frames for each subtraction.
    pub const fn with_passes(self, passes: u8) -> Self {
        DecodeConfig { passes, ..self }
    }

    /// With `later_candidates` candidates in every pass after the first.
    pub const fn with_later_candidates(self, later_candidates: usize) -> Self {
        DecodeConfig {
            later_candidates,
            ..self
        }
    }

    /// With subtraction settings `subtract`.
    pub const fn with_subtract(self, subtract: SubtractConfig) -> Self {
        DecodeConfig { subtract, ..self }
    }

    /// With at most `max_subtractions` subtractions per slot (`None`: no
    /// cap); see [`DecodeConfig::max_subtractions`].
    pub const fn with_max_subtractions(self, max_subtractions: Option<u16>) -> Self {
        DecodeConfig {
            max_subtractions,
            ..self
        }
    }

    /// Entries the candidate buffer needs: the larger of
    /// [`DecodeConfig::candidates`] and [`DecodeConfig::later_candidates`]
    /// (whatever the pass count, so [`Decoder::find_later_candidates`] is
    /// never cut short).
    pub const fn candidate_capacity(&self) -> usize {
        if self.later_candidates > self.candidates {
            self.later_candidates
        } else {
            self.candidates
        }
    }

    /// The most decodes [`Decoder::decode_passes`] can list for one slot
    /// (each candidate gives at most one): [`DecodeConfig::candidates`]
    /// plus [`DecodeConfig::later_candidates`] for every further pass.
    /// A list this long never truncates. 200 for `HOST`, 75 for `SMALL`.
    pub const fn max_decodes(&self) -> usize {
        let later = self.passes.saturating_sub(1) as usize;
        self.candidates + later * self.later_candidates
    }

    /// The most decoding passes.
    pub const MAX_PASSES: u8 = 3;

    /// The largest soft-bit scale accepted. Far beyond any useful value
    /// (4 is the default); much larger scales overflow belief propagation's
    /// sums and give meaningless results.
    pub const MAX_LLR_SCALE: f32 = 1_000.0;

    fn check(&self) -> Result<(), Error> {
        // NaN fails both comparisons.
        let scale_ok = self.llr_scale > 0.0 && self.llr_scale <= Self::MAX_LLR_SCALE;
        let counts = 1..=usize::from(u16::MAX);
        let count_ok = counts.contains(&self.candidates) && counts.contains(&self.later_candidates);
        let passes_ok = (1..=Self::MAX_PASSES).contains(&self.passes);
        if scale_ok && count_ok && passes_ok && self.subtract.validate().is_ok() {
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
    /// signal (25–100 Hz either side; see the `snr` estimator's notes in
    /// the ADR). Where the receiver's passband is
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
///
/// A profiling surface: stages will be added as the decoder grows (for
/// example further decoding methods), each a breaking change on purpose
/// so that every profiler's match on it is revisited. Match on it in
/// profiling and measurement code only, not to steer a decode.
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

// The smallest level buffer (one base bin) holds a subtraction's least
// working storage.
const _: () =
    assert!(super::frontend::ROWS * (1 + super::frontend::BINS_ABOVE_BAND) >= MIN_SCRATCH_BYTES);

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
            .field("counts", &self.tally)
            .field("pass_start", &self.pass_start)
            .finish()
    }
}

impl CandidateScratch {
    /// Message-passing rounds the last candidate decoded with this scratch
    /// ran, whatever its result (for cost measurement).
    pub fn last_rounds(&self) -> u8 {
        self.bp.rounds()
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
    /// [`DecodeConfig::candidate_capacity`] ([`Error::BufferTooSmall`]
    /// otherwise); [`Error::InvalidConfig`] for a bad configuration.
    pub fn new(
        front: &'a mut FrontEnd,
        levels: &'a mut [u8],
        sync_scratch: &'a mut [i32],
        candidates: &'a mut [Candidate],
        band: Band,
        config: DecodeConfig,
    ) -> Result<Self, Error> {
        config.check()?;
        if sync_scratch.len() < Self::sync_scratch_len(band)
            || candidates.len() < config.candidate_capacity()
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

    /// The level buffer as working storage (see
    /// [`Spectrogram::idle_levels`]), the decoder emptied as by
    /// [`Decoder::start_slot`]: no rows and no candidates until the next
    /// feed and search.
    fn idle_levels(&mut self) -> &mut [u8] {
        self.sync = SyncReport::default();
        self.spec.idle_levels()
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

    /// Step 1: searches the rows filled so far and lists the candidates
    /// (at most [`DecodeConfig::candidates`]).
    pub fn find_candidates(&mut self) -> SyncReport {
        self.search(self.config.candidates)
    }

    /// Step 1 of a pass after the first: as [`Decoder::find_candidates`],
    /// keeping at most [`DecodeConfig::later_candidates`].
    pub fn find_later_candidates(&mut self) -> SyncReport {
        self.search(self.config.later_candidates)
    }

    fn search(&mut self, limit: usize) -> SyncReport {
        // `new` checked the buffer holds `candidate_capacity()` entries,
        // at least either limit.
        let out = &mut self.candidates[..limit];
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
    /// One pass, whatever [`DecodeConfig::passes`] says (further passes
    /// need the audio: [`Decoder::decode_passes`]).
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

    /// Every configured pass ([`DecodeConfig::passes`]) over one slot's
    /// audio, on one thread, into `list` (emptied first). Returns the
    /// slot's report ([`PassesReport`]): whole-slot totals over every pass
    /// run, the passes run and why they ended.
    ///
    /// `audio[0]` is slot sample 0; the first `skip` samples are missing
    /// input (neither read nor written); a buffer shorter than a slot is a
    /// slot cut short. A longer one is decoded to the slot's end (as
    /// [`Decoder::feed`] takes it), but [`subtract`](super::subtract::subtract) reads and writes the
    /// whole buffer wherever a decoded waveform overlaps it. A `skip` at or
    /// beyond the buffer's end feeds no samples. Pass 1 runs [`Decoder::start_slot`],
    /// [`Decoder::skip`]`(skip)`, [`Decoder::feed`]`(&audio[skip..])`,
    /// [`Decoder::find_candidates`] and every candidate in order — exactly
    /// [`Decoder::decode`] after feeding the audio — and then
    /// [`Decoder::decode_further_passes`] runs the rest. So on return
    /// `audio` holds the slot less every decode of every pass but the last
    /// (with one pass it is unchanged), and a payload is listed once, at
    /// its first decode.
    ///
    /// With one pass this is [`Decoder::decode`] after feeding the audio:
    /// the same list, and the same report in [`PassesReport::report`].
    ///
    /// **Memory.** `scratch` is the candidates' working storage, as for
    /// [`Decoder::decode`]. The subtractions between passes hold their
    /// reference (46 080 bytes) in the spectrogram's level buffer, which is
    /// idle until the next pass refills it, so they need no storage of
    /// their own. A level buffer smaller than that
    /// ([`Band::spectrogram_len`] below
    /// [`SUBTRACT_SCRATCH_BYTES`](super::SUBTRACT_SCRATCH_BYTES): a band of
    /// 235 base bins or fewer, `hi − lo` up to 731.25 Hz) gives the same
    /// result, a little slower (the reference is run a second time where
    /// it does not fit); a level buffer padded to that size restores the
    /// single run. Besides the decoder's own frames, a subtraction needs
    /// [`SUBTRACT_STACK_BYTES`](super::SUBTRACT_STACK_BYTES) of stack plus
    /// its call frames (about 7.4 KB in all in the Cortex-M33 link check:
    /// `subtract` and its time search's frames; the larger figure in
    /// ADR 0024 adds the check program's own frames above them),
    /// only when more than one pass runs.
    ///
    /// The spectrogram ([`Decoder::spectrogram`]) and the candidates
    /// ([`Decoder::candidates`]) are the last pass's on return, unless the
    /// decode stopped ([`PassesEnd::Stopped`]) after a subtraction: then
    /// the decoder is as after [`Decoder::start_slot`] (no rows filled and
    /// no candidates).
    pub fn decode_passes(
        &mut self,
        audio: &mut [f32],
        skip: usize,
        scratch: &mut CandidateScratch,
        list: &mut DecodeList<'_>,
    ) -> PassesReport {
        self.decode_passes_staged(audio, skip, scratch, list, |_| ControlFlow::Continue(()))
    }

    /// [`Decoder::decode_passes`], calling `event` as each step of each
    /// pass ends (for timing) and with each pass's own counts. The result
    /// is the same while `event` returns [`ControlFlow::Continue`].
    ///
    /// If `event` returns [`ControlFlow::Break`], the decode stops at the
    /// next point where it can stop cleanly: after the current candidate
    /// pass, or after the current subtraction (then `audio` holds a
    /// partial residual and the next pass does not run). The list then
    /// holds every pass run so far, whole, and the report says
    /// [`PassesEnd::Stopped`]. This is a safety net for a deadline; a
    /// count-based limit ([`DecodeConfig::max_subtractions`]) keeps the
    /// output reproducible.
    pub fn decode_passes_staged(
        &mut self,
        audio: &mut [f32],
        skip: usize,
        scratch: &mut CandidateScratch,
        list: &mut DecodeList<'_>,
        mut event: impl FnMut(PassEvent<'_>) -> ControlFlow<()>,
    ) -> PassesReport {
        list.clear();
        let mut stop = false;
        let first = self.pass(1, audio, skip, scratch, list, &mut event, &mut stop);
        stop |= event(PassEvent::Done(&first)).is_break();
        self.further_passes(audio, skip, scratch, list, stop, event)
    }

    /// Passes 2 and on ([`DecodeConfig::passes`]) after a pass 1 run
    /// separately, into the same `list`; returns the slot's report as
    /// [`Decoder::decode_passes`] does (pass 1 included).
    ///
    /// The decoder and `list` must be as pass 1 left them: the slot fed
    /// (by any blocks, with [`Decoder::feed`], [`Decoder::feed_i16`] or
    /// [`Decoder::skip`]) and decoded into `list` with [`Decoder::decode`]
    /// or the split API, with no search or feed since. `audio` is the
    /// slot as pass 1 saw it, from slot sample 0, with `skip` as in
    /// [`Decoder::decode_passes`]; it is overwritten with the residual.
    ///
    /// This lets pass 1 be fed as the audio arrives, as `i16` blocks for
    /// example: [`Decoder::feed_i16`] takes sample `x` as
    /// `f32::from(x) / 32 768.0` (exactly; the scale is a power of two), so
    /// with `audio` holding those values the list, report and residual are
    /// bit for bit those of [`Decoder::decode_passes`] on `audio`.
    ///
    /// **Memory** as for [`Decoder::decode_passes`]: `scratch` is the
    /// candidates' (pass 1 can use the same one), the subtractions use the
    /// spectrogram's level buffer, and each subtraction needs
    /// [`SUBTRACT_STACK_BYTES`](super::SUBTRACT_STACK_BYTES) of stack plus
    /// its call frames.
    pub fn decode_further_passes(
        &mut self,
        audio: &mut [f32],
        skip: usize,
        scratch: &mut CandidateScratch,
        list: &mut DecodeList<'_>,
    ) -> PassesReport {
        self.decode_further_passes_staged(audio, skip, scratch, list, |_| ControlFlow::Continue(()))
    }

    /// [`Decoder::decode_further_passes`] with `event` as in
    /// [`Decoder::decode_passes_staged`] (from pass 2's first step; pass
    /// 1's steps and counts are the caller's).
    pub fn decode_further_passes_staged(
        &mut self,
        audio: &mut [f32],
        skip: usize,
        scratch: &mut CandidateScratch,
        list: &mut DecodeList<'_>,
        event: impl FnMut(PassEvent<'_>) -> ControlFlow<()>,
    ) -> PassesReport {
        self.further_passes(audio, skip, scratch, list, false, event)
    }

    /// Passes 2.. after pass 1 (the decoder's state and `list`); `stop`
    /// if a stop was asked for during pass 1.
    fn further_passes(
        &mut self,
        audio: &mut [f32],
        skip: usize,
        scratch: &mut CandidateScratch,
        list: &mut DecodeList<'_>,
        mut stop: bool,
        mut event: impl FnMut(PassEvent<'_>) -> ControlFlow<()>,
    ) -> PassesReport {
        // Pass 1's report: its search and input counters stay in the
        // totals (later passes' searches are added; the list's counts
        // are taken over the whole slot at the end).
        let mut totals = self.report(list);
        let mut out = PassesReport {
            passes_run: 1,
            ..PassesReport::default()
        };
        // The list entries of the last pass run.
        let mut begin = 0;
        for pass in 2..=self.config.passes {
            let len = list.len();
            let need = len - begin;
            let capped = self
                .config
                .max_subtractions
                .is_some_and(|cap| out.subtracted + out.subtract_failed + need > usize::from(cap));
            out.end = if need == 0 {
                PassesEnd::NothingNew
            } else if len == list.capacity() {
                PassesEnd::ListFull
            } else if stop {
                PassesEnd::Stopped
            } else if capped {
                PassesEnd::SubtractionCap
            } else {
                PassesEnd::AllPasses
            };
            if out.end != PassesEnd::AllPasses {
                break;
            }
            // The subtractions hold their reference in the level buffer, so
            // the decoder is emptied first (a stop leaves it so).
            let config = self.config.subtract;
            let levels = self.idle_levels();
            let (mut subtracted, mut failed) = (0, 0);
            for i in begin..len {
                // A decode's frequency and time are always valid for the
                // waveform, and the settings were checked in `new`, so
                // this does not fail; if it did, the audio would be left
                // as it was and the failure counted.
                let r = subtract_unreported(audio, &list.entries[i], skip, &config, levels);
                debug_assert!(r.is_ok(), "subtracting entry {i}: {r:?}");
                if r.is_ok() {
                    subtracted += 1;
                } else {
                    failed += 1;
                }
                stop |= event(PassEvent::Step {
                    pass,
                    step: PassStep::Subtract,
                })
                .is_break();
                if stop {
                    break;
                }
            }
            out.subtracted += subtracted;
            out.subtract_failed += failed;
            if stop {
                out.end = PassesEnd::Stopped;
                break;
            }
            list.next_pass();
            let mut done = self.pass(pass, audio, skip, scratch, list, &mut event, &mut stop);
            done.subtracted = subtracted;
            done.subtract_failed = failed;
            stop |= event(PassEvent::Done(&done)).is_break();
            let r = &done.report;
            totals.candidates_found += r.candidates_found;
            totals.candidates_kept += r.candidates_kept;
            totals.candidates_dropped += r.candidates_dropped;
            out.passes_run = pass;
            begin = len;
        }
        out.report = DecodeReport {
            candidates_found: totals.candidates_found,
            candidates_kept: totals.candidates_kept,
            candidates_dropped: totals.candidates_dropped,
            rows_filled: totals.rows_filled,
            sanitised_samples: totals.sanitised_samples,
            clamped_samples: totals.clamped_samples,
            skipped_samples: totals.skipped_samples,
            floored_levels: totals.floored_levels,
            top_levels: totals.top_levels,
            ..self.report_counts(&list.tally())
        };
        out
    }

    /// One pass: feed `audio` (from slot sample 0, `skip` missing), the
    /// search (pass 1's count or the later one) and every candidate in
    /// order into `list`. Returns the pass's counts (no subtraction
    /// counts); sets `stop` if `event` asks.
    #[allow(clippy::too_many_arguments)]
    fn pass(
        &mut self,
        pass: u8,
        audio: &[f32],
        skip: usize,
        scratch: &mut CandidateScratch,
        list: &mut DecodeList<'_>,
        event: &mut impl FnMut(PassEvent<'_>) -> ControlFlow<()>,
        stop: &mut bool,
    ) -> PassReport {
        let before = list.tally();
        self.start_slot();
        self.skip(skip);
        self.feed(audio.get(skip..).unwrap_or(&[]));
        *stop |= event(PassEvent::Step {
            pass,
            step: PassStep::Feed,
        })
        .is_break();
        if pass == 1 {
            self.find_candidates();
        } else {
            self.find_later_candidates();
        }
        *stop |= event(PassEvent::Step {
            pass,
            step: PassStep::Sync,
        })
        .is_break();
        let mut bp_rounds = 0;
        for i in 0..self.sync.kept {
            let r = self.decode_candidate_staged(i, scratch, |stage| {
                *stop |= event(PassEvent::Step {
                    pass,
                    step: PassStep::Candidate(stage),
                })
                .is_break();
            });
            bp_rounds += usize::from(scratch.last_rounds());
            if let Some(r) = r {
                // In order by construction, so this cannot fail.
                let _ = list.push(i, &r);
            }
        }
        let counts = list.tally().since(&before);
        PassReport {
            pass,
            first: before.len,
            subtracted: 0,
            subtract_failed: 0,
            earlier_duplicates: counts.earlier,
            bp_rounds,
            report: self.report_counts(&counts),
        }
    }

    /// The report for the last search, the last input fed and `list`.
    pub fn report(&self, list: &DecodeList<'_>) -> DecodeReport {
        self.report_counts(&list.tally())
    }

    /// The report for the last search, the last input fed and `c`.
    fn report_counts(&self, c: &Tally) -> DecodeReport {
        DecodeReport {
            candidates_found: self.sync.found,
            candidates_kept: self.sync.kept,
            candidates_dropped: self.sync.dropped,
            decoded: c.len + c.truncated,
            decodes_truncated: c.truncated,
            duplicates: c.duplicates,
            all_zero_rejected: c.all_zero,
            crc_failed: c.crc_failed,
            not_converged: c.not_converged,
            rows_filled: self.spec.rows_filled(),
            sanitised_samples: self.spec.sanitised_samples(),
            clamped_samples: self.spec.clamped_samples(),
            skipped_samples: self.spec.skipped_samples(),
            floored_levels: self.spec.floored_levels(),
            top_levels: self.spec.top_levels(),
        }
    }
}

/// One step of a pass, reported by [`Decoder::decode_passes_staged`] as
/// it ends.
///
/// A profiling surface, like [`Stage`]: steps will be added as the decoder
/// grows, each a breaking change on purpose so that every profiler's
/// match on it is revisited. Match on it in profiling and measurement code
/// only, not to steer a decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassStep {
    /// One of the previous pass's decodes subtracted (passes after the
    /// first; reported once per decode, in list order).
    Subtract,
    /// `start_slot`, `skip` and `feed`.
    Feed,
    /// The sync search.
    Sync,
    /// One stage of one candidate's decode.
    Candidate(Stage),
}

/// What [`Decoder::decode_passes_staged`] reports as it goes.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum PassEvent<'r> {
    /// A step of pass `pass` (from 1) ended.
    Step {
        /// The pass, from 1.
        pass: u8,
        /// The step.
        step: PassStep,
    },
    /// A pass ended: its own counts.
    Done(&'r PassReport),
}

/// One pass's counts (for profiling and measurement; the slot's totals
/// are [`Decoder::decode_passes`]'s [`PassesReport`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct PassReport {
    /// The pass, from 1.
    pub pass: u8,
    /// The list index of the pass's first new entry (the list's length
    /// when it began).
    pub first: usize,
    /// Decodes subtracted before the pass (0 in pass 1).
    pub subtracted: usize,
    /// Subtractions before the pass that failed (the audio left as it
    /// was). Not expected: every listed decode is a valid waveform.
    pub subtract_failed: usize,
    /// Of [`PassReport::report`]'s duplicates, those of a payload listed
    /// by an earlier pass: a subtraction that left a decodable signal.
    pub earlier_duplicates: usize,
    /// Belief-propagation rounds run in the pass, over every candidate.
    pub bp_rounds: usize,
    /// The pass's own counts: its search, the input it fed (in a later
    /// pass the residual), and the list's counts during the pass
    /// ([`DecodeReport::decoded`]: new entries plus truncated).
    pub report: DecodeReport,
}

/// What [`Decoder::decode_passes`] did with a slot.
///
/// One meaning throughout: [`PassesReport::report`] is the whole slot.
/// Its search and list counts (candidates found, kept and dropped,
/// decoded, truncated, duplicates, all-zero, CRC failures, not converged)
/// are totals over every pass run, so they add up with the
/// [`PassReport`]s; its input counters (rows filled, sanitised, clamped,
/// skipped, floored and top levels) describe the slot's audio as pass 1
/// fed it (later passes feed the residual of the same samples; their
/// counters are in their [`PassReport`]s). With one pass it is
/// [`Decoder::decode`]'s report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct PassesReport {
    /// Whole-slot totals.
    pub report: DecodeReport,
    /// Passes run, from 1.
    pub passes_run: u8,
    /// Decodes subtracted, over every pass.
    pub subtracted: usize,
    /// Subtractions that failed, over every pass (not expected).
    pub subtract_failed: usize,
    /// Why no further pass ran.
    pub end: PassesEnd,
}

impl PassesReport {
    /// Whether a limit ended the passes before an unlimited decode would
    /// have: a full list, the subtraction cap or a stop asked for by the
    /// event callback. The output is then a prefix of the unlimited one.
    pub fn stopped_early(&self) -> bool {
        matches!(
            self.end,
            PassesEnd::ListFull | PassesEnd::SubtractionCap | PassesEnd::Stopped
        )
    }
}

/// Why [`Decoder::decode_passes`] ran no further pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PassesEnd {
    /// Every configured pass ran.
    #[default]
    AllPasses,
    /// The last pass run added no decode, so another would find the same.
    NothingNew,
    /// The list was full after the last pass run: a further pass could
    /// only list what it truncated again.
    ListFull,
    /// The next pass's subtractions would exceed
    /// [`DecodeConfig::max_subtractions`].
    SubtractionCap,
    /// The event callback returned [`ControlFlow::Break`].
    Stopped,
}

impl PassesEnd {
    /// A short snake-case name (`all_passes`, `nothing_new`, `list_full`,
    /// `subtraction_cap`, `stopped`).
    pub const fn name(self) -> &'static str {
        match self {
            PassesEnd::AllPasses => "all_passes",
            PassesEnd::NothingNew => "nothing_new",
            PassesEnd::ListFull => "list_full",
            PassesEnd::SubtractionCap => "subtraction_cap",
            PassesEnd::Stopped => "stopped",
        }
    }
}

/// The decodes of a slot, in candidate-index order, over the caller's
/// storage (its length is the capacity).
pub struct DecodeList<'a> {
    entries: &'a mut [Decode],
    tally: Tally,
    /// The next candidate index expected.
    next: usize,
    /// The list's length when the current pass began.
    pass_start: usize,
}

/// A [`DecodeList`]'s counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Tally {
    /// Entries.
    len: usize,
    /// Decodes not stored because the list was full.
    truncated: usize,
    /// Decodes of a payload already listed.
    duplicates: usize,
    /// Of those, payloads listed before the current pass.
    earlier: usize,
    all_zero: usize,
    crc_failed: usize,
    not_converged: usize,
}

impl Tally {
    /// The counts added since `before` (an earlier tally of the same list).
    fn since(&self, before: &Tally) -> Tally {
        Tally {
            len: self.len - before.len,
            truncated: self.truncated - before.truncated,
            duplicates: self.duplicates - before.duplicates,
            earlier: self.earlier - before.earlier,
            all_zero: self.all_zero - before.all_zero,
            crc_failed: self.crc_failed - before.crc_failed,
            not_converged: self.not_converged - before.not_converged,
        }
    }
}

impl<'a> DecodeList<'a> {
    /// An empty list over `entries`.
    pub fn new(entries: &'a mut [Decode]) -> Self {
        DecodeList {
            entries,
            tally: Tally::default(),
            next: 0,
            pass_start: 0,
        }
    }

    fn tally(&self) -> Tally {
        self.tally
    }

    /// Entries the list can hold.
    pub fn capacity(&self) -> usize {
        self.entries.len()
    }

    /// Starts another pass over the same slot: the next result pushed is
    /// candidate 0 again. Nothing else changes: the entries and counts
    /// stay, and a payload already listed (by any pass) is a duplicate.
    pub fn next_pass(&mut self) {
        self.next = 0;
        self.pass_start = self.tally.len;
    }

    /// Decodes in the list.
    pub fn len(&self) -> usize {
        self.tally.len
    }

    /// Whether the list holds no decode.
    pub fn is_empty(&self) -> bool {
        self.tally.len == 0
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
        let t = &mut self.tally;
        match result {
            Err(BpFailure::AllZero) => t.all_zero += 1,
            Err(BpFailure::Crc) => t.crc_failed += 1,
            Err(BpFailure::NotConverged) => t.not_converged += 1,
            Ok(d) => {
                // Truncated payloads are not kept: a repeat of one counts
                // as another truncated decode.
                if let Some(j) = self.entries[..t.len]
                    .iter()
                    .position(|e| e.payload == d.payload)
                {
                    t.duplicates += 1;
                    if j < self.pass_start {
                        t.earlier += 1;
                    }
                } else if t.len < self.entries.len() {
                    self.entries[t.len] = *d;
                    t.len += 1;
                } else {
                    t.truncated += 1;
                }
            }
        }
        Ok(())
    }

    /// Pushes `results[i]` as candidate `pushed() + i`, in order: `results`
    /// continues from the results already pushed ([`DecodeList::pushed`]).
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
        &self.entries[..self.tally.len]
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

/// Measurement only (`profile`): not part of the crate's API.
#[cfg(feature = "profile")]
impl CandidateScratch {
    /// The last candidate's soft bits.
    pub fn soft_bits(&self) -> &[f32; N] {
        &self.llr
    }

    /// Belief propagation's totals after the last candidate's final round.
    pub fn bp_totals(&self) -> &[f32; N] {
        self.bp.last_totals()
    }
}
