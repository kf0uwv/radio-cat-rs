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

//! Scoring copy: align what was sent with what was copied.
//!
//! [`align`] finds a minimum-edit alignment and labels every difference
//! as a substitution, deletion (missed) or insertion (extra), each tied
//! to a specific expected unit, so [`CharStats`] can say which characters
//! an operator finds hard and what they mistake them for. A unit is a
//! [`Token`]: a character, a prosign, or a word space.
//!
//! The expected text must encode; the copied text is whatever the
//! operator typed — an unknown character is simply a wrong one.
//!
//! # Persisting stats
//!
//! [`CharStats`] is plain data with [`CharStats::merge`]. With the
//! `serde` feature it (and [`Score`], [`Alignment`]) serialize; maps are
//! written as lists of pairs, since a token is not a JSON object key.

use std::collections::BTreeMap;

use crate::codec::{self, CodecError, Prosign, Token};

/// A unit of text being scored.
pub type Unit = Token;

/// Whether word spaces count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SpacePolicy {
    /// A missing, extra or misplaced word space is an error (default).
    #[default]
    Score,
    /// Word spaces are removed from both texts before aligning.
    Ignore,
}

/// Scoring options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScoreOptions {
    /// How word spaces count.
    pub spaces: SpacePolicy,
    /// Treat `=`/`<BT>`, `+`/`<AR>`, `(`/`<KN>` and `&`/`<AS>` as equal
    /// (default `true`): they are the same Morse.
    pub fold_equivalents: bool,
}

impl Default for ScoreOptions {
    fn default() -> Self {
        ScoreOptions {
            spaces: SpacePolicy::Score,
            fold_equivalents: true,
        }
    }
}

/// One step of an alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AlignOp {
    /// Copied correctly.
    Match { expected: Unit },
    /// Copied as something else.
    Substitution { expected: Unit, copied: Unit },
    /// Not copied at all.
    Deletion { expected: Unit },
    /// Copied but never sent. `attributed_to` is the index (into
    /// [`Alignment::expected`]) of the sent unit it is charged to.
    Insertion {
        copied: Unit,
        attributed_to: Option<usize>,
    },
}

/// The alignment of a copy against what was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Alignment {
    /// The sent units, after the options were applied.
    pub expected: Vec<Unit>,
    /// The steps, in order.
    pub ops: Vec<AlignOp>,
}

/// Totals for one alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Score {
    /// Units sent.
    pub expected: usize,
    /// Units copied correctly.
    pub matches: usize,
    /// Units copied as something else.
    pub substitutions: usize,
    /// Units missed.
    pub deletions: usize,
    /// Units copied that were not sent.
    pub insertions: usize,
}

impl Score {
    /// Character error rate: `(S + D + I) / sent`. With nothing sent, 0
    /// for an empty copy and 1 otherwise.
    pub fn cer(&self) -> f64 {
        let errors = self.substitutions + self.deletions + self.insertions;
        if self.expected == 0 {
            if errors == 0 {
                0.0
            } else {
                1.0
            }
        } else {
            errors as f64 / self.expected as f64
        }
    }

    /// Adds another score's counts to this one.
    pub fn add(&mut self, other: &Score) {
        self.expected += other.expected;
        self.matches += other.matches;
        self.substitutions += other.substitutions;
        self.deletions += other.deletions;
        self.insertions += other.insertions;
    }
}

/// Tokenizes what an operator typed. Like [`codec::tokenize`] but never
/// fails: an unknown character is kept as itself, and a `<...>` that is
/// not a prosign is kept as its characters.
pub fn tokenize_copy(text: &str) -> Vec<Unit> {
    let mut out = Vec::new();
    let mut pending_space = false;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
            i += 1;
            continue;
        }
        let mut token = Token::Char(ch.to_ascii_uppercase());
        let mut step = 1;
        if ch == '<' {
            if let Some(close) = chars[i + 1..].iter().position(|&c| c == '>') {
                let name: String = chars[i + 1..i + 1 + close].iter().collect();
                if let Some(p) = Prosign::from_name(&name) {
                    token = Token::Prosign(p);
                    step = close + 2;
                }
            }
        }
        if pending_space {
            out.push(Token::WordSpace);
            pending_space = false;
        }
        out.push(token);
        i += step;
    }
    out
}

fn prepare(units: Vec<Unit>, opts: &ScoreOptions) -> Vec<Unit> {
    units
        .into_iter()
        .filter(|u| !(opts.spaces == SpacePolicy::Ignore && *u == Token::WordSpace))
        .map(|u| {
            if opts.fold_equivalents {
                codec::canonical(u)
            } else {
                u
            }
        })
        .collect()
}

/// Aligns `copied` against `expected` (which must encode).
pub fn align(expected: &str, copied: &str, opts: &ScoreOptions) -> Result<Alignment, CodecError> {
    let e = codec::tokenize(expected)?;
    Ok(align_units(&e, &tokenize_copy(copied), opts))
}

/// Aligns already-tokenized text.
pub fn align_units(expected: &[Unit], copied: &[Unit], opts: &ScoreOptions) -> Alignment {
    let e = prepare(expected.to_vec(), opts);
    let c = prepare(copied.to_vec(), opts);
    let (n, m) = (e.len(), c.len());
    let w = m + 1;
    let mut d = vec![0u32; (n + 1) * w];
    for i in 0..=n {
        d[i * w] = i as u32;
    }
    for (j, cell) in d.iter_mut().enumerate().take(m + 1) {
        *cell = j as u32;
    }
    for i in 1..=n {
        for j in 1..=m {
            let sub = d[(i - 1) * w + j - 1] + u32::from(e[i - 1] != c[j - 1]);
            let del = d[(i - 1) * w + j] + 1;
            let ins = d[i * w + j - 1] + 1;
            d[i * w + j] = sub.min(del).min(ins);
        }
    }
    // Trace back, preferring diagonal, then deletion, then insertion.
    let mut ops = Vec::with_capacity(n.max(m));
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        let here = d[i * w + j];
        if i > 0 && j > 0 {
            let same = e[i - 1] == c[j - 1];
            if here == d[(i - 1) * w + j - 1] + u32::from(!same) {
                ops.push(if same {
                    AlignOp::Match { expected: e[i - 1] }
                } else {
                    AlignOp::Substitution {
                        expected: e[i - 1],
                        copied: c[j - 1],
                    }
                });
                i -= 1;
                j -= 1;
                continue;
            }
        }
        if i > 0 && here == d[(i - 1) * w + j] + 1 {
            ops.push(AlignOp::Deletion { expected: e[i - 1] });
            i -= 1;
        } else {
            ops.push(AlignOp::Insertion {
                copied: c[j - 1],
                attributed_to: None,
            });
            j -= 1;
        }
    }
    ops.reverse();
    attribute_insertions(&mut ops, n);
    Alignment { expected: e, ops }
}

/// Charges each insertion to an expected unit: a neighbouring
/// substitution first (copying `N` as `ET` is one mistake about `N`),
/// else the preceding sent unit, else the following one.
fn attribute_insertions(ops: &mut [AlignOp], n: usize) {
    // Expected index of each op (for non-insertions), and the index of the
    // last expected unit consumed before each op.
    let mut idx_of = vec![None; ops.len()];
    let mut before = vec![None; ops.len()];
    let mut k = 0usize;
    for (p, op) in ops.iter().enumerate() {
        before[p] = k.checked_sub(1);
        if !matches!(op, AlignOp::Insertion { .. }) {
            idx_of[p] = Some(k);
            k += 1;
        }
    }
    let is_sub = |op: &AlignOp| matches!(op, AlignOp::Substitution { .. });
    let snapshot: Vec<AlignOp> = ops.to_vec();
    for p in 0..ops.len() {
        if let AlignOp::Insertion { copied, .. } = snapshot[p] {
            // Nearest non-insertion op on each side.
            let prev = (0..p).rev().find(|&q| idx_of[q].is_some());
            let next = (p + 1..ops.len()).find(|&q| idx_of[q].is_some());
            let target = match (prev, next) {
                (Some(a), _) if is_sub(&snapshot[a]) => idx_of[a],
                (_, Some(b)) if is_sub(&snapshot[b]) => idx_of[b],
                (Some(a), _) => idx_of[a],
                (None, Some(b)) => idx_of[b],
                (None, None) => None,
            };
            let target = target.or(before[p]).filter(|&t| t < n);
            ops[p] = AlignOp::Insertion {
                copied,
                attributed_to: target,
            };
        }
    }
}

impl Alignment {
    /// The totals.
    pub fn score(&self) -> Score {
        let mut s = Score {
            expected: self.expected.len(),
            ..Score::default()
        };
        for op in &self.ops {
            match op {
                AlignOp::Match { .. } => s.matches += 1,
                AlignOp::Substitution { .. } => s.substitutions += 1,
                AlignOp::Deletion { .. } => s.deletions += 1,
                AlignOp::Insertion { .. } => s.insertions += 1,
            }
        }
        s
    }
}

/// The result of [`align_unordered`]: each expected item aligned against
/// the copied word it was matched with (or against nothing), plus the
/// copied words that matched nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct UnorderedAlignment {
    /// One alignment per expected item, in the order given. An item that
    /// was not copied is all deletions.
    pub items: Vec<Alignment>,
    /// For each expected item, the index into `copied_words` it matched.
    pub matched: Vec<Option<usize>>,
    /// The copied text split into words, as scored.
    pub copied_words: Vec<String>,
    /// Copied words that matched no expected item, each as an alignment
    /// of pure insertions, in copied order.
    pub extras: Vec<Alignment>,
}

impl UnorderedAlignment {
    /// The totals across every item and every extra word. Word spaces
    /// between items are not scored: order and separation are free.
    pub fn score(&self) -> Score {
        let mut total = Score::default();
        for a in self.items.iter().chain(&self.extras) {
            total.add(&a.score());
        }
        total
    }
}

/// Order-free scoring, for a pileup: the operator types every call they
/// heard, in any order, separated by spaces.
///
/// Each expected item (normally one callsign) is matched with at most one
/// copied word by an optimal assignment — the Hungarian algorithm over
/// integer edit costs, so the result is the minimum total error and is
/// deterministic. A pairing whose edit errors exceed half the expected
/// item's length is not a copy of it at all, and is scored as a miss plus
/// an extra rather than as a run of substitutions that would charge the
/// operator with confusions they never made. Unmatched expected items are
/// all deletions; unmatched copied words are all insertions.
///
/// Every expected item must encode; the copy may contain anything.
pub fn align_unordered(
    expected: &[&str],
    copied: &str,
    opts: &ScoreOptions,
) -> Result<UnorderedAlignment, CodecError> {
    let exp: Vec<Vec<Unit>> = expected
        .iter()
        .map(|e| codec::tokenize(e).map(|t| prepare(t, opts)))
        .collect::<Result<_, _>>()?;
    let mut words: Vec<Vec<Unit>> = vec![Vec::new()];
    for u in tokenize_copy(copied) {
        if u == Token::WordSpace {
            words.push(Vec::new());
        } else if let Some(w) = words.last_mut() {
            w.push(u);
        }
    }
    words.retain(|w| !w.is_empty());
    let words: Vec<Vec<Unit>> = words.into_iter().map(|w| prepare(w, opts)).collect();

    let (n, m) = (exp.len(), words.len());
    let size = n + m;
    let mut pair_align: Vec<Vec<Option<Alignment>>> = vec![vec![None; m]; n];
    let mut cost = vec![vec![0i64; size]; size];
    for i in 0..size {
        for j in 0..size {
            cost[i][j] = match (i < n, j < m) {
                (true, true) => {
                    let a = align_units(&exp[i], &words[j], opts);
                    let sc = a.score();
                    let errors = (sc.substitutions + sc.deletions + sc.insertions) as i64;
                    let plausible = 2 * errors <= exp[i].len() as i64;
                    let unmatched = (exp[i].len() + words[j].len()) as i64;
                    if plausible {
                        pair_align[i][j] = Some(a);
                        errors
                    } else {
                        unmatched
                    }
                }
                (true, false) => exp[i].len() as i64,
                (false, true) => words[j].len() as i64,
                (false, false) => 0,
            };
        }
    }
    let assign = hungarian(&cost);

    let mut items = Vec::with_capacity(n);
    let mut matched = vec![None; n];
    let mut used = vec![false; m];
    for i in 0..n {
        let j = assign[i];
        match (j < m).then(|| pair_align[i][j].take()).flatten() {
            Some(a) => {
                matched[i] = Some(j);
                used[j] = true;
                items.push(a);
            }
            None => items.push(align_units(&exp[i], &[], opts)),
        }
    }
    let extras = (0..m)
        .filter(|&j| !used[j])
        .map(|j| align_units(&[], &words[j], opts))
        .collect();
    Ok(UnorderedAlignment {
        items,
        matched,
        copied_words: words.iter().map(|w| codec::tokens_to_text(w)).collect(),
        extras,
    })
}

/// Minimum-cost perfect assignment on a square matrix (Hungarian
/// algorithm with potentials, O(n³)). Returns, for each row, its column.
fn hungarian(cost: &[Vec<i64>]) -> Vec<usize> {
    let n = cost.len();
    const INF: i64 = i64::MAX / 4;
    let mut u = vec![0i64; n + 1];
    let mut v = vec![0i64; n + 1];
    let mut p = vec![0usize; n + 1];
    let mut way = vec![0usize; n + 1];
    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0usize;
        let mut minv = vec![INF; n + 1];
        let mut used = vec![false; n + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let mut delta = INF;
            let mut j1 = 0usize;
            for j in 1..=n {
                if !used[j] {
                    let cur = cost[i0 - 1][j - 1] - u[i0] - v[j];
                    if cur < minv[j] {
                        minv[j] = cur;
                        way[j] = j0;
                    }
                    if minv[j] < delta {
                        delta = minv[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=n {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    let mut ans = vec![0usize; n];
    for j in 1..=n {
        if p[j] > 0 {
            ans[p[j] - 1] = j - 1;
        }
    }
    ans
}

#[cfg(feature = "serde")]
mod pairs {
    //! A map written as a list of `[key, value]` pairs.
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S, K, V>(m: &BTreeMap<K, V>, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        K: Serialize,
        V: Serialize,
    {
        s.collect_seq(m.iter())
    }

    pub fn deserialize<'de, D, K, V>(d: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        D: Deserializer<'de>,
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
    {
        Vec::<(K, V)>::deserialize(d).map(|v| v.into_iter().collect())
    }
}

/// How one sent unit has fared.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CharTally {
    /// Times sent.
    pub sent: u32,
    /// Times copied correctly.
    pub correct: u32,
    /// Times missed entirely.
    pub missed: u32,
    /// Extra units copied next to it.
    pub inserted_near: u32,
    /// What it was copied as instead, and how often.
    #[cfg_attr(feature = "serde", serde(with = "pairs"))]
    pub confused_as: BTreeMap<Unit, u32>,
}

impl CharTally {
    /// Fraction copied correctly (1.0 if never sent).
    pub fn accuracy(&self) -> f64 {
        if self.sent == 0 {
            1.0
        } else {
            f64::from(self.correct) / f64::from(self.sent)
        }
    }

    fn merge(&mut self, o: &CharTally) {
        self.sent += o.sent;
        self.correct += o.correct;
        self.missed += o.missed;
        self.inserted_near += o.inserted_near;
        for (u, n) in &o.confused_as {
            *self.confused_as.entry(*u).or_insert(0) += n;
        }
    }
}

/// Per-unit statistics across any number of alignments. Word spaces are
/// not tracked here (they still count in [`Score`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CharStats {
    #[cfg_attr(feature = "serde", serde(with = "pairs"))]
    units: BTreeMap<Unit, CharTally>,
}

impl CharStats {
    /// Empty stats.
    pub fn new() -> Self {
        CharStats::default()
    }

    /// Adds one alignment.
    pub fn record(&mut self, a: &Alignment) {
        for op in &a.ops {
            let (unit, f): (Unit, fn(&mut CharTally)) = match op {
                AlignOp::Match { expected } => (*expected, |t| {
                    t.sent += 1;
                    t.correct += 1;
                }),
                AlignOp::Deletion { expected } => (*expected, |t| {
                    t.sent += 1;
                    t.missed += 1;
                }),
                AlignOp::Substitution { expected, copied } => {
                    if *expected != Token::WordSpace {
                        let t = self.units.entry(*expected).or_default();
                        t.sent += 1;
                        *t.confused_as.entry(*copied).or_insert(0) += 1;
                    }
                    continue;
                }
                AlignOp::Insertion { attributed_to, .. } => {
                    match attributed_to.and_then(|i| a.expected.get(i)) {
                        Some(u) => (*u, |t| t.inserted_near += 1),
                        None => continue,
                    }
                }
            };
            if unit != Token::WordSpace {
                f(self.units.entry(unit).or_default());
            }
        }
    }

    /// Adds an order-free alignment (every item and every extra word).
    pub fn record_unordered(&mut self, a: &UnorderedAlignment) {
        for x in a.items.iter().chain(&a.extras) {
            self.record(x);
        }
    }

    /// Adds another set of stats into this one.
    pub fn merge(&mut self, other: &CharStats) {
        for (u, t) in &other.units {
            self.units.entry(*u).or_default().merge(t);
        }
    }

    /// The tally for one unit.
    pub fn get(&self, unit: Unit) -> Option<&CharTally> {
        self.units.get(&unit)
    }

    /// Every unit seen, in token order.
    pub fn iter(&self) -> impl Iterator<Item = (&Unit, &CharTally)> {
        self.units.iter()
    }

    /// The `n` least accurate units that have been sent, worst first
    /// (ties: more often sent first, then token order).
    pub fn weakest(&self, n: usize) -> Vec<(Unit, f64)> {
        let mut v: Vec<(Unit, f64, u32)> = self
            .units
            .iter()
            .filter(|(_, t)| t.sent > 0)
            .map(|(u, t)| (*u, t.accuracy(), t.sent))
            .collect();
        v.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.2.cmp(&a.2))
                .then(a.0.cmp(&b.0))
        });
        v.into_iter().take(n).map(|(u, a, _)| (u, a)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(ch: char) -> Unit {
        Token::Char(ch)
    }

    fn al(e: &str, g: &str) -> Alignment {
        align(e, g, &ScoreOptions::default()).unwrap()
    }

    #[test]
    fn identical_is_perfect() {
        let a = al("CQ DE W1AW", "cq de w1aw");
        let s = a.score();
        assert_eq!(s.matches, 10);
        assert_eq!(s.cer(), 0.0);
    }

    #[test]
    fn substitution_is_charged_to_the_sent_char() {
        let a = al("CQ", "CO");
        assert_eq!(
            a.ops,
            vec![
                AlignOp::Match { expected: c('C') },
                AlignOp::Substitution {
                    expected: c('Q'),
                    copied: c('O')
                }
            ]
        );
        assert_eq!(a.score().cer(), 0.5);
    }

    #[test]
    fn deletion_and_insertion() {
        let a = al("TEST", "TST");
        assert_eq!(a.score().deletions, 1);
        assert!(a.ops.contains(&AlignOp::Deletion { expected: c('E') }));
        let a = al("5NN", "5NNN");
        let s = a.score();
        assert_eq!((s.insertions, s.matches), (1, 3));
    }

    #[test]
    fn split_letter_is_one_mistake_about_it() {
        // N (-.) copied as E T (. -): a substitution plus an insertion,
        // both charged to N.
        let a = al("ANT", "AETT");
        let s = a.score();
        assert_eq!((s.substitutions, s.insertions), (1, 1));
        let mut stats = CharStats::new();
        stats.record(&a);
        let n = stats.get(c('N')).unwrap();
        assert_eq!(n.sent, 1);
        assert_eq!(n.correct, 0);
        assert_eq!(n.inserted_near, 1);
        assert_eq!(n.confused_as.values().sum::<u32>(), 1);
        assert_eq!(stats.get(c('A')).unwrap().correct, 1);
    }

    #[test]
    fn equivalents_fold_unless_told_not_to() {
        let a = al("TU <BT> 73", "TU = 73");
        assert_eq!(a.score().cer(), 0.0);
        let strict = ScoreOptions {
            fold_equivalents: false,
            ..ScoreOptions::default()
        };
        let a = align("TU <BT> 73", "TU = 73", &strict).unwrap();
        assert_eq!(a.score().substitutions, 1);
    }

    #[test]
    fn spaces_count_unless_ignored() {
        let a = al("CQ DE", "CQDE");
        assert_eq!(a.score().deletions, 1);
        let ignore = ScoreOptions {
            spaces: SpacePolicy::Ignore,
            ..ScoreOptions::default()
        };
        assert_eq!(align("CQ DE", "CQDE", &ignore).unwrap().score().cer(), 0.0);
        // Spaces are not per-character stats.
        let mut stats = CharStats::new();
        stats.record(&a);
        assert!(stats.get(Token::WordSpace).is_none());
    }

    #[test]
    fn copy_may_contain_anything_but_expected_must_encode() {
        let a = al("AB", "A#");
        assert_eq!(a.score().substitutions, 1);
        assert!(align("A#", "A", &ScoreOptions::default()).is_err());
        assert_eq!(
            tokenize_copy("x <ar> <zz>"),
            vec![
                c('X'),
                Token::WordSpace,
                Token::Prosign(Prosign::Ar),
                Token::WordSpace,
                c('<'),
                c('Z'),
                c('Z'),
                c('>')
            ]
        );
    }

    #[test]
    fn empty_cases() {
        assert_eq!(al("", "").score().cer(), 0.0);
        assert_eq!(al("", "E").score().cer(), 1.0);
        assert_eq!(al("EE", "").score().cer(), 1.0);
        let a = al("", "E");
        assert_eq!(
            a.ops,
            vec![AlignOp::Insertion {
                copied: c('E'),
                attributed_to: None
            }]
        );
    }

    #[test]
    fn merge_is_associative_and_weakest_ranks() {
        let a1 = al("CQ CQ", "CO CQ");
        let a2 = al("QTH", "QH");
        let a3 = al("KQ", "KQ");
        let one = |a: &Alignment| {
            let mut s = CharStats::new();
            s.record(a);
            s
        };
        let mut left = one(&a1);
        left.merge(&one(&a2));
        left.merge(&one(&a3));
        let mut right23 = one(&a2);
        right23.merge(&one(&a3));
        let mut right = one(&a1);
        right.merge(&right23);
        assert_eq!(left, right);
        let mut all = CharStats::new();
        for a in [&a1, &a2, &a3] {
            all.record(a);
        }
        assert_eq!(all, left);
        let weakest = all.weakest(2);
        assert_eq!(weakest[0].0, c('T'));
        assert_eq!(weakest[1].0, c('Q'));
        assert!((weakest[1].1 - 0.75).abs() < 1e-9);
    }

    fn un(e: &[&str], g: &str) -> UnorderedAlignment {
        align_unordered(e, g, &ScoreOptions::default()).unwrap()
    }

    #[test]
    fn unordered_any_order_is_perfect() {
        let calls = ["W1AW", "K0ABC", "DL1XYZ"];
        let a = un(&calls, "dl1xyz  w1aw K0ABC");
        let s = a.score();
        assert_eq!(s.cer(), 0.0);
        assert_eq!(s.expected, 15);
        assert_eq!(a.matched, vec![Some(1), Some(2), Some(0)]);
        assert!(a.extras.is_empty());
    }

    #[test]
    fn unordered_typo_missing_and_extra() {
        let calls = ["W1AW", "K0ABC", "DL1XYZ"];
        // K0ABC copied as K0ABD; DL1XYZ missed; N2ZZZ never sent.
        let a = un(&calls, "N2ZZZ K0ABD W1AW");
        let s = a.score();
        assert_eq!(s.substitutions, 1);
        assert_eq!(s.deletions, 6);
        assert_eq!(s.insertions, 5);
        assert_eq!(a.matched, vec![Some(2), Some(1), None]);
        assert_eq!(a.extras.len(), 1);
        let mut st = CharStats::new();
        st.record_unordered(&a);
        assert_eq!(st.get(c('C')).unwrap().confused_as.get(&c('D')), Some(&1));
        assert_eq!(st.get(c('Z')).unwrap().missed, 1);
    }

    #[test]
    fn unordered_refuses_implausible_pairings() {
        // Nothing alike: a miss plus an extra, not five substitutions.
        let a = un(&["W1AW"], "K9XYZ");
        assert_eq!(a.matched, vec![None]);
        let s = a.score();
        assert_eq!((s.substitutions, s.deletions, s.insertions), (0, 4, 5));
        let mut st = CharStats::new();
        st.record_unordered(&a);
        assert!(st.get(c('W')).unwrap().confused_as.is_empty());
    }

    #[test]
    fn unordered_duplicates_and_empties() {
        let a = un(&["W1AW"], "W1AW W1AW");
        let s = a.score();
        assert_eq!((s.matches, s.insertions), (4, 4));
        let a = un(&["W1AW", "K1ABC"], "");
        assert_eq!(a.score().deletions, 9);
        let a = un(&[], "W1AW");
        assert_eq!(a.score().cer(), 1.0);
        assert!(align_unordered(&["W1#W"], "x", &ScoreOptions::default()).is_err());
    }

    #[test]
    fn unordered_is_deterministic_and_optimal() {
        // Two candidates for one call: the closer one wins, every time.
        let calls = ["K1ABC", "K1ABD"];
        let a = un(&calls, "K1ABD K1ABC");
        assert_eq!(a.matched, vec![Some(1), Some(0)]);
        assert_eq!(a.score().cer(), 0.0);
        for _ in 0..5 {
            assert_eq!(un(&calls, "K1ABX K1ABC"), un(&calls, "K1ABX K1ABC"));
        }
        assert_eq!(un(&calls, "K1ABX K1ABC").score().substitutions, 1);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn stats_round_trip_through_json() {
        let mut s = CharStats::new();
        s.record(&al("TU <BT> 73 <SK>", "TU = 78 <SK>"));
        let json = serde_json::to_string(&s).unwrap();
        let back: CharStats = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        let a = al("ANT", "AETT");
        let back: Alignment = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(back, a);
    }
}
